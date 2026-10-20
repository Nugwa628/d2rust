use std::collections::{BTreeMap, BTreeSet, VecDeque};

use d2_data::tables::Record;

use super::catalogue::{self, seeded_offset, SRVDO_TSV, SRVHIT_TSV, SRV_DO, SRV_HIT};
use super::create::MissileParams;
use super::seams::SkillCalc;
use super::*;
use crate::rng::Seed;
use crate::tick::run_timer_events;
use crate::units::RoomId;

/// A zeroed `missiles.txt` record with the fields of an arrow-like row.
fn row() -> MissileRow {
    let mut r = MissileRow::decode(&[0u8; 420]);
    r.psrvdofunc = 1;
    r.vel = 20;
    r.maxvel = 20;
    r.range = 50;
    r.collidetype = 3;
    r.collidekill = 1;
    r.lastcollide = true;
    r.size = 1;
    r
}

#[derive(Default)]
struct Path {
    velocity: i32,
    target_unit: Option<UnitId>,
    target_point: Option<(i32, i32)>,
}

/// A fake of every seam: positions, seeds, stats and states by unit; a
/// path that always moves; scripted collision and to-hit results.
#[derive(Default)]
struct Fake {
    room: Option<RoomId>,
    next_seed: u32,
    seeds: BTreeMap<UnitId, Seed>,
    pos: BTreeMap<UnitId, (i32, i32)>,
    stats: BTreeMap<(UnitId, u16), i32>,
    states: BTreeSet<(UnitId, u16)>,
    flags: BTreeMap<UnitId, u32>,
    hostile_off: bool,
    town: BTreeSet<RoomId>,
    no_path: bool,
    paths: BTreeMap<UnitId, Path>,
    stop_moving: bool,
    word: u16,
    crossed: Vec<(i32, i32)>,
    masks: BTreeMap<(i32, i32), u16>,
    units: BTreeMap<(i32, i32), Vec<UnitId>>,
    hits: VecDeque<bool>,
    good: BTreeSet<UnitId>,
    log: Vec<String>,
    /// Step-9 searches (calls of `crossed_subtiles`).
    probes: std::cell::Cell<u32>,
    /// Allocation fails.
    alloc_fail: bool,
    /// Unit flags a new missile starts with.
    alloc_flags: u32,
    /// Modes handed to allocation.
    alloc_modes: Vec<u8>,
    /// `find_room` finds nothing.
    no_room: bool,
    /// The start room of the last `find_room`.
    room_near: std::cell::Cell<Option<RoomId>>,
    /// Path set-up, stat list, init callback and damage setup calls, in
    /// order.
    calls: Vec<String>,
    /// Sizes handed to `collision_mask`.
    sizes: std::cell::RefCell<Vec<i32>>,
    /// Damage setup sets unit flag bit 3 on the missile.
    setup_sets_valid: bool,
    /// Units that are hirelings.
    hirelings: BTreeSet<UnitId>,
    /// The server bodies' seams (`MissileBodies`, `tests/r9.rs`).
    mb: Bodies,
    /// The rules (`tests/mod_missiles.rs` installs a mod game's).
    rules: crate::rules::Ruleset,
}

/// Scripted answers of the server bodies' seams.
#[derive(Default)]
struct Bodies {
    /// `missile_calc` result.
    calc: i32,
    /// Skills that exist, with (calc1, calc2, aurarange, auralen, calc4).
    skills: BTreeMap<i32, [i32; 5]>,
    new_step: bool,
    target: Option<UnitId>,
    frames: BTreeMap<UnitId, i32>,
    dead: BTreeSet<UnitId>,
    /// `area_units` answer.
    area: Vec<UnitId>,
    /// Raw skills columns by (skill, code): `Param(n)` = n, aura filter
    /// 20, aura target state 21, pet type 22.
    fields: BTreeMap<(i32, u8), i32>,
    states_count: i32,
    overlay_count: i32,
    pet_types: i32,
    max_life: BTreeMap<UnitId, i32>,
    max_mana: BTreeMap<UnitId, i32>,
    pets: BTreeSet<UnitId>,
    allies: BTreeSet<UnitId>,
    demons: BTreeSet<UnitId>,
    undead: BTreeSet<UnitId>,
    large: BTreeSet<UnitId>,
    target_pos: Option<(i32, i32)>,
    /// Line tests that hit, by (from, to).
    walls: BTreeSet<((i32, i32), (i32, i32))>,
    room_seed: Option<Seed>,
    phys: (i32, i32),
    elem_len: i32,
    entry_param1: Option<i32>,
    /// State lists by (unit, state) → expiry.
    lists: BTreeMap<(UnitId, i32), i32>,
    summon: Option<SummonClass>,
    summoned: Option<UnitId>,
    floor: bool,
    mon_list: Vec<i32>,
    spawnable: BTreeMap<i32, bool>,
    monstats: i32,
    modes: BTreeMap<UnitId, i32>,
    frame_cnt1: Option<i32>,
    rects: BTreeMap<RoomId, (i32, i32, i32, i32)>,
    quest_open: bool,
    accept: bool,
    apply_ok: bool,
}

impl Fake {
    fn unit_flags(&mut self, u: UnitId) {
        self.flags
            .insert(u, unit_flag::CAN_BE_ATTACKED | unit_flag::IS_VALID_TARGET);
    }
    fn logged(&self, prefix: &str) -> usize {
        self.log.iter().filter(|s| s.starts_with(prefix)).count()
    }
}

impl MissileUnits for Fake {
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    fn alloc_missile(
        &mut self,
        game: &mut Game,
        class: u16,
        x: i32,
        y: i32,
        room: RoomId,
        mode: u8,
    ) -> Option<UnitId> {
        if self.alloc_fail {
            return None;
        }
        let m = game.spawn_unit(UnitType::Missile, Some(room), false).ok()?;
        self.alloc_modes.push(mode);
        if self.alloc_flags != 0 {
            self.flags.insert(m, self.alloc_flags);
        }
        self.next_seed += 1;
        self.seeds.insert(m, Seed::init_low(self.next_seed));
        self.pos.insert(m, (x, y));
        self.log.push(format!("alloc {class}"));
        Some(m)
    }
    fn remove_unit(&mut self, game: &mut Game, unit: UnitId) {
        self.log.push(format!("remove {}", unit.0));
        game.remove_unit(unit).unwrap();
    }
    fn seed(&mut self, unit: UnitId) -> &mut Seed {
        self.seeds.entry(unit).or_default()
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or((0, 0))
    }
    fn size(&self, _: UnitId) -> i32 {
        1
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.stats.get(&(unit, stat)).copied().unwrap_or(0)
    }
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.stat(unit, stat)
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.stats.insert((unit, stat), value);
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.states.contains(&(unit, state))
    }
    fn state_stat(&self, unit: UnitId, state: u16, stat: u16) -> Option<i32> {
        self.has_state(unit, state).then(|| self.stat(unit, stat))
    }
    fn alloc_stat_list(&mut self, _: UnitId) {
        self.calls.push("statlist".into());
    }
    fn unit_flag(&self, unit: UnitId, bit: u32) -> bool {
        self.flags.get(&unit).copied().unwrap_or(0) & bit != 0
    }
    fn set_unit_flag(&mut self, unit: UnitId, bit: u32, on: bool) {
        let f = self.flags.entry(unit).or_default();
        if on {
            *f |= bit;
        } else {
            *f &= !bit;
        }
    }
    fn may_attack(&self, a: UnitId, d: UnitId) -> bool {
        !self.hostile_off && a != d
    }
    fn alignment(&self, unit: UnitId) -> u8 {
        if self.good.contains(&unit) {
            2
        } else {
            0
        }
    }
    fn is_hireling(&self, unit: UnitId) -> bool {
        self.hirelings.contains(&unit)
    }
    fn apply_justhit(&mut self, _: &mut Game, unit: UnitId, expire: i32) {
        self.log.push(format!("justhit {} {expire}", unit.0));
    }
}

impl MissilePath for Fake {
    fn has_path(&self, _: UnitId) -> bool {
        !self.no_path
    }
    fn set_velocity(&mut self, unit: UnitId, v: i32) {
        self.calls.push(format!("vel {v}"));
        self.paths.entry(unit).or_default().velocity = v;
    }
    fn velocity(&self, unit: UnitId) -> i32 {
        self.paths.get(&unit).map_or(0, |p| p.velocity)
    }
    fn set_target_unit(&mut self, unit: UnitId, target: UnitId) {
        self.calls.push(format!("tunit {}", target.0));
        self.paths.entry(unit).or_default().target_unit = Some(target);
    }
    fn set_target_point(&mut self, unit: UnitId, x: i32, y: i32) {
        self.calls.push(format!("tpoint {x} {y}"));
        self.paths.entry(unit).or_default().target_point = Some((x, y));
    }
    fn set_footprint_mask(&mut self, _: UnitId, mask: u16) {
        self.calls.push(format!("foot {mask:#x}"));
    }
    fn set_move_mask(&mut self, _: UnitId, mask: u16) {
        self.calls.push(format!("move {mask:#x}"));
    }
    fn build(&mut self, _: &mut Game, _: UnitId) {
        self.calls.push("build".into());
    }
    fn set_acceleration(&mut self, _: UnitId, accel: i32, max: i32) {
        self.calls.push(format!("accel {accel} {max}"));
    }
    fn target_distance(&self, _: UnitId) -> i32 {
        10
    }
    fn step(&mut self, _: &mut Game, _: UnitId) -> bool {
        !self.stop_moving
    }
    fn collision_word(&self, _: &Game, _: UnitId) -> u16 {
        self.word
    }
    fn crossed_subtiles(&self, _: UnitId) -> Vec<(i32, i32)> {
        self.probes.set(self.probes.get() + 1);
        self.crossed.clone()
    }
}

impl MissileRooms for Fake {
    fn find_room(&self, _: &Game, near: RoomId, _: i32, _: i32) -> Option<RoomId> {
        self.room_near.set(Some(near));
        (!self.no_room).then_some(near)
    }
    fn in_town(&self, _: &Game, room: RoomId) -> bool {
        self.town.contains(&room)
    }
    fn collision_mask(&self, _: &Game, _: RoomId, x: i32, y: i32, size: i32, mask: u16) -> u16 {
        self.sizes.borrow_mut().push(size);
        self.masks.get(&(x, y)).copied().unwrap_or(0) & mask
    }
    fn collision_at(&self, _: &Game, _: RoomId, x: i32, y: i32, mask: u16) -> u16 {
        self.masks.get(&(x, y)).copied().unwrap_or(0) & mask
    }
    fn clear_footprint(&mut self, _: &mut Game, unit: UnitId) {
        self.log.push(format!("clear {}", unit.0));
    }
    fn units_at(&self, _: &Game, _: RoomId, x: i32, y: i32, _: i32) -> Vec<UnitId> {
        self.units.get(&(x, y)).cloned().unwrap_or_default()
    }
}

impl MissileCombat for Fake {
    fn damage_setup(
        &mut self,
        _: &mut Game,
        owner: UnitId,
        origin: Option<UnitId>,
        missile: UnitId,
        _: i32,
        level: i32,
    ) -> u32 {
        self.log.push("setup".into());
        let origin = origin.map(|o| o.0);
        self.calls.push(format!(
            "setup {} {origin:?} {} {level}",
            owner.0, missile.0
        ));
        if self.setup_sets_valid {
            self.set_unit_flag(missile, unit_flag::IS_VALID_TARGET, true);
        }
        0
    }
    fn hit_test(&mut self, _: &mut Game, _: UnitId, _: UnitId, _: i32) -> bool {
        self.hits.pop_front().unwrap_or(true)
    }
    #[allow(clippy::too_many_arguments)]
    fn apply_damage(
        &mut self,
        _: &mut Game,
        _: &mut MissileStore,
        _: Option<UnitId>,
        _: UnitId,
        unit: UnitId,
        d: &mut Damage,
        _: Option<&MissileData>,
    ) {
        self.log.push(format!("damage {} {}", unit.0, d.phys));
        self.log.push(format!(
            "lengths cold {} freeze {}",
            d.cold_length, d.freeze_length
        ));
        self.log
            .push(format!("record result {:#x}", d.record_result));
        self.log
            .push(format!("stun {} class {:?}", d.stun_length, d.hit_class));
        self.log.push(format!("fire {}", d.fire));
    }
    fn hit_by_missile_event(&mut self, _: &mut Game, _: UnitId, unit: Option<UnitId>) {
        self.log.push(format!("event0 {:?}", unit.map(|u| u.0)));
    }
    fn target_damage_bonus(&self, _: UnitId, _: UnitId) -> i32 {
        0
    }
    fn add_target_ac(&mut self, _: &mut Game, unit: UnitId, delta: i32) {
        self.log.push(format!("ac {} {delta}", unit.0));
    }
}

