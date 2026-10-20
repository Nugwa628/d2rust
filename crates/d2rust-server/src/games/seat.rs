//! The seats of one game, and the parts of the game that act on them:
//!
//! - [`SeatStream`]: a seat's game stream as the hub reads it. It checks
//!   the player's 0x67 join against the seat and writes the game's own
//!   settings into it, and drops character uploads.
//! - [`characters`]: the game's character loader input: each client plays
//!   its seat's character.
//! - [`ServerStore`]: the game's character writer: each client's character
//!   goes to its seat's file.
//! - [`ServerSession`]: the system messages the game's session flow
//!   doesn't take (the ping).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use d2_formats::d2s::appearance::AppearanceTables;
use d2_formats::d2s::{self, D2s, SaveTables};
use d2_net::msg::account::CharSummary;
use d2_net::{Lane, NetError, Packet, Transport};
use d2_proto::FixedMessage;
use d2_server::adapters::character::LoadContext;
use d2_server::adapters::session_flow::{create_flags, CreateGame};
use d2_server::adapters::storage::CharacterStore;
use d2_server::game::{
    character_save, Character, CharacterCodec, Characters, D2sCodec, NewCharacter, RefusedLoad,
    Sim, World, LOAD_FAILED,
};
use d2_server::local_seams::LocalSeams;
use d2_server::seams::{ClientId, MessageSink, SessionHandler};
use d2_sim::wiring::worldgen::WorldSim;
use tokio::sync::mpsc;

use super::{AttachedStream, GameEvent, GameSpec, Seat};

/// C→S 0x67, the join that creates or enters the game.
const CREATE_GAME: u8 = 0x67;
/// C→S 0x6C, a character upload: online characters live on the server.
const SAVE_UPLOAD: u8 = 0x6C;
/// C→S 0x6D, the ping.
const PING: u8 = 0x6D;

/// One seated client.
#[derive(Debug)]
pub(super) struct SeatState {
    pub seat: Seat,
    /// The save the character's live state is laid over at each save: its
    /// file as loaded, or a new character's stub.
    pub base: D2s,
    /// When the seat was taken.
    pub admitted: std::time::Instant,
    /// The client has had a record in the game (its join was taken).
    pub entered: bool,
    /// What the game sent it while it joined: the character state a
    /// re-entry sends again (contracts v12 3.7).
    pub snapshot: Arc<Mutex<JoinSnapshot>>,
}

/// Every S->C message a client was sent since it joined, kept so a re-entry
/// can rebuild its world and character state. Contracts v12 3.7: the client
/// clears its world on a resume; replaying this complete server-side journal
/// (not the hub parked backlog, which misses what a dying connection
/// swallowed) rebuilds the state as it is now: items, stats, quests, skills,
/// waypoints, mercenary, party and the world, in order. Bounded like the
/// hub backlog; past it the client keeps what it had.
#[derive(Debug, Default)]
pub(super) struct JoinSnapshot {
    messages: Vec<Vec<u8>>,
}

/// Most messages kept per client (the hub backlog limit).
const MAX_JOURNAL: usize = 1 << 16;

impl JoinSnapshot {
    fn record(&mut self, m: &[u8]) {
        if self.messages.len() < MAX_JOURNAL {
            self.messages.push(m.to_vec());
        }
    }

    /// Every message the client missed, in send order, without the ones
    /// that open a window: a re-entry closes any open window first, so
    /// replaying a stash or cube open would leave a window the server no
    /// longer has.
    pub fn messages(&self) -> impl Iterator<Item = &Vec<u8>> {
        self.messages.iter().filter(|m| !opens_window(m))
    }
}

/// S→C 0x77 actions that open a window (the stash view and the cube).
fn opens_window(m: &[u8]) -> bool {
    matches!(m, [0x77, 0x10, ..] | [0x77, 0x15, ..])
}

/// The game's seats by client.
#[derive(Debug, Default)]
pub(super) struct Seats {
    by_client: BTreeMap<ClientId, SeatState>,
}

/// Why a seat can't be taken.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Taken {
    Free,
    Full,
    /// The seat's character is in this game already.
    Character,
}

impl Seats {
    pub fn taken(&self, seat: &Seat, max: usize) -> Taken {
        if self
            .by_client
            .values()
            .any(|s| s.seat.character.eq_ignore_ascii_case(&seat.character))
        {
            return Taken::Character;
        }
        if self.by_client.len() >= max {
            return Taken::Full;
        }
        Taken::Free
    }

    pub fn insert(
        &mut self,
        client: ClientId,
        seat: Seat,
        base: D2s,
        snapshot: Arc<Mutex<JoinSnapshot>>,
    ) {
        self.by_client.insert(
            client,
            SeatState {
                seat,
                base,
                admitted: std::time::Instant::now(),
                entered: false,
                snapshot,
            },
        );
    }

