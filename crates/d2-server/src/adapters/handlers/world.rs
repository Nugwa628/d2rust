//! The world intent handlers: NPC interaction, vendors, waypoints, quests
//! and the C→S 0x13 object case. The server's message dispatcher calls
//! [`handle`] for every world-related id; the wired single-player host
//! ([`WiredWorld`]) is what the play loop runs them on.
//!
//! The dispatcher has run the gate and the exact-size check when
//! [`handle`] is called.
//! Every field check, refusal code and S→C message is the `d2-sim`
//! module's: a handler here finds the client's player unit, hands the
//! message to the module through the game's [`WorldHost`], queues what
//! the module sent to the clients of the receiving players, and maps
//! the module's result to a [`ResultCode`].
//!
//! [`WORLD_IDS`] lists every world-related C→S id with its owner. An id
//! with no owner, or whose system the host does not provide, stays a stub
//! ([`handle`] returns `None`).
//!
//! [`WorldHost`] is the one host trait of a game: besides the world
//! systems it carries the cube ([`WorldHost::cube`], `handlers::items`)
//! and the skill handlers ([`WorldHost::skill`], `handlers::skills`).
//! Hosts: [`NoWorld`] (nothing), [`ActionWorld`] (the systems on the
//! action wiring alone: waypoints, skills), [`WiredWorld`] (the wired
//! single-player host: those, plus the economy, NPCs, vendors, quests
//! and the cube on the same unit world).

mod action;
mod gap_items;
mod hireling_drive;
mod hireling_host;
mod item_approach;
mod item_save;
mod npc_approach;
mod object_approach;
mod wired;

#[cfg(test)]
pub(crate) mod tests;

pub use action::{ActionEvents, ActionWorld, Outbox, ProcessState};
pub use gap_items::HirelingBlock;
pub use item_save::LoadedItems;
pub use wired::{Parts, TradeRest, WiredWorld};

use d2_sim::game::Game;
use d2_sim::skills::use_::item_type::ItemTypeSkill;
use d2_sim::tick::EventDispatch;
use d2_sim::units::{ClientId as SimClient, UnitId};
use d2_sim::wiring::action::ObjectCase;
use d2_sim::world::npc::{NpcControl, NpcError, NpcVendors, NpcWorld};
use d2_sim::world::quests::{QuestControl, QuestError, QuestWorld};
use d2_sim::world::vendors::gamble::identify_gamble;
use d2_sim::world::vendors::price::PriceFatal;
use d2_sim::world::vendors::trade::{buy, repair, sell, BuyMsg, RepairMsg, SellMsg};
use d2_sim::world::vendors::{VendorRecord, VendorTables, VendorWorld};
use d2_sim::world::waypoints::{ArrivalList, WaypointData, WaypointError, WaypointWorld};

use super::super::character::{self, StartItemWorld, StartPlace};
use super::super::SimGame;
use super::items::moves::MoveCall;
use super::items::moves::{take_sent as inv_take_sent, InvParts, MoveRest, StagedPlace};
use super::items::CubeCall;
use super::player::{Outcome as PlayerOutcome, Run as PlayerRun};
use super::skills::{Call as SkillCall, Handled as SkillHandled};
use super::walk::{WalkCall, WalkResult};
use crate::buffers::QueueError;
use crate::seams::{ClientId, MessageSink, ResultCode};
use d2_sim::items::inventory::{InvTables, UnitKind};
use d2_sim::items::moves::{InventoryOps, MovePending, MoveUnits, Owner, Spot};
use d2_sim::items::ItemRequest;
use d2_sim::items::{flag, q, stat as istat, ItemStats, ListKey};
use d2_sim::units::lifecycle::LifecycleHooks;
use d2_sim::units::RoomId;
use d2_sim::wiring::economy::quest_reward::create_reward_flags;
use d2_sim::wiring::economy::ItemSpawn;
use d2_sim::wiring::economy::{find_list, Economy, StatCtx, UnitStats};
use d2_sim::wiring::inventory::InvRest;
use std::collections::BTreeMap;

/// Who handles a world-related C→S id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Handled here, by the named `d2-sim` system.
    Implemented(System),
    /// Owned by another handler module.
    OtherModule(&'static str),
    /// No owner (or not the part named): stub.
    NoOwner,
}

/// The `d2-sim` world system an id goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    /// NPC interaction.
    Npc,
    /// Vendor trade.
    Vendors,
    /// Waypoint use.
    Waypoints,
    /// Quest messages.
    Quests,
    /// The object case of 0x13 (unit type 2; [`route`]).
    Objects,
    /// The tile case of 0x13 (unit type 5, a level warp; [`route`]).
    /// Under 5 the warp, 5 to 50 a run and the warp on arrival, else
    /// nothing; the checks w3-01-tile-near / -walk / -far pin the frames.
    Warps,
}

/// Every world-related C→S id: (id, note on its owner, status). The
/// order is the id order. The note strings are free text for readers.
pub const WORLD_IDS: &[(u8, &str, Status)] = &[
    // Unit type 1 is the NPC case; type 2 is the object case
    // (`System::Objects`, chosen by `route`); other types have no owner: stubs.
    (0x13, "world/npc.md §2", Status::Implemented(System::Npc)),
    (
        0x2A,
        "world/cube.md (handlers/items.rs)",
        Status::OtherModule("items"),
    ),
    (0x2F, "world/npc.md §3", Status::Implemented(System::Npc)),
    (0x30, "world/npc.md §3", Status::Implemented(System::Npc)),
    (
        0x31,
        "world/quests.md §7.3",
        Status::Implemented(System::Quests),
    ),
    (
        0x32,
        "world/vendors.md §7.1",
        Status::Implemented(System::Vendors),
    ),
    (
        0x33,
        "world/vendors.md §7.2",
        Status::Implemented(System::Vendors),
    ),
    (0x34, "world/npc.md §6", Status::Implemented(System::Npc)),
    (
        0x35,
        "world/vendors.md §8.1",
        Status::Implemented(System::Vendors),
    ),
    (0x36, "world/npc.md §7.3", Status::Implemented(System::Npc)),
    (
        0x37,
        "world/vendors.md §5.5",
        Status::Implemented(System::Vendors),
    ),
    (0x38, "world/npc.md §4", Status::Implemented(System::Npc)),
    // The item checks have no defined result codes, and `read_clue` has no
    // item seam: stub.
    (0x3E, "world/quests.md §9.4 (partial)", Status::NoOwner),
    (
        0x3F,
        "sim/intents-events.md §9 r5 (handlers/player.rs)",
        Status::OtherModule("player"),
    ),
    (
        0x40,
        "world/quests.md §6.2",
        Status::Implemented(System::Quests),
    ),
    (
        0x44,
        "world/quests-act2.md §8.6 (entry: sim/intents-events.md §9 r7, handlers/player.rs)",
        Status::OtherModule("player"),
    ),
    (
        0x46,
        "sim/intents-events.md §9 r8 (handlers/player.rs)",
        Status::OtherModule("player"),
    ),
    (
        0x47,
        "sim/intents-events.md §9 r8 (handlers/player.rs)",
        Status::OtherModule("player"),
    ),
    (
        0x49,
        "world/waypoints.md §6",
        Status::Implemented(System::Waypoints),
    ),
    (
        0x4C,
        "world/cube.md §10: Transmogrify, owner the item-use spec (not written; which item the always-sent 0x3F names is not stated); handlers/items.rs takes only 0x2A and 0x4F",
        Status::NoOwner,
    ),
    (
        0x4D,
        "sim/intents-events.md §9 r11 (handlers/player.rs)",
        Status::OtherModule("player"),
    ),
    (
        0x4F,
        "world/cube.md (handlers/items.rs)",
        Status::OtherModule("items"),
    ),
    (
        0x58,
        "world/quests.md §1.7",
        Status::Implemented(System::Quests),
    ),
    // The Npc command entry is `NpcControl::make_entity_move`.
    (
        0x59,
        "monsters/ai-bodies.md §9.9",
        Status::Implemented(System::Npc),
    ),
    (0x62, "world/npc.md §7.4", Status::Implemented(System::Npc)),
];