impl MissileHooks for Fake {
    fn init_callback(&mut self, _: &mut Game, _: UnitId, cb: u32, arg: u32) {
        self.calls.push(format!("init {cb} {arg}"));
    }
    fn unique_mod_missile(&mut self, _: &mut Game, _: &mut MissileStore, _: UnitId, _: UnitId) {
        self.log.push("umod".into());
    }
}

impl MissileBodies for Fake {
    fn missile_calc(
        &mut self,
        _: &mut Game,
        m: UnitId,
        _: Option<UnitId>,
        field: u32,
        _: i32,
        level: i32,
    ) -> i32 {
        self.log.push(format!("calc {} {field} {level}", m.0));
        self.mb.calc
    }
    fn skill_exists(&self, skill: i32) -> bool {
        self.mb.skills.contains_key(&skill)
    }
    fn skill_calc(
        &mut self,
        _: &mut Game,
        _: Option<UnitId>,
        skill: i32,
        calc: SkillCalc,
        _: i32,
    ) -> i32 {
        let v = self.mb.skills.get(&skill).copied().unwrap_or_default();
        self.log.push(format!("skillcalc {calc:?}"));
        v[calc as usize]
    }
    fn path_new_step(&self, _: UnitId) -> bool {
        self.mb.new_step
    }
    fn path_target(&mut self, _: &Game, _: UnitId) -> Option<UnitId> {
        self.mb.target
    }
    fn or_collision(&mut self, _: &mut Game, _: RoomId, x: i32, y: i32, bits: u16) {
        self.log.push(format!("or {x} {y} {bits:#x}"));
    }
    fn stamp_collision(&mut self, _: &mut Game, unit: UnitId, bits: u16) {
        self.log.push(format!("stamp {} {bits:#x}", unit.0));
    }
    fn anim_frame(&self, unit: UnitId) -> i32 {
        self.mb.frames.get(&unit).copied().unwrap_or(0)
    }
    fn set_anim_frame(&mut self, unit: UnitId, v: i32) {
        self.mb.frames.insert(unit, v);
    }
    fn is_dead(&self, unit: UnitId) -> bool {
        self.mb.dead.contains(&unit)
    }
    fn area_units(
        &mut self,
        _: &mut Game,
        _: UnitId,
        at: (i32, i32),
        r: i32,
        f: u32,
    ) -> Vec<UnitId> {
        self.log.push(format!("scan {at:?} {r} {f:#x}"));
        self.mb.area.clone()
    }
    fn area_hit(
        &mut self,
        _: &mut Game,
        _: UnitId,
        unit: UnitId,
        rec: &crate::combat::DamageRecord,
    ) {
        self.log.push(format!(
            "areahit {} {} {} {:#x}",
            unit.0, rec.fire, rec.cold_len, rec.result
        ));
    }
    fn scan_units(
        &mut self,
        _: &mut Game,
        _: UnitId,
        at: (i32, i32),
        r: i32,
        f: u32,
        noaura: bool,
    ) -> Vec<UnitId> {
        self.log.push(format!("scan {at:?} {r} {f:#x} {noaura}"));
        self.mb.area.clone()
    }
    fn skill_field(&self, skill: i32, field: SkillField) -> i32 {
        let code = match field {
            SkillField::Param(n) => n,
            SkillField::AuraFilter => 20,
            SkillField::AuraTargetState => 21,
            SkillField::PetType => 22,
        };
        self.mb.fields.get(&(skill, code)).copied().unwrap_or(0)
    }
    fn states_count(&self) -> i32 {
        self.mb.states_count
    }
    fn overlay_count(&self) -> i32 {
        self.mb.overlay_count
    }
    fn pet_type_count(&self) -> i32 {
        self.mb.pet_types
    }
    fn skill_phys(&mut self, _: &mut Game, _: Option<UnitId>, _: i32, _: i32) -> (i32, i32) {
        self.mb.phys
    }
    fn skill_elem_len(&mut self, _: &mut Game, _: UnitId, _: i32, _: i32) -> i32 {
        self.mb.elem_len
    }
    fn skill_entry_param1(&self, _: UnitId, _: i32) -> Option<i32> {
        self.mb.entry_param1
    }
    fn max_life(&self, unit: UnitId) -> i32 {
        self.mb.max_life.get(&unit).copied().unwrap_or(0)
    }
    fn max_mana(&self, unit: UnitId) -> i32 {
        self.mb.max_mana.get(&unit).copied().unwrap_or(0)
    }
    fn overlay(&mut self, _: &mut Game, unit: UnitId, overlay: i32) {
        self.log.push(format!("overlay {} {overlay}", unit.0));
    }
    fn is_large_monster(&self, unit: UnitId) -> bool {
        self.mb.large.contains(&unit)
    }
    fn is_demon(&self, unit: UnitId) -> bool {
        self.mb.demons.contains(&unit)
    }
    fn is_undead(&self, unit: UnitId) -> bool {
        self.mb.undead.contains(&unit)
    }
    fn is_pet(&self, _: &Game, _: UnitId, unit: UnitId, _: i32) -> bool {
        self.mb.pets.contains(&unit)
    }
    fn is_ally(&self, _: &Game, _: UnitId, unit: UnitId, _: i32) -> bool {
        self.mb.allies.contains(&unit)
    }
    fn ally_test(&mut self, _: &mut Game, _: UnitId, unit: UnitId) -> bool {
        self.mb.allies.contains(&unit)
    }
    fn accepts(&self, _: &Game, _: UnitId, _: UnitId, _: u32) -> bool {
        self.mb.accept
    }
    fn target_position(&mut self, _: &Game, _: UnitId) -> Option<(i32, i32)> {
        self.mb.target_pos
    }
    fn path_target_point(&self, unit: UnitId) -> (i32, i32) {
        self.paths
            .get(&unit)
            .and_then(|p| p.target_point)
            .unwrap_or((0, 0))
    }
    fn set_path_type(&mut self, unit: UnitId, ty: i32) {
        self.log.push(format!("ptype {} {ty}", unit.0));
    }
    fn set_path_distance(&mut self, unit: UnitId, d: i32) {
        self.log.push(format!("pdist {} {d}", unit.0));
    }
    fn path_teleport(&mut self, _: &mut Game, unit: UnitId, _: Option<RoomId>, x: i32, y: i32) {
        self.log.push(format!("teleport {} {x} {y}", unit.0));
        self.pos.insert(unit, (x, y));
    }
    fn refresh_room(&mut self, _: &mut Game, _: RoomId) {
        self.log.push("refresh".into());
    }
    fn line_hits(&self, _: &Game, _: RoomId, from: (i32, i32), to: (i32, i32), _: u16) -> bool {
        self.mb.walls.contains(&(from, to))
    }
    fn room_seed(&mut self, _: &mut Game, _: RoomId) -> Option<&mut Seed> {
        self.mb.room_seed.as_mut()
    }
    fn skill_srv_do(
        &mut self,
        _: &mut Game,
        _: &mut MissileStore,
        caster: UnitId,
        index: i32,
        skill: i32,
        level: i32,
    ) {
        self.log
            .push(format!("skilldo {} {index} {skill} {level}", caster.0));
    }
    fn shout_state(&mut self, _: &mut Game, unit: UnitId, _: UnitId, _: i32, _: i32) {
        self.log.push(format!("shout {}", unit.0));
    }
    fn state_list_expiry(&self, unit: UnitId, s: i32) -> Option<i32> {
        self.mb.lists.get(&(unit, s)).copied()
    }
    fn new_state_list(
        &mut self,
        _: &mut Game,
        unit: UnitId,
        s: i32,
        expire: i32,
        _: UnitId,
    ) -> bool {
        self.log.push(format!("newlist {} {s} {expire}", unit.0));
        self.mb.lists.insert((unit, s), expire);
        true
    }
    fn aura_fill(&mut self, _: &mut Game, unit: UnitId, s: i32, _: i32, _: i32) {
        self.log.push(format!("aurafill {} {s}", unit.0));
    }
    fn mark_state_changed(&mut self, unit: UnitId, s: i32) {
        self.log.push(format!("changed {} {s}", unit.0));
    }
    fn set_state_list_expiry(&mut self, unit: UnitId, s: i32, expire: i32) {
        self.log.push(format!("expiry {} {s} {expire}", unit.0));
        self.mb.lists.insert((unit, s), expire);
    }
    fn apply_state(
        &mut self,
        _: &mut Game,
        _: UnitId,
        unit: UnitId,
        _: i32,
        _: i32,
        duration: i32,
        s: i32,
    ) -> bool {
        self.log
            .push(format!("applystate {} {s} {duration}", unit.0));
        self.mb.apply_ok
    }
    fn terror(&mut self, _: &mut Game, source: UnitId, unit: UnitId, _: i32, a: i32, b: i32) {
        self.log
            .push(format!("terror {} {} {a} {b}", source.0, unit.0));
    }
    fn summon_class(&mut self, _: &mut Game, _: UnitId, _: i32, _: i32) -> SummonClass {
        self.mb.summon.unwrap_or(SummonClass { class: -1, mode: 0 })
    }
    fn summon_spawn(
        &mut self,
        _: &mut Game,
        _: UnitId,
        class: i32,
        mode: i32,
        at: (i32, i32),
        pet_type: i32,
    ) -> Option<UnitId> {
        self.log
            .push(format!("summon {class} {mode} {at:?} {pet_type}"));
        self.mb.summoned
    }
    fn bind_bone_wall_piece(
        &mut self,
        _: &mut Game,
        _: UnitId,
        anchor: UnitId,
        piece: UnitId,
        _: i32,
        _: i32,
    ) {
        self.log.push(format!("bind {} {}", anchor.0, piece.0));
    }
    fn create_portal(
        &mut self,
        _: &mut Game,
        _: Option<UnitId>,
        _: Option<RoomId>,
        at: (i32, i32),
        level: i32,
        class: i32,
    ) {
        self.log.push(format!("portal {at:?} {level} {class}"));
    }
    fn unit_sound(&mut self, _: &mut Game, unit: UnitId, id: i32) {
        self.log.push(format!("sound {} {id:#x}", unit.0));
    }
    fn chest_drop(&mut self, _: &mut Game, chest: UnitId) {
        self.log.push(format!("chest {}", chest.0));
    }
    fn floor_drop_spot(
        &mut self,
        _: &Game,
        room: RoomId,
        at: (i32, i32),
    ) -> Option<(RoomId, i32, i32)> {
        self.mb.floor.then_some((room, at.0, at.1))
    }
    fn create_gold(&mut self, _: &mut Game, _: UnitId, _: RoomId, at: (i32, i32)) {
        self.log.push(format!("gold {at:?}"));
    }
    fn unit_mode(&self, unit: UnitId) -> i32 {
        self.mb.modes.get(&unit).copied().unwrap_or(0)
    }
    fn set_unit_mode(&mut self, unit: UnitId, mode: i32) {
        self.mb.modes.insert(unit, mode);
    }
    fn object_frame_cnt1(&self, _: UnitId) -> Option<i32> {
        self.mb.frame_cnt1
    }
    fn room_subtiles(&self, _: &Game, room: RoomId) -> Option<(i32, i32, i32, i32)> {
        self.mb.rects.get(&room).copied()
    }
    fn redemption_effect(
        &mut self,
        _: &mut Game,
        _: UnitId,
        unit: UnitId,
        skill: i32,
        level: i32,
        last: bool,
    ) {
        self.log
            .push(format!("redeem {} {skill} {level} {last}", unit.0));
    }
    fn level_mon_list(&self, _: &Game, _: RoomId) -> Vec<i32> {
        self.mb.mon_list.clone()
    }
    fn monster_is_spawn(&self, class: i32) -> Option<bool> {
        self.mb.spawnable.get(&class).copied()
    }
    fn create_monster(&mut self, _: &mut Game, _: RoomId, class: i32, at: (i32, i32)) -> bool {
        self.log.push(format!("monster {class} {at:?}"));
        true
    }
    fn monstats_count(&self) -> i32 {
        self.mb.monstats
    }
    fn spawn_monster(
        &mut self,
        _: &mut Game,
        _: Option<RoomId>,
        class: i32,
        at: (i32, i32),
        mode: i32,
    ) {
        self.log.push(format!("spawn {class} {at:?} {mode}"));
    }
    fn rabies_poison(&mut self, _: &mut Game, _: UnitId, unit: UnitId, t: i32, _: i32, _: i32) {
        self.log.push(format!("rabies {} {t}", unit.0));
    }
    fn quest_test(&self, _: &Game, id: i32) -> bool {
        id == 36 && self.mb.quest_open
    }
    fn spawn_tyrael(&mut self, _: &mut Game, _: Option<RoomId>, _: UnitId) {
        self.log.push("tyrael".into());
    }
}

