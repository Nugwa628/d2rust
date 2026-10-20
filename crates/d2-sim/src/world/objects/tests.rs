// Spec: specs/world/objects.md §2–§7, §14 (Test vectors, Edge cases)
use std::collections::BTreeSet;

use super::fake::*;
use super::*;
use crate::world::{tsv_num, tsv_rows, TsvError};

const FUNCS_TSV: &str = include_str!("../../../../../tables/world/object-functions.tsv");
const HEADER: &[&str] = &["kind", "index", "address", "label", "rows", "owner"];

// ------------------------------------------------------------------ fixture

/// `effectclass` per shrines.txt row: the 1.14d lists of §2 (class 0: {0};
/// 1: {16..22}; 2: {2, 4}; 3: {3, 5}; 4: {1, 6..15}).
const SHRINE_CLASS: [u8; 23] = [
    0, 4, 2, 3, 2, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 1, 1, 1, 1, 1, 1, 1,
];

/// A synthetic shrines.txt: the §2 classes; `LevelMin` 5 for id 8 and 4
/// for id 18 (the values the §5.1 vectors cite), 5 for id 9, 0 for every
/// other row, which makes ids 13, 19 and 22 (the vectors' accepted picks)
/// pass at level 2.
///
/// The §5.1 vector's pick 4 is id 9 (fixed in the spec, `objects.md`
/// Test vectors); id 9 gets `LevelMin` 5 so the draws (4, 8) give id 13.
fn shrines() -> Vec<Shrines> {
    SHRINE_CLASS
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let mut s = blank_shrine();
            s.effectclass = c;
            s.code = i as u8;
            s.levelmin = match i {
                8 | 9 => 5,
                18 => 4,
                _ => 0,
            };
            s
        })
        .collect()
}

const O: UnitId = UnitId(10);
const P: UnitId = UnitId(20);
const M: UnitId = UnitId(30);

const SHRINE3: u16 = 1; // init 1, Parm0 3
const SHRINE0: u16 = 2; // init 1, Parm0 0
const CHEST: u16 = 3; // init 3, lockable
const URN: u16 = 4; // init 2
const WELL: u16 = 5; // init 16
const DOOR: u16 = 6; // init 5
const TORCH: u16 = 7; // operate 11
const ANIM: u16 = 8; // FrameDelta0 256, Sync 0
const SPARK: u16 = 9; // init 57, lockable
const PREOP: u16 = 12; // PreOperate, Selectable0 1, Selectable2 0
const QUEST_INIT: u16 = 13; // init 4 (quest)
const TP: u16 = 59; // init 11
const PP: u16 = 60; // init 12

fn tables() -> ObjectTables {
    let mut objects: Vec<Objects> = (0..400).map(|_| blank_object()).collect();
    objects[SHRINE3 as usize].initfn = 1;
    objects[SHRINE3 as usize].parm0 = 3;
    objects[SHRINE0 as usize].initfn = 1;
    objects[CHEST as usize].initfn = 3;
    objects[CHEST as usize].lockable = 1;
    objects[URN as usize].initfn = 2;
    objects[WELL as usize].initfn = 16;
    objects[WELL as usize].parm2 = 0x101;
    objects[DOOR as usize].initfn = 5;
    objects[TORCH as usize].operatefn = 11;
    objects[ANIM as usize].framedelta0 = 256;
    objects[ANIM as usize].framecnt0 = 20 * 256;
    objects[ANIM as usize].start0 = 3;
    objects[SPARK as usize].initfn = 57;
    objects[SPARK as usize].lockable = 1;
    objects[PREOP as usize].preoperate = 1;
    objects[PREOP as usize].selectable0 = 1;
    objects[PREOP as usize].framedelta2 = 8;
    objects[PREOP as usize].sync = 1;
    objects[QUEST_INIT as usize].initfn = 4;
    for c in [TP, PP] {
        objects[c as usize].initfn = if c == TP { 11 } else { 12 };
        objects[c as usize].framecnt1 = 10 * 256;
        objects[c as usize].sync = 1;
        objects[c as usize].operatefn = 15;
        objects[c as usize].subclass = 4;
    }
    // Operable classes for §7 (operate 11 = torch: a mode change shows
    // that the function ran).
    for c in [22u16, 121, 122, 267, 136, 371] {
        objects[c as usize].operatefn = 11;
    }
    let mut levels: Vec<Levels> = (0..132).map(|_| blank_level()).collect();
    for (i, l) in levels.iter_mut().enumerate() {
        l.act = match i {
            0..=39 => 0,
            40..=74 => 1,
            75..=102 => 2,
            103..=108 => 3,
            _ => 4,
        };
    }
    levels[2].monlvl1 = 1;
    ObjectTables {
        objects,
        shrines: shrines(),
        levels,
        objgroup: Vec::new(),
        leveldefs: Vec::new(),
    }
}

/// A control with the control seed `c`.
fn control(t: &ObjectTables, c: Seed) -> ObjectControl {
    let (mut ctl, _) = ObjectControl::new(&mut Seed::init(), t);
    ctl.seed = c;
    ctl
}

/// A fake holding object `O` in `mode` on `level`, with unit seed {1, 666}.
fn fake(mode: u8, level: u32) -> Fake {
    let mut f = Fake::default();
    f.modes.insert(O, mode);
    f.levels.insert(O, level);
    f.seeds.insert(O, Seed::init());
    f.guids.insert(O, 0x1234);
    f
}

fn run_create(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    f: &mut Fake,
    class: u16,
    mode: u8,
) -> Result<Created, ObjectError> {
    create(ctl, t, f, O, class, 0x1234, Some(RoomId(1)), mode, 5, 6)
}

// ------------------------------------------------------------------ §2

