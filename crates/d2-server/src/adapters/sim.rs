//! [`Intents`] and [`Tick`] on `d2_sim::game::Game`: [`SimGame`] is the
//! one game the server runs. Intents from the transport are gated, parsed
//! and dispatched to the [`handlers`]; a tick runs `d2-sim`'s steps through
//! the host and then queues what the sim sent to the receivers' clients.
//! The client list order is the sim's (`UnitLists::clients`, newest first).
//!
//! What the seams read that `d2-sim` does not hold (unit mode, state 54,
//! player data, positions, owners, acts of units outside rooms) the caller
//! stages here ([`PlayerFields`], [`UnitFacts`]); nothing here derives or
//! defaults them.
//!
//! The game's systems beyond the event dispatch (world, items, skills)
//! are one host value `W` ([`WorldHost`]): the intent handlers
//! ([`handlers`]) reach their `d2-sim` providers through it, and the
//! messages its seams send, from a handler or from a tick, go to the
//! receivers' clients.

use std::collections::BTreeMap;

use d2_proto::envelope::{self, Direction};
use d2_sim::game::Game;
use d2_sim::rules::{ModOutbox, Ruleset};
use d2_sim::tick::timer::TimerRun;
use d2_sim::tick::{EventDispatch, TickHooks};
use d2_sim::units::{ClientId as SimClient, RoomId, UnitId, UnitType};
use d2_sim::world::quests::HostRequest;

use super::handlers;
use super::handlers::player::{HotKey, HOTKEY_SLOTS};
use super::handlers::world::{self as world_handlers, NoWorld, WorldError, WorldHost};
use super::handlers::world::{ActionEvents, Outbox};
use super::session_flow::{SessionFlow, SessionRunner};
use super::storage::{CharacterStore, SaveFault};
use crate::seams::{
    ClientId, Intents, MessageSink, PlayerGate, PlayerLookup, PointState, Pos, ResultCode, Tick,
    UnitTarget,
};

/// Player data as far as the point parser reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerData {
    /// Frame of the last accepted point target.
    pub last_accept: i32,
}

/// The player fields of a player unit the dispatch gate and the point
/// parser read. Staged by the caller until `d2-sim` holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerFields {
    /// What the dispatch gate reads of the player.
    pub gate: PlayerGate,
    /// `None`: the player has no player data (point messages → 2).
    pub data: Option<PlayerData>,
}

/// Unit fields the unit-target lookup reads. Staged by the caller until
/// `d2-sim` holds them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitFacts {
    /// The unit's act.
    pub act: u8,
    /// Position (dynamic path, or static path for unit types 2, 4, 5).
    pub pos: Pos,
    /// For items: the player unit that owns it.
    pub owner: Option<UnitId>,
}

/// Lets the host's seams read the game before the sim runs on it
/// ([`SimGame::set_host_sync`]): called before each handled intent and at
/// the start of each tick.
pub type HostSync<D> = fn(&Game, &mut D);

/// [`HostSync`] with the host's world as well ([`SimGame::set_world_sync`]).
pub type WorldSync<D, W> = fn(&Game, &mut D, &mut W);

/// Misuse of the adapter's client bookkeeping.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AdapterError {
    /// The transport client already has a record.
    #[error("client {0} already joined")]
    AlreadyJoined(ClientId),
    /// The transport client has no record.
    #[error("client {0} is not in the game")]
    NotJoined(ClientId),
    /// The unit lists refused the change.
    #[error(transparent)]
    List(#[from] d2_sim::units::ListError),
}

/// Timer-event dispatch that runs nothing: the default of a game with no
/// event dispatch of its own.
#[derive(Clone, Copy, Debug, Default)]
pub struct Unspecified;

impl EventDispatch for Unspecified {
    fn run_event(&mut self, _game: &mut Game, _run: &TimerRun) {}
}

/// Every step body keeps its `TickHooks` default (does nothing).
impl TickHooks for Unspecified {}