/// A game with one active room holding a player owner at (100, 100) and
/// a monster at (110, 100).
struct World {
    game: Game,
    fake: Fake,
    store: MissileStore,
    tables: Vec<MissileRow>,
    owner: UnitId,
    monster: UnitId,
}

impl World {
    fn new(r: MissileRow) -> Self {
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let room = game.lists.create_room(0).unwrap();
        let owner = game.spawn_unit(UnitType::Player, Some(room), true).unwrap();
        let monster = game
            .spawn_unit(UnitType::Monster, Some(room), false)
            .unwrap();
        let mut fake = Fake {
            room: Some(room),
            ..Fake::default()
        };
        fake.pos.insert(owner, (100, 100));
        fake.pos.insert(monster, (110, 100));
        fake.unit_flags(owner);
        fake.unit_flags(monster);
        Self {
            game,
            fake,
            store: MissileStore::new(),
            tables: vec![r],
            owner,
            monster,
        }
    }

    fn params(&self) -> MissileParams {
        MissileParams {
            owner: Some(self.owner),
            origin: Some(self.owner),
            class: 0,
            flags: param_flags::TARGET_ABSOLUTE,
            target_x: 110,
            target_y: 100,
            ..MissileParams::default()
        }
    }

    fn create(&mut self, p: &MissileParams) -> Option<UnitId> {
        let mut cx = Ctx {
            tables: &self.tables,
            store: &mut self.store,
            world: &mut self.fake,
        };
        create_missile(&mut self.game, &mut cx, p)
    }

    /// One frame: frame += 1, then the timer queue.
    fn frame(&mut self) {
        self.game.frame += 1;
        struct Nothing;
        impl EventDispatch for Nothing {
            fn run_event(&mut self, _: &mut Game, _: &TimerRun) {}
        }
        let mut d = MissileDispatch {
            cx: Ctx {
                tables: &self.tables,
                store: &mut self.store,
                world: &mut self.fake,
            },
            next: &mut Nothing,
        };
        run_timer_events(&mut self.game, &mut d);
    }

    fn alive(&self, m: UnitId) -> bool {
        self.store.get(m).is_some()
    }

    /// Runs frames until the missile is gone; returns the run count.
    fn lifetime(&mut self, m: UnitId) -> i32 {
        let mut n = 0;
        while self.alive(m) {
            self.frame();
            n += 1;
            assert!(n < 1000);
        }
        n
    }

    fn hit(&mut self, m: UnitId, unit: Option<UnitId>, a4: bool) -> i32 {
        let mut cx = Ctx {
            tables: &self.tables,
            store: &mut self.store,
            world: &mut self.fake,
        };
        hit_handler(&mut self.game, &mut cx, m, unit, a4)
    }
}

// ---- creation ---------------------------------------------------

#[test]
fn creation_velocity_vectors() {
    let p = |level| MissileParams {
        level,
        ..MissileParams::default()
    };
    let mut r = row();
    r.vel = 20;
    assert_eq!(creation_velocity(&r, &p(5), None), 3840);
    r.vel = 10;
    r.vellev = 8;
    assert_eq!(creation_velocity(&r, &p(3), None), 2496);
    r.vellev = 7;
    assert_eq!(creation_velocity(&r, &p(3), None), 2304);
    r.vel = 24;
    r.vellev = 0;
    assert_eq!(creation_velocity(&r, &p(0), Some(50)), 2304);
    // The speeds.
    r.vel = 8;
    assert_eq!(creation_velocity(&r, &p(0), None), 1536);
    r.vel = 24;
    assert_eq!(creation_velocity(&r, &p(0), None), 4608);
    r.vel = 10;
    r.vellev = 8;
    assert_eq!(creation_velocity(&r, &p(0), None), 1920);
    // Given velocity: << 8 unless flag 0x10.
    let mut q = p(0);
    q.flags = param_flags::VELOCITY;
    q.velocity = 4;
    assert_eq!(creation_velocity(&r, &q, None), 768);
    q.flags |= param_flags::VELOCITY_FIXED;
    q.velocity = 1024;
    assert_eq!(creation_velocity(&r, &q, None), 768);
}

#[test]
fn creation_slow_uses_state_87_stat_161() {
    let mut r = row();
    r.vel = 24;
    r.canslow = true;
    let mut w = World::new(r);
    w.fake.states.insert((w.owner, state::SLOWMISSILES));
    w.fake.stats.insert((w.owner, stat::SKILL_HANDOFATHENA), 50);
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.velocity(m), 2304);
}

#[test]
fn creation_target_at_start_moves_one_subtile() {
    let mut w = World::new(row());
    let mut p = w.params();
    p.flags = 0; // target = start
    let m = w.create(&p).unwrap();
    assert_eq!(w.fake.paths[&m].target_point, Some((101, 101)));
}

#[test]
fn creation_target_on_owner_subtile_is_dropped() {
    let mut w = World::new(row());
    let mut p = w.params();
    p.target = Some(w.monster);
    w.fake.pos.insert(w.monster, (100, 100));
    let m = w.create(&p).unwrap();
    assert_eq!(w.fake.paths[&m].target_unit, None);
    assert_eq!(w.fake.paths[&m].target_point, Some((111, 101)));
}

#[test]
fn creation_fails_100_subtiles_away() {
    let mut w = World::new(row());
    let mut p = w.params();
    p.target_x = 200;
    assert_eq!(w.create(&p), None);
    assert_eq!(w.fake.logged("alloc"), 0);
    p.target_x = 199;
    assert!(w.create(&p).is_some());
}

#[test]
fn creation_fails_without_owner_or_bad_class() {
    let mut w = World::new(row());
    let mut p = w.params();
    p.owner = None;
    assert_eq!(w.create(&p), None);
    let mut p = w.params();
    p.class = 1;
    assert_eq!(w.create(&p), None);
    p.class = -1;
    assert_eq!(w.create(&p), None);
}

#[test]
fn creation_without_path_leaves_the_unit_with_its_event() {
    let mut w = World::new(row());
    w.fake.no_path = true;
    assert_eq!(w.create(&w.params()), None);
    let ms = w.game.lists.units_of_type(UnitType::Missile);
    assert_eq!(ms.len(), 1);
    assert_eq!(w.game.timers.unit_timers(ms[0]).len(), 1);
}

#[test]
fn creation_frames_activate_and_order() {
    let mut r = row();
    r.range = 40;
    r.levrange = 2;
    r.activate = 3;
    let mut w = World::new(r);
    let mut p = w.params();
    p.level = 3;
    p.skill = -5;
    let m = w.create(&p).unwrap();
    let d = w.store.get(m).unwrap();
    assert_eq!((d.total, d.current, d.activate), (46, 46, 43));
    assert_eq!((d.skill, d.level), (0, 3));
    assert_eq!(d.last_collided.map(|l| l.ty), Some(UnitType::Player));
    assert_eq!(w.fake.log, ["alloc 0", "setup"]);
    // Every-tick type-0 event.
    let t = w.game.timers.unit_timers(m);
    assert_eq!(t.len(), 1);
    assert_eq!(w.game.timers.event(t[0]).map(|e| e.0), Some(0));
    assert_eq!(w.game.timers.expire(t[0]), Some(-1));
    assert_eq!(w.fake.stat(m, stat::DAMAGE_FRAMERATE), 0);
}

#[test]
fn creation_start_frame_shortens_the_frames() {
    // Flag 0x200: frames −= start frame before the activate frame. The
    // animation frame is not stored (TODO in
    // `create.rs`).
    let mut r = row();
    r.range = 40;
    r.activate = 3;
    let mut w = World::new(r);
    let mut p = w.params();
    p.flags |= param_flags::START_FRAME;
    p.start_frame = 6;
    let m = w.create(&p).unwrap();
    let d = w.store.get(m).unwrap();
    assert_eq!((d.total, d.current, d.activate), (34, 34, 31));
}

#[test]
fn creation_runs_the_skills_init_callbacks_after_the_stat_list() {
    // Step 21 on the fake seams: the jitter caps 100
    // frames at 77, re-seeds from the path target x 110 + a 4, sets type
    // 10 and step counts 77, and builds, after the stat list (step 20)
    // and before the damage setup (step 23).
    use crate::skills::use_::bodies::init_cb;
    let mut w = World::new(row());
    let mut p = w.params();
    p.flags |= param_flags::RANGE;
    p.range = 100;
    p.init = Some((init_cb::JITTER, 4));
    let m = w.create(&p).unwrap();
    let d = w.store.get(m).unwrap();
    assert_eq!((d.total, d.current), (77, 77));
    assert_eq!(*w.fake.seed(m), Seed::init_low(114));
    let i = w.fake.calls.iter().position(|c| c == "statlist").unwrap();
    assert_eq!(w.fake.calls[i + 1], "build");
    assert!(w.fake.log.contains(&format!("ptype {} 10", m.0)));
    assert!(w.fake.log.contains(&format!("pdist {} 77", m.0)));
    assert!(!w.fake.calls.iter().any(|c| c.starts_with("init")));
    // M08: an id no rule names still goes to the host hook, unchanged.
    let mut w = World::new(row());
    let mut p = w.params();
    p.init = Some((0x0012_3456, 9));
    let m = w.create(&p).unwrap();
    assert_eq!(w.store.get(m).unwrap().total, 50);
    assert!(w.fake.calls.contains(&format!("init {} 9", 0x0012_3456)));
}

#[test]
fn creation_frames_from_distance() {
    assert_eq!(frames_from_distance(10, 3840), (10u32 << 16) / (3840 << 4));
    assert_eq!(frames_from_distance(0, 3840), (1u32 << 16) / (3840 << 4));
    assert_eq!(frames_from_distance(10, 0), 0);
    // A negative v divides by a huge unsigned value.
    assert_eq!(frames_from_distance(10, -1), 0);
}

#[test]
fn creation_monster_owner_runs_unique_mod_hook() {
    let mut w = World::new(row());
    let mut p = w.params();
    p.owner = Some(w.monster);
    p.origin = Some(w.monster);
    p.target_x = 100;
    w.create(&p).unwrap();
    assert_eq!(w.fake.logged("umod"), 1);
}

// ---- pierce -----------------------------------------------------

#[test]
fn pierce_vectors() {
    assert_eq!(pierce_count(67, 0), 4);
    assert_eq!(pierce_count(66, 0), 0);
    assert_eq!(pierce_count(52, 1), 3);
    assert_eq!(pierce_count(80, 5), 0);
}

#[test]
fn pierce_set_at_creation_and_used_at_hits() {
    let mut r = row();
    r.pierce = true;
    let mut w = World::new(r);
    w.fake.stats.insert((w.owner, stat::SKILL_PIERCE), 67);
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.stat(m, stat::PIERCE_IDX), 4);
    // A hit with CollideKill keeps the missile while pierce_idx > 0.
    let mon = w.monster;
    assert_eq!(w.hit(m, Some(mon), false), 1);
    assert_eq!(w.fake.stat(m, stat::PIERCE_IDX), 3);
    // The owner losing its pierce stat stops piercing.
    w.fake.stats.insert((w.owner, stat::SKILL_PIERCE), 0);
    w.store.get_mut(m).unwrap().last_collided = None;
    assert_eq!(w.hit(m, Some(mon), false), 2);
}

