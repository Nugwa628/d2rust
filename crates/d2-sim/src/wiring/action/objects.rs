//! Objects and units, timers, the unit lists and the DRLG: [`ObjectView`]
//! implements [`ObjectWorld`] (and the extension traits, on their
//! defaults). The object control and tables live in [`ObjectState`]
//! ([`super::ActionHooks::objects`]), built at game creation
//! ([`super::ActionSim::create_objects`]) and lent to each object call
//! ([`with_objects`]).
//!
//! Real providers: the frame, GUIDs and the object lookup (unit lists),
//! the operator kind, mode, animation, seed and flags (unit records), the
//! update queue (lists), room and level (DRLG), position (path), timers
//! (game timer queue), the host tick ([`ObjectState::host_tick`], set by
//! the host: never read by `d2-sim`). Footprints, sounds, the key test,
//! the interact range, the cursor and player-data tests, allocation from
//! inside an object call and the staff-tomb level go to [`Pending`].
//!
//! Routes into the object module: unit allocation of type 2
//! ([`super::units`] `init_kind` → [`objects::create`]), the object
//! timer events (`object_event` → [`objects::object_event`]), the
//! waypoint mode changes ([`super::waypoints`] → [`objects::set_object_mode`]),
//! the monster door operate ([`super::ai`] → [`objects::operate_in_range`]),
//! the per-client update pass ([`super::dispatch`] →
//! [`objects::update_messages`]) and the C→S 0x13 object case
//! ([`super::ActionSim::operate_object_message`]). What the module hands
//! back (quest, waypoint, `todo` routes) goes to [`Pending::object_route`],
//! or, for a quest route of a host holding the quest control
//! ([`ObjectState::route_quests`]), to that host's queue
//! ([`QuestObjectCall`]).

use std::sync::Arc;

use crate::game::Game;
use crate::rng::Seed;
use crate::units::record::flags;
use crate::units::{RoomId, UnitId, UnitType};
use crate::world::objects::shrines::MissileRequest;
use crate::world::objects::{
    self, ChestWorld, Created, Dispatch, EventRun, MiscWorld, ObjectControl, ObjectError,
    ObjectTables, ObjectWorld, Operate, Operator, Preset, ShrineWorld, StateList, StateRequest,
    UpdateMessage,
};
use crate::world::quests::act2::q4::JerhynStep;

use super::{Pending, View, WiringError};
use crate::path::record::ObjectShape;
use crate::wiring::economy::drop_helpers;
use d2_data::tables::Objects;

/// The game's object state: the object control with the per-object data, the
/// tables, and the host inputs of an object call.
#[derive(Debug, Clone)]
pub struct ObjectState {
    /// The object control: per-object data and the object seeds.
    pub control: ObjectControl,
    /// The game's object seed: the result of one step of the game seed taken
    /// when the object state is created (sent in the S->C 0x03 message).
    pub obj_seed: u32,
    /// The object tables.
    pub tables: Arc<ObjectTables>,
    /// The host's tick count (an input the host sets,
    /// [`super::ActionHooks::set_host_tick`]).
    pub host_tick: u32,
    /// The allocation's (x, y) while [`View::create_object`] allocates
    /// (the per-kind init has no position argument).
    alloc_at: Option<(i32, i32)>,
    /// The quest routes for a host that holds the quest control
    /// ([`ObjectState::route_quests`]); `None` (the default): every
    /// route goes to [`Pending::object_route`].
    quest_calls: Option<Vec<QuestObjectCall>>,
}

/// A quest route of the object module (a quest init, operate or object
/// event 7), queued for the host that holds the quest control
/// (`d2_sim::wiring::economy::quest_objects`), with what the quest
/// function reads beyond the route: the object's class and the init's
/// room and position (the marker init's arguments).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestObjectCall {
    /// The route the object module handed back.
    pub route: ObjectRoute,
    /// The object's class.
    pub class: u16,
    /// The room of the init, when it has one.
    pub room: Option<RoomId>,
    /// The x position of the init.
    pub x: i32,
    /// The y position of the init.
    pub y: i32,
}

/// The quest control of a host, lent to the hooks
/// ([`super::ActionHooks::quest_host`]) so that the quest routes of the
/// object module run at the right point: a quest init inside the object's
/// allocation (after the unit's seed step, before the `PreOperate` draw and
/// the add-to-world), an operate inside the dispatch, object event 7 inside
/// the timer event. The provider is
/// `crate::wiring::economy::QuestLoan`.
pub trait QuestObjectHost<X> {
    /// Runs one route now on the game's parts; `Some`: a route that is not
    /// a known quest route, for [`Pending::object_route`].
    fn run(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        call: QuestObjectCall,
    ) -> Option<ObjectRoute>;
    /// Quest event 3 (CHANGEDLEVEL) from level `from` to `to` for `player`,
    /// run now: the per-client update's level change. Default: nothing.
    fn changed_level(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        player: UnitId,
        from: u32,
        to: u32,
    ) {
        let _ = (game, v, player, from, to);
    }
    /// The next Arcane Sanctuary dummy object class, for preset 582.
    /// Default: 307 (no quest chain).
    fn arcane_object(&mut self, game: &mut Game, v: &mut View<'_, X>) -> u16 {
        let _ = (game, v);
        307
    }
    /// The quest part of the town-leave refresh, run by the level warp before
    /// the act change. Default: nothing.
    fn town_leave(&mut self, game: &mut Game, v: &mut View<'_, X>, player: UnitId, to: u32) {
        let _ = (game, v, player, to);
    }
    /// The quest active test for the NPC AI's interact gate: `class` and
    /// `interact` are the NPC's monstats class and flag. True after the 0x8A
    /// send. Default: false.
    #[allow(clippy::too_many_arguments)]
    fn npc_wants_interact(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        player: UnitId,
        npc: UnitId,
        class: u16,
        interact: bool,
    ) -> bool {
        let _ = (game, v, player, npc, class, interact);
        false
    }
    /// Object init 13: if chain `chain`'s record exists, link `object` to it
    /// and answer true. Default: no record.
    fn object_link(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        object: UnitId,
        chain: u8,
    ) -> bool {
        let _ = (game, v, object, chain);
        false
    }
    /// The Jerhyn AI hooks: whether the palace is active, the palace NPC
    /// state (the AI unit at its path position `at`), whether a guard is
    /// moving and the success of [`JerhynStep::PlaceAt`]. Defaults: no quest
    /// state (active, (1, 0), not moving).
    fn jerhyn_palace_active(&mut self) -> bool {
        true
    }
    fn jerhyn_npc_state(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        at: (i32, i32),
    ) -> JerhynStep {
        let _ = (game, v, at);
        JerhynStep::Out(1, 0)
    }
    fn guard_moving(&mut self) -> bool {
        false
    }
    fn jerhyn_placed(&mut self) {}
    /// The Dark Wanderer's walk target and minion hook, called from its AI.
    /// Defaults: no target, no hook.
    fn wanderer_target(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        wanderer: UnitId,
    ) -> Option<(i32, i32)> {
        let _ = (game, v, wanderer);
        None
    }
    fn wanderer_minions(&mut self, game: &mut Game, v: &mut View<'_, X>, wanderer: UnitId) {
        let _ = (game, v, wanderer);
    }
    /// The palace guard AI hooks: whether the guard at its end position has
    /// the door open, and whether the blocker is open. Defaults: false.
    fn palace_door_open(&mut self) -> bool {
        false
    }
    fn palace_guard_aside(&mut self) -> bool {
        false
    }
    /// The guard's point, when the quest state has one. Default: none.
    fn palace_guard_point(&mut self) -> Option<(i32, i32)> {
        None
    }
    /// The map-AI store, called from the preset object placement after the
    /// object of `class` (459, 461 or 543) is created with the preset's path:
    /// (action, x, y) points. Default: nothing.
    fn map_ai_store(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        class: u16,
        path: Vec<(u32, i32, i32)>,
    ) {
        let _ = (game, v, class, path);
    }
    /// The Act 1 quest 4 calls of `cain1`'s NpcOutOfTown AI. `None`: no quest
    /// state here (the caller takes [`Pending`]'s).
    fn cain_portal(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        unit: UnitId,
        call: CainPortalCall,
    ) -> Option<CainPortalAnswer> {
        let _ = (game, v, unit, call);
        None
    }
    /// Whether the NPC's interaction list holds a node, i.e. a player is
    /// talking to it. Default: no.
    fn npc_interacting(&self, npc: UnitId) -> bool {
        let _ = npc;
        false
    }
    /// Whether `player` has a node in the NPC's interaction list. Default:
    /// no.
    fn npc_in_list(&self, npc: UnitId, player: UnitId) -> bool {
        let _ = (npc, player);
        false
    }
    /// Whether the player's item list holds an item of item type `itype`;
    /// `items`: the game's item store. Default: no inventory model, no.
    fn has_item_type(
        &mut self,
        items: &crate::wiring::economy::ItemStore,
        player: UnitId,
        itype: i16,
    ) -> bool {
        let _ = (items, player, itype);
        false
    }
    /// Adds a quest chain link now, inside its caller (monster init, item
    /// creation). False: not run here (the caller queues it on the Pending).
    /// Default: false.
    fn add_link(&mut self, game: &mut Game, v: &mut View<'_, X>, unit: UnitId, chain: u8) -> bool {
        let _ = (game, v, unit, chain);
        false
    }
    /// The quest kill parse of `victim` and `killer`, run now inside the
    /// kill. False: not run here (the caller queues it). Default: false.
    fn kill_parse(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        victim: UnitId,
        killer: Option<UnitId>,
    ) -> bool {
        let _ = (game, v, victim, killer);
        false
    }
    /// A quest call of an AI body (`QuestCall`), run now inside the think
    /// (Radament, the Summoner). `Some`: the call's answer; `None`: not run
    /// here (the caller asks the Pending). Default: none.
    fn ai_quest_call(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        unit: UnitId,
        call: crate::monsters::ai::QuestCall,
    ) -> Option<bool> {
        let _ = (game, v, unit, call);
        None
    }
    /// The special-monster boss hook (the Golden Bird's boss choice), run now
    /// inside the creation. False: not run here (the caller queues it).
    /// Default: false.
    fn boss_created(&mut self, game: &mut Game, v: &mut View<'_, X>, unit: UnitId) -> bool {
        let _ = (game, v, unit);
        false
    }
    /// The preset-boss hook, run now inside the creation. False: not run
    /// here. Default: false.
    fn preset_boss(&mut self, game: &mut Game, v: &mut View<'_, X>, unit: UnitId) -> bool {
        let _ = (game, v, unit);
        false
    }
    /// Anya's AI portal request, run now inside the think. False: not run
    /// here. Default: false.
    fn anya_open_portal(&mut self, game: &mut Game, v: &mut View<'_, X>, unit: UnitId) -> bool {
        let _ = (game, v, unit);
        false
    }
    /// For the host taking its parts back.
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any>;
}

