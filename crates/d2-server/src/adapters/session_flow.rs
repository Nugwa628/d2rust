//! The single-player session sequence on the host's drain: C→S 0x67 (game
//! creation) and C→S 0x6B (join), both queue-0 system messages that reach
//! the game through [`crate::seams::Intents::session_message`]. A
//! [`SimGame`] runs them only when it has a [`SessionFlow`]
//! ([`SimGame::set_session`]); the other system ids, and every id of a
//! game without a flow, go to the host's `SessionHandler`. The sequence of
//! messages the join queues is in [`super::session`].
//!
//! **0x67** ([`SessionFlow::create`]): the request checks in their order
//! ([`check_create`]; a refusal sends nothing), then the player limit (a
//! game holding [`SessionFlow::max_players`] clients refuses the request
//! the same way); then the arena record's
//! flags (u32@0x27 & 0x3179C7), the client record allocated and prepended
//! (state 0, [`SimGame::join`]), S→C 0x01 (difficulty u8@0x14, the arena
//! flags, expansion = flags bit 20, ladder = bit 21), S→C 0x00, client
//! state 1, S→C 0x02. The messages go straight to the client's buffers
//! (there is no player yet), so they leave with the next flush.
//!
//! **0x6B** ([`SessionFlow::join`]): no client record → stop; the load: a
//! classic game with an expansion class (≥ 5) is result 0x18, else the
//! caller's [`CharacterLoader`] creates the player (unplaced, at (0, 0))
//! and gives the join's [`Entry`] or a result code; a non-zero result
//! removes the client; then the join messages and game entry through
//! [`super::session::enter_game`] (its messages go through the action
//! wiring's transport seam and are queued at the next tick, ahead of the
//! tick's own messages).
//!
//! **0x69** leave, **0x6A** game list, **0x6C** save upload, **0x6E**,
//! **0x70** ([`SessionFlow::leave`], [`SessionFlow::game_list`],
//! [`SessionFlow::upload`]).
//!
//! Not done (named, not guessed):
//! - the game record, acts and seeds: the caller builds the game before
//!   the flow runs;
//! - building an act when a client joins it: D2RUST builds the acts with
//!   the game, so a join into an act without a DRLG fails
//!   ([`JoinError::NoAct`]);
//! - saving the character on leave without storage: with no storage
//!   installed there is no character-save writer, and the leave is
//!   recorded as [`SessionFault::NotSaved`];
//! - 0x6D (ping, out of scope).

use std::collections::{BTreeMap, BTreeSet};

pub use d2_proto::client::{CreateGame, ViewChange};
use d2_proto::FixedMessage;
use d2_sim::units::lists::client_state;
use d2_sim::units::messages as msg;
use d2_sim::units::UnitId;

use super::handlers::player::HotKey;
use super::handlers::world::{ActionEvents, Outbox};
use super::session::{announce_player, enter_game, Entry, GameSetup, JoinError};
use super::storage::SaveFault;
use super::SimGame;
use crate::buffers::QueueError;
use crate::seams::{ClientId, MessageSink};

/// The u32@0x27 bits of 0x67.
pub mod create_flags {
    /// Bit 1 or bit 2 must be set.
    pub const REQUIRED: u32 = 0x2 | 0x4;
    /// Bit 20 → the game's expansion flag.
    pub const EXPANSION: u32 = 1 << 20;
    /// Bit 21 → the game's ladder flag.
    pub const LADDER: u32 = 1 << 21;
    /// The bits the arena record keeps.
    pub const ARENA: u32 = 0x31_79C7;
}

/// Load result of an expansion class in a classic game.
pub const LOAD_CLASSIC_EXPANSION_CLASS: u32 = 0x18;
/// Set on a loader's refusal code (a load result is below 0x20) when the
/// player unit already exists at the failure: the join sends the player's
/// add messages before the 0xB4.
pub const LOAD_AFTER_PLAYER: u32 = 0x0100_0000;
/// With [`LOAD_AFTER_PLAYER`]: the skills section was read too, so S→C 0x94
/// is sent before the 0xB4.
pub const LOAD_AFTER_SKILLS: u32 = 0x0200_0000;
/// The first expansion class (client +8 ≥ 5).
const FIRST_EXPANSION_CLASS: u8 = 5;
/// Class u8@0x12 must be below this.
const CLASS_LIMIT: u8 = 7;
/// u8@0x2D above this is refused.
const LOCALE_MAX: u8 = 14;
/// 0x6C: a total of this or more is refused.
pub const UPLOAD_LIMIT: u32 = 0x2000;
/// S→C 0x5A code 3, a player left.
const EVENT_LEFT: u8 = 3;