#[test]
fn pierce_not_set_without_owner_stat() {
    let mut r = row();
    r.pierce = true;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    assert!(!w.fake.stats.contains_key(&(m, stat::PIERCE_IDX)));
}

// ---- velocity -------------------------------------------------

#[test]
fn acceleration_vector() {
    let mut v = PathVelocity {
        velocity: 1000,
        max: 4 << 8,
        accel: 10,
        counter: 0,
    };
    let mut seen = Vec::new();
    for _ in 0..16 {
        v.advance();
        seen.push(v.velocity);
    }
    assert_eq!(&seen[..4], &[1000; 4]);
    assert_eq!(seen[4], 1010);
    assert_eq!(seen[9], 1020);
    assert_eq!(seen[14], 1024);
    assert_eq!(v.accel, 0);
    assert_eq!(seen[15], 1024);
    // Below 0 → 0.
    let mut v = PathVelocity {
        velocity: 5,
        max: 100,
        accel: -10,
        counter: 4,
    };
    v.advance();
    assert_eq!(v.velocity, 0);
    // Step vector: ((3840 × 0x400) >> 6) × 4096 >> 12.
    let v = PathVelocity {
        velocity: 3840,
        ..PathVelocity::default()
    };
    assert_eq!(v.step_component(4096), 61440);
    assert_eq!(v.step_component(-4096), -61440);
}

// ---- lifetime ---------------------------------------------------

#[test]
fn lifetime_is_range_plus_levrange_runs() {
    let mut r = row();
    r.range = 40;
    r.levrange = 2;
    let mut w = World::new(r);
    let mut p = w.params();
    p.level = 3;
    w.game.frame = 677;
    let m = w.create(&p).unwrap();
    // First run in F + 1, removed in F + 46.
    assert_eq!(w.lifetime(m), 46);
    assert_eq!(w.game.frame, 677 + 46);
    assert!(w.game.lists.unit(m).is_none());
    // The expiry hit ran with no unit; no damage.
    assert_eq!(w.fake.logged("event0 None"), 1);
    assert_eq!(w.fake.logged("damage"), 0);
}

#[test]
fn lifetime_is_real_time_at_a_faster_rate() {
    // The same missile at 60 Hz lives the same real time: 46 base ticks
    // (1.84 s) are 110 ticks (46 · 60 / 25 = 110.4, nearest).
    let mut r = row();
    r.range = 40;
    r.levrange = 2;
    let mut w = World::new(r);
    w.game.tick_rate = crate::tick::TickRate::new(60).unwrap();
    let mut p = w.params();
    p.level = 3;
    w.game.frame = 677;
    let m = w.create(&p).unwrap();
    assert_eq!(w.lifetime(m), 110);
    assert!(w.game.lists.unit(m).is_none());
}

#[test]
fn recorded_full_lifetimes() {
    // Firebolt 50, shafire1 40, rogue1 40 runs (velocity rule).
    for range in [50, 40] {
        let mut r = row();
        r.range = range;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        assert_eq!(w.lifetime(m), i32::from(range));
    }
}

#[test]
fn lifetime_one_frame_or_less() {
    for range in [0, 1] {
        let mut r = row();
        r.range = range;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        assert_eq!(w.lifetime(m), 1);
    }
}

#[test]
fn collision_tested_from_activate_run() {
    let mut r = row();
    r.range = 50;
    r.activate = 3;
    let mut w = World::new(r);
    w.fake.word = coll::PLAYER;
    w.fake.crossed = vec![(5, 5)];
    let m = w.create(&w.params()).unwrap();
    let mut tested = Vec::new();
    let mut k = 0;
    while w.alive(m) {
        let before = w.fake.probes.get();
        w.frame();
        k += 1;
        if w.fake.probes.get() != before {
            tested.push(k);
        }
    }
    assert_eq!(tested.first(), Some(&3));
    assert_eq!(tested.last(), Some(&49));
    assert_eq!(k, 50);
}

#[test]
fn stopped_movement_is_an_expiry_hit() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.fake.stop_moving = true;
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(w.fake.logged("event0 None"), 1);
    assert_eq!(w.fake.logged("clear"), 1);
}

#[test]
fn blocking_word_removes_without_hit() {
    for word in [coll::WALL, coll::MISSILE_BARRIER] {
        let mut w = World::new(row());
        let m = w.create(&w.params()).unwrap();
        w.fake.word = word;
        w.frame();
        assert!(!w.alive(m));
        assert_eq!(w.fake.logged("event0"), 0);
        assert_eq!(w.fake.logged("clear"), 0);
    }
}

#[test]
fn mode_zero_and_six_never_collide() {
    for mode in [0, 6] {
        let mut r = row();
        r.collidetype = mode;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        w.fake.word = coll::MONSTER;
        w.fake.crossed = vec![(110, 100)];
        w.fake.masks.insert((110, 100), coll::MONSTER);
        w.fake.units.insert((110, 100), vec![w.monster]);
        w.frame();
        assert!(w.alive(m));
        assert_eq!(w.fake.logged("event0"), 0);
    }
}

#[test]
fn mode_four_removed_on_any_word() {
    let mut r = row();
    r.collidetype = 4;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    w.fake.word = coll::MONSTER;
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(w.fake.logged("event0"), 0);
}

/// Sets up a unit contact on run 1.
fn contact(w: &mut World) {
    w.fake.word = coll::MONSTER;
    w.fake.crossed = vec![(109, 100), (110, 100)];
    w.fake.masks.insert((110, 100), coll::MONSTER);
    w.fake.units.insert((110, 100), vec![w.monster]);
}

#[test]
fn unit_hit_damages_and_kills() {
    let mut r = row();
    r.tohit = 1;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    contact(&mut w);
    w.fake.stats.insert((m, stat::MINDAMAGE), 0x100);
    w.fake.stats.insert((m, stat::MAXDAMAGE), 0x200);
    w.frame();
    assert!(!w.alive(m));
    let mon = w.monster.0;
    assert_eq!(w.fake.logged(&format!("event0 Some({mon})")), 1);
    assert_eq!(w.fake.logged(&format!("damage {mon}")), 1);
    assert_eq!(w.fake.logged("clear"), 1);
}

#[test]
fn missed_to_hit_always_removes() {
    // Even with pierce left and CollideKill 0.
    let mut r = row();
    r.tohit = 1;
    r.collidekill = 0;
    r.pierce = true;
    let mut w = World::new(r);
    w.fake.stats.insert((w.owner, stat::SKILL_PIERCE), 67);
    let m = w.create(&w.params()).unwrap();
    w.fake.hits.push_back(false);
    let mon = w.monster;
    assert_eq!(w.hit(m, Some(mon), false), 2);
    assert_eq!(w.fake.logged("damage"), 0);
    assert_eq!(w.fake.logged("event0"), 1);
}

#[test]
fn no_collidekill_flies_on() {
    let mut r = row();
    r.collidekill = 0;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    let mon = w.monster;
    assert_eq!(w.hit(m, Some(mon), false), 1);
    assert_eq!(w.fake.logged("damage"), 1);
    // LastCollide: the same unit is ignored next.
    assert_eq!(w.hit(m, Some(mon), false), 1);
    assert_eq!(w.fake.logged("damage"), 1);
}

#[test]
fn last_collide_skips_owner_first() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.fake.word = coll::PLAYER;
    w.fake.crossed = vec![(100, 100)];
    w.fake.masks.insert((100, 100), coll::PLAYER);
    w.fake.units.insert((100, 100), vec![w.owner]);
    w.frame();
    assert!(w.alive(m));
    assert_eq!(w.fake.logged("event0"), 0);
}

#[test]
fn friendly_units_ignored_unless_collide_friend() {
    let mut w = World::new(row());
    w.fake.hostile_off = true;
    let m = w.create(&w.params()).unwrap();
    contact(&mut w);
    w.frame();
    assert!(w.alive(m));
    let mut r = row();
    r.collidefriend = 1;
    let mut w = World::new(r);
    w.fake.hostile_off = true;
    let m = w.create(&w.params()).unwrap();
    contact(&mut w);
    w.frame();
    assert!(!w.alive(m));
}

#[test]
fn mode_one_ignores_good_monsters_after_finding_them() {
    let mut r = row();
    r.collidetype = 1;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    contact(&mut w);
    w.fake.masks.insert((110, 100), coll::PLAYER);
    w.fake.good.insert(w.monster);
    w.frame();
    assert!(w.alive(m));
    assert_eq!(w.fake.logged("event0"), 0);
}

#[test]
fn barrier_on_crossed_subtile_is_expiry_hit() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.fake.word = coll::MONSTER;
    w.fake.crossed = vec![(105, 100)];
    w.fake.masks.insert((105, 100), coll::MISSILE_BARRIER);
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(w.fake.logged("event0 None"), 1);
}

#[test]
fn justhit_with_next_hit() {
    let mut r = row();
    r.nexthit = 1;
    r.nextdelay = 7;
    r.collidekill = 0;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    w.game.frame = 100;
    let mon = w.monster;
    w.hit(m, Some(mon), false);
    assert_eq!(w.fake.logged(&format!("justhit {} 107", mon.0)), 1);
    // A unit with state 86 is ignored.
    w.fake.states.insert((mon, state::JUSTHIT));
    w.store.get_mut(m).unwrap().last_collided = None;
    assert_eq!(w.hit(m, Some(mon), false), 1);
    assert_eq!(w.fake.logged("damage"), 1);
}

#[test]
fn explosion_rows_skip_direct_damage() {
    let mut r = row();
    r.explosion = true;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    let mon = w.monster;
    // c = 0 | CollideKill 1 → dies, no damage stage.
    assert_eq!(w.hit(m, Some(mon), false), 2);
    assert_eq!(w.fake.logged("damage"), 0);
}

#[test]
fn server_hit_runs_on_expiry() {
    let mut r = row();
    // Server-hit 36 (missile in air): with no unit it
    // creates `HitSubMissile1` (class 0 here) at the missile, observable
    // as a second allocation.
    r.psrvhitfunc = 36;
    r.range = 1;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.logged("alloc"), 1);
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(w.fake.logged("alloc"), 2);
    assert!(w.store.unhandled.is_empty());
}

#[test]
fn server_do_dispatch_limits() {
    // pSrvDoFunc 0 or negative: nothing happens.
    for f in [0u16, 0xFDB4, 53] {
        let mut r = row();
        r.psrvdofunc = f;
        r.range = 1;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        for _ in 0..3 {
            w.frame();
        }
        assert!(w.alive(m));
        assert!(w.store.unhandled.is_empty());
    }
    // A body runs (36, Baal FX control: frames left
    // ≤ 100 refreshes the room); a null entry is logged.
    for (f, null) in [(36u16, false), (4, true)] {
        let mut r = row();
        r.psrvdofunc = f;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        w.frame();
        if null {
            let u = Unhandled::NullSrvDo {
                index: 4,
                missile: m,
            };
            assert_eq!(w.store.unhandled, [u]);
            assert_eq!(w.fake.logged("refresh"), 0);
        } else {
            assert!(w.store.unhandled.is_empty());
            assert_eq!(w.fake.logged("refresh"), 1);
        }
    }
}

#[test]
fn town_rules() {
    // A player owner in town removes even `Town` missiles.
    let mut r = row();
    r.town = true;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    let room = w.fake.room.unwrap();
    w.fake.town.insert(room);
    w.frame();
    assert!(!w.alive(m));
    // A monster owner in town with `Town`: kept.
    let mut r = row();
    r.town = true;
    let mut w = World::new(r);
    let mut p = w.params();
    p.owner = Some(w.monster);
    p.origin = Some(w.monster);
    p.target_x = 100;
    let m = w.create(&p).unwrap();
    let room = w.fake.room.unwrap();
    w.fake.town.insert(room);
    w.frame();
    assert!(w.alive(m));
    // Without `Town`: removed.
    let mut w = World::new(row());
    let mut p = w.params();
    p.owner = Some(w.monster);
    p.origin = Some(w.monster);
    p.target_x = 100;
    let m = w.create(&p).unwrap();
    let room = w.fake.room.unwrap();
    w.fake.town.insert(room);
    w.frame();
    assert!(!w.alive(m));
}