/// One Act 1 quest 4 Cain-portal call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CainPortalCall {
    /// The setup call.
    Setup,
    /// Spawn the portal in town.
    SpawnTown,
    /// Spawn the portal outside town.
    SpawnOutside,
    /// Ask for the portal coordinates.
    Coords,
}

/// A [`CainPortalCall`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CainPortalAnswer {
    /// The call's yes/no answer.
    Ok(bool),
    /// The coordinates, when there are any.
    Coords(Option<(i32, i32)>),
}

/// What the object module handed back to its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectRoute {
    /// A created object whose init function is a quest, waypoint or
    /// `todo` function.
    Init { object: UnitId, created: Created },
    /// An operate dispatched to a quest, waypoint or `todo` function by a
    /// caller that does not run it here (the monster door operate; the
    /// 0x13 case hands the waypoint route to the caller instead).
    Operate(Dispatch),
    /// A quest or not-covered object timer event.
    Event { object: UnitId, run: EventRun },
    /// A preset class whose handler is not covered.
    Preset {
        room: RoomId,
        class: u32,
        x: i32,
        y: i32,
    },
}

/// What the C->S 0x13 object case decides before the operate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectReach {
    /// In range and unobstructed: the player is stopped (by the host's
    /// [`Pending::object_approach`]) and the operate runs now.
    Operate,
    /// Distance > 50: result 1.
    TooFar,
    /// Walk to the object; the operate on arrival is the host's.
    Walk,
}

/// The result of the C->S 0x13 object case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectCase {
    /// The handler's result code.
    Code(u32),
    /// Operate 23: the caller runs the waypoint handling (the waypoint
    /// tables live with the host); then result 0.
    Waypoint(Operate),
    /// Not in interact range or the line blocked: the caller runs the player
    /// to this object and repeats the case on arrival; result 0.
    Walk(UnitId),
}

/// Creates a monster (mode 1, spread -1, flags 0) for an object init, by the
/// host that holds the monster population.
pub trait SoulSpawner<X> {
    /// Creates a monster of `class` in `room` near (x, y).
    fn spawn_soul(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        room: RoomId,
        x: i32,
        y: i32,
        class: u16,
    ) -> Option<UnitId>;
}

/// The object code's view of a game.
pub struct ObjectView<'a, X> {
    pub game: &'a mut Game,
    pub v: View<'a, X>,
    /// Population's creation of a monster for an object init that runs
    /// inline (the trapped-soul placeholder): the world state that runs
    /// the room pass is not in the hooks while it does.
    pub spawner: Option<&'a mut (dyn SoulSpawner<X> + 'a)>,
    /// [`ObjectState::host_tick`] of the call.
    pub host_tick: u32,
    /// The object tables of the call (the `levels` rows of the chest
    /// drop).
    pub tables: Arc<ObjectTables>,
}

/// The Chebyshev distance of two units' path positions (sub-tiles), for
/// [`Pending::object_preview_range`]. D2RUST's own, unverified.
fn preview_distance<X: Pending>(v: &View<'_, X>, a: UnitId, b: UnitId) -> i32 {
    let (ax, ay) = v.h.path_position(a);
    let (bx, by) = v.h.path_position(b);
    (ax - bx).abs().max((ay - by).abs())
}

/// A shorter-lived [`View`] over the same parts.
pub fn reborrow<'b, X>(v: &'b mut View<'_, X>) -> View<'b, X> {
    View {
        units: &mut *v.units,
        stats: &mut *v.stats,
        data: v.data,
        h: &mut *v.h,
    }
}

/// Runs `f` with the object control and tables lent out of
/// [`super::ActionHooks::objects`] and an [`ObjectView`] over `game` and
/// `v`. `None`: the game has no object state (not created), or it is
/// lent (a re-entrant call; an error is logged).
pub fn with_objects<X: Pending, R>(
    game: &mut Game,
    v: &mut View<'_, X>,
    f: impl FnOnce(&mut ObjectControl, &ObjectTables, &mut ObjectView<'_, X>) -> R,
) -> Option<R> {
    with_objects_spawner(game, v, None, f)
}

/// [`with_objects`] with the host's [`SoulSpawner`].
pub fn with_objects_spawner<'s, X: Pending, R>(
    game: &mut Game,
    v: &mut View<'_, X>,
    spawner: Option<&'s mut (dyn SoulSpawner<X> + 's)>,
    f: impl FnOnce(&mut ObjectControl, &ObjectTables, &mut ObjectView<'_, X>) -> R,
) -> Option<R> {
    if v.h.objects_out {
        v.h.errors.push(WiringError::Reentrant("objects"));
        return None;
    }
    let mut st = v.h.objects.take()?;
    v.h.objects_out = true;
    let t = st.tables.clone();
    let r = {
        let mut ov = ObjectView {
            game,
            v: reborrow(v),
            spawner: spawner.map(|s| -> &mut dyn SoulSpawner<X> { s }),
            host_tick: st.host_tick,
            tables: t.clone(),
        };
        f(&mut st.control, &t, &mut ov)
    };
    v.h.objects = Some(st);
    v.h.objects_out = false;
    Some(r)
}

/// Logs an object module error.
fn log<T, X>(v: &mut View<'_, X>, r: Result<T, ObjectError>) -> Option<T> {
    match r {
        Ok(t) => Some(t),
        Err(e) => {
            v.h.errors.push(WiringError::Object(e));
            None
        }
    }
}

/// State 102 (`just_portaled`).
const JUST_PORTALED_STATE: u16 = 102;

impl ObjectState {
    /// Creates the object state, taking one step of `game_seed` for the
    /// object seed.
    pub fn new(game_seed: &mut Seed, tables: Arc<ObjectTables>) -> Self {
        let (control, obj_seed) = ObjectControl::new(game_seed, &tables);
        Self {
            control,
            obj_seed,
            tables,
            host_tick: 0,
            alloc_at: None,
            quest_calls: None,
        }
    }

    /// Queues the quest routes for the host instead of handing them to
    /// [`Pending::object_route`] (a host with a quest control turns this
    /// on; idempotent).
    pub fn route_quests(&mut self) {
        self.quest_calls.get_or_insert_with(Vec::new);
    }

    /// The queued quest routes, in queue order (empty when off).
    pub fn take_quest_calls(&mut self) -> Vec<QuestObjectCall> {
        self.quest_calls
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }
}

