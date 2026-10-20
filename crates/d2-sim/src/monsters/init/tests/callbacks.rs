//! The umod callback bodies on a recording host: synthetic
//! test vectors and each callback's calls, draws and timers.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::*;
use crate::combat::DamageRecord;
use crate::skills::use_::bodies::MissileRequest;
use crate::stats::{ListId, StatData, StatLists};
use crate::tick::timer::TimerClass;

use super::super::callbacks::{self as cb, addr, AuraFields, SkillCalc, StateApply};
use super::super::find::FindQuery;

/// Every call the callbacks make into the host, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Missile(MissileRequest<UnitId>),
    Find(UnitId, FindQuery),
    Hit(UnitId, UnitId, DamageRecord),
    Mode(UnitId, u32),
    State(UnitId, u16),
    Apply(StateApply),
    ListSet(u16, i32),
    FreeMinions(UnitId),
    ClearOwner(UnitId),
    RemovePet(UnitId, UnitId),
    Drop(UnitId, [u8; 4], i32, bool),
    Steal(UnitId, UnitId),
    Spawn(UnitId, u32, u32, i32, u32),
    UseSkill(UnitId, u32, u16),
    SetParam(UnitId, i32),
}

struct Cb {
    cx: Ctx<'static>,
    game: Game,
    units: Units,
    store: MonsterStore,
    info: GameInfo,
    room: RoomId,
    stats: BTreeMap<(UnitId, u16), i32>,
    calls: Vec<Call>,
    pos: BTreeMap<UnitId, (i32, i32)>,
    states: BTreeSet<(UnitId, u16)>,
    udead: BTreeSet<UnitId>,
    owners: BTreeMap<UnitId, UnitId>,
    minion_owners: BTreeMap<UnitId, UnitId>,
    minions: BTreeMap<UnitId, Vec<UnitId>>,
    alignment: BTreeMap<UnitId, u8>,
    hostile: bool,
    line: bool,
    targets: BTreeMap<UnitId, UnitId>,
    target_pos: Option<(i32, i32)>,
    path_target: (i32, i32),
    missile_flags: BTreeMap<i32, u32>,
    velocity: i32,
    missile_sl: (i32, i32),
    uber: [bool; 3],
    found: Vec<UnitId>,
    calc: BTreeMap<SkillCalc, i32>,
    aura: Option<AuraFields>,
    lists: StatLists,
    occupied: bool,
    param0: i32,
    raise: bool,
    sw_level: Option<i32>,
    life: i32,
    area_level: Option<i32>,
}

impl Ord for SkillCalc {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        format!("{self:?}").cmp(&format!("{o:?}"))
    }
}
impl PartialOrd for SkillCalc {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Cb {
    fn new(t: Tables) -> Self {
        let mut game = Game::new();
        game.lists.ensure_act(0).unwrap();
        let room = game.lists.create_room(0).unwrap();
        game.frame = 1000;
        Self {
            cx: t.ctx(),
            game,
            units: Units::new(),
            store: MonsterStore::new(),
            info: GameInfo {
                expansion: true,
                game_type: 3,
                players: 1,
                ..GameInfo::default()
            },
            room,
            stats: BTreeMap::new(),
            calls: Vec::new(),
            pos: BTreeMap::new(),
            states: BTreeSet::new(),
            udead: BTreeSet::new(),
            owners: BTreeMap::new(),
            minion_owners: BTreeMap::new(),
            minions: BTreeMap::new(),
            alignment: BTreeMap::new(),
            hostile: true,
            line: true,
            targets: BTreeMap::new(),
            target_pos: None,
            path_target: (0, 0),
            missile_flags: BTreeMap::new(),
            velocity: 0,
            missile_sl: (0, 0),
            uber: [false; 3],
            found: Vec::new(),
            calc: BTreeMap::new(),
            aura: None,
            lists: StatLists::new(Arc::new(StatData::default())),
            occupied: false,
            param0: 0,
            raise: false,
            sw_level: None,
            life: 0,
            area_level: None,
        }
    }

    fn add(&mut self, ty: UnitType, class: u32, seed: u32, mode: u32) -> UnitId {
        let u = self.game.spawn_unit(ty, Some(self.room), false).unwrap();
        let mut r = UnitRecord::new(ty, class, u.0);
        r.seed = Seed::init_low(seed);
        r.mode = mode;
        self.units.insert(u, r);
        u
    }

    /// A monster holding `umods` at (100, 200), level `level`.
    fn monster(&mut self, class: u32, umods: &[u8], unique: bool, level: i32) -> UnitId {
        let u = self.add(UnitType::Monster, class, 77, mode::NEUTRAL);
        let d = self.store.entry(u);
        d.umods[..umods.len()].copy_from_slice(umods);
        if unique {
            d.type_flags |= type_flag::UNIQUE;
        }
        self.stats.insert((u, stat::LEVEL), level);
        self.pos.insert(u, (100, 200));
        u
    }

    fn set_mode_of(&mut self, u: UnitId, m: u32) {
        self.units.get_mut(u).unwrap().mode = m;
    }

    fn s(&self, u: UnitId, s: u16) -> i32 {
        self.stats.get(&(u, s)).copied().unwrap_or(0)
    }

    fn seed_of(&self, u: UnitId) -> Seed {
        self.units.get(u).unwrap().seed
    }

    fn missiles(&self) -> Vec<MissileRequest<UnitId>> {
        self.calls
            .iter()
            .filter_map(|c| match c {
                Call::Missile(r) => Some(*r),
                _ => None,
            })
            .collect()
    }

    /// (event type, expire) of the unit's timers, sorted.
    fn timers(&self, u: UnitId) -> Vec<(u8, i32)> {
        let t = &self.game.timers;
        let mut v: Vec<_> = t
            .unit_timers(u)
            .into_iter()
            .filter_map(|id| Some((t.event(id)?.0, t.expire(id)?)))
            .collect();
        v.sort_unstable();
        v
    }

