//! Monster init on the world wiring: [`InitHost`] on [`WorldHost`]. Unit
//! records and base stats (layer 0) are the action systems'
//! ([`crate::units::record::Units`], [`crate::stats::StatLists`]); AI
//! control and the first AI setup are `monsters::ai`'s
//! ([`crate::monsters::ai::install`]); the level id is the unit's room's
//! level; region appearance entries and the boss count are the game's
//! population regions; minion lists are [`super::WorldState::minions`];
//! montype nesting is the montype equivalence matrix
//! ([`super::WorldTables::montype_equiv`]); the rules (a mod game's umods,
//! `docs/plans/mods/seams.md` MS5) are the game's.
//!
//! Creation seams (`place`, `allocate`, `boss_spawn`, `spawn_boss_minion`,
//! `spawn_with_guid`, `party_minions`, `register_spawn`) keep their
//! defaults: in this wiring every monster is created by population's
//! creation sequence through [`super::population_init`], and the init
//! entry points that would create (`create`, `random_boss`, the full
//! `superunique_init`, the restore paths) are not called. The items,
//! skills, quests and states calls keep their defaults too (other
//! systems answer them), as does `set_combat_mode` (the mode it would set
//! for a dead monster is not defined). `has_inventory` and `has_item_at`
//! keep theirs with `new_inventory`: monsters have no inventory in this
//! wiring, so "no inventory" is what the wired world holds.

use crate::game::Game;
use crate::monsters::ai::{self, AiControl};
use crate::monsters::init::{self, InitHost, MonsterStore};
use crate::monsters::population::OwnerKey;
use crate::units::hooks::Sim;
use crate::units::modes;
use crate::units::record::Units;
use crate::units::UnitId;

use super::{View, WiringError, WorldHost, WorldPending, WorldgenError};

impl<X: WorldPending> WorldHost<'_, X> {
    /// The level id of the unit's room: during the type init the
    /// allocation's room, the unit being in no
    /// room until it is added.
    fn unit_room_level(&self, unit: UnitId) -> i32 {
        self.v
            .init_room(self.game, unit)
            .map_or(0, |r| self.room_level_id(r))
    }

    /// A host-placed monster's creation-time AI: control block, install
    /// state 0, and the first think of a created and placed monster, 2 base
    /// ticks after creation.
    pub(super) fn start_host_ai(&mut self, unit: UnitId) {
        InitHost::alloc_ai(self, unit);
        InitHost::ai_install(self, unit, 0);
        let at = self.game.after(2);
        let _ = self.game.schedule_event(
            unit,
            u32::from(crate::tick::events::event::AI_THINK),
            at,
            None,
            0,
            0,
        );
    }

    /// The region appearance entry for the unit's class in its
    /// room's region: the entry, added on demand on the unit seed.
    fn region_entry(&mut self, unit: UnitId, class: u32) -> Option<(i32, usize)> {
        let level = self.unit_room_level(unit);
        let t = self.w.tables.clone();
        self.regions("region entry")?;
        let seed = &mut self.v.units.get_mut(unit)?.seed;
        let i = self
            .w
            .pop
            .regions
            .entry_for(&t.pop, level, class as i32, seed)?;
        Some((level, i))
    }
}