// ---- damage -----------------------------------------------------

#[test]
fn damage_roll_vectors() {
    let mut s = Seed::init();
    assert_eq!(damage_roll(&mut s, 0x100, 0x100, 0), 0x100);
    assert_eq!(s, Seed::init(), "equal bounds draw nothing");
    let mut s = Seed::init();
    let mut t = Seed::init();
    assert_eq!(
        damage_roll(&mut s, 0x300, 0x100, 0),
        0x100 + t.roll(0x200) as i32
    );
    assert_eq!(s, t);
    let mut s = Seed::init();
    assert_eq!(damage_roll(&mut s, 0, 0x100, 0), 0);
    assert_eq!(damage_roll(&mut s, 0x100, 0, 0), 0);
    assert_eq!(s, Seed::init());
    // Mastery 50 %: 0x100 → 0x180 on both bounds.
    let mut s = Seed::init();
    assert_eq!(damage_roll(&mut s, 0x100, 0x100, 50), 0x180);
    // pct with large operands is exact.
    assert_eq!(pct(0x4000_0000, 200), i32::MIN);
    assert_eq!(pct(3_000_000, 100_000), 3_000_000_000_i64 as i32);
}

#[test]
fn damage_rolls_use_missile_seed_in_order() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    for (min, max) in [
        (stat::MINDAMAGE, stat::MAXDAMAGE),
        (stat::FIREMINDAM, stat::FIREMAXDAM),
        (stat::POISONMINDAM, stat::POISONMAXDAM),
    ] {
        w.fake.stats.insert((m, min), 10);
        w.fake.stats.insert((m, max), 20);
    }
    let seed = *w.fake.seed(m);
    let mut cx = Ctx {
        tables: &w.tables,
        store: &mut w.store,
        world: &mut w.fake,
    };
    let d = fill_damage(&mut cx, m, None);
    let mut s = seed;
    let want = [10 + s.roll(10), 10 + s.roll(10), 10 + s.roll(10)];
    assert_eq!([d.phys, d.fire, d.poison], want.map(|v| v as i32));
    assert_eq!(*w.fake.seed(m), s);
}

#[test]
fn damage_percent_and_deadly_strike() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.fake.stats.insert((m, stat::MINDAMAGE), 100);
    w.fake.stats.insert((m, stat::MAXDAMAGE), 100);
    w.fake.stats.insert((m, stat::DAMAGEPERCENT), -200);
    let mon = w.monster;
    let mut cx = Ctx {
        tables: &w.tables,
        store: &mut w.store,
        world: &mut w.fake,
    };
    assert_eq!(fill_damage(&mut cx, m, Some(mon)).phys, 10);
    cx.world.stats.insert((m, stat::DAMAGEPERCENT), 50);
    cx.world.stats.insert((m, stat::DEADLY_STRIKE), 1);
    let d = fill_damage(&mut cx, m, Some(mon));
    assert_eq!((d.phys, d.crit), (300, true));
    cx.world.stats.insert((m, stat::POISONLENGTH), 100);
    cx.world.stats.insert((m, stat::POISON_COUNT), 3);
    assert_eq!(fill_damage(&mut cx, m, None).poison_length, 33);
}

#[test]
fn knockback_roll_on_missile_seed() {
    let mut r = row();
    r.knockback = 100;
    r.gethit = true;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    let mon = w.monster;
    let seed = *w.fake.seed(m);
    let mut cx = Ctx {
        tables: &w.tables,
        store: &mut w.store,
        world: &mut w.fake,
    };
    let f = result_flags(&w.game, &mut cx, m, mon);
    use super::hit::result_flag as rf;
    assert_eq!(f, rf::HIT | rf::GETHIT | rf::KNOCKBACK);
    let mut s = seed;
    s.step();
    assert_eq!(*cx.world.seed(m), s);
    // State 54: no get-hit, no draw.
    cx.world.states.insert((mon, state::UNINTERRUPTABLE));
    assert_eq!(result_flags(&w.game, &mut cx, m, mon), rf::HIT);
    assert_eq!(*cx.world.seed(m), s);
}

// ---- catalogues -------------------------------------------------

#[test]
fn seeded_offset_vector() {
    let mut s = Seed::default();
    assert_eq!(seeded_offset(&mut s, 5000, 8, 4), (-3, 1));
}

/// Checks a catalogue table against its TSV: one row per index, address
/// `-` exactly for the null entries. Returns the first disagreement.
fn check_catalogue(tsv: &str, table: &[Option<u32>]) -> Result<(), String> {
    let rows: Vec<&str> = tsv.lines().skip(1).filter(|l| !l.is_empty()).collect();
    if rows.len() != table.len() {
        return Err(format!("{} rows, table has {}", rows.len(), table.len()));
    }
    for (i, line) in rows.iter().enumerate() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() != 9 {
            return Err(format!("row {i}: {} columns", cols.len()));
        }
        if cols[0] != i.to_string() {
            return Err(format!("row {i}: index {}", cols[0]));
        }
        let addr = match cols[1] {
            "-" => None,
            a => Some(
                u32::from_str_radix(a.trim_start_matches("0x"), 16)
                    .map_err(|e| format!("row {i}: {e}"))?,
            ),
        };
        if addr != table[i] {
            return Err(format!("index {i}: tsv {addr:x?}, table {:x?}", table[i]));
        }
    }
    Ok(())
}

#[test]
fn catalogues_match_tsv() {
    check_catalogue(SRVDO_TSV, &SRV_DO).unwrap();
    check_catalogue(SRVHIT_TSV, &SRV_HIT).unwrap();
    // The implemented bodies are non-null entries.
    for i in catalogue::SRV_DO_IMPLEMENTED {
        assert!(SRV_DO[i as usize].is_some());
    }
    // Null patterns.
    let null_do: Vec<usize> = (0..53).filter(|&i| SRV_DO[i].is_none()).collect();
    let mut want: Vec<usize> = vec![0, 4];
    want.extend(38..53);
    assert_eq!(null_do, want);
    let null_hit: Vec<usize> = (0..71).filter(|&i| SRV_HIT[i].is_none()).collect();
    let mut want = vec![0, 30, 34, 41, 42, 46, 49];
    want.extend(60..71);
    assert_eq!(null_hit, want);
    assert_eq!(SRV_DO[1], Some(0x005B_0BC0));
}

#[test]
fn catalogue_check_catches_perturbations() {
    // M08: change one address and one null cell; the check names them.
    let bad = SRVDO_TSV.replacen("0x005AE400", "0x005AE401", 1);
    assert_eq!(
        check_catalogue(&bad, &SRV_DO),
        Err("index 2: tsv Some(5ae401), table Some(5ae400)".into())
    );
    let bad = SRVHIT_TSV.replacen("\n30\t-\t", "\n30\t0x1\t", 1);
    assert_eq!(
        check_catalogue(&bad, &SRV_HIT),
        Err("index 30: tsv Some(1), table None".into())
    );
    let mut short = SRV_HIT.to_vec();
    short.pop();
    assert!(check_catalogue(SRVHIT_TSV, &short).is_err());
}

#[test]
fn collide_mode_table() {
    let masks: Vec<u16> = COLLIDE_MODES.iter().map(|c| c.mask).collect();
    assert_eq!(masks, [0, 0x84, 0x104, 0x184, 0, 0x104, 0x4, 0x40, 0x185]);
    assert_eq!(collide_mode(9), None);
}

// ---- Coverage of the remaining units -------------------------------

/// A [`Ctx`] over a [`World`]'s fields (leaves `w.game` free).
macro_rules! cx {
    ($w:expr) => {
        Ctx {
            tables: &$w.tables,
            store: &mut $w.store,
            world: &mut $w.fake,
        }
    };
}

impl World {
    /// Parameters with the monster as owner and origin, aiming at the
    /// player.
    fn monster_params(&self) -> MissileParams {
        let mut p = self.params();
        p.owner = Some(self.monster);
        p.origin = Some(self.monster);
        p.target_x = 100;
        p
    }

    fn room(&self) -> RoomId {
        self.fake.room.unwrap()
    }
}

#[test]
fn flags_dword_bit_n_is_its_column() {
    let fields = |r: &MissileRow| {
        [
            r.lastcollide,
            r.explosion,
            r.pierce,
            r.canslow,
            r.candestroy,
            r.clientsend,
            r.gethit,
            r.softhit,
            r.applymastery,
            r.returnfire,
            r.town,
            r.srctown,
            r.nomultishot,
            r.nouniquemod,
            r.half2hsrc,
            r.missileskill,
        ]
    };
    for n in 0..16 {
        let mut buf = [0u8; 420];
        buf[4..8].copy_from_slice(&(1u32 << n).to_le_bytes());
        let got = fields(&MissileRow::decode(&buf));
        for (i, &b) in got.iter().enumerate() {
            assert_eq!(b, i == n, "bit {n}, column {i}");
        }
    }
}

#[test]
fn missile_data_fields_and_clamps() {
    assert_eq!(clamp_frame(40_000), 0x7FFF);
    assert_eq!(clamp_frame(-40_000), -0x8000);
    assert_eq!(clamp_frame(-5), -5);
    let mut w = World::new(row());
    let mut p = w.params();
    p.flags |= param_flags::RANGE | param_flags::DATA_FLAG_2;
    p.range = 40_000;
    p.skill = 0x9000;
    p.level = -1;
    let m = w.create(&p).unwrap();
    let d = w.store.get(m).unwrap().clone();
    // Frame setters clamp; skill clamps to 0…0x7FFF; level is unclamped.
    assert_eq!((d.total, d.current, d.activate), (0x7FFF, 0x7FFF, 0x7FFF));
    assert_eq!((d.skill, d.level), (0x7FFF, -1));
    // Flag 0x10000 → missile data flag bit 1; target fields start at 0.
    assert_eq!(d.flags, 2);
    assert_eq!(d.target, (0, 0));
    let mut p = w.params();
    p.flags |= param_flags::RANGE;
    p.range = -40_000;
    let m2 = w.create(&p).unwrap();
    let d2 = w.store.get(m2).unwrap();
    assert_eq!((d2.total, d2.current, d2.flags), (-0x8000, -0x8000, 0));
    // Elapsed frames = total − current.
    w.store.get_mut(m).unwrap().current = 10;
    assert_eq!(w.store.get(m).unwrap().elapsed(), 0x7FFF - 10);
    // The last-collided unit is kept only with `LastCollide`.
    let mut r = row();
    r.lastcollide = false;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.store.get(m).unwrap().last_collided, None);
}

#[test]
fn unit_fields_mode_flags_and_size() {
    let mut r = row();
    r.collidetype = 8;
    r.candestroy = true;
    r.size = 3;
    let mut w = World::new(r);
    w.fake.alloc_flags = unit_flag::BIT1 | unit_flag::IS_VALID_TARGET;
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.alloc_modes, [8]);
    assert_eq!(w.store.get(m).unwrap().mode, 8);
    assert!(w.fake.unit_flag(m, unit_flag::CAN_BE_ATTACKED));
    assert!(!w.fake.unit_flag(m, unit_flag::IS_VALID_TARGET));
    assert!(!w.fake.unit_flag(m, unit_flag::BIT1));
    // Without `CanDestroy` bit 2 stays clear.
    let mut w2 = World::new(row());
    let m2 = w2.create(&w2.params()).unwrap();
    assert!(!w2.fake.unit_flag(m2, unit_flag::CAN_BE_ATTACKED));
    // Collision on crossed subtiles uses the row's `Size`.
    w.fake.word = coll::MONSTER;
    w.fake.crossed = vec![(105, 100)];
    w.frame();
    assert_eq!(*w.fake.sizes.borrow(), [3]);
}