    fn run(&mut self, u: UnitId, arg: Option<UnitId>, mode: u8) {
        let cx = self.cx;
        dispatch(&cx, self, u, arg, mode);
    }
}

impl InitHost for Cb {
    fn game(&mut self) -> &mut Game {
        &mut self.game
    }
    fn units(&mut self) -> &mut Units {
        &mut self.units
    }
    fn monsters(&mut self) -> &mut MonsterStore {
        &mut self.store
    }
    fn info(&self) -> GameInfo {
        self.info
    }
    fn stat(&self, unit: UnitId, stat: u16) -> i32 {
        self.s(unit, stat)
    }
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32) {
        self.stats.insert((unit, stat), value);
    }
    fn minions(&mut self, boss: UnitId) -> Vec<UnitId> {
        self.minions.get(&boss).cloned().unwrap_or_default()
    }
    fn set_state(&mut self, unit: UnitId, state: u16) {
        self.calls.push(Call::State(unit, state));
    }
    fn position(&self, unit: UnitId) -> (i32, i32) {
        self.pos.get(&unit).copied().unwrap_or((0, 0))
    }
    fn has_state(&self, unit: UnitId, state: u16) -> bool {
        self.states.contains(&(unit, state))
    }
    fn has_state_in_group(&self, unit: UnitId, group: u8) -> bool {
        group == 33 && self.udead.contains(&unit)
    }
    fn set_mode(&mut self, unit: UnitId, mode: u32) {
        self.calls.push(Call::Mode(unit, mode));
        self.set_mode_of(unit, mode);
    }
    fn owner(&self, unit: UnitId) -> Option<UnitId> {
        self.owners.get(&unit).copied()
    }
    fn minion_owner(&mut self, unit: UnitId) -> Option<UnitId> {
        self.minion_owners.get(&unit).copied()
    }
    fn alignment(&self, unit: UnitId) -> u8 {
        self.alignment.get(&unit).copied().unwrap_or(0)
    }
    fn hostile(&self, _: UnitId, _: UnitId) -> bool {
        self.hostile
    }
    fn target(&self, unit: UnitId) -> Option<UnitId> {
        self.targets.get(&unit).copied()
    }
    fn target_position(&self, _: UnitId) -> Option<(i32, i32)> {
        self.target_pos
    }
    fn path_target_point(&self, _: UnitId) -> (i32, i32) {
        self.path_target
    }
    fn create_missile(&mut self, req: MissileRequest<UnitId>) -> Option<UnitId> {
        self.calls.push(Call::Missile(req));
        let m = self.add(UnitType::Missile, req.class as u32, 5, 0);
        self.owners.insert(m, req.owner);
        Some(m)
    }
    fn missile_row_flags(&self, class: i32) -> Option<u32> {
        self.missile_flags.get(&class).copied()
    }
    fn missile_velocity(&self, _: i32, level: i32) -> i32 {
        assert_eq!(level, 0);
        self.velocity
    }
    fn missile_skill_level(&self, _: UnitId) -> (i32, i32) {
        self.missile_sl
    }
    fn find_units(&mut self, near: UnitId, q: FindQuery) -> Vec<UnitId> {
        self.calls.push(Call::Find(near, q));
        self.found.clone()
    }
    fn line_clear(&mut self, _: (i32, i32), _: UnitId) -> bool {
        self.line
    }
    fn missile_hit(&mut self, src: UnitId, unit: UnitId, rec: &DamageRecord) {
        self.calls.push(Call::Hit(src, unit, *rec));
    }
    fn skill_calc(&mut self, _: UnitId, skill: u16, calc: SkillCalc, _: i32) -> i32 {
        assert_eq!(skill, 66);
        self.calc.get(&calc).copied().unwrap_or(0)
    }
    fn aura_fields(&self, skill: u16) -> Option<AuraFields> {
        assert_eq!(skill, 66);
        self.aura
    }
    fn stat_and_state_counts(&self) -> (i32, i32) {
        (100, 50)
    }
    fn apply_state(&mut self, req: StateApply) -> Option<ListId> {
        self.calls.push(Call::Apply(req));
        Some(self.lists.alloc(0, 0, 0, 0))
    }
    fn set_list_stat(&mut self, _: ListId, stat: u16, value: i32) {
        self.calls.push(Call::ListSet(stat, value));
    }
    fn free_minions(&mut self, unit: UnitId) {
        self.calls.push(Call::FreeMinions(unit));
    }
    fn clear_owner_data(&mut self, unit: UnitId) {
        self.calls.push(Call::ClearOwner(unit));
    }
    fn remove_pet(&mut self, owner: UnitId, pet: UnitId) {
        self.calls.push(Call::RemovePet(owner, pet));
    }
    fn is_undead(&self, unit: UnitId) -> bool {
        self.udead.contains(&unit)
    }
    fn missile_range(&self, class: i32) -> Option<i32> {
        (class == 348).then_some(21)
    }
    fn set_uber_death(&mut self, slot: usize) -> bool {
        self.uber[slot] = true;
        self.uber.iter().all(|&b| b)
    }
    fn game_8c(&self) -> i32 {
        2
    }
    fn quest_drop(&mut self, unit: UnitId, code: [u8; 4], arg: i32, announce: bool) {
        self.calls.push(Call::Drop(unit, code, arg, announce));
    }
    fn steal_belt_item(&mut self, unit: UnitId, target: UnitId) {
        self.calls.push(Call::Steal(unit, target));
    }
    fn spawn_near(&mut self, unit: UnitId, class: u32, mode: u32, spread: i32, flags: u32) {
        self.calls
            .push(Call::Spawn(unit, class, mode, spread, flags));
    }
    fn footprint_occupied(&mut self, _: UnitId) -> bool {
        self.occupied
    }
    fn ai_param0(&mut self, _: UnitId) -> i32 {
        self.param0
    }
    fn set_ai_param0(&mut self, unit: UnitId, v: i32) {
        self.calls.push(Call::SetParam(unit, v));
    }
    fn can_raise(&mut self, _: UnitId) -> bool {
        self.raise
    }
    fn ai_use_skill(&mut self, unit: UnitId, mode: u32, skill: u16) {
        self.calls.push(Call::UseSkill(unit, mode, skill));
    }
    fn skill_level(&mut self, _: UnitId, skill: u16) -> Option<i32> {
        assert_eq!(skill, 268);
        self.sw_level
    }
    fn life_percent(&self, _: UnitId) -> i32 {
        self.life
    }
    fn room_area_level(&mut self, _: UnitId) -> Option<i32> {
        self.area_level
    }
}

fn cbf() -> Cb {
    Cb::new(Tables::new(vec![mon(2, 5, 9, 32); 2]))
}

/// A seed whose first step gives `lo % m == r`.
fn seed_with(m: u32, r: u32) -> u32 {
    (1..).find(|&s| Seed::init_low(s).step() % m == r).unwrap()
}

// ----: the dispatcher ------------------------------------------------------

#[test]
fn every_table_callback_has_a_body() {
    let mut want: Vec<u32> = UMODS
        .iter()
        .flat_map(|r| r.callbacks)
        .filter(|&a| a != 0)
        .collect();
    want.sort_unstable();
    want.dedup();
    let mut have = addr::ALL.to_vec();
    have.sort_unstable();
    assert_eq!(want, have);
    // An address outside the table is still recorded, not run.
    let mut f = cbf();
    let u = f.monster(0, &[], false, 1);
    let cx = f.cx;
    assert!(!cb::run(&cx, &mut f, u, 1, false, 0x1234));
}

