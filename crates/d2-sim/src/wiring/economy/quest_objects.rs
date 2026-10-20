//! Quest routes of the object module. A [`QuestObjectCall`] (queued by the
//! action wiring when the host holds the quest control,
//! `ObjectState::route_quests`) runs on the quest control: the init and
//! operate functions that `object-functions.tsv` gives to the quest code and
//! whose bodies it implements, by function index, and object event 7 through
//! [`quests::object_event`]. A route whose function the quest code does not
//! implement is handed back for the host's `Pending::object_route`.
//!
//! Status: wired, not yet checked against play sessions.
//!
//! TODO: in the game the quest init runs inside the object's allocation and
//! the operate inside the dispatch. The quest control lives with the host,
//! outside the action wiring's call stack, so the queue runs when the host
//! drains it: right after the dispatch for C→S 0x13 (nothing follows the
//! operate there), after the timer event that raised an object event 7, and
//! after the room-pass hook or tick that allocated the object for an init.
//! An init's draws and allocations (the marker init's town Cain) therefore
//! come after the rest of that hook's allocations; whether that order is
//! visible in play is not yet checked.

use crate::game::Game;
use crate::items::ItemTables;
use crate::units::UnitId;
use crate::wiring::action::{
    CainPortalAnswer, CainPortalCall, ObjectRoute, Pending, QuestObjectCall, QuestObjectHost, View,
};
use crate::wiring::interaction::NpcRest;
use crate::world::npc::InteractionList;
use std::collections::BTreeMap;

use super::quest_reward::QuestInventory;
use super::{Economy, EconomyQuests, GameFields, HostQuests, ItemStore, QuestRest};
use crate::wiring::action::ActionHooks;
use crate::world::objects::{Dispatch, EventRun, Operate, Route};
use crate::world::quests::act2::q4::{self, JerhynStep};
use crate::world::quests::act3::{self, InitPoint, KhalimChest};
use crate::world::quests::{self, act1, act2, act4, act5, placeholders, QuestControl, QuestWorld};

/// What running one queued route did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestObjectRun {
    /// A quest function ran (or the function does nothing).
    Ran,
    /// The function's body is not implemented by the quest code, or only a
    /// part of it is (operate 34): hand the route to `Pending::object_route`.
    HandBack(ObjectRoute),
}

/// The host's quest parts lent to the action hooks for a call
/// ([`QuestObjectHost`], `ActionHooks::quest_host`): the quest control,
/// the quests' rest and the item tables (the unique bits are the hooks'
/// own, `ActionHooks::uniques`). A route runs on
/// [`HostQuests`] over an economy built from the call's view (its item
/// store, game seed and unique bits lent and written back, as
/// `with_economy` does),
/// without the deferred mercenary rewards (no object quest function
/// grants one).
pub struct QuestLoan<R, I = NoInventory> {
    /// The quest control block.
    pub quests: QuestControl,
    /// The quests' rest.
    pub rest: R,
    /// The item tables.
    pub tables: ItemTables,
    /// The host's inventory model, lent with the rest (the quest items:
    /// `has_item`, `delete_item`, the reward); none: the rest's answers.
    pub inv: Option<I>,
    /// The NPCs' interaction lists (`InteractionState::lists`), lent for
    /// the AI's interaction tests ([`QuestObjectHost::npc_interacting`],
    /// `npc_in_list`).
    pub chats: BTreeMap<UnitId, InteractionList>,
}

/// An inventory model a host lends to [`QuestLoan`]: `lend` runs `f` on
/// it as the quest calls' inventory ([`QuestInventory`]).
pub trait LoanedInventory {
    fn lend<X: Pending, T>(
        &mut self,
        f: impl FnOnce(&mut dyn QuestInventory<ActionHooks<X>>) -> T,
    ) -> T;
}

/// The loan with no inventory model (no value of this type exists).
pub enum NoInventory {}

impl LoanedInventory for NoInventory {
    fn lend<X: Pending, T>(
        &mut self,
        _: impl FnOnce(&mut dyn QuestInventory<ActionHooks<X>>) -> T,
    ) -> T {
        match *self {}
    }
}

