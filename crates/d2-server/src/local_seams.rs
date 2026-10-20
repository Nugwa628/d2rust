//! [`LocalSeams`]: the seams without a provider (the action and world
//! wiring's `Pending` / `WorldPending` and the transport outbox), the
//! combat seams' copy of the units ([`sync_seams`]). Split from
//! `single_player.rs` (W2-09, pure move).

use crate::adapters::handlers::world::{Outbox, WiredWorld};
use crate::skill_rest::SkillStore;
use d2_sim::game::Game;
use d2_sim::units::hooks::Sim as USim;
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::action::{ActionHooks, Pending, SkillEvent};
use d2_sim::wiring::interaction::skill_events;
use d2_sim::wiring::worldgen::{WorldPending, WorldSim};
use d2_sim::world::quests::QuestFlags;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The action and world wiring's seams without a provider. Positions
/// are stored as the sim sets them (the interaction target is the unit
/// record's, `UnitRecord::interact`); messages the sim
/// sends wait in `sent` for the world handlers ([`Outbox`]); warp and
/// arrival mode, whose bodies are unwritten specs, are logged. The world
/// wiring's seams keep `WorldPending`'s defaults.
#[derive(Debug, Default)]
pub struct LocalSeams {
    pub pos: BTreeMap<UnitId, (i32, i32)>,
    pub sent: Vec<(UnitId, Vec<u8>)>,
    pub log: Vec<String>,
    /// The game's players and monsters (type, allied, path position),
    /// copied by [`sync_seams`] before each intent and tick: the
    /// hostility, alignment and melee-range seams have no game to read.
    pub sides: BTreeMap<UnitId, (UnitType, bool, (i32, i32))>,
    /// The players and monsters of [`Self::sides`] that are dying or dead
    /// (player modes 0 / 17, monster modes 0 / 12).
    pub down: std::collections::BTreeSet<UnitId>,
    /// Unit sizes copied from path records, used by the preview melee-range check.
    pub sizes: BTreeMap<UnitId, i32>,
    /// The socketed children of each item with a non-empty own inventory
    /// (list order), copied from the inventory model by [`world_sync`]
    /// for the inactive item record.
    pub sockets: BTreeMap<UnitId, Vec<UnitId>>,
    /// The keys each player carries in the inventory page, counted from the
    /// inventory model by [`world_sync`] for the key test of a locked
    /// chest or door ([`Pending::object_key_test`]); a passed test takes
    /// one until the next sync.
    pub keys: BTreeMap<UnitId, i32>,
    /// Monsters carrying a usable shield (`SH`), set by [`crate::weapons::sync`].
    pub shielded: std::collections::BTreeSet<UnitId>,
    /// The skill pipeline's per-unit fields and preview fills (`UseRest`,
    /// `LearnRest`: [`crate::skill_rest`]).
    pub skills: SkillStore,
    /// The monsters' mode targets and current skills
    /// ([`crate::monster_ai`]).
    pub monsters: crate::monster_ai::MonsterAi,
    /// The unit tables of the client art's name rules ([`UnitAppearance`]);
    /// none on synthetic data.
    pub looks: Option<Arc<dyn UnitAppearance>>,
    /// The players and monsters for the host rest's NPC and quest seams
    /// (`npc_seams`), written by [`sync_seams`].
    pub snap: crate::npc_seams::SnapRef,
    /// The character is hardcore (the save's status bit 0x4, or `play
    /// --hardcore`): client flag 4 of the one local client
    /// (the client save status).
    pub hardcore: bool,
    /// The clients the server dropped, with the reason
    /// (`Pending::drop_client`): the hardcore resurrect drops with 3.
    pub dropped: Vec<(UnitId, u32)>,
    /// Monster chain links and deaths for the quest control, drained once
    /// per tick (`q-a1-tower`, D2RUST's own, unverified).
    pub quest_events: Vec<d2_sim::wiring::action::QuestEvent>,
    /// The Act II DRLG's staff-tomb level (0 = not yet known), set when
    /// the acts are created (`q-a2-dungeons`).
    pub staff_tomb: u32,
    /// The lair warp check's answer, published once per tick by the quest
    /// control (`Pending::set_lair_open`, q-a2-duriel).
    pub lair_open: bool,
    /// The Arreat Summit warp check's answer (`Pending::set_summit_open`,
    /// q-act3-act5-gaps); the exits stay closed while it is `true`.
    pub summit_closed: bool,
    /// Whether the Ancients' fight is armed; the AI gate refuses activation otherwise.
    pub ancients_armed: bool,
    /// Quest-chain not-intro answers, published by the quest control once per tick.
    pub not_intro: BTreeMap<u8, bool>,
    /// The players' quest flags for the difficulty, published by the
    /// quest control once per tick (`Pending::publish_quest_flags`), for
    /// the quest TC test.
    pub quest_flags: BTreeMap<UnitId, QuestFlags>,
    /// The caged barbarians' group states by GUID (counting, portal
    /// GUID), published by the quest control once per tick
    /// (`Pending::publish_rescue`).
    pub rescue: BTreeMap<u32, (bool, Option<u32>)>,
    /// The Durance of Hate warp check's answer
    /// (`Pending::set_durance_open`, q-play-act3); level 100 stays closed
    /// while it is `true`.
    pub durance_closed: bool,
    /// Whether Alkor has the Golden Bird, and the Gidbinn altar's point while Ormus
    /// may activate it (`Pending::set_act3_npc_answers`, q-play-act3).
    pub alkor_bird: bool,
    pub ormus_altar: Option<(i32, i32)>,
    /// The players' hands and the facts of the items in them
    /// ([`crate::weapons`], q-amazon).
    pub weapons: crate::weapons::Weapons,
}

