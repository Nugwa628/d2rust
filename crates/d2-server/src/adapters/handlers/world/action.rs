//! The world host that runs on the action wiring alone, and the seams the
//! other hosts share with it (`Outbox`, `ActionEvents`). `handlers::world`
//! builds on it, and [`super::WiredWorld`] extends it.
//!
//! [`ActionWorld`]: the systems whose seams have a provider in
//! `d2_sim::wiring::action` alone — the waypoints (`WaypointView` on
//! [`ActionSim`], reached through [`ActionEvents`]) and the skill
//! handlers (a [`SkillHost`] slot, `handlers::skills::wired`). The NPC,
//! vendor, quest and cube ids stay stubs on this host;
//! [`super::WiredWorld`] adds them on the interaction and economy
//! wiring.
//!
//! Game creation ([`ActionEvents::create_game`]): the game seed and the
//! creation fields have their one home on the action wiring
//! (`ActionHooks::game_seed`, `ActionHooks::ai_info`,
//! `UnitData::expansion`); the copies other `d2-sim` readers hold are
//! written from the same values there.

use d2_sim::game::Game;
use d2_sim::monsters::ai::AiStore;
use d2_sim::tick::EventDispatch;
use d2_sim::units::{ClientId as SimClient, UnitId};
use d2_sim::wiring::action::{vitals_sync, ActionSim, ObjectCase, Pending};
use d2_sim::wiring::economy::GameFields;
use d2_sim::wiring::worldgen::{WorldPending, WorldSim};
use d2_sim::world::waypoints::WaypointData;

use super::super::player::{self, HostFacts, Outcome as PlayerOutcome, Run as PlayerRun};
use super::super::skills::{Call as SkillCall, Handled as SkillHandled, NoSkills, SkillHost};
use super::super::walk::{WalkCall, WalkResult};
use super::{WaypointCall, WorldFault, WorldHost};

/// Messages queued by the action wiring's `Pending::send` (the transport
/// seam), handed back in send order.
pub trait Outbox {
    /// Takes the queued `(receiving unit, message bytes)` pairs.
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)>;
}

/// A game dispatch that holds the action wiring: [`ActionSim`] itself, or
/// the world-generation dispatch [`WorldSim`] around it (its `action`
/// field). The handlers on the action seams (waypoints here, skills in
/// `handlers::skills::wired`) reach the action systems through it.
pub trait ActionEvents: EventDispatch {
    /// The transport seam the action wiring sends through.
    type X: Pending;
    /// The action wiring inside this dispatch.
    fn action(&mut self) -> &mut ActionSim<Self::X>;

    /// Runs `f` on [`Self::action`] with the game's monster state lent
    /// to the action hooks (`ActionHooks::monster_world`), so a monster
    /// allocated inside `f` gets its monster type init and a freed one
    /// leaves the monster state.
    /// Without a monster state (a bare [`ActionSim`]) `f` runs as is.
    fn lend_world<R>(&mut self, f: impl FnOnce(&mut ActionSim<Self::X>) -> R) -> R {
        f(self.action())
    }

    /// The game's player count (the clients with a player), which
    /// monster creation scales by. Without the world-generation state
    /// nothing reads it.
    fn set_player_count(&mut self, _players: i32) {}

    /// Game creation's fields from `fields`: the game seed and the
    /// creation fields (difficulty, expansion, game type and ladder; the
    /// item format follows from the expansion) written to their home, `ActionHooks::game_seed`,
    /// `ActionHooks::ai_info` and `UnitData::expansion`, and to the
    /// difficulty copy the unit code reads (`UnitData::difficulty`). The
    /// economy's `GameFields` are built from the home for each call
    /// (`WiredWorld::with_economy`); `fields.uniques` is not used (a new
    /// game's unique bits are zero).
    fn create_game(&mut self, fields: &GameFields) {
        let a = self.action();
        a.sys.hooks.game_seed = fields.seed;
        a.sys.hooks.ai_info = fields.ai_info();
        a.sys.data.difficulty = fields.difficulty;
        // The neutral AI delay reads the difficulty's column when the game type
        // or the ladder flag is set.
        a.sys.data.aidel_by_difficulty = fields.game_type != 0 || fields.ladder;
        a.sys.data.expansion = fields.expansion;
    }
}