// ---- helpers --------------------------------------------------------------

#[test]
fn area_damage_skips_owners_and_needs_hostility_and_line() {
    let mut f = cbf();
    let u = f.monster(0, &[], false, 1);
    let o2 = f.add(UnitType::Player, 0, 1, 1);
    let m = f.add(UnitType::Missile, 117, 1, 0);
    let p = f.add(UnitType::Player, 0, 1, 1);
    f.owners.insert(m, u);
    f.owners.insert(u, o2);
    f.found = vec![u, o2, p];
    let rec = DamageRecord {
        fire: 7,
        ..DamageRecord::default()
    };
    assert!(cb::area_damage(&mut f, m, (5, 6), 3, &rec, false, false, 0));
    assert_eq!(
        f.calls,
        [
            Call::Find(
                m,
                FindQuery {
                    flags: 0x583,
                    x: 5,
                    y: 6,
                    r: 3,
                    ..FindQuery::default()
                }
            ),
            Call::Hit(m, p, rec),
        ]
    );
    // Both owners allowed: three hits, in found order.
    f.calls.clear();
    cb::area_damage(&mut f, m, (5, 6), 3, &rec, true, true, 0x581);
    let hits: Vec<_> = f
        .calls
        .iter()
        .filter_map(|c| match c {
            Call::Hit(_, v, _) => Some(*v),
            _ => None,
        })
        .collect();
    assert_eq!(hits, [u, o2, p]);
    // Not hostile, or the line blocked: nothing hit, 0.
    for (h, l) in [(false, true), (true, false)] {
        f.calls.clear();
        f.hostile = h;
        f.line = l;
        assert!(!cb::area_damage(&mut f, m, (5, 6), 3, &rec, true, true, 0));
        assert_eq!(f.calls.len(), 1);
    }
    // A source without an owner: 0 after the find.
    f.hostile = true;
    f.line = true;
    f.calls.clear();
    let lone = f.add(UnitType::Missile, 1, 1, 0);
    assert!(!cb::area_damage(
        &mut f,
        lone,
        (0, 0),
        1,
        &rec,
        true,
        true,
        0
    ));
    assert_eq!(f.calls.len(), 1);
    // A source that is not a missile: "hit" but no damage applied.
    f.calls.clear();
    f.owners.insert(p, u);
    assert!(cb::area_damage(&mut f, p, (0, 0), 1, &rec, false, false, 0));
    assert_eq!(f.calls.len(), 1);
}

#[test]
fn cross_burst_order_and_record() {
    let mut f = cbf();
    let u = f.monster(0, &[], false, 1);
    cb::cross_burst(&mut f, u, 195, 4);
    let ms = f.missiles();
    assert_eq!(ms.len(), 8);
    let got: Vec<_> = ms
        .iter()
        .map(|r| (r.target_x - 100, r.target_y - 200, r.init.unwrap().1))
        .collect();
    assert_eq!(
        got,
        [
            (0, -1, 0),
            (0, -1, 1),
            (1, 0, 0),
            (1, 0, 1),
            (0, 1, 0),
            (0, 1, 1),
            (-1, 0, 0),
            (-1, 0, 1),
        ]
    );
    for r in ms {
        assert_eq!((r.flags, r.class, r.level, r.skill), (0x21, 195, 4, 0));
        assert_eq!((r.x, r.y, r.owner), (100, 200, u));
        assert_eq!(r.init.unwrap().0, 0x005C_9290);
    }
}

#[test]
fn spectral_vector() {
    let mut t = Tables::new(vec![mon(2, 5, 9, 32)]);
    t.monlvl[30].l_dm = 19;
    let mut f = Cb::new(t);
    // Normal, L-flag 1 (game type 3).
    let s = seed_with(5, 3);
    let u = f.monster(0, &[27], true, 30);
    f.units.get_mut(u).unwrap().seed = Seed::init_low(s);
    f.run(u, None, 0);
    assert_eq!(f.s(u, cb::stat::COLDMINDAM), 12);
    assert_eq!(f.s(u, cb::stat::COLDMAXDAM), 19);
    assert_eq!(f.s(u, cb::stat::COLDLENGTH), 40);
    // Not unique: no draw, nothing set.
    let v = f.monster(0, &[27], false, 30);
    let before = f.seed_of(v);
    f.run(v, None, 0);
    assert_eq!(f.seed_of(v), before);
    assert_eq!(f.s(v, cb::stat::COLDMINDAM), 0);
    // Mode 5: the missile's seed steps; a NoUniqueMod row sets nothing.
    let m = f.add(UnitType::Missile, 9, s, 0);
    f.stats.insert((m, stat::LEVEL), 30);
    f.missile_flags.insert(9, 0);
    f.run(u, Some(m), 5);
    assert_eq!(f.s(m, cb::stat::COLDMAXDAM), 19);
    let n = f.add(UnitType::Missile, 10, s, 0);
    f.missile_flags.insert(10, 1 << 13);
    f.run(u, Some(n), 5);
    assert_ne!(f.seed_of(n), Seed::init_low(s), "the missile's seed steps");
    assert_eq!(f.s(n, cb::stat::COLDMAXDAM), 0);
    // Rows 0–4 by lo' mod 5.
    assert_eq!(cb::SPECTRAL[0], (48, 49, -1));
    assert_eq!(cb::SPECTRAL[4], (57, 58, 59));
}

#[test]
fn think_restart_vectors() {
    let mut ms = vec![mon(2, 5, 9, 32); 2];
    ms[0].baseid = 19;
    ms[1].baseid = 110;
    let mut f = Cb::new(Tables::new(ms));
    let cx = f.cx;
    for (class, mode, want) in [
        (0, 2, vec![]),
        (1, 2, vec![(2, 1002)]),
        (0, 1, vec![(2, 1002)]),
        (1, 1, vec![(2, 1002)]),
        (1, 0, vec![]),
        (1, 12, vec![]),
    ] {
        let u = f.monster(class, &[], false, 1);
        f.set_mode_of(u, mode);
        f.game.schedule_event(u, 2, 1500, None, 0, 0).unwrap();
        cb::think_restart(&cx, &mut f, u);
        assert_eq!(f.timers(u), want, "class {class} mode {mode}");
    }
    // Umod 41: alive → restart + event 7 at F + 75; dead → nothing.
    let u = f.monster(1, &[41], false, 1);
    f.set_mode_of(u, 2);
    f.run(u, None, 2);
    assert_eq!(f.timers(u), [(2, 1002), (7, 1075)]);
    let d = f.monster(1, &[41], false, 1);
    f.set_mode_of(d, mode::DEAD);
    f.run(d, None, 2);
    assert!(f.timers(d).is_empty());
}