impl LocalSeams {
    /// Melee reach: a player's weapon `rangeadder`, or the preview reach for other units.
    fn reach(&self, u: UnitId) -> i32 {
        match self.sides.get(&u) {
            Some(&(UnitType::Player, ..)) => self.weapons.range_adder(u),
            _ => PREVIEW_MELEE_RANGE,
        }
    }

    /// Player side: a player or an allied (good-aligned) monster.
    fn player_side(&self, unit: UnitId) -> Option<bool> {
        self.sides
            .get(&unit)
            .map(|&(ty, allied, _)| ty == UnitType::Player || allied)
    }
}

/// D2RUST's own preview melee reach, in sub-tiles.
const PREVIEW_MELEE_RANGE: i32 = 2;

/// The play host's seam refresh (`SimGame::set_host_sync`): the players
/// and monsters with their allied flag (`UnitLists`), for
/// [`LocalSeams::sides`].
pub fn sync_seams(game: &Game, sim: &mut WorldSim<LocalSeams>) {
    let classes: BTreeMap<UnitId, u32> = [UnitType::Player, UnitType::Monster]
        .into_iter()
        .flat_map(|ty| game.lists.units_of_type(ty))
        .filter_map(|u| Some((u, sim.action.sys.units.get(u)?.class)))
        .collect();
    let down: std::collections::BTreeSet<UnitId> = classes
        .keys()
        .copied()
        .filter(|&u| {
            sim.action.sys.units.get(u).is_some_and(|r| match r.ty {
                UnitType::Player => matches!(r.mode, 0 | 17),
                _ => matches!(r.mode, 0 | 12),
            })
        })
        .collect();
    let hooks = &mut sim.action.sys.hooks;
    hooks.x.down = down;
    // Player pets inherit the player side. Monster pets use their owner's alignment,
    // so only pets of players and allied owners are listed here.
    let pets: std::collections::BTreeSet<u32> = hooks
        .pet_lists
        .iter()
        .filter(|(o, _)| {
            game.lists
                .unit(**o)
                .is_none_or(|e| e.ty != UnitType::Monster || e.allied)
        })
        .flat_map(|(_, l)| l.entries.iter())
        .flat_map(|e| e.nodes.iter().map(|n| n.guid as u32))
        .collect();
    let mut sides = BTreeMap::new();
    let mut sizes = BTreeMap::new();
    for ty in [UnitType::Player, UnitType::Monster] {
        for u in game.lists.units_of_type(ty) {
            if let Some(e) = game.lists.unit(u) {
                let pet = ty == UnitType::Monster && pets.contains(&e.guid);
                sides.insert(u, (ty, e.allied || pet, hooks.path_position(u)));
                let size = hooks.paths.as_ref().and_then(|p| p.record(u));
                let size = match size {
                    Some(d2_sim::path::UnitPath::Dynamic(d)) => d.unit_size,
                    _ => 0,
                };
                sizes.insert(u, size);
            }
        }
    }
    // The init-given skill levels under the
    // summons' entries: the used skill's base level (the egg hatch's count).
    let mut levels: BTreeMap<_, BTreeMap<i32, i32>> = hooks
        .natural_skills
        .iter()
        .map(|(u, l)| (*u, l.to_map()))
        .collect();
    for (u, m) in &hooks.monster_skills {
        levels.entry(*u).or_default().extend(m.to_map());
    }
    hooks.x.monsters.set_levels(levels);
    hooks.x.sides = sides;
    hooks.x.sizes = sizes;
    let mut units = BTreeMap::new();
    // Objects too: the quest object events read the object's act and room level
    // (class 189: "room level in Act I").
    for ty in [UnitType::Player, UnitType::Monster, UnitType::Object] {
        for u in game.lists.units_of_type(ty) {
            let Some(e) = game.lists.unit(u) else {
                continue;
            };
            let act = e
                .room()
                .and_then(|r| game.lists.room(r))
                .map_or(0, |r| r.act);
            let level = e
                .room()
                .and_then(|r| hooks.drlg.level_id(game, r))
                .unwrap_or(0);
            let class = classes.get(&u).copied().unwrap_or(0);
            units.insert(
                u,
                crate::npc_seams::SnapUnit {
                    ty,
                    pos: hooks.path_position(u),
                    act,
                    guid: e.guid,
                    level,
                    class,
                },
            );
        }
    }
    if let Ok(mut snap) = hooks.x.snap.lock() {
        snap.set_units(units);
    }
    crate::skill_rest::sync_shapes(game, sim);
}