/// One `d2-sim` game behind the `d2-server` seams.
///
/// Transport client ids (`d2-server`) and client records (`d2-sim` slots)
/// are mapped by [`SimGame::join`]. Timer events go to `events`; the
/// game's world, item and skill systems are the host `world`.
pub struct SimGame<D = Unspecified, W = NoWorld> {
    /// The `d2-sim` game.
    pub game: Game,
    /// The timer-event dispatch.
    pub events: D,
    clients: BTreeMap<ClientId, SimClient>,
    transport_ids: BTreeMap<SimClient, ClientId>,
    players: BTreeMap<UnitId, PlayerFields>,
    units: BTreeMap<UnitId, UnitFacts>,
    /// The host's seam refresh, if set.
    host_sync: Option<HostSync<D>>,
    /// The host's seam refresh that reads the world too, if set.
    world_sync: Option<WorldSync<D, W>>,
    /// Clients the point parser asked to resync with S→C 0x15, in order
    /// (the player is queued for update as well, `queue_resync`).
    pub resyncs: Vec<ClientId>,
    /// Intents that passed the gate, size check and parse, in order:
    /// (client, id, size). Their handlers are not written (see `handle`).
    pub unhandled: Vec<(ClientId, u8, usize)>,
    /// The game's systems for the intent handlers ([`handlers`]: world,
    /// cube, skills) and the outbox of their seams.
    pub world: W,
    /// Messages sent during a tick that could not be queued, in order.
    pub tick_faults: Vec<(ClientId, WorldError)>,
    /// Whether the update pass announces ground items at all (the play
    /// host turns it on; the unit hosts keep the pass of the players'
    /// update lists only).
    pub announce_ground: bool,
    /// The 0x23 event records a weapon switch queued, sent after the
    /// player's 0x47 / 0x48 of the next update pass.
    pub(crate) switch_events: Vec<(ClientId, Vec<u8>)>,
    /// Ground items already announced (S→C 0x9C) to a client by the item
    /// update pass (`handlers::items::moves::update_pass`). The game keeps
    /// no per-client set: "not yet announced" is the unit's flag 0x10,
    /// cleared by the room clean-up. The room join reads that flag (the sim
    /// sends its item mark only once the flag is clear, answered with 0x9C
    /// action 3 / 2 by `join_marked`), and its answer is recorded here; the
    /// set only keeps the update pass's own fallback
    /// announcement from repeating.
    pub(crate) announced_ground: std::collections::BTreeSet<(ClientId, UnitId)>,
    /// The ground items announced inside this tick's queue walk
    /// (`handlers::items::moves::announce_marked`), for the pass's clean-up.
    pub(crate) walk_announced: Vec<(ClientId, UnitId)>,
    /// The (client, player) pairs whose item messages / sound went out at
    /// their place in this tick's queue walk
    /// (`handlers::items::moves::player_marked`); the pass skips them.
    pub(crate) walk_player_items: Vec<(ClientId, UnitId)>,
    pub(crate) walk_player_sound: Vec<(ClientId, UnitId)>,
    /// The room delete notices (S→C 0x0A, type 4) an item-move handler
    /// queued (the handler only prepends a record to the room's delete
    /// list); the next tick sends them before its item messages.
    pub(crate) deferred_room_deletes: Vec<(ClientId, Vec<u8>)>,
    /// The host calls the quest rules raised (game end, save pass),
    /// drained from the world after each tick's steps, in call order.
    /// Running them is the session layer's (client removal, the save): it
    /// takes them with [`SimGame::take_host_requests`].
    pub host_requests: Vec<HostRequest>,
    /// The hot-key slots of each client (16 slots of 8 bytes;
    /// written by C→S 0x51; read by the join's S→C 0x7B).
    hotkeys: BTreeMap<ClientId, [HotKey; HOTKEY_SLOTS]>,
    /// The session sequence of C→S 0x67 / 0x6B
    /// ([`super::session_flow`]); `None`: every system message goes to the
    /// host's `SessionHandler`.
    session: Option<Box<dyn SessionRunner<D, W>>>,
    /// The host's character writer ([`super::storage`]); `None`: saves
    /// are recorded as [`SaveFault::NoStorage`].
    storage: Option<Box<dyn CharacterStore<D, W>>>,
    /// The periodic saves (every 8192 frames) that wrote nothing, in
    /// order (the leave's go to the session faults).
    pub save_faults: Vec<(ClientId, SaveFault)>,
    /// The rules the game runs under. A mod game's rules take its mod
    /// messages ([`Intents::mod_message`]); D2RUST's own take none.
    pub rules: Ruleset,
}

/// [`SimGame`]'s fields borrowed apart (for a handler).
pub struct SimParts<'s, D, W> {
    /// The `d2-sim` game.
    pub game: &'s mut Game,
    /// The timer-event dispatch.
    pub events: &'s mut D,
    /// The host's world.
    pub world: &'s mut W,
    /// The staged unit facts (act, position, owner).
    pub facts: &'s BTreeMap<UnitId, UnitFacts>,
}

impl SimGame<Unspecified> {
    /// A game with no event dispatch and no world host.
    pub fn new(game: Game) -> Self {
        Self::with_events(game, Unspecified)
    }
}

