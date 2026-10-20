//! Monster AI tests: the tables, install and dispatch, the command
//! list and the function bodies against a small fake world.
use std::collections::{BTreeMap, BTreeSet};

use d2_data::tables::{Levels, Missiles, Monstats, Monstats2, Record, Skills};

use super::functions::IMPLEMENTED;
use super::table::{AI_FUNCTIONS_TSV, SPECD_HERE};
use super::*;
use crate::rng::Seed;
use crate::units::RoomId;

const SEEDS: [u32; 4] = [1, 12345, 3_735_928_559, 4_014_346_870];

#[derive(Default)]
struct Fake {
    /// The game's rules (a mod game's AI, MS5).
    rules: crate::rules::Ruleset,
    marks: Vec<u32>,
    seeds: BTreeMap<UnitId, Seed>,
    class: BTreeMap<UnitId, i32>,
    anim: BTreeMap<UnitId, u8>,
    states: BTreeSet<(UnitId, u16)>,
    dead: BTreeSet<UnitId>,
    pos: BTreeMap<UnitId, (i32, i32)>,
    level: i32,
    life: i32,
    ai_state: u32,
    unique: bool,
    walk_fails: bool,
    can_walk_off: bool,
    town: BTreeSet<RoomId>,
    collides: bool,
    melee: BTreeSet<UnitId>,
    nodes: Vec<Vec<UnitId>>,
    secondary: Option<(UnitId, i32)>,
    log: Vec<String>,
    // Knobs added for the rule tests below (defaults keep the behaviour
    // the older tests rely on).
    align: u8,
    champion: bool,
    vision: Option<u32>,
    interacting: bool,
    busy: BTreeSet<UnitId>,
    /// Modes whose start fails.
    fail_modes: BTreeSet<u8>,
    /// Point targets whose mode start fails.
    fail_points: BTreeSet<(i32, i32)>,
    /// A failed mode start falls into the neutral start, which
    /// adds a think at frame + this when none is pending later.
    fail_think: Option<i32>,
    blocked_path: bool,
    door: Option<(UnitId, bool)>,
    no_los_draw: bool,
    line_blocked: BTreeSet<UnitId>,
    reach_fails: bool,
    spot: Option<(i32, i32, RoomId)>,
    last_dead: BTreeMap<RoomId, [Option<UnitId>; 4]>,
    forced: Option<(UnitId, i32)>,
    good: Option<(UnitId, i32)>,
    /// `good_target_search` result by searching unit (before `good`).
    good_for: BTreeMap<UnitId, (UnitId, i32)>,
    nearest: Option<(UnitId, bool)>,
    special_walk: Option<(UnitId, i32)>,
    /// Acts by unit (default 0).
    acts: BTreeMap<UnitId, u8>,
    /// `choose_alternative` takes the slot-9 alternative.
    take_alt: bool,
    /// NPC interaction block (monster data +0x30).
    npc_block: bool,
    /// Players in the NPC's interaction list.
    npc_list: BTreeSet<UnitId>,
    /// Quest seams: setup result, out-of-town spawn result,
    /// portal coordinates, drehya walk gate.
    portal_setup_fails: bool,
    portal_spawn_ok: bool,
    portal: Option<(i32, i32)>,
    drehya_wait: bool,
    /// Path stops (`stop_path` calls).
    stops: u32,
    /// Unit stats by (unit, stat) for `stat` / `set_stat`.
    stats: BTreeMap<(UnitId, u16), i32>,
    /// Life percent per unit, overriding `life` (Fetish reads T's).
    life_of: BTreeMap<UnitId, i32>,
    path_target: Option<UnitId>,
    footprint: bool,
    evil_monster: Option<UnitId>,
    /// Npc class-case quest answers.
    jerhyn: Option<(i32, i32, bool)>,
    alkor_bird: bool,
    ormus_altar: Option<(i32, i32)>,
    cain_town: Option<(i32, i32)>,
    /// The Act II–V seams (`AiActs`).
    x: Acts,
    /// The `ai-bodies-6/7` seams (`AiSummons`).
    y: Summ,
}

/// Knobs of the `AiSummons` fake; actions go to `Fake::log`.
#[derive(Default)]
struct Summ {
    final_point: BTreeMap<UnitId, (i32, i32)>,
    target_point: BTreeMap<UnitId, (i32, i32)>,
    dir: i32,
    coord: BTreeMap<(i32, i32), i32>,
    free_spot: Option<(i32, i32)>,
    free_point: Option<(i32, i32)>,
    room_box: Option<(i32, i32, i32, i32)>,
    dead_at: BTreeSet<(i32, i32)>,
    slot: Option<i32>,
    trap_kind: Option<i32>,
    regions: Vec<i32>,
    history: (usize, [(i32, i32); 20]),
    last_placed: (i32, i32),
    pets: Vec<UnitId>,
    pet_count: i32,
    /// Result (consulted only when `pettype_count` makes the
    /// skill's pettype valid).
    pet_type: i32,
    pettype_count: i32,
    hire_id: Option<i32>,
    hire_row: Option<HireRow>,
    calc: i32,
    entry_mode: BTreeMap<i32, u8>,
    unit_skills: Vec<(i32, i32)>,
    skill_list: bool,
    class_skills: Vec<i32>,
    missile: Option<UnitId>,
    corpse: Option<UnitId>,
    group_active: bool,
    pgsv: bool,
    hooks: BTreeSet<String>,
    palace: Option<(i32, i32)>,
    wanderer: Option<(i32, i32)>,
    portal: Option<Option<UnitId>>,
    blocked6: bool,
    path_points: bool,
}

/// Knobs of the `AiActs` fake; actions go to `Fake::log`.
#[derive(Default)]
struct Acts {
    flags: BTreeMap<UnitId, u32>,
    max_life: BTreeMap<UnitId, i32>,
    max_mana: BTreeMap<UnitId, i32>,
    groups: BTreeSet<(UnitId, u8)>,
    states_count: i32,
    cursed: BTreeSet<UnitId>,
    hostile: BTreeSet<UnitId>,
    owners: BTreeMap<UnitId, UnitId>,
    owner_record: Option<(i32, u32)>,
    quest_flag: bool,
    portal_guid: Option<u32>,
    component: u8,
    target_unit: Option<UnitId>,
    chain_index: i32,
    skill_level: BTreeMap<i32, i32>,
    skill_entry: BTreeMap<i32, (i32, u8)>,
    hand: BTreeMap<(UnitId, bool), (i32, i32)>,
    param_ok: bool,
    check_fails: bool,
    corpse: Option<UnitId>,
    pattern: i32,
    place_ok: bool,
    point_collides: bool,
    pattern_collides: bool,
    free_point: Option<(i32, i32)>,
    free_spot: Option<(i32, i32)>,
    room_at: Option<RoomId>,
    path_points: bool,
    direction: i32,
    spawn: Option<UnitId>,
    wisps: Vec<UnitId>,
    waves: BTreeMap<i32, (i32, i32)>,
    quests: BTreeSet<String>,
}

impl Fake {
    fn modes(&self) -> Vec<String> {
        self.log
            .iter()
            .filter(|s| s.starts_with("mode"))
            .cloned()
            .collect()
    }
}

impl AiUnits for Fake {
    fn rules(&self) -> &crate::rules::Ruleset {
        &self.rules
    }
    fn seed(&mut self, unit: UnitId) -> &mut Seed {
        self.seeds.entry(unit).or_default()
    }
    fn class(&self, unit: UnitId) -> i32 {
        self.class.get(&unit).copied().unwrap_or(0)
    }
    fn anim_mode(&self, unit: UnitId) -> u8 {
        self.anim.get(&unit).copied().unwrap_or(mode::NEUTRAL)
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.states.contains(&(unit, state))
    }
    fn clear_uninterruptable(&mut self, _: &mut Game, unit: UnitId) {
        self.states.remove(&(unit, state::UNINTERRUPTABLE));
    }
    fn is_dead(&self, unit: UnitId) -> bool {
        self.dead.contains(&unit)
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or((100, 100))
    }
    fn size(&self, _: UnitId) -> i32 {
        1
    }
    fn act(&self, unit: UnitId) -> u8 {
        self.acts.get(&unit).copied().unwrap_or(0)
    }
    fn level_id(&self, _: &Game, _: UnitId) -> i32 {
        self.level
    }
    fn monster_level(&self, _: UnitId) -> i32 {
        5
    }
    fn life_percent(&self, unit: UnitId) -> i32 {
        self.life_of.get(&unit).copied().unwrap_or(self.life)
    }
    fn add_life(&mut self, _: UnitId, amount: i32) {
        self.log.push(format!("life {amount}"));
    }
    fn ai_state(&self, _: UnitId) -> u32 {
        self.ai_state
    }
    fn alignment(&self, _: UnitId) -> u8 {
        self.align
    }
    fn is_unique(&self, _: UnitId) -> bool {
        self.unique
    }
    fn is_champion(&self, _: UnitId) -> bool {
        self.champion
    }
    fn is_boss(&self, _: UnitId) -> bool {
        false
    }
    fn vision_seen(&self, _: UnitId) -> Option<u32> {
        self.vision
    }
    fn mark_seen(&mut self, _: UnitId, v: u32) {
        self.marks.push(v);
    }
    fn ai_reset(&mut self, unit: UnitId) {
        self.log.push(format!("reset {}", unit.0));
    }
    fn interacting(&self, _: UnitId) -> bool {
        self.interacting
    }
    fn busy(&self, unit: UnitId) -> bool {
        self.busy.contains(&unit)
    }
    fn has_interaction_block(&self, _: UnitId) -> bool {
        self.npc_block
    }
    fn in_interaction_list(&self, _: UnitId, player: UnitId) -> bool {
        self.npc_list.contains(&player)
    }
    fn set_life(&mut self, unit: UnitId, value: i32) {
        self.log.push(format!("setlife {} {value}", unit.0));
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.stats.get(&(unit, stat)).copied().unwrap_or(0)
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.stats.insert((unit, stat), value);
    }
    fn set_unit_flag(&mut self, _: UnitId, mask: u32) {
        self.log.push(format!("flag {mask:#x}"));
    }
    fn set_state(&mut self, _: &mut Game, unit: UnitId, state: u16, on: bool) {
        if on {
            self.states.insert((unit, state));
        } else {
            self.states.remove(&(unit, state));
        }
    }
    fn path_target(&self, _: UnitId) -> Option<UnitId> {
        self.path_target
    }
}