/// The system that handles `id` here, if any.
pub fn system(id: u8) -> Option<System> {
    WORLD_IDS.iter().find_map(|&(i, _, s)| match s {
        Status::Implemented(sys) if i == id => Some(sys),
        _ => None,
    })
}

/// The system of one message: [`system`] of its id, except 0x13 of size
/// 9 with unit type 2 (u32 @1), the object case ([`System::Objects`]).
pub fn route(msg: &[u8]) -> Option<System> {
    let id = *msg.first()?;
    if id == 0x13 && msg.len() == 9 && msg[1..5] == 2u32.to_le_bytes() {
        return Some(System::Objects);
    }
    if id == 0x13 && msg.len() == 9 && msg[1..5] == 5u32.to_le_bytes() {
        return Some(System::Warps);
    }
    system(id)
}

/// A fatal condition (or a sink failure) met by a world handler. The
/// reference build ends the game; D2RUST records it with the host
/// ([`WorldHost::fault`]) and the handler returns [`ResultCode::Malformed`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorldError {
    /// A fatal condition of the NPC module.
    #[error(transparent)]
    Npc(#[from] NpcError),
    /// A fatal condition of the waypoint module.
    #[error(transparent)]
    Waypoint(#[from] WaypointError),
    /// A fatal condition of the quest module.
    #[error(transparent)]
    Quest(#[from] QuestError),
    /// A fatal price computation.
    #[error(transparent)]
    Price(#[from] PriceFatal),
    /// A fatal item move.
    #[error(transparent)]
    Move(#[from] d2_sim::items::moves::MoveFatal),
    /// The message sink refused a message.
    #[error("sink: {0}")]
    Sink(String),
}

impl From<QueueError> for WorldError {
    fn from(e: QueueError) -> Self {
        WorldError::Sink(e.to_string())
    }
}

/// A [`WorldError`] with the message that met it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldFault {
    /// The client whose message met the fault.
    pub client: ClientId,
    /// The message id.
    pub id: u8,
    /// What went wrong.
    pub error: WorldError,
}

/// One call into the NPC module with its seams (`NpcWorld + NpcVendors`).
pub trait NpcCall {
    /// What the call returns.
    type Out;
    /// Runs the call on the NPC module.
    fn call<W: NpcWorld + NpcVendors>(self, ctl: &mut NpcControl, w: &mut W) -> Self::Out;
}

/// One call into the vendors module. `records`: the vendor part of every
/// NPC record (one [`VendorRecord`] per NPC record, found by class).
pub trait VendorCall {
    /// What the call returns.
    type Out;
    /// Runs the call on the vendors module.
    fn call<W: VendorWorld>(
        self,
        tables: &VendorTables,
        records: &mut [VendorRecord],
        w: &mut W,
    ) -> Self::Out;
}

/// One call into the waypoint module (with the object control's arrival
/// list).
pub trait WaypointCall {
    /// What the call returns.
    type Out;
    /// Runs the call on the waypoint module.
    fn call<W: WaypointWorld>(
        self,
        data: &WaypointData,
        arrivals: &mut ArrivalList,
        w: &mut W,
    ) -> Self::Out;
}

/// One call into the quest module.
pub trait QuestCall {
    /// What the call returns.
    type Out;
    /// Runs the call on the quest module.
    fn call<W: QuestWorld>(self, ctl: &mut QuestControl, w: &mut W) -> Self::Out;
}

/// The systems of a game beyond its event dispatch and the providers of
/// their seams, as the handlers reach them: the world systems, the cube
/// and the skill handlers. `D` is the game's event dispatch (it owns the
/// unit side, e.g. `d2_sim::wiring::action::ActionSim`).
///
/// A method returning `None` means the game has no provider for that
/// system: the ids stay stubs. Every seam `send` must be kept in order
/// and handed back by [`WorldHost::take_sent`] (after a handler, and
/// after each tick).
#[allow(unused_variables)]
pub trait WorldHost<D> {
    /// Runs an NPC-module call. `None`: no provider.
    fn npc<C: NpcCall>(&mut self, game: &mut Game, events: &mut D, call: C) -> Option<C::Out> {
        None
    }
    /// Runs a vendors-module call. `None`: no provider.
    fn vendors<C: VendorCall>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        call: C,
    ) -> Option<C::Out> {
        None
    }
    /// Runs a waypoint-module call. `None`: no provider.
    fn waypoints<C: WaypointCall>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        call: C,
    ) -> Option<C::Out> {
        None
    }
    /// Runs a quest-module call. `None`: no provider.
    fn quests<C: QuestCall>(&mut self, game: &mut Game, events: &mut D, call: C) -> Option<C::Out> {
        None
    }
    /// The C→S 0x13 object case by `player` on the object with `guid`, on
    /// the host's object state.
    fn objects(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        guid: u32,
    ) -> Option<ObjectCase> {
        None
    }
    /// The C→S 0x13 tile case (a level warp; checks w3-01-tile-near /
    /// -walk / -far) by `player` on the tile with `guid`: the result
    /// code. `None`: no provider.
    fn warp_tile(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        guid: u32,
    ) -> Option<u32> {
        None
    }
    /// The run of `walk.0` to the ground item `walk.1` a pick-up asked
    /// for (cursor flag `walk.2`), with the pick-up on arrival. Default:
    /// nothing.
    fn item_walk(&mut self, game: &mut Game, events: &mut D, walk: (UnitId, UnitId, bool)) {}
    /// The run of `walk.0` to the object `walk.1` a 0x13 asked for
    /// with the 0x13 case again on arrival. Default: nothing.
    fn object_walk(&mut self, game: &mut Game, events: &mut D, walk: (UnitId, UnitId)) {}
    /// The cube (`handlers::items`) on the host's economy.
    fn cube<C: CubeCall>(&mut self, game: &mut Game, events: &mut D, call: C) -> Option<C::Out> {
        None
    }
    /// The item moves and the deferred item messages
    /// (`handlers::items::moves`) on the host's economy and inventories.
    fn moves<C: MoveCall>(&mut self, game: &mut Game, events: &mut D, call: C) -> Option<C::Out> {
        None
    }
    /// The skill-selection steps of a weapon switch of `player` (C→S
    /// 0x60): each [`SwitchSide::Select`] side is
    /// selected on the server list. Returns the S→C 0x23 the handler
    /// queues after 0x97 (one per selecting side, sent even when the
    /// skill was already selected) and the two 0x23 records the next
    /// update sends (left, then right). Default: none.
    ///
    /// [`SwitchSide::Select`]: d2_sim::wiring::inventory::swap::SwitchSide::Select
    fn weapon_switched(
        &mut self,
        events: &mut D,
        player: UnitId,
        guid: u32,
        sides: [d2_sim::wiring::inventory::swap::SwitchSide; 2],
    ) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        (Vec::new(), Vec::new())
    }
    /// The skill handlers (`handlers::skills`).
    fn skill(&mut self, call: SkillCall<'_, D>) -> Option<SkillHandled> {
        None
    }
    /// The small client-intent handlers (`handlers::player`) on the
    /// host's game state.
    fn player(
        &mut self,
        game: &mut Game,
        events: &mut D,
        run: PlayerRun<'_>,
    ) -> Option<PlayerOutcome> {
        None
    }
    /// The path positions (sub-tiles) of the listed units that have a
    /// path, read at the end of each tick to move the staged
    /// `UnitFacts` the point-message parser reads.
    /// Default: none (the caller stages positions itself).
    fn unit_positions(&mut self, events: &mut D, units: &[UnitId]) -> Vec<(UnitId, (i32, i32))> {
        Vec::new()
    }
    /// The walk / run handlers (`handlers::walk`) on the path provider.
    fn walk(&mut self, game: &mut Game, events: &mut D, call: WalkCall) -> Option<WalkResult> {
        None
    }
    /// The live gate fields of a player (unit mode, state 0x36), read
    /// after each tick to refresh the staged
    /// [`super::super::PlayerFields`] (a dead player's gate opens 0x41).
    /// `None`: the host has none; the staged fields stay.
    fn player_gate(
        &mut self,
        game: &Game,
        events: &mut D,
        unit: UnitId,
    ) -> Option<crate::seams::PlayerGate> {
        None
    }
    /// The live act and position of `unit` the point / unit parser reads
    /// from the game's own unit (its room's act, its path position). `None`: the host has none
    /// (the caller's staged [`super::super::UnitFacts`] are used).
    fn live_facts(
        &mut self,
        game: &Game,
        events: &mut D,
        unit: UnitId,
    ) -> Option<super::super::UnitFacts> {
        None
    }
    /// The client vitals sync for one client at
    /// the end of a flush: the messages to send it, in order. `None`:
    /// the host has no sync (or it is off), nothing runs. `staged`: the
    /// host's position of a player without a path record; `queued`: the
    /// client has a queued buffer.
    fn vitals_sync(
        &mut self,
        game: &mut Game,
        events: &mut D,
        client: SimClient,
        staged: (u16, u16),
        queued: bool,
    ) -> Option<Vec<Vec<u8>>> {
        None
    }
    /// The item messages the tick's client pass sends (S→C 0x9C of items
    /// queued for the update pass since the last tick), in queue order.
    /// Default: none.
    fn take_client_pass_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        Vec::new()
    }
    /// The messages the seams sent since the last take, in send order:
    /// (receiving player unit, bytes).
    fn take_sent(&mut self, events: &mut D) -> Vec<(UnitId, Vec<u8>)> {
        Vec::new()
    }
    /// The tick's steps 0–4 (`d2_sim::tick::tick_through_timers` with
    /// `events` as the step hooks), with the host's own step-4 work; a host
    /// may lend its own parts to the hooks around them (`WiredWorld`: the
    /// quest control, so quest object inits run inside their allocation).
    /// The caller runs the head of step 5 (the 8192-frame save) between this and
    /// [`WorldHost::run_tick_from_client_pass`].
    fn run_tick_through_timers(&mut self, game: &mut Game, events: &mut D)
    where
        D: EventDispatch + d2_sim::tick::TickHooks,
    {
        d2_sim::tick::tick_through_timers(game, events);
    }
    /// The tick's steps 5–11 (`d2_sim::tick::tick_from_client_pass`), after
    /// [`WorldHost::run_tick_through_timers`].
    fn run_tick_from_client_pass(&mut self, game: &mut Game, events: &mut D)
    where
        D: EventDispatch + d2_sim::tick::TickHooks,
    {
        d2_sim::tick::tick_from_client_pass(game, events);
    }
    /// Runs after the host's item update pass: the player step 5 / 7 sends
    /// the action wiring held back so the item messages precede them
    /// Default: nothing held.
    fn flush_player_tail(&mut self, events: &mut D) {}
    /// Runs after the tick's steps, before its sends are taken (the
    /// quest routes the tick queued, `WiredWorld`).
    fn after_tick(&mut self, game: &mut Game, events: &mut D) {}
    /// Runs after a session message (C→S 0x67 / 0x6B / 0x70) went
    /// through the session flow: the unit work its load and game entry
    /// raised (`WiredWorld`: the hireling calls, i.e. the join follow,
    /// which runs after the join placement). Its sends stay in the host's outbox
    /// for the next take.
    fn session_work(&mut self, game: &mut Game, events: &mut D) {}
    /// The host's millisecond clock (`Intents::set_host_tick`): the
    /// object code's tick-count input.
    fn host_tick(&mut self, events: &mut D, ms: u32) {}
    /// The host calls the game's quest rules raised since the last take
    /// (`QuestControl::take_host_requests`), in call order. A host without quests raises none.
    fn take_host_requests(&mut self) -> Vec<d2_sim::world::quests::HostRequest> {
        Vec::new()
    }
    /// Fatal paths met inside a call whose result could not carry them,
    /// since the last take; the handler reports them as its own.
    fn take_faults(&mut self) -> Vec<WorldError> {
        Vec::new()
    }
    /// Records a fatal path (see [`WorldError`]).
    fn fault(&mut self, fault: WorldFault);
}