impl<X: Pending> View<'_, X> {
    /// Hands a route back: a quest route to the host's queue when it is
    /// on ([`ObjectState::route_quests`]), anything else (and every route
    /// when it is off) to [`Pending::object_route`].
    pub fn object_route(
        &mut self,
        game: &mut Game,
        route: ObjectRoute,
        room: Option<RoomId>,
        (x, y): (i32, i32),
    ) {
        let object = match route {
            ObjectRoute::Init {
                object,
                created:
                    Created {
                        init: objects::Route::Quest,
                        ..
                    },
            } => Some(object),
            ObjectRoute::Operate(Dispatch::Quest(op)) => Some(op.object),
            ObjectRoute::Event {
                object,
                run: EventRun::Quest,
            } => Some(object),
            _ => None,
        };
        let class = object
            .and_then(|o| self.units.get(o))
            .map(|r| u16::try_from(r.class).unwrap_or(u16::MAX));
        let Some(class) = class else {
            return self.h.x.object_route(game, route);
        };
        let call = QuestObjectCall {
            route,
            class,
            room,
            x,
            y,
        };
        if self.h.quest_host.is_some() {
            return self.run_quest_host(game, call);
        }
        let out = self.h.quest_host_out;
        let queue = self.h.objects.as_mut().and_then(|s| {
            if out {
                // Raised inside a lent route: run right after it.
                s.route_quests();
            }
            s.quest_calls.as_mut()
        });
        match queue {
            Some(q) => q.push(call),
            None => self.h.x.object_route(game, route),
        }
    }

    /// Runs `call` on the lent quest host now, then the routes it raised
    /// (queued while it ran: a quest function that allocates a quest
    /// object), in order.
    fn run_quest_host(&mut self, game: &mut Game, call: QuestObjectCall) {
        let Some(mut host) = self.h.quest_host.take() else {
            return;
        };
        self.h.quest_host_out = true;
        let mut calls = vec![call];
        while !calls.is_empty() {
            for c in calls {
                if let Some(r) = host.run(game, self, c) {
                    self.h.x.object_route(game, r);
                }
            }
            calls = self
                .h
                .objects
                .as_mut()
                .map(|s| s.take_quest_calls())
                .unwrap_or_default();
        }
        self.h.quest_host_out = false;
        self.h.quest_host = Some(host);
    }

    /// The per-kind init of an object allocation (the object data and the
    /// object init) on the object state; a game without one keeps the
    /// default (nothing). A quest, waypoint or `todo` init goes to [`Pending::object_route`].
    ///
    /// The init runs before the unit is added to the world: the unit is in
    /// no list and has no path record yet. Its room is the allocation's
    /// ([`View::init_room`]); (x, y) the allocation's when it went through
    /// [`View::create_object`], else (0, 0).
    pub fn object_init(&mut self, game: &mut Game, unit: UnitId) {
        // An allocation from inside an object call: the caller holds the
        // control and runs the allocation itself (`objects::allocate`).
        if self.h.objects_out {
            return;
        }
        let Some(r) = self.units.get(unit) else {
            return;
        };
        let (class, mode) = (r.class, r.mode);
        let guid = r.guid;
        let room = self.init_room(game, unit);
        let (x, y) = self
            .h
            .objects
            .as_ref()
            .and_then(|s| s.alloc_at)
            .unwrap_or((0, 0));
        // Class and mode bounds are the dispatcher's fatal checks.
        let class = u16::try_from(class).unwrap_or(u16::MAX);
        let mode = u8::try_from(mode).unwrap_or(u8::MAX);
        let r = with_objects(game, self, |ctl, t, w| {
            objects::create_init(ctl, t, w, unit, class, guid, room, mode, x, y)
        });
        let Some(created) = r.and_then(|r| log(self, r)) else {
            return;
        };
        // Init function 17 (the waypoint init) on the hooks' waypoint tables
        // and arrival list, inside the creation.
        if created.init == objects::Route::Waypoint && self.h.waypoint_init.is_some() {
            let level = room.and_then(|r| self.h.drlg.level_id(game, r));
            let facts = crate::world::waypoints::ObjectFacts {
                guid,
                class,
                mode,
                room,
                level,
                x,
                y,
            };
            self.waypoint_init(game, unit, &facts);
            let r = with_objects(game, self, |ctl, t, w| {
                objects::create_rest(ctl, t, w, unit, mode)
            });
            if let Some(r) = r {
                log(self, r);
            }
            return;
        }
        let handed = !matches!(created.init, objects::Route::Here | objects::Route::Null);
        let route = ObjectRoute::Init {
            object: unit,
            created,
        };
        // A quest init on a lent quest host runs inside the creation, before
        // the add-to-world; any other route is handed back after it.
        let now = handed && created.init == objects::Route::Quest && self.h.quest_host.is_some();
        if now {
            self.object_route(game, route, room, (x, y));
        }
        let r = with_objects(game, self, |ctl, t, w| {
            objects::create_rest(ctl, t, w, unit, mode)
        });
        if let Some(r) = r {
            log(self, r);
        }
        if handed && !now {
            self.object_route(game, route, room, (x, y));
        }
    }

    /// Init function 17 (the waypoint init) of the object `unit` being
    /// created (`facts`: its class, mode, room, level and position at the
    /// init), on `ActionHooks::waypoint_init` and `ActionHooks::arrivals`.
    fn waypoint_init(
        &mut self,
        game: &mut Game,
        unit: UnitId,
        facts: &crate::world::waypoints::ObjectFacts,
    ) {
        let Some(data) = self.h.waypoint_init.clone() else {
            return;
        };
        let mut arrivals = std::mem::take(&mut self.h.arrivals);
        let mut w = super::waypoints::WaypointView {
            game,
            v: View::of(self.units, self.stats, self.data, self.h),
        };
        data.init_object(&mut w, &mut arrivals, unit, facts);
        self.h.arrivals = arrivals;
    }

    /// An object of `class` allocated in `room` at (x, y) in `mode`
    /// (a type 2 allocation), its init run ([`View::object_init`]); preset
    /// classes 574-582 go through [`objects::create_preset`]. `None`: no
    /// object state (the caller keeps its [`Pending`] answer), or nothing
    /// allocated.
    pub fn create_object(
        &mut self,
        game: &mut Game,
        room: RoomId,
        class: u32,
        x: i32,
        y: i32,
        mode: u8,
    ) -> Option<UnitId> {
        self.create_object_flags(game, room, class, x, y, mode, true)
    }

    /// [`Self::create_object`] with the allocation's flags bit 1 as
    /// `add`: the allocation sets the mode (its second flag argument)
    /// before the object init and adds the unit to the world only
    /// when flags & 1; clear, the object is returned in no list.
    #[allow(clippy::too_many_arguments)]
    pub fn create_object_flags(
        &mut self,
        game: &mut Game,
        room: RoomId,
        class: u32,
        x: i32,
        y: i32,
        mode: u8,
        add: bool,
    ) -> Option<UnitId> {
        let st = self.h.objects.as_mut()?;
        if class > u32::from(objects::CLASS_BOUND) {
            let level = self.h.drlg.level_id(game, room).unwrap_or(0);
            let r = with_objects(game, self, |ctl, t, w| {
                objects::create_preset(ctl, t, w, room, level, class, x, y, mode)
            })?;
            return match log(self, r)? {
                Preset::Object(u) => u,
                Preset::None => None,
                // Preset 582: the class comes from the quest host, allocated
                // in mode 0.
                Preset::NotCovered(582) if self.h.quest_host.is_some() => {
                    let mut host = self.h.quest_host.take()?;
                    let class = host.arcane_object(game, self);
                    self.h.quest_host = Some(host);
                    self.create_object(game, room, u32::from(class), x, y, 0)
                }
                Preset::NotCovered(class) => {
                    self.h
                        .x
                        .object_route(game, ObjectRoute::Preset { room, class, x, y });
                    None
                }
            };
        }
        st.alloc_at = Some((x, y));
        let req = crate::units::lifecycle::AllocRequest {
            ty: UnitType::Object,
            class,
            room: Some(room),
            add,
            fixed_guid: None,
            mode: u32::from(mode),
            allied: false,
        };
        // Without flag 1 the unit stays out of the world.
        let u = if add {
            self.allocate(game, &req, x, y)
        } else {
            self.allocate_unadded(game, &req)
        };
        if let Some(st) = self.h.objects.as_mut() {
            st.alloc_at = None;
        }
        u
    }

    /// Frees the collision footprint of `object` outside an object call (the
    /// quest code's collision free): [`apply_object_footprint`] with the
    /// object state's tables;
    /// `false`: no object state or `object` is not an object.
    pub fn free_object_footprint(&mut self, game: &Game, object: UnitId) -> bool {
        let Some(st) = self.h.objects.as_ref() else {
            return false;
        };
        let Some(r) = self.units.get(object).filter(|r| r.ty == UnitType::Object) else {
            return false;
        };
        let Ok(o) = st.tables.object(r.class as u16).cloned() else {
            return false;
        };
        let room = game.lists.unit(object).and_then(|e| e.room());
        let (x, y) = self.h.path_position(object);
        apply_object_footprint(&mut self.h.drlg, &o, room, x, y, false);
        true
    }

    /// An object timer event on the object state.
    pub fn object_event(&mut self, game: &mut Game, unit: UnitId, ev: u8) {
        let r = with_objects(game, self, |ctl, t, w| {
            objects::object_event(ctl, t, w, unit, ev)
        });
        let Some(run) = r.and_then(|r| log(self, r)) else {
            return;
        };
        if run != EventRun::Done {
            let room = game.lists.unit(unit).and_then(|e| e.room());
            let at = self.h.path_position(unit);
            self.object_route(game, ObjectRoute::Event { object: unit, run }, room, at);
        }
    }

    /// Object population of the room `info` describes, on the object state.
    /// `None`: no object state (the caller keeps its [`Pending`] answer).
    pub fn populate_objects(
        &mut self,
        game: &mut Game,
        info: &objects::populate::RoomInfo,
    ) -> Option<objects::populate::Populated> {
        self.h.objects.as_ref()?;
        let r = with_objects(game, self, |ctl, t, w| {
            objects::populate::populate_room(ctl, t, w, info)
        })?;
        log(self, r)
    }

    /// [`Self::populate_objects`] with the host's [`SoulSpawner`].
    pub fn populate_objects_with(
        &mut self,
        game: &mut Game,
        info: &objects::populate::RoomInfo,
        spawner: &mut dyn SoulSpawner<X>,
    ) -> Option<objects::populate::Populated> {
        self.h.objects.as_ref()?;
        let r = with_objects_spawner(game, self, Some(spawner), |ctl, t, w| {
            objects::populate::populate_room(ctl, t, w, info)
        })?;
        log(self, r)
    }

    /// The object mode change of an object with object data
    /// ([`objects::set_object_mode`]); `false`: no object state or no
    /// data for `object` (the caller keeps its [`Pending`] answer).
    pub fn object_set_mode(&mut self, game: &mut Game, object: UnitId, mode: u8) -> bool {
        let known = self
            .h
            .objects
            .as_ref()
            .is_some_and(|s| s.control.data.contains_key(&object));
        if !known {
            return false;
        }
        let r = with_objects(game, self, |ctl, t, w| {
            objects::set_object_mode(ctl, t, w, object, mode)
        });
        if let Some(r) = r {
            log(self, r);
        }
        true
    }

    /// The operate entry by `operator` on the object with `guid`: the
    /// entry's result and the dispatch. `None`: no
    /// object state.
    pub fn operate_object(
        &mut self,
        game: &mut Game,
        operator: Option<UnitId>,
        guid: u32,
    ) -> Option<(i32, Option<Dispatch>)> {
        let r = with_objects(game, self, |ctl, t, w| {
            objects::operate_in_range(ctl, t, w, operator, guid)
        })?;
        self.flush_portal_removals(game);
        log(self, r)
    }

    /// The reach steps of the C->S 0x13 object case: the unit distance > 50
    /// is too far; the interact range ([`View::object_in_reach`]) false or
    /// the line test (flags 0x804) blocked means walk; else operate. `None`
    /// without the path provider (the host's seams decide).
    pub(crate) fn object_reach(
        &self,
        game: &Game,
        player: UnitId,
        object: UnitId,
    ) -> Option<ObjectReach> {
        let paths = self.h.paths.as_ref()?;
        let pt = |u: UnitId| {
            let (x, y) = self.h.path_position(u);
            crate::path::Point { x, y }
        };
        let dist = crate::path::walk::geom::unit_distance(
            &paths.tables,
            pt(player),
            self.path_size(player),
            pt(object),
            self.path_size(object),
        );
        if dist > 50 {
            return Some(ObjectReach::TooFar);
        }
        if !self.object_in_reach(player, object, dist) {
            return Some(ObjectReach::Walk);
        }
        // The object's own footprint does not block the line to it: a closed
        // door carries the door bit on its own cells, and a player standing
        // diagonally next to it (distance 2 on both axes) has the line
        // pulled onto the door's centre cell.
        let blocked = match self.units_line_block_point(game, player, object, 0x804)? {
            None => false,
            Some(cell) => !self.in_object_footprint(object, (cell.x, cell.y)),
        };
        Some(if blocked {
            ObjectReach::Walk
        } else {
            ObjectReach::Operate
        })
    }

    /// Whether the cell lies on the footprint box of `object`
    /// (`SizeX` x `SizeY` around its position, the box [`in_reach_box`] uses).
    fn in_object_footprint(&self, object: UnitId, cell: (i32, i32)) -> bool {
        let (ox, oy) = self.h.path_position(object);
        let (w, h) = match self.path_shape(object) {
            Some(crate::path::record::UnitShape::Object(o)) => (o.size_x as i32, o.size_y as i32),
            _ => return false,
        };
        let (bx, by) = (ox - w / 2, oy - h / 2);
        (bx..bx + w.max(1)).contains(&cell.0) && (by..by + h.max(1)).contains(&cell.1)
    }

    /// Whether the player is in interact range of the object: `dist` is the
    /// unit distance. Distance 0 is in range. Else the player's position is
    /// tested against the object's box, the position minus half the sizes
    /// (`SizeX` x `SizeY`, integer halves): an empty box (a size < 1) takes
    /// +-1 around the point; else the box plus 2 on every side, with the four
    /// corner cells of that ring cut off (the player's size is then 2, never
    /// above it).
    fn object_in_reach(&self, player: UnitId, object: UnitId, dist: i32) -> bool {
        if dist == 0 {
            return true;
        }
        let (px, py) = self.h.path_position(player);
        let (ox, oy) = self.h.path_position(object);
        let (w, h) = match self.path_shape(object) {
            Some(crate::path::record::UnitShape::Object(o)) => (o.size_x as i32, o.size_y as i32),
            _ => (0, 0),
        };
        in_reach_box((px, py), (ox, oy), (w, h), self.path_size(player))
    }

    /// The C->S 0x13 object case for `player` and the object with `guid`:
    /// object missing gives 1; mode >= 8 gives 3;
    /// [`Pending::object_approach`] (distance > 50 gives 1; walk); in range
    /// runs the operate entry. Operate 23 is handed back
    /// ([`ObjectCase::Waypoint`]); quest and `todo` routes go to
    /// [`Pending::object_route`]. `None`: no object state.
    ///
    /// Result after the operate: the entry's result 0 gives 3, else 0; after
    /// the walk 0.
    pub fn object_message(
        &mut self,
        game: &mut Game,
        player: UnitId,
        guid: u32,
    ) -> Option<ObjectCase> {
        self.h.objects.as_ref()?;
        let Some(object) = game.lists.find_unit(UnitType::Object, guid) else {
            return Some(ObjectCase::Code(1));
        };
        if self.units.get(object).map_or(0, |r| r.mode) >= u32::from(objects::MODE_BOUND) {
            return Some(ObjectCase::Code(3));
        }
        let reach = match self.object_reach(game, player, object) {
            Some(r) => r,
            None => match self.h.x.object_preview_range() {
                Some(r) => {
                    // D2RUST's own, unverified: the preview's reach test.
                    let d = {
                        let (px, py) = self.h.path_position(player);
                        let (ox, oy) = self.h.path_position(object);
                        (px - ox).abs().max((py - oy).abs())
                    };
                    if d > 50 {
                        ObjectReach::TooFar
                    } else if d > r {
                        ObjectReach::Walk
                    } else {
                        ObjectReach::Operate
                    }
                }
                None => self.h.x.object_approach(game, player, object),
            },
        };
        match reach {
            ObjectReach::TooFar => return Some(ObjectCase::Code(1)),
            ObjectReach::Walk => return Some(ObjectCase::Walk(object)),
            // The player's path is stopped before the operate: a player still
            // walking toward the object stands at its next path step.
            ObjectReach::Operate => {
                if crate::wiring::path::monsters::stop_path(self.h, player).is_none() {
                    self.h.x.stop_path(player);
                }
            }
        }
        let (result, d) = self.operate_object(game, Some(player), guid)?;
        // The entry's result 0 (object gone) gives 3.
        if result == 0 {
            return Some(ObjectCase::Code(3));
        }
        Some(match d {
            Some(Dispatch::Waypoint(op)) => ObjectCase::Waypoint(op),
            Some(d @ (Dispatch::Quest(_) | Dispatch::NotCovered(_))) => {
                let room = game.lists.unit(object).and_then(|e| e.room());
                let at = self.h.path_position(object);
                self.object_route(game, ObjectRoute::Operate(d), room, at);
                ObjectCase::Code(0)
            }
            Some(Dispatch::Done(_)) | None => ObjectCase::Code(0),
        })
    }

    /// The unit distance between `a` and `b` on the path records.
    pub(super) fn object_unit_distance(&self, a: UnitId, b: UnitId) -> i32 {
        let Some(paths) = self.h.paths.as_ref() else {
            return i32::MAX;
        };
        let pt = |u: UnitId| {
            let (x, y) = self.h.path_position(u);
            crate::path::Point { x, y }
        };
        crate::path::walk::geom::unit_distance(
            &paths.tables,
            pt(a),
            self.path_size(a),
            pt(b),
            self.path_size(b),
        )
    }

    /// Interact range ([`objects::interact_range`]) on the path positions, the operator's size and the object's
    /// `objects.txt` `SizeX` / `SizeY`. `None` without the path provider
    /// or an object row.
    pub fn object_range(&self, operator: UnitId, object: UnitId) -> Option<bool> {
        self.h.paths.as_ref()?;
        let class = self
            .units
            .get(object)
            .filter(|r| r.ty == UnitType::Object)?
            .class;
        let row = self.h.objects.as_ref()?.tables.object(class as u16).ok()?;
        Some(objects::interact_range(
            self.h.path_position(operator),
            self.path_size(operator),
            self.h.path_position(object),
            (row.sizex as i32, row.sizey as i32),
            self.object_unit_distance(operator, object),
        ))
    }

    /// The object update pass for one queued object and the client of
    /// `receiver`: S->C 0x0E / 0x4D sent to `receiver` ([`Pending::send`]),
    /// 0x60 likewise, then the extras ([`MiscWorld::update_extras`]).
    /// `false`: not an object with object data (nothing ran).
    pub fn object_update(&mut self, game: &mut Game, receiver: UnitId, unit: UnitId) -> bool {
        let known = self
            .h
            .objects
            .as_ref()
            .is_some_and(|s| s.control.data.contains_key(&unit));
        if !known {
            return false;
        }
        let r = with_objects(game, self, |ctl, t, w| {
            let msgs = objects::update_messages(ctl, t, w, unit);
            w.update_extras(unit);
            msgs
        });
        let Some(msgs) = r.and_then(|r| log(self, r)) else {
            return true;
        };
        for m in msgs {
            match m {
                UpdateMessage::State(b) => self.h.x.send(receiver, &b),
                UpdateMessage::Shrine(b) => self.h.x.send(receiver, &b),
                UpdateMessage::Portal(b) => self.h.x.send(receiver, &b),
            }
        }
        // Flag 0x100 sends the hover message (the overhead 0x26 form 5 /
        // 0x76).
        let (flags, guid) = (
            self.units.get(unit).map_or(0, |r| r.flags),
            game.lists.unit(unit).map_or(0, |e| e.guid),
        );
        if flags & objects::oflags::HOVER_FREED != 0 {
            self.overhead_message(receiver, unit, UnitType::Object as u8, guid);
        }
        true
    }

    /// objects.txt `MonsterOK` of a door; `None`: no object
    /// state or no data.
    pub fn object_monster_ok(&self, door: UnitId) -> Option<bool> {
        let st = self.h.objects.as_ref()?;
        let class = st.control.data.get(&door)?.class;
        st.tables.object(class).ok().map(|o| o.monsterok != 0)
    }
}

