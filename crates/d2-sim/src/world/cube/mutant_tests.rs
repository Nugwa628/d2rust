// Spec: specs/world/cube.md §5, §6; specs/data/callbacks.md §2, §3
//! Mutation-testing kills (METHODS M08): each test pins an outcome the
//! spec decides that no earlier test checked.

use super::tests::*;
use super::*;

// Covers: specs/data/callbacks.md §2 text
#[test]
fn decode_reads_every_slot_at_its_offset() {
    // Every byte distinct-ish: a wrong offset reads a different value.
    let r: Vec<u8> = (0..Cubemain::SIZE).map(|i| (i * 7 + 1) as u8).collect();
    let le = |o: usize| u16::from_le_bytes([r[o], r[o + 1]]);
    let rec = Recipe::decode(&r);
    for (k, s) in rec.inputs.iter().enumerate() {
        let o = 20 + 8 * k;
        let want = InputSlot {
            flags: le(o),
            item: le(o + 2),
            special: le(o + 4),
            quality: r[o + 6],
            quantity: r[o + 7],
        };
        assert_eq!(*s, want, "input {k}");
    }
    for (k, s) in rec.outputs.iter().enumerate() {
        let o = 76 + 84 * k;
        assert_eq!(
            (s.flags, s.item, s.special, s.quality, s.quantity, s.kind),
            (le(o), le(o + 2), le(o + 4), r[o + 6], r[o + 7], r[o + 8]),
            "output {k}"
        );
        assert_eq!(s.pre, [le(o + 12), le(o + 14), le(o + 16)], "pre {k}");
        assert_eq!(s.suf, [le(o + 18), le(o + 20), le(o + 22)], "suf {k}");
    }
}

// From specs/world/cube.md §5.
#[test]
fn stat_ops_compare_le_ne_eq_and_test_record_zero() {
    let d = data(vec![]);
    let mut f = Fake::new();
    f.player_stats.insert(4, 5);
    let mut r = gem_recipe();
    r.param = 4;
    let pass = |op: u8, value: i32| {
        let mut r = r.clone();
        r.op = op;
        r.value = value;
        d.recipe_op(&f, P, &r, (1, 1))
    };
    // op 4: stat <= t
    assert!(pass(4, 5) && pass(4, 6) && !pass(4, 4));
    // op 5: stat != t
    assert!(!pass(5, 5) && pass(5, 4) && pass(5, 6));
    // op 6: stat == t
    assert!(pass(6, 5) && !pass(6, 4) && !pass(6, 6));
    // s = 0 is in range: record 0 is fetched and compared (stat 0 = 0 < 1).
    r.op = 3;
    r.param = 0;
    r.value = 1;
    assert!(!d.recipe_op(&f, P, &r, (1, 1)));
}

// From specs/world/cube.md §6.3.
#[test]
fn upg_elite_item_accepts_elite_slot() {
    let one = |slot: u32, have: u32| {
        let d = data(vec![recipe(
            1,
            &[input(input_flags::USEANY | input_flags::UPG, slot, 0)],
            out(kind::ITEMCODE, RIN),
        )]);
        let mut f = Fake::new();
        f.add(have, 2);
        d.transmute(&mut f, P).record.is_some()
    };
    assert!(one(UHL, UHL));
    assert!(one(XHL, UHL) && one(FHL, UHL));
}

// From specs/world/cube.md §6.2.
#[test]
fn runeword_and_quest_tests_only_when_selected() {
    // Test 7 needs `nru`: without it a runeword item passes.
    let d = data(vec![recipe(1, &[code_in(HAX)], out(kind::ITEMCODE, RIN))]);
    let mut f = Fake::new();
    let h = f.add(HAX, 2);
    f.it(h).flags |= item_flags::RUNEWORD;
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
    // Test 8 needs op 28: with op 0 a quest item below the difficulty passes.
    let d = data(vec![recipe(1, &[code_in(MSF)], out(kind::ITEMCODE, RIN))]);
    let mut f = Fake::new();
    f.difficulty = 2;
    f.add(MSF, 2);
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
}

// From specs/world/cube.md §6.2.
#[test]
fn input0_stat_op_filters_slot0_candidates() {
    // op 15: item stat value >= t on slot-0 candidates.
    let mut r = recipe(1, &[code_in(RIN)], out(kind::ITEMCODE, AMU));
    r.op = 15;
    r.param = 4;
    r.value = 3;
    let d = data(vec![r]);
    let mut f = Fake::new();
    let ring = f.add(RIN, 2);
    f.it(ring).stats.insert(4, 2);
    assert_eq!(d.transmute(&mut f, P).record, None);
    f.it(ring).stats.insert(4, 3);
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
}