/// A game without world systems: every world id stays a stub.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoWorld;

impl<D> WorldHost<D> for NoWorld {
    /// Never reached: no system runs, so nothing can fail.
    fn fault(&mut self, _fault: WorldFault) {}
}

/// The handler result of a module call: the result code, or `None` when
/// the message is not this module's (0x13 for a unit type other than 1).
type Run = Result<Option<u32>, WorldError>;

fn code(r: u32) -> ResultCode {
    match r {
        0 => ResultCode::Done,
        1 => ResultCode::Refused,
        2 => ResultCode::Invalid,
        // The modules return 0–3 only.
        _ => ResultCode::Malformed,
    }
}

/// The first byte of a queue-walk mark (`d2_sim::wiring::action`): host-only,
/// never sent raw.
fn is_queue_walk_mark(b: u8) -> bool {
    use d2_sim::wiring::action::reaction::LEVEL_UP_ITEMS_MARK;
    use d2_sim::wiring::action::{
        GROUND_ITEM_MARK, ITEM_BREAK_MARK, JOIN_ITEM_MARK, PLAYER_ITEMS_MARK, PLAYER_SOUND_MARK,
        STORE_ITEMS_MARK,
    };
    matches!(
        b,
        GROUND_ITEM_MARK
            | d2_sim::wiring::interaction::skill_use::EXHAUST_STACK_MARK
            | ITEM_BREAK_MARK
            | LEVEL_UP_ITEMS_MARK
            | d2_sim::wiring::action::pending::SOCKET_RESTORE_MARK
            | d2_sim::wiring::action::pending::KEY_USE_MARK
            | JOIN_ITEM_MARK
            | PLAYER_ITEMS_MARK
            | PLAYER_SOUND_MARK
            | STORE_ITEMS_MARK
    )
}