// --------------------------------------------------------------------------

#[test]
fn death_event_schedules() {
    let mut f = cbf();
    for (umod, unique, mode, n) in [
        (10, false, 0, 1),
        (18, false, 0, 0),
        (18, true, 0, 1),
        (31, false, 0, 1),
        (32, false, 1, 0),
        (42, false, 0, 1),
        (9, false, 0, 0),
        (9, true, 0, 1),
        (9, true, 3, 0),
    ] {
        let u = f.monster(0, &[umod], unique, 1);
        f.set_mode_of(u, mode);
        f.run(u, None, 1);
        let want: Vec<(u8, i32)> = (0..n).map(|_| (7, 1004)).collect();
        assert_eq!(f.timers(u), want, "umod {umod}");
    }
}

#[test]
fn curse_level_and_cast() {
    assert_eq!([4, 34, 0].map(cb::curse_level), [1, 7, 1], "§5 test vector");
    let mut f = cbf();
    f.aura = Some(AuraFields {
        stats: [5, 7, -1, 100, 8, 0],
        target_state: 9,
    });
    f.calc.insert(SkillCalc::AuraRange, 99);
    f.calc.insert(SkillCalc::AuraLen, 250);
    f.calc.insert(SkillCalc::AuraStat(1), -20);
    f.calc.insert(SkillCalc::AuraStat(2), 3);
    f.calc.insert(SkillCalc::AuraStat(5), 4);
    f.target_pos = Some((30, 40));
    // lo' & 3 = 0: no cast (one step).
    let s0 = seed_with(4, 0);
    let u = f.monster(0, &[7], true, 34);
    f.units.get_mut(u).unwrap().seed = Seed::init_low(s0);
    let v = f.add(UnitType::Player, 0, 1, 1);
    f.found = vec![v];
    f.run(u, None, 3);
    assert!(f.calls.is_empty());
    // lo' & 3 ≠ 0: cast, range clamped to 40.
    let s1 = seed_with(4, 1);
    f.units.get_mut(u).unwrap().seed = Seed::init_low(s1);
    f.run(u, None, 3);
    assert_eq!(
        f.calls,
        [
            Call::Find(
                u,
                FindQuery {
                    flags: 3,
                    exclude: Some(u),
                    x: 30,
                    y: 40,
                    r: 40,
                    ..FindQuery::default()
                }
            ),
            Call::Apply(StateApply {
                source: u,
                target: v,
                skill: 66,
                level: 7,
                duration: 250,
                stat: 5,
                value: -20,
                state: 9,
                callback: 0,
            }),
            Call::ListSet(7, 3),
            Call::ListSet(8, 4),
        ]
    );
    // Not unique: no draw.
    let w = f.monster(0, &[7], false, 34);
    let before = f.seed_of(w);
    f.run(w, None, 3);
    assert_eq!(f.seed_of(w), before);
    // A target state outside the table: nothing applied.
    f.calls.clear();
    f.aura = Some(AuraFields {
        stats: [5, 0, 0, 0, 0, 0],
        target_state: 50,
    });
    f.units.get_mut(u).unwrap().seed = Seed::init_low(s1);
    f.run(u, None, 3);
    assert_eq!(f.calls.len(), 1);
}

#[test]
fn fire_explosion_vectors() {
    assert_eq!(cb::fire_range(100, 50, 0), (38, 22));
    assert_eq!(cb::fire_range(1000, 35, 1), (234, 140));
    assert_eq!(cb::fire_range(2000, 20, 2), (50, 30));
    assert_eq!(cb::fire_range(2, 50, 0), (1, 0));
    // The whole callback: H = maxHP (monlvl HP 100 × MaxHP / 100).
    let mut t = Tables::new(vec![mon(2, 5, 100, 32)]);
    for (d, ce) in t.difficultylevels.iter_mut().zip([50, 35, 20]) {
        d.monstercedamagepercent = ce;
    }
    let mut f = Cb::new(t);
    let u = f.monster(0, &[9], false, 1);
    let m_owner = f.add(UnitType::Player, 0, 1, 1);
    let p = f.add(UnitType::Player, 0, 1, 1);
    f.found = vec![p];
    let mut s = f.seed_of(u);
    let dmg = 22 + s.roll(16) as i32;
    f.run(u, None, 2);
    assert_eq!(f.seed_of(u), s, "one roll(16) on U");
    let ms = f.missiles();
    assert_eq!(ms.len(), 1);
    assert_eq!(
        (
            ms[0].class,
            ms[0].level,
            ms[0].flags,
            ms[0].target_x,
            ms[0].target_y
        ),
        (117, 1, 0x21, 100, 200)
    );
    let rec = DamageRecord {
        physical: dmg << 6,
        fire: dmg << 6,
        ..DamageRecord::default()
    };
    match &f.calls[1..] {
        [Call::Find(_, q), Call::Hit(_, v, r)] => {
            assert_eq!((q.flags, q.r, q.x, q.y), (0x581, 4, 100, 200));
            assert_eq!((*v, *r), (p, rec));
        }
        other => panic!("{other:?}"),
    }
    let _ = m_owner;
    // a = 1: roll(1) still steps.
    let mut t = Tables::new(vec![mon(2, 5, 2, 32)]);
    t.difficultylevels[0].monstercedamagepercent = 50;
    let mut f = Cb::new(t);
    let u = f.monster(0, &[9], false, 1);
    let mut s = f.seed_of(u);
    s.step();
    f.run(u, None, 2);
    assert_eq!(f.seed_of(u), s);
    // Difficulty 3 (no record): the missile only.
    let mut f = cbf();
    f.info.difficulty = 3;
    let u = f.monster(0, &[9], false, 1);
    let before = f.seed_of(u);
    f.run(u, None, 2);
    assert_eq!(f.seed_of(u), before);
    assert_eq!(f.calls.len(), 1);
}

#[test]
fn poisondead_level() {
    let mut f = cbf();
    for (lv, l) in [(0, 1), (1, 1), (2, 2), (9, 9)] {
        f.calls.clear();
        let u = f.monster(0, &[10], false, lv);
        f.run(u, None, 2);
        let m = f.missiles();
        assert_eq!((m.len(), m[0].class, m[0].level), (1, 155, l));
    }
}