impl AiModes for Fake {
    fn change_mode(&mut self, game: &mut Game, unit: UnitId, m: u8, target: ModeTarget) -> bool {
        self.log.push(format!("mode {m} {target:?}"));
        let point_fails =
            matches!(target, ModeTarget::Point(x, y) if self.fail_points.contains(&(x, y)));
        if (self.walk_fails && matches!(m, mode::WALK | mode::RUN))
            || self.fail_modes.contains(&m)
            || point_fails
        {
            if let Some(n) = self.fail_think {
                if pending_think(game, unit) <= game.frame {
                    let at = game.frame + n;
                    game.schedule_event(unit, 2, at, None, 0, 0).unwrap();
                }
            }
            return false;
        }
        self.anim.insert(unit, m);
        true
    }
    fn set_anim_mode(&mut self, _: &mut Game, unit: UnitId, m: u8) {
        self.anim.insert(unit, m);
    }
    fn set_path_steps(&mut self, _: UnitId, steps: i32) {
        self.log.push(format!("steps {steps}"));
    }
    fn path_blocked(&self, _: UnitId) -> bool {
        self.blocked_path
    }
    fn stop_path(&mut self, _: UnitId) {
        self.stops += 1;
    }
    fn set_current_skill(&mut self, _: UnitId, skill: i32) -> bool {
        self.log.push(format!("skill {skill}"));
        skill >= 0
    }
    fn set_skill_flag(&mut self, _: UnitId) {
        self.log.push("skillflag".into());
    }
    fn class_has_mode(&self, _: i32, m: u8) -> bool {
        !(self.can_walk_off && m == mode::WALK)
    }
    fn play_sound(&mut self, _: &mut Game, _: UnitId, sound: u32, to: Option<UnitId>) {
        self.log.push(format!("sound {sound}"));
        if let Some(to) = to {
            self.log.push(format!("sound-to {to:?}"));
        }
    }
    fn knockback_to_gethit(&mut self, _: &mut Game, _: UnitId) {
        self.log.push("gethit".into());
    }
    fn walk_in_radius(
        &mut self,
        _: &mut Game,
        _: UnitId,
        _: UnitId,
        a: i32,
        b: i32,
        _: &mut VelocityRequest,
    ) -> bool {
        self.log.push(format!("radius {a} {b}"));
        true
    }
    fn operate_door(&mut self, _: &mut Game, _: UnitId, door: UnitId) {
        self.log.push(format!("door {door:?}"));
    }
    fn start_overlay(&mut self, _: UnitId, overlay: i32) {
        self.log.push(format!("overlay {overlay}"));
    }
    fn set_facing(&mut self, _: UnitId, dir: i32) {
        self.log.push(format!("facing {dir}"));
    }
}

impl AiWorld for Fake {
    fn in_town(&self, _: &Game, room: RoomId) -> bool {
        self.town.contains(&room)
    }
    fn los_draw(&self, _: &Game, _: RoomId) -> bool {
        !self.no_los_draw
    }
    fn collides(&self, _: &Game, _: UnitId, mask: u16) -> bool {
        self.collides && mask == 0x40
    }
    fn line_blocked(&self, _: &Game, _: UnitId, b: UnitId) -> bool {
        self.line_blocked.contains(&b)
    }
    fn in_melee_range(&self, _: &Game, _: UnitId, b: UnitId) -> bool {
        self.melee.contains(&b)
    }
    fn can_reach_directly(&self, _: &Game, _: UnitId, _: UnitId) -> bool {
        !self.reach_fails
    }
    fn find_spot(&mut self, _: &mut Game, _: UnitId) -> Option<(i32, i32, RoomId)> {
        self.log.push("find_spot".into());
        self.spot
    }
    fn last_dead(&self, _: &Game, room: RoomId) -> [Option<UnitId>; 4] {
        self.last_dead.get(&room).copied().unwrap_or([None; 4])
    }
    fn footprint_ok(&self, _: &Game, class: i32, _: Option<RoomId>, x: i32, y: i32) -> bool {
        let _ = (class, x, y);
        self.footprint
    }
}

impl AiTargets for Fake {
    fn target_nodes(&self, _: &Game) -> crate::game::TargetLists {
        crate::game::TargetLists::from_codes(&self.nodes)
    }
    fn forced_target(
        &mut self,
        _: &mut Game,
        _: UnitId,
        _: bool,
        _: bool,
        _: &AiStore,
    ) -> Option<(UnitId, i32)> {
        self.forced
    }
    fn good_target_search(
        &mut self,
        _: &mut Game,
        unit: UnitId,
        los: bool,
        _: &AiStore,
    ) -> Option<(UnitId, i32)> {
        self.log.push(format!("good {los}"));
        self.good_for.get(&unit).copied().or(self.good)
    }
    fn choose_alternative(
        &mut self,
        _: &mut Game,
        _: UnitId,
        main: Option<(UnitId, i32)>,
        alt: (UnitId, i32),
        _: &AiStore,
    ) -> Option<(UnitId, i32)> {
        self.log.push(format!("alt {}", alt.0 .0));
        if self.take_alt {
            Some(alt)
        } else {
            main
        }
    }
    fn secondary_target(
        &mut self,
        _: &mut Game,
        _: UnitId,
        _: &AiStore,
    ) -> (Option<UnitId>, i32, bool) {
        match self.secondary {
            Some((s, d)) => (Some(s), d, false),
            None => (None, 0x7FFF_FFFF, false),
        }
    }
    fn nearest_player(&mut self, _: &mut Game, unit: UnitId) -> (UnitId, bool) {
        self.nearest.unwrap_or((unit, false))
    }
    fn find_door(&mut self, _: &mut Game, _: UnitId) -> Option<UnitId> {
        self.door.map(|(d, _)| d)
    }
    fn door_monster_ok(&self, door: UnitId) -> bool {
        self.door == Some((door, true))
    }
    fn special_walk_target(&mut self, _: &mut Game, _: UnitId) -> Option<(UnitId, i32)> {
        self.log.push("scan 11".into());
        self.special_walk
    }
    fn nearest_evil_monster(&mut self, _: &mut Game, _: UnitId) -> Option<UnitId> {
        self.log.push("help scan".into());
        self.evil_monster
    }
}