/// The world handler for one dispatched message. `None`: not a world id
/// handled here, no player, the host has no provider for its system, or
/// another module's unit type (0x13): the caller keeps its stub.
pub fn handle<D: EventDispatch, W: WorldHost<D>>(
    sim: &mut SimGame<D, W>,
    client: ClientId,
    msg: &[u8],
    size: usize,
    out: &mut dyn MessageSink,
) -> Option<ResultCode> {
    let id = *msg.first()?;
    // Every handled id has a fixed size ≤ 17, so the drained copy is the whole message.
    let msg = &msg[..size.min(msg.len())];
    let sys = route(msg)?;
    let player = sim.player_of(client)?;
    let (game, events) = (&mut sim.game, &mut sim.events);
    let run = match sys {
        System::Npc => sim.world.npc(game, events, NpcRun { player, msg }),
        System::Vendors => sim.world.vendors(game, events, VendorRun { player, msg }),
        System::Waypoints => sim
            .world
            .waypoints(game, events, WaypointRun { player, msg }),
        System::Quests => sim.world.quests(game, events, QuestRun { player, msg }),
        System::Warps => {
            let guid = u32::from_le_bytes([msg[5], msg[6], msg[7], msg[8]]);
            sim.world
                .warp_tile(game, events, player, guid)
                .map(|c| Ok(Some(c)))
        }
        System::Objects => {
            let guid = u32::from_le_bytes([msg[5], msg[6], msg[7], msg[8]]);
            match sim.world.objects(game, events, player, guid)? {
                ObjectCase::Code(c) => Some(Ok(Some(c))),
                // Not in range / line blocked: the run to the object;
                // result 0.
                ObjectCase::Walk(object) => {
                    sim.world.object_walk(game, events, (player, object));
                    Some(Ok(Some(0)))
                }
                // Operate function 23 on the host's waypoints; without them the id stays a stub.
                ObjectCase::Waypoint(_) => {
                    sim.world
                        .waypoints(game, events, WaypointOperate { player, guid })
                }
            }
        }
    }?;
    let sent = sim.world.take_sent(&mut sim.events);
    let mut faults = sim.world.take_faults();
    for (unit, bytes) in sent {
        // A player without a client receives nothing; a
        // queue-walk mark becomes its real messages (or none) at its place.
        if bytes.len() == 5 && is_queue_walk_mark(bytes[0]) {
            sim.send_or_mark(out, unit, bytes);
            continue;
        }
        if let Some(c) = sim.client_of(unit) {
            if let Err(e) = out.queue(c, &bytes) {
                faults.push(WorldError::from(e));
            }
        }
    }
    let result = match run {
        Ok(None) => None,
        Ok(Some(r)) => Some(code(r)),
        Err(e) => {
            faults.push(e);
            Some(ResultCode::Malformed)
        }
    };
    let failed = !faults.is_empty();
    for error in faults {
        sim.world.fault(WorldFault { client, id, error });
    }
    if failed {
        return Some(ResultCode::Malformed);
    }
    result
}

struct NpcRun<'m> {
    player: UnitId,
    msg: &'m [u8],
}

impl NpcCall for NpcRun<'_> {
    type Out = Run;
    fn call<W: NpcWorld + NpcVendors>(self, ctl: &mut NpcControl, w: &mut W) -> Run {
        let (p, m) = (self.player, self.msg);
        Ok(match m[0] {
            0x13 => ctl.interact(w, p, m)?,
            0x2F => Some(ctl.chat_open(w, p, m)),
            0x59 => Some(ctl.make_entity_move(w, p, m)),
            0x30 => Some(ctl.chat_close(w, p, m)),
            0x34 => Some(ctl.identify(w, p, m)),
            0x36 => Some(ctl.hire(w, p, m)?),
            0x38 => Some(ctl.menu_action(w, p, m)?),
            0x62 => Some(ctl.resurrect(w, p, m)),
            _ => None,
        })
    }
}

struct VendorRun<'m> {
    player: UnitId,
    msg: &'m [u8],
}

/// The vendor record of the NPC with GUID `npc`. When the NPC is missing
/// the trade functions refuse before reading the record, so an empty
/// scratch record stands in.
/// TODO: a class without a record is read as an empty record, like a
/// non-trader's.
fn record_of<'r, W: VendorWorld>(
    w: &W,
    records: &'r mut [VendorRecord],
    scratch: &'r mut VendorRecord,
    npc: u32,
) -> &'r mut VendorRecord {
    let class = w.npc_by_guid(npc).map(|n| w.npc_class(n));
    match class.and_then(|c| records.iter().position(|r| r.class == c)) {
        Some(i) => &mut records[i],
        None => scratch,
    }
}

impl VendorCall for VendorRun<'_> {
    type Out = Run;
    fn call<W: VendorWorld>(
        self,
        tables: &VendorTables,
        records: &mut [VendorRecord],
        w: &mut W,
    ) -> Run {
        let (p, m) = (self.player, self.msg);
        let mut scratch = VendorRecord::default();
        Ok(match m[0] {
            0x32 => match BuyMsg::parse(m) {
                Some(b) => {
                    let rec = record_of(w, records, &mut scratch, b.npc);
                    Some(buy(tables, rec, w, p, &b)?)
                }
                None => Some(3),
            },
            0x33 => match SellMsg::parse(m) {
                Some(s) => {
                    let rec = record_of(w, records, &mut scratch, s.npc);
                    Some(sell(tables, rec, w, p, &s)?)
                }
                None => Some(3),
            },
            // The handler drops the routine's result: 0 for every 17-byte
            // message.
            0x35 => match RepairMsg::parse(m) {
                Some(r) => {
                    repair(tables, w, p, &r)?;
                    Some(0)
                }
                None => Some(3),
            },
            0x37 => Some(identify_gamble(w, p, m)),
            _ => None,
        })
    }
}

struct WaypointRun<'m> {
    player: UnitId,
    msg: &'m [u8],
}

impl WaypointCall for WaypointRun<'_> {
    type Out = Run;
    fn call<W: WaypointWorld>(
        self,
        data: &WaypointData,
        arrivals: &mut ArrivalList,
        w: &mut W,
    ) -> Run {
        Ok(match self.msg[0] {
            0x49 => Some(data.take_or_close(w, arrivals, self.player, self.msg)?),
            _ => None,
        })
    }
}

/// Operate function 23 of the 0x13 object case.
///
/// TODO: the 0x13 result after the operate is undefined; read as 0.
pub(super) struct WaypointOperate {
    pub(super) player: UnitId,
    pub(super) guid: u32,
}

impl WaypointCall for WaypointOperate {
    type Out = Run;
    fn call<W: WaypointWorld>(self, data: &WaypointData, _: &mut ArrivalList, w: &mut W) -> Run {
        let Some((object, facts)) = w.object(self.guid) else {
            return Ok(Some(1));
        };
        data.operate(w, object, &facts, self.player)?;
        Ok(Some(0))
    }
}

/// The quest entry `(game, player, mode)`: the join's call, mode 0 for a
/// loaded character and mode 1 then 0 for a new one. An error is a fatal
/// condition, returned as text.
pub struct QuestEnter {
    /// The entering player.
    pub player: UnitId,
    /// The entry mode.
    pub mode: u8,
}

impl QuestCall for QuestEnter {
    type Out = Result<(), String>;
    fn call<W: QuestWorld>(self, ctl: &mut QuestControl, w: &mut W) -> Self::Out {
        ctl.player_enters(w, self.player, self.mode)
            .map_err(|e| e.to_string())
    }
}

struct QuestRun<'m> {
    player: UnitId,
    msg: &'m [u8],
}