/// The play host's world sync: the hands ([`crate::weapons::sync`]), then
/// the socketed children of [`LocalSeams::sockets`].
pub fn world_sync<R, S>(game: &Game, sim: &mut WorldSim<LocalSeams>, world: &mut WiredWorld<R, S>) {
    crate::weapons::sync(game, sim, world);
    let Some(inv) = world.inventory.as_ref() else {
        return;
    };
    let items = &sim.action.sys.units;
    sim.action.sys.hooks.x.sockets = inv
        .state
        .inventories
        .iter()
        .filter(|(o, i)| {
            !i.items().is_empty() && items.get(**o).is_some_and(|r| r.ty == UnitType::Item)
        })
        .map(|(&o, i)| (o, i.items().to_vec()))
        .collect();
    let stats = &sim.action.sys.stats;
    let keys = |i: &UnitId| {
        let is_key = inv.state.items.get(i).is_some_and(|d| {
            d.page == d2_sim::items::moves::page::INVENTORY
                && inv
                    .tables
                    .item(d.record)
                    .is_some_and(|r| r.code == d2_sim::items::moves::handlers::KEY_CODE)
        });
        if is_key {
            stats
                .unit_base(*i, d2_sim::items::moves::stat::QUANTITY, 0)
                .max(0)
        } else {
            0
        }
    };
    sim.action.sys.hooks.x.keys = inv
        .state
        .inventories
        .iter()
        .filter(|(o, _)| items.get(**o).is_some_and(|r| r.ty == UnitType::Player))
        .map(|(&o, i)| (o, i.items().iter().map(keys).sum::<i32>()))
        .filter(|&(_, n)| n > 0)
        .collect();
}

impl Pending for LocalSeams {
    /// A key in the player's inventory (the last sync's copy): the key is
    /// used up on the inventory model at the mark's place of the tick.
    fn object_key_test(&mut self, player: UnitId) -> bool {
        match self.keys.get_mut(&player) {
            Some(n) if *n > 0 => *n -= 1,
            _ => return false,
        }
        let mark = [d2_sim::wiring::action::pending::KEY_USE_MARK, 0, 0, 0, 0];
        self.sent.push((player, mark.to_vec()));
        true
    }
    /// The children the last sync copied.
    fn socketed_items(&self, item: UnitId) -> Vec<UnitId> {
        self.sockets.get(&item).cloned().unwrap_or_default()
    }
    /// The inventory model is the world's,
    /// out of reach here: a mark to the parent, linked by the host at
    /// this place of the tick.
    fn socket_restore(&mut self, game: &mut Game, parent: UnitId, child: UnitId) {
        let Some(guid) = game.lists.unit(child).map(|e| e.guid) else {
            return;
        };
        let mut mark = [d2_sim::wiring::action::pending::SOCKET_RESTORE_MARK; 5];
        mark[1..].copy_from_slice(&guid.to_le_bytes());
        self.sent.push((parent, mark.to_vec()));
    }
    /// The equipped weapon selected by the inventory sync.
    fn weapon_in_use(&self, unit: UnitId) -> Option<UnitId> {
        self.weapons.hands.get(&unit)?.in_use
    }

    /// The item's `Throwable` flag from its primary type.
    fn item_flag_throw(&self, item: UnitId) -> bool {
        self.weapons.facts(item).throwable
    }

    /// Whether the item can be used for Iron Golem (skills/bodies-2b.md section 7.11).
    fn golem_item(&self, unit: UnitId) -> bool {
        self.weapons.golem_item(unit)
    }

    fn sent_len(&self) -> Option<usize> {
        Some(self.sent.len())
    }

    /// Queue the monster's sound slot for the next S-to-C 0x2C update.
    fn play_sound(&mut self, game: &mut Game, unit: UnitId, sound: u32, to: Option<UnitId>) {
        let _ = d2_sim::units::sound::queue_sound(game, unit, sound as u16, to);
    }

    fn frame_event_index(&self, unit: UnitId) -> i32 {
        LocalSeams::frame_event_index(self, unit)
    }

    fn set_frame_event_index(&mut self, unit: UnitId, i: i32) {
        LocalSeams::set_frame_event_index(self, unit, i);
    }

    fn body_path_op(
        &mut self,
        unit: UnitId,
        op: d2_sim::skills::use_::bodies::PathOp<UnitId>,
    ) -> i32 {
        LocalSeams::path_op(self, unit, op);
        0
    }

    /// Client flag 4: the local character is hardcore.
    fn client_hardcore(&self, _player: UnitId) -> bool {
        self.hardcore
    }

    /// Record the dropped client and reason for the caller to drain.
    fn drop_client(&mut self, player: UnitId, reason: u32) {
        self.dropped.push((player, reason));
    }