/// The interact-range box test for a player at
/// `p` (size `psize`) and an object at `o` of size `w` × `h`, the unit
/// distance being nonzero. The box corner is `o` minus half the size
/// (integer halves). An empty box (w or h < 1) takes ±1 around the corner
/// point; else the box plus 2 on every side, and a player of size ≤ 2
/// loses the four corner cells of that ring (the rows above and below the
/// box keep x within ±1 of it).
pub fn in_reach_box(p: (i32, i32), o: (i32, i32), size: (i32, i32), psize: i32) -> bool {
    let (w, h) = size;
    let (ox, oy) = (o.0 - w / 2, o.1 - h / 2);
    let (px, py) = p;
    if w < 1 || h < 1 {
        return px >= ox - 1 && px <= ox + 1 && py >= oy - 1 && py <= oy + 1;
    }
    if px < ox - 2 || px > ox + w + 2 || py < oy - 2 || py > oy + h + 2 {
        return false;
    }
    if psize > 2 {
        return true;
    }
    if py < oy - 1 || py > oy + h + 1 {
        px >= ox - 1 && px <= ox + w + 1
    } else {
        true
    }
}

/// The placement inputs of an objects row: size `SizeX` x `SizeY`, the
/// footprint mask from `IsDoor`, `BlocksVis`, `BlockMissile`, `SubClass`, and
/// `HasCollision0..7`.
pub fn object_shape(o: &Objects) -> ObjectShape {
    ObjectShape {
        size_x: o.sizex,
        size_y: o.sizey,
        is_door: o.isdoor != 0,
        blocks_vis: o.blocksvis != 0,
        block_missile: o.blockmissile != 0,
        sub_class: u32::from(o.subclass),
        has_collision: [
            o.hascollision0,
            o.hascollision1,
            o.hascollision2,
            o.hascollision3,
            o.hascollision4,
            o.hascollision5,
            o.hascollision6,
            o.hascollision7,
        ]
        .map(|b| b != 0),
    }
}

/// The object footprint stamp (`set`) and free: the `SizeX` x `SizeY` box
/// with the class's footprint mask set or cleared at (x, y), cells looked up
/// from `room`; no room does nothing. The free is a plain box clear with no
/// `HasCollision` test; its callers test `HasCollision` themselves.
pub fn apply_object_footprint(
    drlg: &mut super::DrlgWorld,
    o: &Objects,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    set: bool,
) {
    let s = object_shape(o);
    crate::path::collision::box_apply(drlg, room, x, y, (s.size_x, s.size_y), s.foot_mask(), set);
}