/// The server process's state that outlives its games: the Npc command
/// counter G, the count the Npc command step reads. G is process-wide:
/// zero at process start, counted across every game of the process, never
/// reset or saved.
/// One per server process: [`Self::start_game`] hands G to each new
/// game's AI store, [`Self::end_game`] reads back what the game left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessState {
    pub npc_walk_counter: u32,
}

impl ProcessState {
    /// A fresh process (G = 0).
    pub fn new() -> Self {
        Self::default()
    }
    /// A new game of this process: its AI store continues from G
    /// (`AiStore::with_npc_walk_counter`). A game without an AI store
    /// (`ActionHooks::ai` `None`) gets one.
    pub fn start_game<D: ActionEvents>(&self, d: &mut D) {
        let hooks = &mut d.action().sys.hooks;
        match hooks.ai.as_mut() {
            Some(store) => store.npc_walk_counter = self.npc_walk_counter,
            None => hooks.ai = Some(AiStore::with_npc_walk_counter(self.npc_walk_counter)),
        }
    }
    /// The game ends: G as the game left it, for the process's next game.
    /// A game without an AI store leaves G unchanged.
    pub fn end_game<D: ActionEvents>(&mut self, d: &mut D) {
        if let Some(store) = d.action().sys.hooks.ai.as_ref() {
            self.npc_walk_counter = store.npc_walk_counter;
        }
    }
}

impl<X: Pending> ActionEvents for ActionSim<X> {
    type X = X;
    fn action(&mut self) -> &mut ActionSim<X> {
        self
    }
}

impl<X: WorldPending> ActionEvents for WorldSim<X> {
    type X = X;
    fn action(&mut self) -> &mut ActionSim<X> {
        &mut self.action
    }

    fn lend_world<R>(&mut self, f: impl FnOnce(&mut ActionSim<X>) -> R) -> R {
        self.lend(f)
    }

    /// Monster init's player count (`WorldState::init_info`).
    fn set_player_count(&mut self, players: i32) {
        self.world.init_info.players = players;
    }

    /// Also the world-generation copies (`WorldState::pop_info`,
    /// `init_info`; their player counts are kept).
    fn create_game(&mut self, fields: &GameFields) {
        let a = &mut self.action;
        a.sys.hooks.game_seed = fields.seed;
        a.sys.hooks.ai_info = fields.ai_info();
        a.sys.data.difficulty = fields.difficulty;
        // The neutral AI delay reads the difficulty's column when the game type
        // or the ladder flag is set.
        a.sys.data.aidel_by_difficulty = fields.game_type != 0 || fields.ladder;
        a.sys.data.expansion = fields.expansion;
        let w = &mut self.world;
        w.init_info.difficulty = fields.difficulty;
        w.init_info.expansion = fields.expansion;
        w.init_info.game_type = fields.game_type;
        w.init_info.ladder = fields.ladder;
        w.pop_info.difficulty = fields.difficulty;
        w.pop_info.expansion = fields.expansion;
    }
}

/// The systems of a game wired on [`ActionSim`]: the waypoints and the
/// skill handlers' slot `S` ([`NoSkills`]: skill ids stay stubs).
#[derive(Default)]
pub struct ActionWorld<S = NoSkills> {
    /// Waypoint tables (`WaypointData::new(levels, objects)`); `None`:
    /// 0x49 stays a stub.
    pub waypoints: Option<WaypointData>,
    /// Fatal paths met by the handlers, in order.
    pub faults: Vec<WorldFault>,
    /// The skill handlers (`handlers::skills::wired::WiredSkills`).
    pub skills: S,
}