impl AiQuests for Fake {
    fn portal_setup(&mut self, _: &mut Game, _: UnitId, npc: PortalNpc) -> bool {
        self.log.push(format!("quest setup {npc:?}"));
        !self.portal_setup_fails
    }
    fn spawn_town_portal(&mut self, _: &mut Game, _: UnitId, npc: PortalNpc) {
        self.log.push(format!("quest town portal {npc:?}"));
    }
    fn spawn_outside_portal(&mut self, _: &mut Game, _: UnitId, npc: PortalNpc) -> bool {
        self.log.push(format!("quest outside portal {npc:?}"));
        self.portal_spawn_ok
    }
    fn portal_coords(&mut self, _: &mut Game, _: UnitId, npc: PortalNpc) -> Option<(i32, i32)> {
        self.log.push(format!("quest coords {npc:?}"));
        self.portal
    }
    fn drehya_update(&mut self, _: &mut Game) {
        self.log.push("quest drehya update".into());
    }
    fn drehya_wait(&mut self, _: &mut Game) -> bool {
        self.log.push("quest drehya wait".into());
        self.drehya_wait
    }
    fn jerhyn_palace_active(&mut self, _: &mut Game) -> bool {
        self.jerhyn.is_some()
    }
    fn jerhyn_npc_state(
        &mut self,
        _: &mut Game,
        _: UnitId,
    ) -> crate::world::quests::act2::q4::JerhynStep {
        let (a, b) = self.jerhyn.map_or((0, 0), |(a, b, _)| (a, b));
        crate::world::quests::act2::q4::JerhynStep::Out(a, b)
    }
    fn jerhyn_placed(&mut self, _: &mut Game) {
        self.log.push("quest jerhyn placed".into());
    }
    fn guard_moving(&mut self, _: &mut Game, _: UnitId) -> bool {
        self.jerhyn.is_some_and(|j| j.2)
    }
    fn alkor_bird(&mut self, _: &mut Game) -> bool {
        self.alkor_bird
    }
    fn alkor_reset(&mut self, _: &mut Game) {
        self.log.push("quest alkor reset".into());
    }
    fn ormus_altar(&mut self, _: &mut Game) -> Option<(i32, i32)> {
        self.ormus_altar
    }
    fn ormus_set_altar_mode(&mut self, _: &mut Game) {
        self.log.push("quest ormus altar".into());
    }
    fn cain_town_coords(&mut self, _: &mut Game, _: UnitId) -> Option<(i32, i32)> {
        self.cain_town
    }
    fn cain_in_town_activated(&mut self, _: &mut Game, _: UnitId) {
        self.log.push("quest cain activated".into());
    }
    fn anya_open_portal(&mut self, _: &mut Game, _: UnitId) {
        self.log.push("quest anya portal".into());
    }
}

impl AiActs for Fake {
    fn unit_flags(&self, unit: UnitId) -> u32 {
        self.x.flags.get(&unit).copied().unwrap_or(0)
    }
    fn clear_unit_flag(&mut self, unit: UnitId, mask: u32) {
        self.log.push(format!("unflag {mask:#x}"));
        *self.x.flags.entry(unit).or_default() &= !mask;
    }
    fn max_life(&self, unit: UnitId) -> i32 {
        self.x.max_life.get(&unit).copied().unwrap_or(0)
    }
    fn max_mana(&self, unit: UnitId) -> i32 {
        self.x.max_mana.get(&unit).copied().unwrap_or(0)
    }
    fn has_state_group(&self, unit: UnitId, g: u8) -> bool {
        self.x.groups.contains(&(unit, g))
    }
    fn states_count(&self) -> i32 {
        self.x.states_count
    }
    fn has_list_flag(&self, unit: UnitId, flags: u32) -> bool {
        flags == 0x20 && self.x.cursed.contains(&unit)
    }
    fn hostile(&self, _: &Game, _: UnitId, b: UnitId) -> bool {
        self.x.hostile.contains(&b)
    }
    fn owner(&self, _: &Game, unit: UnitId) -> Option<UnitId> {
        self.x.owners.get(&unit).copied()
    }
    fn owner_record(&self, _: UnitId) -> Option<(i32, u32)> {
        self.x.owner_record
    }
    fn quest_flag(&self, _: UnitId, _: u8, quest: i32, flag: i32) -> bool {
        self.x.quest_flag && quest == 21 && flag == 0
    }
    fn portal_guid(&self, _: UnitId) -> Option<u32> {
        self.x.portal_guid
    }
    fn component(&self, _: UnitId, i: usize) -> u8 {
        if i == 10 {
            self.x.component
        } else {
            0
        }
    }
    fn target_unit(&self, _: &Game, _: UnitId) -> Option<UnitId> {
        self.x.target_unit
    }
    fn set_target_override(&mut self, _: UnitId, kind: i32, guid: u32) {
        self.log.push(format!("override {kind} {guid}"));
    }
    fn chain_index(&self, _: i32) -> i32 {
        self.x.chain_index
    }
    fn class_for_level(&self, _: &Game, _: Option<RoomId>, class: i32) -> i32 {
        class + 1000
    }
    fn skill_level(&self, _: UnitId, skill: i32, _: bool) -> Option<i32> {
        self.x.skill_level.get(&skill).copied()
    }
    fn skill_entry(&self, _: UnitId, skill: i32) -> Option<(i32, u8)> {
        self.x.skill_entry.get(&skill).copied()
    }
    fn hand_skill(&self, unit: UnitId, right: bool) -> Option<(i32, i32)> {
        self.x.hand.get(&(unit, right)).copied()
    }
    fn add_right_skill(&mut self, _: &mut Game, unit: UnitId, skill: i32, level: i32) {
        self.log.push(format!("aura {skill} {level}"));
        self.x.hand.insert((unit, true), (skill, level));
    }
    fn assign_skill(&mut self, _: &mut Game, _: UnitId, skill: i32, level: i32) {
        self.log.push(format!("assign {skill} {level}"));
    }
    fn set_skill_param(&mut self, _: UnitId, skill: i32, value: i32) -> bool {
        self.log.push(format!("skillparam {skill} {value}"));
        self.x.param_ok
    }
    fn skill_check(
        &mut self,
        _: &mut Game,
        _: UnitId,
        skill: i32,
        target: Option<UnitId>,
        x: i32,
        y: i32,
    ) -> bool {
        self.log.push(format!("check {skill} {target:?} {x} {y}"));
        !self.x.check_fails
    }
    fn corpse_search(
        &mut self,
        _: &mut Game,
        _: UnitId,
        _: Option<UnitId>,
        skill: i32,
        level: i32,
    ) -> Option<UnitId> {
        self.log.push(format!("corpse search {skill} {level}"));
        self.x.corpse
    }
    fn path_pattern(&self, _: UnitId) -> i32 {
        self.x.pattern
    }
    fn set_path_pattern(&mut self, _: UnitId, pattern: i32) {
        self.log.push(format!("pattern {pattern}"));
        self.x.pattern = pattern;
    }
    fn set_move_mask(&mut self, _: UnitId, mask: u16) {
        self.log.push(format!("movemask {mask:#x}"));
    }
    fn place_unit(&mut self, _: &mut Game, _: UnitId, _: Option<RoomId>, x: i32, y: i32) -> bool {
        self.log.push(format!("place {x} {y}"));
        self.x.place_ok
    }
    fn stamp_pattern(
        &mut self,
        _: &mut Game,
        _: Option<RoomId>,
        x: i32,
        y: i32,
        pattern: i32,
        mask: u16,
    ) {
        self.log.push(format!("stamp {x} {y} {pattern} {mask:#x}"));
    }
    fn clear_cell(&mut self, _: &mut Game, _: Option<RoomId>, x: i32, y: i32, bits: u16) {
        self.log.push(format!("clearcell {x} {y} {bits:#x}"));
    }
    fn point_collides(&self, _: &Game, _: Option<RoomId>, _: i32, _: i32, _: u16) -> bool {
        self.x.point_collides
    }
    fn pattern_collides(&self, _: &Game, _: UnitId, _: i32, _: u16) -> bool {
        self.x.pattern_collides
    }
    fn free_point(
        &mut self,
        _: &mut Game,
        _: Option<RoomId>,
        x: i32,
        y: i32,
        size: i32,
    ) -> Option<(i32, i32)> {
        self.log.push(format!("freepoint {x} {y} {size}"));
        self.x.free_point
    }
    fn free_spot_for(
        &mut self,
        _: &mut Game,
        _: UnitId,
        class: i32,
        x: i32,
        y: i32,
    ) -> Option<(i32, i32)> {
        self.log.push(format!("freespot {class} {x} {y}"));
        self.x.free_spot
    }
    fn room_at(&self, _: &Game, _: UnitId, _: i32, _: i32) -> Option<RoomId> {
        self.x.room_at
    }
    fn move_in_radius(
        &mut self,
        _: &mut Game,
        _: UnitId,
        target: UnitId,
        mode: u8,
        a: i32,
        b: i32,
    ) -> bool {
        self.log
            .push(format!("mode-radius {mode} {} {a} {b}", target.0));
        true
    }
    fn set_path_target(&mut self, _: UnitId, target: UnitId) {
        self.log.push(format!("pathtarget {}", target.0));
    }
    fn path_has_points(&mut self, _: &mut Game, _: UnitId, target: UnitId) -> bool {
        self.log.push(format!("pathcompute {}", target.0));
        self.x.path_points
    }
    fn direction64(&self, _: UnitId, _: UnitId) -> i32 {
        self.x.direction
    }
    fn stop_unit_path(&mut self, _: UnitId) {
        self.stops += 1;
    }
    fn spawn_monster(
        &mut self,
        _: &mut Game,
        _: RoomId,
        x: i32,
        y: i32,
        class: i32,
        m: u8,
        spread: i32,
        flags: u32,
    ) -> Option<UnitId> {
        self.log
            .push(format!("spawn {class} {x} {y} {m} {spread} {flags:#x}"));
        self.x.spawn
    }
    fn kill(&mut self, _: &mut Game, unit: UnitId, killer: Option<UnitId>) {
        self.log
            .push(format!("kill {} {:?}", unit.0, killer.map(|k| k.0)));
    }
    fn remove_unit(&mut self, _: &mut Game, unit: UnitId) {
        self.log.push(format!("remove {}", unit.0));
    }
    fn link_clone(&mut self, _: &mut Game, _: UnitId, clone: UnitId) {
        self.log.push(format!("link {}", clone.0));
    }
    fn reinit_class(&mut self, _: &mut Game, _: UnitId, class: i32, m: u8) {
        self.log.push(format!("reinit {class} {m}"));
    }
    fn change_class_list(&mut self, _: &mut Game, _: UnitId, class: i32) {
        self.log.push(format!("classlist {class}"));
    }
    fn wisp_buff(&mut self, _: &mut Game, target: UnitId, value: i32, expire: i32) {
        self.log.push(format!("buff {} {value} {expire}", target.0));
    }
    fn preload_class(&mut self, _: &mut Game, _: UnitId, class: i32) {
        self.log.push(format!("preload {class}"));
    }
    fn wisp_find(&mut self, _: &mut Game, _: UnitId) -> Vec<UnitId> {
        self.x.wisps.clone()
    }
    fn wave(&self, w: i32) -> Option<(i32, i32)> {
        self.x.waves.get(&w).copied()
    }
    fn clear_room_portal_flag(&mut self, _: &mut Game, _: Option<RoomId>) {
        self.log.push("portalflag".into());
    }
    fn quest_call(&mut self, _: &mut Game, _: UnitId, call: QuestCall) -> bool {
        let name = format!("{call:?}");
        self.log.push(format!("quest {name}"));
        self.x.quests.contains(&name)
    }
}

