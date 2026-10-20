// Spec: specs/world/cube.md
//! Gap tests: the numbered edge cases (§Edge cases & original bugs 8–14)
//! not yet claimed by other tests, one test per rule.

#[allow(unused_imports)]
use super::*;

use super::tests::{code_in, data, input, out, recipe, Fake, Item, AQV, BOX, GCV, GFV, P, RIN};

/// An item-type output (kind 0xFD, type 77) from one `gcv`.
fn type_output_class(d: &CubeData, f: &mut Fake) -> u32 {
    f.add(GCV, 2);
    let t = d.transmute(f, P);
    assert_eq!(t.record, Some(0));
    assert_eq!(f.requests.len(), 1);
    f.requests[0].class
}

// Covers: specs/world/cube.md §edge-cases-original-bugs r8
#[test]
fn type_pick_skips_stop_and_falls_back_to_item_0() {
    let seed = Seed::init_low(5);
    let mk = || data(vec![recipe(1, &[code_in(GCV)], out(kind::ITEMTYPE, 77))]);
    let n = mk().items.len() as i32;
    let start = {
        let mut s = seed;
        s.roll(n)
    };
    assert!(start >= 2, "vector needs start - 1 > 0 and != 0");
    let stop = start - 1;

    // The only member of the type sits at `stop`: never examined → item 0.
    let d = mk();
    let mut f = Fake::new();
    f.seed = seed;
    f.types = vec![(stop, 77)];
    assert_eq!(type_output_class(&d, &mut f), 0);

    // The only member sits at `start` but does not qualify (not
    // spawnable): no candidate → item 0 (a real item, `hax`).
    let mut d = mk();
    d.items[start as usize].spawnable = 0;
    let mut f = Fake::new();
    f.seed = seed;
    f.types = vec![(start, 77)];
    assert_eq!(type_output_class(&d, &mut f), 0);

    // Control: spawnable at `start` → picked.
    let d = mk();
    let mut f = Fake::new();
    f.seed = seed;
    f.types = vec![(start, 77)];
    assert_eq!(type_output_class(&d, &mut f), start);
}

// Covers: specs/world/cube.md §edge-cases-original-bugs r9
#[test]
fn mod_chance_passes_when_lo_mod_100_le_chance() {
    // The fake's created items carry seed init_low(1234); the mod draw is
    // one step of it.
    let k = Seed::init_low(1234).step() % 100;
    assert!(
        (2..100).contains(&k),
        "vector needs 2 <= lo' mod 100 < 100: {k}"
    );
    let applied = |chance: u8| {
        let mut a = out(kind::ITEMCODE, GFV);
        a.mods[0] = CraftMod {
            property: 5,
            chance,
            ..CraftMod::default()
        };
        let d = data(vec![recipe(1, &[code_in(GCV)], a)]);
        let mut f = Fake::new();
        f.add(GCV, 2);
        let t = d.transmute(&mut f, P);
        assert!(t.committed);
        // The draw happens either way (0 < chance < 100).
        let mut s = Seed::init_low(1234);
        s.step();
        assert_eq!(f.items[&t.outputs[0]].seed, s);
        f.log.contains(&"prop 5 0 0 0".to_string())
    };
    // lo' mod 100 = chance passes (≤, not <); one below fails. So chance c
    // passes for c + 1 of the 100 residues.
    assert!(applied(k as u8));
    assert!(applied(k as u8 + 1));
    assert!(!applied(k as u8 - 1));
}

// Covers: specs/world/cube.md §edge-cases-original-bugs r10
#[test]
fn stat_op_param_equal_to_count_fails_above_passes() {
    // valshift has 10 records (count = 10). Op 3: stat value ≥ t. The
    // player's stats are all 0.
    let run = |param: i32, value: i32| {
        let mut r = recipe(1, &[code_in(GCV)], out(kind::ITEMCODE, GFV));
        r.op = 3;
        r.param = param;
        r.value = value;
        let d = data(vec![r]);
        assert_eq!(d.valshift.len(), 10);
        let mut f = Fake::new();
        f.add(GCV, 2);
        d.transmute(&mut f, P).record
    };
    // Control: a real record, 0 ≥ 0 passes; 0 ≥ 5 fails.
    assert_eq!(run(9, 0), Some(0));
    assert_eq!(run(9, 5), None);
    // param = count: no record → fails even though 0 ≥ 0 would pass.
    assert_eq!(run(10, 0), None);
    // param > count (and param < 0): passes without a test.
    assert_eq!(run(11, 5), Some(0));
    assert_eq!(run(-1, 5), Some(0));
}