impl<X: Pending> ObjectView<'_, X> {
    /// The objects row of an object unit's class (unit record).
    fn object_row(&self, unit: UnitId) -> Option<Objects> {
        let r = self.v.units.get(unit)?;
        if r.ty != UnitType::Object {
            return None;
        }
        self.tables.object(r.class as u16).ok().cloned()
    }

    /// Runs a drop helper ([`crate::wiring::economy::drop_helpers`]) with the drop state of
    /// [`super::ActionHooks::object_drops`] lent out and the object
    /// tables' `levels`; `None`: the game has no drop state. With the
    /// path provider's field (every play game) the spot is the floor
    /// search; without it (field-less hosts, D2RUST's own) the
    /// start spot as is ([`crate::wiring::economy::StartSpot`]).
    fn with_drops<R>(
        &mut self,
        f: impl FnOnce(
            &mut super::ActionHooks<X>,
            &mut crate::units::hooks::Sim<'_>,
            &mut crate::wiring::economy::DeathDrops,
            &[d2_data::tables::Levels],
            &mut crate::wiring::economy::StartSpot,
        ) -> R,
    ) -> Option<R> {
        let mut d = self.v.h.object_drops.take()?;
        let t = self.tables.clone();
        let out = {
            let mut sim = crate::units::hooks::Sim {
                game: &mut *self.game,
                units: &mut *self.v.units,
                stats: &mut *self.v.stats,
                data: self.v.data,
            };
            f(
                &mut *self.v.h,
                &mut sim,
                &mut d,
                &t.levels,
                &mut crate::wiring::economy::StartSpot,
            )
        };
        self.v.h.object_drops = Some(d);
        Some(out)
    }

    fn record(&mut self, u: UnitId) -> Option<&mut crate::units::record::UnitRecord> {
        let r = self.v.units.get_mut(u);
        if r.is_none() {
            self.v.h.errors.push(WiringError::Unit(
                crate::units::modes::UnitError::UnknownUnit(u),
            ));
        }
        r
    }
}

impl<X: Pending> ObjectWorld for ObjectView<'_, X> {
    fn frame(&self) -> i32 {
        self.game.frame
    }
    fn tick_rate(&self) -> crate::tick::TickRate {
        self.game.tick_rate
    }
    fn host_tick(&self) -> u32 {
        self.host_tick
    }
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.v.h.rules
    }
    fn guid(&self, unit: UnitId) -> u32 {
        self.game.lists.unit(unit).map_or(0, |e| e.guid)
    }
    /// Finds the object unit with `guid`.
    fn find_object(&self, guid: u32) -> Option<UnitId> {
        self.game.lists.find_unit(UnitType::Object, guid)
    }
    fn operator(&self, unit: UnitId) -> Operator {
        match self.v.units.get(unit) {
            Some(r) if r.ty == UnitType::Player => Operator::Player(r.class as u8),
            Some(r) if r.ty == UnitType::Monster => Operator::Monster,
            _ => Operator::Other,
        }
    }
    fn mode(&self, unit: UnitId) -> u8 {
        self.v.units.get(unit).map_or(0, |r| r.mode as u8)
    }
    /// Writes the mode, sets unit flag 0x1, and queues the update when
    /// `queue` is set.
    fn write_mode(&mut self, unit: UnitId, mode: u8, queue: bool) {
        let Some(r) = self.record(unit) else {
            return;
        };
        r.mode = u32::from(mode);
        r.flags |= flags::CHANGED;
        if queue {
            self.queue_update(unit);
        }
    }
    fn set_anim(&mut self, unit: UnitId, frame_count: i32, frame: i32, speed: i16) {
        if let Some(r) = self.record(unit) {
            r.anim.frame_count = frame_count;
            r.anim.frame = frame;
            r.anim.speed = speed;
        }
    }
    fn unit_seed(&mut self, unit: UnitId) -> Option<&mut Seed> {
        self.v.units.get_mut(unit).map(|r| &mut r.seed)
    }
    fn flags(&self, unit: UnitId) -> u32 {
        self.v.units.get(unit).map_or(0, |r| r.flags)
    }
    fn set_flags(&mut self, unit: UnitId, f: u32) {
        if let Some(r) = self.record(unit) {
            r.flags = f;
        }
    }
    fn queue_update(&mut self, unit: UnitId) {
        if let Err(e) = self.game.lists.queue_update(unit) {
            self.v.unit_error(crate::game::GameError::from(e).into());
        }
    }
    fn room(&self, unit: UnitId) -> Option<RoomId> {
        self.game.lists.unit(unit).and_then(|e| e.room())
    }
    /// The level of the unit's room; during its allocation's init, the
    /// allocation's room ([`View::init_room`]: the init runs before the unit
    /// is linked into the world).
    fn level(&self, unit: UnitId) -> Option<u32> {
        let room = self.v.init_room(self.game, unit)?;
        self.v.h.drlg.level_id(self.game, room)
    }
    fn room_level(&self, room: RoomId) -> Option<u32> {
        self.v.h.drlg.level_id(self.game, room)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.v.h.path_position(unit)
    }
    /// Schedules timer event `ev` for the unit at `frame`.
    fn schedule(&mut self, unit: UnitId, ev: u8, frame: i32) {
        if let Err(e) = self
            .game
            .schedule_event(unit, u32::from(ev), frame, None, 0, 0)
        {
            self.v.unit_error(e.into());
        }
    }
    /// Cancels all of the unit's timers.
    fn cancel_timers(&mut self, unit: UnitId) {
        self.game.timers.cancel_unit_timers(unit);
    }
    /// Stamps the object's footprint on the act DRLG's collision grids.
    fn stamp_footprint(&mut self, unit: UnitId, room: Option<RoomId>, x: i32, y: i32) {
        let Some(o) = self.object_row(unit) else {
            return;
        };
        apply_object_footprint(&mut self.v.h.drlg, &o, room, x, y, true);
    }
    /// Frees the object's footprint on the act DRLG's collision grids, at its
    /// room and position.
    fn free_footprint(&mut self, unit: UnitId) {
        let Some(o) = self.object_row(unit) else {
            return;
        };
        let room = self.room(unit);
        let (x, y) = self.position(unit);
        apply_object_footprint(&mut self.v.h.drlg, &o, room, x, y, false);
    }
    /// Queues sound `id` ([`crate::units::sound::queue_sound`]). `now`: also
    /// send it at once (the chest's key sound) to the unit's own client when
    /// the unit is a player.
    fn sound(&mut self, unit: UnitId, id: u8, to: Option<UnitId>, now: bool) {
        if let Err(e) = crate::units::sound::queue_sound(self.game, unit, u16::from(id), to) {
            self.v.unit_error(crate::game::GameError::from(e).into());
        }
        let player = self
            .game
            .lists
            .unit(unit)
            .is_some_and(|e| e.ty == UnitType::Player);
        if now && player {
            // The sound slot and flag 0x400 stay set after the at-once send
            // (only the room clean-up clears them), so the player's next unit
            // update sends the event a second time.
            if let Some(m) = crate::units::sound::sound_message(self.game, unit, unit) {
                self.v.h.x.send(unit, &m);
            }
        }
    }
    fn key_test(&mut self, player: UnitId) -> bool {
        self.v.h.x.object_key_test(player)
    }
    fn in_interact_range(&self, operator: UnitId, object: UnitId) -> bool {
        match self.v.h.x.object_preview_range() {
            Some(r) => preview_distance(&self.v, operator, object) <= r,
            None => self
                .v
                .object_range(operator, object)
                .unwrap_or_else(|| self.v.h.x.object_in_range(self.game, operator, object)),
        }
    }
    /// Whether the interact info on the player's unit record is active.
    fn interact_active(&self, player: UnitId) -> bool {
        self.v.units.get(player).is_some_and(|r| r.interact.active)
    }
    fn player_busy(&self, player: UnitId) -> bool {
        self.v.h.x.object_player_busy(player)
    }
    fn cursor_item(&self, player: UnitId) -> bool {
        self.v.h.x.object_cursor_item(player)
    }
    /// An allocation from inside an object call (presets, fire objects): the
    /// unit only; [`View::object_init`] sees the control lent and leaves the
    /// rest of the allocation to [`objects::allocate`].
    fn allocate_object(
        &mut self,
        room: RoomId,
        class: u16,
        x: i32,
        y: i32,
        mode: u8,
    ) -> Option<UnitId> {
        let req = crate::units::lifecycle::AllocRequest {
            ty: UnitType::Object,
            class: u32::from(class),
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: u32::from(mode),
            allied: false,
        };
        self.v.allocate(self.game, &req, x, y)
    }
    /// The add-to-world step of an [`Self::allocate_object`] unit, after its
    /// init.
    fn add_object(&mut self, obj: UnitId, room: RoomId, x: i32, y: i32) {
        let req = crate::units::lifecycle::AllocRequest {
            ty: UnitType::Object,
            class: 0,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 0,
            allied: false,
        };
        if !self.v.add_allocated(self.game, obj, &req, x, y) {
            return;
        }
        // The object footprint added with the unit: the object state is lent
        // here, so `View::path_place` could not read the row; stamp it from
        // the lent tables when `HasCollision[mode]` is non-zero.
        let Some(mode) = self.v.units.get(obj).map(|r| r.mode) else {
            return;
        };
        if self.v.h.paths.is_none() {
            return;
        }
        if let Some(o) = self.object_row(obj) {
            if object_shape(&o).collides_in(mode) {
                let (room, (px, py)) = (self.room(obj), self.position(obj));
                apply_object_footprint(&mut self.v.h.drlg, &o, room, px, py, true);
            }
        }
    }
    fn staff_tomb_level(&self) -> u32 {
        self.v.h.x.object_staff_tomb()
    }
    /// The room's sub-tile box on the act DRLG.
    fn soul_box(&mut self, room: RoomId) -> Option<crate::drlg::TileRect> {
        self.v.h.drlg.subtiles(self.game, room)
    }
    /// Whether a square box collides with `mask` on the DRLG collision.
    fn soul_collides(&mut self, room: RoomId, x: i32, y: i32, size: i32, mask: u32) -> bool {
        use crate::path::place_seams::CollisionView;
        let n = size as u32;
        crate::wiring::path::place::Rooms(&self.v.h.drlg).box_query(room, x, y, n, n, mask) != 0
    }
    /// Creates a monster through the lent monster world (population's
    /// placement and creation), as the quest host's spawn does.
    fn soul_spawn(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        let sp = self.spawner.as_mut()?;
        sp.spawn_soul(&mut *self.game, &mut self.v, room, x, y, class)
    }
    /// Init 13 on the lent quest control; a host without one has no
    /// chain record.
    fn quest_link(&mut self, object: UnitId, chain: u8) -> bool {
        let Some(mut host) = self.v.h.quest_host.take() else {
            return false;
        };
        let linked = host.object_link(&mut *self.game, &mut self.v, object, chain);
        self.v.h.quest_host = Some(host);
        linked
    }
    /// Writes the unit's mode only (no flag, no update).
    fn store_mode(&mut self, unit: UnitId, mode: u8) {
        if let Some(r) = self.record(unit) {
            r.mode = u32::from(mode);
        }
    }
    /// The room containing (x, y), found from `room` on the act DRLG.
    fn room_at(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId> {
        self.v.h.drlg.find_room(self.game, room, x, y)
    }
    /// A gold drop at (x, y) in `room`, on the drop state.
    fn gold_drop(&mut self, room: RoomId, x: i32, y: i32) {
        self.with_drops(|h, sim, d, levels, spots| {
            drop_helpers::gold_drop(h, sim, d, levels, spots, Some(room), (x, y))
        });
    }
    /// Whether the single cell at (x, y) has no bits of `mask`.
    fn point_free(&self, room: RoomId, x: i32, y: i32, mask: u32) -> bool {
        crate::path::collision::point_value(&self.v.h.drlg, Some(room), x, y, mask as u16) == 0
    }
}