impl AiSummons for Fake {
    fn path_final_point(&self, unit: UnitId) -> (i32, i32) {
        self.y.final_point.get(&unit).copied().unwrap_or((0, 0))
    }
    fn path_target_point(&self, unit: UnitId) -> (i32, i32) {
        self.y.target_point.get(&unit).copied().unwrap_or((0, 0))
    }
    fn set_path_target_point(&mut self, _: UnitId, x: i32, y: i32) {
        self.log.push(format!("pathpoint {x} {y}"));
    }
    fn direction64_to(&self, _: UnitId, _: i32, _: i32) -> i32 {
        self.y.dir
    }
    fn snap_direction(&mut self, _: UnitId, dir: i32) {
        self.log.push(format!("snap {dir}"));
    }
    fn line_blocked_mask(&self, _: &Game, _: UnitId, _: UnitId, mask: u16) -> bool {
        mask == 6 && self.y.blocked6
    }
    fn coord_index(&self, _: &Game, _: Option<RoomId>, x: i32, y: i32) -> i32 {
        self.y.coord.get(&(x, y)).copied().unwrap_or(0)
    }
    fn free_spot(
        &mut self,
        _: &mut Game,
        _: Option<RoomId>,
        cl: i32,
        class: i32,
    ) -> Option<(i32, i32)> {
        self.log.push(format!("freespot2 {cl} {class}"));
        self.y.free_spot
    }
    fn free_point_masked(
        &mut self,
        _: &mut Game,
        _: Option<RoomId>,
        x: i32,
        y: i32,
        size: i32,
        mask: u16,
        n: i32,
    ) -> Option<(i32, i32)> {
        self.log
            .push(format!("freepoint2 {x} {y} {size} {mask:#x} {n}"));
        self.y.free_point
    }
    fn room_box(&self, _: &Game, _: RoomId) -> Option<(i32, i32, i32, i32)> {
        self.y.room_box
    }
    fn class_dead_at(&self, _: &Game, _: RoomId, x: i32, y: i32, _: i32) -> bool {
        self.y.dead_at.contains(&(x, y))
    }
    fn target_slot(&self, _: UnitId) -> i32 {
        self.y.slot.unwrap_or(11)
    }
    fn register_target_node(&mut self, _: &mut Game, _: UnitId, slot: i32) {
        self.log.push(format!("node {slot}"));
        self.y.slot = Some(slot);
    }
    fn set_unit_flags2(&mut self, _: UnitId, mask: u32) {
        self.log.push(format!("flag2 {mask:#x}"));
    }
    fn trap_kind(&self, _: &Game, _: UnitId) -> i32 {
        self.y.trap_kind.unwrap_or(-1)
    }
    fn set_trap_kind(&mut self, _: &mut Game, _: UnitId, kind: i32) {
        self.log.push(format!("trapkind {kind}"));
        self.y.trap_kind = Some(kind);
    }
    fn region_classes(&self, _: &Game, _: UnitId) -> Vec<i32> {
        self.y.regions.clone()
    }
    fn position_history(&self, _: UnitId) -> (usize, [(i32, i32); 20]) {
        self.y.history
    }
    fn last_placed_point(&self, _: UnitId) -> (i32, i32) {
        self.y.last_placed
    }
    fn pets(&self, _: &Game, _: UnitId) -> Vec<UnitId> {
        self.y.pets.clone()
    }
    fn pet_count(&self, _: UnitId) -> i32 {
        self.y.pet_count
    }
    fn pet_type_of(&self, _: &Game, _: UnitId, _: UnitId) -> i32 {
        self.y.pet_type
    }
    fn pettype_count(&self) -> i32 {
        self.y.pettype_count
    }
    fn hireling_id(&self, _: &Game, _: UnitId, _: UnitId) -> Option<i32> {
        self.y.hire_id
    }
    fn hireling_row(&self, _: &Game, _: i32, _: i32) -> Option<HireRow> {
        self.y.hire_row
    }
    fn skill_calc(&mut self, _: &mut Game, _: UnitId, skill: i32, _: u32, level: i32) -> i32 {
        let _ = (skill, level);
        self.y.calc
    }
    fn entry_mode(&self, _: UnitId, skill: i32) -> Option<u8> {
        self.y.entry_mode.get(&skill).copied()
    }
    fn unit_skills(&self, _: UnitId) -> Vec<(i32, i32)> {
        self.y.unit_skills.clone()
    }
    fn has_skill_list(&self, _: UnitId) -> bool {
        self.y.skill_list
    }
    fn class_skills(&self, _: i32) -> Vec<i32> {
        self.y.class_skills.clone()
    }
    fn make_right_skill(&mut self, _: &mut Game, _: UnitId, skill: i32) {
        self.log.push(format!("rightskill {skill}"));
    }
    fn set_hand_skill(&mut self, _: UnitId, skill: i32, right: bool) {
        self.log.push(format!("hand {skill} {right}"));
    }
    fn skill_missile(
        &mut self,
        _: &mut Game,
        _: UnitId,
        skill: i32,
        level: i32,
        missile: i32,
        x: i32,
        y: i32,
    ) -> Option<UnitId> {
        self.log
            .push(format!("missile {skill} {level} {missile} {x} {y}"));
        self.y.missile
    }
    fn link_owner(&mut self, _: &mut Game, a: UnitId, b: UnitId) {
        self.log.push(format!("link {} {}", a.0, b.0));
    }
    fn corpse_find(&mut self, _: &mut Game, _: UnitId, n: i32) -> Option<UnitId> {
        self.log.push(format!("corpsefind {n}"));
        self.y.corpse
    }
    fn state_group_active(&self, _: UnitId, _: i32) -> bool {
        self.y.group_active
    }
    fn has_pgsv_state(&self, _: UnitId) -> bool {
        self.y.pgsv
    }
    fn quest_hook(&mut self, _: &mut Game, _: UnitId, _: Option<UnitId>, hook: QuestHook) -> bool {
        let name = format!("{hook:?}");
        self.log.push(format!("hook {name}"));
        self.y.hooks.contains(&name)
    }
    fn palace_guard_point(&mut self, _: &mut Game, x: i32, y: i32) -> (bool, i32, i32) {
        match self.y.palace {
            Some((px, py)) => (true, px, py),
            None => (false, x, y),
        }
    }
    fn dark_wanderer_target(&mut self, _: &mut Game, _: UnitId) -> Option<(i32, i32)> {
        self.y.wanderer
    }
    fn rescue_portal(&mut self, _: &mut Game, _: UnitId) -> Option<Option<UnitId>> {
        self.y.portal
    }
    fn npc_wants_interact(&mut self, _: &mut Game, _: UnitId, _: UnitId) -> bool {
        self.log.push("0x8a".into());
        true
    }
    fn path_has_points_no_target(&mut self, _: &mut Game, _: UnitId) -> bool {
        self.y.path_points
    }
}

/// A monstats row using AI `ai` with Normal aip1..aip5 and `aidel`.
fn monstats(ai: u16, aips: [i16; 5], aidel: u8) -> Monstats {
    let mut r = Monstats::decode(&vec![0u8; Monstats::SIZE]);
    r.ai = ai;
    r.aidel = aidel;
    r.aip1 = aips[0] as u16;
    r.aip2 = aips[1] as u16;
    r.aip3 = aips[2] as u16;
    r.aip4 = aips[3] as u16;
    r.aip5 = aips[4] as u16;
    r.skill1 = 0xFFFF;
    r.skill2 = 0xFFFF;
    r.skill3 = 0xFFFF;
    r
}

