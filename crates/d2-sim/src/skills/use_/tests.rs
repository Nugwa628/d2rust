// Skill use: reference vectors, edge cases and randomness;
// tables/skills/functions.tsv (table check). The base-game skills are
// rebuilt as synthetic records from the rule values.
use super::table::{self, check_tsv, Kind};
use super::*;
use crate::rng::Seed;
use crate::skills::fake::{skill_rec, skill_tables};
use crate::skills::SkillUnits;
use std::collections::BTreeMap;

// ------------------------------------------------------------ fake

#[derive(Debug, Clone)]
pub(crate) struct U {
    pub(crate) kind: UnitType,
    pub(crate) class: i32,
    pub(crate) stats: BTreeMap<u16, i32>,
    pub(crate) states: Vec<u16>,
    pub(crate) state_stats: BTreeMap<(u16, u16), i32>,
    pub(crate) state_lists: Vec<u16>,
    pub(crate) seed: Seed,
    pub(crate) skills: Vec<SkillEntry>,
    pub(crate) used: Option<SkillEntry>,
    pub(crate) used_flags: u32,
    pub(crate) left: Option<SkillEntry>,
    pub(crate) right: Option<SkillEntry>,
    pub(crate) mode: u32,
    pub(crate) cursor: bool,
    pub(crate) endanim: i32,
    pub(crate) target: Option<usize>,
    pub(crate) flags: u32,
    pub(crate) event_arg: i32,
    pub(crate) alive: bool,
    pub(crate) room: RoomKind,
    pub(crate) pos: (i32, i32),
    pub(crate) player_data: bool,
    pub(crate) last_point: i32,
    pub(crate) items: BTreeMap<u8, usize>,
    pub(crate) param4: i32,
    pub(crate) use_state: BTreeMap<i32, UseState>,
    pub(crate) entry_mode: BTreeMap<i32, u32>,
    pub(crate) owner: Option<usize>,
    pub(crate) path: i32,
    pub(crate) act: i32,
    pub(crate) shapeshifted: bool,
}