/// What a [`CharacterLoader`] gives the join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// The player unit the load created: no room, at (0, 0).
    pub player: UnitId,
    /// The character's join values (act, name, hot keys, record).
    pub entry: Entry,
}

/// The character load of a join (a save, or a new character): creates the
/// player of `client` from the 0x67 request and
/// returns it, or a non-zero load result (0x13, 0x14, 0x15, 0x17, 0x18).
pub type CharacterLoader<D, W> =
    Box<dyn FnMut(&mut SimGame<D, W>, ClientId, &CreateGame) -> Result<Loaded, u32>>;

/// Why 0x67's checks refused a request (nothing happens).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateRefusal {
    /// The character name (cstr16@0x15) has no NUL in its 16 bytes.
    CharName,
    /// u8@0x2D > 14.
    Locale(u8),
    /// The game name (cstr16@1) has no NUL in its 16 bytes.
    GameName,
    /// u32@0x27 has neither bit 1 nor bit 2.
    Flags(u32),
    /// The client already has a record (a game); the character name of
    /// that record.
    HasGame([u8; 16]),
    /// Another record has the same character name (case-insensitive,
    /// 16 bytes); that record's name.
    NameTaken([u8; 16]),
    /// Class u8@0x12 ≥ 7.
    Class(u8),
    /// The game already has its player limit of clients (the flow's
    /// `max_players`, carried here).
    GameFull(usize),
}

/// A session step that did not complete, in drain order (D2RUST
/// diagnostics).
#[derive(Debug, PartialEq, Eq)]
pub enum SessionFault {
    /// 0x67 refused by its checks.
    CreateRefused(CreateRefusal),
    /// 0x6B without a client record: logged, nothing else happens.
    NoClientRecord,
    /// The load gave a non-zero result; the direct S→C 0xB4
    /// with the code was sent and the client removed.
    LoadRefused(u32),
    /// The join failed after the load.
    Join(JoinError),
    /// A message could not be queued.
    Queue(QueueError),
    /// The leave's character save of this client's player ran with no
    /// storage installed
    /// ([`SimGame::set_storage`]).
    NotSaved,
    /// The character storage refused the save.
    SaveFailed(String),
    /// Part of a leaving player's belongings could not be freed.
    Teardown(String),
    /// 0x6C with total ≥ 0x2000.
    UploadTotal(u32),
    /// 0x6C chunk with count + len > total.
    UploadOverflow { count: u32, len: u32, total: u32 },
    /// A live view change could not be applied.
    View(String),
}

/// A save upload in progress (the client's buffer and received count).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Upload {
    /// The `total` bytes, as received so far.
    pub buffer: Vec<u8>,
    /// The announced total length of the save.
    pub total: u32,
    /// Received count.
    pub count: u32,
    /// The upload is complete (the client's upload-done state bit).
    pub complete: bool,
    /// The save checksum of the complete buffer.
    pub checksum: u32,
}

/// The session sequence of one game (module docs).
pub struct SessionFlow<D, W> {
    loader: CharacterLoader<D, W>,
    /// The 0x67 request of each client with a record, read by its 0x6B;
    /// its character name is the record's, the name table the
    /// duplicate-name check searches.
    pub requests: BTreeMap<ClientId, CreateGame>,
    /// The game's name (from the 0x67 that created it).
    pub game_name: Option<[u8; 16]>,
    /// The game's settings (difficulty, arena flags, expansion, ladder),
    /// fixed by the 0x67 that created it. A later client's 0x67 joins
    /// this game: its S→C 0x01 carries these, not the request's.
    pub setup: Option<GameSetup>,
    /// The game's id in the game table (the u16 at 0x33 of S→C 0xB2):
    /// 0 before a game is created, then the id [`GameIds`] gave it.
    pub game_id: u16,
    /// The process-wide id counter and the table slots in use.
    pub ids: GameIds,
    /// The latest live view each client requested, once its rooms are ready.
    pub views: BTreeMap<ClientId, ViewChange>,
    /// Views applied on the authority but not acknowledged yet.
    pub pending_views: BTreeMap<ClientId, ViewChange>,
    /// Save uploads by client.
    pub uploads: BTreeMap<ClientId, Upload>,
    /// Clients whose heartbeat flag is set by 0x70 (read only by the host
    /// heartbeat).
    pub heartbeat_flag: BTreeSet<ClientId>,
    /// Session steps that did not complete, in drain order.
    pub faults: Vec<(ClientId, SessionFault)>,
    /// How many clients the game takes: a 0x67 from one more client is
    /// refused ([`CreateRefusal::GameFull`]). [`DEFAULT_MAX_PLAYERS`]
    /// unless the server sets another; nothing else in the game has a
    /// fixed player count, so raising it is only this value.
    pub max_players: usize,
}

