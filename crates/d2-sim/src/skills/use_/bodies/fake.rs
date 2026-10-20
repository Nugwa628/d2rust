//! A [`BodyWorld`] for the body tests: units, stats, combat and the
//! kick reads are [`crate::skills::fake::Fake`]'s (field `c`); stat
//! lists, handlers, rooms, positions, skill entries, missiles and summons
//! are kept here. Every effect is logged into `c.log` in call order.

use super::effects::{BodyEffect, PathOp};
use super::{BodyStat, BodyWorld, Handler, MissileRequest, MonsterSpawn, ProgressiveMsg, ScanRoom};
use crate::combat::{CombatEntry, DamageRecord, RoomKind};
use crate::rng::Seed;
use crate::skills::fake::{FUnit, Fake};
use crate::skills::use_::{
    MissileAim, ModeTarget, ServerMsg, SkillFunctions, UseMissiles, UseState, UseWorld,
};
use crate::skills::{KickItems, ManaUnits, SkillEntry, SkillUnits};
use crate::units::UnitType;
use d2_data::tables::Monlvl;
use std::collections::{BTreeMap, BTreeSet};

/// One stat list.
#[derive(Debug, Clone, Default)]
pub struct FList {
    pub flags: u32,
    pub expire: i32,
    /// The owner the list was allocated with.
    #[allow(dead_code)]
    pub owner: Option<usize>,
    pub state: i32,
    pub skill: i32,
    pub lvl: i32,
    pub stats: BTreeMap<i32, i32>,
    pub callback: u32,
    pub unit: Option<usize>,
    pub freed: bool,
}

/// One `unit_find` call: (at, radius, filter).
pub type FindArgs = ((i32, i32), i32, u32);

/// One `unit_find` call: (centre, radius, flags).
pub type FindCall = ((i32, i32), i32, u32);

/// The fake body world.
#[derive(Debug, Clone, Default)]
pub struct BodyFake {
    pub c: Fake,
    pub lists: Vec<FList>,
    pub handlers: BTreeMap<usize, Vec<Handler>>,
    /// States count; `state_flags[(s, g)]` the flag groups.
    pub states: i32,
    pub state_flags: BTreeSet<(i32, usize)>,
    pub state_groups: BTreeMap<i32, i32>,
    pub stat_infos: BTreeMap<i32, BodyStat>,
    pub overlays: i32,
    pub monlvl: Vec<Monlvl>,
    pub pettypes: i32,
    pub l_flag: bool,
    pub pos: BTreeMap<usize, (i32, i32)>,
    pub targets: BTreeMap<usize, usize>,
    pub tpos: BTreeMap<usize, (i32, i32)>,
    /// Unit → room; rooms in town; the room of every point (default
    /// `point_room`, unless `point_rooms` names the point).
    pub room_of: BTreeMap<usize, usize>,
    pub town: BTreeSet<usize>,
    pub point_room: Option<usize>,
    pub point_rooms: BTreeMap<(i32, i32), Option<usize>>,
    pub teleport: Option<i32>,
    pub blocked: bool,
    pub collides: bool,
    pub free_shift: Option<(i32, i32)>,
    /// The units every scan visits, in order.
    pub scan: Vec<usize>,
    pub entries: BTreeMap<(usize, i32, u8), i32>,
    pub entry_flags: BTreeMap<(usize, i32), u32>,
    pub alive: BTreeSet<usize>,
    pub dead: BTreeSet<usize>,
    pub missiles: Vec<MissileRequest<usize>>,
    /// What every `srvdo` returns (the do core's result).
    pub srvdo_result: i32,
    pub no_missiles: bool,
    pub no_monsters: bool,
    /// The next `n` `spawn_monster` calls fail.
    pub fail_spawns: u32,
    pub c8: BTreeMap<usize, u32>,
    pub node: BTreeMap<usize, i32>,
    pub frame_index: BTreeMap<usize, i32>,
    pub frame_count: BTreeMap<usize, i32>,
    pub anim_frame: BTreeMap<usize, i32>,
    pub hand_class: i32,
    pub inventory: bool,
    pub shield: Option<usize>,
    pub path: bool,
    pub path_points: i32,
    pub path_target: (i32, i32),
    pub spawn_class: Option<i32>,
    pub minion_owner: BTreeMap<usize, usize>,
    pub allies: BTreeSet<(usize, usize)>,
    pub sequence: Option<Vec<[u8; 6]>>,
    pub frames: Option<i32>,
    // ---- batch 4
    /// `dir64` answers by target point (default 0).
    pub dirs: BTreeMap<(i32, i32), i32>,
    pub action: BTreeMap<usize, i32>,
    pub seq_speed: BTreeMap<usize, i32>,
    pub anim_speed: BTreeMap<usize, i32>,
    /// Missile unit → (total, left).
    pub mframes: BTreeMap<usize, (i32, i32)>,
    pub seq_frames: Option<i32>,
    pub seq_events: BTreeMap<i32, i32>,
    pub anim_data: Option<(u32, Vec<u8>)>,
    pub action_event: bool,
    pub chains: BTreeMap<i32, i32>,
    /// `class_for_level` remaps (class → class); absent → unchanged.
    pub level_classes: BTreeMap<i32, i32>,
    /// `missile_owner` answers (default none).
    pub missile_owners: BTreeMap<usize, usize>,
    pub books: BTreeMap<usize, (i32, i32)>,
    pub inv_nodes: Vec<(usize, i32)>,
    pub found: Vec<usize>,
    pub point_collide: bool,
    pub components: BTreeMap<(usize, usize), i32>,
    /// Selected skill per (unit, left hand) (`stat_cb` tests).
    pub sel: BTreeMap<(usize, bool), SkillEntry>,
    /// Pettype row → skill ids (`stat_cb` tests).
    pub pet_skills: BTreeMap<i32, Vec<i32>>,
    /// `place_unit` refuses (the move test fails).
    pub place_fails: bool,
    /// The (at, radius, filter) of every `unit_find`.
    pub find_args: std::cell::RefCell<Vec<FindArgs>>,
    /// Item stat values by (item, stat) (`item_stat_of`).
    pub item_stats: BTreeMap<(usize, u16), i32>,
    /// The (centre, radius, flags) of every `unit_find` call.
    pub finds: std::cell::RefCell<Vec<FindCall>>,
    /// Base stats given to every missile `spawn_missile` creates.
    pub missile_base: Vec<(u16, i32)>,
    // ---- coverage tests
    pub shoots: BTreeSet<usize>,
    pub max_stack: BTreeMap<usize, i32>,
    pub max_dura: BTreeMap<usize, i32>,
    pub composit_class: i32,
    /// Units of a second (town) room every scan visits after `scan`.
    pub scan_town: Vec<usize>,
    pub frame_bonus_v: i32,
    pub unsummon_ok: bool,
    /// `inventory_busy` answers this.
    pub busy: bool,
    /// `item_missile_type` by item (default 0).
    pub item_missiles: BTreeMap<usize, i32>,
}