impl QuestCall for QuestRun<'_> {
    type Out = Run;
    fn call<W: QuestWorld>(self, ctl: &mut QuestControl, w: &mut W) -> Run {
        let (p, m) = (self.player, self.msg);
        Ok(match m[0] {
            0x31 => Some(ctl.quest_message(w, p, m)),
            // Size 1 (checked by the dispatcher), then the quest data
            // request. No result is defined; read as 0.
            0x40 => {
                ctl.request_quest_data(w, p)?;
                Some(0)
            }
            0x58 => Some(ctl.quest_completed(w, p, m)),
            _ => None,
        })
    }
}

/// What the start items did on the wired host ([`WiredWorld::start_items`]).
#[derive(Debug, Default)]
pub struct StartItems {
    /// Each created item and where it went, in creation order.
    pub items: Vec<(UnitId, StartPlace)>,
    /// What the inventory placements queued (receiving unit, bytes), in
    /// send order. Not sent by the join; the caller decides.
    pub sent: Vec<(UnitId, Vec<u8>)>,
    /// Why no item was made: no charstats row, no inventory model, or an
    /// item creation error.
    pub faults: Vec<String>,
}

impl<R, S> WiredWorld<R, S> {
    /// What the inventory model queued outside a vendor or quest call
    /// (the quest objects' item removals, run on the loan): receiving
    /// unit, bytes.
    pub fn take_inventory_sent<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
    ) -> Vec<(UnitId, Vec<u8>)> {
        let Some(mut inv) = self.inventory.take() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        self.with_economy(game, events, |econ, _| {
            let mut d = inv.desk(econ);
            out = inv_take_sent(&mut d)
                .into_iter()
                .filter_map(|(u, b)| Some((u?, b)))
                .collect();
        });
        self.inventory = Some(inv);
        out
    }

    /// The start items of `player` on this host's economy and inventory model
    /// ([`character::start_items`] over [`WiredStart`]). The player's
    /// inventory is added to the model when it has none (the first item
    /// user does it here, not the unit allocation).
    pub fn start_items<D: ActionEvents>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
    ) -> StartItems {
        let mut r = StartItems::default();
        let Some(mut inv) = self.inventory.take() else {
            r.faults
                .push("no inventory model (WiredWorld::inventory is None)".into());
            return r;
        };
        let extra = self.start_extra.clone();
        self.with_economy(game, events, |econ, _| {
            let Some((class, guid)) = econ.units.get(player).map(|u| (u.class, u.guid)) else {
                r.faults.push(format!("no player unit {player:?}"));
                return;
            };
            // D2RUST's own, unverified: the play host never adds the player's
            // inventory (nothing in the unit allocation does); the quest
            // items (a reward, a delete) need it without start items.
            if !inv.state.inventories.contains_key(&player) {
                inv.state
                    .add_inventory(player, UnitKind::Player { class: class as u8 }, guid);
            }
            let Some(vitals) = econ.hooks.vitals.clone() else {
                r.faults
                    .push("no vitals tables (charstats) on the action wiring".into());
                return;
            };
            let Some(cs) = vitals.charstats(class as i32) else {
                // No charstats row: nothing.
                return;
            };
            let mut slots = character::start_slots(cs).to_vec();
            // D2RUST's own, unverified (REC-244): after the charstats slots,
            // one inventory copy of each extra code (loc 0, count 1).
            slots.extend(extra.iter().map(|&code| character::StartSlot {
                code,
                loc: 0,
                count: 1,
            }));
            let skills = econ.hooks.tables.skills.skills.len();
            let start_skill = Some(cs.startskill).filter(|&k| usize::from(k) < skills);
            // The players' skill lists lent to the rest for the call, as
            // every wired item call does (`wired::lend_skills`): the
            // scrolls' item-skill link (S→C 0x22) and, on the way back, the
            // stat 97 / 107 callback of the worn items (the start weapon's
            // `StartSkill`, S→C 0x21).
            wired::lend_skills(econ, &mut inv);
            let mut w = WiredStart {
                econ,
                inv: &mut inv,
                player,
                owner: Owner::player(guid),
                faults: Vec::new(),
            };
            r.items = character::start_items(&mut w, &slots, start_skill);
            r.faults.append(&mut w.faults);
            wired::return_skills(econ, &mut inv, true);
            let mut d = inv.desk(econ);
            // The placements put the items on the player's update list
            // The join sends that list as the player's item messages and
            // resets it, so no tick's unit update sends them again. Here
            // both run at the start items' end: the load's later steps
            // (start skill, quest entry, 0x0B, 0x5F, the stat messages)
            // never touch the list, and the messages are held for the join
            // send (`join_items`). The stream is one 0x9D / 0x9C per start
            // item after the stat messages, before the 0x23 pair.
            let owner = Owner::player(guid);
            let mut listed = Vec::new();
            if d.update_bits(owner) & 1 != 0 {
                match d2_sim::items::moves::update_list_pass(&mut d, guid, guid) {
                    Ok(m) => listed = m,
                    Err(e) => r.faults.push(format!("start items: update list: {e:?}")),
                }
                d.update_done(owner);
            }
            d.sync_out();
            r.sent = inv_take_sent(&mut d)
                .into_iter()
                .filter_map(|(u, b)| Some((u?, b)))
                .collect();
            r.sent.extend(listed.into_iter().map(|m| (player, m)));
        });
        self.inventory = Some(inv);
        r
    }
}

/// [`StartItemWorld`] on the wired host's economy and inventory model:
/// creation through the quest reward's `create_reward` (with the start
/// items' arguments), the stat writes through the item's stat lists,
/// placement through an inventory desk per call.
pub struct WiredStart<'e, 'a, H> {
    /// The economy the items are created on.
    pub econ: &'e mut Economy<'a, H>,
    /// The inventory model the items are placed in.
    pub inv: &'e mut InvParts,
    /// The player receiving the items.
    pub player: UnitId,
    /// The player as an item owner.
    pub owner: Owner,
    /// What went wrong, as text.
    pub faults: Vec<String>,
}

impl<H: LifecycleHooks> WiredStart<'_, '_, H> {
    fn guid(&self, item: UnitId) -> u32 {
        self.econ.units.get(item).map_or(u32::MAX, |u| u.guid)
    }
    fn set_stat(&mut self, item: UnitId, id: u16, v: i32) {
        self.econ
            .with_stats(|ctx| UnitStats::new(ctx, item).set_base(id, 0, v));
    }
}