impl<D: EventDispatch, W> SimGame<D, W> {
    /// A game with the given event dispatch and a default world host.
    pub fn with_events(game: Game, events: D) -> Self
    where
        W: Default,
    {
        Self::with_world(game, events, W::default())
    }

    /// A game with the given world host (one without a `Default`, e.g.
    /// `handlers::world::WiredWorld`, built from the game's tables).
    pub fn with_world(game: Game, events: D, world: W) -> Self {
        Self {
            game,
            events,
            clients: BTreeMap::new(),
            transport_ids: BTreeMap::new(),
            players: BTreeMap::new(),
            units: BTreeMap::new(),
            host_sync: None,
            world_sync: None,
            resyncs: Vec::new(),
            unhandled: Vec::new(),
            world,
            tick_faults: Vec::new(),
            announce_ground: false,
            walk_announced: Vec::new(),
            walk_player_items: Vec::new(),
            walk_player_sound: Vec::new(),
            deferred_room_deletes: Vec::new(),
            switch_events: Vec::new(),
            announced_ground: std::collections::BTreeSet::new(),
            host_requests: Vec::new(),
            hotkeys: BTreeMap::new(),
            session: None,
            storage: None,
            save_faults: Vec::new(),
            rules: Ruleset::Original,
        }
    }

    /// Installs the character writer the saves run ([`super::storage`]).
    pub fn set_storage(&mut self, store: Box<dyn CharacterStore<D, W>>) {
        self.storage = Some(store);
    }

    /// Saves the character of every client with a player, in client-list
    /// order. Returns each such client with its save's result.
    pub fn save_characters(&mut self) -> Vec<(ClientId, Result<(), SaveFault>)> {
        // A save write can stall a frame; the perf report names it.
        let _perf = crate::perf::span("save");
        let mut store = self.storage.take();
        let mut out = Vec::new();
        for c in self.client_list() {
            if self.player_of(c).is_none() {
                continue;
            }
            let r = match store.as_mut() {
                Some(s) => s.save(self, c).map_err(SaveFault::Failed),
                None => Err(SaveFault::NoStorage),
            };
            out.push((c, r));
        }
        // A store the save installed wins over the one it ran with.
        if self.storage.is_none() {
            self.storage = store;
        }
        out
    }

    /// The quests' host requests drained so far ([`SimGame::host_requests`]),
    /// in call order, for the session layer to run.
    pub fn take_host_requests(&mut self) -> Vec<HostRequest> {
        std::mem::take(&mut self.host_requests)
    }

    /// Runs C→S 0x67 / 0x6B through `flow` from now on.
    pub fn set_session(&mut self, flow: SessionFlow<D, W>)
    where
        D: ActionEvents + 'static,
        D::X: Outbox,
        W: 'static + super::session::GolemWorld,
    {
        self.session = Some(Box::new(flow));
    }

    /// The session flow, if one is set.
    pub fn session(&self) -> Option<&SessionFlow<D, W>> {
        self.session.as_deref().map(|r| r.flow())
    }

    /// Adds a client record for transport client `client`
    /// (`UnitLists::add_client`: prepended).
    pub fn join(
        &mut self,
        client: ClientId,
        player: Option<UnitId>,
        room: Option<RoomId>,
        state: u32,
    ) -> Result<SimClient, AdapterError> {
        if self.clients.contains_key(&client) {
            return Err(AdapterError::AlreadyJoined(client));
        }
        let id = self.game.lists.add_client(player, room, state);
        self.clients.insert(client, id);
        self.transport_ids.insert(id, client);
        Ok(id)
    }

    /// Removes the client's record (`UnitLists::remove_client`).
    pub fn leave(&mut self, client: ClientId) -> Result<(), AdapterError> {
        let id = self
            .clients
            .remove(&client)
            .ok_or(AdapterError::NotJoined(client))?;
        self.transport_ids.remove(&id);
        self.hotkeys.remove(&client);
        self.game.lists.remove_client(id)?;
        Ok(())
    }

    /// The transport clients with a record, in client-list order.
    pub fn client_list(&self) -> Vec<ClientId> {
        self.game
            .lists
            .clients()
            .into_iter()
            .filter_map(|id| self.transport_ids.get(&id).copied())
            .collect()
    }

    /// The `d2-sim` client record of a transport client.
    pub fn sim_client(&self, client: ClientId) -> Option<SimClient> {
        self.clients.get(&client).copied()
    }

    /// Stages a player unit's gate fields and player data.
    pub fn set_player(&mut self, unit: UnitId, fields: PlayerFields) {
        self.players.insert(unit, fields);
    }

    /// The staged player fields of a unit.
    pub fn player_fields(&self, unit: UnitId) -> Option<&PlayerFields> {
        self.players.get(&unit)
    }