impl BodyFake {
    /// A world with `states` states and the overlay count 200.
    pub fn new() -> Self {
        Self {
            states: 200,
            overlays: 200,
            pettypes: 15,
            point_room: Some(1),
            path: true,
            hand_class: 1,
            ..Self::default()
        }
    }

    /// Adds a unit at `at`, alive, in room 1.
    pub fn add(&mut self, u: FUnit, at: (i32, i32)) -> usize {
        let id = self.c.add(u);
        self.pos.insert(id, at);
        self.room_of.insert(id, 1);
        self.alive.insert(id);
        self.node.insert(id, 11);
        id
    }

    /// Takes the log.
    pub fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.c.log)
    }

    fn log(&mut self, s: String) {
        self.c.log.push(s);
    }

    /// The live list of state `s` on `u`.
    pub fn list_of(&self, u: usize, s: i32) -> Option<&FList> {
        self.lists
            .iter()
            .find(|l| !l.freed && l.unit == Some(u) && l.state == s)
    }
}

impl SkillUnits for BodyFake {
    type Unit = usize;
    type Item = usize;
    fn unit_type(&self, u: usize) -> UnitType {
        self.c.unit_type(u)
    }
    fn class_id(&self, u: usize) -> i32 {
        self.c.class_id(u)
    }
    fn stat(&self, u: usize, s: u16, l: u16) -> i32 {
        self.c.stat(u, s, l)
    }
    fn item_stat(&self, u: usize, s: u16, l: u16) -> i32 {
        self.c.item_stat(u, s, l)
    }
    fn base_stat(&self, u: usize, s: u16, l: u16) -> i32 {
        self.c.stat(u, s, l)
    }
    fn formula_stat(&self, u: usize, s: u16, m: i32) -> i32 {
        self.c.formula_stat(u, s, m)
    }
    fn stat_entries(&self, u: usize, s: u16, max: usize) -> Vec<(u16, i32)> {
        self.c.stat_entries(u, s, max)
    }
    fn has_state(&self, u: usize, s: u16) -> bool {
        self.c.has_state(u, s)
    }
    fn state_stat(&self, u: usize, s: u16, st: u16) -> Option<i32> {
        self.list_of(u, i32::from(s))
            .and_then(|l| l.stats.get(&i32::from(st)).copied())
    }
    fn seed(&mut self, u: usize) -> &mut Seed {
        self.c.seed(u)
    }
    fn skill_list(&self, u: usize) -> Vec<SkillEntry> {
        self.c.skill_list(u)
    }
    fn used_skill(&self, u: usize) -> Option<SkillEntry> {
        self.c.used_skill(u)
    }
    fn current_weapon(&self, u: usize) -> Option<usize> {
        self.c.current_weapon(u)
    }
    fn weapon(&self, u: usize) -> Option<usize> {
        self.c.weapon(u)
    }
    fn item_at(&self, u: usize, loc: u8) -> Option<usize> {
        self.c.item_at(u, loc)
    }
    fn item_is(&self, i: usize, t: i32) -> bool {
        self.c.item_is(i, t)
    }
    fn itype_is(&self, a: i32, b: i32) -> bool {
        self.c.itype_is(a, b)
    }
    fn wield_type(&self, i: usize) -> i32 {
        self.c.wield_type(i)
    }
    fn item_damage(&self, i: usize, max: bool) -> i32 {
        self.c.item_damage(i, max)
    }
    fn str_dex_bonus(&self, i: usize) -> (i32, i32) {
        self.c.str_dex_bonus(i)
    }
    fn item_flag_throw(&self, i: usize) -> bool {
        self.c.item_flag_throw(i)
    }
    fn missile_level(&self, u: usize) -> i32 {
        self.c.missile_level(u)
    }
}