    fn monster_quest_chain(&mut self, unit: UnitId, chain: u32) {
        if let Ok(chain) = u8::try_from(chain) {
            self.quest_events
                .push(d2_sim::wiring::action::QuestEvent::Link { unit, chain });
        }
    }
    /// Radament's and the Summoner's AI calls run on the lent quest
    /// control inside the think (`QuestObjectHost::ai_quest_call`,
    /// the quest's own timing); this queue serves only a tick
    /// with no quest control lent.
    fn ai_quest_call(
        &mut self,
        _: &mut Game,
        unit: UnitId,
        call: d2_sim::monsters::ai::QuestCall,
    ) -> bool {
        use d2_sim::monsters::ai::QuestCall;
        use d2_sim::wiring::action::QuestEvent;
        match call {
            QuestCall::RadamentActivated => self
                .quest_events
                .push(QuestEvent::RadamentActivated { unit }),
            QuestCall::SummonerActivated => {
                self.quest_events.push(QuestEvent::SummonerActivated);
                // The event runs after the tick: mark where the AI raised
                // it in the sent stream (`QUEST_EVENT_MARK`).
                let mut m = [0u8; 5];
                m[0] = d2_sim::wiring::action::QUEST_EVENT_MARK;
                self.sent.push((unit, m.to_vec()));
            }
            // The Act V AI calls of a tick with no quest control lent (the
            // lent path runs them in the think).
            QuestCall::Shenk => self.quest_events.push(QuestEvent::ShenkActivated { unit }),
            QuestCall::Nihlathak => self.quest_events.push(QuestEvent::NihlathakActivated),
            QuestCall::BaalToStairs => self.quest_events.push(QuestEvent::BaalToStairs),
            // The Ancients cannot activate until their fight is armed.
            QuestCall::AncientsNotActivatable => return !self.ancients_armed,
            _ => return false,
        }
        true
    }
    /// Anya's AI asks for the temple portal; queued only with no quest
    /// control lent (the lent path runs it in the think).
    fn anya_open_portal(&mut self, _: &mut Game, unit: UnitId) {
        self.quest_events
            .push(d2_sim::wiring::action::QuestEvent::AnyaOpenPortal { unit });
    }
    /// The Act II staff tomb, containing the orifice.
    fn object_staff_tomb(&self) -> u32 {
        if self.staff_tomb == 0 {
            u32::MAX
        } else {
            self.staff_tomb
        }
    }
    /// Quest gates for Duriel's Lair, the Summit and Durance of Hate.
    fn warp_quest_gate(&self, source: u32, level: u32) -> u32 {
        u32::from(
            (level == DURIELS_LAIR
                && !(self.lair_open && source == self.staff_tomb))
                // Leaving the Summit is closed while its quest gate is closed.
                || (source == d2_sim::world::quests::act5::q5::SUMMIT
                    && matches!(level, 118 | 128)
                    && self.summit_closed)
                // Durance of Hate 1 waits on the quest gate, except when returning from level 2.
                || (level == DURANCE_1 && source != DURANCE_2 && self.durance_closed),
        )
    }
    fn set_ancients_armed(&mut self, armed: bool) {
        self.ancients_armed = armed;
    }
    fn set_summit_open(&mut self, open: bool) {
        self.summit_closed = !open;
    }
    /// A chain without a published not-intro record defaults to true.
    fn quest_not_intro(&self, chain: u8) -> bool {
        self.not_intro.get(&chain).copied().unwrap_or(true)
    }
    fn publish_not_intro(&mut self, records: &[(u8, bool)]) {
        self.not_intro = records.iter().copied().collect();
    }
    fn publish_quest_flags(&mut self, flags: &[(UnitId, QuestFlags)]) {
        self.quest_flags = flags.iter().copied().collect();
    }
    /// `p` is a player (it has a published record)
    /// whose flags of `quest` have none of 15 (completed before), 1
    /// (reward pending) and `cp`.
    fn quest_tc_open(&self, p: UnitId, quest: u8, cp: u8) -> bool {
        self.quest_flags
            .get(&p)
            .is_some_and(|f| !(f.get(quest, 15) || f.get(quest, 1) || f.get(quest, cp)))
    }
    /// D2RUST's own, unverified: the prisoner AI's hooks with an effect run on
    /// the quest control after the tick.
    fn queue_quest_event(&mut self, e: d2_sim::wiring::action::QuestEvent) {
        self.quest_events.push(e);
    }
    fn quest_rescue(&self, guid: u32) -> (bool, Option<u32>) {
        self.rescue.get(&guid).copied().unwrap_or((false, None))
    }
    fn publish_rescue(&mut self, barbarians: &[(u32, bool, Option<u32>)]) {
        self.rescue = barbarians.iter().map(|&(g, c, p)| (g, (c, p))).collect();
    }
    /// D2RUST's own, unverified: Tyrael's spawn runs on the quest control after
    /// the tick, not inside the missile body.
    fn missile_spawn_tyrael(
        &mut self,
        room: Option<d2_sim::units::RoomId>,
        missile: UnitId,
        x: i32,
        y: i32,
    ) {
        self.quest_events
            .push(d2_sim::wiring::action::QuestEvent::SpawnTyrael {
                room,
                missile,
                x,
                y,
            });
    }
    fn set_lair_open(&mut self, open: bool) {
        self.lair_open = open;
    }
    fn set_durance_open(&mut self, open: bool) {
        self.durance_closed = !open;
    }
    fn set_act3_npc_answers(&mut self, alkor_bird: bool, ormus_altar: Option<(i32, i32)>) {
        self.alkor_bird = alkor_bird;
        self.ormus_altar = ormus_altar;
    }
    /// The quest control's published Golden Bird answer.
    fn alkor_bird(&mut self, _: &mut Game) -> bool {
        self.alkor_bird
    }
    /// Queue the reset for the quest control after the tick. Clear the published
    /// answer immediately so later calls in this tick see it cleared.
    fn alkor_reset(&mut self, _: &mut Game) {
        self.alkor_bird = false;
        self.quest_events
            .push(d2_sim::wiring::action::QuestEvent::AlkorReset);
    }
    /// The quest control's published Ormus altar point.
    fn ormus_altar(&mut self, _: &mut Game) -> Option<(i32, i32)> {
        self.ormus_altar
    }
    /// Clear the altar point and queue its mode update, as with [`Self::alkor_reset`].
    fn ormus_set_altar_mode(&mut self, _: &mut Game) {
        self.ormus_altar = None;
        self.quest_events
            .push(d2_sim::wiring::action::QuestEvent::OrmusAltar);
    }
    /// Queue C-to-S 0x44 for the quest control after the tick. The handler returns
    /// zero; its 0x58 and 0x0E responses therefore leave one frame later.
    fn staff_in_orifice(
        &mut self,
        _: &mut Game,
        player: UnitId,
        object: u32,
        item: u32,
        action: u16,
    ) -> Option<u32> {
        self.quest_events
            .push(d2_sim::wiring::action::QuestEvent::InsertItem {
                player,
                object,
                item,
                action,
            });
        Some(0)
    }
    fn take_quest_events(&mut self) -> Vec<d2_sim::wiring::action::QuestEvent> {
        std::mem::take(&mut self.quest_events)
    }
    fn kill_step(
        &mut self,
        _: &mut Game,
        step: d2_sim::wiring::action::KillStep,
        defender: UnitId,
        attacker: UnitId,
    ) {
        if step == d2_sim::wiring::action::KillStep::QuestKill {
            self.quest_events
                .push(d2_sim::wiring::action::QuestEvent::Kill {
                    victim: defender,
                    killer: Some(attacker),
                });
        }
    }
    /// D2RUST's own, unverified (REC-108): mode DT and the treasure drop
    /// ([`crate::monster_drop::death_start`]).
    fn monster_death_start(
        h: &mut ActionHooks<Self>,
        sim: &mut d2_sim::units::hooks::Sim<'_>,
        unit: UnitId,
        target: Option<UnitId>,
    ) -> bool {
        crate::monster_drop::death_start(h, sim, unit, target)
    }
    // The skill timer events and the action frame reach the skill use
    // pipeline; without them a cast's do step never
    // runs: no mana spent, no missile.
    fn skill_event(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, ev: SkillEvent) {
        skill_events::route(h, sim, ev);
    }
    fn action_frame(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        unit: UnitId,
        a1: u32,
        a2: u32,
    ) -> u32 {
        skill_events::action_frame(h, sim, unit, a1, a2)
    }
    fn apply_state_req(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        req: d2_sim::skills::use_::bodies::helpers::StateRequest<UnitId>,
    ) -> Option<d2_sim::stats::ListId> {
        skill_events::apply_state_req(h, sim, req)
    }
    fn monster_skill_start(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, unit: UnitId) -> i32 {
        skill_events::monster_skill_start(h, sim, unit)
    }
    fn monster_right_aura(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        unit: UnitId,
        skill: i32,
        level: i32,
    ) {
        skill_events::monster_right_aura(h, sim, unit, skill, level);
    }
    fn monster_passive_refresh(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        unit: UnitId,
        skill: i32,
        level: i32,
    ) {
        skill_events::monster_passive_refresh(h, sim, unit, skill, level);
    }
    fn monster_sequence_frame(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, unit: UnitId) {
        skill_events::monster_sequence_frame(h, sim, unit);
    }
    fn monster_attack_skill(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, unit: UnitId) {
        skill_events::monster_attack_skill(h, sim, unit);
    }
    fn monster_attack_strike(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        unit: UnitId,
        moving: bool,
    ) {
        skill_events::monster_attack_strike(h, sim, unit, moving);
    }
    fn monster_mode_damage(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, unit: UnitId, mode: u32) {
        skill_events::monster_mode_damage(h, sim, unit, mode);
    }
    fn golem_resummon(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, player: UnitId) -> bool {
        skill_events::golem_resummon(h, sim, player)
    }
    fn missile_summon_class(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        owner: UnitId,
        skill: i32,
        level: i32,
    ) -> (i32, i32) {
        skill_events::missile_summon_class(h, sim, owner, skill, level)
    }
    fn skill_stat_refresh(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        unit: UnitId,
        stat: u16,
        layer: u16,
    ) {
        skill_events::skill_stat_refresh(h, sim, unit, stat, layer);
    }
    fn missile_area_units(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        owner: UnitId,
        at: (i32, i32),
        r: i32,
        f: u32,
        noaura: bool,
    ) -> Vec<UnitId> {
        skill_events::missile_area_units(h, sim, owner, at, r, f, noaura)
    }
    fn missile_ally_test(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        a: UnitId,
        b: UnitId,
    ) -> bool {
        skill_events::missile_ally_test(h, sim, a, b)
    }
    fn missile_shout_state(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        unit: UnitId,
        owner: UnitId,
        skill: i32,
        level: i32,
    ) {
        skill_events::missile_shout_state(h, sim, unit, owner, skill, level)
    }
    fn missile_terror(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        source: UnitId,
        unit: UnitId,
        b: i32,
    ) {
        skill_events::missile_terror(h, sim, source, unit, b)
    }
    fn missile_skill_srv_do(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        caster: UnitId,
        index: i32,
        skill: i32,
        level: i32,
    ) {
        skill_events::missile_skill_srv_do(h, sim, caster, index, skill, level)
    }
    fn missile_summon_spawn(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        owner: UnitId,
        class: i32,
        mode: i32,
        at: (i32, i32),
        pet_type: i32,
    ) -> Option<UnitId> {
        skill_events::missile_summon_spawn(h, sim, owner, class, mode, at, pet_type)
    }
    fn missile_bone_wall_piece(
        h: &mut ActionHooks<Self>,
        sim: &mut USim<'_>,
        owner: UnitId,
        anchor: UnitId,
        piece: UnitId,
        skill: i32,
        level: i32,
    ) {
        skill_events::missile_bone_wall_piece(h, sim, owner, anchor, piece, skill, level);
    }
    fn right_aura_select(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, player: UnitId) {
        skill_events::right_aura_select(h, sim, player);
    }
    fn passive_refresh_all(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, unit: UnitId) {
        skill_events::passive_refresh_all(h, sim, unit);
    }
    fn passive_states_on(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, unit: UnitId) {
        skill_events::passive_states_on(h, sim, unit);
    }
    fn assign_right_aura(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, unit: UnitId) {
        skill_events::assign_right_aura(h, sim, unit);
    }
    fn summon_follow(h: &mut ActionHooks<Self>, sim: &mut USim<'_>, unit: UnitId) {
        d2_sim::wiring::interaction::summon::summon_follow(h, sim, unit);
    }
    // D2RUST's own, unverified (q-amazon, REC-150): the hand class, the item
    // shoots / stack facts of the skill bodies ([`crate::weapons`]).
    fn composit_weapon_class(&self, unit: UnitId) -> i32 {
        self.weapons.cof_class(unit)
    }
    fn hand_class(&self, unit: UnitId) -> i32 {
        self.weapons.hand_class(unit)
    }
    fn item_is(&self, item: UnitId, itype: i32) -> bool {
        self.weapons.item_is(item, itype)
    }
    // D2RUST's own, unverified (q-weapon-combat): combat's weapon is the weapon
    // in use; its damage reaches the wearer through the linked stat list.
    fn current_weapon(&self, unit: UnitId) -> Option<UnitId> {
        self.weapons.weapon(unit)
    }
    // Pick the hands' damage weapon for the used skill and current frame event.
    fn damage_weapon(&self, unit: UnitId, weapsel: u8, flag: bool) -> Option<Option<UnitId>> {
        let idx = self.skills.unit(unit).map_or(0, |s| s.event_arg);
        d2_sim::wiring::interaction::UseRest::skill_damage_weapon(self, unit, weapsel, idx, flag)
    }
    fn weapon(&self, unit: UnitId) -> Option<UnitId> {
        self.weapons.weapon(unit)
    }
    // The player's weapon hit class, or 1 without a weapon.
    fn weapon_hit_class(&self, unit: UnitId) -> u32 {
        self.weapons.weapon_hit_class(unit)
    }
    fn item_at(&self, unit: UnitId, loc: u8) -> Option<UnitId> {
        self.weapons.item_at(unit, loc)
    }
    // D2RUST's own, unverified (q-assassin-gaps): a player whose hands the
    // weapon copy knows has an inventory, and a copied item is usable (not
    // broken); the dual-claw test of srvdo 35.
    fn has_inventory(&self, unit: UnitId) -> bool {
        self.weapons.hands.contains_key(&unit)
    }
    fn item_usable(&self, item: UnitId) -> bool {
        self.weapons.items.contains_key(&item)
    }
    // D2RUST's own, unverified (q-skill-gaps): the shield and its smite damage
    // from the weapon copy.
    fn shield(&self, unit: UnitId) -> Option<UnitId> {
        self.weapons.shield(unit)
    }
    fn has_shield(&self, unit: UnitId) -> bool {
        self.weapons.shield(unit).is_some()
    }
    fn shield_damage(&self, item: UnitId) -> Option<(i32, i32)> {
        Some(self.weapons.facts(item).dam)
    }
    fn wield_type(&self, item: UnitId) -> i32 {
        self.weapons.facts(item).grip
    }
    // Monster shield presence for the block-chance check (combat/hit.md section 5).
    fn composit_shield(&self, unit: UnitId) -> bool {
        self.shielded.contains(&unit)
    }
    // The items row's `StrBonus` / `DexBonus` damage bonus.
    fn str_dex_bonus(&self, item: UnitId) -> (i32, i32) {
        let f = self.weapons.facts(item);
        (f.str_bonus, f.dex_bonus)
    }
    // The items-row durability flag. The sim checks max durability and
    // indestructibility separately (`View::item_has_durability`).
    fn item_has_durability(&self, item: UnitId) -> bool {
        self.weapons.facts(item).breakable
    }
    fn item_type_class(&self, item: UnitId) -> u32 {
        self.weapons.type_class(item)
    }
    fn item_shoots(&self, item: UnitId) -> bool {
        self.weapons.facts(item).shoots
    }
    fn item_stackable(&self, item: UnitId) -> bool {
        self.weapons.facts(item).stackable
    }
    fn item_max_stack(&self, item: UnitId) -> i32 {
        self.weapons.facts(item).max_stack
    }
    fn item_missile_type(&self, item: UnitId) -> i32 {
        self.weapons.facts(item).missile_type
    }
    fn anim_name(&self, unit: UnitId, ty: UnitType, class: u32, mode: u32) -> Option<[u8; 8]> {
        let weapon = self.weapons.cof_class(unit);
        self.looks.as_deref()?.anim_key(ty, class, mode, weapon)
    }
    fn anim_rate(&self, _: UnitId, speed: Option<u32>) -> i16 {
        speed.map_or(0, |s| s as i16)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or_default()
    }
    fn place(&mut self, unit: UnitId, x: i32, y: i32) {
        self.pos.insert(unit, (x, y));
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.sent.push((player, msg.to_vec()));
    }
    fn warp(&mut self, _: &mut Game, player: UnitId, level: u32, tile_code: u8) {
        self.log
            .push(format!("warp {} {level} {tile_code}", player.0));
    }
    fn set_player_mode_arrival(&mut self, _: &mut Game, player: UnitId) {
        self.log.push(format!("arrival mode {}", player.0));
    }
    /// D2RUST's own preview hostility: players and allied monsters may attack
    /// the other monsters, never themselves or units without a side.
    fn may_attack(&self, attacker: UnitId, defender: UnitId) -> bool {
        attacker != defender
            && matches!(
                (self.player_side(attacker), self.player_side(defender)),
                (Some(a), Some(d)) if a != d
            )
    }
    /// AI hostility uses the same side test as [`Self::may_attack`].
    fn ai_hostile(&self, _game: &Game, a: UnitId, b: UnitId) -> bool {
        self.may_attack(a, b)
    }
    // Keep the AI request target as the unit's one path target, replacing any
    // target from a previous mode start before later target checks run.
    fn set_mode_target(&mut self, unit: UnitId, target: d2_sim::monsters::ai::ModeTarget) {
        self.monsters.set_target(unit, target);
        if let Some(s) = self.skills.units.get_mut(&unit) {
            s.target = None;
        }
    }
    fn set_current_skill(&mut self, unit: UnitId, skill: i32) -> bool {
        self.monsters.set_current(unit, skill)
    }
    fn clear_current_skill(&mut self, unit: UnitId) {
        self.monsters.clear_current(unit);
    }
    fn class_has_mode(&self, class: i32, mode: u8) -> bool {
        self.monsters.class_has_mode(class, mode)
    }
    fn used_skill(&self, unit: UnitId) -> Option<d2_sim::skills::SkillEntry> {
        self.monsters.used_skill(unit)
    }
    // D2RUST's own, unverified (preview; q-a2-charge-jab): a monster has no
    // skill list, so its used entry's flags (Charge's moving flag) are kept per
    // unit in the skill store, and its entries' params (Charge's target and hit
    // mode, Armageddon's reseed value) per unit and entry.
    fn entry_param(&self, unit: UnitId, e: &d2_sim::skills::SkillEntry, i: u8) -> i32 {
        self.unit_entry_param(unit, e, i)
    }
    fn set_entry_param_of(&mut self, unit: UnitId, e: &d2_sim::skills::SkillEntry, i: u8, v: i32) {
        self.set_unit_entry_param(unit, e, i, v);
    }
    fn entry_flags(&self, unit: UnitId, _: &d2_sim::skills::SkillEntry) -> u32 {
        d2_sim::wiring::interaction::UseRest::used_skill_flags(self, unit)
    }
    fn set_entry_flags(&mut self, unit: UnitId, _: &d2_sim::skills::SkillEntry, f: u32) {
        d2_sim::wiring::interaction::UseRest::set_used_skill_flags(self, unit, f);
    }
    /// D2RUST's own preview melee reach, including the player's weapon range.
    fn melee_range(&self, u: UnitId) -> i32 {
        self.reach(u)
    }
    /// D2RUST's own, unverified (preview, D1; the melee range rule
    /// with the preview reach and no line test): the larger axis
    /// distance of the synced positions within reach + `extra` + 1.
    fn in_melee_range(&self, a: UnitId, d: UnitId, extra: i32) -> bool {
        let (Some(&(_, _, pa)), Some(&(_, _, pd))) = (self.sides.get(&a), self.sides.get(&d))
        else {
            return false;
        };
        if matches!(self.sides.get(&a), Some(&(UnitType::Player, ..))) {
            // For players, compare size-adjusted unit distance against reach + extra + 1.
            // For example, two size-2 units three sub-tiles apart have distance zero.
            static TABLES: std::sync::OnceLock<Option<d2_sim::path::tables::PathTables>> =
                std::sync::OnceLock::new();
            if let Some(t) = TABLES.get_or_init(|| d2_sim::path::tables::PathTables::spec().ok()) {
                let size = |u| self.sizes.get(&u).copied().unwrap_or(2);
                let d = d2_sim::path::walk::geom::unit_distance(
                    t,
                    d2_sim::path::Point::new(pa.0, pa.1),
                    size(a),
                    d2_sim::path::Point::new(pd.0, pd.1),
                    size(d),
                );
                return d <= 0 || self.reach(a).wrapping_add(extra).wrapping_add(1) >= d;
            }
        }
        let dist = (pa.0 - pd.0).abs().max((pa.1 - pd.1).abs());
        dist <= PREVIEW_MELEE_RANGE + extra + 1
    }
    /// D2RUST's own preview alignment: players and allied monsters are good (2);
    /// other units are evil (0).
    fn alignment(&self, unit: UnitId) -> u8 {
        if self.player_side(unit) == Some(true) {
            2
        } else {
            0
        }
    }
    /// One target list per player (the player alone; the units attached
    /// to it are added by the sim), in unit-list order, for every player
    /// in the game.
    fn target_nodes(&self, game: &Game) -> d2_sim::game::TargetLists {
        d2_sim::game::TargetLists {
            players: game
                .lists
                .units_of_type(UnitType::Player)
                .into_iter()
                .map(|p| vec![p])
                .collect(),
            ..Default::default()
        }
    }
    /// The operate entry's interact-range fallback. The path provider handles
    /// the walk; this seam accepts the client's C-to-S 0x13 request.
    fn object_in_range(&self, _: &Game, _: UnitId, _: UnitId) -> bool {
        true
    }
    /// D2RUST's own, unverified: the preview has no quest records, so every
    /// player has one and the portal's quest gate is the level's own
    /// `leveldefs` flag.
    fn object_quest_record(&self, _: UnitId) -> bool {
        true
    }
    // The object case uses interact range and the server's walk path provider.
}