// Covers: specs/world/objects.md §2 text, §2 r1, §2 r2, §2 r3, §2 r4, §2 r5
#[test]
fn control_build_seed_regions_and_shrine_lists() {
    let mut t = tables();
    t.levels.truncate(4);
    t.levels[1].act = 0;
    t.levels[2].act = 1;
    t.levels[3].act = 4;
    let mut game = Seed::new(7, 666);
    let (ctl, lo) = ObjectControl::new(&mut game, &t);
    let mut expect = Seed::new(7, 666);
    let lo2 = expect.step();
    assert_eq!(lo, lo2);
    assert_eq!(game, expect, "one game-seed step");
    assert_eq!(ctl.seed, Seed::init_low(lo2));
    assert_eq!(ctl.regions.len(), 4);
    assert_eq!(ctl.regions[0], None);
    for (id, act) in [(1, 0), (2, 1), (3, 4)] {
        assert_eq!(
            ctl.regions[id],
            Some(Region {
                act,
                counted: 0,
                w08: 0x7FFF_FFFF,
                health: 0,
                shrines: 0,
                wells: 0,
                w1c: -1,
                well_points: [(0, 0); 4],
                shrine_points: [(0, 0); 10],
            })
        );
    }
    let lists = ctl.shrine_lists.to_vec();
    assert_eq!(lists[0], vec![0]);
    assert_eq!(lists[1], (16..=22).collect::<Vec<u16>>());
    assert_eq!(lists[2], vec![2, 4]);
    assert_eq!(lists[3], vec![3, 5]);
    let mut c4 = vec![1];
    c4.extend(6..=15);
    assert_eq!(lists[4], c4);
    for l in &lists[5..] {
        assert!(l.is_empty());
    }
    // Class 8 or more: ignored.
    t.shrines[0].effectclass = 8;
    let (ctl, _) = ObjectControl::new(&mut Seed::init(), &t);
    assert!(ctl.shrine_lists[0].is_empty());
}

// ------------------------------------------------------------------ §3

// Covers: specs/world/objects.md §3 text, §3 r1, §3 r3, §3 r4, §3 r9
#[test]
fn create_resets_data_flags_and_timers() {
    let mut t = tables();
    t.objects[DOOR as usize].isattackable0 = 1;
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(0, 2);
    f.flags.insert(O, oflags::INIT_CLEARED | oflags::SELECTABLE);
    ctl.data.insert(
        O,
        ObjectData {
            interact: 9,
            spark: 1,
            operator: 4,
            ..ObjectData::default()
        },
    );
    let r = run_create(&mut ctl, &t, &mut f, DOOR, 0).unwrap();
    assert_eq!(
        r,
        Created {
            init: Route::Here,
            init_fn: 5
        }
    );
    let d = ctl.get(O).unwrap();
    assert_eq!(d.guid, 0x1234);
    assert_eq!(d.class, DOOR);
    assert_eq!((d.interact, d.spark, d.operator), (0, 0, 0));
    assert_eq!(d.owner, Some(-1));
    // 0x8 cleared, 0x4 set, 0x2 from Selectable0 = 0.
    assert_eq!(f.flags[&O], oflags::ATTACKABLE);
    assert_eq!(f.calls, vec![Call::CancelTimers(O)]);
    // IsAttackable0 = 0 clears the flag.
    t.objects[DOOR as usize].isattackable0 = 0;
    run_create(&mut ctl, &t, &mut f, DOOR, 0).unwrap();
    assert_eq!(f.flags[&O] & oflags::ATTACKABLE, 0);
}

// Covers: specs/world/objects.md §3 r2, §3 r5
#[test]
fn create_fatal_bounds() {
    let mut t = tables();
    t.objects.resize_with(574, blank_object);
    t.objects[100].initfn = 80;
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(0, 2);
    assert_eq!(
        run_create(&mut ctl, &t, &mut f, DOOR, 8),
        Err(ObjectError::Mode(8))
    );
    assert_eq!(
        run_create(&mut ctl, &t, &mut f, 573, 0),
        Err(ObjectError::Class(573))
    );
    assert_eq!(
        run_create(&mut ctl, &t, &mut f, 100, 0),
        Err(ObjectError::InitFn(80))
    );
    t.objects[100].initfn = 79;
    assert!(run_create(&mut ctl, &t, &mut f, 100, 0).is_ok());
}

// Covers: specs/world/objects.md §3 r6
#[test]
fn create_routes_inits_owned_elsewhere() {
    let mut t = tables();
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(0, 2);
    let r = run_create(&mut ctl, &t, &mut f, QUEST_INIT, 0).unwrap();
    assert_eq!(r.init, Route::Quest);
    assert_eq!(ctl.seed, Seed::init(), "a quest init draws nothing here");
    for (n, route) in [
        (17, Route::Waypoint),
        (8, Route::Here),
        (0, Route::Null),
        (35, Route::Null),
        (36, Route::Null),
        (40, Route::Null),
    ] {
        t.objects[QUEST_INIT as usize].initfn = n;
        let r = run_create(&mut ctl, &t, &mut f, QUEST_INIT, 0).unwrap();
        assert_eq!((r.init, r.init_fn), (route, n));
        assert_eq!(ctl.seed, Seed::init());
    }
}