impl ManaUnits for BodyFake {
    fn shapeshifted(&self, u: usize) -> bool {
        self.c.shapeshifted(u)
    }
    fn consume_charges(&mut self, u: usize, e: &SkillEntry) -> bool {
        self.c.consume_charges(u, e)
    }
    fn pay_life(&mut self, u: usize, cost: i32) -> bool {
        self.c.pay_life(u, cost)
    }
    fn set_stat(&mut self, u: usize, s: u16, v: i32) {
        ManaUnits::set_stat(&mut self.c, u, s, v);
    }
}

impl KickItems for BodyFake {
    fn toggle_weapon_lists(&mut self, u: usize, on: bool) {
        self.c.toggle_weapon_lists(u, on);
    }
    fn boots_damage(&self, i: usize) -> (i32, i32) {
        self.c.boots_damage(i)
    }
}

impl UseMissiles for BodyFake {
    fn create_skill_missile(
        &mut self,
        u: usize,
        skill: i32,
        l: i32,
        m: u16,
        lob: bool,
        _: MissileAim,
    ) {
        self.log(format!("skillmissile {u} {skill} {l} {m} {lob}"));
    }
}

impl SkillFunctions for BodyFake {
    fn srvst(&mut self, index: u16, u: usize, skill: i32, lvl: i32) -> i32 {
        self.log(format!("srvst {index} {u} {skill} {lvl}"));
        0
    }
    fn srvdo(
        &mut self,
        index: u16,
        u: usize,
        skill: i32,
        lvl: i32,
        _: bool,
        _: bool,
        _: bool,
    ) -> i32 {
        self.log(format!("srvdo {index} {u} {skill} {lvl}"));
        self.srvdo_result
    }
}

