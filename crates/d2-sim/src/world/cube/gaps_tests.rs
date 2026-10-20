// Spec: specs/world/cube.md
//! Gap tests: rules of the spec not yet claimed by other tests.

#[allow(unused_imports)]
use super::*;

use super::tests::{
    code_in, data, input, out, recipe, Fake, Item, AQV, BOX, FHL, GCR, GCV, GFV, HAX, LEG, P, RIN,
};

fn put_msg(item: UnitId, cube: UnitId) -> Vec<u8> {
    let mut m = vec![0x2A];
    m.extend(item.0.to_le_bytes());
    m.extend(cube.0.to_le_bytes());
    m
}

// Covers: specs/world/cube.md §2 text
#[test]
fn put_in_size_and_any_item_kind() {
    let d = data(vec![]);
    let mut f = Fake::new();
    let cube = f.new_item(Item {
        class: Some(BOX),
        ..Item::default()
    });
    // A second cube on the cursor is offered like any item.
    let other = f.new_item(Item {
        class: Some(BOX),
        mode: 4,
        page: 0xFF,
        ..Item::default()
    });
    let m = put_msg(other, cube);
    assert_eq!(d.put_in(&mut f, P, &m[..8]), 3);
    assert_eq!(d.put_in(&mut f, P, &[&m[..], &[0]].concat()), 3);
    assert!(f.log.is_empty());
    assert_eq!(d.put_in(&mut f, P, &m), 0);
    assert_eq!(f.items[&other].page, CUBE_PAGE);
    assert_eq!(
        f.log,
        ["targeting".to_string(), format!("place {}", other.0)]
    );
    assert!(f.sent.is_empty());
}

// Covers: specs/world/cube.md §6 text, §6.1 r1
#[test]
fn seven_slots_in_order_empty_slots_pass() {
    // Slot 1 has flags without 0x0003 (empty); slot 6 is still tested.
    let mut ins = [InputSlot::default(); 7];
    ins[0] = code_in(GCV);
    ins[1] = input(input_flags::NOS, 0, 5);
    ins[6] = code_in(RIN);
    let d = data(vec![recipe(2, &ins, out(kind::ITEMCODE, GFV))]);
    let mut f = Fake::new();
    f.add(GCV, 2);
    f.add(RIN, 2);
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
    // Slot 6 fails → the record fails.
    let mut f = Fake::new();
    f.add(GCV, 2);
    f.add(GFV, 2);
    assert_eq!(d.transmute(&mut f, P).record, None);
}

// Covers: specs/world/cube.md §6.1 r2
#[test]
fn quantity_zero_needs_one() {
    let try_ = |ins: &[InputSlot], n: u8, items: &[u32]| {
        let d = data(vec![recipe(n, ins, out(kind::ITEMCODE, GFV))]);
        let mut f = Fake::new();
        for &c in items {
            f.add(c, 2);
        }
        d.transmute(&mut f, P).record
    };
    let rin = code_in(RIN);
    assert_eq!(try_(&[input(1, GCV, 0), rin], 2, &[GCV, RIN]), Some(0));
    assert_eq!(try_(&[input(1, GCV, 2), rin], 2, &[GCV, RIN]), None);
    // Exactly one when not stackable.
    assert_eq!(try_(&[input(1, GCV, 0)], 2, &[GCV, GCV]), None);
}

// Covers: specs/world/cube.md §6.1 r6
#[test]
fn stackable_match_means_at_least() {
    let run = |ins: &[InputSlot], items: &[u32], types: &[(u32, u16)]| {
        let d = data(vec![recipe(
            items.len() as u8,
            ins,
            out(kind::ITEMCODE, GFV),
        )]);
        let mut f = Fake::new();
        f.types = types.to_vec();
        for &c in items {
            f.add(c, 2);
        }
        d.transmute(&mut f, P).record
    };
    assert_eq!(run(&[input(1, AQV, 1)], &[AQV, AQV, AQV], &[]), Some(0));
    assert_eq!(run(&[input(1, GCV, 1)], &[GCV, GCV, GCV], &[]), None);
    // One stackable match among the found makes the test "at least".
    let ty = input(input_flags::ITEMCODE, 50, 1);
    let types = [(AQV, 50), (GCV, 50)];
    assert_eq!(run(&[ty], &[GCV, AQV], &types), Some(0));
    assert_eq!(run(&[ty], &[GCV, GCV], &types), None);
}