// From specs/world/cube.md §6.4.
#[test]
fn capture_raises_level_and_keys_on_mod_flag() {
    let d = data(vec![]);
    let mut f = Fake::new();
    let h = f.add(HAX, 2);
    f.it(h).level = 0;
    let mut r = recipe(1, &[code_in(HAX)], out(kind::ITEMCODE, RIN));
    let mut cap = [Capture::default(); 3];
    d.capture(&mut f, &r, h, HAX, &mut cap);
    assert_eq!(f.items[&h].level, 1);
    assert!(cap.iter().all(|c| c.level == 1 && c.item.is_none()));
    r.outputs[0].flags = output_flags::MOD;
    let mut cap = [Capture::default(); 3];
    d.capture(&mut f, &r, h, HAX, &mut cap);
    assert!(cap.iter().all(|c| c.item == Some(h)));
}

// From specs/world/cube.md §6.4.
#[test]
fn capture_class_upgrade_follows_exc_eli_and_version() {
    let class_of = |flags: u16, expansion: bool, xhl_version: u16| {
        let mut d = data(vec![]);
        d.items[XHL as usize].version = xhl_version;
        let mut f = Fake::new();
        f.expansion = expansion;
        let h = f.add(FHL, 2);
        let mut a = out(kind::USETYPE, 0);
        a.flags = flags;
        let r = recipe(1, &[code_in(FHL)], a);
        let mut cap = [Capture::default(); 3];
        d.capture(&mut f, &r, h, FHL, &mut cap);
        cap[0].class
    };
    assert_eq!(class_of(0, true, 100), FHL as i32);
    assert_eq!(class_of(output_flags::EXC, true, 100), XHL as i32);
    assert_eq!(class_of(output_flags::ELI, true, 100), UHL as i32);
    // Classic game: the upgrade needs version < 100.
    assert_eq!(class_of(output_flags::EXC, false, 0), XHL as i32);
    assert_eq!(class_of(output_flags::EXC, false, 100), FHL as i32);
}

/// A game seed whose first roll(n) is `first`.
fn seed_rolling(n: i32, first: u32) -> Seed {
    (1..)
        .map(Seed::init_low)
        .find(|s| s.clone().roll(n) == first)
        .unwrap()
}

// From specs/world/cube.md §7.5 r1, §7.5 r2.
#[test]
fn type_pick_stop_and_version_filter() {
    let d = data(vec![]);
    let n = d.items.len() as i32;
    let pick = |ty_class: u32, start: u32, expansion: bool| {
        let mut f = Fake::new();
        f.expansion = expansion;
        f.types = vec![(ty_class, 9)];
        f.seed = seed_rolling(n, start);
        d.type_pick(&mut f, 9, 99)
    };
    // start 1: stop = 0, so the last item (amu) is examined.
    assert_eq!(pick(AMU, 1, true), AMU);
    // start 0: stop = N − 1; version < 100 or item format ≥ 100.
    assert_eq!(pick(FHL, 0, false), FHL);
    assert_eq!(pick(XHL, 0, false), 0);
    assert_eq!(pick(XHL, 0, true), XHL);
}

// From specs/world/cube.md §7.5 r2, §7.5 r3.
#[test]
fn type_pick_keeps_at_most_256_candidates() {
    let mut d = data(vec![]);
    d.items = (0..300).map(|_| rec(b"rin ")).collect();
    let mut f = Fake::new();
    f.types = (0..300).map(|i| (i, 9)).collect();
    f.seed = Seed::init_low(7);
    let mut s = f.seed;
    let start = s.roll(300);
    let k = s.roll(256);
    assert_eq!(d.type_pick(&mut f, 9, 99), (start + k) % 300);
}

/// Transmute one HAX (set up by `setup`) with output a = `a`.
fn run(a: OutputSlot, setup: &dyn Fn(&mut Fake, UnitId)) -> (Fake, Transmute) {
    let d = data(vec![recipe(1, &[code_in(HAX)], a)]);
    let mut f = Fake::new();
    let h = f.add(HAX, 2);
    setup(&mut f, h);
    let t = d.transmute(&mut f, P);
    (f, t)
}

fn none(_: &mut Fake, _: UnitId) {}