    /// Stages a unit's act, position and owner.
    pub fn set_unit(&mut self, unit: UnitId, facts: UnitFacts) {
        self.units.insert(unit, facts);
    }

    /// Runs `sync` before each handled intent and at the start of each
    /// tick (a host whose seams answer from a copy of the game's units).
    pub fn set_host_sync(&mut self, sync: HostSync<D>) {
        self.host_sync = Some(sync);
    }

    /// Like [`SimGame::set_host_sync`], for a host whose seams answer
    /// from the game's world (the inventory model): called at the same
    /// points, after the host sync.
    pub fn set_world_sync(&mut self, sync: WorldSync<D, W>) {
        self.world_sync = Some(sync);
    }

    fn run_host_sync(&mut self) {
        if let Some(f) = self.host_sync {
            f(&self.game, &mut self.events);
        }
        if let Some(f) = self.world_sync {
            f(&self.game, &mut self.events, &mut self.world);
        }
    }

    /// The client's player unit, if it is a player (unit type 0).
    fn player_unit(&self, client: ClientId) -> Option<UnitId> {
        let id = self.clients.get(&client)?;
        let unit = self.game.lists.client(*id)?.player?;
        (self.game.lists.unit(unit)?.ty == UnitType::Player).then_some(unit)
    }

    /// The client's hot-key slots (a slot never bound is
    /// [`HotKey::UNBOUND`]).
    pub fn hotkeys(&self, client: ClientId) -> [HotKey; HOTKEY_SLOTS] {
        self.hotkeys
            .get(&client)
            .copied()
            .unwrap_or([HotKey::UNBOUND; HOTKEY_SLOTS])
    }

    /// Stores one hot-key slot; `slot` < 16.
    pub fn set_hotkey(&mut self, client: ClientId, slot: usize, key: HotKey) {
        if slot < HOTKEY_SLOTS {
            self.hotkeys
                .entry(client)
                .or_insert([HotKey::UNBOUND; HOTKEY_SLOTS])[slot] = key;
        }
    }

    /// The staged position of the client's player ([`UnitFacts`]).
    pub fn player_pos(&self, client: ClientId) -> Option<Pos> {
        let unit = self.player_unit(client)?;
        Some(self.units.get(&unit)?.pos)
    }

    /// The client's player unit (unit type 0), for the handlers.
    pub fn player_of(&self, client: ClientId) -> Option<UnitId> {
        self.player_unit(client)
    }

    /// The fields a handler works on, borrowed apart.
    pub fn parts(&mut self) -> SimParts<'_, D, W> {
        SimParts {
            game: &mut self.game,
            events: &mut self.events,
            world: &mut self.world,
            facts: &self.units,
        }
    }

    /// The transport client whose player is `unit` (`None`: a player
    /// without a client).
    pub fn client_of(&self, unit: UnitId) -> Option<ClientId> {
        self.game
            .lists
            .clients()
            .into_iter()
            .find(|&id| {
                self.game
                    .lists
                    .client(id)
                    .is_some_and(|r| r.player == Some(unit))
            })
            .and_then(|id| self.transport_ids.get(&id).copied())
    }
}

impl<D: ActionEvents, W: super::session::GolemWorld> SimGame<D, W> {
    /// Sends `client` its view again after a resume (contracts 3.7): the
    /// rooms' 0x07 and every visible unit's add messages, its own add
    /// messages with its current position, and its player update. The
    /// player unit, its position and the game state stay as they are.
    pub fn reenter_client(&mut self, client: ClientId) {
        let Some(id) = self.sim_client(client) else {
            return;
        };
        let game = &mut self.game;
        self.events
            .action()
            .with(game, |g, v| v.resend_client(g, id));
    }

    /// Removes a leaving player from the game: its belongings in the world
    /// (items, mercenary: [`super::session::GolemWorld::free_player_belongings`]),
    /// its summons, its town portal, then the player unit itself. Each
    /// unit leaves its room first, so the other clients near it are told
    /// at the next tick. The player's staged fields and its items' facts
    /// go too. Returns one text per fault.
    pub fn remove_player(&mut self, player: UnitId) -> Vec<String> {
        let faults = self
            .world
            .free_player_belongings(&mut self.game, &mut self.events, player);
        let pets: Vec<u32> = self
            .events
            .action()
            .hooks()
            .pet_lists
            .remove(&player)
            .map(|l| {
                l.entries
                    .into_iter()
                    .flat_map(|e| e.nodes)
                    .filter_map(|n| u32::try_from(n.guid).ok())
                    .collect()
            })
            .unwrap_or_default();
        let game = &mut self.game;
        self.events.lend_world(|a| {
            a.with(game, |g, v| {
                v.close_player_portal(g, player);
                for guid in pets {
                    if let Some(u) = g.lists.find_unit(UnitType::Monster, guid) {
                        v.leave_room(g, u, true);
                        v.remove(g, u);
                    }
                }
                v.leave_room(g, player, true);
                v.remove(g, player);
            })
        });
        self.players.remove(&player);
        self.units
            .retain(|&u, f| u != player && f.owner != Some(player));
        faults
    }
}