impl WorldPending for LocalSeams {
    /// The quest control's published answer for population preset swaps.
    fn quest_flag(&self, flag: u8) -> bool {
        Pending::quest_not_intro(self, flag)
    }
    /// Queue the special monster creation hook for the quest control when it
    /// is not lent to this tick. A lent control handles it inside creation
    /// (`QuestObjectHost::boss_created`).
    fn boss_quest_hook(&mut self, boss: UnitId) {
        self.quest_events
            .push(d2_sim::wiring::action::QuestEvent::BossCreated { unit: boss });
    }
    /// Queue the preset boss event for the quest control.
    fn quest_preset_boss(&mut self, unit: UnitId) {
        self.quest_events
            .push(d2_sim::wiring::action::QuestEvent::PresetBoss { unit });
    }
}

impl Outbox for LocalSeams {
    fn take_sent(&mut self) -> Vec<(UnitId, Vec<u8>)> {
        std::mem::take(&mut self.sent)
    }
}

/// Animation names and shield choices from the game's loaded unit tables.
pub trait UnitAppearance: std::fmt::Debug {
    fn anim_key(&self, ty: UnitType, class: u32, mode: u32, weapon: i32) -> Option<[u8; 8]>;
    fn shield_choices(&self, class: u32) -> Option<u16>;
}