impl<X: WorldPending> InitHost for WorldHost<'_, X> {
    fn game(&mut self) -> &mut Game {
        self.game
    }

    fn units(&mut self) -> &mut Units {
        self.v.units
    }

    fn monsters(&mut self) -> &mut MonsterStore {
        &mut self.w.monsters
    }

    fn info(&self) -> init::GameInfo {
        self.w.init_info
    }

    fn rules(&self) -> &crate::rules::Ruleset {
        &self.v.h.rules
    }

    /// Writes the game's difficulty (the type init reads it for difficulty 2).
    fn set_difficulty(&mut self, d: u8) {
        self.w.init_info.difficulty = d;
        self.w.pop_info.difficulty = d;
        self.v.h.ai_info.difficulty = d;
    }

    /// `StatLists::unit_base`, layer 0.
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.v.stats.unit_base(unit, stat, 0)
    }

    /// Max life: `StatLists::max_life` (stat 7 total).
    fn max_life(&self, unit: UnitId) -> i32 {
        self.v.stats.max_life(unit)
    }

    /// `StatLists::unit_set`, layer 0 (the unit's value callback).
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.v.set_base(unit, stat, value);
    }

    /// The AI control block (`AiStore`), allocated in the type init.
    fn alloc_ai(&mut self, unit: UnitId) {
        match self.v.h.ai.as_mut() {
            Some(ai) => ai.entry(unit).control = Some(AiControl::default()),
            None => self.w.errors.push(WorldgenError::AiLent),
        }
    }

    /// Installs the AI through the AI module's context.
    fn ai_install(&mut self, unit: UnitId, state: u32) {
        let Some(mut store) = self.v.h.ai.take() else {
            self.w.errors.push(WorldgenError::AiLent);
            return;
        };
        let t = self.v.h.tables.clone();
        let info = self.v.h.ai_info;
        {
            let mut v = View::of(
                &mut *self.v.units,
                &mut *self.v.stats,
                self.v.data,
                &mut *self.v.h,
            );
            let mut cx = ai::Ctx {
                tables: ai::AiTables {
                    monstats: &t.combat.monstats,
                    monstats2: &t.combat.monstats2,
                    levels: &t.levels,
                    skill_modes: &t.skill_modes,
                    skills: &t.skills.skills,
                    missiles: &t.skills.missiles,
                },
                info,
                store: &mut store,
                world: &mut v,
            };
            ai::install(self.game, &mut cx, unit, state);
        }
        self.v.h.ai = Some(store);
    }

    /// The level id of the unit's room.
    fn level_id(&mut self, unit: UnitId) -> i32 {
        self.unit_room_level(unit)
    }

    /// The variant count of the class's region entry.
    fn region_variant_count(&mut self, unit: UnitId, class: u32) -> u8 {
        let Some((level, i)) = self.region_entry(unit, class) else {
            return 0;
        };
        self.w
            .pop
            .regions
            .get(level)
            .and_then(|r| r.entries.get(i))
            .map_or(0, |e| e.variant_count)
    }

    /// The 16 component bytes of variant `i`.
    fn region_variant(&mut self, unit: UnitId, class: u32, i: u32) -> [u8; 16] {
        let Some((level, e)) = self.region_entry(unit, class) else {
            return [0; 16];
        };
        self.w
            .pop
            .regions
            .get(level)
            .and_then(|r| r.entries.get(e))
            .and_then(|e| e.variants.get(i as usize))
            .copied()
            .unwrap_or([0; 16])
    }

    /// Sets the unit's alignment: its state-105 list
    /// ([`crate::wiring::action::View::set_alignment`]),
    /// then the host's copy.
    fn set_alignment(&mut self, unit: UnitId, alignment: u8) {
        self.v.set_alignment(self.game, unit, alignment);
        self.v.h.x.set_alignment(unit, alignment);
    }

    /// The region's bosses-spawned count += 1 (the region caps how many
    /// bosses it spawns).
    fn count_region_boss(&mut self, unit: UnitId) {
        let level = self.w.monsters.get(unit).map_or(0, |m| m.level_id);
        if let Some(r) = self
            .regions("boss count")
            .and_then(|regions| regions.get_mut(level))
        {
            r.bosses = r.bosses.wrapping_add(1);
        }
    }

    /// The boss quest hook inside the creation.
    fn boss_quest_hook(&mut self, unit: UnitId) {
        if let Some(mut host) = self.v.h.quest_host.take() {
            let done = host.boss_created(&mut *self.game, &mut self.v, unit);
            self.v.h.quest_host = Some(host);
            if done {
                return;
            }
        }
        self.v.h.x.boss_quest_hook(unit);
    }

    /// The quest chain hook inside monster init.
    fn quest_chain(&mut self, unit: UnitId, chain: u32) {
        self.v.quest_link_now(&mut *self.game, unit, chain);
    }

    /// A link for the quest chain in the level
    /// row's `Quest` column (Den of Evil 1, Arcane Sanctuary 11).
    fn attach_quest_chain(&mut self, unit: UnitId) {
        let level = self.unit_room_level(unit);
        let quest = self.w.tables.pop.level(level).map_or(0, |l| l.quest);
        if quest != 0 {
            self.v
                .quest_link_now(&mut *self.game, unit, u32::from(quest));
        }
    }

    /// The quest preset boss hook inside the creation.
    fn quest_preset_boss(&mut self, unit: UnitId) {
        if let Some(mut host) = self.v.h.quest_host.take() {
            let done = host.preset_boss(&mut *self.game, &mut self.v, unit);
            self.v.h.quest_host = Some(host);
            if done {
                return;
            }
        }
        self.v.h.x.quest_preset_boss(unit);
    }

    /// The boss's owner data `(game, boss, boss GUID, 1, 1, 0)`: the boss owns itself.
    fn boss_owner_data(&mut self, unit: UnitId) {
        self.v
            .h
            .x
            .set_owner_data(unit, OwnerKey::Guid(unit), 1, 1, 0);
    }

    /// Links a minion: the leader's minion list and the minion's leash
    /// owner (the owner data is written by population).
    fn link_minion(&mut self, boss: UnitId, minion: UnitId) {
        self.v.h.x.unique_minion_owner_data(boss, minion);
        self.w.minions.entry(boss).or_default().push(minion);
        self.w.owners.insert(minion, boss);
    }

    /// Type
    /// `montype` is `ty` or nested in it, bit (row `montype`, column `ty`)
    /// of the montype matrix itself (out-of-range rows and columns are 0).
    /// The eligibility test asks it with row = the exclude type and
    /// column = the class's MonType.
    fn montype_is(&mut self, montype: u16, ty: u16) -> bool {
        self.w
            .tables
            .montype_equiv
            .get(usize::from(montype), usize::from(ty))
    }

    /// The boss's minions.
    fn minions(&mut self, boss: UnitId) -> Vec<UnitId> {
        self.w.minions.get(&boss).cloned().unwrap_or_default()
    }

    /// Prepares the animation after the
    /// type init.
    fn after_type_init(&mut self, unit: UnitId) {
        let mut sim = Sim {
            game: &mut *self.game,
            units: &mut *self.v.units,
            stats: &mut *self.v.stats,
            data: self.v.data,
        };
        if let Err(e) = modes::prepare_animation(&mut sim, &mut *self.v.h, unit) {
            self.w
                .errors
                .push(WorldgenError::Wiring(WiringError::Unit(e)));
        }
    }

    /// The monster base list, the only list with
    /// flag 1, allocated
    /// and attached to the unit. The mode
    /// damage and the umod
    /// callbacks write into it.
    ///
    /// The list is allocated with flags 1, expire 0, owner type 1 and the
    /// monster's GUID, attached with reset 1. A DYNAMIC
    /// list would keep `mindamage` / `maxdamage` / `tohit` out of the
    /// unit's totals, and monsters hit with them.
    fn post_extra_list(&mut self, unit: UnitId) {
        let Some((ty, guid)) = self.v.units.get(unit).map(|r| (r.ty, r.guid)) else {
            return;
        };
        let l = self.v.stats.alloc(1, 0, ty.index() as u32, guid);
        self.v.stats.attach(&mut *self.v.h, unit, l, true);
    }

    /// State toggle.
    fn set_state(&mut self, unit: UnitId, state: u16) {
        self.v.set_state(unit, state, true);
    }

    /// Bloodraven: state 118 corpse_noselect on
    /// (her corpse cannot be selected; the client sees it in the 0xAA state
    /// message).
    fn set_corpse_noselect(&mut self, unit: UnitId) {
        self.v.set_state(unit, 118, true);
    }

    // ---- umod callbacks (bodies in `umod_host.rs`)

    fn stat_total(&self, unit: UnitId, stat: u16) -> i32 {
        self.v.stat(unit, stat)
    }
    fn set_base_list_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.umod_set_base_list(unit, stat, value);
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.v.h.path_position(unit)
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.v.stats.has_state(unit, u32::from(state))
    }
    fn has_state_in_group(&self, unit: UnitId, group: u8) -> bool {
        self.v.stats.has_group(unit, usize::from(group))
    }
    fn set_mode(&mut self, unit: UnitId, mode: u32) {
        self.umod_set_mode(unit, mode);
    }
    fn owner(&self, unit: UnitId) -> Option<UnitId> {
        self.umod_owner(unit)
    }
    /// The wired world's minion owner link ([`super::WorldState::owners`]).
    fn minion_owner(&mut self, unit: UnitId) -> Option<UnitId> {
        self.w.owners.get(&unit).copied()
    }
    fn alignment(&self, unit: UnitId) -> u8 {
        self.v.h.x.alignment(unit)
    }
    fn hostile(&self, a: UnitId, b: UnitId) -> bool {
        self.v.h.x.may_attack(a, b)
    }
    fn target(&self, unit: UnitId) -> Option<UnitId> {
        self.umod_path_target(unit)
            .or_else(|| self.v.h.x.umod_target(unit))
    }
    // The host's answer,
    // else the path's target unit position or its target point; both
    // coordinates must be non-zero (the umod 7 curse needs a real target).
    fn target_position(&self, unit: UnitId) -> Option<(i32, i32)> {
        if let Some(p) = self.v.h.x.umod_target_position(unit) {
            return Some(p);
        }
        let (x, y) = match self.umod_path_target(unit) {
            Some(t) => self.v.h.path_position(t),
            None => {
                let d = self.v.h.paths.as_ref()?.dynamic(unit)?;
                (i32::from(d.target_x), i32::from(d.target_y))
            }
        };
        (x != 0 && y != 0).then_some((x, y))
    }
    fn path_target_point(&self, missile: UnitId) -> (i32, i32) {
        self.v
            .path_target_xy(missile)
            .unwrap_or_else(|| self.v.h.x.path_target_point(missile))
    }
    fn create_missile(
        &mut self,
        req: crate::skills::use_::bodies::MissileRequest<UnitId>,
    ) -> Option<UnitId> {
        self.umod_create_missile(req)
    }
    fn missile_row_flags(&self, class: i32) -> Option<u32> {
        self.umod_missile_flags(class)
    }
    fn missile_velocity(&self, class: i32, level: i32) -> i32 {
        self.umod_missile_velocity(class, level)
    }
    fn missile_skill_level(&self, missile: UnitId) -> (i32, i32) {
        self.umod_missile_skill_level(missile)
    }
    fn find_units(&mut self, near: UnitId, q: init::find::FindQuery) -> Vec<UnitId> {
        self.umod_find_units(near, q)
    }
    fn line_clear(&mut self, from: (i32, i32), unit: UnitId) -> bool {
        self.umod_line_clear(from, unit)
    }
    fn missile_hit(&mut self, src: UnitId, unit: UnitId, rec: &crate::combat::DamageRecord) {
        self.umod_missile_hit(src, unit, rec);
    }
    fn skill_calc(
        &mut self,
        unit: UnitId,
        skill: u16,
        calc: init::callbacks::SkillCalc,
        level: i32,
    ) -> i32 {
        self.umod_skill_calc(unit, skill, calc, level)
    }
    fn aura_fields(&self, skill: u16) -> Option<init::callbacks::AuraFields> {
        self.umod_aura_fields(skill)
    }
    fn stat_and_state_counts(&self) -> (i32, i32) {
        self.umod_counts()
    }
    // The skill pipeline's `apply_state` when the host provides it.
    fn apply_state(&mut self, req: init::callbacks::StateApply) -> Option<crate::stats::ListId> {
        if let Some(l) = self.v.h.x.umod_apply_state(req) {
            return Some(l);
        }
        let q = crate::skills::use_::bodies::helpers::StateRequest {
            source: req.source,
            target: req.target,
            skill: req.skill,
            level: req.level,
            duration: req.duration,
            stat: req.stat,
            value: req.value,
            state: req.state,
            callback: req.callback,
        };
        let mut sim = crate::units::hooks::Sim {
            game: &mut *self.game,
            units: &mut *self.v.units,
            stats: &mut *self.v.stats,
            data: self.v.data,
        };
        X::apply_state_req(&mut *self.v.h, &mut sim, q)
    }
    fn set_list_stat(&mut self, list: crate::stats::ListId, stat: u16, value: i32) {
        self.v.set_list_stat(list, stat, value);
    }
    /// Frees the unit's minion list in the wired world.
    fn free_minions(&mut self, unit: UnitId) {
        self.w.minions.remove(&unit);
    }
    fn clear_owner_data(&mut self, unit: UnitId) {
        self.umod_clear_owner_data(unit);
    }
    fn remove_pet(&mut self, owner: UnitId, pet: UnitId) {
        self.v.h.x.remove_pet(owner, pet);
    }
    fn is_undead(&self, unit: UnitId) -> bool {
        self.v.h.x.umod_is_undead(unit)
    }
    fn missile_range(&self, class: i32) -> Option<i32> {
        self.v.h.x.umod_missile_range(class)
    }
    fn set_uber_death(&mut self, slot: usize) -> bool {
        self.v.h.x.umod_set_uber_death(slot)
    }
    fn game_8c(&self) -> i32 {
        self.v.h.x.umod_game_8c()
    }
    /// A umod quest drop: the drop helper with the item code `code`
    /// and quality `arg` ([`crate::wiring::economy::unit_quest_drop`]) when the game holds
    /// the drop state; with `announce`, the recharge message on the
    /// item ([`WorldPending::umod_recharge`]). Without the drop state:
    /// [`WorldPending::umod_quest_drop`].
    fn quest_drop(&mut self, unit: UnitId, code: [u8; 4], arg: i32, announce: bool) {
        let Some(mut d) = self.v.h.object_drops.take() else {
            self.v.h.x.umod_quest_drop(unit, code, arg, announce);
            return;
        };
        let item = {
            let mut sim = Sim {
                game: &mut *self.game,
                units: &mut *self.v.units,
                stats: &mut *self.v.stats,
                data: self.v.data,
            };
            crate::wiring::economy::unit_quest_drop(
                &mut *self.v.h,
                &mut sim,
                &mut d,
                &mut crate::wiring::economy::NoSpot,
                unit,
                Some(code),
                arg as u8,
                -1,
                0,
            )
        };
        self.v.h.object_drops = Some(d);
        if let (true, Some(item)) = (announce, item) {
            self.v.h.x.umod_recharge(item);
        }
    }
    /// The monster's inventory is a
    /// holdings entry ([`ActionHooks::monster_equip`]); the NPC store
    /// inventory is the vendors'.
    ///
    /// [`ActionHooks::monster_equip`]: crate::wiring::action::ActionHooks
    fn new_inventory(&mut self, unit: UnitId, npc_store: bool) {
        if !npc_store {
            self.v.h.monster_equip.entry(unit).or_default();
        }
    }
    /// The monster has an inventory.
    fn has_inventory(&mut self, unit: UnitId) -> bool {
        self.v.h.monster_equip.contains_key(&unit)
    }
    /// The monster's inventory holds an item at `loc`
    /// ([`ActionHooks::monster_equip`], the holdings).
    ///
    /// [`ActionHooks::monster_equip`]: crate::wiring::action::ActionHooks
    fn has_item_at(&mut self, unit: UnitId, loc: u8) -> bool {
        let sim = Sim {
            game: &mut *self.game,
            units: &mut *self.v.units,
            stats: &mut *self.v.stats,
            data: self.v.data,
        };
        crate::wiring::economy::monster_item_at(&*self.v.h, &sim, unit, loc).is_some()
    }
    /// Creates a monster's equipment item when the game holds the drop state
    /// (the item tables); without it no item is made.
    fn create_equip_item(
        &mut self,
        unit: UnitId,
        code: [u8; 4],
        loc: u8,
        modifier: u8,
        level: i32,
    ) {
        let Some(mut d) = self.v.h.object_drops.take() else {
            return;
        };
        {
            let mut sim = Sim {
                game: &mut *self.game,
                units: &mut *self.v.units,
                stats: &mut *self.v.stats,
                data: self.v.data,
            };
            crate::wiring::economy::create_monster_equip(
                &mut *self.v.h,
                &mut sim,
                &mut d,
                unit,
                code,
                loc,
                modifier,
                level,
            );
        }
        self.v.h.object_drops = Some(d);
    }
    /// The monprop property assignment on the
    /// unit ([`crate::wiring::economy::monster_property`]); without the
    /// drop state (no item tables) nothing is written.
    fn apply_property(&mut self, unit: UnitId, prop: i32, par: i32, min: i32, max: i32) {
        let Some(d) = self.v.h.object_drops.take() else {
            return;
        };
        {
            let mut sim = Sim {
                game: &mut *self.game,
                units: &mut *self.v.units,
                stats: &mut *self.v.stats,
                data: self.v.data,
            };
            crate::wiring::economy::monster_property(
                &mut *self.v.h,
                &mut sim,
                &d,
                unit,
                prop,
                par,
                min,
                max,
            );
        }
        self.v.h.object_drops = Some(d);
    }
    /// The item tier code for the difficulty
    /// ([`crate::wiring::economy::tier_code`]); without
    fn item_tier_code(&mut self, code: [u8; 4], difficulty: u8) -> [u8; 4] {
        match self.v.h.object_drops.as_ref() {
            Some(d) => {
                crate::wiring::economy::tier_code(&d.tables.treasure_items, code, difficulty)
            }
            None => code,
        }
    }
    /// A boss item: `(game, unit, &entry, level, 4)`, the
    /// monequip helper with quality 4 (magic).
    fn create_boss_item(&mut self, unit: UnitId, code: [u8; 4], loc: u8, level: i32) {
        self.create_equip_item(unit, code, loc, 4, level);
    }
    fn steal_belt_item(&mut self, unit: UnitId, target: UnitId) {
        self.v.h.x.steal_belt_item(unit, target);
    }
    fn spawn_near(&mut self, unit: UnitId, class: u32, mode: u32, spread: i32, flags: u32) {
        self.v.h.x.spawn_near(unit, class, mode, spread, flags);
    }
    fn footprint_occupied(&mut self, unit: UnitId) -> bool {
        self.umod_footprint_occupied(unit)
    }
    /// A boss aura modifier: give and assign the aura skill (the monster's
    /// right skill).
    fn give_aura(&mut self, unit: UnitId, skill: u16, level: i32) {
        let mut sim = Sim {
            game: &mut *self.game,
            units: &mut *self.v.units,
            stats: &mut *self.v.stats,
            data: self.v.data,
        };
        X::monster_right_aura(&mut *self.v.h, &mut sim, unit, i32::from(skill), level);
    }
    /// AI control flags |= `flag` (a umod's AI behaviour bit); a unit
    /// without an AI control writes nothing.
    fn set_ai_flag(&mut self, unit: UnitId, flag: u16) {
        if let Some(c) = self.v.h.ai.as_mut().and_then(|ai| ai.control_mut(unit)) {
            c.flags |= flag;
        }
    }
    fn ai_param0(&mut self, unit: UnitId) -> i32 {
        self.v.h.x.ai_param0(unit)
    }
    fn set_ai_param0(&mut self, unit: UnitId, v: i32) {
        self.v.h.x.set_ai_param0(unit, v);
    }
    fn can_raise(&mut self, unit: UnitId) -> bool {
        self.v.h.x.can_raise(unit)
    }
    fn ai_use_skill(&mut self, unit: UnitId, mode: u32, skill: u16) {
        self.v.h.x.ai_use_skill(unit, mode, skill);
    }
    fn give_skill(&mut self, unit: UnitId, skill: u16, level: i32, _mode: Option<u8>) {
        self.v
            .h
            .natural_skills
            .entry(unit)
            .or_default()
            .insert(i32::from(skill), level);
    }
    fn skill_level(&mut self, unit: UnitId, skill: u16) -> Option<i32> {
        self.v.h.x.skill_level(unit, skill)
    }
    /// The life percent as the AI reads it ([`crate::monsters::ai::AiUnits`]).
    fn life_percent(&self, unit: UnitId) -> i32 {
        crate::monsters::ai::AiUnits::life_percent(&self.v, unit)
    }
    fn room_area_level(&mut self, unit: UnitId) -> Option<i32> {
        self.umod_room_area_level(unit)
    }
}