/// [`ModOutbox`] of one mod message: each payload is queued as an S→C
/// envelope for the client of the named player.
struct ModSends<'a> {
    /// Player unit to transport client, as the message arrived.
    clients: BTreeMap<UnitId, ClientId>,
    out: &'a mut dyn MessageSink,
}

impl ModOutbox for ModSends<'_> {
    fn send(&mut self, player: UnitId, payload: &[u8]) -> bool {
        self.clients.get(&player).is_some_and(|&client| {
            envelope::encode(Direction::ToClient, payload)
                .is_ok_and(|msg| self.out.queue(client, &msg).is_ok())
        })
    }
}

impl<D: EventDispatch, W: WorldHost<D>> Intents for SimGame<D, W> {
    /// Not joined (or record gone) → not in game; no player unit, not a
    /// player, or no staged [`PlayerFields`] → no player.
    fn player(&self, client: ClientId) -> PlayerLookup {
        let in_game = self
            .clients
            .get(&client)
            .is_some_and(|id| self.game.lists.client(*id).is_some());
        if !in_game {
            return PlayerLookup::NotInGame;
        }
        match self.player_unit(client).and_then(|u| self.players.get(&u)) {
            Some(p) => PlayerLookup::Player(p.gate),
            None => PlayerLookup::NoPlayer,
        }
    }

    fn frame(&self) -> i32 {
        self.game.frame
    }

    /// The mod game's rules ([`ModRules::message`]) take the payload from
    /// the client's player; what they send goes out as S→C envelopes to
    /// the clients of the players they name.
    fn mod_message(&mut self, client: ClientId, payload: &[u8], out: &mut dyn MessageSink) -> bool {
        let Ruleset::Mod(rules) = &self.rules else {
            return false;
        };
        let Some(player) = self.player_unit(client) else {
            return false;
        };
        let rules = rules.clone();
        let clients = self
            .game
            .lists
            .clients()
            .into_iter()
            .filter_map(|id| {
                let unit = self.game.lists.client(id)?.player?;
                Some((unit, *self.transport_ids.get(&id)?))
            })
            .collect();
        let mut outbox = ModSends { clients, out };
        rules.message(&mut self.game, player, payload, &mut outbox);
        true
    }

    /// The host world's object host tick ([`WorldHost::host_tick`]).
    fn set_host_tick(&mut self, ms: u32) {
        self.world.host_tick(&mut self.events, ms);
    }

    /// `None` without player data, or without a staged position.
    /// The facts of the client's player and, for a unit message, of the
    /// target, from the world host when the caller staged none (the app
    /// stages none: without this every walk was refused `Invalid`).
    fn refresh_targets(&mut self, client: ClientId, msg: &[u8]) {
        let mut units: Vec<UnitId> = self.player_unit(client).into_iter().collect();
        if let (Some(&id), Some(ty), Some(guid)) = (
            msg.first(),
            msg.get(1..5)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
            msg.get(5..9)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
        ) {
            if crate::dispatch::is_unit(id) {
                if let Some(&t) = UnitType::ALL.get(ty as usize) {
                    units.extend(self.game.lists.find_unit(t, guid));
                }
            }
        }
        // The world's facts win over a staged copy whenever the world has
        // the unit: a staged copy is only moved by the tick
        // (`unit_positions`), so between ticks (the join's (0, 0) before
        // the first tick, a warp earlier in the same frame, an act
        // change) it is stale, the range test of the point parser refuses
        // in-range walks and after 25 frames S→C 0x15 snaps the player
        // back (q-proto-audit, rubber-banding). A staged owner the world
        // does not report is kept.
        for u in units {
            if let Some(mut f) = self.world.live_facts(&self.game, &mut self.events, u) {
                if f.owner.is_none() {
                    f.owner = self.units.get(&u).and_then(|s| s.owner);
                }
                self.units.insert(u, f);
            }
        }
    }

    fn point_state(&self, client: ClientId) -> Option<PointState> {
        let unit = self.player_unit(client)?;
        let data = self.players.get(&unit)?.data?;
        let player = self.units.get(&unit)?.pos;
        Some(PointState {
            player,
            last_accept: data.last_accept,
        })
    }