/// Everything creation left behind, for comparing two creations.
fn creation_snapshot(w: &World, m: UnitId) -> String {
    let stats: Vec<_> = w.fake.stats.iter().filter(|((u, _), _)| *u == m).collect();
    format!(
        "{:?} {:?} {:?} {:?} {:?}",
        w.store.get(m),
        w.fake.pos.get(&m),
        w.fake.calls,
        stats,
        w.fake.flags.get(&m)
    )
}

#[test]
fn parameter_record_fields_read_only_with_their_flags() {
    let mut r = row();
    r.subloop = 1;
    r.substart = 2;
    r.substop = 6;
    let base = |w: &World| MissileParams {
        x: 104,
        y: 100,
        velocity: 9,
        loops: 3,
        start_frame: 5,
        activate: 7,
        attack_bonus: 33,
        range: 20,
        skill: 11,
        level: 2,
        ..w.params()
    };
    let create = |flags: u32| {
        let mut w = World::new(r.clone());
        let mut p = base(&w);
        p.flags |= flags;
        let m = w.create(&p).unwrap();
        (creation_snapshot(&w, m), w, m)
    };
    // Bits not read by creation are ignored.
    let (plain, w, m) = create(0);
    let ignored = 0x40 | 0x80 | 0x100 | 0x2000 | 0x4000 | 0xFFFE_0000;
    assert_eq!(create(ignored).0, plain);
    // Without their flags the fields are not used: origin position, table
    // velocity, frames = Range, Activate from the table, no tohit stat.
    let d = w.store.get(m).unwrap();
    assert_eq!(w.fake.position(m), (100, 100));
    assert_eq!(w.fake.velocity(m), 3840);
    assert_eq!((d.total, d.activate, d.skill, d.level), (50, 50, 11, 2));
    assert_eq!(w.fake.stats.get(&(m, stat::TOHIT)), None);
    // Each flag reads its field.
    let (_, w, m) = create(param_flags::POSITION);
    assert_eq!(w.fake.position(m), (104, 100));
    let (_, w, m) = create(param_flags::VELOCITY);
    assert_eq!(w.fake.velocity(m), (9 << 8) * 75 / 100);
    let (_, w, m) = create(param_flags::LOOPS);
    assert_eq!(w.store.get(m).unwrap().total, 50 + 3 * 4);
    let (_, w, m) = create(param_flags::START_FRAME);
    assert_eq!(w.store.get(m).unwrap().total, 45);
    let (_, w, m) = create(param_flags::ACTIVATE);
    assert_eq!(w.store.get(m).unwrap().activate, 43);
    let (_, w, m) = create(param_flags::ATTACK_BONUS);
    assert_eq!(w.fake.stat(m, stat::TOHIT), 33);
    let (_, w, m) = create(param_flags::RANGE);
    assert_eq!(w.store.get(m).unwrap().total, 20);
    let (_, w, m) = create(param_flags::DATA_FLAG_2);
    assert_eq!(w.store.get(m).unwrap().flags, 2);
    let (_, w, m) = create(param_flags::FRAMES_FROM_DISTANCE);
    let d = w.store.get(m).unwrap();
    assert_eq!(
        (d.total, i32::from(d.current)),
        (50, frames_from_distance(10, 3840) as i32)
    );
    // Target relative (2) wins over absolute (0x20); the init callback
    // runs once with its argument.
    let mut w = World::new(r.clone());
    let mut p = base(&w);
    p.flags = param_flags::TARGET_RELATIVE | param_flags::TARGET_ABSOLUTE;
    p.target_x = 3;
    p.target_y = 0;
    p.init = Some((7, 9));
    let m = w.create(&p).unwrap();
    assert_eq!(w.fake.paths[&m].target_point, Some((103, 100)));
    assert_eq!(
        w.fake
            .calls
            .iter()
            .filter(|c| c.starts_with("init"))
            .count(),
        1
    );
    assert!(w.fake.calls.contains(&"init 7 9".to_string()));
}

#[test]
fn creation_start_position() {
    let mut w = World::new(row());
    let mut p = w.params();
    p.flags |= param_flags::POSITION;
    p.x = 104;
    p.y = 101;
    p.origin = Some(w.monster);
    let m = w.create(&p).unwrap();
    assert_eq!(w.fake.position(m), (104, 101));
    // Flag 1 clear: the origin's position.
    let mut p = w.params();
    p.origin = Some(w.monster);
    p.target_x = 100;
    let m = w.create(&p).unwrap();
    assert_eq!(w.fake.position(m), (110, 100));
    // No origin → fail before allocation.
    let mut p = w.params();
    p.origin = None;
    let allocs = w.fake.logged("alloc");
    assert_eq!(w.create(&p), None);
    assert_eq!(w.fake.logged("alloc"), allocs);
}

#[test]
fn creation_room_search_from_owner_room() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.room_near.get(), Some(w.room()));
    assert_eq!(w.game.lists.unit(m).unwrap().room(), Some(w.room()));
    w.fake.no_room = true;
    assert_eq!(w.create(&w.params()), None);
    assert_eq!(w.fake.logged("alloc"), 1);
}

#[test]
fn creation_path_setup() {
    let mut r = row();
    r.collision = 1;
    r.collidetype = 8;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    assert_eq!(
        &w.fake.calls[..4],
        ["vel 0", "tpoint 110 100", "foot 0x40", "move 0x185"]
    );
    assert!(!w.fake.unit_flag(m, unit_flag::CAN_BE_ATTACKED));
    // Target unit instead of a point; Collision 0 → footprint 0;
    // `CanDestroy` sets unit flag bit 2.
    let mut r = row();
    r.candestroy = true;
    let mut w = World::new(r);
    let mut p = w.params();
    p.target = Some(w.monster);
    let mon = w.monster.0;
    let m = w.create(&p).unwrap();
    assert_eq!(
        &w.fake.calls[..4],
        [
            "vel 0".to_string(),
            format!("tunit {mon}"),
            "foot 0x0".into(),
            "move 0x184".into()
        ]
    );
    assert!(w.fake.unit_flag(m, unit_flag::CAN_BE_ATTACKED));
}

#[test]
fn creation_velocity_build_and_acceleration() {
    let mut r = row();
    r.vel = 20;
    r.maxvel = 30;
    r.accel = 0xFFF6; // −10 as i16
    let mut w = World::new(r);
    let _ = w.create(&w.params()).unwrap();
    assert_eq!(&w.fake.calls[4..7], ["vel 3840", "build", "accel -10 7680"]);
    // v = 0: no velocity, no build.
    let mut r = row();
    r.vel = 0;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.velocity(m), 0);
    assert!(!w.fake.calls.iter().any(|c| c == "build"));
    assert_eq!(
        w.fake.calls.iter().filter(|c| c.starts_with("vel")).count(),
        1
    );
}

#[test]
fn creation_stat_list_callback_then_damage_setup() {
    let mut w = World::new(row());
    let mut p = w.params();
    p.origin = Some(w.monster);
    p.target_x = 100;
    p.level = 4;
    p.init = Some((3, 5));
    let m = w.create(&p).unwrap();
    let n = w.fake.calls.len();
    let setup = format!("setup {} Some({}) {} 4", w.owner.0, w.monster.0, m.0);
    assert_eq!(
        w.fake.calls[n - 3..],
        ["statlist".to_string(), "init 3 5".into(), setup]
    );
    // No callback: none called.
    let mut w = World::new(row());
    let _ = w.create(&w.params()).unwrap();
    assert!(!w.fake.calls.iter().any(|c| c.starts_with("init")));
}

#[test]
fn creation_stores_owner_and_clears_bit_3_after_setup() {
    let mut w = World::new(row());
    w.fake.setup_sets_valid = true;
    let m = w.create(&w.params()).unwrap();
    let guid = w.game.lists.unit(w.owner).unwrap().guid;
    assert_eq!(
        w.store.get(m).unwrap().owner,
        Some(UnitRef {
            ty: UnitType::Player,
            guid
        })
    );
    assert!(!w.fake.unit_flag(m, unit_flag::IS_VALID_TARGET));
}

#[test]
fn creation_tohit_damage_rate_and_result() {
    let mut r = row();
    r.damagerate = 25;
    let mut w = World::new(r);
    let mut p = w.params();
    p.flags |= param_flags::ATTACK_BONUS;
    p.attack_bonus = 70;
    let m = w.create(&p).unwrap();
    assert_eq!(w.game.lists.units_of_type(UnitType::Missile), [m]);
    assert_eq!(w.fake.stat(m, stat::TOHIT), 70);
    assert_eq!(w.fake.stat(m, stat::DAMAGE_FRAMERATE), 25);
    let m2 = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.stats.get(&(m2, stat::TOHIT)), None);
}

/// A dispatcher after the missile one that records what reaches it.
#[derive(Default)]
struct Next(Vec<(TimerClass, u8)>);

impl EventDispatch for Next {
    fn run_event(&mut self, _: &mut Game, run: &TimerRun) {
        self.0.push((run.class, run.event));
    }
}

#[test]
fn class_handler_ignores_event_type_and_args() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    let frame = w.game.frame;
    w.game.schedule_event(m, 5, frame + 1, None, 7, 8).unwrap();
    let mon = w.monster;
    w.game
        .schedule_event(mon, 5, frame + 1, None, 0, 0)
        .unwrap();
    w.game.frame += 1;
    let mut next = Next::default();
    let mut d = MissileDispatch {
        cx: Ctx {
            tables: &w.tables,
            store: &mut w.store,
            world: &mut w.fake,
        },
        next: &mut next,
    };
    run_timer_events(&mut w.game, &mut d);
    // The every-tick run and the type-5 event both ran the handler.
    assert_eq!(w.store.get(m).unwrap().current, 48);
    // The monster's event went to the next dispatcher.
    assert_eq!(next.0, [(TimerClass::Monster, 5)]);
}

#[test]
fn missile_without_room_passes_town_tests() {
    // Mode 0: the flight never reaches the room test, so only acts.
    for in_room in [true, false] {
        let mut r = row();
        r.collidetype = 0;
        let mut w = World::new(r);
        let p = w.monster_params();
        let m = w.create(&p).unwrap();
        let room = w.room();
        w.fake.town.insert(room);
        // Keep the owner out of town (no owner room).
        let mon = w.monster;
        w.game.lists.room_remove(mon).unwrap();
        if !in_room {
            w.game.lists.room_remove(m).unwrap();
        }
        w.frame();
        assert_eq!(w.alive(m), !in_room, "in room: {in_room}");
    }
    // A collision mode: removed by flight step 5 on run 1, without a hit.
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.game.lists.room_remove(m).unwrap();
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(w.fake.logged("event0"), 0);
    assert_eq!(w.fake.logged("clear"), 0);
    // No path: the same.
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.fake.no_path = true;
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(w.fake.logged("event0"), 0);
}

#[test]
fn owner_that_no_longer_exists_is_none() {
    for owner_alive in [true, false] {
        let mut r = row();
        r.town = true;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        let room = w.room();
        w.fake.town.insert(room);
        if !owner_alive {
            let o = w.owner;
            w.game.remove_unit(o).unwrap();
            let cx = cx!(w);
            assert_eq!(cx.owner(&w.game, m), None);
        }
        w.frame();
        // A player owner in town removes it only while it exists.
        assert_eq!(w.alive(m), !owner_alive);
    }
}

#[test]
fn src_town_removes_when_owner_room_is_in_town() {
    // Monster owner (not the town rule), `Town` row (not the town row rule).
    let case = |src_town: bool, owner_room: bool, owner_alive: bool| {
        let mut r = row();
        r.town = true;
        r.srctown = src_town;
        r.collidetype = 0;
        let mut w = World::new(r);
        let p = w.monster_params();
        let m = w.create(&p).unwrap();
        let room = w.room();
        w.fake.town.insert(room);
        let mon = w.monster;
        if !owner_room {
            w.game.lists.room_remove(mon).unwrap();
        }
        if !owner_alive {
            w.game.remove_unit(mon).unwrap();
        }
        w.frame();
        w.alive(m)
    };
    assert!(!case(true, true, true));
    assert!(case(false, true, true));
    assert!(case(true, false, true));
    assert!(case(true, true, false));
}