impl<X: d2_sim::wiring::action::Pending> StartItemWorld
    for WiredStart<'_, '_, d2_sim::wiring::action::ActionHooks<X>>
{
    fn create(&mut self, code: [u8; 4]) -> Option<UnitId> {
        // `create_reward`: level 0 → the player's base level (≥ 1),
        // quality 2, spawn mode 4, no sockets, never ethereal, no seeds,
        // then durability := max and page 0 (step 2.6 sets 72 again after the
        // placement, the same value).
        match create_reward_flags(
            self.econ,
            self.player,
            code,
            0,
            q::NORMAL,
            d2_sim::items::req::NO_SOCKETS | d2_sim::items::req::NEVER_ETHEREAL,
        ) {
            Ok(i) => i,
            Err(e) => {
                self.faults.push(format!("start item {code:?}: {e:?}"));
                None
            }
        }
    }
    fn drop_class_skill_list(&mut self, item: UnitId) {
        self.econ.with_stats(|ctx| {
            let mut c = ctx.borrow_mut();
            let StatCtx { lists, host } = &mut *c;
            if let Some(l) = find_list(lists, item, ListKey::ITEM) {
                lists.unit_detach(*host, l);
                lists.free_plain(*host, l);
            }
        });
    }
    fn set_single_skill(&mut self, item: UnitId, skill: u16) {
        self.econ.with_stats(|ctx| {
            UnitStats::new(ctx, item).list_set(ListKey::ITEM, istat::ITEM_SINGLESKILL, skill, 1)
        });
    }
    fn stackable(&mut self, item: UnitId) -> bool {
        let g = self.guid(item);
        self.inv.desk(self.econ).stackable(g)
    }
    fn fill_stack(&mut self, item: UnitId) {
        let g = self.guid(item);
        let n = self.inv.desk(self.econ).max_stack(g);
        self.set_stat(item, istat::QUANTITY, n);
    }
    fn mark_start(&mut self, item: UnitId, loc: u8) {
        if let Some(i) = self.econ.items.get_mut(item) {
            i.flags |= flag::STARTITEM;
        }
        let d = self.inv.desk(self.econ);
        if let Some(i) = d.state.items.get_mut(&item) {
            i.body_loc = loc;
        }
    }
    fn beltable(&mut self, item: UnitId) -> bool {
        let g = self.guid(item);
        InventoryOps::beltable(&self.inv.desk(self.econ), g)
    }
    fn place_belt(&mut self, item: UnitId) -> bool {
        // The belt's free-slot search picks the slot, then the 0x23 body
        // runs (the slot placement, link, mode 2, page 0xFF, command flag
        // 0x400 → 0x9C action 0x0E, the update list), as the vendor's
        // auto-belt does (`vendor_inv.rs` `put_in_belt`). The item was made
        // on the cursor (mode 4).
        let (o, g) = (self.owner, self.guid(item));
        let mut d = self.inv.desk(self.econ);
        match d.belt_free_slot(o, g) {
            Some(s) => {
                d2_sim::items::moves::handlers::to_belt(&mut d, o, g, u32::from(s))
                    == d2_sim::items::moves::Outcome::DONE
            }
            None => false,
        }
    }
    fn place_inventory(&mut self, item: UnitId) -> bool {
        if let Some(i) = self.econ.items.get_mut(item) {
            i.inv_page = 0;
        }
        let p = self.player;
        self.inv.desk(self.econ).place(p, item, (0, 0), true, true)
    }
    fn equip(&mut self, item: UnitId, loc: u8) -> bool {
        let (o, g) = (self.owner, self.guid(item));
        let done = self
            .inv
            .desk(self.econ)
            .equip_from_cursor(o, g, loc, true)
            .0;
        // The equip's stat refresh runs the stat 97 / 107 callback inline:
        // the start weapon's `StartSkill` sends its 0x21 now, before a
        // later slot's 0x22 (the join's message order). The lent lists are
        // taken back with that sync and lent again for the next slots.
        wired::return_skills(self.econ, self.inv, true);
        wired::lend_skills(self.econ, self.inv);
        done
    }
    fn quiver(&mut self, item: UnitId) -> bool {
        let g = self.guid(item);
        self.inv.desk(self.econ).quiver(g)
    }
    fn set_quantity(&mut self, item: UnitId, n: i32) {
        self.set_stat(item, istat::QUANTITY, n);
    }
    fn fill_durability(&mut self, item: UnitId) {
        // 0 without a base stat 73, else its total.
        let max = self.econ.with_stats(|ctx| {
            let s = UnitStats::new(ctx, item);
            if s.base(istat::MAXDURABILITY, 0) == 0 {
                0
            } else {
                s.stat(istat::MAXDURABILITY, 0)
            }
        });
        self.set_stat(item, istat::DURABILITY, max);
    }
}

/// The item-move seams no d2-sim module provides, answered for the first
/// playable preview so the play host can have an inventory model
/// ([`preview_inv_parts`]): nothing is active,
/// no own contribution, every location allowed, no quiver kind, player
/// data busy and trade bytes zero, no NPC talk, no player trade; the sends are
/// collected for [`MoveRest::take_sent`].
///
/// D2RUST's own, unverified: every answer here is a preview fill, not a
/// defined behaviour; the `InvRest` method docs name the open point each
/// one stands in for.
#[derive(Debug, Default)]
pub struct PreviewMoveRest {
    sent: Vec<(Owner, Vec<u8>)>,
    /// The places staged at the start of the call ([`MoveRest::stage`]).
    places: BTreeMap<Owner, StagedPlace>,
    /// The room boxes staged at the start of the call
    /// ([`MoveRest::stage_rooms`]), the players' rooms first.
    rooms: Vec<(RoomId, d2_sim::drlg::TileRect)>,
    item_format: u16,
    /// The players' quest flags staged at the start of the call, and the
    /// writes made since ([`MoveRest::stage_quest_flags`]).
    quest_flags: BTreeMap<Owner, d2_sim::world::quests::QuestFlags>,
    quest_writes: Vec<(Owner, u8, u8, bool)>,
    /// The pick-ups of the call (player, item), for the quest hook.
    picked: Vec<(Owner, d2_sim::items::moves::Guid)>,
    /// The skill seams over the players' lists.
    skills: super::items::moves::preview_skills::PreviewSkills,
    /// The players' first golem pets and the Blood Golem's `Param6`,
    /// staged at the start of the call ([`MoveRest::stage_pets`]).
    golems: BTreeMap<Owner, Owner>,
    blood_golem_param6: Option<i32>,
}