impl UseWorld for BodyFake {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn frame(&self) -> i32 {
        self.c.frame
    }
    fn send(&mut self, u: usize, msg: ServerMsg) {
        self.log(format!("send {u} {msg:?}"));
    }
    fn has_player_data(&self, u: usize) -> bool {
        self.c.unit_type(u) == UnitType::Player
    }
    fn last_point_frame(&self, _: usize) -> i32 {
        0
    }
    fn set_last_point_frame(&mut self, _: usize, _: i32) {}
    fn position(&self, u: usize) -> (i32, i32) {
        self.pos.get(&u).copied().unwrap_or((0, 0))
    }
    fn find_unit(&self, ty: u32, guid: u32) -> Option<usize> {
        self.c
            .units
            .iter()
            .position(|x| x.kind.index() as u32 == ty && x.guid == guid)
    }
    fn in_own_inventory(&self, _: usize, _: usize) -> bool {
        false
    }
    fn same_act(&self, _: usize, _: usize) -> bool {
        true
    }
    fn within_reach(&self, _: usize, _: usize) -> bool {
        true
    }
    fn owner(&self, u: usize) -> Option<usize> {
        self.minion_owner.get(&u).copied()
    }
    fn left_skill(&self, _: usize) -> Option<SkillEntry> {
        None
    }
    fn right_skill(&self, _: usize) -> Option<SkillEntry> {
        None
    }
    fn set_left_skill(&mut self, _: usize, _: SkillEntry) {}
    fn set_right_skill(&mut self, _: usize, _: SkillEntry) {}
    fn find_entry(&self, u: usize, skill: i32) -> Option<SkillEntry> {
        self.c.units[u]
            .skills
            .iter()
            .find(|e| e.skill == skill)
            .copied()
    }
    fn find_entry_owned(&self, u: usize, skill: i32, owner: i32) -> Option<SkillEntry> {
        self.c.units[u]
            .skills
            .iter()
            .find(|e| e.skill == skill && e.owner_guid == owner)
            .copied()
    }
    fn owns_skill(&self, u: usize, skill: i32) -> bool {
        self.find_entry(u, skill).is_some()
    }
    fn set_used_skill(&mut self, u: usize, e: Option<SkillEntry>) {
        self.log(format!("used {u} {:?}", e.map(|e| e.skill)));
        self.c.units[u].used = e;
    }
    fn used_skill_flags(&self, _: usize) -> u32 {
        0
    }
    fn set_used_skill_flags(&mut self, _: usize, _: u32) {}
    fn entry_mode(&self, _: usize, _: &SkillEntry) -> u32 {
        0
    }
    fn attack_param4(&self, _: usize) -> i32 {
        0
    }
    fn set_attack_param4(&mut self, _: usize, _: i32) {}
    fn use_state(&mut self, _: usize, _: &SkillEntry) -> UseState {
        UseState::Usable
    }
    fn dec_quantity(&mut self, u: usize, skill: i32) {
        self.log(format!("decquant {u} {skill}"));
    }
    fn can_dual_wield(&self, _: usize) -> bool {
        false
    }
    fn equippable(&self, _: usize) -> bool {
        true
    }
    fn bow_equipped(&self, _: usize) -> bool {
        false
    }
    fn state_mask(&self, _: usize, _: u32) -> bool {
        false
    }
    fn in_melee_range(&self, a: usize, b: usize) -> bool {
        crate::combat::CombatWorld::in_melee_range(&self.c, a, b, 0)
    }
    fn mode(&self, u: usize) -> u32 {
        self.c.units[u].mode as u32
    }
    fn cursor_item(&self, _: usize) -> bool {
        false
    }
    fn endanim_expire(&self, _: usize) -> i32 {
        0
    }
    fn set_mode(&mut self, u: usize, mode: u32) {
        self.log(format!("mode {u} {mode}"));
        self.c.units[u].mode = mode as i32;
    }
    fn start_mode(&mut self, u: usize, mode: u32, t: ModeTarget<usize>) {
        self.log(format!("startmode {u} {mode} {t:?}"));
    }
    fn run_to(&mut self, _: usize, _: usize, _: SkillEntry) {}
    fn target(&self, u: usize) -> Option<usize> {
        self.targets.get(&u).copied()
    }
    fn clear_target(&mut self, u: usize) {
        self.targets.remove(&u);
    }
    fn unit_flags(&self, u: usize) -> u32 {
        self.c.units[u].flags
    }
    fn set_unit_flags(&mut self, u: usize, f: u32) {
        self.c.units[u].flags = f;
    }
    fn event_arg(&self, _: usize) -> i32 {
        0
    }
    fn set_event_arg(&mut self, _: usize, _: i32) {}
    fn step_path(&mut self, _: usize) -> i32 {
        0
    }
    fn is_alive(&self, u: usize) -> bool {
        self.alive.contains(&u) && !self.dead.contains(&u)
    }
    fn is_hostile(&self, _: usize, _: usize) -> bool {
        self.c.hostile
    }
    fn is_pet(&self, _: usize, _: usize) -> bool {
        false
    }
    fn is_ally(&self, a: usize, b: usize) -> bool {
        self.allied(a, b)
    }
    fn room(&self, u: usize) -> RoomKind {
        match self.room_of.get(&u) {
            None => RoomKind::None,
            Some(r) if self.town.contains(r) => RoomKind::Town,
            Some(_) => RoomKind::Field,
        }
    }
    fn target_position(&self, u: usize) -> Option<(i32, i32)> {
        if let Some(&p) = self.tpos.get(&u) {
            return Some(p);
        }
        self.target(u).map(|t| self.position(t))
    }
    fn line_clear(&self, _: usize, _: (i32, i32), _: u32) -> bool {
        !self.blocked
    }
    fn schedule(&mut self, u: usize, kind: u8, frame: i32, a1: i32, a2: i32) {
        self.log(format!("schedule {u} {kind} {frame} {a1} {a2}"));
    }
    fn delete_timers(&mut self, u: usize, kind: u8, a1: i32) {
        self.log(format!("deltimers {u} {kind} {a1}"));
    }
    fn has_state_list(&self, u: usize, s: u16) -> bool {
        self.list_of(u, i32::from(s)).is_some()
    }
    fn create_delay_list(&mut self, u: usize, e: i32) {
        self.log(format!("delay {u} {e}"));
    }
    fn set_state_list_expiry(&mut self, _: usize, _: u16, _: i32) {}
    fn free_aura_state(&mut self, _: usize, _: u16) {}
    fn set_aura_state(&mut self, _: usize, _: u16, _: i32, _: i32) {}
}