// Covers: specs/world/objects.md §3 r7, §3 r8, §edge-cases-original-bugs r1
#[test]
fn preoperate_roll_and_selectable_from_m0() {
    let t = tables();
    // Vector: C = {1, 666}, roll(14) = 9: mode unchanged.
    let mut ctl = control(&t, Seed::new(1, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, PREOP, 0).unwrap();
    let mut c = Seed::new(1, 666);
    assert_eq!(c.roll(14), 9);
    assert_eq!(ctl.seed, c);
    assert_eq!(f.modes[&O], 0);
    assert_ne!(f.flags[&O] & oflags::SELECTABLE, 0);
    // C = {12, 666}: roll(14) = 0 → mode 2; the flag still follows mode 0
    // (Selectable2 = 0): edge case 1.
    let mut ctl = control(&t, Seed::new(12, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, PREOP, 0).unwrap();
    assert_eq!(f.modes[&O], 2);
    assert!(f.calls.contains(&Call::Mode(O, 2, true)));
    assert_ne!(f.flags[&O] & oflags::SELECTABLE, 0);
    // Flag 0x80 blocks the roll (no draw).
    let mut ctl = control(&t, Seed::new(12, 666));
    let mut f = fake(0, 2);
    f.flags.insert(O, oflags::KEEP_MODE);
    run_create(&mut ctl, &t, &mut f, PREOP, 0).unwrap();
    assert_eq!(ctl.seed, Seed::new(12, 666));
    assert_eq!(f.modes[&O], 0);
    // Allocated in mode 2: Selectable2 = 0 decides.
    let mut ctl = control(&t, Seed::new(1, 666));
    let mut f = fake(2, 2);
    run_create(&mut ctl, &t, &mut f, PREOP, 2).unwrap();
    assert_eq!(f.flags[&O] & oflags::SELECTABLE, 0);
}

// Covers: specs/world/objects.md §3 r7, §edge-cases-original-bugs r1, §5.5
#[test]
fn selectable_uses_mode_before_init() {
    let mut t = tables();
    t.objects[PP as usize].selectable0 = 1;
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, PP, 0).unwrap();
    assert_eq!(f.modes[&O], 1, "init 12 opened the portal");
    assert_ne!(f.flags[&O] & oflags::SELECTABLE, 0);
}

// ------------------------------------------------------------------ §4

// Covers: specs/world/objects.md §4 text, §4 r1, §4 r2, §4 r3
#[test]
fn anim_vector_and_sync() {
    let mut t = tables();
    // From mode 1 (a set to the current mode runs no setup, §4 rule 5).
    let mut f = fake(1, 2);
    f.seeds.insert(O, Seed::new(12345, 666));
    set_mode(&t, &mut f, O, ANIM, 0, true).unwrap();
    let mut u = Seed::new(12345, 666);
    assert_eq!(u.roll(32), 23);
    assert_eq!(f.seeds[&O], u);
    // speed = 23 + 256 − 16 = 263.
    assert_eq!(
        f.calls,
        vec![
            Call::Mode(O, 0, true),
            Call::Anim(O, 20 * 256, 3 * 256, 263)
        ]
    );
    // Sync ≠ 0: speed = d, no draw.
    t.objects[ANIM as usize].sync = 1;
    let mut f = fake(1, 2);
    set_mode(&t, &mut f, O, ANIM, 0, false).unwrap();
    assert_eq!(f.seeds[&O], Seed::init());
    assert_eq!(f.calls[1], Call::Anim(O, 20 * 256, 768, 256));
    // Negative delta: roll(−2) draws nothing; −16 + 1 ≤ 0 → 0.
    t.objects[ANIM as usize].sync = 0;
    t.objects[ANIM as usize].framedelta1 = 0xFFF0;
    let mut f = fake(0, 2);
    set_mode(&t, &mut f, O, ANIM, 1, true).unwrap();
    assert_eq!(f.seeds[&O], Seed::init());
    assert_eq!(f.calls[1], Call::Anim(O, 0, 0, 0));
    // §4 rule 5 / `objects-2.md` §24 rule 1: the same mode only writes
    // (queue, flag 0x1): no animation setup, no draw.
    let mut f = fake(0, 2);
    f.seeds.insert(O, Seed::new(12345, 666));
    set_mode(&t, &mut f, O, ANIM, 0, true).unwrap();
    assert_eq!(f.calls, vec![Call::Mode(O, 0, true)]);
    assert_eq!(f.seeds[&O], Seed::new(12345, 666));
    // Mode ≥ 8: fatal.
    assert_eq!(
        set_mode(&t, &mut f, O, ANIM, 8, true),
        Err(ObjectError::Mode(8))
    );
}

// ------------------------------------------------------------------ §5

// Covers: specs/world/objects.md §5 text, §5.1 text, §5.1 r1, §5.1 r3, §5.1 r5
#[test]
fn shrine_init_class_pick_vectors() {
    let t = tables();
    // Parm0 3, level 2, C = {1, 666}: roll(10) = 1 → class 4; picks 4 (id
    // 8, LevelMin 5 > 2), 8 (id 13): id 13.
    let mut ctl = control(&t, Seed::new(1, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, SHRINE3, 0).unwrap();
    let d = ctl.get(O).unwrap();
    assert_eq!((d.interact, d.shrine), (13, Some(13)));
    let mut c = Seed::new(1, 666);
    assert_eq!((c.roll(10), c.roll(11), c.roll(11)), (1, 4, 8));
    assert_eq!(ctl.seed, c, "three draws");
    // C = {4, 666}: roll(10) = 0 → class 1; picks 2 (id 18, LevelMin 4),
    // 3 (id 19): id 19.
    let mut ctl = control(&t, Seed::new(4, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, SHRINE3, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 19);
    let mut c = Seed::new(4, 666);
    assert_eq!((c.roll(10), c.roll(7), c.roll(7)), (0, 2, 3));
    assert_eq!(ctl.seed, c);
}

// Covers: specs/world/objects.md §5.1 r2
#[test]
fn shrine_init_any_row_vector() {
    let t = tables();
    // Parm0 0, level 2, C = {1, 666}: roll(22) = 21 → id 22.
    let mut ctl = control(&t, Seed::new(1, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, SHRINE0, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().shrine, Some(22));
    let mut c = Seed::new(1, 666);
    assert_eq!(c.roll(22), 21);
    assert_eq!(ctl.seed, c);
}

// Covers: specs/world/objects.md §3 r6, §5.1 r3
#[test]
fn init_reads_the_level_of_the_init_room() {
    // §3 runs before the unit joins the world: the unit has no level of
    // its own, the init record's room has level 2. Shrine class 2 ({2, 4},
    // LevelMin 1 and 2 here) passes on the first try: one draw, not eight.
    let mut t = tables();
    t.objects[SHRINE3 as usize].parm0 = 1;
    t.shrines[2].levelmin = 1;
    t.shrines[4].levelmin = 2;
    let mut ctl = control(&t, Seed::new(1542758918, 666));
    let mut f = fake(0, 2);
    f.levels.clear();
    f.room_levels.insert(RoomId(1), 2);
    let mut c = ctl.seed;
    run_create(&mut ctl, &t, &mut f, SHRINE3, 0).unwrap();
    c.roll(2);
    assert_eq!(ctl.seed, c, "one shrine draw");
    assert_eq!(ctl.get(O).unwrap().interact, 2);
}

// Covers: specs/world/objects.md §5.1 r1, §5.1 r3, §5.1 r4, §edge-cases-original-bugs r4
#[test]
fn shrine_pick_rules_and_remap() {
    let mut t = tables();
    // Parm0 1 → class 2 ({2, 4}); 2 → class 3 ({3, 5}): 4 → 2, 5 → 3.
    for (parm0, ok) in [(1, 2u8), (2, 3)] {
        t.objects[SHRINE3 as usize].parm0 = parm0;
        let mut seen = BTreeSet::new();
        for lo in 1..64 {
            let mut ctl = control(&t, Seed::new(lo, 666));
            let mut f = fake(0, 2);
            run_create(&mut ctl, &t, &mut f, SHRINE3, 0).unwrap();
            seen.insert(ctl.get(O).unwrap().interact);
        }
        assert_eq!(seen, BTreeSet::from([ok]));
    }
    // Class 0 or > 4 → 2; an empty list is fatal.
    let mut ctl = control(&t, Seed::init());
    for class in [0, 5, 7] {
        let id = shrine_pick(&mut ctl, &t, class, 2).unwrap();
        assert!(id == 2 || id == 4);
    }
    ctl.shrine_lists[3].clear();
    assert_eq!(
        shrine_pick(&mut ctl, &t, 3, 2),
        Err(ObjectError::EmptyShrineList(3))
    );
    // Id 0 in a list → 1; after 8 failures the last id stays.
    ctl.shrine_lists[1] = vec![0];
    t.shrines[1].levelmin = 50;
    let before = ctl.seed;
    assert_eq!(shrine_pick(&mut ctl, &t, 1, 2), Ok(1));
    let mut c = before;
    for _ in 0..8 {
        c.roll(1);
    }
    assert_eq!(ctl.seed, c, "eight tries");
    // Class 1 never yields 16 (→ 18): Enirhs never come from init.
    let t = tables();
    let mut seen = BTreeSet::new();
    for lo in 1..200 {
        let mut ctl = control(&t, Seed::new(lo, 666));
        let mut f = fake(0, 30);
        run_create(&mut ctl, &t, &mut f, SHRINE3, 0).unwrap();
        seen.insert(ctl.get(O).unwrap().interact);
    }
    for never in [4, 5, 16] {
        assert!(!seen.contains(&never), "{never} from init");
    }
}

// Covers: specs/world/objects.md §5.2
#[test]
fn chest_init_vectors() {
    let t = tables();
    // C = {1, 666}: 51 (not trapped), 31 (not locked); U := init_low(55249).
    let mut ctl = control(&t, Seed::new(1, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, CHEST, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 0);
    assert_eq!(f.seeds[&O], Seed::init_low(55249));
    assert_eq!(ctl.seed, Seed::new(671_516_612, 330_169_957));
    // C = {410, 666}: 0 (trapped), trap 8, 0 (locked): 0x88; U :=
    // init_low(48426).
    let mut ctl = control(&t, Seed::new(410, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, CHEST, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 0x88);
    assert_eq!(f.seeds[&O], Seed::init_low(48426));
}

// Covers: specs/world/objects.md §5.2, §edge-cases-original-bugs r12
#[test]
fn urn_and_sparkling_init() {
    let mut t = tables();
    // Urn: same draws as the chest's first two.
    let mut ctl = control(&t, Seed::new(410, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, URN, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 8);
    let mut c = Seed::new(410, 666);
    c.roll(100);
    c.roll(8);
    assert_eq!(ctl.seed, c);
    assert_eq!(f.seeds[&O], Seed::init(), "init 2 leaves U");
    // Init 57: init 3, then spark 1.
    let mut ctl = control(&t, Seed::new(410, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, SPARK, 0).unwrap();
    let d = ctl.get(O).unwrap();
    assert_eq!((d.interact, d.spark), (0x88, 1));
    assert_eq!(f.seeds[&O], Seed::init_low(48426));
    // Not lockable: bit 0x80 cleared, no lock draw.
    t.objects[CHEST as usize].lockable = 0;
    let mut ctl = control(&t, Seed::new(410, 666));
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, CHEST, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 8);
    // Trap 9 never comes from init.
    for lo in 1..300 {
        let mut ctl = control(&t, Seed::new(lo, 666));
        let mut f = fake(0, 2);
        run_create(&mut ctl, &t, &mut f, URN, 0).unwrap();
        assert!(ctl.get(O).unwrap().interact <= 8);
    }
}

// Covers: specs/world/objects.md §5.3
#[test]
fn well_init_charges() {
    let mut t = tables();
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(0, 2);
    // Parm2 0x101 read as u8 1 → 2 charges.
    run_create(&mut ctl, &t, &mut f, WELL, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 2);
    t.objects[WELL as usize].parm2 = 3;
    run_create(&mut ctl, &t, &mut f, WELL, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 6);
    assert_eq!(ctl.seed, Seed::init());
}

// Covers: specs/world/objects.md §5.4
#[test]
fn door_init_returns_at_once() {
    let t = tables();
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(6, 2);
    run_create(&mut ctl, &t, &mut f, DOOR, 6).unwrap();
    assert_eq!(f.modes[&O], 6);
    assert_eq!(f.calls, vec![Call::CancelTimers(O)]);
    assert_eq!(ctl.seed, Seed::init());
}

// Covers: specs/world/objects.md §5.5
#[test]
fn town_portal_init() {
    let t = tables();
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(1, 80);
    f.frame = 100;
    run_create(&mut ctl, &t, &mut f, TP, 1).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 75, "act III town");
    assert_eq!(
        f.calls,
        vec![
            Call::CancelTimers(O),
            Call::Stamp(O),
            Call::Schedule(O, oevent::END_ANIM, 111)
        ]
    );
    // Mode 0: no stamp, no ENDANIM.
    let mut f = fake(0, 120);
    run_create(&mut ctl, &t, &mut f, TP, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 109);
    assert_eq!(f.calls, vec![Call::CancelTimers(O)]);
}

// Covers: specs/world/objects.md §5.5
#[test]
fn permanent_portal_init() {
    let t = tables();
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(0, 1);
    f.frame = 50;
    run_create(&mut ctl, &t, &mut f, PP, 0).unwrap();
    assert_eq!(ctl.get(O).unwrap().interact, 39);
    assert_eq!(
        f.calls,
        vec![
            Call::CancelTimers(O),
            Call::Mode(O, 1, true),
            Call::Anim(O, 10 * 256, 0, 0),
            Call::Stamp(O),
            Call::Schedule(O, oevent::END_ANIM, 61)
        ]
    );
    let pairs = [
        (39, 1),
        (38, 4),
        (4, 38),
        (74, 46),
        (46, 74),
        (73, 66),
        (66, 73),
        (121, 109),
        (109, 121),
        (125, 111),
        (126, 112),
        (127, 117),
        (5, 0),
    ];
    for (level, dest) in pairs {
        let mut f = fake(1, level);
        f.staff_tomb = 66;
        run_create(&mut ctl, &t, &mut f, PP, 1).unwrap();
        assert_eq!(ctl.get(O).unwrap().interact, dest, "level {level}");
        assert_eq!(f.calls, vec![Call::CancelTimers(O)], "mode 1: no setup");
    }
    for (level, dest) in [(111, 125), (112, 126), (117, 127)] {
        let mut f = fake(1, level);
        f.frame = 7;
        run_create(&mut ctl, &t, &mut f, PP, 1).unwrap();
        assert_eq!(ctl.get(O).unwrap().interact, dest);
        assert_eq!(f.schedules(), vec![(O, oevent::DELAYED_PORTAL, 8)]);
    }
}

// ------------------------------------------------------------------ §6

fn preset_ids(class: u32) -> BTreeSet<u8> {
    let t = tables();
    let mut seen = BTreeSet::new();
    for lo in 1..200 {
        let mut ctl = control(&t, Seed::init());
        let mut f = Fake {
            next_alloc: Some(100),
            ..Fake::default()
        };
        let u = UnitId(100);
        f.seeds.insert(u, Seed::new(lo, 666));
        ctl.data.insert(
            u,
            ObjectData {
                class: 136,
                ..ObjectData::default()
            },
        );
        let r = create_preset(&mut ctl, &t, &mut f, RoomId(1), 2, class, 3, 4, 0).unwrap();
        assert_eq!(r, Preset::Object(Some(u)));
        assert_eq!(f.calls[0], Call::Allocate(RoomId(1), 136, 3, 4, 0));
        let d = ctl.get(u).unwrap();
        assert_eq!(d.shrine, Some(u16::from(d.interact)));
        seen.insert(d.interact);
        assert_eq!(ctl.seed, Seed::init(), "drawn on U, not C");
    }
    seen
}

// Covers: specs/world/objects.md §6, §edge-cases-original-bugs r4
#[test]
fn preset_shrines() {
    assert_eq!(preset_ids(576), BTreeSet::from([8, 9, 10]));
    assert_eq!(preset_ids(574), BTreeSet::from([2, 3]));
    assert_eq!(preset_ids(578), BTreeSet::from([1, 2, 3]));
    assert_eq!(preset_ids(575), BTreeSet::from([7]));
    assert_eq!(preset_ids(579), BTreeSet::from([14]));
}

// Covers: specs/world/objects.md §6, §edge-cases-original-bugs r22; specs/world/objects-2.md §24 r1, §22 r1
#[test]
fn preset_bounds_and_580() {
    let t = tables();
    let mut ctl = control(&t, Seed::init());
    let mut f = Fake::default();
    for class in [0, 573, 583, 600] {
        assert_eq!(
            create_preset(&mut ctl, &t, &mut f, RoomId(1), 2, class, 0, 0, 0),
            Ok(Preset::None)
        );
    }
    assert_eq!(
        create_preset(&mut ctl, &t, &mut f, RoomId(1), 2, 576, 0, 0, 0),
        Ok(Preset::Object(None))
    );
    // 582's class is the quest spec's (`0x0059D830`): the host's.
    assert_eq!(
        create_preset(&mut ctl, &t, &mut f, RoomId(1), 2, 582, 0, 0, 0),
        Ok(Preset::NotCovered(582))
    );
    // 580 and 581 draw the chest class first; no allocation → none.
    for class in [580, 581] {
        assert_eq!(
            create_preset(&mut ctl, &t, &mut f, RoomId(1), 2, class, 0, 0, 0),
            Ok(Preset::Object(None))
        );
    }
    let mut ctl = control(&t, Seed::init());
    // 580 on level 25: class 371, spark, InteractType 3, flag 0x80, mode 0.
    let mut f = Fake {
        next_alloc: Some(100),
        ..Fake::default()
    };
    let u = UnitId(100);
    ctl.data.insert(
        u,
        ObjectData {
            class: 371,
            ..ObjectData::default()
        },
    );
    let r = create_preset(&mut ctl, &t, &mut f, RoomId(1), 25, 580, 3, 4, 1).unwrap();
    assert_eq!(r, Preset::Object(Some(u)));
    // The preset mode (1) is ignored: allocated in mode 0 (§6, edge 22).
    assert_eq!(f.calls[0], Call::Allocate(RoomId(1), 371, 3, 4, 0));
    let d = ctl.get(u).unwrap();
    assert_eq!((d.spark, d.interact), (1, 3));
    assert_ne!(f.flags[&u] & oflags::KEEP_MODE, 0);
    assert_eq!(f.modes[&u], 0);
    // 581's class draw happens at level 25 too (one control-seed step).
    let mut want = Seed::init();
    want.step();
    assert_eq!(ctl.seed, want);
    // objects-2 §24 r1: the final mode set (mode 0 → 0) runs no setup and
    // draws nothing, but queues the object and sets flag 0x1.
    assert!(f.calls.contains(&Call::Mode(u, 0, true)));
    assert_ne!(f.flags[&u] & oflags::CHANGED, 0);
}

/// The class 581 allocates at `level` with a control seed whose next
/// step gives `lo'`, and the control seed after it.
fn preset_chest_class(level: u32, lo: u32) -> (u16, Seed) {
    let t = tables();
    let mut ctl = control(&t, Seed::new(0, lo));
    let mut f = Fake {
        next_alloc: Some(100),
        ..Fake::default()
    };
    let u = UnitId(100);
    ctl.data.insert(u, ObjectData::default());
    let r = create_preset(&mut ctl, &t, &mut f, RoomId(1), level, 581, 3, 4, 1).unwrap();
    assert_eq!(r, Preset::Object(Some(u)));
    let Call::Allocate(room, class, x, y, mode) = f.calls[0].clone() else {
        panic!("{:?}", f.calls);
    };
    // The preset mode (1) is ignored: mode 0 (§6).
    assert_eq!((room, x, y, mode), (RoomId(1), 3, 4, 0));
    (class, ctl.seed)
}

// Covers: specs/world/objects.md §6
#[test]
fn preset_chest_581_picks_by_act_on_one_control_step() {
    // Act III: level 83 {329, 330, 331, 332}[lo' & 3], else {181, 183}[lo' & 1]
    // (the Durance of Hate chests of `a3-warp-durance-ama`).
    assert_eq!(preset_chest_class(100, 6).0, 181);
    assert_eq!(preset_chest_class(100, 7).0, 183);
    assert_eq!(preset_chest_class(83, 6).0, 331);
    assert_eq!(preset_chest_class(83, 3).0, 332);
    // Act II: level 74 {387, 389, 390, 391}, else {87, 88}.
    assert_eq!(preset_chest_class(74, 1).0, 389);
    assert_eq!(preset_chest_class(50, 9).0, 88);
    // Acts I, IV, V: lo' mod 13.
    assert_eq!(preset_chest_class(2, 13 + 4).0, 141);
    assert_eq!(preset_chest_class(110, 12).0, 243);
    assert_eq!(preset_chest_class(105, 0).0, 5);
    // One step of the control seed.
    let mut want = Seed::new(0, 6);
    want.step();
    assert_eq!(preset_chest_class(100, 6).1, want);
}

// ------------------------------------------------------------------ §7

/// A fake with operable object `O` (GUID 0x1234) of `class`, player `P`
/// (class 0) and monster `M`.
fn operate_fake(ctl: &mut ObjectControl, class: u16) -> Fake {
    let mut f = fake(0, 2);
    f.operators.insert(P, Operator::Player(0));
    f.operators.insert(M, Operator::Monster);
    ctl.data.insert(
        O,
        ObjectData {
            guid: 0x1234,
            class,
            ..ObjectData::default()
        },
    );
    f
}

// Covers: specs/world/objects.md §7.1 text, §7.1 r1, §7.1 r2, §7.1 r3, §7.1 r4
#[test]
fn operate_entry_checks() {
    let mut t = tables();
    let mut ctl = control(&t, Seed::init());
    let mut f = operate_fake(&mut ctl, TORCH);
    // No such unit → 0.
    assert_eq!(
        operate_in_range(&mut ctl, &t, &mut f, Some(P), 0x9999),
        Ok((0, None))
    );
    // Monster and MonsterOK = 0 → 1, nothing.
    assert_eq!(
        operate_in_range(&mut ctl, &t, &mut f, Some(M), 0x1234),
        Ok((1, None))
    );
    assert_eq!(f.modes[&O], 0);
    // Out of range → 1, nothing.
    f.out_of_range.insert(P);
    assert_eq!(
        operate_in_range(&mut ctl, &t, &mut f, Some(P), 0x1234),
        Ok((1, None))
    );
    assert_eq!(f.modes[&O], 0);
    // In range → the dispatch runs; result 1.
    f.out_of_range.clear();
    assert_eq!(
        operate_in_range(&mut ctl, &t, &mut f, Some(P), 0x1234),
        Ok((1, Some(Dispatch::Done(1))))
    );
    assert_eq!(f.modes[&O], 1);
    // A monster passes with MonsterOK; it skips the player tests.
    t.objects[TORCH as usize].monsterok = 1;
    f.busy.insert(M);
    assert_eq!(
        operate_in_range(&mut ctl, &t, &mut f, Some(M), 0x1234),
        Ok((1, Some(Dispatch::Done(1))))
    );
    assert_eq!(f.modes[&O], 0);
    // No operator: no range test.
    assert_eq!(
        operate_in_range(&mut ctl, &t, &mut f, None, 0x1234),
        Ok((1, Some(Dispatch::Done(1))))
    );
}

// Covers: specs/world/objects.md §7.2 text, §7.2 r1, §7.2 r2
#[test]
fn dispatch_player_refusals() {
    let t = tables();
    let mut ctl = control(&t, Seed::init());
    for which in 0..3 {
        let mut f = operate_fake(&mut ctl, TORCH);
        match which {
            0 => f.interact_active.insert(P),
            1 => f.busy.insert(P),
            _ => f.cursor.insert(P),
        };
        assert_eq!(
            dispatch(&mut ctl, &t, &mut f, O, Some(P)),
            Ok(Dispatch::Done(0))
        );
        assert!(f.calls.is_empty(), "nothing else");
    }
    // Cursor item with the stash class 267: allowed.
    let mut f = operate_fake(&mut ctl, STASH_CLASS);
    f.cursor.insert(P);
    assert_eq!(
        dispatch(&mut ctl, &t, &mut f, O, Some(P)),
        Ok(Dispatch::Done(1))
    );
    assert_eq!(f.modes[&O], 1);
}

// Covers: specs/world/objects.md §7.2 r3, §7.2 r4
#[test]
fn dispatch_table_rules() {
    let mut t = tables();
    let mut ctl = control(&t, Seed::init());
    for class in REFUSED_CLASSES {
        let mut f = operate_fake(&mut ctl, class);
        assert_eq!(
            dispatch(&mut ctl, &t, &mut f, O, None),
            Ok(Dispatch::Done(0))
        );
        assert!(f.calls.is_empty());
    }
    let op = |n| Operate {
        object: O,
        operator: Some(P),
        class: 100,
        operate_fn: n,
    };
    for (n, want) in [
        (0, Dispatch::Done(0)),
        (35, Dispatch::Done(0)),
        (60, Dispatch::Done(0)),
        (74, Dispatch::Done(0)),
        (100, Dispatch::Done(0)),
        (23, Dispatch::Waypoint(op(23))),
        (6, Dispatch::Quest(op(6))),
        (13, Dispatch::Done(0)),
        (11, Dispatch::Done(1)),
    ] {
        t.objects[100].operatefn = n;
        let mut f = operate_fake(&mut ctl, 100);
        assert_eq!(
            dispatch(&mut ctl, &t, &mut f, O, Some(P)),
            Ok(want),
            "fn {n}"
        );
    }
    t.objects[100].operatefn = 101;
    let mut f = operate_fake(&mut ctl, 100);
    assert_eq!(
        dispatch(&mut ctl, &t, &mut f, O, Some(P)),
        Err(ObjectError::OperateFn(101))
    );
    // The fatal check comes before the refused classes.
    t.objects[22].operatefn = 200;
    let mut f = operate_fake(&mut ctl, 22);
    assert_eq!(
        dispatch(&mut ctl, &t, &mut f, O, None),
        Err(ObjectError::OperateFn(200))
    );
}

// Covers: specs/world/objects.md §7.2 r3
#[test]
fn null_operate_slots_return_zero_and_do_nothing() {
    // Steeg stone, guild vault, trophy case, message board (35–38) and 60.
    for n in [35u8, 36, 37, 38, 60] {
        let mut t = tables();
        t.objects[TORCH as usize].operatefn = n;
        let mut ctl = control(&t, Seed::init());
        let mut f = operate_fake(&mut ctl, TORCH);
        assert_eq!(
            operate_in_range(&mut ctl, &t, &mut f, Some(P), 0x1234),
            Ok((1, Some(Dispatch::Done(0)))),
            "operate {n}"
        );
        assert_eq!(f.modes[&O], 0, "operate {n}");
        assert_eq!(ctl.seed, Seed::init(), "operate {n}");
    }
}

// ------------------------------------------------------------------ M05

/// Every init/operate row of `object-functions.tsv` against
/// [`init_route`] / [`operate_route`]; returns the rows that disagree.
fn route_mismatches(text: &str) -> Result<Vec<(String, u32)>, TsvError> {
    let tn = "object-functions.tsv";
    let mut out = Vec::new();
    for (line, c) in tsv_rows(tn, text, HEADER)? {
        let kind = c[0];
        if kind != "init" && kind != "operate" {
            continue;
        }
        let index = tsv_num(tn, line, "index", c[1])?;
        let address = tsv_num(tn, line, "address", c[2])?;
        let want = match (c[5], address) {
            // Null slots (address 0): owner `-`, or the section that
            // names the null entry (e44dcaa1).
            (o, 0) if o == "-" || o.starts_with('§') => Some(Route::Null),
            (o, a) if o.starts_with("world/quests") && a != 0 => Some(Route::Quest),
            ("world/waypoints.md", a) if a != 0 => Some(Route::Waypoint),
            ("todo", a) if a != 0 => Some(Route::NotCovered),
            (o, a) if o.starts_with('§') && a != 0 => Some(Route::Here),
            _ => None,
        };
        let n = u8::try_from(index).unwrap_or(u8::MAX);
        let got = if kind == "init" {
            init_route(n)
        } else {
            operate_route(n)
        };
        if want != Some(got) {
            out.push((kind.to_string(), index));
        }
    }
    Ok(out)
}

// Covers: specs/world/objects.md §3 r6, §7.2 r3, §15
#[test]
fn routes_match_function_table() {
    assert_eq!(route_mismatches(FUNCS_TSV), Ok(vec![]));
    // Rows the table does not list: entry 0, operate 74–100, the bounds.
    assert_eq!(init_route(0), Route::Null);
    assert_eq!(operate_route(0), Route::Null);
    for n in 74..=100 {
        assert_eq!(operate_route(n), Route::Null);
    }
    assert_eq!(init_route(INIT_FN_BOUND), Route::Null);
    assert_eq!(operate_route(OPERATE_FN_BOUND), Route::Null);
}

// Covers: specs/world/objects.md §3 r6, §7.2 r3
#[test]
fn route_check_catches_perturbations() {
    let door = "operate\t8\t0x00581D40\tDoor\t23\t§10";
    assert!(FUNCS_TSV.contains(door));
    let text = FUNCS_TSV.replace(door, "operate\t8\t0x00581D40\tDoor\t23\ttodo");
    assert_eq!(route_mismatches(&text), Ok(vec![("operate".into(), 8)]));
    let wp = "init\t17\t0x00547210";
    assert!(FUNCS_TSV.contains(wp));
    let text = FUNCS_TSV.replace(wp, "init\t17\t0");
    assert_eq!(route_mismatches(&text), Ok(vec![("init".into(), 17)]));
    let null = "init\t35\t0\t-\t1\t§3";
    assert!(FUNCS_TSV.contains(null));
    let text = FUNCS_TSV.replace(null, "init\t35\t0\t-\t1\tworld/quests.md");
    assert_eq!(route_mismatches(&text), Ok(vec![("init".into(), 35)]));
}

// ------------------------------------------------------------------ events

// Covers: specs/sim/units.md §6.4
#[test]
fn end_anim_and_delayed_portal_events() {
    let mut t = tables();
    t.objects[TORCH as usize].mode2 = 1;
    let mut ctl = control(&t, Seed::init());
    let mut f = operate_fake(&mut ctl, TORCH);
    f.modes.insert(O, 1);
    assert_eq!(
        object_event(&mut ctl, &t, &mut f, O, oevent::END_ANIM),
        Ok(EventRun::Done)
    );
    assert_eq!(f.modes[&O], 2);
    // `objects-2.md` §18.6: a direct write (no mode set, no queue).
    assert_eq!(
        f.calls[0],
        Call::Other(format!("store {} 2", O.0)),
        "no update queued"
    );
    assert_eq!(f.calls.last(), Some(&Call::Free(O)));
    // HasCollision2 ≠ 0: footprint kept; not mode 1: nothing.
    t.objects[TORCH as usize].hascollision2 = 1;
    f.modes.insert(O, 1);
    f.calls.clear();
    object_event(&mut ctl, &t, &mut f, O, oevent::END_ANIM).unwrap();
    assert!(!f.calls.contains(&Call::Free(O)));
    f.calls.clear();
    object_event(&mut ctl, &t, &mut f, O, oevent::END_ANIM).unwrap();
    assert!(f.calls.is_empty());
    // Mode2 = 0: stays in mode 1.
    t.objects[TORCH as usize].mode2 = 0;
    f.modes.insert(O, 1);
    object_event(&mut ctl, &t, &mut f, O, oevent::END_ANIM).unwrap();
    assert_eq!(f.modes[&O], 1);
    // Event 11: portal row by level.
    for (level, row) in [(111, 125), (112, 126), (117, 127), (5, 127)] {
        f.levels.insert(O, level);
        f.calls.clear();
        object_event(&mut ctl, &t, &mut f, O, oevent::DELAYED_PORTAL).unwrap();
        assert_eq!(
            f.calls,
            vec![Call::Other(format!("level_portal {O:?} {row}"))]
        );
    }
    assert_eq!(
        object_event(&mut ctl, &t, &mut f, O, oevent::QUEST),
        Ok(EventRun::Quest)
    );
    // Events 0, 3, 8, 9, 10 run `objects-2.md` §18 (`mech` tests); an
    // event type with no handler is handed back.
    for e in [0, 3, 8, 9, 10] {
        assert_eq!(object_event(&mut ctl, &t, &mut f, O, e), Ok(EventRun::Done));
    }
    assert_eq!(
        object_event(&mut ctl, &t, &mut f, O, 13),
        Ok(EventRun::NotCovered(13))
    );
}

// ------------------------------------------------------------------ §14

// Covers: specs/world/objects.md §14 text, §14 r1; specs/world/objects-2.md §23 r1, §23 r3
#[test]
fn update_message_bytes() {
    assert_eq!(
        state_message(0x0403_0201, true, 2),
        [0x0E, 2, 1, 2, 3, 4, 3, 1, 2, 0, 0, 0]
    );
    assert_eq!(
        shrine_message(0x0403_0201, 0x0807_0605, 15),
        [0x4D, 2, 1, 2, 3, 4, 5, 6, 7, 8, 15, 0, 0, 0, 0, 0, 0]
    );
    let mut t = tables();
    t.objects[SHRINE3 as usize].subclass = 1;
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(1, 2);
    ctl.data.insert(
        O,
        ObjectData {
            guid: 0x1234,
            class: SHRINE3,
            shrine: Some(7),
            operator: 0x51,
            ..ObjectData::default()
        },
    );
    // Flag 0x1 clear: nothing.
    assert_eq!(update_messages(&ctl, &t, &f, O), Ok(vec![]));
    f.flags.insert(O, oflags::CHANGED | oflags::SELECTABLE);
    assert_eq!(
        update_messages(&ctl, &t, &f, O),
        Ok(vec![
            UpdateMessage::State(state_message(0x1234, true, 1)),
            UpdateMessage::Shrine(shrine_message(0x1234, 0x50, 7)),
        ])
    );
    // Mode 0 or no operator: no 0x4D.
    f.modes.insert(O, 0);
    assert_eq!(update_messages(&ctl, &t, &f, O).unwrap().len(), 1);
    // Subclass bit 2: the portal message instead (flags, destination,
    // GUID).
    assert_eq!(
        portal_message(&ObjectData {
            guid: 0x0403_0201,
            portal_flags: 3,
            interact: 40,
            ..ObjectData::default()
        }),
        [0x60, 3, 40, 1, 2, 3, 4]
    );
    f.modes.insert(O, 1);
    t.objects[SHRINE3 as usize].subclass = 5;
    assert_eq!(
        update_messages(&ctl, &t, &f, O),
        Ok(vec![
            UpdateMessage::State(state_message(0x1234, true, 1)),
            UpdateMessage::Portal(portal_message(ctl.get(O).unwrap())),
        ])
    );
}

// Covers: specs/world/objects.md §1
#[test]
fn object_data_fields_and_flags() {
    // The §1 flag bits.
    assert_eq!(
        [
            oflags::CHANGED,
            oflags::SELECTABLE,
            oflags::ATTACKABLE,
            oflags::INIT_CLEARED,
            oflags::KEEP_MODE,
            oflags::HOVER_FREED,
            oflags::SOUND_QUEUED,
        ],
        [0x1, 0x2, 0x4, 0x8, 0x80, 0x100, 0x400]
    );
    // The data is zeroed at allocation: stale fields of a reused unit are
    // gone; only the GUID, the class and the timer owner −1 are set by
    // creation of a class without an init function.
    let t = tables();
    let mut ctl = control(&t, Seed::init());
    ctl.data.insert(
        O,
        ObjectData {
            guid: 7,
            class: 1,
            interact: 0x88,
            portal_flags: 3,
            shrine: Some(3),
            operator: 21,
            spark: 2,
            owner: Some(5),
            drop_code: 0x2020_6B6B,
            last_tick: 99,
        },
    );
    let mut f = fake(0, 2);
    run_create(&mut ctl, &t, &mut f, ANIM, 0).unwrap();
    assert_eq!(
        ctl.get(O).unwrap(),
        &ObjectData {
            guid: 0x1234,
            class: ANIM,
            owner: Some(-1),
            ..ObjectData::default()
        }
    );
    // `InteractType` of a chest: trap type in bits 0–6, locked in bit 7.
    let mut ctl = control(&t, Seed::new(410, 666));
    run_create(&mut ctl, &t, &mut f, CHEST, 0).unwrap();
    let it = ctl.get(O).unwrap().interact;
    assert_eq!((it & 0x7F, it & 0x80), (8, 0x80));
}

// Covers: specs/world/objects.md §edge-cases-original-bugs text, §edge-cases-original-bugs r2
#[test]
fn chest_and_urn_read_classic_normal_monlvl() {
    // Only `MonLvl1` feeds the trap and lock thresholds: the Nightmare,
    // Hell and expansion columns change nothing.
    let base = tables();
    let mut other = tables();
    for l in &mut other.levels {
        (l.monlvl2, l.monlvl3) = (200, 200);
        (l.monlvl1ex, l.monlvl2ex, l.monlvl3ex) = (200, 200, 200);
    }
    let mut high = tables();
    high.levels[2].monlvl1 = 200;
    let run = |t: &ObjectTables, class: u16, lo: u32| {
        let mut ctl = control(t, Seed::new(lo, 666));
        let mut f = fake(0, 2);
        run_create(&mut ctl, t, &mut f, class, 0).unwrap();
        (ctl.get(O).unwrap().interact, ctl.seed)
    };
    let mut differs = false;
    for lo in 1..200 {
        for class in [URN, CHEST] {
            assert_eq!(run(&base, class, lo), run(&other, class, lo), "lo {lo}");
            differs |= run(&base, class, lo) != run(&high, class, lo);
        }
    }
    assert!(differs, "MonLvl1 itself is read");
}

// Covers: specs/world/objects.md §7.1
#[test]
fn interact_range_vectors() {
    use super::interact_range;
    // Waypoint 119 (5 × 5) at (4899, 4209): x band [4895, 4904], y band
    // [4205, 4214]; the player (size 2) at offset (4, 3) is in a middle
    // row, at offset 5 outside rule 5's box (REC-94 run r3).
    let wp = (4899, 4209);
    assert!(interact_range((4895, 4212), 2, wp, (5, 5), 9));
    assert!(!interact_range((4894, 4212), 2, wp, (5, 5), 9));
    // The outer rows cut the corners: (bx − 2, by − 2) is out, (bx − 1,
    // by − 2) in.
    assert!(!interact_range((4895, 4205), 2, wp, (5, 5), 9));
    assert!(interact_range((4896, 4205), 2, wp, (5, 5), 9));
    // A unit wider than 2 keeps the corners.
    assert!(interact_range((4895, 4205), 3, wp, (5, 5), 9));
    // Stash 267 (1 × 1) at (4866, 4229): offset (3, 1) in, (4, 1) out.
    let st = (4866, 4229);
    assert!(interact_range((4869, 4228), 2, st, (1, 1), 5));
    assert!(!interact_range((4870, 4228), 2, st, (1, 1), 5));
    // Unit distance 0 is in range whatever the box.
    assert!(interact_range((4900, 4300), 2, st, (1, 1), 0));
    // A size below 1: the 3 × 3 cells around the origin.
    assert!(interact_range((4867, 4230), 2, st, (0, 1), 5));
    assert!(!interact_range((4868, 4230), 2, st, (0, 1), 5));
}