    fn set_point_accept(&mut self, client: ClientId, frame: i32) {
        let data = self
            .player_unit(client)
            .and_then(|u| self.players.get_mut(&u))
            .and_then(|p| p.data.as_mut());
        if let Some(d) = data {
            d.last_accept = frame;
        }
    }

    /// Records the request and queues the player for the update pass
    /// that sends S→C 0x15 ([`handlers::player::resync`]).
    fn queue_resync(&mut self, client: ClientId, _out: &mut dyn MessageSink) {
        self.resyncs.push(client);
        handlers::player::resync(self, client);
    }

    /// The unit-target lookup, in this order: missing → owned item → other
    /// act → positions. A target or player without staged [`UnitFacts`] counts
    /// as missing.
    fn unit_target(&self, client: ClientId, unit_type: u32, unit_id: u32) -> UnitTarget {
        let Some(&ty) = UnitType::ALL.get(unit_type as usize) else {
            return UnitTarget::Missing;
        };
        let Some(target) = self.game.lists.find_unit(ty, unit_id) else {
            return UnitTarget::Missing;
        };
        let Some(t) = self.units.get(&target) else {
            return UnitTarget::Missing;
        };
        let player = self.player_unit(client);
        if ty == UnitType::Item && player.is_some() && t.owner == player {
            return UnitTarget::OwnedItem;
        }
        let Some(p) = player.and_then(|u| self.units.get(&u)) else {
            return UnitTarget::Missing;
        };
        if p.act != t.act {
            return UnitTarget::OtherAct;
        }
        UnitTarget::At {
            player: p.pos,
            target: t.pos,
        }
    }

    /// Per-intent behaviour belongs to the systems (movement, skills,
    /// items, NPCs, quests). Ids a [`handlers`] module owns run there when
    /// the host provides their system (the skill messages' `pierce_idx`
    /// += 1 is `use_::handle_message`'s); every other handler is a stub: it
    /// records the intent and returns 0, the "does nothing" result.
    fn handle(
        &mut self,
        client: ClientId,
        msg: &[u8],
        size: usize,
        out: &mut dyn MessageSink,
    ) -> ResultCode {
        self.run_host_sync();
        if let Some(r) = handlers::items::handle(self, client, msg, out) {
            return r;
        }
        if let Some(r) = handlers::items::moves::handle(self, client, msg, size, out) {
            return r;
        }
        if let Some(code) = world_handlers::handle(self, client, msg, size, out) {
            return code;
        }
        if let Some(code) = super::handlers::skills::handle(self, client, msg, out) {
            return code;
        }
        if let Some(code) = handlers::walk::handle(self, client, msg, out) {
            return code;
        }
        if let Some(code) = handlers::player::handle(self, client, msg, size, out) {
            return code;
        }
        self.unhandled.push((client, msg[0], size));
        ResultCode::Done
    }

    /// `UnitLists::clients` (newest first), as transport ids.
    fn clients(&self) -> Vec<ClientId> {
        self.game
            .lists
            .clients()
            .into_iter()
            .filter_map(|id| self.transport_ids.get(&id).copied())
            .collect()
    }

    /// The game's session flow, when set ([`SimGame::set_session`]).
    fn session_message(
        &mut self,
        client: ClientId,
        msg: &[u8],
        _size: usize,
        out: &mut dyn MessageSink,
    ) -> bool {
        let Some(mut flow) = self.session.take() else {
            return false;
        };
        let handled = flow.run(self, client, msg, out);
        self.session = Some(flow);
        if handled {
            self.world.session_work(&mut self.game, &mut self.events);
        }
        handled
    }

    /// The session flow's [`SessionFlow::gone`]; without a flow the
    /// client's record alone goes.
    fn client_gone(&mut self, client: ClientId, out: &mut dyn MessageSink) {
        match self.session.take() {
            Some(mut flow) => {
                flow.gone(self, client, out);
                self.session = Some(flow);
                self.world.session_work(&mut self.game, &mut self.events);
            }
            None => {
                let _ = self.leave(client);
            }
        }
    }
}