impl BodyWorld for BodyFake {
    type List = usize;
    type Room = usize;
    type Combat = Fake;
    fn combat(&mut self) -> &mut Fake {
        &mut self.c
    }
    fn stat_info(&self, s: i32) -> Option<BodyStat> {
        Some(self.stat_infos.get(&s).copied().unwrap_or(BodyStat {
            maxstat: -1,
            ..BodyStat::default()
        }))
    }
    fn state_count(&self) -> i32 {
        self.states
    }
    fn state_flag(&self, s: i32, g: usize) -> bool {
        self.state_flags.contains(&(s, g))
    }
    fn state_group(&self, s: i32) -> i32 {
        self.state_groups.get(&s).copied().unwrap_or(0)
    }
    fn state_is_aura(&self, _: i32) -> bool {
        false
    }
    fn overlay_count(&self) -> i32 {
        self.overlays
    }
    fn has_group(&self, u: usize, g: usize) -> bool {
        self.c.units[u]
            .states
            .iter()
            .any(|&s| self.state_flags.contains(&(i32::from(s), g)))
    }
    fn state_on(&mut self, u: usize, s: i32, on: bool) {
        crate::combat::CombatWorld::set_state(&mut self.c, u, s as u16, on);
    }
    fn mark_state_changed(&mut self, u: usize, s: i32) {
        self.log(format!("changed {u} {s}"));
    }
    fn clear_group_states(&mut self, u: usize, g: usize) {
        self.log(format!("cleargroup {u} {g}"));
    }
    fn queue_update(&mut self, u: usize) {
        self.log(format!("update {u}"));
    }
    fn stays_on_death(&self, _: usize, _: i32) -> bool {
        false
    }
    fn state_list(&self, u: usize, s: i32) -> Option<usize> {
        self.lists
            .iter()
            .position(|l| !l.freed && l.unit == Some(u) && l.state == s)
    }
    fn first_list_with_flags(&self, u: usize, flags: u32) -> Option<usize> {
        self.lists
            .iter()
            .position(|l| !l.freed && l.unit == Some(u) && l.flags & flags == flags)
    }
    fn alloc_list(&mut self, flags: u32, expire: i32, owner: Option<usize>) -> Option<usize> {
        self.lists.push(FList {
            flags,
            expire,
            owner,
            state: 0,
            ..FList::default()
        });
        Some(self.lists.len() - 1)
    }
    fn list_state(&self, l: usize) -> i32 {
        self.lists[l].state
    }
    fn set_list_state(&mut self, l: usize, s: i32) {
        self.lists[l].state = s;
    }
    fn list_skill(&self, l: usize) -> (i32, i32) {
        (self.lists[l].skill, self.lists[l].lvl)
    }
    fn set_list_skill(&mut self, l: usize, skill: i32, lvl: i32) {
        self.lists[l].skill = skill;
        self.lists[l].lvl = lvl;
    }
    fn list_expire(&self, l: usize) -> i32 {
        self.lists[l].expire
    }
    fn set_list_expire(&mut self, l: usize, e: i32) {
        self.lists[l].expire = e;
    }
    fn list_get(&self, l: usize, s: i32) -> i32 {
        self.lists[l].stats.get(&s).copied().unwrap_or(0)
    }
    fn list_set(&mut self, l: usize, s: i32, v: i32) {
        self.lists[l].stats.insert(s, v);
    }
    /// Layer `k` > 0 is kept under the key `k << 16 | s`.
    fn list_set_layer(&mut self, l: usize, s: i32, v: i32, layer: u16) {
        self.lists[l].stats.insert(i32::from(layer) << 16 | s, v);
    }
    fn attach(&mut self, u: usize, l: usize) {
        self.lists[l].unit = Some(u);
    }
    fn set_remove_callback(&mut self, l: usize, cb: u32) {
        self.lists[l].callback = cb;
    }
    fn detach_free(&mut self, u: usize, l: usize) {
        self.log(format!("free {u} {l}"));
        self.lists[l].freed = true;
    }
    fn add_handler(&mut self, u: usize, h: Handler) {
        self.log(format!("handler {u} {} {} {}", h.event, h.func, h.key));
        self.handlers.entry(u).or_default().insert(0, h);
    }
    fn remove_handlers(&mut self, u: usize, kt: i32, key: i32) {
        self.log(format!("unhandle {u} {kt} {key}"));
        if let Some(v) = self.handlers.get_mut(&u) {
            v.retain(|h| !(h.key_type == kt && h.key == key));
        }
    }
    fn scan_rooms(&self, _: usize, _: Option<(i32, i32)>) -> Option<Vec<ScanRoom<usize>>> {
        let mut v = vec![ScanRoom {
            town: false,
            units: self.scan.clone(),
        }];
        if !self.scan_town.is_empty() {
            v.push(ScanRoom {
                town: true,
                units: self.scan_town.clone(),
            });
        }
        Some(v)
    }
    fn allied(&self, a: usize, b: usize) -> bool {
        a == b || self.allies.contains(&(a, b))
    }
    fn missile_owner(&self, u: usize) -> Option<usize> {
        self.missile_owners.get(&u).copied()
    }
    fn minion_owner(&self, u: usize) -> Option<usize> {
        self.minion_owner.get(&u).copied()
    }
    fn pet_unsummonable(&self, _: usize, _: usize) -> bool {
        self.unsummon_ok
    }
    fn frame_bonus(&self, _: usize) -> i32 {
        self.frame_bonus_v
    }
    fn set_anim_frame(&mut self, u: usize, v: i32) {
        self.anim_frame.insert(u, v);
    }
    fn set_entry_param(&mut self, u: usize, i: u8, v: i32) {
        let k = self.c.units[u].used.map_or(-1, |e| e.skill);
        self.entries.insert((u, k, i), v);
    }
    fn stat_max(&self, u: usize, s: u16) -> i32 {
        self.c.stat(u, s + 1, 0)
    }
    fn composit_weapon_class(&self, _: usize) -> i32 {
        self.composit_class
    }
    fn hand_class(&self, _: usize) -> i32 {
        self.hand_class
    }
    fn item_shoots(&self, i: usize) -> bool {
        self.shoots.contains(&i)
    }
    fn item_stackable(&self, i: usize) -> bool {
        self.c.items[i].throw
    }
    fn item_stat_of(&self, i: usize, s: u16) -> i32 {
        self.item_stats.get(&(i, s)).copied().unwrap_or(0)
    }
    fn set_item_stat(&mut self, i: usize, s: u16, v: i32) {
        self.item_stats.insert((i, s), v);
        self.log(format!("itemstat {i} {s} {v}"));
    }
    fn item_max_stack(&self, i: usize) -> i32 {
        self.max_stack.get(&i).copied().unwrap_or(0)
    }
    fn item_max_durability(&self, i: usize) -> i32 {
        self.max_dura.get(&i).copied().unwrap_or(0)
    }
    fn quantity_timer(&mut self, _: usize) {}
    fn send_item_stat(&mut self, u: usize, i: usize, s: u16, v: i32) {
        self.log(format!("send0x3E {u} {i} {s} {v}"));
    }
    fn attack_cleanup(&mut self, u: usize) {
        self.log(format!("attackcleanup {u}"));
    }
    fn weapon_cleanup(&mut self, u: usize) {
        self.log(format!("weaponcleanup {u}"));
    }
    fn spawn_missile(&mut self, req: MissileRequest<usize>) -> Option<usize> {
        self.missiles.push(req);
        if self.no_missiles {
            return None;
        }
        let m = self.c.add(FUnit::new(UnitType::Missile, req.class));
        for &(s, v) in &self.missile_base {
            self.c.set(m, s, v);
        }
        self.pos.insert(m, (req.x, req.y));
        Some(m)
    }
    fn passive_refresh(&mut self, u: usize) {
        self.log(format!("passive {u}"));
    }
    fn buff_refresh(&mut self, u: usize) {
        self.log(format!("buffrefresh {u}"));
    }
    fn skill_resync(&mut self, _: usize) {}
    fn passive_state_apply(&mut self, _: usize, _: &SkillEntry) {}
    fn set_ai_state(&mut self, u: usize, k: i32) {
        self.log(format!("ai {u} {k}"));
    }
    fn blood_mana(&mut self, _: usize, _: i32) {}
    fn queue_progressive(&mut self, _: usize, _: ProgressiveMsg<usize>) {}