/// The mechanism seams on the unit lists, records, the act DRLG and the path
/// provider: interact (the unit record's interact info), messages
/// ([`Pending::send`]), adjacency, free point and placement, the 0x07 room
/// reveal, flags 2, the town test and the player lookup, the item drops
/// ([`drop_helpers`]). Trap damage, the gem test, the tome recount, the warp
/// tile, the day period keep their defaults.
impl<X: Pending> objects::MechWorld for ObjectView<'_, X> {
    /// The object the player is interacting with, from the unit record's
    /// interact info.
    fn interact_unit(&self, player: UnitId) -> Option<UnitId> {
        // Only the obelisk reads it, comparing with an object.
        let (ty, g) = self.v.units.get(player)?.interact.get()?;
        (ty == UnitType::Object as u8)
            .then(|| self.game.lists.find_unit(UnitType::Object, g))
            .flatten()
    }
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        if let Some(r) = self.v.units.get_mut(player) {
            r.interact.set(unit_type, guid);
        }
    }
    fn clear_interact(&mut self, player: UnitId) {
        if let Some(r) = self.v.units.get_mut(player) {
            r.interact.reset();
        }
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.v.h.x.send(player, msg);
    }
    /// Whether the player holds an item of type 20 (gem) in the item list, read from the inventory model lent with the
    /// quest parts ([`QuestObjectHost::has_item_type`]); none lent: no.
    fn has_gem(&mut self, player: UnitId) -> bool {
        let Some(mut host) = self.v.h.quest_host.take() else {
            return false;
        };
        let r = host.has_item_type(&self.v.h.items, player, 20);
        self.v.h.quest_host = Some(host);
        r
    }
    fn adjacent_rooms(&self, room: RoomId) -> Vec<RoomId> {
        use crate::path::collision::CollisionRooms;
        let d = &self.v.h.drlg;
        (0..d.adjacent_count(room))
            .filter_map(|i| d.adjacent(room, i))
            .collect()
    }
    fn free_point(
        &self,
        room: RoomId,
        x: i32,
        y: i32,
        size: i32,
        mask: u32,
    ) -> Option<(RoomId, i32, i32)> {
        let mut p = crate::path::coords::Point::new(x, y);
        let rooms = crate::wiring::path::place::Rooms(&self.v.h.drlg);
        match crate::path::search::free_point(&rooms, Some(room), &mut p, size, mask, false) {
            Ok(Some(r)) => Some((r, p.x, p.y)),
            _ => None,
        }
    }
    fn place_unit(&mut self, unit: UnitId, room: RoomId, x: i32, y: i32) -> bool {
        if self.v.h.paths.is_none() {
            return false;
        }
        let c = crate::wiring::path::PathCtx::of(&mut self.v, self.game);
        crate::wiring::path::place::place_unit(c, unit, Some(room), x, y, false, false)
    }
    fn send_room_reveal(&mut self, player: UnitId, room: RoomId) {
        let Some((d, r)) = self.v.h.drlg.drlg_room(self.game, room) else {
            return;
        };
        let dr = d.room(r);
        let level = d.level(dr.level).id;
        let msg =
            crate::wiring::path::place::map_reveal(dr.rect.x as u16, dr.rect.y as u16, level as u8);
        self.v.h.x.send(player, &msg);
    }
    fn set_flags2(&mut self, unit: UnitId, bits: u32) {
        if let Some(r) = self.record(unit) {
            r.flags2 |= bits;
        }
    }
    fn in_town(&self, room: RoomId) -> bool {
        self.v.h.drlg.in_town(self.game, room)
    }
    fn find_player(&self, guid: u32) -> Option<UnitId> {
        self.game.lists.find_unit(UnitType::Player, guid)
    }
    /// Drops a stand's item (weapon or armor) at the object's position.
    fn stand_drop(&mut self, object: UnitId, weapon: bool) {
        let room = self.room(object);
        let pos = self.position(object);
        self.with_drops(|h, sim, d, levels, spots| {
            drop_helpers::stand_drop(h, sim, d, levels, spots, room, pos, weapon, -1, false, None)
        });
    }
    /// Drops an item of `quality` from the object's drop `code`.
    fn drop_code_quality(&mut self, object: UnitId, code: u32, quality: u8) {
        self.with_drops(|h, sim, d, levels, spots| {
            drop_helpers::source_drop(h, sim, d, levels, spots, object, code, quality, -1, false)
        });
    }
}

/// The population seams on the act DRLG: the active room seed, the level's
/// populated-room count, the collision queries and the unit record's class.
impl<X: Pending> objects::populate::PopulateWorld for ObjectView<'_, X> {
    fn room_seed(&mut self, room: RoomId) -> Option<&mut Seed> {
        let act = self.game.lists.room(room)?.act;
        let d = self
            .v
            .h
            .drlg
            .dungeon
            .acts
            .get_mut(usize::from(act))?
            .as_mut()?;
        let r = d.drlg_room_of(room)?;
        d.active_room_seed_mut(r)
    }
    fn populated_room_count(&mut self, act: u8, level: u32) -> i32 {
        let r = self.v.h.drlg.with_act(act, &mut self.game.lists, |d, svc| {
            d.populated_room_count(svc.data, svc.types, level)
        });
        match r {
            Some(Ok(n)) => n as i32,
            Some(Err(e)) => {
                self.v.h.errors.push(WiringError::Drlg(e));
                0
            }
            None => 0,
        }
    }
    fn box_query(&self, room: RoomId, x: i32, y: i32, sx: u32, sy: u32, mask: u32) -> u32 {
        let mask = mask as u16;
        let drlg = &self.v.h.drlg;
        u32::from(if sx <= 1 && sy <= 1 {
            crate::path::collision::point_value(drlg, Some(room), x, y, mask)
        } else {
            crate::path::collision::box_value(drlg, Some(room), x, y, (sx, sy), mask)
        })
    }
    fn set_unit_class(&mut self, unit: UnitId, class: u16) {
        if let Some(r) = self.record(unit) {
            r.class = u32::from(class);
        }
    }
}

/// The chest seams on the providers the action wiring holds: the chest
/// drop on [`super::ActionHooks::object_drops`]
/// ([`crate::wiring::economy::object_chest_drop`]), the code drop and the
/// drop item code on the same drop state ([`drop_helpers`]), unit type (unit
/// records), item quality (the game's item store), the room's units (the
/// room unit list). The rest keep their defaults (no provider here: the key
/// test, trap monsters and monster spawns, the free-spot search with mask
/// 0x3F11, the player's skill start, the range test's metric, trap damage,
/// the "inside the room" bound).
impl<X: Pending> ChestWorld for ObjectView<'_, X> {
    /// The chest drop with the operate record. Without
    /// the path provider's field (a field-less host, D2RUST's own) the
    /// free-spot search takes the start spot as is
    /// ([`crate::wiring::economy::StartSpot`]); with it, the floor search.
    fn chest_drop(&mut self, op: &Operate, q: u8) -> Option<UnitId> {
        let mut d = self.v.h.object_drops.take()?;
        let t = self.tables.clone();
        let out = {
            let mut sim = crate::units::hooks::Sim {
                game: &mut *self.game,
                units: &mut *self.v.units,
                stats: &mut *self.v.stats,
                data: self.v.data,
            };
            crate::wiring::economy::object_chest_drop(
                &mut *self.v.h,
                &mut sim,
                &mut d,
                &t.levels,
                &mut crate::wiring::economy::StartSpot,
                op.object,
                op.operator,
                q,
            )
        };
        self.v.h.object_drops = Some(d);
        out
    }
    /// The code drop of the object ([`drop_helpers::code_drop`]).
    fn code_drop(&mut self, object: UnitId, code: u32) -> Option<UnitId> {
        self.with_drops(|h, sim, d, levels, spots| {
            drop_helpers::code_drop(h, sim, d, levels, spots, object, code, 0)
        })
        .flatten()
    }
    /// Drops an item from the object's drop code.
    // The chest operate passes quality 2, type -1 and act flag 1; the act
    // flag is read only by the random class of a source with no drop code,
    // so `false` here gives the same item.
    fn drop_item_code(&mut self, object: UnitId, code: u32) {
        self.with_drops(|h, sim, d, levels, spots| {
            drop_helpers::source_drop(h, sim, d, levels, spots, object, code, 2, -1, false)
        });
    }
    fn unit_type(&self, unit: UnitId) -> Option<u8> {
        self.v.units.get(unit).map(|r| r.ty as u8)
    }
    fn item_quality(&self, item: UnitId) -> Option<u8> {
        self.v.h.items.get(item).map(|i| i.quality)
    }
    fn room_units(&self, room: RoomId) -> Vec<UnitId> {
        self.game.lists.room_units(room)
    }
    /// The lent monster world's region classes for `level`.
    fn monster_region_classes(&self, level: u32) -> Option<Vec<i32>> {
        self.v.h.monster_world.as_ref()?.region_classes(level)
    }
    fn monstats_count(&self) -> u32 {
        self.v
            .h
            .monster_world
            .as_ref()
            .map_or(0, |m| m.monstats_count())
    }
    fn unit_class(&self, unit: UnitId) -> Option<u32> {
        self.v.units.get(unit).map(|r| r.class)
    }
    /// The size of an object (`SizeX`); other units: not read here.
    fn unit_size(&self, unit: UnitId) -> i32 {
        self.v
            .units
            .get(unit)
            .filter(|r| r.ty == UnitType::Object)
            .and_then(|r| self.tables.object(r.class as u16).ok())
            .map_or(0, |o| o.sizex as i32)
    }
    fn room_rect(&self, room: RoomId) -> Option<(i32, i32, i32, i32)> {
        self.v
            .h
            .drlg
            .subtiles(self.game, room)
            .map(|r| (r.x, r.y, r.w, r.h))
    }
}