/// The player limit of a game unless the server sets another: eight
/// players, as the game is designed and balanced for.
pub const DEFAULT_MAX_PLAYERS: usize = 8;

/// The game table's ids: a counter (initially 1) names the next id to try; ids run 1 ..= 0x400
/// and wrap to 1; the first id from the counter whose slot is free is
/// taken and the counter moves past it; none free: 0 (no game).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameIds {
    next: u16,
    used: BTreeSet<u16>,
}

impl Default for GameIds {
    fn default() -> Self {
        Self {
            next: 1,
            used: BTreeSet::new(),
        }
    }
}

impl GameIds {
    /// The ids' upper bound (the table has 1,024 slots).
    pub const MAX: u16 = 0x400;

    /// Takes the next free id, 0 when the table is full.
    pub fn alloc(&mut self) -> u16 {
        let mut id = self.next;
        for _ in 0..Self::MAX {
            if id == 0 || id > Self::MAX {
                id = 1;
            }
            if !self.used.contains(&id) {
                self.used.insert(id);
                self.next = if id >= Self::MAX { 1 } else { id + 1 };
                return id;
            }
            id += 1;
        }
        0
    }

    /// Frees `id`'s slot (the game is destroyed); the counter stays.
    pub fn release(&mut self, id: u16) {
        self.used.remove(&id);
    }
}

impl<D, W> SessionFlow<D, W> {
    /// A flow with no game yet, loading characters through `loader`.
    pub fn new(loader: CharacterLoader<D, W>) -> Self {
        Self {
            loader,
            requests: BTreeMap::new(),
            views: BTreeMap::new(),
            pending_views: BTreeMap::new(),
            game_name: None,
            setup: None,
            game_id: 0,
            ids: GameIds::default(),
            uploads: BTreeMap::new(),
            heartbeat_flag: BTreeSet::new(),
            faults: Vec::new(),
            max_players: DEFAULT_MAX_PLAYERS,
        }
    }

    /// The same flow with a player limit of `n` clients.
    pub fn with_max_players(mut self, n: usize) -> Self {
        self.max_players = n;
        self
    }
}

/// Whether two character names are equal: ASCII case-insensitive, up to
/// 16 bytes or the first NUL.
pub fn same_name(a: &[u8; 16], b: &[u8; 16]) -> bool {
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (x.to_ascii_lowercase(), y.to_ascii_lowercase());
        if x != y {
            return false;
        }
        if x == 0 {
            break;
        }
    }
    true
}

/// The checks of a 0x67 request in their order (single player: no host
/// callbacks, so u8@0x11 is not tested). `has_game`: the client's record
/// name when it has one; `taken`: the name of a record with the same
/// character name.
pub fn check_create(
    r: &CreateGame,
    has_game: Option<[u8; 16]>,
    taken: Option<[u8; 16]>,
) -> Result<(), CreateRefusal> {
    if !r.char_name.contains(&0) {
        return Err(CreateRefusal::CharName);
    }
    if r.locale > LOCALE_MAX {
        return Err(CreateRefusal::Locale(r.locale));
    }
    if !r.game_name.contains(&0) {
        return Err(CreateRefusal::GameName);
    }
    if r.flags & create_flags::REQUIRED == 0 {
        return Err(CreateRefusal::Flags(r.flags));
    }
    if let Some(n) = has_game {
        return Err(CreateRefusal::HasGame(n));
    }
    if let Some(n) = taken {
        return Err(CreateRefusal::NameTaken(n));
    }
    if r.class >= CLASS_LIMIT {
        return Err(CreateRefusal::Class(r.class));
    }
    Ok(())
}