    fn effect(&mut self, e: BodyEffect<usize, usize, usize>) {
        self.log(format!("{e:?}"));
    }
    fn path_op(&mut self, u: usize, op: PathOp<usize>) -> i32 {
        self.log(format!("path {u} {op:?}"));
        if let PathOp::TargetUnit(t) = op {
            match t {
                Some(t) => self.targets.insert(u, t),
                None => self.targets.remove(&u),
            };
        }
        i32::from(op == PathOp::Compute)
    }
    fn monlvl(&self) -> &[Monlvl] {
        &self.monlvl
    }
    fn pettype_count(&self) -> i32 {
        self.pettypes
    }
    fn l_flag(&self) -> bool {
        self.l_flag
    }
    fn unit_c8(&self, u: usize) -> u32 {
        self.c8.get(&u).copied().unwrap_or(0)
    }
    fn set_unit_c8(&mut self, u: usize, v: u32) {
        self.c8.insert(u, v);
    }
    fn anim_frame(&self, u: usize) -> i32 {
        self.anim_frame.get(&u).copied().unwrap_or(0)
    }
    fn frame_event_index(&self, u: usize) -> i32 {
        self.frame_index.get(&u).copied().unwrap_or(0)
    }
    fn set_frame_event_index(&mut self, u: usize, i: i32) {
        self.frame_index.insert(u, i);
    }
    fn frame_count(&self, u: usize) -> i32 {
        self.frame_count.get(&u).copied().unwrap_or(0)
    }
    fn set_frame_count(&mut self, u: usize, v: i32) {
        self.frame_count.insert(u, v);
    }
    fn node_slot(&self, u: usize) -> i32 {
        self.node.get(&u).copied().unwrap_or(11)
    }
    fn has_stat_holder(&self, _: usize) -> bool {
        true
    }
    fn unit_size(&self, _: usize) -> i32 {
        1
    }
    fn minion_spawn_class(&self, _: usize) -> Option<i32> {
        self.spawn_class
    }
    fn linked_unit(&self, _: usize) -> Option<usize> {
        None
    }
    fn killer_of(&self, _: usize) -> Option<usize> {
        None
    }
    fn minion_owner_ident(&self, u: usize) -> Option<(u32, u32)> {
        let o = self.minion_owner.get(&u)?;
        Some((self.c.units[*o].guid, self.c.units[*o].kind.index() as u32))
    }
    fn add_stat(&mut self, u: usize, s: u16, v: i32) {
        let b = self.c.stat(u, s, 0);
        self.c.units[u].stats.insert((s, 0), b.wrapping_add(v));
        self.log(format!("add {u} {s} {v}"));
    }
    fn list_add(&mut self, l: usize, s: i32, v: i32) {
        *self.lists[l].stats.entry(s).or_default() += v;
    }
    fn list_clear(&mut self, l: usize) {
        self.lists[l].stats.clear();
    }
    fn has_handler(&self, u: usize, kt: i32, key: i32, skill: i32) -> bool {
        self.handlers.get(&u).is_some_and(|v| {
            v.iter()
                .any(|h| h.key_type == kt && h.key == key && h.skill == skill)
        })
    }
    fn entry_param(&self, u: usize, e: &SkillEntry, i: u8) -> i32 {
        self.entries.get(&(u, e.skill, i)).copied().unwrap_or(0)
    }
    fn set_entry_param_of(&mut self, u: usize, e: &SkillEntry, i: u8, v: i32) {
        self.entries.insert((u, e.skill, i), v);
    }
    fn entry_flags(&self, u: usize, e: &SkillEntry) -> u32 {
        self.entry_flags.get(&(u, e.skill)).copied().unwrap_or(0)
    }
    fn set_entry_flags(&mut self, u: usize, e: &SkillEntry, f: u32) {
        self.entry_flags.insert((u, e.skill), f);
    }
    fn set_entry_mode(&mut self, u: usize, e: &SkillEntry, m: u32) {
        self.log(format!("entrymode {u} {} {m}", e.skill));
    }
    fn disguise_mode(&self, _: usize, m: u32) -> u32 {
        m
    }
    fn skill_sequence(&self, _: usize) -> Option<Vec<[u8; 6]>> {
        self.sequence.clone()
    }
    fn anim_rewind(&mut self, u: usize, p: i32) {
        self.log(format!("rewind {u} {p}"));
    }
    fn anim_restart(&mut self, u: usize, v: i32) {
        self.log(format!("restart {u} {v}"));
    }
    fn anim_from(&mut self, u: usize, f: i32) {
        self.log(format!("animfrom {u} {f}"));
    }
    fn unit_room(&self, u: usize) -> Option<usize> {
        self.room_of.get(&u).copied()
    }
    fn room_at(&self, _: usize, x: i32, y: i32) -> Option<usize> {
        match self.point_rooms.get(&(x, y)) {
            Some(r) => *r,
            None => self.point_room,
        }
    }
    fn room_in_town(&self, r: usize) -> bool {
        self.town.contains(&r)
    }
    fn room_act(&self, _: usize) -> i32 {
        0
    }
    fn room_teleport(&self, _: usize) -> Option<i32> {
        self.teleport
    }
    fn free_point(
        &mut self,
        r: usize,
        (x, y): (i32, i32),
        _: i32,
        _: u32,
        _: bool,
    ) -> Option<(usize, (i32, i32))> {
        let (dx, dy) = self.free_shift?;
        Some((r, (x + dx, y + dy)))
    }
    fn pattern_collides(&self, _: usize, _: (i32, i32), _: usize, _: u32) -> bool {
        self.collides
    }
    fn box_collides(&self, _: usize, _: (i32, i32), _: i32, _: u32) -> bool {
        self.collides
    }
    fn line_blocked(&self, _: usize, _: (i32, i32), _: (i32, i32), _: u32) -> bool {
        self.blocked
    }
    fn place_unit(&mut self, u: usize, r: Option<usize>, at: (i32, i32)) -> bool {
        self.log(format!("place {u} {r:?} {at:?}"));
        if self.place_fails {
            return false;
        }
        self.pos.insert(u, at);
        true
    }
    fn has_path(&self, _: usize) -> bool {
        self.path
    }
    fn path_point_count(&self, _: usize) -> i32 {
        self.path_points
    }
    fn path_last_point(&self, _: usize) -> (i32, i32) {
        self.path_target
    }
    fn path_target_point(&self, _: usize) -> (i32, i32) {
        self.path_target
    }
    fn create_monster(
        &mut self,
        r: usize,
        at: (i32, i32),
        class: i32,
        mode: i32,
        spread: i32,
    ) -> Option<usize> {
        self.log(format!("monster {r} {at:?} {class} {mode} {spread}"));
        if self.no_monsters {
            return None;
        }
        let m = self.add(FUnit::new(UnitType::Monster, class), at);
        Some(m)
    }
    fn mode_request(&mut self, m: usize, mode: i32, t: Option<usize>) -> i32 {
        self.log(format!("moderequest {m} {mode} {t:?}"));
        1
    }
    fn as_item(&self, u: usize) -> Option<usize> {
        (self.c.unit_type(u) == UnitType::Item).then_some(self.c.units[u].class as usize)
    }
    fn inventory_busy(&self, _: usize) -> bool {
        self.busy
    }
    fn has_inventory(&self, _: usize) -> bool {
        self.inventory
    }
    fn weapon_in_use(&self, u: usize) -> Option<usize> {
        self.c.current_weapon(u)
    }
    fn body_loc(&self, _: usize) -> i32 {
        4
    }
    fn item_usable(&self, _: usize) -> bool {
        true
    }
    fn item_active(&self, _: usize) -> bool {
        true
    }
    fn item_breakable(&self, i: usize) -> bool {
        self.c.items[i].durability
    }
    fn shield(&self, _: usize) -> Option<usize> {
        self.shield
    }
    fn shield_damage(&self, i: usize) -> Option<(i32, i32)> {
        Some(self.c.items[i].damage)
    }
    fn item_missile_type(&self, i: usize) -> i32 {
        self.item_missiles.get(&i).copied().unwrap_or(0)
    }
    fn golem_item(&self, t: usize) -> bool {
        self.c.unit_type(t) == UnitType::Item
    }
    fn item_first_loc(&self, _: usize) -> i32 {
        4
    }
    fn two_melee_weapons(&self, _: usize) -> bool {
        false
    }
    fn attack_frames(&self, _: usize, _: usize) -> Option<i32> {
        self.frames
    }
    fn dir64(&self, _: usize, at: (i32, i32)) -> i32 {
        self.dirs.get(&at).copied().unwrap_or(0)
    }
    fn action_frame(&self, u: usize) -> i32 {
        self.action.get(&u).copied().unwrap_or(0)
    }
    fn set_action_frame(&mut self, u: usize, v: i32) {
        self.action.insert(u, v);
    }
    fn set_seq_speed(&mut self, u: usize, v: i32) {
        self.seq_speed.insert(u, v);
    }
    fn anim_speed(&self, u: usize) -> i32 {
        self.anim_speed.get(&u).copied().unwrap_or(0)
    }
    fn set_anim_speed(&mut self, u: usize, v: i32) {
        self.anim_speed.insert(u, v);
    }
    fn missile_frames(&self, m: usize) -> i32 {
        self.mframes.get(&m).map_or(0, |f| f.0)
    }
    fn set_missile_frames(&mut self, m: usize, total: i32, left: i32) {
        self.mframes.insert(m, (total, left));
    }
    fn sequence_frames(&self, _: usize) -> Option<i32> {
        self.seq_frames
    }
    fn sequence_event(&self, _: usize, f: i32) -> i32 {
        self.seq_events.get(&f).copied().unwrap_or(0)
    }
    fn anim_data(&self, _: usize) -> Option<(u32, Vec<u8>)> {
        self.anim_data.clone()
    }
    fn action_event_between(&self, _: usize, _: i32, _: i32) -> bool {
        self.action_event
    }
    fn chain_position(&self, class: i32) -> i32 {
        self.chains.get(&class).copied().unwrap_or(0)
    }
    fn class_for_level(&self, _: Option<usize>, class: i32) -> i32 {
        self.level_classes.get(&class).copied().unwrap_or(class)
    }
    fn book_skills(&self, i: usize) -> Option<(i32, i32)> {
        self.books.get(&i).copied()
    }
    fn inventory_nodes(&self, _: usize) -> Vec<(usize, i32)> {
        self.inv_nodes.clone()
    }
    fn unit_find(&self, _: usize, at: (i32, i32), r: i32, f: u32) -> Vec<usize> {
        self.find_args.borrow_mut().push((at, r, f));
        self.finds.borrow_mut().push((at, r, f));
        self.found.clone()
    }
    fn point_collides(&self, _: usize, _: (i32, i32), _: u32) -> bool {
        self.point_collide
    }
    fn spawn_monster(&mut self, q: MonsterSpawn<usize, usize>) -> Option<usize> {
        self.log(format!("{q:?}"));
        if self.fail_spawns > 0 {
            self.fail_spawns -= 1;
            return None;
        }
        if self.no_monsters {
            return None;
        }
        let (class, at) = match q {
            MonsterSpawn::At { x, y, class, .. }
            | MonsterSpawn::Leader { x, y, class, .. }
            | MonsterSpawn::Minion { x, y, class, .. } => (class, (x, y)),
            MonsterSpawn::Near { unit, class, .. }
            | MonsterSpawn::NearLevel { unit, class, .. } => {
                (class, self.pos.get(&unit).copied().unwrap_or((0, 0)))
            }
        };
        Some(self.add(FUnit::new(UnitType::Monster, class), at))
    }
    fn place_unit_flag(&mut self, u: usize, r: Option<usize>, at: (i32, i32), a: i32) -> bool {
        self.log(format!("place {u} {r:?} {at:?} {a}"));
        self.pos.insert(u, at);
        true
    }
    fn component(&self, u: usize, k: usize) -> i32 {
        self.components.get(&(u, k)).copied().unwrap_or(0)
    }
}

/// A combat entry for the pair (a, d) with `record`, as `start_combat`
/// stores it.
pub fn stored(f: &mut BodyFake, a: usize, d: usize, record: DamageRecord) {
    let e = CombatEntry {
        attacker: crate::combat::CombatWorld::ident(&f.c, a),
        defender: crate::combat::CombatWorld::ident(&f.c, d),
        record,
    };
    f.c.units[a].combat.push(e);
}
