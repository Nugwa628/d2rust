// Spec: specs/world/objects.md
//! Mutation-testing kills for the object module (METHODS M08): each test
//! pins one spec rule that no earlier test decided. Own fixture, so the
//! file does not depend on the private helpers of `tests.rs`.

use d2_data::tables::{Levels, Objects};

use super::fake::*;
use super::*;

const O: UnitId = UnitId(10);

const URN: u16 = 4; // init 2
const DOOR: u16 = 6; // init 5

fn tables() -> ObjectTables {
    let mut objects: Vec<Objects> = (0..400).map(|_| blank_object()).collect();
    objects[URN as usize].initfn = 2;
    objects[DOOR as usize].initfn = 5;
    let levels: Vec<Levels> = (0..132).map(|_| blank_level()).collect();
    ObjectTables {
        objects,
        shrines: vec![blank_shrine(); 3],
        levels,
        objgroup: Vec::new(),
        leveldefs: Vec::new(),
    }
}

fn control(t: &ObjectTables, c: Seed) -> ObjectControl {
    let (mut ctl, _) = ObjectControl::new(&mut Seed::init(), t);
    ctl.seed = c;
    ctl
}

fn fake(mode: u8, level: u32) -> Fake {
    let mut f = Fake::default();
    f.modes.insert(O, mode);
    f.levels.insert(O, level);
    f.seeds.insert(O, Seed::init());
    f.guids.insert(O, 0x1234);
    f
}

fn run_create(ctl: &mut ObjectControl, t: &ObjectTables, f: &mut Fake, class: u16) {
    create(ctl, t, f, O, class, 0x1234, None, 0, 5, 6).unwrap();
}

// ------------------------------------------------------------------ §3

// Covers: specs/world/objects.md §3 r3, §3 r7
#[test]
fn create_keeps_attackable_and_selectable_already_set() {
    // "flag 0x4 set" / "flag 0x2 set" are an OR: a bit already set stays.
    let mut t = tables();
    t.objects[DOOR as usize].isattackable0 = 1;
    t.objects[DOOR as usize].selectable0 = 1;
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(0, 2);
    f.flags.insert(O, oflags::ATTACKABLE | oflags::SELECTABLE);
    run_create(&mut ctl, &t, &mut f, DOOR);
    assert_eq!(
        f.flags(O) & (oflags::ATTACKABLE | oflags::SELECTABLE),
        oflags::ATTACKABLE | oflags::SELECTABLE
    );
}

// ------------------------------------------------------------------ §5.2

// Covers: specs/world/objects.md §5.2
#[test]
fn urn_trap_threshold_reads_monlvl1_as_i16() {
    let mut t = tables();
    // t = 800: threshold 800 / 8 + 5 = 105 > every roll(100): always trapped.
    t.levels[3].monlvl1 = 800;
    // t = −64 (0xFFC0 read as i16): −8 + 5 = −3, as u16 65533: always
    // trapped too.
    t.levels[4].monlvl1 = 0xFFC0;
    assert_eq!(t.mon_lvl1(3), 800);
    assert_eq!(t.mon_lvl1(4), -64);
    for level in [3, 4] {
        for s in 1..40u32 {
            let mut ctl = control(&t, Seed::new(s, 666));
            let mut f = fake(0, level);
            run_create(&mut ctl, &t, &mut f, URN);
            let it = ctl.get(O).unwrap().interact;
            assert!((1..=8).contains(&it), "level {level} seed {s}: {it}");
        }
    }
}

/// §5.2 on a copy of the control seed: (urn `InteractType`, locked bit)
/// for MonLvl1 `t`, chest when `lockable`.
fn model_52(mut c: Seed, t: i16, lockable: bool) -> (u8, bool) {
    let t = i32::from(t);
    let r = c.roll(100);
    let it = if r < u32::from(((t / 8) + 5) as u16) {
        1 + c.roll(8) as u8
    } else {
        0
    };
    let locked = lockable && (c.roll(100) as i32) < t / 2 + 8;
    (it, locked)
}