/// S→C 0x01's game fields from a request: the arena record's flags are u32@0x27 & 0x3179C7.
pub fn game_setup(r: &CreateGame) -> GameSetup {
    GameSetup {
        difficulty: r.difficulty,
        arena_flags: r.flags & create_flags::ARENA,
        expansion: r.flags & create_flags::EXPANSION != 0,
        ladder: r.flags & create_flags::LADDER != 0,
    }
}

/// The 40-byte S→C 0x5A of a join (code 2) or leave (code 3), shared
/// with the sim's join sequence.
pub use d2_sim::units::messages::player_event;

/// The 53-byte S→C 0xB2 game list entry: the name (16
/// bytes), u16@0x31, u16@0x33; bytes 0x11–0x30 are never written (D2RUST:
/// zero).
pub fn game_list_entry(name: &[u8; 16], players: u16, id: u16) -> [u8; 53] {
    let mut m = [0u8; 53];
    m[0] = 0xB2;
    m[1..17].copy_from_slice(name);
    m[0x31..0x33].copy_from_slice(&players.to_le_bytes());
    m[0x33..0x35].copy_from_slice(&id.to_le_bytes());
    m
}

impl<D: ActionEvents, W: super::session::GolemWorld> SessionFlow<D, W>
where
    D::X: Outbox,
{
    /// One queue-0 message; false for the ids the flow does not run.
    pub fn message(
        &mut self,
        s: &mut SimGame<D, W>,
        client: ClientId,
        m: &[u8],
        out: &mut dyn MessageSink,
    ) -> bool {
        match m.first() {
            Some(&ViewChange::ID) => {
                if let Ok(r) = ViewChange::decode(m) {
                    self.change_view(s, client, r);
                }
                true
            }
            Some(&CreateGame::ID) => {
                // The transport checked the 46-byte size.
                if let Ok(r) = CreateGame::decode(m) {
                    self.create(s, client, &r, out);
                }
                true
            }
            Some(&0x69) => {
                self.leave(s, client, out);
                true
            }
            Some(&0x6A) => {
                self.game_list(s, client, out);
                true
            }
            Some(&0x6B) => {
                self.join(s, client, out);
                true
            }
            Some(&0x6C) => {
                self.upload(s, client, m);
                true
            }
            // 0x6E: no state change and no message.
            Some(&0x6E) => true,
            Some(&0x70) => {
                // 0x70: the record found → its heartbeat flag := 1.
                if s.sim_client(client).is_some() {
                    self.heartbeat_flag.insert(client);
                }
                true
            }
            _ => false,
        }
    }

    /// C→S 0x67 (module docs).
    /// C->S 0x72: apply a live view, sharing the largest requested range
    /// with every client so one client's shrink cannot hide another's rooms.
    pub fn change_view(&mut self, s: &mut SimGame<D, W>, client: ClientId, r: ViewChange) {
        let Some(id) = s.sim_client(client) else {
            return;
        };
        if s.game.lists.client(id).map(|e| e.state) != Some(client_state::IN_GAME) {
            return;
        }
        if r.width == 0 || r.height == 0 {
            return;
        }
        if let Some(old) = self.views.get(&client) {
            if r.seq != old.seq && (r.seq.wrapping_sub(old.seq) as i32) <= 0 {
                return;
            }
        }
        let previous = self.views.insert(client, r);
        let gap = self
            .requests
            .iter()
            .map(|(&c, initial)| {
                self.views.get(&c).map_or(
                    i32::from(initial.view_gap).max(d2_sim::drlg::NEAR_GAP),
                    |r| d2_sim::drlg::view_gap_for_frame(r.width as i32, r.height as i32),
                )
            })
            .max()
            .unwrap_or(d2_sim::drlg::NEAR_GAP);
        let game = &mut s.game;
        let result = s
            .events
            .lend_world(|a| a.with(game, |game, v| v.resize_view_gap(game, gap)));
        if let Err(e) = result {
            if let Some(previous) = previous {
                self.views.insert(client, previous);
            } else {
                self.views.remove(&client);
            }
            self.faults
                .push((client, SessionFault::View(format!("{e:?}"))));
            return;
        }
        self.pending_views.insert(client, r);
    }

    /// Queue the acknowledgement of every pending view whose rooms are now
    /// populated. Runs after the tick's room, unit and map messages.
    pub fn finish_views(&mut self, s: &mut SimGame<D, W>, out: &mut dyn MessageSink) {
        let pending: Vec<_> = self
            .pending_views
            .iter()
            .map(|(&client, &r)| (client, r))
            .collect();
        for (client, r) in pending {
            let Some(id) = s.sim_client(client) else {
                self.pending_views.remove(&client);
                continue;
            };
            let Some(room) = s.game.lists.client(id).and_then(|e| e.room) else {
                continue;
            };
            let ready = {
                let game = &s.game;
                let drlg = &s.events.action().sys.hooks.drlg;
                drlg.drlg_room(game, room).is_some_and(|(d, anchor)| {
                    let rooms = d.view_rooms(anchor);
                    !rooms.is_empty()
                        && rooms.into_iter().all(|r| {
                            d.active_room(r)
                                .and_then(|a| game.lists.room(a.id))
                                .is_some_and(|e| e.populated)
                        })
                })
            };
            if !ready {
                continue;
            }
            let ack = d2_proto::server::ViewReady {
                seq: r.seq,
                width: r.width,
                height: r.height,
            };
            match out.queue(client, &ack.encode()) {
                Ok(()) => {
                    self.pending_views.remove(&client);
                }
                Err(e) => self.faults.push((client, SessionFault::Queue(e))),
            }
        }
    }
    pub fn create(
        &mut self,
        s: &mut SimGame<D, W>,
        client: ClientId,
        r: &CreateGame,
        out: &mut dyn MessageSink,
    ) {
        let has_game = s
            .sim_client(client)
            .map(|_| self.requests.get(&client).map_or([0; 16], |q| q.char_name));
        let taken = self
            .requests
            .values()
            .map(|q| q.char_name)
            .find(|n| same_name(n, &r.char_name));
        if let Err(e) = check_create(r, has_game, taken) {
            self.faults.push((client, SessionFault::CreateRefused(e)));
            return;
        }
        // A full game takes no more clients: the request is refused like
        // a failed check, and the client has no record.
        if self.requests.len() >= self.max_players {
            let full = CreateRefusal::GameFull(self.max_players);
            self.faults
                .push((client, SessionFault::CreateRefused(full)));
            return;
        }
        // The client record, prepended, state 0 (checked absent above, so
        // this cannot fail).
        let Ok(id) = s.join(client, None, None, 0) else {
            return;
        };
        self.requests.insert(client, *r);
        if self.game_name.is_none() {
            // The game is created with this 0x67.
            self.game_name = Some(r.game_name);
            self.setup = Some(game_setup(r));
            self.game_id = self.ids.alloc();
            // The game's view (its creator's frame): one view gap for
            // every client of the game.
            s.events
                .action()
                .sys
                .hooks
                .drlg
                .dungeon
                .set_view_gap(i32::from(r.view_gap));
            // A mod game's hook: the game exists, no client has loaded.
            s.events.action().game_created(&mut s.game);
        }
        // Every client of the game is told the game's own settings.
        let g = self.setup.unwrap_or_else(|| game_setup(r));
        let flags = msg::game_flags(g.difficulty, g.arena_flags, g.expansion, g.ladder);
        // S→C 0x01, 0x00 (client state 1), 0x02.
        let mut sent = out.queue(client, &flags);
        sent = sent.and_then(|_| out.queue(client, &msg::GAME_LOADING));
        if let Some(e) = s.game.lists.client_mut(id) {
            e.state = client_state::LOADING;
        }
        sent = sent.and_then(|_| out.queue(client, &msg::LOAD_SUCCESSFUL));
        if let Err(e) = sent {
            self.faults.push((client, SessionFault::Queue(e)));
        }
    }

    /// C→S 0x69: only for a client in state 4. The characters of the
    /// game's clients with a player are saved (single player always saves),
    /// then the leaving client gets S→C 0x05, 0x06, a direct 0xB0 and its
    /// buffers flushed; its record is removed; S→C 0x5C (its player GUID)
    /// then the 0x5A code 3 go to every remaining client in state 4
    /// (client-list order) when the name has a NUL in its 16 bytes.
    /// Returns whether the client left.
    pub fn leave(
        &mut self,
        s: &mut SimGame<D, W>,
        client: ClientId,
        out: &mut dyn MessageSink,
    ) -> bool {
        let Some(id) = s.sim_client(client) else {
            return false;
        };
        if s.game.lists.client(id).map(|e| e.state) != Some(client_state::IN_GAME) {
            return false;
        }
        self.save_all(s);
        let mut sent = out.queue(client, &[0x05]);
        sent = sent.and_then(|_| out.queue(client, &[0x06]));
        sent = sent.and_then(|_| out.send_direct(client, &[0xB0]));
        sent = sent.and_then(|_| out.flush_client(client));
        if let Err(e) = sent {
            self.faults.push((client, SessionFault::Queue(e)));
        }
        self.remove_client(s, client, out);
        true
    }

    /// A client whose connection ended without a leave (lost, timed out,
    /// or its reconnect grace ran out): in any state, the characters are
    /// saved and the client leaves the game as with C→S 0x69, without the
    /// messages to the client itself. Returns whether it had a record.
    pub fn gone(
        &mut self,
        s: &mut SimGame<D, W>,
        client: ClientId,
        out: &mut dyn MessageSink,
    ) -> bool {
        if s.sim_client(client).is_none() {
            return false;
        }
        if s.player_of(client).is_some() {
            self.save_all(s);
        }
        self.remove_client(s, client, out);
        true
    }

    /// Saves the character of every client with a player (a leave
    /// always saves), recording the faults.
    fn save_all(&mut self, s: &mut SimGame<D, W>) {
        for (c, r) in s.save_characters() {
            match r {
                Ok(()) => {}
                Err(SaveFault::NoStorage) => self.faults.push((c, SessionFault::NotSaved)),
                Err(SaveFault::Failed(e)) => self.faults.push((c, SessionFault::SaveFailed(e))),
            }
        }
    }

    /// The leave's removal: the client's session state and record, its
    /// player and belongings, the player count, the game's end with its
    /// last client, then S→C 0x5C and the 0x5A code 3 to the clients in
    /// game.
    fn remove_client(
        &mut self,
        s: &mut SimGame<D, W>,
        client: ClientId,
        out: &mut dyn MessageSink,
    ) {
        let name = self
            .requests
            .remove(&client)
            .map_or([0; 16], |r| r.char_name);
        self.uploads.remove(&client);
        self.heartbeat_flag.remove(&client);
        self.views.remove(&client);
        self.pending_views.remove(&client);
        let guid = s
            .player_of(client)
            .and_then(|p| s.game.lists.unit(p))
            .map(|e| e.guid);
        let player = s.player_of(client);
        // Cannot fail: the client is joined.
        let _ = s.leave(client);
        // The player and what it owns leave the game with its client.
        if let Some(p) = player {
            for f in s.remove_player(p) {
                self.faults.push((client, SessionFault::Teardown(f)));
            }
        }
        set_player_count(s);
        // The last client gone: the game ends and its id slot is free.
        if self.requests.is_empty() {
            self.game_name = None;
            self.setup = None;
            self.ids.release(std::mem::take(&mut self.game_id));
            // A mod game's hook: the game is over.
            s.events.action().game_freed(&mut s.game);
        }
        // The remaining clients in state 4, client-list order (the leaver
        // is already unlinked, so it gets neither message).
        let in_game: Vec<ClientId> = s
            .client_list()
            .into_iter()
            .filter(|&c| {
                s.sim_client(c)
                    .and_then(|i| s.game.lists.client(i))
                    .is_some_and(|e| e.state == client_state::IN_GAME)
            })
            .collect();
        // S→C 0x5C during the removal.
        if let Some(g) = guid {
            let m = msg::player_left(g);
            for &c in &in_game {
                if let Err(e) = out.queue(c, &m) {
                    self.faults.push((c, SessionFault::Queue(e)));
                }
            }
        }
        if name.contains(&0) {
            let m = player_event(EVENT_LEFT, &name);
            for &c in &in_game {
                if let Err(e) = out.queue(c, &m) {
                    self.faults.push((c, SessionFault::Queue(e)));
                }
            }
        }
    }

    /// C→S 0x6A, the game list request: a direct S→C 0xB2 for the game (D2RUST runs
    /// one game per flow), then the terminator (empty name, 0, 0xFFFF).
    /// The client ignores 0xB2.
    pub fn game_list(&mut self, s: &SimGame<D, W>, client: ClientId, out: &mut dyn MessageSink) {
        let mut sent = Ok(());
        if let Some(name) = self.game_name {
            // The game's client count is the client list's length (a
            // client add increments it, a removal decrements it).
            let players = s.client_list().len() as u16;
            sent = out.send_direct(client, &game_list_entry(&name, players, self.game_id));
        }
        sent = sent.and_then(|_| out.send_direct(client, &game_list_entry(&[0; 16], 0, 0xFFFF)));
        if let Err(e) = sent {
            self.faults.push((client, SessionFault::Queue(e)));
        }
    }

    /// C→S 0x6C: one save chunk (len u8@1, total
    /// u32@2, data @6) appended to the client's upload; complete →
    /// the client's upload-done bit and the checksum. No client record → nothing.
    pub fn upload(&mut self, s: &SimGame<D, W>, client: ClientId, m: &[u8]) {
        let (Some(&len), Some(t)) = (m.get(1), m.get(2..6)) else {
            return;
        };
        let total = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
        if total >= UPLOAD_LIMIT {
            self.faults.push((client, SessionFault::UploadTotal(total)));
            return;
        }
        if s.sim_client(client).is_none() {
            return;
        }
        let u = self.uploads.entry(client).or_default();
        if u.count == 0 {
            *u = Upload {
                buffer: Vec::with_capacity(total as usize),
                total,
                ..Upload::default()
            };
        }
        let len = u32::from(len);
        if u.count + len > u.total {
            let f = SessionFault::UploadOverflow {
                count: u.count,
                len,
                total: u.total,
            };
            self.faults.push((client, f));
            return;
        }
        let data = m.get(6..6 + len as usize).unwrap_or(&[]);
        u.buffer.extend_from_slice(data);
        u.count += len;
        if u.count == u.total {
            u.complete = true;
            u.checksum = d2_formats::d2s::checksum(&u.buffer);
        }
    }

    /// C→S 0x6B (module docs). Returns the placed player.
    pub fn join(
        &mut self,
        s: &mut SimGame<D, W>,
        client: ClientId,
        out: &mut dyn MessageSink,
    ) -> Option<UnitId> {
        let (Some(id), Some(r)) = (s.sim_client(client), self.requests.get(&client).copied())
        else {
            self.faults.push((client, SessionFault::NoClientRecord));
            return None;
        };
        let classic = r.flags & create_flags::EXPANSION == 0;
        let loaded = if classic && r.class >= FIRST_EXPANSION_CLASS {
            Err(LOAD_CLASSIC_EXPANSION_CLASS)
        } else {
            (self.loader)(s, client, &r)
        };
        let loaded = match loaded {
            Ok(l) => l,
            Err(code) => {
                // S→C 0xB4 (direct) with the code, then the client is
                // removed.
                let flags = LOAD_AFTER_PLAYER | LOAD_AFTER_SKILLS;
                let announce = code < 4 * LOAD_AFTER_PLAYER && code & LOAD_AFTER_PLAYER != 0;
                let skills = announce && code & LOAD_AFTER_SKILLS != 0;
                let code = if announce { code & !flags } else { code };
                if announce {
                    if let Err(e) = announce_player(s, client, &r.char_name, skills, out) {
                        self.faults.push((client, SessionFault::Join(e)));
                    }
                }
                let mut b = vec![0xB4];
                b.extend_from_slice(&code.to_le_bytes());
                if let Err(e) = out.send_direct(client, &b) {
                    self.faults.push((client, SessionFault::Queue(e)));
                }
                // Cannot fail: the client is joined.
                let _ = s.leave(client);
                self.requests.remove(&client);
                self.faults.push((client, SessionFault::LoadRefused(code)));
                return None;
            }
        };
        if let Some(e) = s.game.lists.client_mut(id) {
            e.player = Some(loaded.player);
        }
        // The load writes the client's hot-key slots: the slots the save
        // reads back and C→S 0x51 changes. Unbound slots (skill −1) stay as
        // a new record holds them.
        for (slot, k) in loaded.entry.hotkeys.iter().enumerate() {
            if k.skill >= 0 {
                let key = HotKey {
                    skill: k.skill,
                    left: k.flag,
                    item: k.item,
                };
                s.set_hotkey(client, slot, key);
            }
        }
        let entered = enter_game(s, client, &loaded.entry);
        set_player_count(s);
        match entered {
            Ok(p) => Some(p),
            Err(e) => {
                self.faults.push((client, SessionFault::Join(e)));
                None
            }
        }
    }
}