/// Duriel's Lair (`levels` row 73, act 1).
pub const DURIELS_LAIR: u32 = 73;
/// Durance of Hate levels 1 and 2.
pub const DURANCE_1: u32 = 100;
pub const DURANCE_2: u32 = 101;

#[cfg(test)]
mod player_melee_range_tests {
    use super::*;
    use crate::weapons::{Hands, ItemFacts};

    /// A player at (100, 100) with a weapon of `rangeadder` `adder` and a
    /// hostile monster at (x, 100); both size 2 (the default).
    fn seams(x: i32, adder: i32) -> LocalSeams {
        let mut s = LocalSeams::default();
        s.sides
            .insert(UnitId(1), (UnitType::Player, true, (100, 100)));
        s.sides
            .insert(UnitId(2), (UnitType::Monster, false, (x, 100)));
        let item = UnitId(9);
        s.weapons.items.insert(
            item,
            ItemFacts {
                class: 2,
                range_adder: adder,
                ..ItemFacts::default()
            },
        );
        s.weapons.hands.insert(
            UnitId(1),
            Hands {
                right: Some(item),
                weapon: Some(item),
                ..Hands::default()
            },
        );
        s
    }

    // Covers size-adjusted melee distance and the player's weapon range adder.
    #[test]
    fn a_player_reaches_by_unit_distance_and_rangeadder() {
        // An axe (adder 0): 3 sub-tiles apart is distance 0, in reach.
        assert!(seams(103, 0).in_melee_range(UnitId(1), UnitId(2), 0));
        // 4 sub-tiles is distance 2: out of reach for adder 0, in for 1.
        assert!(!seams(104, 0).in_melee_range(UnitId(1), UnitId(2), 0));
        assert!(seams(104, 1).in_melee_range(UnitId(1), UnitId(2), 0));
        // Far away is out whatever the adder.
        assert!(!seams(120, 1).in_melee_range(UnitId(1), UnitId(2), 0));
        // No weapon: adder 0.
        let mut s = seams(104, 1);
        s.weapons = Default::default();
        assert!(!s.in_melee_range(UnitId(1), UnitId(2), 0));
    }
}