impl U {
    pub(crate) fn new(kind: UnitType, class: i32) -> Self {
        Self {
            kind,
            class,
            stats: BTreeMap::new(),
            states: Vec::new(),
            state_stats: BTreeMap::new(),
            state_lists: Vec::new(),
            seed: Seed::new(0x1234_5678, 666),
            skills: Vec::new(),
            used: None,
            used_flags: 0,
            left: None,
            right: None,
            mode: mode::NU,
            cursor: false,
            endanim: 0,
            target: None,
            flags: FLAG_TARGETABLE,
            event_arg: 0,
            alive: true,
            room: RoomKind::Field,
            pos: (100, 100),
            player_data: kind == UnitType::Player,
            last_point: 0,
            items: BTreeMap::new(),
            param4: 0,
            use_state: BTreeMap::new(),
            entry_mode: BTreeMap::new(),
            owner: None,
            path: 0,
            act: 0,
            shapeshifted: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct F {
    pub(crate) units: Vec<U>,
    /// Item types per item handle.
    pub(crate) items: Vec<Vec<i32>>,
    pub(crate) frame: i32,
    pub(crate) log: Vec<String>,
    pub(crate) srvst_ret: i32,
    pub(crate) srvdo_ret: i32,
    pub(crate) hostile: bool,
    pub(crate) ally: bool,
    pub(crate) los: bool,
    pub(crate) melee: bool,
    pub(crate) bow: bool,
    pub(crate) mask: bool,
    pub(crate) dual: bool,
}

pub(crate) fn native(skill: i32, base: i32) -> SkillEntry {
    SkillEntry {
        skill,
        base,
        owner_guid: -1,
        ..SkillEntry::default()
    }
}

impl F {
    pub(crate) fn new() -> Self {
        Self {
            los: true,
            srvst_ret: 1,
            srvdo_ret: 1,
            ..Self::default()
        }
    }
    pub(crate) fn add(&mut self, u: U) -> usize {
        self.units.push(u);
        self.units.len() - 1
    }
    pub(crate) fn player(&mut self, skills: &[(i32, i32)]) -> usize {
        let mut u = U::new(UnitType::Player, 1);
        u.skills = skills.iter().map(|&(s, l)| native(s, l)).collect();
        self.add(u)
    }
    pub(crate) fn get(&self, u: usize, s: u16) -> i32 {
        self.units[u].stats.get(&s).copied().unwrap_or(0)
    }
    pub(crate) fn take_log(&mut self) -> Vec<String> {
        std::mem::take(&mut self.log)
    }
}

impl SkillUnits for F {
    type Unit = usize;
    type Item = usize;
    fn unit_type(&self, u: usize) -> UnitType {
        self.units[u].kind
    }
    fn class_id(&self, u: usize) -> i32 {
        self.units[u].class
    }
    fn stat(&self, u: usize, s: u16, layer: u16) -> i32 {
        if layer == 0 {
            self.get(u, s)
        } else {
            0
        }
    }
    fn item_stat(&self, u: usize, s: u16, layer: u16) -> i32 {
        self.stat(u, s, layer)
    }
    fn base_stat(&self, u: usize, s: u16, layer: u16) -> i32 {
        self.stat(u, s, layer)
    }
    fn formula_stat(&self, u: usize, s: u16, _mode: i32) -> i32 {
        self.get(u, s)
    }
    fn stat_entries(&self, _u: usize, _s: u16, _max: usize) -> Vec<(u16, i32)> {
        Vec::new()
    }
    fn has_state(&self, u: usize, s: u16) -> bool {
        self.units[u].states.contains(&s)
    }
    fn state_stat(&self, u: usize, st: u16, s: u16) -> Option<i32> {
        self.units[u].state_stats.get(&(st, s)).copied()
    }
    fn seed(&mut self, u: usize) -> &mut Seed {
        &mut self.units[u].seed
    }
    fn skill_list(&self, u: usize) -> Vec<SkillEntry> {
        self.units[u].skills.clone()
    }
    fn used_skill(&self, u: usize) -> Option<SkillEntry> {
        self.units[u].used
    }
    fn current_weapon(&self, _u: usize) -> Option<usize> {
        None
    }
    fn weapon(&self, _u: usize) -> Option<usize> {
        None
    }
    fn item_at(&self, u: usize, loc: u8) -> Option<usize> {
        self.units[u].items.get(&loc).copied()
    }
    fn item_is(&self, item: usize, itype: i32) -> bool {
        self.items[item].contains(&itype)
    }
    fn itype_is(&self, itype: i32, parent: i32) -> bool {
        itype == parent
    }
    fn wield_type(&self, _item: usize) -> i32 {
        0
    }
    fn item_damage(&self, _item: usize, _max: bool) -> i32 {
        0
    }
    fn str_dex_bonus(&self, _item: usize) -> (i32, i32) {
        (0, 0)
    }
    fn item_flag_throw(&self, _item: usize) -> bool {
        false
    }
    fn missile_level(&self, _u: usize) -> i32 {
        0
    }
}

impl ManaUnits for F {
    fn shapeshifted(&self, u: usize) -> bool {
        self.units[u].shapeshifted
    }
    fn consume_charges(&mut self, u: usize, e: &SkillEntry) -> bool {
        self.log.push(format!("charges {u} {}", e.skill));
        true
    }
    fn pay_life(&mut self, u: usize, cost: i32) -> bool {
        self.log.push(format!("paylife {u} {cost}"));
        true
    }
    fn set_stat(&mut self, u: usize, s: u16, v: i32) {
        self.units[u].stats.insert(s, v);
    }
}

impl SkillFunctions for F {
    fn srvst(&mut self, index: u16, u: usize, skill: i32, lvl: i32) -> i32 {
        self.log.push(format!("srvst {index} {u} {skill} {lvl}"));
        self.srvst_ret
    }
    fn srvdo(
        &mut self,
        index: u16,
        u: usize,
        skill: i32,
        lvl: i32,
        charge: bool,
        item: bool,
        aim: bool,
    ) -> i32 {
        self.log.push(format!(
            "srvdo {index} {u} {skill} {lvl} {charge} {item} {aim}"
        ));
        self.srvdo_ret
    }
}

impl UseMissiles for F {
    fn create_skill_missile(
        &mut self,
        u: usize,
        skill: i32,
        lvl: i32,
        missile: u16,
        lob: bool,
        aim: MissileAim,
    ) {
        self.log
            .push(format!("missile {u} {skill} {lvl} {missile} {lob} {aim:?}"));
    }
}

impl UseWorld for F {
    fn tick_rate(&self) -> crate::tick::TickRate {
        crate::tick::TickRate::BASE
    }
    fn frame(&self) -> i32 {
        self.frame
    }
    fn send(&mut self, u: usize, msg: ServerMsg) {
        self.log.push(format!("send {u} {msg:?}"));
    }
    fn has_player_data(&self, u: usize) -> bool {
        self.units[u].player_data
    }
    fn last_point_frame(&self, u: usize) -> i32 {
        self.units[u].last_point
    }
    fn set_last_point_frame(&mut self, u: usize, frame: i32) {
        self.units[u].last_point = frame;
    }
    fn position(&self, u: usize) -> (i32, i32) {
        self.units[u].pos
    }
    fn find_unit(&self, ty: u32, guid: u32) -> Option<usize> {
        let i = guid as usize;
        self.units.get(i).filter(|x| x.kind as u32 == ty).map(|_| i)
    }
    fn in_own_inventory(&self, u: usize, item: usize) -> bool {
        self.units[item].owner == Some(u)
    }
    fn same_act(&self, a: usize, b: usize) -> bool {
        self.units[a].act == self.units[b].act
    }
    fn within_reach(&self, a: usize, b: usize) -> bool {
        let (p, q) = (self.units[a].pos, self.units[b].pos);
        (p.0 - q.0).abs() <= 50 && (p.1 - q.1).abs() <= 50
    }
    fn owner(&self, u: usize) -> Option<usize> {
        self.units[u].owner
    }
    fn left_skill(&self, u: usize) -> Option<SkillEntry> {
        self.units[u].left
    }
    fn right_skill(&self, u: usize) -> Option<SkillEntry> {
        self.units[u].right
    }
    fn set_left_skill(&mut self, u: usize, e: SkillEntry) {
        self.log.push(format!("left {u} {}", e.skill));
        self.units[u].left = Some(e);
    }
    fn set_right_skill(&mut self, u: usize, e: SkillEntry) {
        self.log.push(format!("right {u} {}", e.skill));
        self.units[u].right = Some(e);
    }
    fn find_entry(&self, u: usize, skill: i32) -> Option<SkillEntry> {
        self.units[u]
            .skills
            .iter()
            .find(|e| e.skill == skill)
            .copied()
    }
    fn find_entry_owned(&self, u: usize, skill: i32, owner: i32) -> Option<SkillEntry> {
        self.units[u]
            .skills
            .iter()
            .find(|e| e.skill == skill && e.owner_guid == owner)
            .copied()
    }
    fn owns_skill(&self, u: usize, skill: i32) -> bool {
        self.find_entry(u, skill).is_some()
    }
    fn set_used_skill(&mut self, u: usize, e: Option<SkillEntry>) {
        self.log.push(format!("used {u} {:?}", e.map(|e| e.skill)));
        self.units[u].used = e;
    }
    fn used_skill_flags(&self, u: usize) -> u32 {
        self.units[u].used_flags
    }
    fn set_used_skill_flags(&mut self, u: usize, f: u32) {
        self.units[u].used_flags = f;
    }
    fn entry_mode(&self, u: usize, e: &SkillEntry) -> u32 {
        self.units[u]
            .entry_mode
            .get(&e.skill)
            .copied()
            .unwrap_or(mode::A1)
    }
    fn attack_param4(&self, u: usize) -> i32 {
        self.units[u].param4
    }
    fn set_attack_param4(&mut self, u: usize, v: i32) {
        self.units[u].param4 = v;
    }
    fn use_state(&mut self, u: usize, e: &SkillEntry) -> UseState {
        self.units[u]
            .use_state
            .get(&e.skill)
            .copied()
            .unwrap_or(UseState::Usable)
    }
    fn dec_quantity(&mut self, u: usize, skill: i32) {
        self.log.push(format!("decquant {u} {skill}"));
    }
    fn can_dual_wield(&self, _u: usize) -> bool {
        self.dual
    }
    fn equippable(&self, _item: usize) -> bool {
        true
    }
    fn bow_equipped(&self, _u: usize) -> bool {
        self.bow
    }
    fn state_mask(&self, _u: usize, mask: u32) -> bool {
        assert_eq!(mask, 0x26);
        self.mask
    }
    fn in_melee_range(&self, _u: usize, _t: usize) -> bool {
        self.melee
    }
    fn mode(&self, u: usize) -> u32 {
        self.units[u].mode
    }
    fn cursor_item(&self, u: usize) -> bool {
        self.units[u].cursor
    }
    fn endanim_expire(&self, u: usize) -> i32 {
        self.units[u].endanim
    }
    fn set_mode(&mut self, u: usize, m: u32) {
        self.log.push(format!("mode {u} {m}"));
        self.units[u].mode = m;
    }
    fn start_mode(&mut self, u: usize, m: u32, target: ModeTarget<usize>) {
        self.log.push(format!("start {u} {m} {target:?}"));
        let x = &mut self.units[u];
        x.mode = m;
        x.flags &= !FLAG_MISSILE_FIRED;
        x.target = match target {
            ModeTarget::Unit(t) => Some(t),
            ModeTarget::Point(..) => None,
        };
    }
    fn run_to(&mut self, u: usize, target: usize, e: SkillEntry) {
        self.log.push(format!("run {u} {target} {}", e.skill));
    }
    fn target(&self, u: usize) -> Option<usize> {
        self.units[u].target
    }
    fn clear_target(&mut self, u: usize) {
        self.log.push(format!("cleartarget {u}"));
        self.units[u].target = None;
    }
    fn unit_flags(&self, u: usize) -> u32 {
        self.units[u].flags
    }
    fn set_unit_flags(&mut self, u: usize, f: u32) {
        self.units[u].flags = f;
    }
    fn event_arg(&self, u: usize) -> i32 {
        self.units[u].event_arg
    }
    fn set_event_arg(&mut self, u: usize, a: i32) {
        self.units[u].event_arg = a;
    }
    fn step_path(&mut self, u: usize) -> i32 {
        self.units[u].path
    }
    fn is_alive(&self, u: usize) -> bool {
        self.units[u].alive
    }
    fn is_hostile(&self, _a: usize, _b: usize) -> bool {
        self.hostile
    }
    fn is_pet(&self, _a: usize, _b: usize) -> bool {
        false
    }
    fn is_ally(&self, _a: usize, _b: usize) -> bool {
        self.ally
    }
    fn room(&self, u: usize) -> RoomKind {
        self.units[u].room
    }
    fn target_position(&self, u: usize) -> Option<(i32, i32)> {
        self.units[u].target.map(|t| self.units[t].pos)
    }
    fn line_clear(&self, u: usize, to: (i32, i32), mask: u32) -> bool {
        let _ = (u, to, mask);
        self.los
    }
    fn schedule(&mut self, u: usize, kind: u8, frame: i32, a1: i32, a2: i32) {
        self.log
            .push(format!("schedule {u} {kind} {frame} {a1} {a2}"));
    }
    fn delete_timers(&mut self, u: usize, kind: u8, a1: i32) {
        self.log.push(format!("delete {u} {kind} {a1}"));
    }
    fn has_state_list(&self, u: usize, st: u16) -> bool {
        self.units[u].state_lists.contains(&st)
    }
    fn create_delay_list(&mut self, u: usize, expire: i32) {
        self.log.push(format!("delaylist {u} {expire}"));
        self.units[u].state_lists.push(state::SKILL_DELAY);
        self.units[u].states.push(state::SKILL_DELAY);
    }
    fn set_state_list_expiry(&mut self, u: usize, st: u16, expire: i32) {
        self.log.push(format!("expiry {u} {st} {expire}"));
    }
    fn free_aura_state(&mut self, u: usize, st: u16) {
        self.log.push(format!("freeaura {u} {st}"));
    }
    fn set_aura_state(&mut self, u: usize, st: u16, skill: i32, lvl: i32) {
        self.log.push(format!("aura {u} {st} {skill} {lvl}"));
    }
}

// ------------------------------------------------------------ records

/// A skills record without formulas, missiles or functions.
pub(crate) fn rec() -> Skills {
    let mut r = skill_rec();
    r.srvmissile = 0xFFFF;
    r.delay = 0xFFFF_FFFF;
    r.perdelay = 0xFFFF_FFFF;
    r.intown = true;
    r
}

/// Tables with `n` blank skills and `set` applied to selected ids.
/// A change to one skills record.
pub(crate) type Edit<'a> = (i32, &'a dyn Fn(&mut Skills));

pub(crate) fn tables(n: usize, set: &[Edit]) -> SkillTables {
    let mut v: Vec<Skills> = (0..n).map(|_| rec()).collect();
    for (i, f) in set {
        f(&mut v[*i as usize]);
    }
    skill_tables(v)
}

pub(crate) fn mana(r: &mut Skills, m: i16, lvl: i16, shift: u16, min: i16) {
    r.mana = m as u16;
    r.lvlmana = lvl as u16;
    r.manashift = shift;
    r.minmana = min as u16;
}

// -----------------------------------------------------------

const FUNCTIONS_TSV: &str = include_str!("../../../../../tables/skills/functions.tsv");

#[test]
fn function_tables_match_tsv() {
    assert_eq!(check_tsv(FUNCTIONS_TSV), Vec::new());
    let starts: Vec<u16> = table::FUNCS
        .iter()
        .filter(|f| f.kind == Kind::Start)
        .map(|f| f.index)
        .collect();
    let dos = table::FUNCS.iter().filter(|f| f.kind == Kind::Do).count();
    assert_eq!(starts.len(), 64);
    assert!(!starts.contains(&0) && !starts.contains(&30));
    assert!(starts.iter().all(|&i| (1..=65).contains(&i)));
    assert_eq!(dos, 152);
    assert!(table::lookup(Kind::Do, 152).is_some());
    assert!(table::lookup(Kind::Do, 153).is_none());
    assert!(table::lookup(Kind::Start, 18).is_some(), "Attract stub");
}

#[test]
fn function_table_check_reports_perturbations() {
    // A changed address on line 3 (srvst 1).
    let bad = FUNCTIONS_TSV.replacen("0x0056CA40", "0x0056CA41", 1);
    let m = check_tsv(&bad);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].line, 3);
    // A filled slot marked null.
    let bad = FUNCTIONS_TSV.replacen(
        "srvst\t2\t0x0056CAF0\tSrvSt02_Kick\tspec'd-here",
        "srvst\t2\tnull\t-\tnull",
        1,
    );
    let m = check_tsv(&bad);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].line, 4);
    // A status changed (srvdo 65 spec'd-here → mapped): one mismatch on
    // its line.
    let bad = FUNCTIONS_TSV.replacen(
        "SrvDo065_BasicAura\tspec'd-here",
        "SrvDo065_BasicAura\tmapped",
        1,
    );
    assert_ne!(bad, FUNCTIONS_TSV);
    let m = check_tsv(&bad);
    assert_eq!(m.len(), 1, "{m:?}");
    assert!(m[0].what.starts_with("srvdo 65"), "{m:?}");
    // A dropped row: the slot is reported missing.
    let bad: String = FUNCTIONS_TSV
        .lines()
        .filter(|l| !l.starts_with("srvdo\t153-190"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(check_tsv(&bad).len(), 38);
}

// -----------------------------------------------------------

pub(crate) fn point(id: u8, x: u16, y: u16) -> Vec<u8> {
    let mut m = vec![id];
    m.extend(x.to_le_bytes());
    m.extend(y.to_le_bytes());
    m
}

pub(crate) fn unit_msg(id: u8, ty: u32, guid: u32) -> Vec<u8> {
    let mut m = vec![id];
    m.extend(ty.to_le_bytes());
    m.extend(guid.to_le_bytes());
    m
}

#[test]
fn point_validator() {
    let mut f = F::new();
    let p = f.player(&[]);
    f.frame = 10;
    assert_eq!(validate_point(&mut f, p, &point(5, 150, 50)), Ok((150, 50)));
    assert_eq!(f.units[p].last_point, 10);
    f.frame = 30;
    assert_eq!(validate_point(&mut f, p, &point(5, 151, 100)), Err(1));
    assert_eq!(validate_point(&mut f, p, &point(5, 100, 49)), Err(1));
    assert!(f.take_log().is_empty(), "20 frames: no resync");
    f.frame = 36;
    assert_eq!(validate_point(&mut f, p, &[5, 0, 0, 0]), Err(3));
    assert_eq!(f.take_log(), ["send 0 Resync"]);
    assert_eq!(f.units[p].last_point, 10);
    f.units[p].player_data = false;
    assert_eq!(validate_point(&mut f, p, &point(5, 100, 100)), Err(2));
}

#[test]
fn unit_validator() {
    let mut f = F::new();
    let p = f.player(&[]);
    let m = f.add(U::new(UnitType::Monster, 0));
    let mut item = U::new(UnitType::Item, 0);
    item.owner = Some(p);
    item.act = 3;
    let item = f.add(item);
    assert_eq!(
        validate_unit(&f, p, &unit_msg(6, 1, m as u32)),
        Ok((1, m as u32, m))
    );
    assert_eq!(validate_unit(&f, p, &[6, 1]), Err(TargetError::Size));
    assert_eq!(
        validate_unit(&f, p, &unit_msg(6, 6, 0)),
        Err(TargetError::BadType)
    );
    assert_eq!(
        validate_unit(&f, p, &unit_msg(6, 1, 9)),
        Err(TargetError::NotFound)
    );
    // Own inventory item passes even in another act.
    assert!(validate_unit(&f, p, &unit_msg(6, 4, item as u32)).is_ok());
    f.units[m].act = 1;
    assert_eq!(
        validate_unit(&f, p, &unit_msg(6, 1, m as u32)),
        Err(TargetError::OtherAct)
    );
    f.units[m].act = 0;
    f.units[m].pos = (151, 100);
    assert_eq!(
        validate_unit(&f, p, &unit_msg(6, 1, m as u32)),
        Err(TargetError::Far)
    );
    assert_eq!(TargetError::Far.code(), None);
    assert_eq!(TargetError::NotFound.code(), Some(1));
}

#[test]
fn messages_select_skill_and_count_pierce_idx() {
    let t = tables(3, &[(0, &|r: &mut Skills| r.range = 1)]);
    let mut f = F::new();
    let p = f.player(&[(0, 1), (2, 1)]);
    let m = f.add(U::new(UnitType::Monster, 0));
    // No left skill → 3, pierce_idx untouched.
    assert_eq!(
        handle_message(&mut f, &t, p, &point(0x05, 100, 100)),
        Some(MsgResult::Code(3))
    );
    assert_eq!(f.get(p, PIERCE_IDX), 0);
    // Any skill on the left button (leftskill unread).
    f.units[p].left = Some(native(2, 1));
    f.units[p].right = Some(native(0, 1));
    assert_eq!(
        handle_message(&mut f, &t, p, &point(0x05, 100, 100)),
        Some(MsgResult::Code(0))
    );
    assert_eq!(f.get(p, PIERCE_IDX), 1);
    assert!(f.log.iter().any(|l| l == "start 0 7 Point(100, 100)"));
    assert_eq!(f.units[p].used.map(|e| e.skill), Some(2));
    // A failed use still counts.
    f.units[p].use_state.insert(0, UseState::Disabled);
    f.take_log();
    assert_eq!(
        handle_message(&mut f, &t, p, &point(0x0C, 100, 100)),
        Some(MsgResult::Code(0))
    );
    assert_eq!(f.get(p, PIERCE_IDX), 2);
    assert!(f.take_log().is_empty());
    // Unit messages: run allowed for 0x06 / 0x0D only.
    f.units[p].use_state.clear();
    f.units[p].mode = mode::NU;
    f.melee = false;
    handle_message(&mut f, &t, p, &unit_msg(0x0D, 1, m as u32));
    assert_eq!(f.take_log(), ["run 0 1 0"], "h2h out of range runs");
    handle_message(&mut f, &t, p, &unit_msg(0x0E, 1, m as u32));
    assert!(f.take_log().iter().any(|l| l == "start 0 7 Unit(1)"));
    assert_eq!(f.get(p, PIERCE_IDX), 4);
    // Validator failures come back unchanged.
    assert_eq!(
        handle_message(&mut f, &t, p, &unit_msg(0x06, 1, 7)),
        Some(MsgResult::Code(1))
    );
    assert_eq!(
        handle_message(&mut f, &t, p, &unit_msg(0x06, 7, 0)),
        Some(MsgResult::Unspecified(TargetError::BadType))
    );
    assert_eq!(handle_message(&mut f, &t, p, &[0x3A, 0, 0]), None);
}

#[test]
fn hold_handlers_return_zero() {
    let t = tables(1, &[]);
    let mut f = F::new();
    let p = f.player(&[(0, 1)]);
    assert_eq!(
        handle_message(&mut f, &t, p, &point(0x08, 100, 100)),
        Some(MsgResult::Code(3))
    );
    f.units[p].left = Some(native(0, 1));
    // The plain handler fails (far point) but the hold handler returns 0.
    assert_eq!(
        handle_message(&mut f, &t, p, &point(0x08, 300, 100)),
        Some(MsgResult::Code(0))
    );
    assert_eq!(f.get(p, PIERCE_IDX), 0);
    assert_eq!(
        handle_message(&mut f, &t, p, &point(0x08, 100, 100)),
        Some(MsgResult::Code(0))
    );
    assert_eq!(f.get(p, PIERCE_IDX), 1);
    assert_eq!(f.units[p].mode, mode::A1);
    // Right hold forms need a right skill.
    assert_eq!(
        handle_message(&mut f, &t, p, &unit_msg(0x10, 1, 0)),
        Some(MsgResult::Code(3))
    );
}

// -----------------------------------------------------------

pub(crate) fn dual_wielder(f: &mut F) -> usize {
    f.dual = true;
    f.items = vec![vec![45], vec![45]];
    let p = f.player(&[(0, 1), (5, 1)]);
    f.units[p].items.insert(4, 0);
    f.units[p].items.insert(5, 1);
    p
}

#[test]
fn dual_wield_alternates() {
    let t = tables(6, &[]);
    let mut f = F::new();
    let p = dual_wielder(&mut f);
    let mut used = Vec::new();
    for _ in 0..3 {
        f.units[p].mode = mode::NU;
        f.units[p].used = None;
        assert_eq!(use_at_point(&mut f, &t, p, 0, 100, 100), 0);
        used.push(f.units[p].used.unwrap().skill);
    }
    assert_eq!(used, [5, 0, 5], "Left Hand Swing, Attack, Left Hand Swing");
    // Not Attack, a type-38 hand, or no dual wield: unchanged.
    assert_eq!(dual_wield(&mut f, p, 2), 2);
    f.items[1].push(38);
    assert_eq!(dual_wield(&mut f, p, 0), 0);
    f.items[1].pop();
    f.dual = false;
    assert_eq!(dual_wield(&mut f, p, 0), 0);
}

#[test]
fn use_at_point_states() {
    let t = tables(3, &[(2, &|r: &mut Skills| r.attacknomana = true)]);
    let mut f = F::new();
    let p = f.player(&[(0, 1), (1, 1), (2, 1)]);
    // Death modes.
    for m in [mode::DT, mode::DD] {
        f.units[p].mode = m;
        assert_eq!(use_at_point(&mut f, &t, p, 1, 100, 100), 2);
    }
    f.units[p].mode = mode::NU;
    // No entry → 2.
    assert_eq!(use_at_point(&mut f, &t, p, 7, 100, 100), 2);
    // No mana: 0x5A, 2.
    f.units[p].use_state.insert(1, UseState::NoMana);
    assert_eq!(use_at_point(&mut f, &t, p, 1, 100, 100), 2);
    assert_eq!(f.take_log(), ["send 0 CantDo"]);
    // Other states: 2, silent.
    for s in [UseState::Disabled, UseState::Cooldown, UseState::Passive] {
        f.units[p].use_state.insert(1, s);
        assert_eq!(use_at_point(&mut f, &t, p, 1, 100, 100), 2);
    }
    assert!(f.take_log().is_empty());
    // AttackNoMana: Attack instead, for states 1, 2, 4.
    for s in [UseState::NoMana, UseState::NoQuantity, UseState::Shape] {
        f.units[p].mode = mode::NU;
        f.units[p].use_state.insert(2, s);
        assert_eq!(use_at_point(&mut f, &t, p, 2, 100, 100), 0);
        assert_eq!(f.units[p].used.unwrap().skill, 0);
    }
    // Skill mode 0 → 2 even with no mana (no message).
    f.units[p].entry_mode.insert(1, 0);
    f.units[p].use_state.insert(1, UseState::NoMana);
    f.take_log();
    assert_eq!(use_at_point(&mut f, &t, p, 1, 100, 100), 2);
    assert!(f.take_log().is_empty());
}

// -----------------------------------------------------------

#[test]
fn use_on_unit_paths() {
    let t = tables(
        5,
        &[
            (1, &|r: &mut Skills| r.range = 1),
            (2, &|r: &mut Skills| r.range = 2),
            (3, &|r: &mut Skills| r.range = 3),
            (4, &|r: &mut Skills| {
                r.range = 4;
                r.targetitem = true;
            }),
        ],
    );
    let mut f = F::new();
    let p = f.player(&[(1, 1), (2, 1), (3, 1), (4, 1)]);
    let m = f.add(U::new(UnitType::Monster, 0)) as u32;
    let o = f.add(U::new(UnitType::Object, 0)) as u32;
    let reset = |f: &mut F| {
        f.units[p].mode = mode::NU;
        f.take_log()
    };
    // h2h in melee range: now; out of range: run.
    f.melee = true;
    assert_eq!(use_on_unit(&mut f, &t, p, 1, 1, m, true), 0);
    assert!(reset(&mut f).contains(&"start 0 7 Unit(1)".to_string()));
    f.melee = false;
    use_on_unit(&mut f, &t, p, 1, 1, m, true);
    assert_eq!(reset(&mut f), ["run 0 1 1"]);
    // Shift (run = 0): mode change even out of range.
    use_on_unit(&mut f, &t, p, 1, 1, m, false);
    assert!(reset(&mut f).contains(&"start 0 7 Unit(1)".to_string()));
    // Ranged and location skills cast now; both uses the equipped weapon.
    use_on_unit(&mut f, &t, p, 2, 1, m, true);
    assert!(reset(&mut f).contains(&"start 0 7 Unit(1)".to_string()));
    use_on_unit(&mut f, &t, p, 3, 1, m, true);
    assert_eq!(reset(&mut f), ["run 0 1 3"]);
    f.bow = true;
    use_on_unit(&mut f, &t, p, 3, 1, m, true);
    assert!(reset(&mut f).contains(&"start 0 7 Unit(1)".to_string()));
    // A player with range 2 and state mask 0x26: h2h.
    f.mask = true;
    use_on_unit(&mut f, &t, p, 2, 1, m, true);
    assert_eq!(reset(&mut f), ["run 0 1 2"]);
    f.mask = false;
    // Location skills cast on distant monsters and TargetItem objects in place.
    use_on_unit(&mut f, &t, p, 4, 1, m, true);
    let log = reset(&mut f);
    assert!(log.contains(&"start 0 7 Unit(1)".to_string()));
    assert!(!log.iter().any(|s| s.starts_with("run ")));
    // Objects only with TargetItem.
    assert_eq!(use_on_unit(&mut f, &t, p, 1, 2, o, true), 2);
    assert_eq!(use_on_unit(&mut f, &t, p, 4, 2, o, true), 0);
    let log = reset(&mut f);
    assert!(log.contains(&"start 0 7 Unit(2)".to_string()));
    assert!(!log.iter().any(|s| s.starts_with("run ")));
    // Missing target: nothing.
    assert_eq!(use_on_unit(&mut f, &t, p, 1, 1, 99, true), 0);
    assert!(reset(&mut f).is_empty());
    // Any non-zero state → 2 without a message.
    f.units[p].use_state.insert(1, UseState::NoMana);
    assert_eq!(use_on_unit(&mut f, &t, p, 1, 1, m, true), 2);
    assert!(reset(&mut f).is_empty());
    f.units[p].use_state.clear();
    // Skill mode 0 → 2.
    f.units[p].entry_mode.insert(1, 0);
    assert_eq!(use_on_unit(&mut f, &t, p, 1, 1, m, true), 2);
    f.units[p].entry_mode.clear();
    // Attached target: its owner is targeted.
    let owner = f.add(U::new(UnitType::Monster, 0));
    f.units[m as usize].states.push(state::ATTACHED);
    f.units[m as usize].owner = Some(owner);
    use_on_unit(&mut f, &t, p, 1, 1, m, false);
    assert!(reset(&mut f).contains(&format!("start 0 7 Unit({owner})")));
}

// -----------------------------------------------------------

#[test]
fn can_change_mode_table() {
    let mut t = tables(2, &[]);
    let mut f = F::new();
    let p = f.player(&[(0, 1)]);
    f.frame = 100;
    let cases: &[(u32, u32, bool)] = &[
        (mode::NU, mode::A1, true),
        (mode::WL, mode::A1, true),
        (mode::RN, mode::A1, true),
        (mode::TN, mode::A1, true),
        (mode::TW, mode::A1, true),
        (mode::S2, mode::A1, true),
        (mode::S4, mode::A1, true),
        (mode::KB, mode::A1, true),
        // Any current mode above 18: past the jump table, yes.
        (20, mode::A1, true),
        (mode::DT, mode::A1, false),
        (mode::GH, mode::A1, false),
        (mode::BL, mode::A1, false),
        (mode::DD, mode::A1, false),
        (mode::DD, mode::NU, true),
        (mode::GH, mode::DT, true),
        (mode::GH, mode::TN, true),
        (mode::GH, mode::DD, true),
    ];
    for &(cur, new, want) in cases {
        f.units[p].mode = cur;
        assert_eq!(can_change_mode(&f, &t, p, new), want, "{cur} → {new}");
    }
    // A1/A2/SC/TH: frame ≤ E + 5, or GH / BL.
    for cur in [mode::A1, mode::A2, mode::SC, mode::TH] {
        f.units[p].mode = cur;
        f.units[p].endanim = 95;
        assert!(can_change_mode(&f, &t, p, mode::A1));
        f.units[p].endanim = 94;
        assert!(!can_change_mode(&f, &t, p, mode::A1));
        assert!(can_change_mode(&f, &t, p, mode::GH));
        assert!(can_change_mode(&f, &t, p, mode::BL));
    }
    f.units[p].mode = mode::KK;
    assert!(!can_change_mode(&f, &t, p, mode::GH));
    f.units[p].endanim = 95;
    assert!(can_change_mode(&f, &t, p, mode::A1));
    // S1: no for an Amazon; S3: no for a Druid.
    f.units[p].mode = mode::S1;
    assert!(can_change_mode(&f, &t, p, mode::A1));
    f.units[p].class = 0;
    assert!(!can_change_mode(&f, &t, p, mode::A1));
    f.units[p].mode = mode::S3;
    assert!(can_change_mode(&f, &t, p, mode::A1));
    f.units[p].class = 5;
    assert!(!can_change_mode(&f, &t, p, mode::A1));
    // SQ: seqinput > 0, else frame ≤ E + 5.
    f.units[p].mode = mode::SQ;
    f.units[p].used = Some(native(1, 1));
    f.units[p].endanim = 0;
    assert!(!can_change_mode(&f, &t, p, mode::A1));
    t.skills[1].seqinput = 1;
    assert!(can_change_mode(&f, &t, p, mode::A1));
    // Cursor item: only 0 / 17.
    f.units[p].mode = mode::NU;
    f.units[p].cursor = true;
    assert!(!can_change_mode(&f, &t, p, mode::NU));
    assert!(can_change_mode(&f, &t, p, mode::DT));
    assert!(can_change_mode(&f, &t, p, mode::DD));
}

#[test]
fn interrupt_gate_without_roll() {
    let t = tables(
        80,
        &[
            (67, &|r: &mut Skills| r.srvdofunc = 67),
            (76, &|r: &mut Skills| {
                r.srvdofunc = 76;
                r.interrupt = true;
            }),
        ],
    );
    let mut f = F::new();
    let p = f.player(&[(1, 1), (67, 1), (76, 1)]);
    let seed = f.units[p].seed;
    let req = native(1, 1);
    // State 54, death modes: no. New mode 0: yes. No used skill: yes.
    f.units[p].states.push(state::UNINTERRUPTABLE);
    assert!(!interrupt_gate(&mut f, &t, p, mode::A1, &req));
    f.units[p].states.clear();
    f.units[p].mode = mode::DD;
    assert!(!interrupt_gate(&mut f, &t, p, mode::DT, &req));
    f.units[p].mode = mode::A1;
    assert!(interrupt_gate(&mut f, &t, p, mode::DT, &req));
    assert!(interrupt_gate(&mut f, &t, p, mode::A1, &req));
    // Same Charge / Whirlwind entry again: yes.
    f.units[p].used = Some(native(67, 1));
    f.units[p].endanim = 0;
    f.frame = 100;
    assert!(interrupt_gate(&mut f, &t, p, mode::A1, &native(67, 1)));
    // Without interrupt: NU → set NU, yes; else attack modes in time.
    assert!(!interrupt_gate(&mut f, &t, p, mode::A1, &req));
    f.units[p].endanim = 95;
    for (new, want) in [
        (mode::A1, true),
        (mode::A2, true),
        (mode::SC, true),
        (mode::TH, true),
        (mode::S1, true),
        (mode::SQ, true),
        (mode::KK, false),
        (mode::WL, false),
    ] {
        assert_eq!(interrupt_gate(&mut f, &t, p, new, &req), want, "{new}");
    }
    f.units[p].mode = mode::NU;
    f.take_log();
    assert!(interrupt_gate(&mut f, &t, p, mode::WL, &req));
    assert_eq!(f.take_log(), ["mode 0 1"]);
    // With interrupt and neither state: yes. No draw anywhere.
    f.units[p].used = Some(native(76, 1));
    f.units[p].mode = mode::A1;
    assert!(interrupt_gate(&mut f, &t, p, mode::A1, &req));
    assert_eq!(f.units[p].seed, seed);
}

#[test]
fn interrupt_gate_concentration_roll() {
    let t = tables(2, &[(1, &|r: &mut Skills| r.interrupt = true)]);
    let mut f = F::new();
    let p = f.player(&[(1, 1)]);
    f.units[p].used = Some(native(1, 1));
    f.units[p].mode = mode::A1;
    f.units[p].states.push(state::CONCENTRATION);
    let mut expect = f.units[p].seed;
    let r = expect.roll(100) as i32;
    // Threshold just above the roll: blocked; current mode A1 → no.
    f.units[p]
        .state_stats
        .insert((state::CONCENTRATION, CONCENTRATION_STAT), r + 1);
    assert!(!interrupt_gate(&mut f, &t, p, mode::A1, &native(1, 1)));
    assert_eq!(f.units[p].seed, expect, "one roll(100) on the unit seed");
    // Threshold at the roll: not blocked.
    let r2 = expect.clone().roll(100) as i32;
    f.units[p]
        .state_stats
        .insert((state::CONCENTRATION, CONCENTRATION_STAT), r2);
    assert!(interrupt_gate(&mut f, &t, p, mode::A1, &native(1, 1)));
    expect.roll(100);
    assert_eq!(f.units[p].seed, expect);
    // No stat list: stat 164 reads 0; the draw is still made and never
    // blocks.
    f.units[p].state_stats.clear();
    assert!(interrupt_gate(&mut f, &t, p, mode::A1, &native(1, 1)));
    expect.roll(100);
    assert_eq!(f.units[p].seed, expect);
    // State 15 blocks without a draw; in NU: set NU, yes.
    f.units[p].states = vec![state::CONCENTRATE];
    f.units[p].mode = mode::NU;
    f.take_log();
    assert!(interrupt_gate(&mut f, &t, p, mode::A1, &native(1, 1)));
    assert_eq!(f.take_log(), ["mode 0 1"]);
    f.units[p].mode = mode::A2;
    assert!(!interrupt_gate(&mut f, &t, p, mode::A1, &native(1, 1)));
    assert_eq!(f.units[p].seed, expect);
}

// -----------------------------------------------------------

#[test]
fn skill_modes() {
    let mut r = rec();
    r.anim = mode::S2 as u8;
    r.monanim = mode::A2 as u8;
    assert_eq!(skill_mode(UnitType::Player, 1, &r, 9, false), mode::S2);
    assert_eq!(skill_mode(UnitType::Player, 6, &r, 5, false), mode::S4);
    assert_eq!(skill_mode(UnitType::Player, 1, &r, 5, false), mode::S2);
    assert_eq!(skill_mode(UnitType::Monster, 0, &r, 9, false), mode::A2);
    assert_eq!(skill_mode(UnitType::Player, 1, &r, 9, true), mode::SC);
    for a in [mode::A1, mode::A2, mode::SC, mode::TH, mode::SQ] {
        r.anim = a as u8;
        assert_eq!(skill_mode(UnitType::Player, 1, &r, 9, true), a);
    }
}

#[test]
fn frame_event_codes() {
    // Recording 021854: one code-1 frame at offset 6 → type 0 (1, 0).
    assert_eq!(frame_events(&[0, 0, 0, 0, 0, 0, 1, 0]), [(6, 1, 0)]);
    assert_eq!(
        frame_events(&[1, 3, 2, 0, 4, 5]),
        [(0, 1, 0), (1, 3, 0), (2, 2, 1), (4, 4, 2)]
    );
}

#[test]
fn attack_frame_events() {
    let t = tables(2, &[(1, &|r: &mut Skills| r.srvdofunc = 1)]);
    let mut f = F::new();
    let p = f.player(&[(1, 1)]);
    f.units[p].used = Some(native(1, 1));
    // arg1 1 / 2 run the do; 3 / 4 don't.
    for (a1, ran) in [(1, true), (2, true), (3, false), (4, false)] {
        assert_eq!(attack_frame_event(&mut f, &t, p, a1, 7), 1);
        assert_eq!(f.units[p].event_arg, 7);
        assert_eq!(
            f.take_log().iter().any(|l| l.starts_with("srvdo 1")),
            ran,
            "arg1 {a1}"
        );
    }
    // Flag 0x40 set: skip.
    f.units[p].flags |= FLAG_MISSILE_FIRED;
    attack_frame_event(&mut f, &t, p, 1, 0);
    assert!(f.take_log().is_empty());
    // Moving skills: the path decides, flag 0x40 ignored.
    f.units[p].used_flags = SKILL_MOVING;
    f.units[p].path = 1;
    attack_frame_event(&mut f, &t, p, 1, 0);
    assert!(f.take_log().is_empty());
    // Not finished: step 3 still runs: with flag 0x40 clear,
    // arg1 1 / 2 do on the way (Leap's frame-11 Land, Whirlwind's
    // strikes); 3 / 4 don't; the arrived flag stays clear.
    f.units[p].flags &= !FLAG_MISSILE_FIRED;
    for (a1, ran) in [(1, true), (2, true), (3, false), (4, false)] {
        attack_frame_event(&mut f, &t, p, a1, 0);
        assert_eq!(
            f.take_log().iter().any(|l| l.starts_with("srvdo 1")),
            ran,
            "moving, arg1 {a1}"
        );
        assert_eq!(f.units[p].used_flags, SKILL_MOVING);
    }
    f.units[p].path = 2;
    attack_frame_event(&mut f, &t, p, 3, 0);
    assert_eq!(f.units[p].used_flags, SKILL_MOVING | SKILL_ARRIVED);
    assert!(f.take_log().iter().any(|l| l.starts_with("srvdo 1")));
    // Died: 2.
    f.units[p].alive = false;
    assert_eq!(attack_frame_event(&mut f, &t, p, 3, 0), 2);
}

/// A player whose used skill is `skill`, in a field room.
pub(crate) fn caster(f: &mut F, skill: i32, lvl: i32, mana: i32) -> usize {
    let p = f.player(&[(skill, lvl)]);
    f.units[p].used = Some(native(skill, lvl));
    f.units[p].stats.insert(stat::MANA, mana);
    p
}

#[test]
fn start_refusals() {
    let t = tables(
        3,
        &[
            (1, &|r: &mut Skills| {
                r.srvstfunc = 1;
                r.targetableonly = true;
            }),
            (2, &|r: &mut Skills| {
                r.srvstfunc = 1;
                r.intown = false;
                r.targetcorpse = true;
            }),
        ],
    );
    let mut f = F::new();
    // No used skill: 0, nothing.
    let p = f.player(&[]);
    assert_eq!(start(&mut f, &t, p), 0);
    assert!(f.take_log().is_empty());
    // TargetableOnly against a non-hostile target: neutral, 0.
    let p = caster(&mut f, 1, 1, 0);
    let m = f.add(U::new(UnitType::Monster, 0));
    f.units[p].target = Some(m);
    // Neutral is NU in a field room (measured, Telekinesis),
    // TN in a town room.
    assert_eq!(start(&mut f, &t, p), 0);
    assert_eq!(
        f.take_log(),
        ["delete 1 0 0", "delete 1 1 0", "mode 1 1", "used 1 None"]
    );
    f.units[p].used = Some(native(1, 1));
    f.units[p].room = RoomKind::Town;
    assert_eq!(start(&mut f, &t, p), 0);
    assert_eq!(
        f.take_log(),
        ["delete 1 0 0", "delete 1 1 0", "mode 1 5", "used 1 None"]
    );
    f.units[p].used = Some(native(1, 1));
    f.units[p].room = RoomKind::Field;
    f.hostile = true;
    assert_eq!(start(&mut f, &t, p), 1);
    assert_eq!(f.take_log(), ["srvst 1 1 1 1"]);
    // Target without flag 2 dropped; dead monster dropped without
    // TargetCorpse.
    f.units[m].flags = 0;
    start(&mut f, &t, p);
    assert_eq!(f.take_log()[0], "cleartarget 1");
    f.units[m].flags = FLAG_TARGETABLE;
    f.units[p].target = Some(m);
    f.units[m].mode = crate::units::modes::monster_mode::DD;
    start(&mut f, &t, p);
    assert_eq!(f.take_log()[0], "cleartarget 1");
    // Item skill with 0 charges, ally without TargetAlly: 0, no reset.
    f.units[p].used = Some(SkillEntry {
        skill: 1,
        base: 1,
        owner_guid: 9,
        charges: 0,
        ..SkillEntry::default()
    });
    assert_eq!(start(&mut f, &t, p), 0);
    assert!(f.take_log().is_empty());
    f.units[p].used = Some(native(1, 1));
    f.units[m].mode = 1;
    f.units[p].target = Some(m);
    f.ally = true;
    assert_eq!(start(&mut f, &t, p), 0);
    assert!(f.take_log().is_empty());
    f.ally = false;
    // Not InTown in town: used skill cleared, neutral. Living monster with
    // TargetCorpse dropped first.
    let q = caster(&mut f, 2, 1, 0);
    f.units[q].target = Some(m);
    f.units[q].room = RoomKind::Town;
    assert_eq!(start(&mut f, &t, q), 0);
    assert_eq!(
        f.take_log(),
        [
            format!("cleartarget {q}"),
            format!("used {q} None"),
            format!("delete {q} 0 0"),
            format!("delete {q} 1 0"),
            format!("mode {q} 5"),
            format!("used {q} None")
        ]
    );
    // Core result 0: neutral.
    f.units[p].target = None;
    f.srvst_ret = 0;
    assert_eq!(start(&mut f, &t, p), 0);
    assert_eq!(
        f.take_log(),
        [
            "srvst 1 1 1 1",
            "delete 1 0 0",
            "delete 1 1 0",
            "mode 1 1",
            "used 1 None"
        ]
    );
}

/// The neutral request of a failed start is the mode request `(mode 5,
/// forced)`: the mode check first (a cursor item refuses
/// mode 5, the mode stays), then sets 1 in a field room and
/// 5 in a town room or with no room (L35 2026-10-14).
#[test]
fn start_failure_neutral_request_checks_the_mode_and_keeps_5_without_room() {
    let t = tables(2, &[(1, &|r: &mut Skills| r.srvstfunc = 1)]);
    let mut f = F::new();
    let p = caster(&mut f, 1, 1, 0);
    f.srvst_ret = 0;
    f.units[p].room = RoomKind::None;
    f.units[p].used = Some(native(1, 1));
    assert_eq!(start(&mut f, &t, p), 0);
    // The type-0/1 timers of any arg cancelled,
    // the mode set, the used skill cleared.
    assert_eq!(
        f.take_log(),
        [
            format!("delete {p} 0 0"),
            format!("delete {p} 1 0"),
            format!("mode {p} 5"),
            format!("used {p} None")
        ]
    );
    assert!(f.units[p].used.is_none());
    f.units[p].room = RoomKind::Field;
    f.units[p].mode = 4;
    f.units[p].cursor = true;
    f.units[p].used = Some(native(1, 1));
    assert_eq!(start(&mut f, &t, p), 0);
    assert_eq!(f.take_log(), [format!("srvst 1 {p} 1 1")]);
    assert_eq!(f.units[p].mode, 4);
}

#[test]
fn start_core_steps() {
    let t = tables(
        8,
        &[
            (1, &|r: &mut Skills| {
                r.srvstfunc = 4;
                mana(r, 4, 1, 8, 0);
            }),
            (2, &|r: &mut Skills| r.srvstfunc = 30),
            (3, &|r: &mut Skills| r.srvstfunc = 91),
            (4, &|r: &mut Skills| {
                r.srvstfunc = 13;
                r.periodic = true;
            }),
            (5, &|r: &mut Skills| {
                r.srvstfunc = 1;
                r.lineofsight = 4;
            }),
            (6, &|r: &mut Skills| {
                r.srvstfunc = 1;
                r.lineofsight = 6;
            }),
        ],
    );
    let mut f = F::new();
    // Multiple Shot L10 (srvst 4; 4, +1, shift 8): 3,328 charged at start.
    let p = caster(&mut f, 1, 10, 4000);
    assert_eq!(start(&mut f, &t, p), 1);
    assert_eq!(f.get(p, stat::MANA), 4000 - 3328);
    // Not enough mana for the check: 0 → neutral, nothing charged.
    assert_eq!(start(&mut f, &t, p), 0);
    assert_eq!(
        f.take_log(),
        [
            "srvst 4 0 1 10",
            "delete 0 0 0",
            "delete 0 1 0",
            "mode 0 1",
            "used 0 None"
        ]
    );
    // No room: 0.
    let q = caster(&mut f, 2, 1, 0);
    f.units[q].room = RoomKind::None;
    assert_eq!(start(&mut f, &t, q), 0);
    f.units[q].used = Some(native(2, 1));
    // Null start slot: 1, nothing called; past the table: 0.
    f.units[q].room = RoomKind::Field;
    f.take_log();
    assert_eq!(start(&mut f, &t, q), 1);
    assert!(f.take_log().is_empty());
    let q = caster(&mut f, 3, 1, 0);
    assert_eq!(start(&mut f, &t, q), 0);
    // Periodic: type-8 timers with arg 0 deleted after a non-zero start.
    let q = caster(&mut f, 4, 1, 0);
    f.take_log();
    start(&mut f, &t, q);
    assert_eq!(f.take_log(), ["srvst 13 3 4 1", "delete 3 8 0"]);
    // Line of sight 4: mask 0x804; blocked → 0; value > 5 → 0.
    let q = caster(&mut f, 5, 1, 0);
    let m = f.add(U::new(UnitType::Monster, 0));
    f.units[q].target = Some(m);
    f.hostile = true;
    assert_eq!(start(&mut f, &t, q), 1);
    f.los = false;
    assert_eq!(start(&mut f, &t, q), 0);
    // No target position: the line test is skipped (passes).
    f.units[q].used = Some(native(5, 1));
    f.units[q].target = None;
    assert_eq!(start(&mut f, &t, q), 1);
    f.los = true;
    let q = caster(&mut f, 6, 1, 0);
    assert_eq!(start(&mut f, &t, q), 0);
    assert_eq!(LOS_MASKS, [4, 0x1C09, 0x180, 0x804, 0x805]);
}

#[test]
fn teleport_mana_check_and_consume() {
    // Teleport: mana 24, lvlmana −1, shift 8, minmana 1.
    let t = tables(2, &[(1, &|r: &mut Skills| mana(r, 24, -1, 8, 1))]);
    let mut f = F::new();
    let p = caster(&mut f, 1, 1, 0);
    for (l, check, consume) in [
        (1, 6144, 6144),
        (20, 1280, 1280),
        (25, 0, 256),
        (30, -1280, 256),
    ] {
        f.units[p].stats.insert(stat::MANA, check);
        assert!(mana_check(&f, &t, p, &native(1, l), l), "L{l}");
        f.units[p].stats.insert(stat::MANA, check - 1);
        assert!(!mana_check(&f, &t, p, &native(1, l), l), "L{l}");
        f.units[p].stats.insert(stat::MANA, 10_000);
        assert!(consume_mana(&mut f, &t, Some(p), 1, l));
        assert_eq!(10_000 - f.get(p, stat::MANA), consume, "L{l}");
    }
    // Blood mana (state 114): the start/do check still compares mana.
    f.units[p].states.push(114);
    f.units[p].stats.insert(stat::MANA, 0);
    f.units[p].stats.insert(stat::HITPOINTS, 100_000);
    assert!(!mana_check(&f, &t, p, &native(1, 1), 1));
    // Non-players pass; item skills need charges.
    let m = f.add(U::new(UnitType::Monster, 0));
    assert!(mana_check(&f, &t, m, &native(1, 1), 1));
    let item = SkillEntry {
        skill: 1,
        owner_guid: 3,
        charges: 1,
        ..SkillEntry::default()
    };
    assert!(mana_check(&f, &t, p, &item, 1));
    assert!(!mana_check(
        &f,
        &t,
        p,
        &SkillEntry { charges: 0, ..item },
        1
    ));
}

#[test]
fn fire_bolt_and_fire_ball_charge_at_do() {
    // Fire Bolt L1 (5, 0, shift 7, minmana 1, no start/do, srvmissile).
    let t = tables(
        3,
        &[
            (1, &|r: &mut Skills| {
                mana(r, 5, 0, 7, 1);
                r.srvmissile = 0;
            }),
            (2, &|r: &mut Skills| {
                mana(r, 10, 1, 7, 1);
                r.srvmissile = 0;
                r.lob = true;
            }),
        ],
    );
    let mut f = F::new();
    let p = caster(&mut f, 1, 1, 640);
    assert!(mana_check(&f, &t, p, &native(1, 1), 1));
    assert_eq!(start(&mut f, &t, p), 1, "null start: 1, nothing charged");
    assert_eq!(f.get(p, stat::MANA), 640);
    assert_eq!(do_skill(&mut f, &t, p, 1, 1), 1);
    assert_eq!(f.get(p, stat::MANA), 0, "640 charged at do");
    assert_ne!(f.units[p].flags & FLAG_MISSILE_FIRED, 0);
    assert_eq!(f.take_log(), ["missile 0 1 1 0 false None"]);
    // Fire Ball L10: (10 + 9) << 7 = 2,432 at do; lob.
    let p = caster(&mut f, 2, 10, 3000);
    do_skill(&mut f, &t, p, 2, 10);
    assert_eq!(f.get(p, stat::MANA), 3000 - 2432);
    assert_eq!(f.take_log(), ["missile 1 2 10 0 true None"]);
}

#[test]
fn blade_fury_charges_each_do() {
    // Blade Fury L1 (8, +1, shift 5, startmana 6, usemanaondo) with a start.
    let t = tables(
        2,
        &[(1, &|r: &mut Skills| {
            mana(r, 8, 1, 5, 0);
            r.startmana = 6;
            r.usemanaondo = true;
            r.srvstfunc = 1;
            r.srvdofunc = 1;
        })],
    );
    let mut f = F::new();
    let p = caster(&mut f, 1, 1, 600);
    assert_eq!(start(&mut f, &t, p), 1);
    assert_eq!(f.get(p, stat::MANA), 600, "start charges 0");
    assert_eq!(do_skill(&mut f, &t, p, 1, 1), 1);
    assert_eq!(f.get(p, stat::MANA), 344);
    assert_eq!(do_skill(&mut f, &t, p, 1, 1), 1);
    assert_eq!(f.get(p, stat::MANA), 88);
    // Check fails at the do: 0, nothing.
    f.take_log();
    assert_eq!(do_skill(&mut f, &t, p, 1, 1), 0);
    assert_eq!(f.get(p, stat::MANA), 88);
    assert!(f.take_log().is_empty());
    // A failed consume is ignored: the skill still executes.
    let t2 = tables(2, &[(1, &|r: &mut Skills| r.srvdofunc = 1)]);
    let q = caster(&mut f, 1, 1, 0);
    assert_eq!(do_skill(&mut f, &t2, q, 1, 1), 1, "mana 0 and lvlmana 0");
}

#[test]
fn do_core_gates() {
    let t = tables(
        5,
        &[
            (1, &|r: &mut Skills| {
                r.srvdofunc = 1;
                r.intown = false;
            }),
            (2, &|r: &mut Skills| r.srvdofunc = 191),
            (3, &|r: &mut Skills| {
                r.srvdofunc = 2;
                r.itemeffect = 36;
            }),
            (4, &|r: &mut Skills| r.srvdofunc = 1),
        ],
    );
    let mut f = F::new();
    // Not InTown, in town: used skill cleared, neutral, 0.
    let p = caster(&mut f, 1, 1, 0);
    f.units[p].room = RoomKind::Town;
    assert_eq!(do_core(&mut f, &t, p, 1, 1, true, false, false), 0);
    assert_eq!(f.take_log(), ["used 0 None", "mode 0 5"]);
    // Level 0 and no entry with a level: 0.
    let p = caster(&mut f, 4, 0, 0);
    assert_eq!(do_core(&mut f, &t, p, 4, 0, true, false, false), 0);
    // L > 0 does not pass by itself: the used entry's level is read.
    assert_eq!(do_core(&mut f, &t, p, 4, 1, true, false, false), 0);
    f.units[p].skills[0].base = 2;
    assert_eq!(do_core(&mut f, &t, p, 4, 0, true, false, false), 1);
    // srvdofunc past the table: 0.
    let p = caster(&mut f, 2, 1, 0);
    assert_eq!(do_core(&mut f, &t, p, 2, 1, true, false, false), 0);
    // ItemEffect with item and aim.
    let p = caster(&mut f, 3, 1, 0);
    f.take_log();
    do_core(&mut f, &t, p, 3, 1, true, true, true);
    assert_eq!(f.take_log(), ["srvdo 36 3 3 1 true true true"]);
    do_core(&mut f, &t, p, 3, 1, true, true, false);
    assert_eq!(f.take_log(), ["srvdo 2 3 3 1 true true false"]);
}

#[test]
fn item_aimed_missile_position() {
    let t = tables(
        2,
        &[(1, &|r: &mut Skills| {
            r.srvmissile = 0;
        })],
    );
    let mut f = F::new();
    let p = caster(&mut f, 1, 1, 0);
    let mut m = U::new(UnitType::Monster, 0);
    m.pos = (110, 95);
    let m = f.add(m);
    f.units[p].target = Some(m);
    do_core(&mut f, &t, p, 1, 1, true, true, true);
    assert_eq!(
        f.take_log(),
        ["missile 0 1 1 0 false At { offset: (10, -5), aim: (120, 90) }"]
    );
}

// -----------------------------------------------------------

/// A code buffer with `v` (< 128) as its only formula at offset 0.
fn konst(t: &mut SkillTables, v: u8) {
    t.skills_code = vec![0x07, v, 0x00];
}

#[test]
fn meteor_cooldown() {
    let mut t = tables(
        3,
        &[
            (1, &|r: &mut Skills| {
                r.srvdofunc = 1;
                r.delay = 0;
            }),
            (2, &|r: &mut Skills| r.delay = 0),
        ],
    );
    konst(&mut t, 30);
    let mut f = F::new();
    f.frame = 1000;
    let p = caster(&mut f, 1, 1, 0);
    f.units[p].skills.push(native(2, 1));
    assert!(!cooldown_blocks(&mut f, &t, p, 2, 1));
    do_skill(&mut f, &t, p, 1, 1);
    assert_eq!(
        f.take_log(),
        [
            "srvdo 1 0 1 1 true false false",
            "delaylist 0 1030",
            "expiry 0 121 1030",
            "schedule 0 12 1030 0 0"
        ]
    );
    // Any delay skill is blocked meanwhile; one without a delay is not.
    assert!(cooldown_blocks(&mut f, &t, p, 2, 1));
    assert!(cooldown_blocks(&mut f, &t, p, 1, 1));
    let mut t0 = t.clone();
    t0.skills[2].delay = 0xFFFF_FFFF;
    assert!(!cooldown_blocks(&mut f, &t0, p, 2, 1));
    // Again: the list exists, expiry moved, another type-12 timer.
    f.frame = 1010;
    do_skill(&mut f, &t, p, 1, 1);
    assert_eq!(
        f.take_log(),
        [
            "srvdo 1 0 1 1 true false false",
            "expiry 0 121 1040",
            "schedule 0 12 1040 0 0"
        ]
    );
    // SQ: only the first event (unit +0x38 bits 8+ = 0) sets the delay.
    f.units[p].mode = mode::SQ;
    f.units[p].event_arg = 1;
    do_skill(&mut f, &t, p, 1, 1);
    assert_eq!(f.take_log(), ["srvdo 1 0 1 1 true false false"]);
    f.units[p].event_arg = 0;
    do_skill(&mut f, &t, p, 1, 1);
    assert_eq!(f.take_log().len(), 3);
    // No charge: no delay. Monsters: none.
    do_core(&mut f, &t, p, 1, 1, false, false, false);
    assert_eq!(f.take_log().len(), 1);
    set_delay(&mut f, p, 0);
    assert!(f.take_log().is_empty());
    let mut mu = U::new(UnitType::Monster, 0);
    mu.skills.push(native(1, 1));
    mu.used = Some(native(1, 1));
    let m = f.add(mu);
    do_skill(&mut f, &t, m, 1, 1);
    assert_eq!(f.take_log().len(), 1);
}

// -----------------------------------------------------------

fn might() -> SkillTables {
    let mut t = tables(
        3,
        &[
            (1, &|r: &mut Skills| {
                r.aura = true;
                r.immediate = true;
                r.perdelay = 0;
                r.srvdofunc = 65;
                r.aurastate = 33;
            }),
            (2, &|r: &mut Skills| {
                r.aura = true;
                r.perdelay = 0;
                r.srvdofunc = 65;
                r.aurastate = 34;
            }),
        ],
    );
    konst(&mut t, 50);
    t
}

#[test]
fn might_assigned_runs_now_and_every_50() {
    let t = might();
    let mut f = F::new();
    let p = f.player(&[(1, 1), (2, 3)]);
    f.frame = 1234;
    let mut m = vec![0x3C];
    m.extend(1u32.to_le_bytes());
    m.extend((-1i32).to_le_bytes());
    assert_eq!(select_skill(&mut f, &t, p, &m), 0);
    assert_eq!(
        f.take_log(),
        [
            "right 0 1",
            "srvdo 65 0 1 1 true false false",
            "delete 0 8 -1",
            "schedule 0 8 1251 -1 0"
        ]
    );
    // The type-8 event at 1251: do again, next at 1301.
    f.frame = 1251;
    periodic_event(&mut f, &t, p, -1);
    assert_eq!(
        f.take_log(),
        [
            "srvdo 65 0 1 1 true false false",
            "delete 0 8 -1",
            "schedule 0 8 1301 -1 0"
        ]
    );
    // Assigned at 1250 → 1251.
    f.frame = 1250;
    assert_eq!(period(&mut f, &t, p, 1, 1), 1251);
    // perdelay ≤ 5 → 5.
    let mut t5 = t.clone();
    konst(&mut t5, 2);
    f.frame = 7;
    assert_eq!(period(&mut f, &t5, p, 1, 1), 11);
    // Dead: no do, not rescheduled.
    f.units[p].alive = false;
    periodic_event(&mut f, &t, p, -1);
    assert!(f.take_log().is_empty());
    f.units[p].alive = true;
    // Replacing the aura: its state freed, type-8 (−1) deleted; a new
    // non-immediate aura switches its state on.
    let mut m2 = vec![0x3C];
    m2.extend(2u32.to_le_bytes());
    m2.extend((-1i32).to_le_bytes());
    f.frame = 1300;
    assert_eq!(select_skill(&mut f, &t, p, &m2), 0);
    assert_eq!(
        f.take_log(),
        [
            "freeaura 0 33",
            "delete 0 8 -1",
            "right 0 2",
            "aura 0 34 2 3",
            "delete 0 8 -1",
            "schedule 0 8 1301 -1 0"
        ]
    );
}

#[test]
fn select_skill_checks_and_left() {
    let t = might();
    let mut f = F::new();
    let p = f.player(&[(1, 0), (2, 1)]);
    let msg = |skill: u32, left: bool, owner: i32| {
        let mut m = vec![0x3C];
        m.extend((skill | if left { 0x8000_0000 } else { 0 }).to_le_bytes());
        m.extend(owner.to_le_bytes());
        m
    };
    assert_eq!(select_skill(&mut f, &t, p, &[0x3C, 0, 0]), 3);
    assert_eq!(select_skill(&mut f, &t, p, &msg(9, false, -1)), 3);
    assert_eq!(select_skill(&mut f, &t, p, &msg(2, false, 5)), 3, "owner");
    assert_eq!(
        select_skill(&mut f, &t, p, &msg(1, false, -1)),
        3,
        "level 0"
    );
    assert_eq!(select_skill(&mut f, &t, p, &msg(2, true, -1)), 0);
    assert_eq!(f.take_log(), ["left 0 2"]);
}

#[test]
fn periodic_skill_events() {
    let mut t = tables(
        3,
        &[(1, &|r: &mut Skills| {
            r.periodic = true;
            r.perdelay = 0;
            r.srvdofunc = 1;
            r.aurastate = 40;
        })],
    );
    konst(&mut t, 25);
    let mut f = F::new();
    f.frame = 100;
    let p = caster(&mut f, 1, 4, 0);
    // The do wrapper schedules (skill, L) after the do.
    do_skill(&mut f, &t, p, 1, 4);
    assert_eq!(
        f.take_log(),
        [
            "srvdo 1 0 1 4 true false false",
            "delete 0 8 1",
            "schedule 0 8 101 1 4"
        ]
    );
    // Type 8 (skill): needs the aura state with stat 350 = skill, level
    // from 351, and the skill owned.
    periodic_event(&mut f, &t, p, 1);
    assert!(f.take_log().is_empty());
    f.units[p].states.push(40);
    f.units[p].state_stats.insert((40, AURA_SKILL), 1);
    f.units[p].state_stats.insert((40, AURA_LEVEL), 7);
    periodic_event(&mut f, &t, p, 1);
    assert_eq!(
        f.take_log(),
        [
            "srvdo 1 0 1 7 true false false",
            "delete 0 8 1",
            "schedule 0 8 101 1 7"
        ]
    );
    f.units[p].state_stats.insert((40, AURA_SKILL), 2);
    periodic_event(&mut f, &t, p, 1);
    assert!(f.take_log().is_empty());
    // Not periodic and not an aura: nothing scheduled.
    schedule_periodic(&mut f, &t, p, 2, 1, false);
    assert!(f.take_log().is_empty());
    // Type 9 (item aura): do core (…, 1, 1, 0); type 5: srvdo[srvactivefunc].
    item_aura_event(&mut f, &t, p, 1, 3);
    assert_eq!(f.take_log(), ["srvdo 1 0 1 3 true true false"]);
    assert_eq!(active_state_event(&mut f, p, 145, 1, 3), 1);
    assert_eq!(f.take_log(), ["srvdo 145 0 1 3 true false false"]);
    assert_eq!(active_state_event(&mut f, p, 160, 1, 3), 0);
}

// ------------------------------------------------------------ test vectors

#[test]
fn request_starts_mode_and_runs_start_in_the_same_tick() {
    // Recording 021854 pattern: a request changes the mode and the start
    // runs at once; the frame events are the mode start's (units).
    let t = tables(2, &[(1, &|r: &mut Skills| r.srvstfunc = 1)]);
    let mut f = F::new();
    f.frame = 886;
    let p = f.player(&[(1, 1)]);
    assert_eq!(use_at_point(&mut f, &t, p, 1, 110, 100), 0);
    assert_eq!(
        f.take_log(),
        [
            "used 0 Some(1)",
            "start 0 7 Point(110, 100)",
            "srvst 1 0 1 1"
        ]
    );
    // Chained one frame before ENDANIM (frame ≤ E + 5).
    f.units[p].endanim = 900;
    f.frame = 899;
    assert_eq!(use_at_point(&mut f, &t, p, 1, 110, 100), 0);
    assert!(f
        .take_log()
        .contains(&"start 0 7 Point(110, 100)".to_string()));
    // Gate refusal: nothing set.
    f.units[p].mode = mode::GH;
    assert_eq!(use_at_point(&mut f, &t, p, 1, 110, 100), 0);
    assert!(f.take_log().is_empty());
}

mod bodies;