impl<X: Pending, R: QuestRest + NpcRest + 'static, I: LoanedInventory + 'static> QuestObjectHost<X>
    for QuestLoan<R, I>
{
    fn run(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        call: QuestObjectCall,
    ) -> Option<ObjectRoute> {
        match self.on_world(game, v, LoanCall::Route(&call)) {
            LoanOut::Run(QuestObjectRun::HandBack(r)) => Some(r),
            _ => None,
        }
    }

    fn changed_level(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        player: UnitId,
        from: u32,
        to: u32,
    ) {
        self.on_world(game, v, LoanCall::ChangedLevel { player, from, to });
        // Event 3 runs inside the client pass before the room switch, and the
        // game queues each of its sends to the client buffer at once, so they
        // precede the room switch's messages (a2-wp-46: 0x5D / 0x0A first in the
        // burst).
        for (unit, bytes) in self.rest.drain_sent() {
            v.h.x.send(unit, &bytes);
        }
    }

    fn arcane_object(&mut self, game: &mut Game, v: &mut View<'_, X>) -> u16 {
        match self.on_world(game, v, LoanCall::ArcaneObject) {
            LoanOut::Class(c) => c,
            _ => 307,
        }
    }

    fn town_leave(&mut self, game: &mut Game, v: &mut View<'_, X>, player: UnitId, to: u32) {
        // What the rest queued before this call (a talk's act-complete
        // 0x28 / 0x61, run before the deferred act change) stays queued.
        let held = self.rest.drain_sent();
        self.on_world(game, v, LoanCall::TownLeave { player, to });
        // The town-leave refresh queues the quest messages at once, so they
        // precede the act change's.
        for (unit, bytes) in self.rest.drain_sent() {
            v.h.x.send(unit, &bytes);
        }
        for (unit, bytes) in held {
            self.rest.send(unit, &bytes);
        }
    }

    fn npc_wants_interact(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        player: UnitId,
        npc: UnitId,
        class: u16,
        interact: bool,
    ) -> bool {
        let call = LoanCall::NpcWantsInteract {
            player,
            npc,
            class,
            interact,
        };
        self.on_world(game, v, call) == LoanOut::Active(true)
    }

    fn object_link(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        object: UnitId,
        chain: u8,
    ) -> bool {
        self.on_world(game, v, LoanCall::ObjectLink { object, chain }) == LoanOut::Active(true)
    }

    /// Runs the link now; its sends go out at once, as the game queues them
    /// to the client buffer inside the caller.
    fn add_link(&mut self, game: &mut Game, v: &mut View<'_, X>, unit: UnitId, chain: u8) -> bool {
        self.on_world(game, v, LoanCall::Link { unit, chain });
        for (unit, bytes) in self.rest.drain_sent() {
            v.h.x.send(unit, &bytes);
        }
        true
    }

    /// Runs the kill parse now (inside the kill, after the treasure
    /// drop); its sends go out at once.
    fn kill_parse(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        victim: UnitId,
        killer: Option<UnitId>,
    ) -> bool {
        self.on_world(game, v, LoanCall::Kill { victim, killer });
        for (unit, bytes) in self.rest.drain_sent() {
            v.h.x.send(unit, &bytes);
        }
        true
    }

    /// Radament's and the Summoner's AI calls run now, inside the think;
    /// their sends go out at once (where the queued path put its
    /// `QUEST_EVENT_MARK`). Other calls: the Pending's.
    fn ai_quest_call(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        unit: UnitId,
        call: crate::monsters::ai::QuestCall,
    ) -> Option<bool> {
        use crate::monsters::ai::QuestCall;
        let c = match call {
            QuestCall::RadamentActivated => LoanCall::RadamentAi { unit },
            QuestCall::SummonerActivated => LoanCall::SummonerSeen,
            QuestCall::Shenk => LoanCall::ShenkAi { unit },
            QuestCall::Nihlathak => LoanCall::NihlathakAi,
            QuestCall::BaalToStairs => LoanCall::BaalToStairs,
            _ => return None,
        };
        self.on_world(game, v, c);
        for (unit, bytes) in self.rest.drain_sent() {
            v.h.x.send(unit, &bytes);
        }
        // Handled, as the queueing host answers.
        Some(true)
    }

    /// The Golden Bird's boss choice, now (the monstats flags byte built
    /// from `flying`, as the queued path does).
    fn boss_created(&mut self, game: &mut Game, v: &mut View<'_, X>, unit: UnitId) -> bool {
        let Some(class) = v.units.get(unit).map(|r| r.class) else {
            return true;
        };
        let flags = v.h.tables.combat.monstats.get(class as usize).map(|m| {
            if m.flying {
                act3::q4::FLYING_0D
            } else {
                0
            }
        });
        if let Ok(class) = u16::try_from(class) {
            self.on_world(game, v, LoanCall::BossCreated { unit, class, flags });
            self.drain_to(v);
        }
        true
    }

    /// The preset-boss hook: a `ret` stub in levels ≥ 108; below, the
    /// council's hook, now.
    fn preset_boss(&mut self, game: &mut Game, v: &mut View<'_, X>, unit: UnitId) -> bool {
        let level = game
            .lists
            .unit(unit)
            .and_then(|e| e.room())
            .and_then(|r| v.h.drlg.level_id(game, r));
        if level.is_some_and(|l| l < 108) {
            self.on_world(game, v, LoanCall::CouncilPreset { unit });
            self.drain_to(v);
        }
        true
    }

    /// Anya's portal request, now.
    fn anya_open_portal(&mut self, game: &mut Game, v: &mut View<'_, X>, unit: UnitId) -> bool {
        self.on_world(game, v, LoanCall::AnyaPortal { unit });
        self.drain_to(v);
        true
    }

    fn jerhyn_palace_active(&mut self) -> bool {
        q4::jerhyn_palace_active(&self.quests)
    }

    fn jerhyn_npc_state(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        at: (i32, i32),
    ) -> JerhynStep {
        match self.on_world(game, v, LoanCall::JerhynState { at }) {
            LoanOut::Jerhyn(s) => s,
            _ => JerhynStep::Out(1, 0),
        }
    }

    fn wanderer_target(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        wanderer: UnitId,
    ) -> Option<(i32, i32)> {
        match self.on_world(game, v, LoanCall::WandererTarget { wanderer }) {
            LoanOut::Target(t) => t,
            _ => None,
        }
    }

    fn wanderer_minions(&mut self, game: &mut Game, v: &mut View<'_, X>, wanderer: UnitId) {
        self.on_world(game, v, LoanCall::WandererMinions { wanderer });
    }

    fn guard_moving(&mut self) -> bool {
        q4::guard_moved(&mut self.quests)
    }

    fn cain_portal(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        unit: UnitId,
        call: CainPortalCall,
    ) -> Option<CainPortalAnswer> {
        match self.on_world(game, v, LoanCall::CainPortal { unit, call }) {
            LoanOut::Cain(a) => Some(a),
            _ => None,
        }
    }

    fn jerhyn_placed(&mut self) {
        q4::jerhyn_placed(&mut self.quests);
    }

    fn palace_door_open(&mut self) -> bool {
        q4::guard_at_end(&self.quests)
    }

    fn palace_guard_aside(&mut self) -> bool {
        q4::blocker_open(&self.quests)
    }

    fn palace_guard_point(&mut self) -> Option<(i32, i32)> {
        q4::guard_target(&self.quests)
    }

    fn map_ai_store(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        class: u16,
        path: Vec<(u32, i32, i32)>,
    ) {
        let call = LoanCall::MapAiStore { class, path: &path };
        self.on_world(game, v, call);
    }

    fn has_item_type(&mut self, items: &ItemStore, player: UnitId, itype: i16) -> bool {
        let Some(inv) = self.inv.as_mut() else {
            return false;
        };
        let list = inv.lend::<X, _>(|q| q.items_of(player));
        list.into_iter().any(|i| {
            items
                .get(i)
                .is_some_and(|it| self.tables.is_type(it.record, itype))
        })
    }

    fn npc_interacting(&self, npc: UnitId) -> bool {
        self.chats.get(&npc).is_some_and(|l| !l.nodes.is_empty())
    }

    fn npc_in_list(&self, npc: UnitId, player: UnitId) -> bool {
        self.chats
            .get(&npc)
            .is_some_and(|l| l.state(player).is_some())
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

impl<R: QuestRest + NpcRest + 'static, I: LoanedInventory + 'static> QuestLoan<R, I> {
    /// The rest's sends of a call, out at once (the game queues them to the
    /// client buffer inside the caller).
    fn drain_to<X: Pending>(&mut self, v: &mut View<'_, X>) {
        for (unit, bytes) in self.rest.drain_sent() {
            v.h.x.send(unit, &bytes);
        }
    }

    /// Runs `call` on the quest control and a [`HostQuests`] over the
    /// game's parts (the economy of `v`, this loan's rest and tables),
    /// with the lent inventory model when there is one.
    fn on_world<X: Pending>(
        &mut self,
        game: &mut Game,
        v: &mut View<'_, X>,
        call: LoanCall<'_>,
    ) -> LoanOut {
        let h = &mut *v.h;
        let mut fields = GameFields::from_action(
            h.game_seed,
            &h.ai_info,
            v.data.expansion,
            std::mem::take(&mut h.uniques),
        );
        let mut items = std::mem::take(&mut h.items);
        let out = {
            let mut econ = Economy {
                game,
                units: &mut *v.units,
                stats: &mut *v.stats,
                data: v.data,
                hooks: &mut *v.h,
                fields: &mut fields,
                tables: &self.tables,
                items: &mut items,
            };
            let (quests, rest, chats) = (&mut self.quests, &mut self.rest, &mut self.chats);
            match self.inv.as_mut() {
                Some(inv) => {
                    inv.lend::<X, _>(|q| on_host(&mut econ, rest, quests, Some(q), chats, call))
                }
                None => on_host(&mut econ, rest, quests, None, chats, call),
            }
        };
        // Links recorded on the hooks' (empty) store during the lend.
        for (unit, chain) in v.h.items.take_quest_links() {
            items.push_quest_link(unit, chain);
        }
        v.h.items = items;
        v.h.game_seed = fields.seed;
        v.h.uniques = fields.uniques;
        out
    }
}

/// What a [`QuestLoan`] runs on the game's parts.
#[derive(Clone, Copy)]
enum LoanCall<'c> {
    /// One queued object route.
    Route(&'c QuestObjectCall),
    /// The map-AI store (class and path).
    MapAiStore {
        class: u16,
        path: &'c [(u32, i32, i32)],
    },
    /// Quest event 3 CHANGEDLEVEL.
    ChangedLevel { player: UnitId, from: u32, to: u32 },
    /// The quest part of the town-leave refresh.
    TownLeave { player: UnitId, to: u32 },
    /// The quest active test.
    NpcWantsInteract {
        player: UnitId,
        npc: UnitId,
        class: u16,
        interact: bool,
    },
    /// Jerhyn's palace NPC state.
    JerhynState { at: (i32, i32) },
    /// Object init 13: the quest link for `chain`.
    ObjectLink { object: UnitId, chain: u8 },
    /// The quest link for `chain` from monster init or item creation.
    Link { unit: UnitId, chain: u8 },
    /// Radament's AI call.
    RadamentAi { unit: UnitId },
    /// The Summoner's AI call.
    SummonerSeen,
    /// Shenk's AI call.
    ShenkAi { unit: UnitId },
    /// Nihlathak's AI call.
    NihlathakAi,
    /// The Baal crab at the Worldstone portal.
    BaalToStairs,
    /// Anya's portal request.
    AnyaPortal { unit: UnitId },
    /// The Golden Bird's boss choice (class, monstats flags byte).
    BossCreated {
        unit: UnitId,
        class: u16,
        flags: Option<u8>,
    },
    /// The council's preset hook (below level 108).
    CouncilPreset { unit: UnitId },
    /// The kill parse inside the kill.
    Kill {
        victim: UnitId,
        killer: Option<UnitId>,
    },
    /// The next Arcane Sanctuary dummy object class, for preset 582.
    ArcaneObject,
    /// The Dark Wanderer's walk target.
    WandererTarget { wanderer: UnitId },
    /// The Dark Wanderer's minion hook.
    WandererMinions { wanderer: UnitId },
    /// `cain1`'s A1Q4 portal calls.
    CainPortal { unit: UnitId, call: CainPortalCall },
}

/// What a [`LoanCall`] gave back.
#[derive(Debug, PartialEq, Eq)]
enum LoanOut {
    Run(QuestObjectRun),
    /// The active test's answer.
    Active(bool),
    /// A class (the Arcane Sanctuary dummy).
    Class(u16),
    /// Jerhyn's palace NPC state outputs.
    Jerhyn(JerhynStep),
    /// The Dark Wanderer's walk target.
    Target(Option<(i32, i32)>),
    /// A Cain-portal call's answer.
    Cain(CainPortalAnswer),
}

/// `call` on [`HostQuests`] over `econ` and `rest`, with the host's
/// inventory model when one is lent.
fn on_host<'e, X: Pending, R: QuestRest>(
    econ: &'e mut Economy<'_, ActionHooks<X>>,
    rest: &'e mut R,
    quests: &mut QuestControl,
    inv: Option<&'e mut dyn QuestInventory<ActionHooks<X>>>,
    chats: &'e mut BTreeMap<UnitId, InteractionList>,
    call: LoanCall<'_>,
) -> LoanOut {
    let inner = EconomyQuests::new(econ, rest);
    let mut w = HostQuests::new(inner);
    w.inventory = inv;
    w.chats = Some(chats);
    link_items(quests, &mut w);
    let out = match call {
        LoanCall::Route(c) => LoanOut::Run(run(quests, &mut w, c)),
        LoanCall::MapAiStore { class, path } => {
            let nodes: Vec<_> = path
                .iter()
                .map(|&(action, x, y)| crate::monsters::ai::MapNode {
                    action: action as i32,
                    x,
                    y,
                })
                .collect();
            // The copy's handle: index + 1 (0: none).
            w.inner.econ.hooks.map_ai_paths.push(nodes);
            let h = w.inner.econ.hooks.map_ai_paths.len() as u32;
            match class {
                543 => act5::q1::larzuk_map_ai(quests, &mut w, h),
                459 => act5::q3::map_ai_store(quests, &mut w, h, false),
                461 => act5::q3::map_ai_store(quests, &mut w, h, true),
                _ => {}
            }
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::TownLeave { player, to } => {
            quests.town_leave(&mut w, player, to);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::ChangedLevel { player, from, to } => {
            quests.changed_level(&mut w, player, from, to);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        // The set not picked (a fatal in the game) answers false.
        LoanCall::NpcWantsInteract {
            player,
            npc,
            class,
            interact,
        } => LoanOut::Active(
            quests.npc_wants_interact(&mut w, player, npc, class, interact) == Ok(true),
        ),
        LoanCall::ArcaneObject => LoanOut::Class(act2::q6::arcane_object(quests, &mut w)),
        LoanCall::WandererTarget { wanderer } => {
            LoanOut::Target(act3::wanderer_target(quests, &mut w, wanderer))
        }
        LoanCall::WandererMinions { wanderer } => {
            act3::wanderer_minions(quests, &mut w, wanderer);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::CainPortal { unit, call } => LoanOut::Cain(match call {
            CainPortalCall::Setup => {
                CainPortalAnswer::Ok(act1::q4::cain_portal_setup(quests, &mut w, unit))
            }
            CainPortalCall::SpawnTown => {
                act1::q4::cain_leaves_tristram(quests, &mut w);
                CainPortalAnswer::Ok(true)
            }
            CainPortalCall::SpawnOutside => {
                CainPortalAnswer::Ok(act1::q4::cain_spawn_outside_portal(quests, &mut w, unit))
            }
            CainPortalCall::Coords => {
                CainPortalAnswer::Coords(act1::q4::cain_portal_coords(quests, &mut w, unit))
            }
        }),
        LoanCall::JerhynState { at } => LoanOut::Jerhyn(q4::jerhyn_npc_state(quests, &mut w, at)),
        // The link record is found first; the link itself may already exist.
        // Chain 4 on a class-61 object runs a special case first.
        LoanCall::ObjectLink { object, chain } => {
            let special = (chain == 4).then_some(0x0059_2F80);
            quests.add_link(&mut w, object, chain, special);
            LoanOut::Active(quests.find(chain).is_some())
        }
        LoanCall::Link { unit, chain } => {
            quests.add_link(&mut w, unit, chain, None);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::Kill { victim, killer } => {
            quests.monster_killed(&mut w, victim, killer);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::RadamentAi { unit } => {
            act2::q1::radament_ai(quests, &mut w, unit);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::SummonerSeen => {
            act2::q5::summoner_seen(quests, &mut w);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::ShenkAi { unit } => {
            act5::q1::shenk_activated(quests, &mut w, unit);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::NihlathakAi => {
            act5::q4::nihlathak_ai_status(quests, &mut w);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::BaalToStairs => {
            act5::q6::chamber_open(quests, &mut w);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::AnyaPortal { unit } => {
            act5::q4::anya_ai_portal(quests, &mut w, unit);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::BossCreated { unit, class, flags } => {
            act3::choose_bird_boss(quests, &mut w, unit, class, flags);
            LoanOut::Run(QuestObjectRun::Ran)
        }
        LoanCall::CouncilPreset { unit } => {
            act3::council_preset(quests, &mut w, unit);
            LoanOut::Run(QuestObjectRun::Ran)
        }
    };
    // Items the call created (a quest drop) link before the next call.
    link_items(quests, &mut w);
    out
}

/// The chain links of the items created since the last quest call
/// (`ItemStore::take_quest_links`), added in creation order before the
/// quest control runs anything else.
pub fn link_items<W: QuestWorld + ItemLinks>(quests: &mut QuestControl, w: &mut W) {
    for (item, chain) in w.take_item_links() {
        quests.add_link(w, item, chain, None);
    }
}

/// A quest world that reaches the game's item store
/// ([`ItemStore::take_quest_links`]).
pub trait ItemLinks {
    fn take_item_links(&mut self) -> Vec<(UnitId, u8)>;
}

impl<X, R> ItemLinks for HostQuests<'_, '_, X, R> {
    fn take_item_links(&mut self) -> Vec<(UnitId, u8)> {
        self.inner.econ.items.take_quest_links()
    }
}

/// Runs the queued routes in order; returns those handed back.
pub fn run_all<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    calls: Vec<QuestObjectCall>,
) -> Vec<ObjectRoute> {
    calls
        .into_iter()
        .filter_map(|c| match run(ctl, w, &c) {
            QuestObjectRun::Ran => None,
            QuestObjectRun::HandBack(r) => Some(r),
        })
        .collect()
}

/// One queued route.
pub fn run<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    c: &QuestObjectCall,
) -> QuestObjectRun {
    match c.route {
        ObjectRoute::Init { object, created } if created.init == Route::Quest => {
            init(ctl, w, c, object, created.init_fn)
        }
        ObjectRoute::Operate(Dispatch::Quest(op)) => operate(ctl, w, c, op),
        ObjectRoute::Event {
            object,
            run: EventRun::Quest,
        } => {
            // Object event 7: by object class.
            quests::object_event(ctl, w, object, c.class);
            QuestObjectRun::Ran
        }
        r => QuestObjectRun::HandBack(r),
    }
}

/// The quest init functions the quest code implements, by `InitFn` index
/// (`object-functions.tsv` kind `init`, owner the quest code), with the
/// function address in the table.
pub fn init_fn(n: u8) -> Option<u32> {
    Some(match n {
        // TowerTome.
        4 => 0x0059_5A00,
        // InvisibleObject: the chain 4 link or mode 2.
        13 => 0x0059_4020,
        // CairnStone, objects 17–21.
        6 => 0x0059_35E0,
        // CainGibbet.
        7 => 0x0054_4990,
        // InifussTree.
        9 => 0x0059_3FC0,
        // MalusStand.
        15 => 0x0054_4950,
        // JerhynPosition.
        18 => 0x0054_48B0,
        // JerhynPositionEx.
        19 => 0x0054_48E0,
        // TaintedAltar.
        20 => 0x0054_4910,
        // HoradricOrifice.
        21 => 0x0059_DB50,
        // ArcaneSanctuaryPortal.
        29 => 0x0059_BA40,
        // HaremBlocker.
        30 => 0x0059_B7D0,
        // HoradricChest 31–33: bare `ret`.
        31 => 0x0059_9EE0,
        32 => 0x0059_9F00,
        33 => 0x0059_9EF0,
        // TyraelsDoor.
        38 => 0x0059_DAD0,
        // Act III (q-a3-quests).
        23 => 0x0054_4E30,
        25 => 0x005B_9AE0,
        39 => 0x005B_9D40,
        41 => 0x005B_8660,
        42 => 0x005B_86B0,
        43 => 0x005B_D1F0,
        44 => 0x005B_CBF0,
        45 => 0x005B_CB90,
        49 => 0x005B_70B0,
        50 => 0x005B_7160,
        52 => 0x005B_CE80,
        53 => 0x005B_BB70,
        60 => 0x005B_BBA0,
        // CountessChest.
        47 => 0x0059_5A50,
        // CainStartPosition.
        54 => 0x0059_40E0,
        // CainPortal.
        61 => 0x0059_4290,
        // TrappedSoulPlaceHolder.
        46 => 0x0055_06D0,
        // Act IV objects (q-a4-endgame).
        48 => 0x005B_5A20,
        55 => 0x005B_5590,
        56 => 0x005B_5620,
        59 => 0x0054_FE10,
        78 => 0x005B_5840,
        // Act V objects.
        62 => 0x0058_86A0,
        63 => 0x0058_D150,
        64 => 0x0058_D190,
        65 => 0x0058_D110,
        66 => 0x0058_EAC0,
        67 => 0x0058_A5B0,
        68 => 0x0058_A610,
        69 => 0x0058_A6C0,
        70 => 0x0058_7830,
        71 => 0x0058_7840,
        72 => 0x0058_D240,
        73 => 0x0058_D280,
        74 => 0x0058_AA50,
        75 => 0x0058_E670,
        76 => 0x0058_D640,
        77 => 0x0058_E710,
        79 => 0x0058_E830,
        _ => return None,
    })
}

/// The quest operate functions the quest code implements (in whole, or
/// in part for 34), by `OperateFn` index, with the function address in the
/// table.
pub fn operate_fn(n: u8) -> Option<u32> {
    Some(match n {
        // TowerTome.
        6 => 0x0059_4E70,
        // Act III (q-a3-quests).
        28 => 0x005B_7A60,
        31 => 0x005B_9B40,
        44 => 0x005B_84E0,
        45 => 0x005B_8530,
        53 => 0x005B_B980,
        57 => 0x005B_8860,
        58 => 0x005B_8940,
        59 => 0x005B_8A20,
        // Monolith (Cairn stone).
        9 => 0x0059_3710,
        // CainGibbet.
        10 => 0x0059_3480,
        // InifussTree.
        12 => 0x0059_3AF0,
        // HoradrimMalus.
        21 => 0x0059_1AC0,
        // TaintedSunAltar.
        24 => 0x0059_A7E0,
        // StaffOrifice.
        25 => 0x0059_DC70,
        // WirtsBody.
        33 => 0x0058_3E70,
        // ArcaneSanctuaryPortal: its level-dependent call.
        34 => 0x0058_46B0,
        // Cube / scroll / staff chests.
        39 => 0x0059_9DF0,
        40 => 0x0059_9C10,
        41 => 0x0059_9CF0,
        // SanctuaryTome.
        42 => 0x0059_B970,
        // Act IV objects (q-a4-endgame).
        49 => 0x005B_5C10,
        52 => 0x005B_5630,
        54 => 0x005B_6B70,
        55 => 0x005B_6BC0,
        56 => 0x005B_6C10,
        73 => 0x005B_5880,
        // Act V objects.
        62 => 0x0058_D1E0,
        63 => 0x0058_D200,
        64 => 0x0058_D220,
        65 => 0x0058_D310,
        66 => 0x0058_D400,
        67 => 0x0058_ABC0,
        69 => 0x0058_D5E0,
        70 => 0x0058_E6A0,
        71 => 0x0058_D6A0,
        72 => 0x0058_E740,
        _ => return None,
    })
}

/// Init function `n` ([`init_fn`]) on the object.
fn init<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    c: &QuestObjectCall,
    object: crate::units::UnitId,
    n: u8,
) -> QuestObjectRun {
    if init_fn(n).is_none() {
        return QuestObjectRun::HandBack(c.route);
    }
    // The init point of the object (room, x, y); a null room has none.
    let at = c.room.map(|room| InitPoint {
        room,
        x: c.x,
        y: c.y,
    });
    w.set_init_point(c.room.map(|room| (object, c.x, c.y, room)));
    match n {
        4 => act1::q5::object_init(ctl, w, object),
        // Chain 4's record exists: link the object to it (special case first);
        // else mode 2 unless it is already in mode 2.
        13 => {
            if !ctl.add_link(w, object, 4, Some(0x0059_2F80)) && w.object_mode(object) != 2 {
                w.set_object_mode(object, 2);
            }
        }
        23 => act3::tome_init(ctl, w, object),
        25 => {
            if let Some(at) = at {
                act3::decoy_init(ctl, w, object, at);
            }
        }
        39 => {
            if let Some(at) = at {
                act3::altar_init(ctl, w, object, at);
            }
        }
        41 => act3::stairs_init(ctl, w, object),
        42 => act3::lever_init(ctl, w, object),
        43 => {
            if let Some(at) = at {
                act3::wanderer_init(ctl, w, at);
            }
        }
        44 => act3::hellgate_init(ctl, w, object),
        45 => act3::bridge_init(ctl, w, object),
        49 => {
            if let Some(at) = at {
                act3::hratli_start_init(ctl, w, at);
            }
        }
        50 => {
            if let Some(at) = at {
                act3::hratli_end_init(ctl, w, at);
            }
        }
        52 => act3::natalya_init(ctl, w, object),
        53 => act3::stairs_r_init(ctl, w, object),
        60 => act3::orb_init(ctl, w, object),
        48 => act4::q3::forge_init(ctl, w, object),
        55 => act4::q2::start_point_init(ctl, w, object),
        59 => act4::q2::dummy_init(w, object),
        78 => act4::q2::portal_init(ctl, w, object),
        // The stone's class is its value.
        6 => act1::q4::stone_init(ctl, w, object, c.class),
        7 => act1::q4::gibbet_init(ctl, w, object),
        9 => act1::q4::tree_init(ctl, w, object),
        15 => act1::malus_init(ctl, w, object),
        18 => {
            if let Some(at) = at {
                act2::q4::start_jerhyn_init(ctl, w, at);
            }
        }
        19 => {
            if let Some(at) = at {
                act2::q4::palace_jerhyn_init(ctl, w, object, at);
            }
        }
        20 => act2::q3::altar_init(ctl, w, object),
        21 => act2::q6::orifice_init(ctl, w, object),
        29 => act2::q4::portal_init(ctl, w, object),
        30 => act2::q4::blocker_init(ctl, w, object),
        38 => act2::q6::door_init(ctl, w, object),
        47 => act1::q5::chest_init(ctl, w, object),
        // The init args' room and position; a null room spawns nothing.
        54 => act1::q4::marker_init(ctl, w, object, c.room, c.x, c.y),
        46 => placeholders::trapped_soul_init(w, object, at),
        61 => act1::q4::cain_portal_init(w, object),
        62 => act5::q2::cage_init(ctl, w, object),
        // The statue's class (474–476) picks its slot.
        63..=65 => act5::q5::statue_init(ctl, w, object, c.class),
        66 => act5::q3::anya_town_dummy_init(ctl, w, object),
        67 => act5::q3::anya_dummy_init(ctl, w, object),
        68 => act5::q3::nihlathak_town_dummy_init(ctl, w, object),
        69 => act5::q3::nihlathak_temple_dummy_init(ctl, w, object),
        71 => act5::q1::larzuk_dummy_init(ctl, w, object),
        72 => act5::q5::altar_init(ctl, w, object),
        73 => act5::q5::keep_door_init(ctl, w, object),
        74 => act5::q3::frozen_anya_init(ctl, w, object),
        75 => act5::q6::portal_init(ctl, w, object),
        76 => act5::q5::summit_door_init(ctl, w, object),
        77 => act5::q6::last_portal_init(ctl, w, object),
        79 => act5::q6::zoo_init(ctl, w),
        // 31–33 and 70: `ret`.
        _ => {}
    }
    w.set_init_point(None);
    QuestObjectRun::Ran
}

/// Operate function `n` ([`operate_fn`]). Every quest operate takes the
/// operating player; without an operator the route is handed back.
fn operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    c: &QuestObjectCall,
    op: Operate,
) -> QuestObjectRun {
    let (Some(player), Some(_)) = (op.operator, operate_fn(op.operate_fn)) else {
        return QuestObjectRun::HandBack(c.route);
    };
    let o = op.object;
    match op.operate_fn {
        6 => act1::q5::tome_operate(ctl, w, o, player),
        28 => act3::tome_operate(ctl, w, o, player),
        31 => act3::decoy_operate(ctl, w, o, player),
        44 => act3::stairs_operate(ctl, w, o, player),
        45 => act3::lever_operate(ctl, w, o, player),
        53 => act3::orb_operate(ctl, w, o, player),
        57 => act3::chest_operate(ctl, w, o, player, KhalimChest::Heart),
        58 => act3::chest_operate(ctl, w, o, player, KhalimChest::Eye),
        59 => act3::chest_operate(ctl, w, o, player, KhalimChest::Brain),
        // The stone's value is its class.
        9 => act1::q4::stone_operate(ctl, w, o, player, op.class),
        10 => act1::q4::gibbet_operate(ctl, w, o, player),
        12 => act1::q4::tree_operate(ctl, w, o, player),
        21 => act1::malus_operate(ctl, w, o, player),
        33 => act1::q4::wirt_body_operate(w, o),
        24 => {
            act2::q3::altar_operate(ctl, w, o, player);
        }
        // With the "no chain 13 record gives 0" step.
        25 => {
            act2::q6::orifice_operate_checked(ctl, w, o, player);
        }
        // Only the level-dependent call is a quest call; the rest of the
        // operate (the object module's) is handed back.
        // Mode 0: mode 1 and the open event one frame after the animation (no
        // quest call); an open portal runs the level-dependent call below.
        34 if w.object_mode(o) == 0 => {
            w.set_object_mode(o, 1);
            let at = w.frame() + (w.object_anim_length(o) >> 8) + 1;
            w.schedule_object_event(o, 1, at);
            return QuestObjectRun::Ran;
        }
        34 => {
            if let Some(level) = w.unit_level(o) {
                act2::q4::portal_operate(ctl, w, level);
            }
            return QuestObjectRun::HandBack(c.route);
        }
        39 => {
            act2::q2::cube_chest(ctl, w, o, player);
        }
        40 => {
            act2::q2::scroll_chest(ctl, w, o, player);
        }
        41 => {
            act2::q2::staff_chest(ctl, w, o, player);
        }
        42 => act2::q4::tome_operate(ctl, w, o, player),
        62..=64 => {
            act5::q5::statue_operate(w, player);
        }
        65 => {
            act5::q5::altar_operate(ctl, w, o, player);
        }
        66 => {
            act5::q5::keep_door_operate(ctl, w, o, player);
        }
        67 => {
            act5::q3::frozen_anya_operate(ctl, w, o, player);
        }
        69 => {
            act5::q5::invisible_ancient_operate(w, o, player);
        }
        70 => {
            act5::q6::portal_operate(ctl, w, player);
        }
        71 => {
            act5::q5::summit_door_operate(ctl, w, o, player);
        }
        72 => {
            act5::q6::last_portal_operate(ctl, w, player);
        }
        49 => act4::q3::forge_operate(ctl, w, o, player),
        52 => act4::q2::seal_operate(ctl, w, o, player, op.class),
        54 => act4::q2::infector_seal_operate(ctl, w, o, player, op.class),
        55 => act4::q2::de_seis_seal_operate(ctl, w, o, player, op.class),
        56 => act4::q2::vizier_seal_operate(ctl, w, o, player, op.class),
        73 => act4::q2::portal_operate(ctl, w, o, player),
        _ => return QuestObjectRun::HandBack(c.route),
    }
    QuestObjectRun::Ran
}

impl<X: Pending> View<'_, X> {
    /// The quest link for `unit` and `chain` where the game makes it, inside
    /// monster init or item creation: on the lent quest host
    /// ([`QuestObjectHost::add_link`]) now; a host that lends none (or a call
    /// from inside a lent quest call) queues it on the Pending
    /// ([`Pending::monster_quest_chain`]).
    pub fn quest_link_now(&mut self, game: &mut Game, unit: UnitId, chain: u32) {
        if let (Ok(c), Some(mut host)) = (u8::try_from(chain), self.h.quest_host.take()) {
            let done = host.add_link(game, self, unit, c);
            self.h.quest_host = Some(host);
            if done {
                return;
            }
        }
        self.h.x.monster_quest_chain(unit, chain);
    }

    /// The quest link for a caller with no game at hand (a monster placed
    /// outside its creation function): recorded in the item store's pending
    /// links and applied at the next quest call, before it runs anything.
    pub fn queue_quest_link(&mut self, unit: UnitId, chain: u32) {
        if let Ok(c) = u8::try_from(chain) {
            self.h.items.push_quest_link(unit, c);
        }
    }

    /// The kill parse inside the kill: on the lent quest host
    /// ([`QuestObjectHost::kill_parse`]) now; otherwise queued through
    /// [`Pending::kill_step`] (`KillStep::QuestKill`).
    pub fn quest_kill_now(&mut self, game: &mut Game, victim: UnitId, killer: UnitId) {
        if let Some(mut host) = self.h.quest_host.take() {
            let done = host.kill_parse(game, self, victim, Some(killer));
            self.h.quest_host = Some(host);
            if done {
                return;
            }
        }
        self.h.x.kill_step(
            game,
            crate::wiring::action::KillStep::QuestKill,
            victim,
            killer,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::objects::{init_route, operate_route};

    const FUNCS_TSV: &str = include_str!("../../../../../tables/world/object-functions.tsv");

    /// Every implemented index against its `object-functions.tsv` row: owner
    /// the quest code, the same address, a quest route in the object module;
    /// returns the mismatches.
    fn check(init: impl Fn(u8) -> Option<u32>, operate: impl Fn(u8) -> Option<u32>) -> Vec<String> {
        let rows: Vec<(String, u32, u32, String)> = FUNCS_TSV
            .lines()
            .skip(1)
            .map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                let addr = u32::from_str_radix(f[2].trim_start_matches("0x"), 16).unwrap_or(0);
                (
                    f[0].to_string(),
                    f[1].parse().unwrap(),
                    addr,
                    f[5].to_string(),
                )
            })
            .collect();
        let mut bad = Vec::new();
        for n in 0..=u8::MAX {
            for (kind, got, route) in [
                ("init", init(n), init_route(n)),
                ("operate", operate(n), operate_route(n)),
            ] {
                let Some(addr) = got else { continue };
                let row = rows.iter().find(|r| r.0 == kind && r.1 == u32::from(n));
                match row {
                    Some(r) if r.2 == addr && r.3 == "world/quests.md" && route == Route::Quest => {
                    }
                    _ => bad.push(format!("{kind} {n}")),
                }
            }
        }
        bad
    }

    #[test]
    fn stated_functions_match_the_table() {
        assert_eq!(check(init_fn, operate_fn), Vec::<String>::new());
        // Every Act II row with an init or operate is implemented.
        for n in [20, 21, 29, 30, 31, 32, 33, 38] {
            assert!(init_fn(n).is_some(), "init {n}");
        }
        for n in [24, 25, 34, 39, 40, 41, 42] {
            assert!(operate_fn(n).is_some(), "operate {n}");
        }
    }

    #[test]
    fn act_v_functions_are_stated() {
        for n in [
            62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 79,
        ] {
            assert!(init_fn(n).is_some(), "init {n}");
        }
        for n in [62, 63, 64, 65, 66, 67, 69, 70, 71, 72] {
            assert!(operate_fn(n).is_some(), "operate {n}");
        }
    }

    #[test]
    fn act1_answered_functions_are_stated() {
        // WW-6: the gibbet, tree and cain portal inits, Wirt's body's operate;
        // Wirt's body has no init and init 37 is not Act I's.
        for n in [7, 9, 61] {
            assert!(init_fn(n).is_some(), "init {n}");
        }
        assert!(operate_fn(33).is_some());
        assert!(init_fn(37).is_none());
    }

    #[test]
    fn act3_functions_are_stated() {
        for n in [23, 25, 39, 41, 42, 43, 44, 45, 49, 50, 52, 53, 60] {
            assert!(init_fn(n).is_some(), "init {n}");
        }
        for n in [28, 31, 44, 45, 53, 57, 58, 59] {
            assert!(operate_fn(n).is_some(), "operate {n}");
        }
    }

    #[test]
    fn act4_functions_are_stated() {
        for n in [48, 55, 56, 59, 78] {
            assert!(init_fn(n).is_some(), "init {n}");
        }
        for n in [49, 52, 54, 55, 56, 73] {
            assert!(operate_fn(n).is_some(), "operate {n}");
        }
    }

    // A wrong address and a non-quest index are reported.
    #[test]
    fn table_check_catches_perturbations() {
        let wrong = |n: u8| {
            if n == 10 {
                Some(0x0059_3481)
            } else {
                operate_fn(n)
            }
        };
        assert_eq!(check(init_fn, wrong), vec!["operate 10".to_string()]);
        let not_quest = |n: u8| {
            if n == 1 {
                Some(0x0058_6410)
            } else {
                init_fn(n)
            }
        };
        assert_eq!(check(not_quest, operate_fn), vec!["init 1".to_string()]);
    }

    /// A lent quest host recording the links and kill parses it ran;
    /// `lent` false keeps the trait defaults (not run here).
    struct HookLog {
        lent: bool,
        log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    }

    impl QuestObjectHost<crate::wiring::action::tests::TestPending> for HookLog {
        fn run(
            &mut self,
            _: &mut Game,
            _: &mut View<'_, crate::wiring::action::tests::TestPending>,
            _: QuestObjectCall,
        ) -> Option<ObjectRoute> {
            None
        }
        fn add_link(
            &mut self,
            _: &mut Game,
            _: &mut View<'_, crate::wiring::action::tests::TestPending>,
            unit: UnitId,
            chain: u8,
        ) -> bool {
            self.log
                .borrow_mut()
                .push(format!("link {} {chain}", unit.0));
            self.lent
        }
        fn kill_parse(
            &mut self,
            _: &mut Game,
            _: &mut View<'_, crate::wiring::action::tests::TestPending>,
            victim: UnitId,
            killer: Option<UnitId>,
        ) -> bool {
            let k = killer.map_or(0, |k| k.0);
            self.log.borrow_mut().push(format!("kill {} {k}", victim.0));
            self.lent
        }
        fn ai_quest_call(
            &mut self,
            _: &mut Game,
            _: &mut View<'_, crate::wiring::action::tests::TestPending>,
            unit: UnitId,
            call: crate::monsters::ai::QuestCall,
        ) -> Option<bool> {
            self.log
                .borrow_mut()
                .push(format!("ai {} {call:?}", unit.0));
            self.lent.then_some(true)
        }
        fn anya_open_portal(
            &mut self,
            _: &mut Game,
            _: &mut View<'_, crate::wiring::action::tests::TestPending>,
            unit: UnitId,
        ) -> bool {
            self.log.borrow_mut().push(format!("anya {}", unit.0));
            self.lent
        }
        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
            self
        }
    }

    // An AI body's quest call runs on the lent quest control inside the
    // think; with none lent (or not run there) the Pending answers.
    #[test]
    fn ai_quest_calls_run_on_the_lent_host() {
        use crate::monsters::ai::{AiActs, QuestCall};
        let mut fx = crate::wiring::action::tests::Fx::new();
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        fx.sim.sys.hooks.quest_host = Some(Box::new(HookLog {
            lent: true,
            log: log.clone(),
        }));
        let game = &mut fx.game;
        let r = fx.sim.with(game, |g, view| {
            view.quest_call(g, UnitId(3), QuestCall::RadamentActivated)
        });
        assert!(r);
        assert_eq!(*log.borrow(), ["ai 3 RadamentActivated"]);
        assert!(fx.sim.sys.hooks.quest_host.is_some());
        // None lent: the Pending's answer (TestPending: false).
        fx.sim.sys.hooks.quest_host = None;
        let game = &mut fx.game;
        let r = fx.sim.with(game, |g, view| {
            view.quest_call(g, UnitId(3), QuestCall::SummonerActivated)
        });
        assert!(!r);
        assert_eq!(log.borrow().len(), 1);
    }

    // The Act V AI calls (Shenk, Nihlathak, the Baal crab) and Anya's portal
    // request run on the lent quest control inside the think.
    #[test]
    fn act_v_ai_calls_run_on_the_lent_host() {
        use crate::monsters::ai::{AiActs, AiQuests, QuestCall};
        let mut fx = crate::wiring::action::tests::Fx::new();
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        fx.sim.sys.hooks.quest_host = Some(Box::new(HookLog {
            lent: true,
            log: log.clone(),
        }));
        let game = &mut fx.game;
        fx.sim.with(game, |g, view| {
            view.quest_call(g, UnitId(4), QuestCall::Shenk);
            view.anya_open_portal(g, UnitId(6));
        });
        assert_eq!(*log.borrow(), ["ai 4 Shenk", "anya 6"]);
        assert!(fx.sim.sys.hooks.quest_host.is_some());
    }

    // The links of monster init / item creation and the kill parse run on
    // the lent quest control inside their caller; a host that lends none
    // (or answers false) queues them on the Pending as before.
    #[test]
    fn quest_hooks_run_on_the_lent_host_else_queue() {
        let mut fx = crate::wiring::action::tests::Fx::new();
        let (v, k) = (UnitId(7), UnitId(9));
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        // Lent: run now, nothing queued, the host goes back to the hooks.
        fx.sim.sys.hooks.quest_host = Some(Box::new(HookLog {
            lent: true,
            log: log.clone(),
        }));
        let game = &mut fx.game;
        fx.sim.with(game, |g, view| {
            view.quest_link_now(g, v, 2);
            view.quest_kill_now(g, v, k);
        });
        assert_eq!(*log.borrow(), ["link 7 2", "kill 7 9"]);
        assert!(fx
            .sim
            .sys
            .hooks
            .x
            .log
            .iter()
            .all(|l| !l.starts_with("kill")));
        assert!(fx.sim.sys.hooks.quest_host.is_some());
        // Not run by the host: the kill goes to the Pending's kill step.
        fx.sim.sys.hooks.quest_host = Some(Box::new(HookLog {
            lent: false,
            log: log.clone(),
        }));
        let game = &mut fx.game;
        fx.sim.with(game, |g, view| view.quest_kill_now(g, v, k));
        assert!(fx
            .sim
            .sys
            .hooks
            .x
            .log
            .contains(&"kill QuestKill 7 9".to_string()));
        // None lent (inside a lent call): queued too.
        fx.sim.sys.hooks.quest_host = None;
        fx.sim.sys.hooks.x.log.clear();
        let game = &mut fx.game;
        fx.sim.with(game, |g, view| view.quest_kill_now(g, v, k));
        assert_eq!(fx.sim.sys.hooks.x.log, ["kill QuestKill 7 9"]);
    }

    // A link with no game at hand waits in the item store's pending list
    // for the next quest call.
    #[test]
    fn a_queued_link_waits_in_the_item_store() {
        let mut fx = crate::wiring::action::tests::Fx::new();
        let game = &mut fx.game;
        fx.sim.with(game, |_, view| {
            view.queue_quest_link(UnitId(5), 23);
            view.queue_quest_link(UnitId(6), 300);
        });
        // Chains past u8 are no chain.
        assert_eq!(
            fx.sim.sys.hooks.items.take_quest_links(),
            vec![(UnitId(5), 23)]
        );
    }
}