struct World {
    game: Game,
    fake: Fake,
    store: AiStore,
    monstats: Vec<Monstats>,
    monstats2: Vec<Monstats2>,
    levels: Vec<Levels>,
    modes: Vec<[u8; 8]>,
    skills: Vec<Skills>,
    missiles: Vec<Missiles>,
    room: RoomId,
    mon: UnitId,
    player: UnitId,
}

impl World {
    fn new(row: Monstats) -> Self {
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let room = game.lists.create_room(0).unwrap();
        let mon = game
            .spawn_unit(UnitType::Monster, Some(room), false)
            .unwrap();
        let player = game.spawn_unit(UnitType::Player, Some(room), true).unwrap();
        let mut fake = Fake {
            life: 100,
            ..Fake::default()
        };
        fake.pos.insert(mon, (100, 100));
        fake.pos.insert(player, (105, 100));
        let mut store = AiStore::new();
        store.entry(mon).control = Some(AiControl::default());
        let mut w = Self {
            game,
            fake,
            store,
            monstats: vec![row],
            monstats2: vec![Monstats2::decode(&vec![0u8; Monstats2::SIZE])],
            levels: vec![Levels::decode(&vec![0u8; Levels::SIZE])],
            modes: vec![[0; 8]],
            skills: Vec::new(),
            missiles: Vec::new(),
            room,
            mon,
            player,
        };
        let mon = w.mon;
        w.with(|g, cx| install(g, cx, mon, 0));
        w
    }

    fn with<R>(&mut self, f: impl FnOnce(&mut Game, &mut Ctx<'_, Fake>) -> R) -> R {
        let mut cx = Ctx {
            tables: AiTables {
                monstats: &self.monstats,
                monstats2: &self.monstats2,
                levels: &self.levels,
                skill_modes: &self.modes,
                skills: &self.skills,
                missiles: &self.missiles,
            },
            info: GameInfo::default(),
            store: &mut self.store,
            world: &mut self.fake,
        };
        f(&mut self.game, &mut cx)
    }

    fn seed(&mut self, lo: u32) {
        self.fake.seeds.insert(self.mon, Seed::init_low(lo));
    }

    /// Runs the current AI function with a target at distance `d`.
    fn run(&mut self, combat: bool, d: i32) {
        let p = TickParam {
            target: Some(self.player),
            distance: d,
            combat,
            class: 0,
            class2: 0,
        };
        let mon = self.mon;
        self.with(|g, cx| {
            let f = cx.store.control(mon).unwrap().function;
            run_function(g, cx, f, mon, &p)
        });
    }

    /// Next think frames of the monster.
    fn thinks(&self) -> Vec<i32> {
        self.game
            .timers
            .unit_timers(self.mon)
            .into_iter()
            .filter(|&t| self.game.timers.event(t).map(|e| e.0) == Some(EVENT_THINK))
            .filter_map(|t| self.game.timers.expire(t))
            .collect()
    }
}

#[test]
fn draw_vectors() {
    let want = [
        [51, 31, 12, 93],
        [87, 64, 71, 25],
        [53, 46, 20, 96],
        [0, 42, 13, 80],
    ];
    for (s, w) in SEEDS.iter().zip(want) {
        let mut seed = Seed::init_low(*s);
        let got: Vec<u32> = (0..4).map(|_| seed.step() % 100).collect();
        assert_eq!(got, w, "seed {s}");
    }
}

#[test]
fn wander_vectors() {
    let want = [
        ((103, 100), (3_064_641_593, 280_084_454)),
        ((97, 98), (4_008_788_125, 674_806_599)),
        ((103, 100), (4_097_238_796, 611_865_796)),
        ((98, 103), (4_002_178_480, 452_242_291)),
    ];
    for (s, (pt, (lo, hi))) in SEEDS.iter().zip(want) {
        let mut seed = Seed::init_low(*s);
        assert_eq!(wander_point(&mut seed, (100, 100), 3), pt, "seed {s}");
        assert_eq!(seed, Seed::new(lo, hi), "seed {s}");
    }
}

fn last_mode(w: &World) -> String {
    w.fake.modes().last().cloned().unwrap_or_default()
}

fn walk_point(x: i32, y: i32) -> String {
    format!("mode 2 Point({x}, {y})")
}

#[test]
fn zombie_vectors() {
    let row = || monstats(3, [30, 10, 0, 20, 0], 15);
    let a1 = |w: &World| format!("mode 4 Unit({:?})", w.player);
    let a2 = |w: &World| format!("mode 5 Unit({:?})", w.player);
    // C.
    for (s, want_a1) in SEEDS.iter().zip([false, false, false, true]) {
        let mut w = World::new(row());
        w.seed(*s);
        w.run(true, 1);
        let want = if want_a1 { a1(&w) } else { a2(&w) };
        assert_eq!(last_mode(&w), want, "seed {s}");
    }
    // Not C, D = 5.
    let want = [
        Some((97, 98)),
        Some((99, 103)),
        Some((100, 103)),
        None, // run to T
    ];
    for (s, want) in SEEDS.iter().zip(want) {
        let mut w = World::new(row());
        w.seed(*s);
        w.run(false, 5);
        match want {
            Some((x, y)) => assert_eq!(last_mode(&w), walk_point(x, y), "seed {s}"),
            None => {
                assert_eq!(last_mode(&w), format!("mode 15 Unit({:?})", w.player));
                assert_eq!(w.store.get(w.mon).unwrap().velocity.speed, 100);
            }
        }
    }
    // Not C, D = 12: no draw before the wander.
    let want = [(103, 100), (97, 98), (103, 100), (98, 103)];
    for (s, (x, y)) in SEEDS.iter().zip(want) {
        let mut w = World::new(row());
        w.seed(*s);
        w.run(false, 12);
        assert_eq!(last_mode(&w), walk_point(x, y), "seed {s}");
    }
    // Level 17 always runs; AI state 3 runs without a draw.
    let mut w = World::new(row());
    w.fake.level = 17;
    w.run(false, 12);
    assert_eq!(last_mode(&w), format!("mode 15 Unit({:?})", w.player));
    let mut w = World::new(row());
    w.fake.ai_state = 19;
    w.seed(1);
    w.run(false, 5);
    assert_eq!(w.fake.seeds[&w.mon], Seed::init_low(1));
}

#[test]
fn fallen_vectors() {
    let want = [None, None, None, Some(5)];
    for (s, want) in SEEDS.iter().zip(want) {
        let mut w = World::new(monstats(6, [30, 10, 50, 20, 0], 15));
        w.seed(*s);
        w.run(true, 1);
        match want {
            None => {
                assert!(w.fake.modes().is_empty(), "seed {s}");
                assert_eq!(w.thinks(), [10], "seed {s}");
            }
            Some(m) => {
                assert_eq!(last_mode(&w), format!("mode {m} Unit({:?})", w.player));
                assert!(w.thinks().is_empty(), "the fallen deleted its thinks");
            }
        }
    }
}

#[test]
fn brute_vectors() {
    for (s, m) in SEEDS.iter().zip([4, 5, 5, 4]) {
        let mut w = World::new(monstats(7, [0, 0, 100, 45, 0], 15));
        w.seed(*s);
        w.run(true, 1);
        assert_eq!(last_mode(&w), format!("mode {m} Unit({:?})", w.player));
    }
    // Not C: speed 100 − clamp(life%, 40, 100), method 13.
    let mut w = World::new(monstats(7, [0, 0, 100, 45, 0], 15));
    w.fake.life = 10;
    w.run(false, 9);
    let v = w.store.get(w.mon).unwrap().velocity;
    assert_eq!((v.method, v.speed), (13, 60));
    assert_eq!(last_mode(&w), format!("mode 2 Unit({:?})", w.player));
}

#[test]
fn quill_rat_vectors() {
    for (s, escape) in SEEDS.iter().zip([true, true, true, false]) {
        let mut w = World::new(monstats(14, [10, 35, 0, 2, 0], 15));
        w.seed(*s);
        w.run(false, 5);
        if escape {
            // Escape from T at (105, 100) by 2: own + sign(own − T)·2.
            assert_eq!(last_mode(&w), walk_point(98, 100), "seed {s}");
        } else {
            assert_eq!(last_mode(&w), format!("mode 5 Unit({:?})", w.player));
        }
    }
    // D ≥ aip1: wander max(aip4, 3).
    let mut w = World::new(monstats(14, [10, 35, 0, 2, 0], 15));
    w.seed(1);
    w.run(false, 10);
    assert_eq!(last_mode(&w), walk_point(103, 100));
}

#[test]
fn circle_vectors() {
    for (s, m) in SEEDS.iter().zip([5, 5, 6, 5]) {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.seed(*s);
        let (mon, pl) = (w.mon, w.player);
        w.with(|g, cx| circle(g, cx, mon, Some(pl), 4, false));
        assert_eq!(w.store.get(mon).unwrap().velocity.method, m, "seed {s}");
        assert_eq!(w.store.get(mon).unwrap().velocity.steps, 4);
    }
}

#[test]
fn corrupt_lancer_vectors() {
    for (s, walk) in SEEDS.iter().zip([true, false, true, true]) {
        let mut w = World::new(monstats(36, [60, 75, 9, 0, 15], 15));
        w.seed(*s);
        w.run(false, 10);
        if walk {
            assert_eq!(last_mode(&w), format!("mode 2 Unit({:?})", w.player));
            assert!(w.fake.log.contains(&"steps 3".to_string()), "seed {s}");
        } else {
            assert!(w.fake.modes().is_empty());
            assert_eq!(w.thinks(), [9]);
        }
    }
    // D > aip5: run with MeleeRng steps, param 0 := 1.
    let mut w = World::new(monstats(36, [60, 75, 9, 0, 15], 15));
    w.monstats2[0].meleerng = 4;
    w.run(false, 20);
    assert_eq!(last_mode(&w), format!("mode 15 Unit({:?})", w.player));
    assert_eq!(w.store.control(w.mon).unwrap().params[0], 1);
}

#[test]
fn idle_ai_thinks_every_200() {
    for ai in [1, 100] {
        let mut w = World::new(monstats(ai, [0; 5], 15));
        w.game.frame = 50;
        w.run(false, 0);
        assert_eq!(w.thinks(), [250]);
    }
}

// ---- scheduling ----------------------------------------------------

#[test]
fn neutral_start_schedules_aidel() {
    let cases = [
        (15, false, None, vec![115]),
        (0, false, None, vec![115]),
        (15, true, None, vec![145]),
        (15, false, Some(107), vec![107]),
    ];
    for (aidel, stunned, pending, want) in cases {
        let mut w = World::new(monstats(3, [0; 5], aidel));
        w.game.frame = 100;
        if stunned {
            w.fake.states.insert((w.mon, state::STUNNED));
        }
        if let Some(at) = pending {
            w.game.schedule_event(w.mon, 2, at, None, 0, 0).unwrap();
        }
        let mon = w.mon;
        w.with(|g, cx| neutral_mode_start(g, cx, mon));
        assert_eq!(w.thinks(), want);
    }
}

#[test]
fn aidel_difficulty_gate() {
    // Edge case 2: the Normal column unless game +0x6A or +0x74 is set.
    let mut row = monstats(3, [0; 5], 15);
    row.aidel_n = 14;
    let mut w = World::new(row);
    let mut cx = Ctx {
        tables: AiTables {
            monstats: &w.monstats,
            monstats2: &w.monstats2,
            levels: &w.levels,
            skill_modes: &w.modes,
            skills: &w.skills,
            missiles: &w.missiles,
        },
        info: GameInfo {
            difficulty: 1,
            ..GameInfo::default()
        },
        store: &mut w.store,
        world: &mut w.fake,
    };
    assert_eq!(cx.aidel(0), 15);
    cx.info.game_type_ex = 1;
    assert_eq!(cx.aidel(0), 14);
}

#[test]
fn target_mode_1_idle_by_distance() {
    for (d, want) in [
        (Some(40), 125),
        (Some(30), 120),
        (Some(20), 110),
        (None, 125),
    ] {
        let mut w = World::new(monstats(3, [0; 5], 15));
        w.game.frame = 100;
        if let Some(d) = d {
            w.fake.pos.insert(w.player, (100 + d, 100));
            w.fake.nodes = vec![vec![w.player]];
        }
        // aidist 0 → 35: nothing within reach at d ≥ 35; closer players
        // become targets, so test the no-target idle with aidist 1.
        w.monstats[0].aidist = 1;
        let mon = w.mon;
        w.with(|g, cx| think(g, cx, mon));
        assert_eq!(w.thinks(), [want], "d {d:?}");
    }
}

#[test]
fn target_mode_4_idles_20() {
    let mut w = World::new(monstats(15, [0; 5], 15));
    w.game.frame = 100;
    let mon = w.mon;
    w.with(|g, cx| think(g, cx, mon));
    assert_eq!(w.thinks(), [120]);
    assert!(w.fake.modes().is_empty());
}

#[test]
fn idle_helpers() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.game.frame = 100;
    let mon = w.mon;
    w.with(|g, cx| idle(g, cx, mon, 0));
    assert_eq!(w.thinks(), [101]);
    // Idle in neutral first changes a non-neutral unit's mode.
    w.fake.anim.insert(mon, mode::WALK);
    w.with(|g, cx| idle(g, cx, mon, 5));
    assert_eq!(w.thinks(), [105]);
    assert_eq!(last_mode(&w), format!("mode 1 Unit({mon:?})"));
    // Pending at +4 kept for N = 10; at +12 replaced.
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.game.frame = 100;
    let mon = w.mon;
    w.game.schedule_event(mon, 2, 104, None, 0, 0).unwrap();
    w.with(|g, cx| idle_if_later(g, cx, mon, 10));
    assert_eq!(w.thinks(), [104]);
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.game.frame = 100;
    let mon = w.mon;
    w.game.schedule_event(mon, 2, 112, None, 0, 0).unwrap();
    w.with(|g, cx| idle_if_later(g, cx, mon, 10));
    assert_eq!(w.thinks(), [110]);
}