/// The shrine seams on the unit's stat list (getter, set, add, the maxima,
/// level = stat 12). The rest keep their defaults (hovers, the timed-state
/// helper and its list writer, skill refresh, to-hit, two-handed test, gems
/// and item drops, the units in range, missiles, the free spot, portals, the
/// unique monster).
///
/// Open point: the stat set is not paired with a client update message of its
/// own; the set runs through the stat list's host ([`super::ActionHooks`])
/// only.
impl<X: Pending> ShrineWorld for ObjectView<'_, X> {
    /// Shows the object's overhead text (a shrine's message), with the
    /// text of string `string_id`, ending `8 · length + 125` frames from
    /// now; sent to clients as the overhead 0x26 of the update pass
    /// (flag 0x100). The text is the decimal
    /// string id (`"%d"`, 3683 + shrine id);
    /// the client resolves it against its
    /// string tables. Byte +8 is the string-table language index,
    /// 0 for the default language D2RUST runs.
    fn create_hover(&mut self, obj: UnitId, string_id: u32) -> bool {
        let text = string_id.to_string().into_bytes();
        let end = self
            .game
            .after(crate::world::objects::shrines::hover_lifetime(
                text.len() as u32
            ));
        self.v.replace_overhead(obj, &text, 0, end);
        true
    }
    /// A free spot of `size` and `mask` at the unit's position, in its room
    /// (used by shrine code 17).
    fn free_spot(&mut self, unit: UnitId, size: i32, mask: u32) -> Option<(i32, i32)> {
        let room = ObjectWorld::room(self, unit)?;
        let (x, y) = ObjectWorld::position(self, unit);
        objects::MechWorld::free_point(self, room, x, y, size, mask).map(|(_, px, py)| (px, py))
    }
    fn hover_expiry(&self, obj: UnitId) -> Option<i32> {
        self.v.units.get(obj)?.hover
    }
    fn free_hover(&mut self, obj: UnitId) {
        if let Some(r) = self.v.units.get_mut(obj) {
            r.hover = None;
        }
        self.v.h.session.overheads.remove(&obj);
    }
    /// The timed-state helper for a shrine: a state list on the target ending
    /// `duration` frames from now, the state on, and the type-12 timer that
    /// expires it. The shrine states 128-137 are in the `curse` group (11;
    /// test `shrine_states_are_in_the_curse_group`), so the curse branch
    /// applies: curse resistance (109) >= 100 refuses, a lower one shortens
    /// the duration by that percent, state 57 refuses, and the old list is
    /// the target's first list with flag 0x20 (another shrine's or a
    /// curse's); the new list carries flag 0x20. An old list of the same
    /// state (both shrine callers leave skill and level 0) is refreshed; any
    /// other is detached and freed, its remove callback run at once
    /// (`UnitHooks::lists_expired`, where the shrine remove callbacks run
    /// too).
    fn apply_state(&mut self, req: StateRequest) -> Option<StateList> {
        use crate::skills::use_::bodies::helpers::{group, stat as bst, state as bstate};
        let (p, state) = (req.target, req.state);
        let owner = self.game.lists.unit(req.source).map(|e| (e.ty, e.guid));
        // A state outside the table gives none.
        if usize::from(state) >= self.v.stats.data().states.count() {
            return None;
        }
        let mut duration = req.duration;
        let curse = self
            .v
            .stats
            .data()
            .states
            .has_flag(u32::from(state), group::CURSE);
        let (old, mut flags) = if curse {
            let r = self.v.stats.unit_total(p, bst::CURSE_RESISTANCE, 0);
            if r >= 100 {
                return None;
            }
            if r != 0 {
                duration = duration.wrapping_sub(crate::combat::pct(duration, r, 100));
            }
            if self.v.stats.has_state(p, bstate::ATTRACT as u32) {
                return None;
            }
            let r = self.v.stats.unit_list(p);
            (r.and_then(|r| self.v.stats.list_by_flags(r, 0x20)), 0x20)
        } else {
            (self.v.state_list(p, state), 0)
        };
        let end = self.game.frame.wrapping_add(duration);
        if let Some(old) = old {
            // The same state is refreshed, not replaced: a nonzero duration
            // moves its expiry and arms a new type-12 timer; the request's
            // stat is not written (only the caller's sets after the call
            // write the list).
            if self.v.stats.state(old) == u32::from(state) {
                if duration != 0 {
                    self.v.stats.set_expire(old, end);
                    if let Err(e) = self.game.schedule_event(p, 12, end, None, 0, 0) {
                        self.v.unit_error(e.into());
                    }
                }
                return Some(StateList((p.0 << 8) | u32::from(state)));
            }
            // Another state: detached (its remove callback run there) and
            // freed.
            self.v.stats.detach(&mut *self.v.h, old);
            self.v.stats.free_plain(&mut *self.v.h, old);
            let mut sim = crate::units::hooks::Sim {
                game: &mut *self.game,
                units: &mut *self.v.units,
                stats: &mut *self.v.stats,
                data: self.v.data,
            };
            crate::units::hooks::UnitHooks::lists_expired(&mut *self.v.h, &mut sim, p);
        }
        // Flag 2 with a duration; the list carries the owner's type and
        // GUID, the state, and is attached with reset 1.
        let expire = if duration != 0 {
            flags |= 2;
            end
        } else {
            0
        };
        let (ty, guid) = owner.map_or((6, u32::MAX), |(t, g)| (t.index() as u32, g));
        let l = self.v.stats.alloc(flags, expire, ty, guid);
        self.v.stats.set_state(l, u32::from(state));
        self.v.stats.attach(&mut *self.v.h, p, l, true);
        if req.stat != 0 || req.value != 0 {
            self.v.set_list_stat(l, req.stat, req.value);
        }
        use crate::world::objects::shrines::{RemoveCallback as Cb, SKILL_REMOVE, STAMINA_REMOVE};
        let cb = match req.remove {
            Cb::Default => 0x0056_E900,
            Cb::Skill => SKILL_REMOVE,
            Cb::Stamina => STAMINA_REMOVE,
        };
        self.v
            .stats
            .set_remove_callback(l, Some(crate::stats::lists::RemoveCallback(cb)));
        self.v.set_state(p, state, true);
        let _ = self.game.lists.queue_update(p);
        if duration != 0 {
            if let Err(e) = self.game.schedule_event(p, 12, end, None, 0, 0) {
                self.v.unit_error(e.into());
            }
        }
        Some(StateList((p.0 << 8) | u32::from(state)))
    }
    /// A stat on a list [`Self::apply_state`] returned (the handle packs
    /// the unit slot and the state).
    fn set_list_stat(&mut self, list: StateList, id: u16, value: i32) {
        let (unit, state) = (UnitId(list.0 >> 8), (list.0 & 0xFF) as u16);
        if let Some(l) = self.v.state_list(unit, state) {
            self.v.set_list_stat(l, id, value);
        }
    }
    /// The player's to-hit for a shrine: the attack rating, 0 under a weapon
    /// of item type 38, raised by (the skill's `LevToHit` * (level - 1) +
    /// `ToHit`, the weapon mastery, stat 119) percent, plus the class'
    /// `ToHitFactor`. It uses skill 0, level 1 and the weapon in use (not
    /// the skill-selected one); at level 1 the `LevToHit` term is 0.
    fn to_hit(&mut self, player: UnitId) -> i32 {
        use crate::combat::hit;
        let tables = Arc::clone(&self.v.h.tables);
        let weapon = self.v.h.x.weapon_in_use(player);
        let cv = super::combat::CombatView {
            game: &mut *self.game,
            v: reborrow(&mut self.v),
        };
        let mut ar = hit::attack_rating(&cv, &tables.combat, player);
        if weapon.is_some_and(|w| cv.v.h.x.item_is(w, 38)) {
            ar = 0;
        }
        let mastery = crate::skills::levels::weapon_mastery(
            &cv,
            &tables.skills,
            Some(player),
            weapon,
            Some(0),
            0,
        );
        let (tohit, lev) = tables
            .skills
            .skill(0)
            .map_or((0, 0), |r| (r.tohit as i32, r.levtohit as i32));
        const LEVEL: i32 = 1;
        let pct = lev
            .wrapping_mul(LEVEL - 1)
            .wrapping_add(tohit)
            .wrapping_add(mastery)
            .wrapping_add(
                <super::combat::CombatView<'_, X> as crate::skills::SkillUnits>::stat(
                    &cv, player, 119, 0,
                ),
            );
        let total = ar.wrapping_add(pct.wrapping_mul(ar) / 100);
        let factor = tables
            .combat
            .charstats(cv.v.units.get(player).map_or(0, |r| r.class as i32))
            .map_or(0, |c| c.tohitfactor as i32);
        total.wrapping_add(factor)
    }
    fn stat(&self, unit: UnitId, id: u16) -> i32 {
        self.v.stat(unit, id)
    }
    fn set_stat(&mut self, unit: UnitId, id: u16, value: i32) {
        self.v.set_base(unit, id, value);
    }
    fn add_base_stat(&mut self, unit: UnitId, id: u16, delta: i32) {
        self.v.stats.unit_add(&mut *self.v.h, unit, id, delta, 0);
    }
    fn max_life(&self, unit: UnitId) -> i32 {
        self.v.stats.max_life(unit)
    }
    fn max_mana(&self, unit: UnitId) -> i32 {
        self.v.stats.max_mana(unit)
    }
    fn max_stamina(&self, unit: UnitId) -> i32 {
        self.v.stats.max_stamina(unit)
    }
    /// Missile creation on the missile store, with the parameter record the
    /// storm and potion shrines build: the start is the shrine's path
    /// position, the
    /// target is that position plus the offset when the absolute-target
    /// flag (0x20) is set, else the offset itself (relative, flag 2).
    fn create_missile(&mut self, m: MissileRequest) {
        use crate::missiles::{self, param_flags, MissileParams};
        let (x, y) = self.v.h.path_position(m.from);
        let (target_x, target_y) = if m.flags & param_flags::TARGET_ABSOLUTE != 0 {
            (x.wrapping_add(m.offset.0), y.wrapping_add(m.offset.1))
        } else {
            m.offset
        };
        let p = MissileParams {
            flags: m.flags,
            owner: Some(m.owner),
            origin: Some(m.from),
            class: i32::from(m.class),
            x,
            y,
            target_x,
            target_y,
            level: m.skill_level,
            ..MissileParams::default()
        };
        let Some(mut store) = self.v.h.missiles.take() else {
            self.v.h.errors.push(WiringError::Reentrant("missiles"));
            return;
        };
        let t = self.v.h.tables.clone();
        {
            let mut cx = missiles::Ctx {
                tables: &t.missiles,
                store: &mut store,
                world: &mut self.v,
            };
            let _ = missiles::create_missile(self.game, &mut cx, &p);
        }
        self.v.h.missiles = Some(store);
    }
    /// Storm's finder call: the shrine's room, centre and `range`, with a
    /// filter (a player not in mode 0 or 17; a monster not in mode 0 or 12;
    /// any other type refused; no distance test), then the dead test.
    fn units_in_range(&mut self, center: UnitId, range: i32) -> Vec<UnitId> {
        use crate::missiles::{self, bodies_ext2::unit_find_by};
        use crate::units::UnitType;
        let room = self.room(center);
        let at = self.v.h.path_position(center);
        let Some(mut store) = self.v.h.missiles.take() else {
            self.v.h.errors.push(WiringError::Reentrant("missiles"));
            return Vec::new();
        };
        let t = self.v.h.tables.clone();
        let found = {
            let mut cx = missiles::Ctx {
                tables: &t.missiles,
                store: &mut store,
                world: &mut self.v,
            };
            unit_find_by(self.game, &mut cx, room, at, range, 0, |g, cx, u| {
                let Some(ty) = g.lists.unit(u).map(|e| e.ty) else {
                    return false;
                };
                let mode = cx.world.units.get(u).map_or(0, |r| r.mode as i32);
                match ty {
                    UnitType::Player => mode != 0 && mode != 17,
                    UnitType::Monster => mode != 0 && mode != 12,
                    _ => false,
                }
            })
        };
        self.v.h.missiles = Some(store);
        found
            .into_iter()
            .filter(|&u| !self.v.units.is_dead(u))
            .collect()
    }
    fn player_level(&self, player: UnitId) -> i32 {
        self.v.stat(player, STAT_LEVEL)
    }
    fn drop_near_player(&mut self, player: UnitId, code: [u8; 4]) -> bool {
        self.shrine_item_near(player, code, 2)
    }
    fn drop_potion_near_player(&mut self, player: UnitId, code: [u8; 4], _quantity: i32) {
        self.shrine_item_near(player, code, 0);
    }
}