// Covers: specs/world/cube.md §edge-cases-original-bugs r11
#[test]
fn quality_9_mismatch_is_a_plain_mismatch() {
    let run = |item_quality: u8| {
        let mut i = input(input_flags::USEANY, RIN, 0);
        i.quality = 9;
        let d = data(vec![recipe(1, &[i], out(kind::ITEMCODE, GFV))]);
        let mut f = Fake::new();
        f.add(RIN, item_quality);
        let t = d.transmute(&mut f, P);
        (t.record, f.log.clone(), f.requests.len())
    };
    // The getter called on the mismatch has no effect: nothing logged,
    // nothing made, the record just fails.
    for q in [2, 4, 8] {
        assert_eq!(run(q), (None, vec![], 0), "quality {q}");
    }
    assert_eq!(run(9).0, Some(0));
}

// Covers: specs/world/cube.md §edge-cases-original-bugs r12
#[test]
fn reg_keeps_source_level_unclamped() {
    let mut a = out(kind::USETYPE, 0);
    a.flags = output_flags::REG;
    let d = data(vec![recipe(1, &[code_in(RIN)], a)]);
    assert_eq!(d.max_level, 99);
    let mut f = Fake::new();
    let r = f.add(RIN, 4);
    f.it(r).level = 150;
    d.transmute(&mut f, P);
    // Without `reg` the level would clamp to [1, 99]; with it the source
    // item's level goes through as is.
    assert_eq!(f.requests[0].level, 150);
    assert_eq!(f.requests[0].quality, 4);
    // Control: same output without `reg` → clamped (empty sum → 1).
    let d = data(vec![recipe(1, &[code_in(RIN)], out(kind::USETYPE, 0))]);
    let mut f = Fake::new();
    let r = f.add(RIN, 4);
    f.it(r).level = 150;
    d.transmute(&mut f, P);
    assert_eq!(f.requests[0].level, 1);
}

// Covers: specs/world/cube.md §edge-cases-original-bugs r13
#[test]
fn rep_with_qty_255_refills_to_full_stack() {
    // A throwing-weapon-like stackable: maxstack 40 (< 255).
    let mut a = out(kind::USEITEM, 0);
    a.flags = output_flags::REP;
    a.quantity = 255;
    let mut d = data(vec![recipe(1, &[code_in(AQV)], a)]);
    d.items[AQV as usize].maxstack = 40;
    let mut f = Fake::new();
    let q = f.add(AQV, 2);
    f.it(q).stats.insert(STAT_QUANTITY, 3);
    let t = d.transmute(&mut f, P);
    assert!(t.committed);
    let x = t.outputs[0];
    assert_eq!(f.items[&x].stats[&STAT_QUANTITY], 40);
    assert!(f.log.contains(&format!("stat {} {STAT_QUANTITY} 40", x.0)));
}

// Covers: specs/world/cube.md §edge-cases-original-bugs r14
#[test]
fn put_in_ignores_placement_result() {
    let d = data(vec![]);
    let mut f = Fake::new();
    f.place_ok = false;
    let cube = f.new_item(Item {
        class: Some(BOX),
        ..Item::default()
    });
    let it = f.new_item(Item {
        class: Some(GCV),
        mode: 4,
        page: 0xFF,
        ..Item::default()
    });
    let mut m = vec![0x2A];
    m.extend(it.0.to_le_bytes());
    m.extend(cube.0.to_le_bytes());
    // Placement fails (nothing placed), the handler still succeeds.
    assert_eq!(d.put_in(&mut f, P, &m), 0);
    assert_eq!(f.items[&it].page, CUBE_PAGE);
    assert_eq!(f.log, ["targeting".to_string()]);
}