    pub fn get(&self, client: ClientId) -> Option<&SeatState> {
        self.by_client.get(&client)
    }

    /// The seat of the character `name` (any case).
    pub fn client_of(&self, name: &str) -> Option<ClientId> {
        self.by_client
            .iter()
            .find(|(_, s)| s.seat.character.eq_ignore_ascii_case(name))
            .map(|(&c, _)| c)
    }

    /// The seated characters' lowercase names.
    pub fn name_keys(&self) -> std::collections::BTreeSet<String> {
        self.by_client
            .values()
            .map(|s| s.seat.character.to_ascii_lowercase())
            .collect()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ClientId, &mut SeatState)> {
        self.by_client.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.by_client.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_client.is_empty()
    }

    /// Takes out the seats whose client is not in `held`.
    pub fn remove_missing(&mut self, held: &[ClientId]) -> Vec<SeatState> {
        let gone: Vec<ClientId> = self
            .by_client
            .keys()
            .copied()
            .filter(|c| !held.contains(c))
            .collect();
        gone.into_iter()
            .filter_map(|c| self.by_client.remove(&c))
            .collect()
    }
}

/// The character each joining client plays: its seat's. A client with no
/// seat (never the case: the hub only reads seated clients) is refused.
pub(super) fn characters(seats: Rc<RefCell<Seats>>, spec: GameSpec) -> Characters {
    Box::new(move |client, r: &CreateGame| {
        let seats = seats.borrow();
        let Some(s) = seats.get(client) else {
            return Character::Refused(RefusedLoad {
                result: LOAD_FAILED,
                name: r.char_name,
                expansion: spec.expansion,
                difficulty: spec.difficulty,
                class: r.class,
                after_player: false,
                skills: None,
            });
        };
        match &s.seat.new {
            Some(new) => Character::Named(NewCharacter {
                class: new.class,
                name: name16(&s.seat.character),
                difficulty: spec.difficulty,
            }),
            None => Character::Save(
                Box::new(s.base.clone()),
                LoadContext {
                    difficulty: spec.difficulty,
                    map_seed_applies: false,
                },
            ),
        }
    })
}

/// A name as the 16-byte NUL-padded field of the game's messages.
fn name16(name: &str) -> [u8; 16] {
    let mut b = [0u8; 16];
    let n = name.len().min(15);
    b[..n].copy_from_slice(&name.as_bytes()[..n]);
    b
}

/// A seat's game stream as the hub reads it.
///
/// The player's 0x67 join must name the seat's character (any case), or
/// the stream breaks the rules and the hub drops it. The join is then
/// rewritten with the game's settings (its name, difficulty and kind), the
/// character's stored name and, for a new character, its class (the game
/// makes a new character's player of the join's class), so the client
/// decides none of them. A character upload (0x6C) is dropped: the client
/// never sends a character.
pub struct SeatStream {
    inner: Box<dyn AttachedStream>,
    character: String,
    /// A new character's class; `None` for a stored one (the game takes
    /// its class from the save).
    new_class: Option<u8>,
    game_name: [u8; 16],
    difficulty: u8,
    expansion: bool,
    /// Where a joining client's S→C messages are kept for a re-entry.
    snapshot: Option<Arc<Mutex<JoinSnapshot>>>,
}

impl SeatStream {
    pub(super) fn new(inner: Box<dyn AttachedStream>, seat: &Seat, spec: &GameSpec) -> Self {
        Self::with_snapshot(inner, seat, spec, None)
    }

    pub(super) fn capturing(
        inner: Box<dyn AttachedStream>,
        seat: &Seat,
        spec: &GameSpec,
        snapshot: Arc<Mutex<JoinSnapshot>>,
    ) -> Self {
        Self::with_snapshot(inner, seat, spec, Some(snapshot))
    }

    fn with_snapshot(
        inner: Box<dyn AttachedStream>,
        seat: &Seat,
        spec: &GameSpec,
        snapshot: Option<Arc<Mutex<JoinSnapshot>>>,
    ) -> Self {
        Self {
            inner,
            character: seat.character.clone(),
            new_class: seat.new.as_ref().map(|n| n.class),
            game_name: name16(&spec.name),
            difficulty: spec.difficulty,
            expansion: spec.expansion,
            snapshot,
        }
    }

    /// The seat's version of a 0x67 join, or `None` if it names another
    /// character or doesn't decode.
    fn seat_join(&self, msg: &[u8]) -> Option<Vec<u8>> {
        let mut r = CreateGame::decode(msg).ok()?;
        let n = r.char_name.iter().position(|&b| b == 0).unwrap_or(16);
        let named = std::str::from_utf8(&r.char_name[..n]).ok()?;
        if !named.eq_ignore_ascii_case(&self.character) {
            return None;
        }
        r.char_name = name16(&self.character);
        if let Some(class) = self.new_class {
            r.class = class;
        }
        r.game_name = self.game_name;
        r.difficulty = self.difficulty;
        r.flags &= !(create_flags::EXPANSION | create_flags::LADDER);
        if self.expansion {
            r.flags |= create_flags::EXPANSION;
        }
        Some(r.encode().to_vec())
    }
}