fn has(f: &Fake, prefix: &str) -> bool {
    f.log.iter().any(|l| l.starts_with(prefix))
}

// From specs/world/cube.md §7.2.
#[test]
fn kind_none_makes_nothing_even_with_mod() {
    let mut a = out(kind::NONE, 0);
    a.flags = output_flags::MOD;
    let (f, t) = run(a, &none);
    assert!(t.outputs.is_empty() && !t.committed);
    assert!(!has(&f, "dup"));
}

// From specs/world/cube.md §7.3.
#[test]
fn mod_copy_class_by_kind() {
    let class = |a: OutputSlot, input: u32, f0: &dyn Fn(&mut Fake)| {
        let d = data(vec![recipe(1, &[code_in(input)], a)]);
        let mut f = Fake::new();
        f.add(input, 2);
        f0(&mut f);
        let t = d.transmute(&mut f, P);
        f.items[&t.outputs[0]].class
    };
    let mut a = out(kind::ITEMCODE, RIN);
    a.flags = output_flags::MOD;
    assert_eq!(class(a, HAX, &|_| {}), Some(RIN));
    let mut a = out(kind::USETYPE, 0);
    a.flags = output_flags::MOD;
    assert_eq!(class(a, JEW, &|_| {}), Some(JEW));
    // Type pick with L = 1: rin is the only item of the type.
    let mut a = out(kind::ITEMTYPE, 9);
    a.flags = output_flags::MOD;
    let n = items().len() as i32;
    let pick = |f: &mut Fake| {
        f.types = vec![(RIN, 9)];
        f.seed = seed_rolling(n, 0);
    };
    assert_eq!(class(a, HAX, &pick), Some(RIN));
}

// From specs/world/cube.md §7.4 text.
#[test]
fn request_sockets_only_without_quantity() {
    let mut a = out(kind::ITEMCODE, RIN);
    a.flags = output_flags::SOCK;
    a.quantity = 3;
    let (f, _) = run(a, &none);
    assert_eq!(f.requests[0].flags2, 0x08 | 0x02);
}

// From specs/world/cube.md §7.6 r2.
#[test]
fn uns_alone_loses_the_fillers() {
    let mut a = out(kind::ITEMCODE, RIN);
    a.flags = output_flags::MOD | output_flags::UNS;
    let (f, t) = run(a, &|f, h| {
        let gem = f.new_item(Item {
            class: Some(GCV),
            ..Item::default()
        });
        f.it(h).socketed = vec![gem];
    });
    assert!(t.committed);
    assert!(has(&f, "drop rw"));
    assert_eq!(f.log.iter().filter(|l| l.starts_with("dup")).count(), 1);
}

// From specs/world/cube.md §7.6 r3.
#[test]
fn craft_mod_property_zero_and_chance_100() {
    let mut a = out(kind::ITEMCODE, RIN);
    a.mods[0] = CraftMod {
        property: 0,
        param: 1,
        min: 2,
        max: 3,
        chance: 100,
    };
    let (f, t) = run(a, &none);
    assert!(f.log.iter().any(|l| l == "prop 0 1 2 3"));
    // chance 100 draws nothing from the output's seed.
    assert_eq!(f.items[&t.outputs[0]].seed, Seed::init_low(1234));
}

fn stat_set(f: &Fake, x: UnitId, stat: u16) -> Option<&String> {
    let p = format!("stat {} {stat} ", x.0);
    f.log.iter().find(|l| l.starts_with(&p))
}