/// Tells the game how many players it has (the clients with a player
/// unit): monster scaling reads it. A count of 0 or 1 plays the same.
fn set_player_count<D: ActionEvents, W>(s: &mut SimGame<D, W>) {
    let n = s
        .client_list()
        .into_iter()
        .filter(|&c| s.player_of(c).is_some())
        .count();
    s.events
        .set_player_count(i32::try_from(n).unwrap_or(i32::MAX));
}

/// A [`SessionFlow`] behind [`SimGame`]'s `session` field, so the
/// game's `Intents` impl (no `ActionEvents` bound) can run it.
pub trait SessionRunner<D, W> {
    /// Runs one queue-0 message of `client`; false for ids the flow does
    /// not run.
    fn run(
        &mut self,
        s: &mut SimGame<D, W>,
        client: ClientId,
        m: &[u8],
        out: &mut dyn MessageSink,
    ) -> bool;
    /// Queue the acknowledgements of views whose rooms are now populated.
    fn finish_views(&mut self, _s: &mut SimGame<D, W>, _out: &mut dyn MessageSink) {}
    /// The flow itself, for its requests and faults.
    fn flow(&self) -> &SessionFlow<D, W>;
    /// A client whose connection ended ([`SessionFlow::gone`]).
    fn gone(&mut self, s: &mut SimGame<D, W>, client: ClientId, out: &mut dyn MessageSink);
}