impl<D: EventDispatch + TickHooks, W: WorldHost<D>> Tick for SimGame<D, W> {
    /// The rate the game was made with ([`Game::tick_rate`]).
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.game.tick_rate
    }

    /// `d2_sim::tick::tick` with `D` as the step hooks, through
    /// [`WorldHost::run_tick_through_timers`] and
    /// [`WorldHost::run_tick_from_client_pass`] (the wired dispatch's room,
    /// DRLG and population steps run; a
    /// dispatch without them keeps the defaults), with the 8192-frame
    /// character save between them, then the host's
    /// [`WorldHost::after_tick`]; the quests' host requests are drained
    /// into [`SimGame::host_requests`] (after the frame's quest step, in
    /// call order). What the host's seams
    /// sent during the tick ([`WorldHost::take_sent`]) is queued to the
    /// receivers' clients in send order (a player without a client
    /// receives nothing); a queueing failure is recorded in
    /// [`SimGame::tick_faults`]. Then the deferred item messages
    /// (`handlers::items::moves::update_pass`).
    /// The client vitals sync runs in the flush ([`Tick::flush_sync`]).
    fn tick(&mut self, out: &mut dyn MessageSink) {
        {
            let _span = crate::perf::span("host.sync");
            self.run_host_sync();
        }
        self.world
            .run_tick_through_timers(&mut self.game, &mut self.events);
        // Step 5's head: at frame % 8192 = 0 every client's character is
        // saved before the per-client loop, so the file holds the state
        // after steps 0-4. No heartbeat drop is modelled.
        if d2_sim::tick::is_due(self.game.frame, d2_sim::tick::period::CHARACTER_SAVE) {
            for (c, r) in self.save_characters() {
                if let Err(e) = r {
                    self.save_faults.push((c, e));
                }
            }
        }
        self.world
            .run_tick_from_client_pass(&mut self.game, &mut self.events);
        // The client pass raised the save it is due for; it ran above.
        self.game.character_save_due = false;
        self.world.after_tick(&mut self.game, &mut self.events);
        let _span = crate::perf::span("host.emission");
        // The staged positions follow the path records, so the point
        // parser sees where the walking player is.
        let staged: Vec<UnitId> = self.units.keys().copied().collect();
        for (unit, (x, y)) in self.world.unit_positions(&mut self.events, &staged) {
            if let Some(f) = self.units.get_mut(&unit) {
                f.pos = Pos { x, y };
            }
        }
        // So does their act: a cross-act warp moves a staged unit into
        // the new act's rooms; the position and owner stay the caller's.
        for u in staged {
            if let Some(live) = self.world.live_facts(&self.game, &mut self.events, u) {
                if let Some(f) = self.units.get_mut(&u) {
                    f.act = live.act;
                }
            }
        }
        // The gate reads the unit's live mode (a dead player's 0x41).
        let staged: Vec<UnitId> = self.players.keys().copied().collect();
        for u in staged {
            if let Some(g) = self.world.player_gate(&self.game, &mut self.events, u) {
                if let Some(p) = self.players.get_mut(&u) {
                    p.gate = g;
                }
            }
        }
        let requests = self.world.take_host_requests();
        self.host_requests.extend(requests);
        let sent = self.world.take_sent(&mut self.events);
        for (c, bytes) in std::mem::take(&mut self.deferred_room_deletes) {
            // The room's own removal already sends the notice for a unit
            // the tick unlinks (a freed ground scroll): one per client.
            let again = sent
                .iter()
                .any(|(u, b)| *b == bytes && self.client_of(*u) == Some(c));
            if !again {
                if let Err(e) = out.queue(c, &bytes) {
                    self.tick_faults.push((c, WorldError::from(e)));
                }
            }
        }
        for (unit, bytes) in sent {
            self.send_or_mark(out, unit, bytes);
        }
        // The trade open's store items (0x9C action 11): the client pass,
        // after the monster updates.
        for (unit, bytes) in self.world.take_client_pass_sent() {
            if let Some(c) = self.client_of(unit) {
                if let Err(e) = out.queue(c, &bytes) {
                    self.tick_faults.push((c, WorldError::from(e)));
                }
            }
        }
        handlers::items::moves::update_pass(self, out);
        // The player's step 5 / 7 sends follow its item messages.
        self.world.flush_player_tail(&mut self.events);
        for (unit, bytes) in self.world.take_sent(&mut self.events) {
            self.send_or_mark(out, unit, bytes);
        }
        if let Some(mut flow) = self.session.take() {
            flow.finish_views(self, out);
            self.session = Some(flow);
        }
    }

    /// The client vitals sync (`SimGame::vitals_sync`) of the flush.
    fn flush_sync(&mut self, out: &mut dyn MessageSink) {
        self.vitals_sync(out);
    }
}