impl MovePending for PreviewMoveRest {
    /// The has-used-skill flag of the staged player; an unstaged one
    /// keeps the seam's default (used).
    fn has_used_skill(&self, player: Owner) -> bool {
        self.places.get(&player).is_none_or(|p| p.used_skill)
    }
    fn send(&mut self, player: Owner, bytes: Vec<u8>) {
        self.sent.push((player, bytes));
    }
    /// The room delete record only queues S→C 0x0A (type 4, GUID,
    /// `units::messages::remove_unit`) for the room's clients; they go out
    /// in the tick, ahead of the item's 0x9C / the tome's 0x3E.
    /// The staged players stand for the room's clients (single player: the
    /// one). The item-move handler defers the notice to the next tick
    /// (`SimGame::deferred_room_deletes`); the client then drops the ground
    /// unit before the 0x9C re-adds the item.
    fn room_delete_notice(&mut self, item: d2_sim::items::moves::Guid) {
        let players: Vec<Owner> = self
            .places
            .values()
            .filter(|p| p.owner.is_player())
            .map(|p| p.owner)
            .collect();
        for p in players {
            let m = d2_sim::units::messages::remove_unit(4, item).to_vec();
            // The record is idempotent (the unit is already unlinked from
            // the room the second time): one notice per call and client.
            if !self.sent.iter().any(|(o, b)| *o == p && *b == m) {
                self.sent.push((p, m));
            }
        }
    }
    /// Hook ITEMPICKEDUP: recorded, run on the quest
    /// control after the call (`WiredWorld::moves`).
    fn quest_item_picked(&mut self, player: Owner, item: d2_sim::items::moves::Guid) {
        self.picked.push((player, item));
    }
    /// The staged quest record.
    fn quest_flag(&self, player: Owner, quest: u8, flag: u8) -> bool {
        self.quest_flags
            .get(&player)
            .is_some_and(|f| f.get(quest, flag))
    }
    /// Quest flag set / clear: applied to the staged copy at once, and
    /// recorded for the host to write back.
    fn set_quest_flag(&mut self, player: Owner, quest: u8, flag: u8, on: bool) {
        if let Some(f) = self.quest_flags.get_mut(&player) {
            if on {
                f.set(quest, flag);
            } else {
                f.clear(quest, flag);
            }
        }
        self.quest_writes.push((player, quest, flag, on));
    }
    /// Unit distance of the staged places, out of range without both.
    /// The unit sizes: a player 2, an item 1; any other unit type 2 (a
    /// monster's `monstats2` size and an object's `SizeX` are not staged
    /// here: D2RUST's own, unverified).
    fn distance(&self, a: Owner, b: Owner) -> i32 {
        use d2_sim::path::walk::geom::unit_distance;
        use d2_sim::path::{PathTables, Point};
        static TABLES: std::sync::OnceLock<Option<PathTables>> = std::sync::OnceLock::new();
        let size = |o: Owner| if o.ty == Owner::ITEM { 1 } else { 2 };
        match (self.places.get(&a), self.places.get(&b)) {
            (Some(pa), Some(pb)) => match TABLES.get_or_init(|| PathTables::spec().ok()) {
                Some(t) => unit_distance(
                    t,
                    Point::new(pa.pos.0, pa.pos.1),
                    size(a),
                    Point::new(pb.pos.0, pb.pos.1),
                    size(b),
                ),
                None => i32::MAX,
            },
            _ => i32::MAX,
        }
    }
    /// D2RUST's own, unverified (D1): a room exists where a staged room box
    /// contains the point; with no staged geometry, wherever the player's
    /// room does.
    fn room_at(&self, x: i32, y: i32) -> bool {
        self.room_containing(x, y).is_some()
    }
    /// D2RUST's own, unverified (D1): no collision search here; the start
    /// point is taken as free and belongs to the room that contains it, else
    /// the origin does (the player's own spot).
    fn free_spot(
        &self,
        start: (i32, i32),
        origin: (i32, i32),
        _: u32,
        _: u32,
        _: u32,
        _: u32,
    ) -> Option<Spot> {
        [start, origin]
            .into_iter()
            .find_map(|(x, y)| self.room_containing(x, y).map(|room| Spot { room, x, y }))
    }
}

impl PreviewMoveRest {
    /// The room that contains sub-tile (x, y): the first staged room box
    /// holding it (the player's room is staged first). With no geometry
    /// staged, the player's room.
    fn room_containing(&self, x: i32, y: i32) -> Option<RoomId> {
        if self.rooms.is_empty() {
            return self.player_room();
        }
        self.rooms
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|&(room, _)| room)
    }

    /// The room of the first staged player (the preview has one).
    fn player_room(&self) -> Option<RoomId> {
        self.places
            .values()
            .find(|p| p.owner.is_player())
            .and_then(|p| p.room)
    }
}

impl InvRest for PreviewMoveRest {
    /// The staged first golem pet (type 3 only).
    fn first_pet(&self, player: Owner, t: i32) -> Option<Owner> {
        (t == 3)
            .then(|| self.golems.get(&player).copied())
            .flatten()
    }
    /// The staged Blood Golem `Param6` (class 290 only).
    fn monster_skill1_param6(&self, class: u16) -> Option<i32> {
        (class == d2_sim::wiring::inventory::potion::BLOOD_GOLEM)
            .then_some(self.blood_golem_param6)
            .flatten()
    }
    fn mouse_skill(&self, u: Owner, left: bool) -> Option<(i32, i32)> {
        self.skills.mouse_skill(u, left)
    }
    fn select_skill(&mut self, u: Owner, left: bool, skill: (i32, i32)) {
        self.skills.select_skill(u, left, skill)
    }
    fn has_skill_owned(&self, u: Owner, skill: (i32, i32)) -> bool {
        self.skills.has_skill_owned(u, skill)
    }
    fn throw_skill_row(&self, skill: i32) -> bool {
        self.skills.throw_skill_row(skill)
    }
    fn saved_mouse_skill(&self, u: Owner, left: bool) -> (i32, i32) {
        self.skills.saved_mouse_skill(u, left)
    }
    fn skill_use_state(&mut self, u: Owner, skill: (i32, i32)) -> u8 {
        self.skills.skill_use_state(u, skill)
    }
    fn skill_item_types(&self, skill: i32) -> Option<ItemTypeSkill> {
        self.skills.skill_item_types(skill)
    }
    fn set_saved_mouse_skill(&mut self, u: Owner, left: bool, skill: (i32, i32)) {
        self.skills.set_saved_mouse_skill(u, left, skill)
    }
    fn trade_swap_skills(&mut self, u: Owner) -> [(i32, i32); 2] {
        self.skills
            .stage
            .as_mut()
            .and_then(|st| st.lists.get_mut(&u))
            .map_or([(0, -1); 2], |sl| sl.list.trade_swap_pairs())
    }
    fn skill_quantity(&self, u: Owner, skill: i32) -> Option<i32> {
        self.skills.skill_quantity(u, skill)
    }
    fn set_skill_quantity(&mut self, u: Owner, skill: i32, q: i32) {
        self.skills.set_skill_quantity(u, skill, q)
    }
    fn learn_skill(&mut self, u: Owner, skill: i32) {
        self.skills.learn_skill(u, skill)
    }
    /// D2RUST's own, unverified (D1): the staged place of a unit.
    fn pos(&self, u: Owner) -> (i32, i32) {
        self.places.get(&u).map_or((0, 0), |p| p.pos)
    }
    fn set_pos(&mut self, u: Owner, x: i32, y: i32) {
        if let Some(p) = self.places.get_mut(&u) {
            p.pos = (x, y);
        }
    }
    /// D2RUST's own, unverified (D1): the request layout of a `gld` pile
    /// is unwritten; a plain normal-quality item of the game's format,
    /// ground mode.
    fn gold_request(&self, _: Owner, gld: usize) -> Option<(ItemRequest, ItemSpawn)> {
        Some((
            ItemRequest {
                item: gld as i32,
                format: self.item_format,
                ilvl: 1,
                quality: q::NORMAL,
                flags2: 0x2,
                ..ItemRequest::default()
            },
            ItemSpawn {
                room: None,
                mode: 3,
                init_flags: 1,
            },
        ))
    }
    fn item_active_on(&self, _: u32, _: Owner) -> bool {
        false
    }
    fn own_contribution(&self, _: u32, _: Owner, _: u16) -> i32 {
        0
    }
    fn one_or_two_handed(&self, _: Owner, _: u32) -> bool {
        false
    }
    fn has_allowed_location(&self, _: u32) -> bool {
        true
    }
    fn quiver_kind(&self, _: u32) -> bool {
        false
    }
    fn player_data_4c(&self, _: Owner) -> u32 {
        0
    }
    fn player_data_50(&self, _: Owner) -> u32 {
        0
    }
    fn npc_talking(&self, _: Owner, _: Owner) -> bool {
        false
    }
    fn player_trade_gate(&self, _: Owner) -> Option<bool> {
        None
    }
}