#[test]
fn server_do_result_two_removes() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.frame();
    assert!(w.alive(m));
    assert_eq!(w.fake.logged("remove"), 0);
    // Result 2 (blocking word): "remove" frees the unit, its timers and
    // its data.
    w.fake.word = coll::WALL;
    w.frame();
    assert_eq!(w.fake.log.last().unwrap(), &format!("remove {}", m.0));
    assert!(!w.alive(m));
    assert!(w.game.lists.unit(m).is_none());
    assert!(w.game.timers.unit_timers(m).is_empty());
    w.frame();
    assert_eq!(w.fake.logged("remove"), 1);
}

// The state is unreachable (creation fails without a
// record); the hit handler asserts the record.
#[test]
#[cfg_attr(debug_assertions, should_panic(expected = "no missiles.txt record"))]
fn flight_without_record_is_an_expiry_hit() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.store.get_mut(m).unwrap().class = 5; // past the table
    let want = w.hit(m, None, true);
    let mut cx = cx!(w);
    let got = default_flight(&mut w.game, &mut cx, m);
    assert_eq!(got, want);
    // Returned before moving or counting down.
    assert_eq!(w.store.get(m).unwrap().current, 50);
}

#[test]
fn count_down_then_expiry_below_one() {
    let mut r = row();
    r.range = 3;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    w.frame();
    assert_eq!(w.store.get(m).unwrap().current, 2);
    w.frame();
    assert_eq!(w.store.get(m).unwrap().current, 1);
    assert_eq!(w.fake.logged("event0"), 0);
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(w.fake.logged("event0 None"), 1);
}

#[test]
fn no_subtile_hit_keeps_flying() {
    let mut w = World::new(row());
    let m = w.create(&w.params()).unwrap();
    w.fake.word = coll::MONSTER;
    // A clear subtile, then one with a matching mask but only the owner
    // (rejected: last collided) and no barrier bit.
    w.fake.crossed = vec![(104, 100), (100, 100)];
    w.fake.masks.insert((100, 100), coll::PLAYER);
    w.fake.units.insert((100, 100), vec![w.owner]);
    let mut cx = cx!(w);
    assert_eq!(default_flight(&mut w.game, &mut cx, m), 1);
    assert!(w.alive(m));
    assert_eq!(w.fake.logged("event0"), 0);
}

#[test]
fn max_vel_caps_only_acceleration() {
    // Created at 3840 with MaxVel 4 (1024): the path keeps 3840.
    let mut r = row();
    r.maxvel = 4;
    let mut w = World::new(r);
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.velocity(m), 3840);
    assert!(w.fake.calls.contains(&"accel 0 1024".to_string()));
    let mut v = PathVelocity {
        velocity: 3840,
        max: 1024,
        accel: 0,
        counter: 0,
    };
    for _ in 0..10 {
        v.advance();
    }
    assert_eq!(v.velocity, 3840);
    // Step vector with base 0x400 at that speed.
    assert_eq!(v.step_component(4096), (((3840 * 0x400) >> 6) * 4096) >> 12);
    // Once it accelerates, the cap applies.
    v.accel = 10;
    for _ in 0..4 {
        v.advance();
    }
    assert_eq!(v.velocity, 3840);
    v.advance();
    assert_eq!((v.velocity, v.accel), (1024, 0));
}

#[test]
fn collide_kill_sets_result_bit_one() {
    for (explosion, kill, want) in [(false, 1, 2), (false, 0, 1), (true, 1, 2), (true, 0, 1)] {
        let mut r = row();
        r.explosion = explosion;
        r.collidekill = kill;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        let mon = w.monster;
        assert_eq!(w.hit(m, Some(mon), false), want, "{explosion} {kill}");
    }
}

#[test]
fn no_unit_no_a4_skips_to_exit_unless_always_explode() {
    for (always, kill) in [(0, 1), (0, 0), (1, 1)] {
        let mut r = row();
        // Server-hit 36: with no unit it creates a
        // class-0 missile and returns 1, observable as an allocation.
        r.psrvhitfunc = 36;
        r.alwaysexplode = always;
        r.collidekill = kill;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        let res = w.hit(m, None, false);
        assert_eq!(res, if kill == 1 { 2 } else { 1 });
        let ran = always == 1;
        assert_eq!(w.fake.logged("event0 None"), usize::from(ran));
        assert_eq!(w.fake.logged("alloc"), 1 + usize::from(ran));
        assert!(w.store.unhandled.is_empty());
    }
}

#[test]
fn damage_stage_fills_then_server_damage() {
    // Function 4 (iceblast) moves the filled cold length into the
    // freeze length; null entries 15–30 are unhandled; 0 and ≥ 31 run
    // nothing.
    let cases = [
        (4u16, true),
        (15, false),
        (30, false),
        (31, false),
        (0, false),
        (0xFFFF, false),
    ];
    for (f, call) in cases {
        let mut r = row();
        r.psrvdmgfunc = f;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        w.fake.stats.insert((m, stat::MINDAMAGE), 0x100);
        w.fake.stats.insert((m, stat::MAXDAMAGE), 0x100);
        w.fake.stats.insert((m, stat::COLDLENGTH), 50);
        let mon = w.monster;
        w.hit(m, Some(mon), false);
        let i = f as i16;
        let lengths = if call {
            "lengths cold 0 freeze 50"
        } else {
            "lengths cold 50 freeze 0"
        };
        assert_eq!(w.fake.logged(lengths), 1, "{f}");
        let null = (15..=30).contains(&i);
        let want = if null {
            vec![Unhandled::NullSrvDmg {
                index: i,
                missile: m,
            }]
        } else {
            vec![]
        };
        assert_eq!(w.store.unhandled, want);
        assert_eq!(w.fake.logged(&format!("damage {} 256", mon.0)), 1);
    }
}

#[test]
fn earlier_removals() {
    let run1 = |r: MissileRow, set: &dyn Fn(&mut World)| {
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        set(&mut w);
        w.frame();
        !w.alive(m)
    };
    // Control: nothing set → alive after run 1 of 50.
    assert!(!run1(row(), &|_| {}));
    assert!(run1(row(), &|w| w.fake.stop_moving = true));
    assert!(run1(row(), &|w| w.fake.word = coll::WALL));
    let mut r4 = row();
    r4.collidetype = 4;
    assert!(run1(r4, &|w| w.fake.word = coll::MONSTER));
    assert!(run1(row(), &|w| {
        w.fake.word = coll::MONSTER;
        w.fake.crossed = vec![(105, 100)];
        w.fake.masks.insert((105, 100), coll::MISSILE_BARRIER);
    }));
    assert!(run1(row(), &contact));
    let mut miss = row();
    miss.tohit = 1;
    miss.collidekill = 0;
    assert!(run1(miss, &|w| {
        contact(w);
        w.fake.hits.push_back(false);
    }));
    assert!(run1(row(), &|w| {
        let room = w.room();
        w.fake.town.insert(room);
    }));
}

#[test]
fn pierce_test_only_for_pierce_rows() {
    // Player and monster owners run the test with `Pierce`; without it
    // nothing is set even with a pierce stat.
    for pierce in [true, false] {
        let mut r = row();
        r.pierce = pierce;
        let mut w = World::new(r);
        w.fake.stats.insert((w.owner, stat::ITEM_PIERCE), 67);
        w.fake.stats.insert((w.monster, stat::SKILL_PIERCE), 67);
        let m = w.create(&w.params()).unwrap();
        let p = w.monster_params();
        let mm = w.create(&p).unwrap();
        let want = pierce.then_some(4);
        assert_eq!(w.fake.stats.get(&(m, stat::PIERCE_IDX)).copied(), want);
        assert_eq!(w.fake.stats.get(&(mm, stat::PIERCE_IDX)).copied(), want);
    }
}

/// Checks the column layout of a catalogue. Returns the first
/// disagreement.
fn check_tsv_columns(tsv: &str, rows_want: usize) -> Result<(), String> {
    let mut lines = tsv.lines().filter(|l| !l.is_empty());
    let header = lines.next().ok_or("empty")?;
    let want = [
        "index",
        "addr_114d",
        "d2moo_name",
        "rows",
        "examples",
        "params",
        "rng",
        "behaviour",
        "status",
    ];
    if header.split('\t').collect::<Vec<_>>() != want {
        return Err(format!("header {header:?}"));
    }
    let rows: Vec<&str> = lines.collect();
    if rows.len() != rows_want {
        return Err(format!("{} rows", rows.len()));
    }
    let is_id = |t: &str| {
        t.split(')')
            .next()
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
    };
    for (i, line) in rows.iter().enumerate() {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() != 9 || c[0] != i.to_string() {
            return Err(format!("row {i}: {line:?}"));
        }
        if c[1] != "-" && !c[1].starts_with("0x") {
            return Err(format!("row {i}: addr {:?}", c[1]));
        }
        let n: usize = c[3]
            .parse()
            .map_err(|_| format!("row {i}: rows {:?}", c[3]))?;
        // `name(id)` items: count the "(digits)" groups.
        let examples = c[4].split('(').skip(1).filter(|t| is_id(t)).count();
        if examples > 4 || examples > n || (c[4] != "-" && examples == 0) {
            return Err(format!("row {i}: examples {:?}", c[4]));
        }
        if !["spec'd-here", "summarized", "D2MOO-only", "unread"].contains(&c[8]) {
            return Err(format!("row {i}: status {:?}", c[8]));
        }
    }
    Ok(())
}

#[test]
fn catalogue_tsv_columns() {
    check_tsv_columns(SRVDO_TSV, 53).unwrap();
    check_tsv_columns(SRVHIT_TSV, 71).unwrap();
    // M08: perturbations are reported.
    let bad = SRVDO_TSV.replacen("\tspec'd-here", "\tdone", 1);
    assert_eq!(
        check_tsv_columns(&bad, 53),
        Err("row 0: status \"done\"".into())
    );
    assert!(check_tsv_columns(SRVDO_TSV, 52).is_err());
    let bad = SRVDO_TSV.replacen("index\t", "idx\t", 1);
    assert!(check_tsv_columns(&bad, 53).is_err());
}

/// A parent missile at (5000, 100) with 8 elapsed frames, skill 7,
/// level 3.
fn helper_world() -> (World, UnitId) {
    let mut w = World::new(row());
    let mut p = w.params();
    p.skill = 7;
    p.level = 3;
    let m = w.create(&p).unwrap();
    w.fake.pos.insert(m, (5000, 100));
    let d = w.store.get_mut(m).unwrap();
    d.total = 50;
    d.current = 42;
    (w, m)
}

fn run_helper(w: &mut World, m: UnitId, interval: i32, mask: u16) -> Option<UnitId> {
    let mut cx = cx!(w);
    catalogue::create_with_collision_check(&mut w.game, &mut cx, m, 4, interval, 0, mask)
}

#[test]
fn sub_missile_helper_preconditions() {
    let (mut w, m) = helper_world();
    let seed = *w.fake.seed(m);
    // i = 0, or 8 not a multiple of 3: nothing, no re-seed.
    assert_eq!(run_helper(&mut w, m, 0, 0), None);
    assert_eq!(run_helper(&mut w, m, 3, 0), None);
    assert_eq!(*w.fake.seed(m), seed);
    // Signed remainder: elapsed −10, i = 5 → 0 (unsigned would be 1).
    let d = w.store.get_mut(m).unwrap();
    d.total = 0;
    d.current = 10;
    assert!(run_helper(&mut w, m, 5, 0).is_some());
    // No room / no owner: nothing, no re-seed.
    let (mut w, m) = helper_world();
    let seed = *w.fake.seed(m);
    w.game.lists.room_remove(m).unwrap();
    assert_eq!(run_helper(&mut w, m, 4, 0), None);
    assert_eq!(*w.fake.seed(m), seed);
    let (mut w, m) = helper_world();
    let seed = *w.fake.seed(m);
    let o = w.owner;
    w.game.remove_unit(o).unwrap();
    assert_eq!(run_helper(&mut w, m, 4, 0), None);
    assert_eq!(*w.fake.seed(m), seed);
}