#[test]
fn state_54_schedule_cancels_pending_thinks() {
    // Edge case 10.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.game.schedule_event(mon, 2, 50, None, 0, 0).unwrap();
    w.game.schedule_event(mon, 2, 60, None, 0, 0).unwrap();
    w.fake.states.insert((mon, state::UNINTERRUPTABLE));
    w.with(|g, cx| schedule_think(g, cx, mon, 70));
    assert_eq!(w.thinks(), [70]);
    assert!(!w.fake.has_state(mon, state::UNINTERRUPTABLE));
}

#[test]
fn update_ai_callback_rules() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.game.frame = 10;
    let mon = w.mon;
    w.game.schedule_event(mon, 2, 25, None, 0, 0).unwrap();
    w.with(|g, cx| update_ai_callback(g, cx, mon));
    assert_eq!(w.thinks(), [12]);
    for m in [mode::DEATH, mode::DEAD, mode::WALK] {
        w.fake.anim.insert(mon, m);
        w.with(|g, cx| update_ai_callback(g, cx, mon));
        assert!(w.thinks().is_empty(), "mode {m}");
    }
    // Base class 110 (vulture1) also in other modes.
    w.monstats[0].baseid = 110;
    w.with(|g, cx| update_ai_callback(g, cx, mon));
    assert_eq!(w.thinks(), [12]);
}

#[test]
fn creation_pair_and_room_entry() {
    // Neutral start +15, then the room entry cancels it and adds +2.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let (mon, room) = (w.mon, w.room);
    w.with(|g, cx| neutral_mode_start(g, cx, mon));
    assert_eq!(w.thinks(), [15]);
    w.with(|g, cx| update_ai_callback(g, cx, mon));
    assert_eq!(w.thinks(), [2]);
    w.game.frame = 40;
    w.with(|g, cx| client_entered_room(g, cx, room));
    assert_eq!(w.thinks(), [42]);
}

#[test]
fn knockback_end_rules() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.with(|g, cx| knockback_end(g, cx, mon));
    assert!(w.thinks().is_empty());
    assert!(w.fake.log.contains(&"gethit".to_string()));
    w.monstats[0].baseid = 78;
    w.with(|g, cx| knockback_end(g, cx, mon));
    assert_eq!(w.thinks(), [15]);
}

#[test]
fn mode_end_inline_think() {
    // Walk end: neutral, think at once (Idle AI → +200).
    let mut w = World::new(monstats(1, [0; 5], 15));
    let mon = w.mon;
    w.fake.anim.insert(mon, mode::WALK);
    w.with(|g, cx| mode_end(g, cx, mon, mode::WALK));
    assert_eq!(w.thinks(), [200]);
    // Frozen and alive: no think.
    let mut w = World::new(monstats(1, [0; 5], 15));
    let mon = w.mon;
    w.fake.states.insert((mon, state::FREEZE));
    w.with(|g, cx| mode_end(g, cx, mon, mode::RUN));
    assert!(w.thinks().is_empty());
    // An attack end requests neutral (the attack-end record): with a path
    // target unit the record targets it; without one the point (0, 0).
    let mut w = World::new(monstats(1, [0; 5], 15));
    let mon = w.mon;
    let pl = w.player;
    w.fake.path_target = Some(pl);
    w.with(|g, cx| mode_end(g, cx, mon, mode::ATTACK1));
    assert_eq!(last_mode(&w), format!("mode 1 Unit({pl:?})"));
    let mut w = World::new(monstats(1, [0; 5], 15));
    let mon = w.mon;
    w.fake.path_target = None;
    w.with(|g, cx| mode_end(g, cx, mon, 8));
    assert_eq!(last_mode(&w), "mode 1 Point(0, 0)");
}