impl MoveRest for PreviewMoveRest {
    fn stage_skills(
        &mut self,
        stage: super::items::moves::preview_skills::SkillStage,
        tables: &InvTables,
    ) {
        if self.skills.throw_rows.is_empty() {
            self.skills.throw_rows =
                super::items::moves::preview_skills::throw_rows(&stage.tables, &tables.equiv);
        }
        self.skills.stage = Some(stage);
    }
    fn take_skills(&mut self) -> Option<super::items::moves::preview_skills::SkillStage> {
        self.skills.stage.take()
    }
    fn queue_sent(&mut self, sent: Vec<(Owner, Vec<u8>)>) {
        self.sent.extend(sent);
    }
    fn take_sent(&mut self) -> Vec<(Owner, Vec<u8>)> {
        let mut sent = std::mem::take(&mut self.sent);
        sent.append(&mut self.skills.sent);
        sent
    }
    fn stage(&mut self, places: &[StagedPlace], item_format: u16) {
        self.places = places.iter().map(|p| (p.owner, *p)).collect();
        self.item_format = item_format;
    }
    fn stage_rooms(&mut self, rooms: &[(RoomId, d2_sim::drlg::TileRect)]) {
        self.rooms = rooms.to_vec();
    }
    fn stage_quest_flags(&mut self, flags: &[(Owner, d2_sim::world::quests::QuestFlags)]) {
        self.quest_flags = flags.iter().copied().collect();
        self.quest_writes.clear();
    }
    fn take_quest_flag_writes(&mut self) -> Vec<(Owner, u8, u8, bool)> {
        std::mem::take(&mut self.quest_writes)
    }
    fn take_picked_items(&mut self) -> Vec<(Owner, d2_sim::items::moves::Guid)> {
        std::mem::take(&mut self.picked)
    }
    fn stage_pets(&mut self, golems: &[(Owner, Owner)], blood_golem_param6: Option<i32>) {
        self.golems = golems.iter().copied().collect();
        self.blood_golem_param6 = blood_golem_param6;
    }
}

/// D2RUST's own, unverified (REC-119): the cube's calls no written spec owns
/// (`ItemPending`) in the play host. The inventory pass queues nothing
/// (the transmuted items leave through the inventory model's own sends),
/// a copy, a tempered affix, the runeword and repair / recharge effects
/// and the quest hooks do nothing, and the Cow portal is refused.
#[derive(Debug, Default)]
pub struct PreviewCubePending {
    /// `hst ` hooks recorded for the quest control.
    quest_items: Vec<(UnitId, [u8; 4])>,
}

impl super::items::ItemPending for PreviewCubePending {
    fn inventory_pass(&mut self, _: UnitId, _: &mut Vec<Vec<u8>>) {}
    fn duplicate(&mut self, _: UnitId, _: bool) -> Option<UnitId> {
        None
    }
    fn tempered_affix(&mut self, _: UnitId, _: bool) -> u16 {
        0
    }
    fn drop_runeword_stats(&mut self, _: UnitId) {}
    fn repair(&mut self, _: UnitId) {}
    fn recharge(&mut self, _: UnitId) {}
    fn quest_item_hook(&mut self, player: UnitId, _: UnitId, code: [u8; 4]) {
        self.quest_items.push((player, code));
    }
    fn take_quest_items(&mut self) -> Vec<(UnitId, [u8; 4])> {
        std::mem::take(&mut self.quest_items)
    }
    fn cow_portal(&mut self, _: UnitId) -> bool {
        false
    }
}

/// The cube parts of the play host ([`WiredWorld::cube`]) over `cube`
/// (`CubeData::from_fixed`) with [`PreviewCubePending`].
pub fn preview_cube_parts(cube: d2_sim::world::cube::CubeData) -> super::items::CubeParts {
    super::items::CubeParts::new(cube, Box::new(PreviewCubePending::default()))
}

/// An inventory model for the play host ([`WiredWorld::inventory`]) over
/// `tables` (`InvTables::from_fixed`) with [`PreviewMoveRest`].
pub fn preview_inv_parts(tables: InvTables) -> InvParts {
    let mut parts = InvParts::new(tables, Box::new(PreviewMoveRest::default()));
    // Worn items feed the wearer's stats, set bonuses included
    // (`wiring/inventory/item_link.rs`).
    parts.state.link_item_stats = true;
    // The item-move effects (gold rest pile, 0x5D) on the desk.
    parts.state.move_effects = true;
    parts
}

#[cfg(test)]
mod room_delete_notice_tests {
    use super::*;

    /// The room delete notice is queued as S→C 0x0A (type 4) for the staged
    /// player; the item-move handler holds it back until the tick.
    #[test]
    fn the_room_delete_notice_queues_a_remove_unit() {
        let mut rest = PreviewMoveRest::default();
        let owner = d2_sim::items::moves::Owner::player(1);
        rest.places.insert(
            owner,
            crate::adapters::handlers::items::moves::StagedPlace {
                owner,
                pos: (0, 0),
                room: None,
                used_skill: false,
            },
        );
        rest.room_delete_notice(2);
        rest.room_delete_notice(2);
        assert_eq!(rest.sent.len(), 1);
        assert!(rest.sent.iter().all(|(_, b)| b[0] == 0x0A && b[1] == 4));
    }
}

#[cfg(test)]
mod ground_spot_room_tests {
    use super::*;
    use d2_sim::drlg::TileRect;

    /// The player's room (id 3, 40×40 sub-tiles at (5080, 5160)) and the
    /// room to its right (id 4), as the move call stages them.
    fn staged(neighbour: bool) -> PreviewMoveRest {
        let mut rest = PreviewMoveRest::default();
        let owner = d2_sim::items::moves::Owner::player(1);
        rest.places.insert(
            owner,
            crate::adapters::handlers::items::moves::StagedPlace {
                owner,
                pos: (5119, 5167),
                room: Some(RoomId(3)),
                used_skill: false,
            },
        );
        let mut rooms = vec![(RoomId(3), TileRect::new(5080, 5160, 40, 40))];
        if neighbour {
            rooms.push((RoomId(4), TileRect::new(5120, 5160, 40, 40)));
        }
        rest.stage_rooms(&rooms);
        rest
    }

    /// A ground spot belongs to the room that contains it: the start point
    /// one sub-tile past the player's room (5121, 5170) is in the room
    /// next to it. It was filed under the player's room, which does not
    /// contain it (soak finding `room:outside:Item`).
    #[test]
    fn a_drop_past_the_room_edge_is_filed_under_the_room_it_lands_in() {
        let rest = staged(true);
        assert!(rest.room_at(5121, 5170));
        let spot = rest
            .free_spot((5121, 5170), (5119, 5167), 1, 0, 0, 1)
            .unwrap();
        assert_eq!(
            spot,
            Spot {
                room: RoomId(4),
                x: 5121,
                y: 5170
            }
        );
    }

    /// A start point in no staged room falls back to the player's own spot
    /// and its room: the item never lands outside the room it is filed in.
    #[test]
    fn a_drop_outside_every_room_falls_back_to_the_players_spot() {
        let rest = staged(false);
        assert!(!rest.room_at(5121, 5170));
        let spot = rest
            .free_spot((5121, 5170), (5119, 5167), 1, 0, 0, 1)
            .unwrap();
        assert_eq!(
            spot,
            Spot {
                room: RoomId(3),
                x: 5119,
                y: 5167
            }
        );
    }

    /// Half-open: the room's far edge (x = 5120) is outside it.
    #[test]
    fn the_far_edge_belongs_to_the_next_room() {
        let rest = staged(true);
        let spot = rest
            .free_spot((5120, 5170), (5119, 5167), 1, 0, 0, 1)
            .unwrap();
        assert_eq!(spot.room, RoomId(4));
        let spot = rest
            .free_spot((5119, 5170), (5119, 5167), 1, 0, 0, 1)
            .unwrap();
        assert_eq!(spot.room, RoomId(3));
    }
}