#[test]
fn sub_missile_helper_reseeds_and_creates() {
    let (mut w, m) = helper_world();
    let sub = run_helper(&mut w, m, 4, coll::WALL).unwrap();
    // Re-seeded with x + elapsed = 5008, then two rolls of 6.
    let mut s = Seed::init_low(5008);
    s.roll(6);
    s.roll(6);
    assert_eq!(*w.fake.seed(m), s);
    // At (x − 3, y + 1) (test vector), owner, skill and level of the parent.
    assert_eq!(w.fake.position(sub), (4997, 101));
    let (pd, sd) = (w.store.get(m).unwrap(), w.store.get(sub).unwrap());
    assert_eq!((sd.owner, sd.skill, sd.level), (pd.owner, 7, 3));
    // Collision there & mask ≠ 0 → nothing (the re-seed still happened).
    let (mut w, m) = helper_world();
    w.fake.masks.insert((4997, 101), coll::WALL);
    let allocs = w.fake.logged("alloc");
    assert_eq!(run_helper(&mut w, m, 4, coll::WALL), None);
    assert_eq!(w.fake.logged("alloc"), allocs);
    assert_eq!(*w.fake.seed(m), s);
    // Other bits there do not block.
    assert!(run_helper(&mut w, m, 4, coll::MISSILE_BARRIER).is_some());
}

/// A recorded-missile row: server-do 1, server-hit 0, CollideType
/// 3, CollideKill 1, LastCollide 1, Size 1, Activate 0, Accel 0, LevRange
/// 0; flags 0x249 (+ Pierce).
fn recorded_row(vel: u8, vellev: u8, range: u16, tohit: u8, pierce: bool) -> MissileRow {
    let mut r = row();
    r.vel = vel;
    r.vellev = vellev;
    r.maxvel = vel;
    r.range = range;
    r.tohit = tohit;
    r.pierce = pierce;
    r.canslow = true;
    r.gethit = true;
    r.returnfire = true;
    r
}

#[test]
fn recorded_arrow_path() {
    let rogue1 = || {
        let mut r = recorded_row(24, 0, 40, 1, true);
        r.alwaysexplode = 1;
        r
    };
    // Owner first on the path, then the monster: the owner is skipped and
    // the monster is hit; to-hit hit → damage and removal.
    let mut w = World::new(rogue1());
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.velocity(m), 4608);
    contact(&mut w);
    w.fake.crossed = vec![(100, 100), (110, 100)];
    w.fake.masks.insert((100, 100), coll::PLAYER);
    w.fake.units.insert((100, 100), vec![w.owner]);
    w.fake.hits.push_back(true);
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(w.fake.logged(&format!("damage {}", w.monster.0)), 1);
    assert!(w.fake.hits.is_empty());
    // Miss → removed without damage.
    let mut w = World::new(rogue1());
    let m = w.create(&w.params()).unwrap();
    contact(&mut w);
    w.fake.hits.push_back(false);
    w.frame();
    assert!(!w.alive(m));
    assert_eq!(w.fake.logged("damage"), 0);
    // Pierce count left → flies on after a hit.
    let mut w = World::new(rogue1());
    w.fake.stats.insert((w.owner, stat::SKILL_PIERCE), 67);
    let m = w.create(&w.params()).unwrap();
    contact(&mut w);
    w.frame();
    assert!(w.alive(m));
    assert_eq!(w.fake.logged("damage"), 1);
    // No contact: expires after `Range` runs.
    let mut w = World::new(rogue1());
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.lifetime(m), 40);
}

#[test]
fn recorded_spike1() {
    let mut w = World::new(recorded_row(10, 8, 40, 1, false));
    let mut p = w.monster_params();
    p.level = 2;
    w.fake.stats.insert((w.monster, stat::SKILL_PIERCE), 67);
    let m = w.create(&p).unwrap();
    assert_eq!(w.fake.velocity(m), ((10 + 2 * 8 / 8) << 8) * 75 / 100);
    assert_eq!(w.fake.stats.get(&(m, stat::PIERCE_IDX)), None);
    // To-hit rolled: a miss removes it without damage.
    w.fake.hits.push_back(false);
    let o = w.owner;
    assert_eq!(w.hit(m, Some(o), false), 2);
    assert_eq!(w.fake.logged("damage"), 0);
}

#[test]
fn recorded_shafire1_and_firebolt() {
    // shafire1: monster owner without pierce stats, no to-hit test.
    let mut w = World::new(recorded_row(8, 0, 40, 0, true));
    let p = w.monster_params();
    let m = w.create(&p).unwrap();
    assert_eq!(w.fake.velocity(m), 1536);
    assert_eq!(w.fake.stats.get(&(m, stat::PIERCE_IDX)), None);
    w.fake.hits.push_back(false);
    let o = w.owner;
    assert_eq!(w.hit(m, Some(o), false), 2);
    assert_eq!(w.fake.logged(&format!("damage {}", o.0)), 1);
    assert_eq!(w.fake.hits.len(), 1, "no to-hit draw");
    // firebolt: no `Pierce`, so a player's pierce stat does nothing.
    let mut w = World::new(recorded_row(20, 0, 50, 0, false));
    w.fake.stats.insert((w.owner, stat::SKILL_PIERCE), 67);
    let m = w.create(&w.params()).unwrap();
    assert_eq!(w.fake.velocity(m), 3840);
    assert_eq!(w.fake.stats.get(&(m, stat::PIERCE_IDX)), None);
    w.fake.hits.push_back(false);
    let mon = w.monster;
    assert_eq!(w.hit(m, Some(mon), false), 2);
    assert_eq!(w.fake.logged("damage"), 1);
}

#[test]
fn damage_roll_never_reaches_max() {
    let mut seen = BTreeSet::new();
    for lo in 0..500u32 {
        let mut s = Seed::init_low(lo);
        assert_eq!(damage_roll(&mut s, 0x100, 0x101, 0), 0x100);
        let mut s = Seed::init_low(lo);
        seen.insert(damage_roll(&mut s, 1, 3, 0));
    }
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), [1, 2]);
}

// Null dispatch entries are reported through the unhandled log.
#[test]
fn null_table_entries_are_flagged() {
    // Dispatch logs each null entry instead of invoking a handler.
    let mut do_null: Vec<i16> = vec![4];
    do_null.extend(38..53);
    for i in do_null {
        let mut r = row();
        r.psrvdofunc = i as u16;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        w.frame();
        let want = Unhandled::NullSrvDo {
            index: i,
            missile: m,
        };
        assert_eq!(w.store.unhandled, [want]);
    }
    let mut hit_null: Vec<i16> = vec![30, 34, 41, 42, 46, 49];
    hit_null.extend(60..71);
    for i in hit_null {
        let mut r = row();
        r.psrvhitfunc = i as u16;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        w.hit(m, None, true);
        let want = Unhandled::NullSrvHit {
            index: i,
            missile: m,
        };
        assert_eq!(w.store.unhandled, [want]);
    }
    for i in 15..31i16 {
        let mut r = row();
        r.psrvdmgfunc = i as u16;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        let mon = w.monster;
        w.hit(m, Some(mon), false);
        let want = Unhandled::NullSrvDmg {
            index: i,
            missile: m,
        };
        assert_eq!(w.store.unhandled, [want]);
        assert_eq!(w.fake.logged("lengths cold 0 freeze 0"), 1);
    }
}

#[test]
fn server_damage_functions_adjust_the_record() {
    // (function, dParam1, dParam2, setup) → the record the damage
    // application receives.
    let run = |f: u16, p1: u32, p2: u32, etype: u8, set: &dyn Fn(&mut World, UnitId)| {
        let mut r = row();
        r.psrvdmgfunc = f;
        r.dparam1 = p1;
        r.dparam2 = p2;
        r.etype = etype;
        let mut w = World::new(r);
        let m = w.create(&w.params()).unwrap();
        w.fake.stats.insert((m, stat::MINDAMAGE), 0x1000);
        w.fake.stats.insert((m, stat::MAXDAMAGE), 0x1000);
        w.fake.stats.insert((m, stat::COLDLENGTH), 50);
        set(&mut w, m);
        let mon = w.monster;
        w.hit(m, Some(mon), false);
        w
    };
    // 1: c = DmgCalc1 (here 25 %) of the physical moves to the element
    // (EType 1, fire).
    let w = run(1, 0, 0, 1, &|w, _| w.fake.mb.calc = 25);
    let mon = w.monster;
    assert_eq!(w.fake.logged(&format!("damage {} {}", mon.0, 0xC00)), 1);
    assert_eq!(w.fake.logged("fire 1024"), 1);
    // c ≥ 100 converts all of it.
    let w = run(1, 0, 0, 1, &|w, _| w.fake.mb.calc = 400);
    assert_eq!(w.fake.logged(&format!("damage {} 0", mon.0)), 1);
    // 2: freeze := pct(cold length 50, dParam1 150) = 75, cold length 0.
    let w = run(2, 150, 0, 0, &|_, _| {});
    assert_eq!(w.fake.logged("lengths cold 0 freeze 75"), 1);
    // 3: dParam1 128 > every (lo & 0x7F): soft hit 0x4000; 0: never.
    let w = run(3, 128, 0, 0, &|_, _| {});
    assert_eq!(w.fake.logged("record result 0x4000"), 1);
    let w = run(3, 0, 0, 0, &|_, _| {});
    assert_eq!(w.fake.logged("record result 0x0"), 1);
    // 7: stun := dParam1 (> 0), hit class 0x60.
    let w = run(7, 30, 0, 0, &|_, _| {});
    assert_eq!(w.fake.logged("stun 30 class Some(96)"), 1);
    // 10: freeze := cold length, cold length kept.
    let w = run(10, 0, 0, 0, &|_, _| {});
    assert_eq!(w.fake.logged("lengths cold 50 freeze 50"), 1);
    // 14: a monster neither small nor large, dParam1 = 1: p = dParam2 =
    // 100 → knockback 8; a large one with dParam1 = 1: none.
    let w = run(14, 1, 100, 0, &|_, _| {});
    assert_eq!(w.fake.logged("record result 0x8"), 1);
    let w = run(14, 1, 100, 0, &|w, _| {
        let mon = w.monster;
        w.fake.mb.large.insert(mon);
    });
    assert_eq!(w.fake.logged("record result 0x0"), 1);
}

#[test]
fn add_elem_and_clear_elems_by_etype() {
    use crate::combat::DamageRecord;
    use crate::missiles::srv_dmg::{add_elem, clear_elems};
    let mut r = DamageRecord::default();
    for (e, a) in [(0u8, 1), (1, 2), (2, 3), (3, 4), (4, 5), (5, 6), (6, 7)] {
        add_elem(&mut r, e, a);
    }
    for (e, a) in [(7u8, 8), (8, 9), (9, 10), (11, 11), (10, 99), (13, 99)] {
        add_elem(&mut r, e, a);
    }
    assert_eq!(
        (r.physical, r.fire, r.lightning, r.magic, r.cold, r.poison),
        (1, 2, 3, 4, 5, 6)
    );
    assert_eq!(
        (
            r.life_leech,
            r.mana_leech,
            r.stamina_leech,
            r.stun_len,
            r.burn
        ),
        (7, 8, 9, 10, 11)
    );
    // 12 sets cold.
    add_elem(&mut r, 12, 40);
    assert_eq!(r.cold, 40);
    let lengths = |r: &DamageRecord| (r.cold_len, r.poison_len, r.burn_len, r.freeze_len);
    for (etype, want) in [
        (4u8, (1, 0, 0, 4)),
        (12, (1, 0, 0, 4)),
        (5, (0, 2, 0, 0)),
        (11, (0, 0, 3, 0)),
        (0, (0, 0, 0, 0)),
    ] {
        let mut c = DamageRecord {
            hit_flags: 0x701,
            cold_len: 1,
            poison_len: 2,
            burn_len: 3,
            freeze_len: 4,
            ..r
        };
        clear_elems(&mut c, etype);
        assert_eq!(lengths(&c), want, "etype {etype}");
        assert_eq!(c.hit_flags, 1);
        assert_eq!((c.physical, c.cold, c.stun_len, c.burn), (0, 0, 0, 0));
    }
}

// Tests written against surviving mutants (METHODS M08); a child module so
// they share this module's fakes.
#[path = "mutant_tests.rs"]
mod mutant_tests;

mod cov_text;
mod ext;
mod mod_missiles;
mod r9;