#[test]
fn spcdamage_vectors() {
    assert_eq!(
        cb::trap_stats(326, 10),
        [(21, 0), (22, 0), (48, 5), (49, 15)]
    );
    assert_eq!(cb::trap_stats(327, 10), [(21, 6), (22, 10)]);
    assert_eq!(cb::trap_stats(328, 2), [(21, 2), (22, 2)]);
    assert_eq!(
        cb::trap_stats(329, 10),
        [(21, 0), (22, 0), (57, 10), (58, 20), (59, 20)]
    );
    assert_eq!(
        cb::trap_stats(330, 10),
        [(21, 0), (22, 0), (50, 5), (51, 15)]
    );
    assert_eq!(cb::trap_stats(369, 10), cb::trap_stats(330, 10));
    assert!(cb::trap_stats(354, 10).is_empty());
    let mut f = Cb::new(Tables::new(vec![mon(2, 5, 9, 32); 330]));
    for (al, level, tohit) in [
        (Some(10), 10, 60),
        (None, 2, 52),
        (Some(50), 50, 90),
        (Some(0), 1, 51),
    ] {
        f.area_level = al;
        let u = f.monster(327, &[14], false, 1);
        f.run(u, None, 0);
        assert_eq!((f.s(u, 12), f.s(u, 19)), (level, tohit));
        assert_eq!(f.s(u, 22), level);
    }
}

#[test]
fn partydead() {
    let mut f = cbf();
    let o = f.monster(0, &[], false, 1);
    let u = f.monster(0, &[15], false, 1);
    let a = f.monster(0, &[], false, 1);
    let b = f.monster(0, &[], false, 1);
    f.set_mode_of(b, mode::DEATH);
    f.minions.insert(u, vec![a, b]);
    f.minion_owners.insert(u, o);
    // Not dying: nothing.
    f.run(u, None, 1);
    assert!(f.calls.is_empty());
    f.set_mode_of(u, mode::DEATH);
    f.run(u, None, 1);
    assert_eq!(
        f.calls,
        [
            Call::FreeMinions(a),
            Call::ClearOwner(a),
            Call::Mode(a, 0),
            Call::FreeMinions(b),
            Call::ClearOwner(b),
            Call::ClearOwner(o),
            Call::Mode(o, 0),
        ]
    );
    // The owner is the unit itself: no mode set on it.
    f.calls.clear();
    f.minions.clear();
    f.minion_owners.insert(u, u);
    f.run(u, None, 1);
    assert_eq!(f.calls, [Call::ClearOwner(u)]);
}

#[test]
fn lightning_burst_and_cooldown() {
    assert_eq!([1, 9].map(cb::burst_level), [1, 4]);
    let mut f = cbf();
    let u = f.monster(0, &[17], true, 9);
    // Mode 1 in GH: event 7 at F + 2.
    f.set_mode_of(u, mode::GETHIT);
    f.run(u, None, 1);
    assert_eq!(f.timers(u), [(7, 1002)]);
    // Last burst F − 9: refused, bit 0x100 cleared.
    f.store.entry(u).last_burst = 991;
    f.store.entry(u).type_flags |= 0x100;
    f.run(u, None, 2);
    assert!(f.missiles().is_empty());
    assert_eq!(f.store.get(u).unwrap().type_flags & 0x100, 0);
    // F − 10: bursts, +0x18 := F.
    f.store.entry(u).last_burst = 990;
    f.run(u, None, 2);
    let ms = f.missiles();
    assert_eq!(ms.len(), 8);
    assert!(ms.iter().all(|m| m.class == 195 && m.level == 4));
    let d = f.store.get(u).unwrap();
    assert_eq!((d.last_burst, d.type_flags & 0x100), (1000, 0x100));
    // Mode 4: bursts at once unless in GH.
    f.calls.clear();
    f.store.entry(u).last_burst = 0;
    f.run(u, None, 4);
    assert!(f.missiles().is_empty(), "in GH");
    f.set_mode_of(u, mode::NEUTRAL);
    f.run(u, None, 4);
    assert_eq!(f.missiles().len(), 8);
    // Not unique: nothing.
    let v = f.monster(0, &[17], false, 9);
    f.calls.clear();
    f.run(v, None, 2);
    f.run(v, None, 4);
    assert!(f.calls.is_empty());
}

#[test]
fn cold_ring() {
    let mut f = cbf();
    f.velocity = 9;
    for (lv, l) in [(3, 1), (40, 20)] {
        f.calls.clear();
        let u = f.monster(0, &[18], true, lv);
        f.run(u, None, 2);
        let ms = f.missiles();
        assert_eq!(ms.len(), 64);
        assert!(ms
            .iter()
            .all(|m| (m.class, m.level, m.flags, m.velocity) == (194, l, 7, 9)));
    }
    let u = f.monster(0, &[18], false, 40);
    f.calls.clear();
    f.run(u, None, 2);
    assert!(f.calls.is_empty());
}

#[test]
fn hireable_vectors() {
    assert_eq!(cb::hireable_values(3, 7, 2, 5), (4, 11));
    assert_eq!(cb::hireable_values(0, 1, 0, 0), (0, 0));
    assert_eq!(cb::hireable_missile_values(512, 1024, 2, 5), (768, 2048));
    let mut f = Cb::new(Tables::new(vec![mon(2, 5, 9, 32); 272]));
    let u = f.monster(1, &[19], false, 1);
    for (s, v) in [(21, 3), (22, 7), (23, 2), (24, 5)] {
        f.stats.insert((u, s), v);
    }
    f.run(u, None, 0);
    assert_eq!((f.s(u, 21), f.s(u, 22)), (4, 11));
    // Class 271 (roguehire): nothing at mode 0; its missiles get the
    // secondary damage.
    let r = f.monster(271, &[19], false, 1);
    for (s, v) in [(21, 3), (22, 7), (23, 2), (24, 5)] {
        f.stats.insert((r, s), v);
    }
    f.run(r, None, 0);
    assert_eq!((f.s(r, 21), f.s(r, 22)), (3, 7));
    let m = f.add(UnitType::Missile, 1, 1, 0);
    f.owners.insert(m, r);
    f.stats.insert((m, 21), 512);
    f.stats.insert((m, 22), 1024);
    f.run(r, Some(m), 5);
    assert_eq!((f.s(m, 21), f.s(m, 22)), (768, 2048));
    // Another owner class: unchanged.
    let n = f.add(UnitType::Missile, 1, 1, 0);
    f.owners.insert(n, u);
    f.stats.insert((n, 22), 1024);
    f.run(r, Some(n), 5);
    assert_eq!(f.s(n, 22), 1024);
}

