//! [`InitHost`]: the state monster init works on and every call into
//! another system. Defaults are the narrowest reading: nothing happens,
//! or the value that makes init do nothing. The system answering each
//! method is named in its doc. Spawning seams re-enter init (a spawned
//! monster runs [`super::create`] / [`super::type_init`] through the
//! host), so the host owns all state and init reaches it through `&mut
//! self`. [`InitHost`] is object-safe: a mod game's umods
//! (`docs/plans/mods/seams.md` MS5) run on `dyn InitHost`.

use crate::game::Game;
use crate::units::record::Units;
use crate::units::UnitId;

use crate::combat::DamageRecord;
use crate::skills::use_::bodies::MissileRequest;
use crate::stats::lists::ListId;

use super::callbacks::{AuraFields, SkillCalc, StateApply};
use super::find::FindQuery;
use super::{CreateRequest, GameInfo, MonsterStore};

#[allow(unused_variables)]
pub trait InitHost: AsDynInit {
    // ---- state ----

    fn game(&mut self) -> &mut Game;
    /// Unit records.
    fn units(&mut self) -> &mut Units;
    /// Monster data blocks.
    fn monsters(&mut self) -> &mut MonsterStore;
    /// Game settings at the time of the call.
    fn info(&self) -> GameInfo;
    /// The rules the game runs under: a mod game's players-X formula and
    /// umods come from them. Default: D2RUST's own.
    fn rules(&self) -> &crate::rules::Ruleset {
        &crate::rules::ORIGINAL
    }
    /// The difficulty check writes difficulty 2 into the game when it finds
    /// a difficulty of 3 or more.
    fn set_difficulty(&mut self, d: u8) {}

    // ---- stats (provider: the stat lists, `StatLists::unit_base`
    // / `unit_set`, layer 0) ----