impl Transport for SeatStream {
    fn send(&mut self, p: Packet) -> Result<(), NetError> {
        if let (Some(snapshot), Packet::Deliver(m)) = (&self.snapshot, &p) {
            snapshot.lock().unwrap_or_else(|e| e.into_inner()).record(m);
        }
        self.inner.send(p)
    }

    fn recv(&mut self) -> Result<Option<Packet>, NetError> {
        loop {
            match self.inner.recv()? {
                Some(Packet::Send {
                    lane: Lane::System,
                    msg,
                }) if msg.first() == Some(&CREATE_GAME) => {
                    let msg = self.seat_join(&msg).ok_or(NetError::Malformed)?;
                    return Ok(Some(Packet::Send {
                        lane: Lane::System,
                        msg,
                    }));
                }
                Some(Packet::Send { msg, .. }) if msg.first() == Some(&SAVE_UPLOAD) => continue,
                other => return Ok(other),
            }
        }
    }

    fn flush(&mut self) -> Result<(), NetError> {
        self.inner.flush()
    }
}

/// The system messages the session flow leaves to the host: the ping is
/// answered with the pong; the rest decide nothing on a server.
#[derive(Debug, Default)]
pub struct ServerSession;

/// S→C 0x8F, the pong: the id and 32 zero bytes.
const PONG: [u8; 33] = {
    let mut b = [0; 33];
    b[0] = 0x8F;
    b
};

impl SessionHandler for ServerSession {
    fn system_message(
        &mut self,
        client: ClientId,
        msg: &[u8],
        _size: usize,
        out: &mut dyn MessageSink,
    ) {
        if msg.first() == Some(&PING) {
            let _ = out.queue(client, &PONG);
        }
    }
}

/// The game's character writer: a client's character is read from the
/// running game, laid over its seat's base, written in D2RUST's character
/// format and wrapped in the server's envelope at the seat's file.
pub(super) struct ServerStore {
    pub seats: Rc<RefCell<Seats>>,
    pub tables: Arc<dyn SaveTables + Send + Sync>,
    pub appearance: Option<Arc<AppearanceTables>>,
    pub events: mpsc::Sender<GameEvent>,
    pub game: u32,
}

impl CharacterStore<WorldSim<LocalSeams>, World> for ServerStore {
    fn save(&mut self, sim: &mut Sim, client: ClientId) -> Result<(), String> {
        let Some((seat, base)) = self
            .seats
            .borrow()
            .get(client)
            .map(|s| (s.seat.clone(), s.base.clone()))
        else {
            return Ok(());
        };
        let Some(player) = sim.player_of(client) else {
            return Ok(());
        };
        let live = character_save::read_live(sim, player);
        if base.header.version < d2s::VERSION
            && character_save::old_items_pass_through(&base, &live)
        {
            return Err(format!(
                "{}: a version {:#x} character whose items the game cannot rewrite is not overwritten",
                seat.character, base.header.version
            ));
        }
        let now = character_save::now_secs();
        let mut save = character_save::apply_live(&base, &live, now);
        if let Some(t) = &self.appearance {
            character_save::rebuild_appearance(sim, player, t, &mut save);
        }
        let record = D2sCodec.write(&save, &*self.tables)?;
        write_record(&seat.file, &record)
            .map_err(|e| format!("{}: writing {}: {e}", seat.character, seat.file.display()))?;
        let summary = summary(&seat.character, &save, now);
        // The file exists now: the next load reads it.
        if let Some(s) = self.seats.borrow_mut().by_client.get_mut(&client) {
            s.seat.new = None;
        }
        let _ = self.events.blocking_send(GameEvent::Saved {
            game: self.game,
            account: seat.account,
            summary,
        });
        Ok(())
    }
}

/// What character select and the lobby read of a saved character.
pub fn summary(name: &str, save: &D2s, now: u32) -> CharSummary {
    let h = &save.header;
    let mut look = h.components.to_vec();
    look.extend_from_slice(&h.colours);
    CharSummary {
        name: name.to_owned(),
        class: h.class,
        level: h.level,
        status: h.status,
        difficulties: d2s::status::difficulties(h.status),
        look,
        last_played: u64::from(now),
    }
}

/// Writes `record` in the server's character envelope to `path`: a
/// temporary file next to it, flushed to disk, then renamed over it.
pub fn write_record(path: &Path, record: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&crate::characters::encode(record))?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}