#[test]
fn scarab_and_poisonhit() {
    let mut f = cbf();
    let u = f.monster(0, &[20], false, 6);
    f.run(u, None, 1);
    assert!(f.timers(u).is_empty());
    for m in [mode::GETHIT, mode::DEATH] {
        let v = f.monster(0, &[20], false, 6);
        f.set_mode_of(v, m);
        f.run(v, None, 1);
        assert_eq!(f.timers(v), [(7, 1002)]);
    }
    f.run(u, None, 2);
    let ms = f.missiles();
    assert!(ms.len() == 8 && ms.iter().all(|m| m.class == 225 && m.level == 6));
    f.calls.clear();
    let q = f.monster(0, &[23], false, 0);
    f.run(q, None, 0);
    let ms = f.missiles();
    assert!(ms.len() == 8 && ms.iter().all(|m| m.class == 321 && m.level == 0));
}

#[test]
fn killself() {
    let mut f = cbf();
    let u = f.monster(0, &[21], false, 1);
    f.states.insert((u, 54));
    f.run(u, None, 2);
    assert_eq!(f.timers(u), [(7, 1003)]);
    f.states.clear();
    let p = f.add(UnitType::Player, 0, 1, 1);
    f.minion_owners.insert(u, p);
    f.run(u, None, 2);
    assert_eq!(f.calls, [Call::RemovePet(p, u)]);
    f.calls.clear();
    f.minion_owners.clear();
    f.run(u, None, 2);
    assert_eq!(f.calls, [Call::Mode(u, 0)]);
    // Dead: nothing.
    f.calls.clear();
    f.run(u, None, 2);
    assert!(f.calls.is_empty());
}

#[test]
fn questcomplete_calls() {
    assert_eq!(cb::quest_death_call(156), Some(0x005D_FE00));
    assert_eq!(cb::quest_death_call(267), Some(0x005D_FD90));
    assert_eq!(cb::quest_death_call(541), Some(0x005E_0060));
    assert_eq!(cb::quest_death_call(709), Some(0x005E_0070));
    assert_eq!(cb::quest_death_call(706), None);
    let mut f = Cb::new(Tables::new(vec![mon(2, 5, 9, 32); 710]));
    let u = f.monster(229, &[22], false, 1);
    // An evil minion, a good one and Diablo's clone around Radament.
    let m = f.monster(5, &[], false, 1);
    let good = f.monster(5, &[], false, 1);
    f.alignment.insert(good, 2);
    let clone = f.monster(333, &[], false, 1);
    f.found = vec![m, good, clone];
    f.run(u, None, 1);
    assert!(f.calls.is_empty());
    f.set_mode_of(u, 0);
    let mut s = f.seed_of(u);
    f.run(u, None, 1);
    // Purge (35, 40, max(21 − 100, 100) = 100, 0): flags 0x583 around
    // Radament, Radament excluded.
    let Call::Find(near, q) = f.calls[0] else {
        panic!("{:?}", f.calls)
    };
    assert_eq!((near, q.flags, q.r, q.exclude), (u, 0x583, 35, Some(u)));
    // Only the evil non-clone monster: event 7 at F + 40 + rnd(60) on
    // Radament's seed, then umod 21.
    let at = f.game.frame + 40 + s.roll(60) as i32;
    assert_eq!(f.seed_of(u), s);
    assert!(f.timers(m).contains(&(EVENT_UMOD as u8, at)));
    assert!(f.timers(good).is_empty() && f.timers(clone).is_empty());
    assert_eq!(f.store.entry(m).umod_list(), [21]);
    // Then missile 347 at Radament's position, skill 0, level 1.
    let ms = f.missiles();
    assert_eq!(ms.len(), 1);
    assert_eq!(
        (ms[0].class, ms[0].flags, ms[0].x, ms[0].y, ms[0].level),
        (347, 1, 100, 200, 1)
    );
    // Mephisto: missile 299, flags 0x8000, range 100.
    f.calls.clear();
    f.found.clear();
    let me = f.monster(242, &[22], false, 1);
    f.set_mode_of(me, 0);
    f.run(me, None, 1);
    let ms = f.missiles();
    assert_eq!(
        (ms[0].class, ms[0].flags, ms[0].range, ms[0].origin),
        (299, 0x8000, 100, Some(me))
    );
    // The ubers: drops only when all three died, then on every death.
    f.calls.clear();
    for c in [704, 705] {
        let x = f.monster(c, &[22], false, 1);
        f.set_mode_of(x, 0);
        f.run(x, None, 1);
    }
    assert!(f.calls.is_empty());
    let b = f.monster(709, &[22], false, 1);
    f.set_mode_of(b, 0);
    f.run(b, None, 1);
    assert_eq!(
        f.calls,
        [
            Call::Drop(b, *b"cm2 ", 7, true),
            Call::Drop(b, *b"std ", 2, false),
            Call::Drop(b, *b"std ", 2, false),
        ]
    );
}

#[test]
fn thief_gate() {
    let mut f = cbf();
    let p = f.add(UnitType::Player, 0, 1, 1);
    let u = f.monster(0, &[24], true, 1);
    f.targets.insert(u, p);
    f.units.get_mut(u).unwrap().seed = Seed::init_low(seed_with(100, 29));
    f.run(u, None, 3);
    assert!(f.calls.is_empty());
    f.units.get_mut(u).unwrap().seed = Seed::init_low(seed_with(100, 30));
    f.run(u, None, 3);
    assert_eq!(f.calls, [Call::Steal(u, p)]);
}

#[test]
fn multishot_copies() {
    assert_eq!(
        cb::multishot_targets((10, 10), (20, 10), 5),
        [(20, 9), (20, 11)]
    );
    assert_eq!(
        cb::multishot_targets((10, 10), (20, 10), 64),
        [(20, 10), (20, 10)]
    );
    let mut f = cbf();
    let o = f.monster(0, &[29], true, 1);
    f.pos.insert(o, (10, 10));
    let t = f.add(UnitType::Player, 0, 1, 1);
    f.pos.insert(t, (20, 10));
    f.targets.insert(o, t);
    f.missile_flags.insert(5, 0);
    f.missile_sl = (12, 3);
    let m = f.add(UnitType::Missile, 5, 1, 0);
    f.owners.insert(m, o);
    f.run(o, Some(m), 5);
    let ms = f.missiles();
    let got: Vec<_> = ms
        .iter()
        .map(|r| (r.class, r.skill, r.level, r.target_x, r.target_y, r.owner))
        .collect();
    assert_eq!(got, [(5, 12, 3, 20, 9, o), (5, 12, 3, 20, 11, o)]);
    assert_eq!(f.store.get(o).unwrap().type_flags & 0x80, 0);
    // A copy in progress, or NoMultiShot: nothing.
    f.calls.clear();
    f.store.entry(o).type_flags |= 0x80;
    f.run(o, Some(m), 5);
    assert!(f.missiles().is_empty());
    f.store.entry(o).type_flags &= !0x80;
    f.missile_flags.insert(5, 1 << 12);
    f.run(o, Some(m), 5);
    assert!(f.missiles().is_empty());
    // No target: the missile path's target point.
    f.missile_flags.insert(5, 0);
    f.targets.clear();
    f.path_target = (5, 10);
    f.run(o, Some(m), 5);
    let got: Vec<_> = f
        .missiles()
        .iter()
        .map(|r| (r.target_x, r.target_y))
        .collect();
    assert_eq!(got, [(5, 11), (5, 9)]);
}