#[test]
fn freeze_drops_thinks_and_type_10_resets() {
    let mut w = World::new(monstats(1, [0; 5], 15));
    let mon = w.mon;
    w.game.schedule_event(mon, 2, 1, None, 0, 0).unwrap();
    w.game.schedule_event(mon, 10, 1, None, 0, 0).unwrap();
    w.fake.states.insert((mon, state::FREEZE));
    struct Nothing;
    impl EventDispatch for Nothing {
        fn run_event(&mut self, _: &mut Game, _: &TimerRun) {}
    }
    w.game.frame = 1;
    let mut d = MonsterDispatch {
        cx: Ctx {
            tables: AiTables {
                monstats: &w.monstats,
                monstats2: &w.monstats2,
                levels: &w.levels,
                skill_modes: &w.modes,
                skills: &w.skills,
                missiles: &w.missiles,
            },
            info: GameInfo::default(),
            store: &mut w.store,
            world: &mut w.fake,
        },
        next: &mut Nothing,
    };
    crate::tick::run_timer_events(&mut w.game, &mut d);
    // The think was dropped (nothing rescheduled); the reset ran.
    assert!(w.thinks().is_empty());
    assert!(w.fake.log.contains(&format!("reset {}", mon.0)));
}

// ---- dispatch and targets -------------------------------------

#[test]
fn stun_idles_3() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.fake.states.insert((mon, state::STUNNED));
    w.with(|g, cx| think(g, cx, mon));
    assert_eq!(w.thinks(), [3]);
}

#[test]
fn main_search_picks_nearest_qualifying_player() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    let other = w
        .game
        .spawn_unit(UnitType::Player, Some(w.room), true)
        .unwrap();
    w.fake.pos.insert(w.player, (110, 100));
    w.fake.pos.insert(other, (104, 100));
    w.fake.nodes = vec![vec![w.player], vec![other]];
    w.fake.melee.insert(other);
    let mon = w.mon;
    let s = w.with(|g, cx| main_search(g, cx, mon));
    assert_eq!((s.target, s.distance, s.combat), (Some(other), 4, true));
    // A dead head is not chosen but still sets M.
    w.fake.dead.insert(other);
    let s = w.with(|g, cx| main_search(g, cx, mon));
    assert_eq!(s.target, Some(w.player));
    // Players in town do not count.
    let room = w.room;
    w.fake.town.insert(room);
    let s = w.with(|g, cx| main_search(g, cx, mon));
    assert_eq!((s.target, s.distance), (None, target::NO_DISTANCE));
}

#[test]
fn main_search_sees_players_past_the_eighth_list() {
    // Ten players: the nearest is in player list 9 (node code 13), which
    // the search reads like the first eight.
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mut nodes = vec![Vec::new(); 14];
    nodes[0] = vec![w.player];
    w.fake.pos.insert(w.player, (120, 100));
    let mut far = Vec::new();
    for code in (1..8).chain(12..13) {
        let p = w
            .game
            .spawn_unit(UnitType::Player, Some(w.room), true)
            .unwrap();
        w.fake.pos.insert(p, (130, 100));
        nodes[code] = vec![p];
        far.push(p);
    }
    let near = w
        .game
        .spawn_unit(UnitType::Player, Some(w.room), true)
        .unwrap();
    w.fake.pos.insert(near, (104, 100));
    w.fake.melee.insert(near);
    nodes[13] = vec![near];
    w.fake.nodes = nodes;
    let mon = w.mon;
    let s = w.with(|g, cx| main_search(g, cx, mon));
    assert_eq!((s.target, s.distance), (Some(near), 4));
}

#[test]
fn boss_sound_once() {
    let mut w = World::new(monstats(3, [30, 10, 0, 20, 0], 15));
    w.fake.unique = true;
    w.fake.nodes = vec![vec![w.player]];
    let mon = w.mon;
    w.with(|g, cx| think(g, cx, mon));
    assert_eq!(w.fake.log.iter().filter(|s| *s == "sound 16").count(), 1);
    assert_eq!(w.thinks(), [20]);
    w.with(|g, cx| think(g, cx, mon));
    assert_eq!(w.fake.log.iter().filter(|s| *s == "sound 16").count(), 1);
}

#[test]
fn distances() {
    assert_eq!(distance_no_size((0, 0), (10, 4)), 12);
    assert_eq!(distance_no_size((0, 0), (3, 3)), 4);
    // Full size clamps each axis at 0 (edge case 4).
    assert_eq!(distance_full_size((0, 0), 2, (1, 10)), 8);
    assert_eq!(distance_full_size((0, 0), 2, (0, 0)), 0);
}

#[test]
fn velocity_request() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.with(|_, cx| {
        set_velocity(cx, mon, 1, 50, 100);
        set_velocity(cx, mon, 0, 0, 5);
        set_velocity(cx, mon, 2, 127, 0);
    });
    let v = w.store.get(mon).unwrap().velocity;
    assert_eq!((v.method, v.speed, v.steps), (7, 50, 5));
    assert_eq!(
        w.store.unhandled,
        [Unhandled::VelocityAssert {
            unit: mon,
            speed: 127
        }]
    );
}

#[test]
fn failed_move_fallback_draw() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    w.fake.walk_fails = true;
    w.seed(4_014_346_870); // 0 < 70 → wander 4 (which fails too)
    let (mon, pl) = (w.mon, w.player);
    let ok = w.with(|g, cx| walk_to(g, cx, mon, Some(pl), 7));
    assert!(ok);
    assert!(w.thinks().is_empty(), "flag 4 deleted the thinks");
    assert_ne!(w.store.control(mon).unwrap().flags & flag::FORCE_LOS, 0);
    w.seed(12345); // 87 ≥ 70 → idle 10
    w.with(|g, cx| walk_to(g, cx, mon, Some(pl), 7));
    assert_eq!(w.thinks(), [10]);
}

#[test]
fn commands() {
    let mut w = World::new(monstats(6, [0; 5], 15));
    let mon = w.mon;
    w.with(|_, cx| {
        assert_eq!(current_command(cx, mon), None);
        copy_command(
            cx,
            mon,
            AiCommand {
                params: [1, 0, 0, 0, 0],
            },
        );
        copy_command(
            cx,
            mon,
            AiCommand {
                params: [2, 0, 0, 0, 0],
            },
        );
        assert_eq!(current_command(cx, mon).unwrap().params[0], 2);
        free_current_command(cx, mon);
        assert_eq!(current_command(cx, mon).unwrap().params[0], 1);
        free_current_command(cx, mon);
        assert_eq!(current_command(cx, mon), None);
    });
}

// Freeing the current command makes its ring next current, also when it
// is the last index (its next is index 0); the search then starts after
// that new current.
#[test]
fn freeing_the_last_command_wraps_to_its_next() {
    let mut w = World::new(monstats(6, [0; 5], 15));
    let mon = w.mon;
    w.with(|_, cx| {
        let c = cx.store.control_mut(mon).unwrap();
        c.commands = [[4, 1, 0, 0, 0], [4, 2, 0, 0, 0], [5, 3, 0, 0, 0]]
            .map(|params| AiCommand { params })
            .to_vec();
        c.cur = 2;
        free_current_command(cx, mon);
        // Ring 0 → 1 → 0: index 0 (the freed node's next) is current.
        assert_eq!(current_command(cx, mon).unwrap().params, [4, 1, 0, 0, 0]);
        // The type-4 search tests current's next (index 1) first.
        assert_eq!(find_command(cx, mon, 4, false), Some(1));
        // The copy links the new command before current (index 0).
        copy_command(
            cx,
            mon,
            AiCommand {
                params: [6, 0, 0, 0, 0],
            },
        );
        let c = cx.store.control(mon).unwrap();
        assert_eq!(c.cur, 0);
        assert_eq!(c.commands[1].params, [4, 1, 0, 0, 0]);
    });
}

// ---- install and tables ----------------------------------------------

#[test]
fn install_sets_think_and_alternate() {
    let mut w = World::new(monstats(15, [0; 5], 15));
    let mon = w.mon;
    assert_eq!(w.store.control(mon).unwrap().function, 0x005F_1800);
    w.store.control_mut(mon).unwrap().params = [1, 2, 3];
    // Re-install while running: the alternate function, nothing reset.
    w.with(|g, cx| install(g, cx, mon, 0));
    let c = w.store.control(mon).unwrap();
    assert_eq!((c.function, c.params), (0x005F_1750, [1, 2, 3]));
    // An init function runs (FoulCrowNest's: param 0 := frame;
    // BoneWall's: param 0 := frame + `Param2`).
    let mut w = World::new(monstats(43, [0; 5], 15));
    let mon = w.mon;
    assert!(w.store.unhandled.is_empty());
    w.game.frame = 77;
    w.store.control_mut(mon).unwrap().function = 0;
    w.with(|g, cx| install(g, cx, mon, 0));
    assert_eq!(w.store.control(mon).unwrap().params, [77, 0, 0]);
    let mut w = World::new(monstats(84, [0; 5], 15));
    let mon = w.mon;
    w.monstats[0].skill1 = 1;
    let mut sk = Skills::decode(&vec![0u8; Skills::SIZE]);
    sk.param2 = 30;
    w.skills = vec![sk.clone(), sk];
    w.game.frame = 10;
    w.store.control_mut(mon).unwrap().function = 0;
    w.with(|g, cx| install(g, cx, mon, 0));
    assert!(w.store.unhandled.is_empty());
    assert_eq!(w.store.control(mon).unwrap().params, [40, 0, 0]);
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.with(|g, cx| install(g, cx, mon, 18)); // state ≥ 18: nothing
    assert_eq!(w.store.control(mon).unwrap().function, 0x005E_FE20);
}