// Covers: specs/world/cube.md §7 text, §7.6 r1
#[test]
fn outputs_in_slot_order_item_sets_success() {
    // a: a failing cow portal (success 0); b, c: items → success 1.
    let mut r = recipe(1, &[code_in(LEG)], out(kind::COW_PORTAL, 0));
    r.outputs[1] = out(kind::ITEMCODE, GFV);
    r.outputs[2] = out(kind::ITEMCODE, GCR);
    let d = data(vec![r]);
    let mut f = Fake::new();
    let leg = f.add(LEG, 2);
    let t = d.transmute(&mut f, P);
    assert!(t.committed);
    assert_eq!(
        f.requests.iter().map(|q| q.class).collect::<Vec<_>>(),
        [GFV, GCR]
    );
    assert_eq!(
        f.log[..3],
        [
            "cow".to_string(),
            "sound 20".into(),
            format!("remove {}", leg.0)
        ]
    );
    // Nothing made: success stays 0, nothing removed.
    let d = data(vec![recipe(1, &[code_in(LEG)], out(kind::NONE, 0))]);
    let mut f = Fake::new();
    f.add(LEG, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!((t.record, t.committed), (Some(0), false));
    assert!(f.log.is_empty());
}

// Covers: specs/world/cube.md §7.1 r1
#[test]
fn remove_flag_is_uns_or_rem() {
    for (flags, remove) in [
        (0, false),
        (output_flags::UNS, true),
        (output_flags::REM, true),
        (output_flags::REP, false),
    ] {
        let mut a = out(kind::USEITEM, 0);
        a.flags = flags;
        let d = data(vec![recipe(1, &[code_in(HAX)], a)]);
        let mut f = Fake::new();
        let h = f.add(HAX, 2);
        let t = d.transmute(&mut f, P);
        let x = t.outputs[0];
        assert!(f.log.contains(&format!("dup {} {}", h.0, !remove)));
        assert_eq!(f.log.contains(&format!("drop rw {}", x.0)), remove);
    }
}

// Covers: specs/world/cube.md §7.5 text
#[test]
fn type_pick_never_examines_stop() {
    let d = data(vec![]);
    let n = d.items.len() as i32;
    let seed = Seed::init_low(5);
    let start = {
        let mut s = seed;
        s.roll(n)
    };
    assert_ne!(start, 0);
    let stop = start - 1;
    // The only candidate sits at `stop`: no candidate, item 0, one draw.
    let mut f = Fake::new();
    f.seed = seed;
    f.types = vec![(stop, 77)];
    assert_eq!(d.type_pick(&mut f, 77, 99), 0);
    let mut one = seed;
    one.roll(n);
    assert_eq!(f.seed, one);
    // At `start` it is found.
    let mut f = Fake::new();
    f.seed = seed;
    f.types = vec![(start, 77)];
    assert_eq!(d.type_pick(&mut f, 77, 99), start);
}

// Covers: specs/world/cube.md §7.6 text, §7.6 r5
#[test]
fn after_item_steps_in_order_and_recharge() {
    let run = |flags: u16| {
        let mut a = out(kind::USEITEM, 0);
        a.flags = flags;
        a.quantity = 2;
        a.mods[0] = CraftMod {
            property: 5,
            ..CraftMod::default()
        };
        let d = data(vec![recipe(1, &[code_in(HAX)], a)]);
        let mut f = Fake::new();
        let h = f.add(HAX, 2);
        f.it(h).max_sockets = 6;
        f.it(h).stats.insert(STAT_MAX_DURABILITY, 50);
        f.it(h).stats.insert(STAT_DURABILITY, 10);
        let t = d.transmute(&mut f, P);
        (f.log.clone(), t.outputs[0])
    };
    use output_flags as o;
    let (log, x) = run(o::REP | o::RCH | o::SOCK);
    let at = |s: &str| log.iter().position(|l| l == s).unwrap();
    let order = [
        at("prop 5 0 0 0"),
        at(&format!("stat {} 72 50", x.0)),
        at("recharge"),
        at("sockets 2"),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{log:?}");
    let (log, _) = run(o::REP | o::SOCK);
    assert!(!log.contains(&"recharge".to_string()));
}

// Covers: specs/world/cube.md §edge-cases-original-bugs r1, §edge-cases-original-bugs r2, §edge-cases-original-bugs r3, §edge-cases-original-bugs r4, §edge-cases-original-bugs r5, §edge-cases-original-bugs r6, §edge-cases-original-bugs r7
#[test]
fn edge_cases() {
    // 1. First matching record wins even if all its outputs fail.
    let d = data(vec![
        recipe(1, &[code_in(GCV)], out(kind::NONE, 0)),
        recipe(1, &[code_in(GCV)], out(kind::ITEMCODE, GFV)),
    ]);
    let mut f = Fake::new();
    f.add(GCV, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!((t.record, t.committed), (Some(0), false));
    assert!(f.requests.is_empty());

    // 2, 3. An item in slot a, a failing portal in b: nothing consumed,
    // the item is discarded without being placed or freed.
    let mut r = recipe(1, &[code_in(GCV)], out(kind::ITEMCODE, GFV));
    r.outputs[1] = out(kind::PANDEMONIUM, 0);
    let d = data(vec![r]);
    let mut f = Fake::new();
    f.add(GCV, 2);
    let t = d.transmute(&mut f, P);
    assert!(!t.committed && t.outputs.len() == 1);
    assert_eq!(f.cube_contents(), [Some(GCV)]);
    assert!(f.log.is_empty());
    assert!(f.items.contains_key(&t.outputs[0]));

    // 4. Unplaceable outputs are destroyed after the inputs are gone.
    let d = data(vec![recipe(1, &[code_in(GCV)], out(kind::ITEMCODE, GFV))]);
    let mut f = Fake::new();
    f.place_ok = false;
    let g = f.add(GCV, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!(
        f.log,
        [
            format!("remove {}", g.0),
            "sound 4".into(),
            format!("free {}", t.outputs[0].0)
        ]
    );

    // 5. Capture keys on output a: usetype in b with itemcode in a → 0.
    let mut r = recipe(1, &[code_in(RIN)], out(kind::ITEMCODE, GFV));
    r.outputs[1] = out(kind::USETYPE, 0);
    let d = data(vec![r.clone()]);
    let mut f = Fake::new();
    f.add(RIN, 2);
    d.transmute(&mut f, P);
    assert_eq!(f.requests[1].class, 0);
    r.outputs[0] = out(kind::USETYPE, 0);
    let d = data(vec![r]);
    let mut f = Fake::new();
    f.add(RIN, 2);
    d.transmute(&mut f, P);
    assert_eq!((f.requests[0].class, f.requests[1].class), (RIN, RIN));

    // 6. useitem without mod ignores exc; useitem,mod on hax → class 0.
    let mut a = out(kind::USEITEM, 0);
    a.flags = output_flags::EXC;
    let d = data(vec![recipe(1, &[code_in(FHL)], a)]);
    let mut f = Fake::new();
    f.add(FHL, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!(f.items[&t.outputs[0]].class, Some(FHL));
    a.flags |= output_flags::MOD;
    let d = data(vec![recipe(1, &[code_in(HAX)], a)]);
    let mut f = Fake::new();
    f.add(HAX, 2);
    let t = d.transmute(&mut f, P);
    assert_eq!(f.items[&t.outputs[0]].class, Some(HAX));
    assert_eq!(HAX, 0);

    // 7. Quantity counts items, not stack sizes …
    let d = data(vec![recipe(
        1,
        &[input(1, AQV, 2)],
        out(kind::ITEMCODE, GFV),
    )]);
    let mut f = Fake::new();
    let q = f.add(AQV, 2);
    f.it(q).stats.insert(STAT_QUANTITY, 500);
    assert_eq!(d.transmute(&mut f, P).record, None);
    // … "at least" with a stackable match …
    let d = data(vec![recipe(
        3,
        &[input(1, AQV, 2)],
        out(kind::ITEMCODE, GFV),
    )]);
    let mut f = Fake::new();
    for _ in 0..3 {
        f.add(AQV, 2);
    }
    assert_eq!(d.transmute(&mut f, P).record, Some(0));
    // … and a slot marks every match used (V4).
    let d = data(vec![recipe(
        3,
        &[input(1, AQV, 2), code_in(AQV)],
        out(kind::ITEMCODE, GFV),
    )]);
    let mut f = Fake::new();
    for _ in 0..3 {
        f.add(AQV, 2);
    }
    assert_eq!(d.transmute(&mut f, P).record, None);
}