#[test]
fn goboom() {
    let mut f = cbf();
    let u = f.monster(0, &[31], false, 1);
    let p = f.add(UnitType::Player, 0, 1, 1);
    f.found = vec![p];
    let before = f.seed_of(u);
    f.run(u, None, 2);
    assert_eq!(f.seed_of(u), before, "no draw on U");
    match &f.calls[1..] {
        [Call::Find(_, q), Call::Hit(_, _, r)] => {
            assert_eq!((q.flags, q.r), (0x583, 6));
            assert_eq!((r.physical, r.fire), (0x6400, 0x6400));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn firespike_draw_only() {
    let mut m = mon(2, 5, 9, 32);
    m.el1mind = 5;
    m.el1maxd = 10;
    m.aip3 = 15;
    let mut f = Cb::new(Tables::new(vec![m]));
    let u = f.monster(0, &[32], false, 4);
    let o = f.add(UnitType::Player, 0, 1, 1);
    f.owners.insert(u, o);
    f.found = vec![f.add(UnitType::Player, 0, 1, 1)];
    let mut s = f.seed_of(u);
    s.roll(5);
    f.run(u, None, 2);
    assert_eq!(f.seed_of(u), s);
    match f.calls.as_slice() {
        [Call::Find(near, q)] => {
            assert_eq!(*near, u);
            assert_eq!((q.flags, q.r), (0x583, 60));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn suicide_minion() {
    let (r, m) = cb::suicide_record(1, 512);
    assert_eq!((r.result, r.physical, r.fire, m), (8, 512, 512, 427));
    let (r, m) = cb::suicide_record(15, 512);
    assert_eq!((r.cold, r.cold_len, r.fire, m), (512, 15, 0, 428));
    let (r, m) = cb::suicide_record(0, 512);
    assert_eq!((r.cold, r.fire, m), (0, 0, 426));
    // Real-values vector shape: L-DM 20, A1MinD 100, A1MaxD 150.
    let mut ms = mon(2, 5, 9, 32);
    ms.a1mind = 100;
    ms.a1maxd = 150;
    ms.aip1 = 15;
    ms.aip4 = 4;
    let mut t = Tables::new(vec![ms.clone()]);
    t.monlvl[31].l_dm = 20;
    let monlvl = t.monlvl.clone();
    assert_eq!(a1_damage(&ms, &monlvl, true, 0, 31), (20, 30));
    let mut f = Cb::new(t);
    let u = f.monster(0, &[33], false, 31);
    let p = f.add(UnitType::Player, 0, 1, 1);
    f.found = vec![p];
    let mut s = f.seed_of(u);
    let v = (20 + s.roll(10) as i32) * 256;
    f.run(u, None, 2);
    assert_eq!(f.seed_of(u), s);
    match f.calls.as_slice() {
        [Call::Missile(mr), Call::Find(_, q), Call::Hit(_, _, r)] => {
            assert_eq!(mr.class, 428);
            assert_eq!((q.flags, q.r), (0x581, 4));
            assert_eq!((r.cold, r.cold_len, r.physical, r.result), (v, 15, v, 8));
        }
        other => panic!("{other:?}"),
    }
    // Mode 1 from GH: mode set 0, then event 7 at F + 4.
    f.calls.clear();
    f.set_mode_of(u, mode::GETHIT);
    f.run(u, None, 1);
    assert_eq!(f.calls, [Call::Mode(u, 0)]);
    assert_eq!(f.timers(u), [(7, 1004)]);
    for m in [mode::DEATH, mode::DEAD] {
        let w = f.monster(0, &[33], false, 31);
        f.set_mode_of(w, m);
        f.run(w, None, 1);
        assert_eq!(f.timers(w), [(7, 1004)]);
    }
}

#[test]
fn ai_after_death_and_revive() {
    let mut ms = mon(2, 5, 9, 32);
    ms.aip8 = 30;
    ms.aip1 = 10;
    let mut f = Cb::new(Tables::new(vec![ms]));
    for (r, want) in [(29, vec![(7, 1101)]), (30, vec![])] {
        let u = f.monster(0, &[34], false, 1);
        f.set_mode_of(u, mode::DEATH);
        f.game.schedule_event(u, 7, 1500, None, 0, 0).unwrap();
        f.units.get_mut(u).unwrap().seed = Seed::init_low(seed_with(100, r));
        f.run(u, None, 1);
        assert_eq!(f.timers(u), want, "lo' % 100 = {r}");
    }
    // Alignment ≠ 0: nothing, no draw.
    let u = f.monster(0, &[34], false, 1);
    f.set_mode_of(u, mode::DEATH);
    f.alignment.insert(u, 1);
    let before = f.seed_of(u);
    f.run(u, None, 1);
    assert_eq!(f.seed_of(u), before);
    // Revive: AI param 0 = 2 → only the type-2 cancel.
    let u = f.monster(0, &[34], false, 1);
    f.set_mode_of(u, mode::DEAD);
    f.game.schedule_event(u, 2, 1500, None, 0, 0).unwrap();
    f.param0 = 2;
    f.raise = true;
    f.run(u, None, 2);
    assert!(f.timers(u).is_empty());
    assert!(f.calls.is_empty());
    // Footprint occupied: event 7 at F + 101.
    f.occupied = true;
    f.run(u, None, 2);
    assert_eq!(f.timers(u), [(7, 1101)]);
    // The revive itself.
    f.occupied = false;
    f.param0 = 1;
    let v = f.monster(0, &[34], false, 1);
    f.set_mode_of(v, mode::DEAD);
    f.units.get_mut(v).unwrap().flags = 0x0402_0001;
    f.run(v, None, 2);
    assert_eq!(f.calls, [Call::UseSkill(v, 8, 293), Call::SetParam(v, 2)]);
    assert_eq!(f.units.get(v).unwrap().flags, 1);
    assert_eq!(f.timers(v), [(2, 1051)]);
    // Alive, udead, unique, alignment 2, raise refused: nothing.
    f.calls.clear();
    let w = f.monster(0, &[34], false, 1);
    f.run(w, None, 2);
    for setup in 0..4 {
        let x = f.monster(0, &[34], setup == 1, 1);
        f.set_mode_of(x, mode::DEAD);
        match setup {
            0 => {
                f.udead.insert(x);
            }
            2 => {
                f.alignment.insert(x, 2);
            }
            _ => f.raise = false,
        }
        f.run(x, None, 2);
    }
    assert!(f.calls.is_empty());
}

#[test]
fn shatter_and_worms() {
    let mut f = cbf();
    let u = f.monster(0, &[35, 40], false, 1);
    f.run(u, None, 1);
    assert!(f.calls.is_empty());
    f.set_mode_of(u, 0);
    f.run(u, None, 1);
    assert_eq!(
        f.calls,
        [
            Call::State(u, 107),
            Call::Spawn(u, 551, 1, 1, 0),
            Call::State(u, 104),
        ]
    );
}

#[test]
fn lightningdeath_level() {
    assert_eq!(cb::lightning_death_level(Some(1), 0), 1);
    assert_eq!(cb::lightning_death_level(Some(2), 0), 2);
    assert_eq!(cb::lightning_death_level(Some(40), 0), 15);
    assert_eq!(cb::lightning_death_level(None, 0), 1);
    assert_eq!(cb::lightning_death_level(Some(40), 11), 1);
    assert_eq!(cb::lightning_death_level(Some(40), 10), 15);
    let mut f = cbf();
    f.velocity = 4;
    let o = f.add(UnitType::Player, 0, 1, 1);
    let u = f.monster(0, &[42], true, 1);
    f.minion_owners.insert(u, o);
    f.sw_level = Some(6);
    f.run(u, None, 2);
    let ms = f.missiles();
    assert!(
        ms.len() == 64
            && ms
                .iter()
                .all(|m| (m.class, m.level, m.velocity) == (90, 4, 4))
    );
    let v = f.monster(0, &[42], false, 1);
    f.calls.clear();
    f.run(v, None, 2);
    assert!(f.calls.is_empty());
}

#[test]
fn any_event7_runs_every_mode2_callback() {
    // Fire enchanted with umod 41: alive, each event 7 explodes too.
    let mut f = cbf();
    let u = f.monster(0, &[9, 41], true, 1);
    f.run(u, None, 2);
    assert_eq!(f.missiles().len(), 1);
    assert!(f.timers(u).contains(&(7, 1075)));
    let _ = TimerClass::Monster;
}

#[test]
fn unique_is_the_flag_of_the_walked_monster_and_mode_5_uses_the_owner() {
    // Mode 5: the callback gets the missile, unique is the OWNER's flag
    // 8 (umod 29 multishot copies only for a unique).
    for unique in [false, true] {
        let mut f = cbf();
        let o = f.monster(0, &[29], unique, 1);
        f.pos.insert(o, (10, 10));
        let t = f.add(UnitType::Player, 0, 1, 1);
        f.pos.insert(t, (20, 10));
        f.targets.insert(o, t);
        f.missile_flags.insert(5, 0);
        let m = f.add(UnitType::Missile, 5, 1, 0);
        f.owners.insert(m, o);
        f.run(o, Some(m), 5);
        assert_eq!(
            f.missiles().len(),
            if unique { 2 } else { 0 },
            "unique {unique}"
        );
    }
    // Mode 2: the walked monster's own flag (umod 41 ... via umod 9 fire
    // explosion, a unique or not alike; umod 27 spectral hit differs).
    for unique in [false, true] {
        let mut f = cbf();
        let u = f.monster(0, &[18], unique, 1);
        f.set_mode_of(u, mode::DEATH);
        f.run(u, None, 1);
        assert_eq!(
            f.timers(u).len(),
            usize::from(unique),
            "umod 18 unique {unique}"
        );
    }
}

#[test]
fn one_u_step_and_roll_below_1_draws_nothing() {
    // `roll(n)` with n < 1 does not step; the curse callback (umod 7,
    // unique) takes exactly one U step.
    let mut s = Seed::init_low(77);
    let before = s;
    assert_eq!(s.roll(0), 0);
    assert_eq!(s.roll(-3), 0);
    assert_eq!(s, before);
    let mut f = cbf();
    let u = f.monster(0, &[7], true, 1);
    let mut want = f.seed_of(u);
    let lo = want.step();
    f.run(u, None, 3);
    assert_eq!(f.seed_of(u), want, "one generator step, lo' = {lo}");
    // A unit with no draws leaves the seed alone.
    let v = f.monster(0, &[10], false, 1);
    let kept = f.seed_of(v);
    f.run(v, None, 2);
    assert_eq!(f.seed_of(v), kept);
}

#[test]
fn fire_and_suicide_hit_players_only_goboom_monsters_too_and_never_the_owners() {
    let mut ms = mon(2, 5, 100, 32);
    ms.a1mind = 100;
    ms.a1maxd = 150;
    ms.aip1 = 15;
    ms.aip4 = 4;
    for (umod, flags) in [(9u8, 0x581u32), (33, 0x581), (31, 0x583)] {
        let mut f = Cb::new(Tables::new(vec![ms.clone()]));
        let u = f.monster(0, &[umod], false, 1);
        let o2 = f.add(UnitType::Player, 0, 1, 1);
        let p = f.add(UnitType::Player, 0, 1, 1);
        // The exploding monster u owns the missile; its owner is o2.
        f.owners.insert(u, o2);
        f.found = vec![u, o2, p];
        f.run(u, None, 2);
        let finds: Vec<_> = f
            .calls
            .iter()
            .filter_map(|c| match c {
                Call::Find(_, q) => Some(q.flags),
                _ => None,
            })
            .collect();
        assert_eq!(finds, [flags], "umod {umod}");
        let hits: Vec<_> = f
            .calls
            .iter()
            .filter_map(|c| match c {
                Call::Hit(_, v, _) => Some(*v),
                _ => None,
            })
            .collect();
        assert_eq!(hits, [p], "umod {umod}: owner and owner's owner never hit");
    }
}

#[test]
fn always_run_ai_chains_and_an_extra_event_starts_a_second_chain() {
    let mut f = cbf();
    // Dead: nothing at all.
    let d = f.monster(0, &[41], false, 1);
    f.set_mode_of(d, mode::DEAD);
    f.run(d, None, 2);
    assert!(f.timers(d).is_empty());
    // Alive (NU): think restart (event 2 at F + 2), then event 7 at F + 75.
    let u = f.monster(0, &[41], false, 1);
    f.run(u, None, 2);
    assert_eq!(f.timers(u), [(2, 1002), (7, 1075)]);
    // The event that ran re-schedules itself; a second type-7 event of
    // the same monster adds a second chain.
    f.run(u, None, 2);
    assert_eq!(f.timers(u), [(2, 1002), (7, 1075), (7, 1075)]);
}