// From specs/world/cube.md §7.6 r4, §7.6 r6.
#[test]
fn rep_and_quantity_steps() {
    use output_flags as fl;
    let made = |kind_item: u32, flags: u16, qty: u8, setup: &dyn Fn(&mut Fake, UnitId)| {
        let mut a = out(kind::ITEMCODE, kind_item);
        a.flags = flags;
        a.quantity = qty;
        let (f, t) = run(a, setup);
        let x = t.outputs[0];
        (f, x)
    };
    // No `rep`: a copy below max durability is left alone, no repair.
    let worn = |f: &mut Fake, h: UnitId| {
        f.it(h).stats.insert(STAT_MAX_DURABILITY, 10);
        f.it(h).stats.insert(STAT_DURABILITY, 5);
        f.it(h).flags |= item_flags::BROKEN;
    };
    let (f, x) = made(RIN, fl::MOD, 0, &worn);
    assert!(stat_set(&f, x, STAT_DURABILITY).is_none() && !has(&f, "repair"));
    // `rep`: durability equal to max is not rewritten.
    let (f, x) = made(RIN, fl::REP, 0, &none);
    assert!(stat_set(&f, x, STAT_DURABILITY).is_none());
    // `rep` quantity needs stackable and quantity ≠ 0.
    let (f, x) = made(RIN, fl::REP, 3, &none);
    assert!(stat_set(&f, x, STAT_QUANTITY).is_none());
    let (f, x) = made(CQV, fl::REP, 0, &none);
    assert!(stat_set(&f, x, STAT_QUANTITY).is_none());
    // No `sock`: quantity ≠ 0 and stackable → stat 70, no sockets.
    let (f, x) = made(CQV, 0, 5, &none);
    assert_eq!(
        stat_set(&f, x, STAT_QUANTITY).unwrap(),
        &format!("stat {} 70 5", x.0)
    );
    assert!(!has(&f, "sockets"));
    let (f, x) = made(CQV, 0, 0, &none);
    assert!(stat_set(&f, x, STAT_QUANTITY).is_none());
    let (f, x) = made(RIN, 0, 3, &none);
    assert!(stat_set(&f, x, STAT_QUANTITY).is_none());
}

// From specs/world/cube.md §7.6 r6.
#[test]
fn sock_with_no_room_adds_nothing() {
    let mut a = out(kind::ITEMCODE, RIN);
    a.flags = output_flags::MOD | output_flags::SOCK;
    a.quantity = 2;
    let (f, t) = run(a, &none); // hax copy: max sockets 0
    assert!(!has(&f, "sockets"));
    assert_eq!(f.items[&t.outputs[0]].flags & item_flags::SOCKETED, 0);
}

// From specs/world/cube.md §8 r3.
#[test]
fn quest_hook_only_for_hst_and_qf2() {
    let (f, t) = run(out(kind::ITEMCODE, MSF), &none);
    assert!(t.committed);
    assert!(!has(&f, "hook"));
}

// From specs/world/cube.md §6.2 test 9: the output-a socket test is for
// slot 0 only.
#[test]
fn socket_test_only_on_slot_0() {
    let mut a = out(kind::USETYPE, 0);
    a.flags = output_flags::SOCK;
    let d = data(vec![recipe(2, &[code_in(HAX), code_in(RIN)], a)]);
    let run = |hax_max: i32| {
        let mut f = Fake::new();
        let h = f.add(HAX, 2);
        f.it(h).max_sockets = hax_max;
        f.add(RIN, 2); // max sockets 0, slot 1
        d.transmute(&mut f, P).record
    };
    assert_eq!(run(2), Some(0));
    assert_eq!(run(0), None);
}

fn put_msg(item: UnitId, cube: UnitId) -> Vec<u8> {
    let mut m = vec![0x2A];
    m.extend(item.0.to_le_bytes());
    m.extend(cube.0.to_le_bytes());
    m
}

// From specs/world/cube.md §2 r3: the cube must be mode 0 and `box `; the
// trading refusal needs trading and a cube page ≠ 0.
#[test]
fn put_in_cube_tests() {
    let d = data(vec![]);
    let setup = |cube_class: u32| {
        let mut f = Fake::new();
        let cube = f.add(cube_class, 2);
        f.it(cube).page = 0;
        let ring = f.add(RIN, 2);
        f.it(ring).page = 0;
        f.it(ring).mode = 4;
        (f, cube, ring)
    };
    // A stored hax is no cube.
    let (mut f, hax, ring) = setup(HAX);
    assert_eq!(d.put_in(&mut f, P, &put_msg(ring, hax)), 3);
    assert_eq!(f.items[&ring].page, 0);
    // Not trading, cube on page 1: placed, no sound.
    let (mut f, cube, ring) = setup(BOX);
    f.it(cube).page = 1;
    assert_eq!(d.put_in(&mut f, P, &put_msg(ring, cube)), 0);
    assert_eq!(f.items[&ring].page, CUBE_PAGE);
    assert!(!f.log.iter().any(|l| l == "sound 19"));
    // Trading, cube on page 0: placed, no sound.
    let (mut f, cube, ring) = setup(BOX);
    f.trading = true;
    assert_eq!(d.put_in(&mut f, P, &put_msg(ring, cube)), 0);
    assert_eq!(f.items[&ring].page, CUBE_PAGE);
    assert!(!f.log.iter().any(|l| l == "sound 19"));
}