// Covers: specs/world/objects.md §5.2
#[test]
fn urn_and_chest_thresholds_match_the_rule_at_every_roll() {
    // t = 40: trapped below 40 / 8 + 5 = 10, locked below 40 / 2 + 8 =
    // 28. Enough seeds that both comparisons meet r = threshold.
    const CHEST: u16 = 3;
    let mut t = tables();
    t.objects[CHEST as usize].initfn = 3;
    t.objects[CHEST as usize].lockable = 1;
    t.levels[5].monlvl1 = 40;
    let (mut at_trap, mut at_lock) = (false, false);
    for s in 1..3000u32 {
        let c = Seed::new(s, 666);
        let mut probe = c;
        let r1 = probe.roll(100);
        at_trap |= r1 == 10;
        for (class, lockable) in [(URN, false), (CHEST, true)] {
            let mut ctl = control(&t, c);
            let mut f = fake(0, 5);
            run_create(&mut ctl, &t, &mut f, class);
            let (it, locked) = model_52(c, 40, lockable);
            let got = ctl.get(O).unwrap().interact;
            assert_eq!(got & 0x7F, it, "seed {s} class {class}");
            assert_eq!(got & 0x80 != 0, locked, "seed {s} class {class}");
            if lockable {
                let mut p = c;
                p.roll(100);
                if it != 0 {
                    p.roll(8);
                }
                at_lock |= p.roll(100) == 28;
            }
        }
    }
    assert!(at_trap && at_lock, "the seeds reach both boundaries");
}

// Covers: specs/world/objects.md §5.2
#[test]
fn chest_lock_threshold_is_signed() {
    // t = −64 (0xFFC0 read as i16): −32 + 8 < 0, never locked; the urn
    // threshold −3 as u16 is 65533, always trapped.
    const CHEST: u16 = 3;
    let mut t = tables();
    t.objects[CHEST as usize].initfn = 3;
    t.objects[CHEST as usize].lockable = 1;
    t.levels[5].monlvl1 = 0xFFC0;
    for s in 1..200u32 {
        let mut ctl = control(&t, Seed::new(s, 666));
        let mut f = fake(0, 5);
        run_create(&mut ctl, &t, &mut f, CHEST);
        let it = ctl.get(O).unwrap().interact;
        assert_eq!(it & 0x80, 0, "seed {s}");
        assert!((1..=8).contains(&it), "seed {s}");
    }
}

// ------------------------------------------------------------------ §5.1

// Covers: specs/world/objects.md §5.1 r2
#[test]
fn shrine_without_parm0_draws_once_with_one_row_besides_row_0() {
    // n = shrines count − 1 = 1: `roll(1)` is a draw (n < 1 is the only
    // no-draw case); id 1.
    const SHRINE: u16 = 2;
    let mut t = tables();
    t.objects[SHRINE as usize].initfn = 1;
    t.shrines.truncate(2);
    let c = Seed::new(77, 666);
    let mut ctl = control(&t, c);
    let mut f = fake(0, 5);
    run_create(&mut ctl, &t, &mut f, SHRINE);
    let mut want = c;
    want.step();
    assert_eq!(ctl.seed, want);
    assert_eq!(ctl.get(O).unwrap().shrine, Some(1));
    // n = 0 (row 0 only): no draw, id 1, whose row is then missing.
    t.shrines.truncate(1);
    let mut ctl = control(&t, c);
    let mut f = fake(0, 5);
    assert!(matches!(
        create(&mut ctl, &t, &mut f, O, SHRINE, 1, None, 0, 0, 0),
        Err(ObjectError::NoRow {
            table: "shrines",
            row: 1
        })
    ));
    assert_eq!(ctl.seed, c, "n = 0 draws nothing");
}

// ------------------------------------------------------------------ §14

// Covers: specs/world/objects.md §14 r1
#[test]
fn update_sends_0x4d_only_for_subclass_bit_0() {
    // Mode 1 with an operator, but `SubClass` 2 (neither bit 0 nor bit 2):
    // 0x0E alone.
    const C: u16 = 20;
    let mut t = tables();
    t.objects[C as usize].subclass = 2;
    let mut ctl = control(&t, Seed::init());
    let mut f = fake(1, 2);
    f.flags.insert(O, oflags::CHANGED);
    ctl.data.insert(
        O,
        ObjectData {
            guid: 0x1234,
            class: C,
            shrine: Some(1),
            operator: 0x51,
            ..ObjectData::default()
        },
    );
    assert_eq!(
        update_messages(&ctl, &t, &f, O),
        Ok(vec![UpdateMessage::State(state_message(0x1234, false, 1))])
    );
    // The same object with bit 0 sends the shrine message.
    t.objects[C as usize].subclass = 3;
    assert_eq!(update_messages(&ctl, &t, &f, O).unwrap().len(), 2);
}