impl<D: EventDispatch, W: WorldHost<D>> SimGame<D, W> {
    /// One message the sim sent: a queue-walk mark becomes the real
    /// messages at its place, anything else is queued to the unit's client.
    pub(crate) fn send_or_mark(&mut self, out: &mut dyn MessageSink, unit: UnitId, bytes: Vec<u8>) {
        // A ground-item mark of the queue walk: the item's 0x9C here.
        if bytes.len() == 5 && bytes[0] == d2_sim::wiring::action::GROUND_ITEM_MARK {
            let guid = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            handlers::items::moves::announce_marked(self, out, unit, guid);
            return;
        }
        // The store items' place in the queue walk (trade open).
        if bytes.len() == 5 && bytes[0] == d2_sim::wiring::action::STORE_ITEMS_MARK {
            for (u, b) in self.world.take_client_pass_sent() {
                if let Some(c) = self.client_of(u) {
                    if let Err(e) = out.queue(c, &b) {
                        self.tick_faults.push((c, WorldError::from(e)));
                    }
                }
            }
            return;
        }
        // A room join's item add: the ground update.
        if bytes.len() == 5 && bytes[0] == d2_sim::wiring::action::JOIN_ITEM_MARK {
            let guid = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            handlers::items::moves::join_marked(self, out, unit, guid);
            return;
        }
        // An armor piece broken by a hit: the inventory model breaks it
        // here, at the place of the hit.
        if bytes.len() == 5 && bytes[0] == d2_sim::wiring::action::ITEM_BREAK_MARK {
            let guid = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            handlers::items::moves::break_marked(self, unit, guid);
            return;
        }
        // A level-up: the item relink and the
        // inventory pass run on the inventory model here.
        if bytes.len() == 5 && bytes[0] == d2_sim::wiring::action::reaction::LEVEL_UP_ITEMS_MARK {
            let guid = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            handlers::items::moves::level_up_marked(self, guid);
            return;
        }
        if bytes.len() == 5
            && bytes[0] == d2_sim::wiring::interaction::skill_use::EXHAUST_STACK_MARK
        {
            let guid = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            handlers::items::moves::exhaust_stack_marked(self, unit, guid);
            return;
        }
        // A key a locked chest or door took: used up from the player's
        // (`unit`) inventory here.
        if bytes.len() == 5 && bytes[0] == d2_sim::wiring::action::pending::KEY_USE_MARK {
            handlers::items::moves::key_use_marked(self, unit);
            return;
        }
        // A restored inactive item's child: linked into the parent (`unit`)
        // on the inventory model here.
        if bytes.len() == 5 && bytes[0] == d2_sim::wiring::action::pending::SOCKET_RESTORE_MARK {
            let guid = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            handlers::items::moves::socket_restore_marked(self, unit, guid);
            return;
        }
        // A hireling's item-update mark of the queue walk (step 5).
        if bytes.len() == 5 && bytes[0] == d2_sim::wiring::action::MERC_ITEMS_MARK {
            let guid = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            handlers::items::moves::merc_marked(self, out, unit, guid);
            return;
        }
        // A player-update mark of the queue walk: the player's item
        // messages or sound here.
        if bytes.len() == 5
            && matches!(
                bytes[0],
                d2_sim::wiring::action::PLAYER_ITEMS_MARK
                    | d2_sim::wiring::action::PLAYER_SOUND_MARK
            )
        {
            let guid = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
            let sound = bytes[0] == d2_sim::wiring::action::PLAYER_SOUND_MARK;
            handlers::items::moves::player_marked(self, out, unit, guid, sound);
            return;
        }
        if let Some(c) = self.client_of(unit) {
            if let Err(e) = out.queue(c, &bytes) {
                self.tick_faults.push((c, WorldError::from(e)));
            }
        }
    }
}

impl<D: EventDispatch + TickHooks, W: WorldHost<D>> SimGame<D, W> {
    /// The client vitals sync: every flush with argument 1 runs it for each
    /// client in game, before its buffers are sent, so these messages end
    /// the tick's batch; run by the host's flush ([`Tick::flush_sync`]), in
    /// client list order. A player
    /// without a path record is at its staged position ([`UnitFacts`];
    /// (0, 0) when none is staged). Off unless the host's world turns it
    /// on (`WorldHost::vitals_sync`).
    fn vitals_sync(&mut self, out: &mut dyn MessageSink) {
        for c in self.clients() {
            let Some(sc) = self.clients.get(&c).copied() else {
                continue;
            };
            let staged = self
                .game
                .lists
                .client(sc)
                .and_then(|r| r.player)
                .and_then(|p| self.units.get(&p))
                .map_or((0, 0), |f| (f.pos.x as u16, f.pos.y as u16));
            let queued = out.has_queued(c);
            let Some(msgs) =
                self.world
                    .vitals_sync(&mut self.game, &mut self.events, sc, staged, queued)
            else {
                continue;
            };
            for m in msgs {
                if let Err(e) = out.queue(c, &m) {
                    self.tick_faults.push((c, WorldError::from(e)));
                }
            }
        }
    }
}