impl<D: ActionEvents, S: SkillHost<D>> WorldHost<D> for ActionWorld<S>
where
    D::X: Outbox,
{
    fn waypoints<C: WaypointCall>(
        &mut self,
        game: &mut Game,
        events: &mut D,
        call: C,
    ) -> Option<C::Out> {
        let data = self.waypoints.as_ref()?;
        // The arrival list is the action hooks' (init function 17 reads
        // it inside an object's creation).
        let mut arrivals = std::mem::take(&mut events.action().hooks().arrivals);
        let out = events
            .action()
            .waypoints(game, |w| call.call(data, &mut arrivals, w));
        events.action().hooks().arrivals = arrivals;
        Some(out)
    }

    fn unit_positions(&mut self, events: &mut D, units: &[UnitId]) -> Vec<(UnitId, (i32, i32))> {
        let hooks = events.action().hooks();
        units
            .iter()
            .filter(|&&u| hooks.path_has(u))
            .map(|&u| (u, hooks.path_position(u)))
            .collect()
    }

    /// The 0x13 object case on the action wiring's object state
    /// (`ActionSim::operate_object_message`; `None` until
    /// `ActionSim::create_objects` ran). The object calls' host tick is
    /// the frame's ([`WorldHost::host_tick`]).
    fn objects(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        guid: u32,
    ) -> Option<ObjectCase> {
        events.action().operate_object_message(game, player, guid)
    }

    /// The 0x13 tile case on the action wiring.
    fn warp_tile(
        &mut self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
        guid: u32,
    ) -> Option<u32> {
        events.action().warp_tile_message(game, player, guid)
    }

    fn skill(&mut self, call: SkillCall<'_, D>) -> Option<SkillHandled> {
        self.skills.handle(call)
    }

    /// `handlers::player::action::run` on the action wiring.
    fn player(
        &mut self,
        game: &mut Game,
        events: &mut D,
        run: PlayerRun<'_>,
    ) -> Option<PlayerOutcome> {
        Some(player::action::run(
            game,
            events,
            &run,
            HostFacts::default(),
        ))
    }

    /// `handlers::walk::run` (the path provider of the action wiring).
    fn walk(&mut self, game: &mut Game, events: &mut D, call: WalkCall) -> Option<WalkResult> {
        super::super::walk::run(game, events, call)
    }

    /// A player, monster or object in a room: the room's act and the
    /// unit's position (path, or the seam's staged position without the
    /// path provider). Items and missiles: none (an item's owner is not
    /// read here).
    fn player_gate(
        &mut self,
        _: &Game,
        events: &mut D,
        unit: UnitId,
    ) -> Option<crate::seams::PlayerGate> {
        let s = &events.action().sys;
        if !s.hooks.death.died.contains(&unit) {
            return None;
        }
        Some(crate::seams::PlayerGate {
            mode: s.units.get(unit)?.mode,
            uninterruptable: s.stats.has_state(unit, 0x36),
        })
    }

    fn live_facts(
        &mut self,
        game: &Game,
        events: &mut D,
        unit: UnitId,
    ) -> Option<crate::adapters::UnitFacts> {
        let e = game.lists.unit(unit)?;
        if !matches!(
            e.ty,
            d2_sim::units::UnitType::Player
                | d2_sim::units::UnitType::Monster
                | d2_sim::units::UnitType::Object
        ) {
            return None;
        }
        let act = game.lists.room(e.room()?)?.act;
        // A unit without a path reads (0, 0): not a position (the tick's
        // `unit_positions` skips it the same way).
        let hooks = events.action().hooks();
        if !hooks.path_has(unit) {
            return None;
        }
        let (x, y) = hooks.path_position(unit);
        Some(crate::adapters::UnitFacts {
            act,
            pos: crate::seams::Pos { x, y },
            owner: None,
        })
    }

    /// `d2_sim::wiring::action::vitals_sync::run` on the action wiring
    /// (on when `ActionHooks::enable_vitals_sync` ran).
    fn vitals_sync(
        &mut self,
        game: &mut Game,
        events: &mut D,
        client: SimClient,
        staged: (u16, u16),
        queued: bool,
    ) -> Option<Vec<Vec<u8>>> {
        vitals_sync::run(events.action(), game, client, staged, queued)
    }

    fn take_sent(&mut self, events: &mut D) -> Vec<(UnitId, Vec<u8>)> {
        events.action().hooks().x.take_sent()
    }

    /// `ActionHooks::set_host_tick` (no object state: nothing).
    fn host_tick(&mut self, events: &mut D, ms: u32) {
        events.action().sys.hooks.set_host_tick(ms);
    }

    fn fault(&mut self, fault: WorldFault) {
        self.faults.push(fault);
    }
}