impl<X: Pending> ObjectView<'_, X> {
    /// The shrine item drop (gem upgrade and the inline potion drop): the
    /// floor search from the player's position, an item of `code` at the
    /// player's level, then quantity (stat 70) set to 1. True when the item
    /// was made. Not modelled: the new item's unit flag 0x1 (sent to the
    /// clients on the next update) and, for a gem upgrade, the client message
    /// for the old gem before it is freed; neither draws or changes game
    /// state.
    fn shrine_item_near(&mut self, player: UnitId, code: [u8; 4], quality: u8) -> bool {
        let made = self
            .with_drops(|h, sim, d, levels, spots| {
                drop_helpers::near_player_drop(
                    h,
                    sim,
                    d,
                    levels,
                    spots,
                    player,
                    u32::from_le_bytes(code),
                    quality,
                )
            })
            .flatten();
        if let Some(item) = made {
            self.v.set_base(item, STAT_QUANTITY, 1);
        }
        made.is_some()
    }
}

/// Stat 70 `quantity`.
const STAT_QUANTITY: u16 = 70;

/// Stat 12 `level`.
const STAT_LEVEL: u16 = 12;

/// The door, well and portal seams on the unit's stat lists: the vitals
/// and their maxima, the vital set, the state list removal (free) and the
/// cure (every curable state with a list). The rest keep their defaults
/// (portal creation, update extras, the footprint test, the pet heal,
/// players, hostility, portal travel).
///
/// Open point: the client update of the vital set is the stat list host's;
/// no separate message is sent here.
impl<X: Pending> MiscWorld for ObjectView<'_, X> {
    /// Whether some cell under the object's `SizeX` x `SizeY` box at its
    /// room and position has bits in `mask` (the box query; sizes <= 1 read a
    /// single point; a cell without a room reads 0x27).
    fn footprint_collides(&self, object: UnitId, mask: u16) -> bool {
        let Some(o) = self.object_row(object) else {
            return false;
        };
        let Some(room) = self.room(object) else {
            return true;
        };
        let (x, y) = self.position(object);
        use objects::populate::PopulateWorld;
        self.box_query(room, x, y, o.sizex, o.sizey, u32::from(mask)) != 0
    }
    fn party_id(&self, unit: UnitId) -> u16 {
        self.v.h.x.object_party_id(unit)
    }
    /// The portal's partner on the pair links ([`View::portal_partner`]); an
    /// object without a link: [`Pending::object_portal_partner`].
    fn portal_partner(&mut self, object: UnitId) -> Option<UnitId> {
        if self.v.h.portals.link(object).is_some() {
            return self.v.portal_partner(self.game, object);
        }
        self.v.h.x.object_portal_partner(self.game, object)
    }
    fn has_quest_record(&self, player: UnitId) -> bool {
        self.v.h.x.object_quest_record(player)
    }
    fn expansion(&self) -> bool {
        self.v.data.expansion
    }
    fn player_quest_bit(&self, player: UnitId, quest: u32, bit: u8) -> bool {
        self.v.h.x.object_quest_bit(player, quest, bit)
    }
    /// The player's portal GUID
    /// ([`super::town_portal::PortalLinks::player_portal`]); never written:
    /// [`Pending::object_portal_guid`].
    fn player_portal_guid(&self, player: UnitId) -> u32 {
        self.v
            .h
            .portals
            .player_portal(player)
            .unwrap_or_else(|| self.v.h.x.object_portal_guid(player))
    }
    /// The level spawn point ([`crate::wiring::path::place::level_spawn`])
    /// for a portal without a partner (a class-60 portal of a quest; every
    /// pair has one from creation).
    fn level_spawn_point(&mut self, level: u32) -> Option<(RoomId, i32, i32)> {
        if let Some(p) = self.v.h.x.object_level_spawn(self.game, level) {
            return Some(p);
        }
        let act = crate::drlg::act_of_level(level);
        crate::wiring::path::place::level_spawn(&mut self.v, self.game, act, level, 0)
    }
    fn quest_level_change(&mut self, player: UnitId, from: u32, to: u32) {
        self.v.h.x.object_quest_level_change(player, from, to);
    }
    /// Moves the player to (x, y) in `mode` with the path provider
    /// ([`crate::wiring::path`]).
    fn player_mode_xy(&mut self, player: UnitId, mode: u8, x: i32, y: i32) {
        if self.v.h.paths.is_some() {
            let mut c = crate::wiring::path::PathCtx::of(&mut self.v, self.game);
            c.walk_to(player, u32::from(mode), x, y);
        }
    }
    fn remove_portal(&mut self, object: UnitId) {
        self.v.h.x.object_remove_portal(self.game, object);
        // The unit is freed once the object call returns
        // ([`View::flush_portal_removals`]: the object state is lent).
        self.v.h.portals.doom(object);
    }
    fn portal_act5_hook(&mut self, partner: UnitId) {
        self.v.h.x.object_portal_act5(partner);
    }
    fn just_portaled(&mut self, player: UnitId, expire: i32) {
        self.v.h.x.object_just_portaled(self.game, player, expire);
        // A stat list (expiring at frame + 75), event 12 at frame + 75, state
        // 102 on, list state 102, and the default remove callback.
        if usize::from(JUST_PORTALED_STATE) >= self.v.stats.data().states.count() {
            return;
        }
        if self.v.state_list(player, JUST_PORTALED_STATE).is_some() {
            self.v
                .stats
                .free_state_list(&mut *self.v.h, player, u32::from(JUST_PORTALED_STATE));
        }
        let Some(l) = self
            .v
            .create_state_list(player, JUST_PORTALED_STATE, None, expire)
        else {
            return;
        };
        // The default remove callback: the list's expiry by event 12 turns
        // the state off, and the next player update sends 0xA9.
        self.v.stats.set_remove_callback(
            l,
            Some(crate::wiring::interaction::skill_use::DELAY_REMOVE_CALLBACK),
        );
        if let Err(e) = self.game.schedule_event(player, 12, expire, None, 0, 0) {
            self.v.unit_error(e.into());
        }
        self.v.set_state(player, JUST_PORTALED_STATE, true);
        let _ = self.game.lists.queue_update(player);
    }
    fn vital_stat(&self, unit: UnitId, id: u16) -> u32 {
        let st = &*self.v.stats;
        (match id {
            7 => st.max_life(unit),
            9 => st.max_mana(unit),
            11 => st.max_stamina(unit),
            _ => st.unit_total(unit, id, 0),
        }) as u32
    }
    fn set_vital_stat(&mut self, unit: UnitId, id: u16, value: u32) {
        self.v.set_base(unit, id, value as i32);
    }
    fn remove_state_list(&mut self, unit: UnitId, state: u16) -> bool {
        if self.v.state_list(unit, state).is_none() {
            return false;
        }
        self.v
            .stats
            .free_state_list(&mut *self.v.h, unit, u32::from(state));
        true
    }
    fn cure_states(&mut self, unit: UnitId) -> bool {
        let count = self.v.stats.data().states.count();
        let mut changed = false;
        for s in 0..count {
            let Ok(s16) = u16::try_from(s) else {
                break;
            };
            let st = u32::from(s16);
            let curable = self
                .v
                .stats
                .data()
                .states
                .has_flag(st, crate::wiring::interaction::npc_world::STATE_CURABLE);
            if self.v.stats.has_state(unit, st) && curable && self.v.state_list(unit, s16).is_some()
            {
                self.v.stats.free_state_list(&mut *self.v.h, unit, st);
                changed = true;
            }
        }
        changed
    }
}

impl<X> super::ActionHooks<X> {
    /// For an object whose init is running (between allocation and
    /// add-to-world): the allocation's point and room argument, which the
    /// init reads as the object's position and room before the unit is
    /// added. `None` outside such an allocation.
    pub fn object_alloc_spot(&self, unit: UnitId) -> Option<(i32, i32, Option<RoomId>)> {
        let (_, room) = *self.alloc_rooms.iter().rev().find(|&&(u, _)| u == unit)?;
        let (x, y) = self.objects.as_ref()?.alloc_at?;
        Some((x, y, room))
    }
}