impl<D: ActionEvents, W: super::session::GolemWorld> SessionRunner<D, W> for SessionFlow<D, W>
where
    D::X: Outbox,
{
    fn run(
        &mut self,
        s: &mut SimGame<D, W>,
        client: ClientId,
        m: &[u8],
        out: &mut dyn MessageSink,
    ) -> bool {
        self.message(s, client, m, out)
    }
    fn finish_views(&mut self, s: &mut SimGame<D, W>, out: &mut dyn MessageSink) {
        self.finish_views(s, out)
    }
    fn flow(&self) -> &SessionFlow<D, W> {
        self
    }
    fn gone(&mut self, s: &mut SimGame<D, W>, client: ClientId, out: &mut dyn MessageSink) {
        SessionFlow::gone(self, s, client, out);
    }
}

#[cfg(test)]
mod game_id_tests {
    use super::*;

    /// The first game of a process has id 1, the next 2; a freed id is
    /// reused only when the counter wraps to it.
    #[test]
    fn ids_start_at_one_and_count_up() {
        let mut ids = GameIds::default();
        assert_eq!((ids.alloc(), ids.alloc()), (1, 2));
        ids.release(1);
        assert_eq!(ids.alloc(), 3);
    }

    /// Ids run 1..= 0x400 and wrap to 1, skipping taken slots; a full
    /// table gives 0.
    #[test]
    fn ids_wrap_and_skip_taken_slots() {
        let mut ids = GameIds::default();
        let all: Vec<u16> = (0..0x400).map(|_| ids.alloc()).collect();
        assert_eq!(all[0], 1);
        assert_eq!(all[0x3FF], 0x400);
        assert_eq!(ids.alloc(), 0, "no slot free");
        ids.release(5);
        assert_eq!(ids.alloc(), 5, "the wrapped counter finds the free slot");
    }

    /// The 0xB2 entry carries the id at u16@0x33.
    #[test]
    fn game_list_entry_carries_the_id() {
        let e = game_list_entry(b"game\0\0\0\0\0\0\0\0\0\0\0\0", 1, 2);
        assert_eq!(u16::from_le_bytes([e[0x33], e[0x34]]), 2);
    }
}