    /// The unit's base stat (layer 0).
    fn stat(&self, unit: UnitId, stat: u16) -> i32;
    /// Sets the unit's base stat (layer 0) through the value callback.
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32);
    /// The unit's maximum life, the total of stat 7 with its
    /// items and modifiers (umod 2 reads it). Default: the base stat.
    fn max_life(&self, unit: UnitId) -> i32 {
        self.stat(unit, crate::stats::stat::MAXHP)
    }

    // ---- type init ----

    /// Allocate AI control, AI params and the interaction
    /// block. Provider: AI
    /// system (`AiStore::entry(unit).control = Some(..)`).
    fn alloc_ai(&mut self, unit: UnitId) {}
    /// (`monsters::ai::install`).
    fn ai_install(&mut self, unit: UnitId, state: u32) {}
    /// The level id of the unit's room (the area-level lookup reads it).
    /// Provider: DRLG.
    fn level_id(&mut self, unit: UnitId) -> i32 {
        0
    }
    /// The variant count of the class's region
    /// data, 0 when it has none.
    fn region_variant_count(&mut self, unit: UnitId, class: u32) -> u8 {
        0
    }
    /// The 16 component bytes of variant `i`.
    fn region_variant(&mut self, unit: UnitId, class: u32, i: u32) -> [u8; 16] {
        [0; 16]
    }
    /// The quest hook on a new unit (quests).
    fn attach_quest_chain(&mut self, unit: UnitId) {}
    /// Combat mode (units).
    fn set_combat_mode(&mut self, unit: UnitId) {}
    fn after_type_init(&mut self, unit: UnitId) {}
    /// Post an extra stat list (a new list, then attached to the
    /// unit).
    fn post_extra_list(&mut self, unit: UnitId) {}
    /// `npc_store` → NPC store inventory, else a
    /// new inventory. Provider: items / vendors.
    fn new_inventory(&mut self, unit: UnitId, npc_store: bool) {}
    /// The unit has an inventory.
    fn has_inventory(&mut self, unit: UnitId) -> bool {
        false
    }
    /// Give `skill` at `level`, then its mode when given
    /// (skills).
    fn give_skill(&mut self, unit: UnitId, skill: u16, level: i32, mode: Option<u8>) {}
    /// Item property assignment on the unit (property
    /// system).
    fn apply_property(&mut self, unit: UnitId, prop: i32, par: i32, min: i32, max: i32) {}
    /// The inventory holds an item at body location `loc`.
    fn has_item_at(&mut self, unit: UnitId, loc: u8) -> bool {
        false
    }
    /// The base code of the owner's item
    /// at body location `loc`; `None` when it has none.
    fn owner_item_code(&mut self, owner: UnitId, loc: u8) -> Option<[u8; 4]> {
        None
    }
    /// Create item `code` at `loc` with modifier `modifier`, item
    /// level `level` (treasure; its draws use the unit
    /// seed).
    fn create_equip_item(
        &mut self,
        unit: UnitId,
        code: [u8; 4],
        loc: u8,
        modifier: u8,
        level: i32,
    ) {
    }

    // ---- creation sequence ----

    /// Placement (active room seed draws): the
    /// accepted point, `None` when there is none.
    fn place(&mut self, req: &CreateRequest) -> Option<(i32, i32)> {
        None
    }
    /// The unit allocation (`units::lifecycle::allocate`), whose
    /// `init_kind` must run [`super::type_init`]. `None` when refused.
    fn allocate(&mut self, req: &CreateRequest, x: i32, y: i32) -> Option<UnitId> {
        None
    }
    /// Region count (nc = `never_count`) and the
    /// coord list.
    fn register_spawn(&mut self, unit: UnitId, req: &CreateRequest, never_count: u32) {}
    /// Alignment value (0, 1 or 2; units).
    fn set_alignment(&mut self, unit: UnitId, alignment: u8) {}
    /// Party minions.
    fn party_minions(&mut self, unit: UnitId, req: &CreateRequest) {}

    // ---- bosses ----

    /// Quest chain record (quests).
    fn quest_chain(&mut self, unit: UnitId, chain: u32) {}
    /// Bloodraven: state 118 corpse_noselect on.
    fn set_corpse_noselect(&mut self, unit: UnitId) {}
    /// Ancient barbarian equipment: difficulty 1 replaces `code`
    /// by its items row's `ubercode`, 2 by `ultracode`
    /// (row found by code). Default: unchanged.
    fn item_tier_code(&mut self, code: [u8; 4], difficulty: u8) -> [u8; 4] {
        code
    }
    /// Create `code`
    /// as a magic item (spawn mode 4) at item level `level` and equip it
    /// at body location `loc` (game- and
    /// item-seed draws). Default: nothing.
    fn create_boss_item(&mut self, unit: UnitId, code: [u8; 4], loc: u8, level: i32) {}
    /// Boss steps 1–4: placement and
    /// creation of a boss. Step 5 is [`super::mark_boss`].
    fn boss_spawn(
        &mut self,
        req: &CreateRequest,
        guid: Option<u32>,
        warp_check: bool,
    ) -> Option<UnitId> {
        None
    }
    /// Bosses spawned of the boss's region += 1.
    fn count_region_boss(&mut self, unit: UnitId) {}
    /// Quest hook (quests).
    fn boss_quest_hook(&mut self, unit: UnitId) {}
    /// Owner data.
    fn boss_owner_data(&mut self, unit: UnitId) {}
    /// One boss minion spawn (with the coord list when there is one;
    /// spread 3,
    /// flags 0x40).
    fn spawn_boss_minion(
        &mut self,
        boss: UnitId,
        class: u32,
        coord_list: Option<u32>,
    ) -> Option<UnitId> {
        None
    }
    /// Owner data, minion list, owner GUID (monster
    /// AI).
    fn link_minion(&mut self, boss: UnitId, minion: UnitId) {}
    /// The unit's minion list.
    fn minions(&mut self, boss: UnitId) -> Vec<UnitId> {
        Vec::new()
    }
    /// Monster type `montype` is `ty` or nested in it
    /// (montype.txt).
    fn montype_is(&mut self, montype: u16, ty: u16) -> bool {
        montype == ty
    }
    /// Give and assign the aura skill.
    fn give_aura(&mut self, unit: UnitId, skill: u16, level: i32) {}
    /// Umod 26: set AI control flag 0x20.
    fn set_ai_flag(&mut self, unit: UnitId, flag: u16) {}
    /// Spawn a monster near the unit (class, mode, spread, flags).
    fn spawn_near_unit(&mut self, unit: UnitId, class: u32, mode: u32, spread: i32, flags: u32) {}
    /// "spawn(c, r, n, f)": a group of `n` of class `c` near the boss,
    /// mode 1, radius `r`, flags `f`.
    fn spawn_group(&mut self, boss: UnitId, class: u32, r: i32, n: i32, flags: u32) {}
    /// The quest preset-boss hook.
    fn quest_preset_boss(&mut self, unit: UnitId) {}
    /// HcIdx 60: owner data on the unit itself (own GUID, type 1,
    /// a 1, b 0).
    fn owner_data_self(&mut self, unit: UnitId) {}
    /// The class remapped for the unit's
    /// room level. Default: unchanged.
    fn class_for_level(&mut self, unit: UnitId, class: u32) -> u32 {
        class
    }
    /// A state on.
    fn set_state(&mut self, unit: UnitId, state: u16) {}
    /// Minion restore: a spawn with the GUID, flags 0x62
    /// (the population's retries).
    fn spawn_with_guid(&mut self, req: &CreateRequest) -> Option<UnitId> {
        None
    }

    // ---- callbacks ----

    /// A stat total:
    /// level(12), the damage stats, tohit. Default: the base value.
    fn stat_total(&self, unit: UnitId, stat: u16) -> i32 {
        self.stat(unit, stat)
    }
    /// "Base list set": list set on the unit's flag-1 list.
    /// Default: [`InitHost::set_stat`].
    fn set_base_list_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.set_stat(unit, stat, value);
    }
    /// The unit's position (static path for types 2, 4, 5, else the
    /// dynamic path; (0, 0) without a path).
    fn position(&self, unit: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// "Has s".
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        false
    }
    /// A state of flag group `group` (the runtime-map
    /// bitsets).
    fn has_state_in_group(&self, unit: UnitId, group: u8) -> bool {
        false
    }
    /// Step 4, the monster teardown: drop the unit's own
    /// combat-list entries; with `free_inventory` remove
    /// the inventory's items and free it;
    /// return the hover record (pointer kept); free AI
    /// control, AI params and the interaction block. The timers are
    /// cancelled by the
    /// caller. Default: nothing.
    fn monster_teardown(&mut self, unit: UnitId, free_inventory: bool) {}
    /// The plain mode set.
    /// Default: the unit record's mode.
    fn set_mode_plain(&mut self, unit: UnitId, mode: u32) {
        if let Some(r) = self.units().get_mut(unit) {
            r.mode = mode;
        }
    }
    /// "Mode set m": the mode request is built and sent with flag 1,
    /// which runs the umod dispatcher again.
    fn set_mode(&mut self, unit: UnitId, mode: u32) {}
    /// Owner.
    fn owner(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// Minion owner.
    fn minion_owner(&mut self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// Alignment (0, 1, 2).
    fn alignment(&self, unit: UnitId) -> u8 {
        0
    }
    /// Hostility.
    fn hostile(&self, a: UnitId, b: UnitId) -> bool {
        false
    }
    /// Target (the unit's target unit).
    fn target(&self, unit: UnitId) -> Option<UnitId> {
        None
    }
    /// Target position.
    fn target_position(&self, unit: UnitId) -> Option<(i32, i32)> {
        None
    }
    /// The target point of a missile's path (path target x /
    /// y).
    fn path_target_point(&self, missile: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// Missile creation with a parameter record. Runs the owner's mode-5
    /// umods.
    fn create_missile(&mut self, req: MissileRequest<UnitId>) -> Option<UnitId> {
        None
    }
    /// missiles row `class` flags dword; `None` without a row.
    fn missile_row_flags(&self, class: i32) -> Option<u32> {
        None
    }
    /// The velocity of missile `class` at `level`.
    fn missile_velocity(&self, class: i32, level: i32) -> i32 {
        0
    }
    /// A missile's (skill, level).
    fn missile_skill_level(&self, missile: UnitId) -> (i32, i32) {
        (0, 0)
    }
    /// The unit find around (x, y) from the room
    /// containing the point, searched from `near`'s room
    /// ([`super::find`]): the found units in
    /// order.
    fn find_units(&mut self, near: UnitId, q: FindQuery) -> Vec<UnitId> {
        Vec::new()
    }
    /// The line test of area damage: `from` to the unit is clear (mask 0x805).
    fn line_clear(&mut self, from: (i32, i32), unit: UnitId) -> bool {
        false
    }
    /// The missile hit (hit flags,
    /// block, damage apply and reaction); nothing unless `src` is a
    /// missile with an owner.
    fn missile_hit(&mut self, src: UnitId, unit: UnitId, rec: &DamageRecord) {}
    /// A skill-calc field of a skill at a level (the skill levels'
    /// `eval`) with `unit` as the caster.
    fn skill_calc(&mut self, unit: UnitId, skill: u16, calc: SkillCalc, level: i32) -> i32 {
        0
    }
    /// The aura columns of a skill row (`None` past the skills count).
    fn aura_fields(&self, skill: u16) -> Option<AuraFields> {
        None
    }
    /// itemstatcost and states row counts.
    fn stat_and_state_counts(&self) -> (i32, i32) {
        (0, 0)
    }
    /// `apply_state`; the state list made.
    fn apply_state(&mut self, req: StateApply) -> Option<ListId> {
        None
    }
    /// List set on a state list.
    fn set_list_stat(&mut self, list: ListId, stat: u16, value: i32) {}
    /// The unit's minion list freed.
    fn free_minions(&mut self, unit: UnitId) {}
    /// Owner data.
    fn clear_owner_data(&mut self, unit: UnitId) {}
    /// Pet remove (kill 1).
    fn remove_pet(&mut self, owner: UnitId, pet: UnitId) {}
    /// Undead class test.
    fn is_undead(&self, unit: UnitId) -> bool {
        false
    }
    /// missiles row `class` `Range`; `None` without a row (there is
    /// then no record to read).
    fn missile_range(&self, class: i32) -> Option<i32> {
        None
    }
    /// Set the uber death flag of `slot` (0 = ubermephisto,
    /// 1 = uberdiablo, 2 = uberbaal); true when all three are set. Default: never all set.
    fn set_uber_death(&mut self, slot: usize) -> bool {
        false
    }
    /// The game's uber quest counter.
    fn game_8c(&self) -> i32 {
        0
    }
    /// The unit's item code:= `code`, then the drop
    /// helper (the quest item drop
    /// of Act I); with `announce` also.
    fn quest_drop(&mut self, unit: UnitId, code: [u8; 4], arg: i32, announce: bool) {}
    /// Umod 24's steal (belt slot, copy, drop; through the items
    /// seams). No unit reaches it with the game's data.
    fn steal_belt_item(&mut self, unit: UnitId, target: UnitId) {}
    fn spawn_near(&mut self, unit: UnitId, class: u32, mode: u32, spread: i32, flags: u32) {}
    /// The point collides with mask 0x3C01 (pattern 1).
    fn footprint_occupied(&mut self, unit: UnitId) -> bool {
        false
    }
    /// AI param 0.
    fn ai_param0(&mut self, unit: UnitId) -> i32 {
        0
    }
    /// AI param 0:= v.
    fn set_ai_param0(&mut self, unit: UnitId, v: i32) {}
    /// The raise test.
    fn can_raise(&mut self, unit: UnitId) -> bool {
        false
    }
    /// The AI skill use (mode, skill, no target).
    fn ai_use_skill(&mut self, unit: UnitId, mode: u32, skill: u16) {}
    /// Clears unit flags: flags &= !mask.
    fn clear_unit_flags(&mut self, unit: UnitId, mask: u32) {
        if let Some(r) = self.units().get_mut(unit) {
            r.flags &= !mask;
        }
    }
    /// The level of the unit's skill `skill` (with bonus); `None` without
    /// the entry.
    fn skill_level(&mut self, unit: UnitId, skill: u16) -> Option<i32> {
        None
    }
    /// Life percent.
    fn life_percent(&self, unit: UnitId) -> i32 {
        0
    }
    /// The area level of the unit's room (level id, difficulty,
    /// expansion); `None` when the room has no
    /// level id.
    fn room_area_level(&mut self, unit: UnitId) -> Option<i32> {
        None
    }
}

/// Any init host as `dyn InitHost`, so generic init code can hand its host
/// to a mod game's umods (MS5).
pub trait AsDynInit {
    fn as_dyn_init(&mut self) -> &mut dyn InitHost;
}

impl<T: InitHost> AsDynInit for T {
    fn as_dyn_init(&mut self) -> &mut dyn InitHost {
        self
    }
}