#[test]
fn special_states_10_to_12_need_switchai() {
    let mut w = World::new(monstats(3, [0; 5], 15));
    let mon = w.mon;
    w.store.control_mut(mon).unwrap().special_state = 11;
    let r = w.with(|_, cx| cx.record(mon));
    assert_eq!(r, AI_TABLE[3]);
    w.monstats[0].switchai = true;
    let r = w.with(|_, cx| cx.record(mon));
    assert_eq!(r, SPECIAL_TABLE[11]);
    w.store.control_mut(mon).unwrap().special_state = 8;
    w.monstats[0].switchai = false;
    let r = w.with(|_, cx| cx.record(mon));
    assert_eq!(r, SPECIAL_TABLE[8]);
    // An AI index outside 0..147 uses record 0.
    w.store.control_mut(mon).unwrap().special_state = 0;
    w.monstats[0].ai = 0xFFFF;
    let r = w.with(|_, cx| cx.record(mon));
    assert_eq!(r, AI_TABLE[0]);
}

#[test]
fn stub_ai_logged() {
    // Every AI table think has a body now; a special-state think without
    // one (an address outside the table) is a logged stub.
    let mut w = World::new(monstats(41, [0; 5], 15));
    let mon = w.mon;
    let p = TickParam {
        target: None,
        distance: 0,
        combat: false,
        class: 0,
        class2: 0,
    };
    w.with(|g, cx| run_function(g, cx, 0x005E_7C11, mon, &p));
    assert_eq!(
        w.store.unhandled,
        [Unhandled::Function {
            addr: 0x005E_7C11,
            unit: mon
        }]
    );
}

/// Checks `AI_TABLE` against the catalogue: 148 rows, columns index,
/// think, init, alt, target mode. Returns the first disagreement.
fn check_ai_table(tsv: &str, table: &[AiRecord]) -> Result<(), String> {
    let rows: Vec<&str> = tsv.lines().skip(1).filter(|l| !l.is_empty()).collect();
    if rows.len() != table.len() {
        return Err(format!("{} rows, table has {}", rows.len(), table.len()));
    }
    let addr = |s: &str| -> Result<u32, String> {
        if s == "-" {
            return Ok(0);
        }
        u32::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| e.to_string())
    };
    for (i, line) in rows.iter().enumerate() {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() != 11 || c[0] != i.to_string() {
            return Err(format!("row {i}: bad row"));
        }
        let rec = AiRecord {
            think: addr(c[2])?,
            init: addr(c[3])?,
            alt: addr(c[4])?,
            target_mode: c[5].parse().map_err(|_| format!("row {i}: target mode"))?,
        };
        if rec != table[i] {
            return Err(format!("index {i}: tsv {rec:x?}"));
        }
    }
    Ok(())
}

#[test]
fn ai_table_matches_tsv() {
    check_ai_table(AI_FUNCTIONS_TSV, &AI_TABLE).unwrap();
    // Target-mode counts.
    let count = |m| AI_TABLE.iter().filter(|r| r.target_mode == m).count();
    assert_eq!(
        [count(0), count(1), count(2), count(4), count(5)],
        [26, 104, 16, 1, 1]
    );
}

#[test]
fn ai_table_check_catches_perturbations() {
    let bad = AI_FUNCTIONS_TSV.replacen("0x005EFE20", "0x005EFE21", 1);
    let err = check_ai_table(&bad, &AI_TABLE).unwrap_err();
    assert!(err.starts_with("index 3:"), "{err}");
    let bad = AI_FUNCTIONS_TSV.replacen("\t0x005F1750\t4\t", "\t0x005F1750\t5\t", 1);
    let err = check_ai_table(&bad, &AI_TABLE).unwrap_err();
    assert!(err.starts_with("index 15:"), "{err}");
}

/// Checks [`SPECD_HERE`] against the catalogue's `status` column, row by
/// row. Returns the first disagreement.
fn check_specd_here(tsv: &str, specd: &[u8]) -> Result<(), String> {
    let rows: Vec<&str> = tsv.lines().skip(1).filter(|l| !l.is_empty()).collect();
    if specd.windows(2).any(|w| w[0] >= w[1]) {
        return Err("SPECD_HERE not ascending".into());
    }
    if let Some(&i) = specd.iter().find(|&&i| usize::from(i) >= rows.len()) {
        return Err(format!("index {i}: no row"));
    }
    for (i, line) in rows.iter().enumerate() {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() != 11 || c[0] != i.to_string() {
            return Err(format!("row {i}: bad row"));
        }
        let tsv = c[10] == "spec'd-here";
        let ours = specd.contains(&(i as u8));
        if tsv != ours {
            return Err(format!("index {i}: status {}, mirror {ours}", c[10]));
        }
    }
    Ok(())
}

#[test]
fn specd_here_matches_tsv() {
    check_specd_here(AI_FUNCTIONS_TSV, &SPECD_HERE).unwrap();
}

#[test]
fn specd_here_check_catches_perturbations() {
    // A row's status changed in the catalogue.
    let row98 = AI_FUNCTIONS_TSV
        .lines()
        .find(|l| l.starts_with("98\t"))
        .unwrap();
    let bad = AI_FUNCTIONS_TSV.replacen(row98, &row98.replace("\tspec'd-here", "\tsummarized"), 1);
    let err = check_specd_here(&bad, &SPECD_HERE).unwrap_err();
    assert!(err.starts_with("index 98:"), "{err}");
    // An index dropped from, or added to, the mirror.
    let mut fewer = SPECD_HERE.to_vec();
    fewer.retain(|&i| i != 60);
    let err = check_specd_here(AI_FUNCTIONS_TSV, &fewer).unwrap_err();
    assert!(err.starts_with("index 60:"), "{err}");
    // Every row is spec'd-here: an added index is out of range or out of
    // order.
    let mut more = SPECD_HERE.to_vec();
    more.push(148);
    let err = check_specd_here(AI_FUNCTIONS_TSV, &more).unwrap_err();
    assert!(err.starts_with("index 148:"), "{err}");
    let mut more = SPECD_HERE.to_vec();
    more.push(147);
    let err = check_specd_here(AI_FUNCTIONS_TSV, &more).unwrap_err();
    assert_eq!(err, "SPECD_HERE not ascending");
    // A row dropped while its status stays spec'd-here, at the end.
    let err = check_specd_here(AI_FUNCTIONS_TSV, &SPECD_HERE[..147]).unwrap_err();
    assert!(err.starts_with("index 147:"), "{err}");
}

#[test]
fn implemented_matches_catalogue() {
    // Every spec'd-here think has a body here, and only those.
    let mut ours: Vec<u8> = IMPLEMENTED.iter().map(|&(_, i)| i).collect();
    ours.sort_unstable();
    assert_eq!(ours, SPECD_HERE);
    for (addr, i) in IMPLEMENTED {
        assert_eq!(AI_TABLE[i as usize].think, addr, "index {i}");
        assert!(implemented(addr));
    }
}
mod act2;
mod act3;
mod act4;
mod act5;
mod act6;
mod act6_cov;
mod act7;
mod act7_cov;
mod bodies;
mod forced;
mod install_cov;
mod mod_ai;
mod npc;
mod rules;
mod scans;
mod skill_check;

#[test]
fn walk_in_radius_points_follow_the_recorded_walks() {
    // Warriv's three walks at the Rogue Encampment arrival (`-seed 1234`,
    // player at (4873, 4228)), size 2.
    let p = (4873, 4228);
    assert_eq!(radius_point((4866, 4235), 2, p, 3, 2), (4868, 4233));
    assert_eq!(radius_point((4868, 4233), 2, p, 2, 2), (4869, 4232));
    assert_eq!(radius_point((4869, 4232), 2, p, 1, 2), (4870, 4231));
    // Jerhyn at (5146, 5192), player at (5145, 5199), (3, 2): ax 1, ay 7,
    // k 3, n 8 → (0, 2), fixed up to (1, 3) (target (5145, 5195)).
    assert_eq!(
        radius_point((5146, 5192), 2, (5145, 5199), 3, 2),
        (5145, 5195)
    );
    // Jerhyn's second walk (measured, frame 50): the fix-up grows both.
    assert_eq!(
        radius_point((5149, 5196), 2, (5153, 5203), 3, 2),
        (5151, 5198)
    );
    // Within b: s = −1, the unit steps away (d 0, k 2, (1, 1)).
    assert_eq!(radius_point((4872, 4229), 2, p, 3, 2), (4871, 4230));
    // On the target: k = 0, the unit's own cell.
    assert_eq!(radius_point(p, 2, p, 3, 0), p);
}
