//! Tests written from the treasure rules to kill mutants that survived
//! `cargo mutants` on `crates/d2-sim/src/treasure/`.
//! Each test asserts what the
//! rule says at the boundary or bit the mutant changed.

#![allow(clippy::float_arithmetic)]

use d2_data::tables::{Armor, Itemratio, Levels, Misc, Weapons};

use super::softfloat::F64;
use super::tests::{
    abcd, args, classes, code, equiv, game, item, ratio_row, src, tc, tcx, zeroed, Sink, World,
    OBJECT,
};
use super::*;
use crate::rng::Seed;

// ------------------------------------------------------- softfloat

fn f(v: i64) -> F64 {
    F64::from_i64(v).unwrap()
}

/// is IEEE binary64 arithmetic: results equal the host's bit
/// for bit, including the sign of zero (the host comparison `==` treats
/// −0 and +0 as equal, so bits are compared here).
#[test]
fn signed_zero_bits_match_ieee() {
    let pz = f(0);
    let nz = pz.negate();
    let host = |x: F64| x.to_bits();
    assert_eq!(host(pz), 0.0f64.to_bits());
    assert_eq!(host(nz), (-0.0f64).to_bits());
    let cases: [(F64, f64); 4] = [(f(3), 3.0), (f(-3), -3.0), (pz, 0.0), (nz, -0.0)];
    for &(a, ha) in &cases {
        for &(b, hb) in &cases {
            let ctx = format!("{ha} {hb}");
            assert_eq!(host(a.mul_rne(b).unwrap()), (ha * hb).to_bits(), "{ctx} *");
            assert_eq!(host(a.add_rne(b).unwrap()), (ha + hb).to_bits(), "{ctx} +");
            if hb != 0.0 {
                assert_eq!(host(a.div_rne(b).unwrap()), (ha / hb).to_bits(), "{ctx} /");
            }
        }
    }
}

/// Rounding that carries into a new power of two (2^54 − 1 → 2^54).
#[test]
fn rounding_carry_matches_ieee() {
    for v in [(1i64 << 54) - 1, -((1i64 << 54) - 1), (1i64 << 55) - 1] {
        assert_eq!(f(v).to_bits(), (v as f64).to_bits(), "{v}");
    }
}

/// A rounding carry leaves a normalized value: later operations see
/// 2^54, not twice it.
#[test]
fn rounding_carry_then_operate() {
    let v = (1i64 << 54) - 1;
    let x = f(v).mul_rne(F64::ONE).unwrap();
    assert_eq!(x.to_bits(), (v as f64).to_bits());
    let y = f(v).add_rne(f(1)).unwrap();
    assert_eq!(y.to_bits(), ((v as f64) + 1.0).to_bits());
}

/// The binary64 normal range ends at 2^−1022 and 2^1023 (both
/// representable).
#[test]
fn normal_range_ends() {
    let step = f(1 << 62);
    let mut small = F64::ONE;
    let mut large = F64::ONE;
    for _ in 0..16 {
        small = small.div_rne(step).unwrap();
        large = large.mul_rne(step).unwrap();
    }
    small = small.div_rne(f(1 << 30)).unwrap();
    large = large.mul_rne(f(1 << 31)).unwrap();
    assert_eq!(small.to_bits(), f64::MIN_POSITIVE.to_bits());
    assert_eq!(large.to_bits(), 2f64.powi(1023).to_bits());
}

/// Truncation of values far outside i32 (exponent > 10) is out of range,
/// never a wrapped small value.
#[test]
fn trunc_large_exponents_out_of_range() {
    let big = f(1 << 32).mul_rne(f(1 << 32)).unwrap(); // 2^64
    assert_eq!(big.trunc_i32(), Err(softfloat::OutOfRange));
    let big = f(1 << 40).mul_rne(f(1 << 40)).unwrap(); // 2^80
    assert_eq!(big.trunc_i32(), Err(softfloat::OutOfRange));
}

/// Q = 0 gives N = 0 (C = 0: x = 1, p′ = 1).
#[test]
fn nodrop_q_zero() {
    assert!(f(0).is_zero());
    assert!(!f(1).is_zero());
    assert_eq!(nodrop(5, 0, 2), Ok(0));
}

// -------------------------------------------------------- runtime

/// `Lv` − 3 < level ≤ `Lv`: a level-3 item is in `bow3` only.
#[test]
fn automatic_tc_level_window() {
    let mut s = src();
    let mut b = item("tbw", 10, 3);
    b.version = 0;
    s.items.push(b);
    let t = s.build().unwrap();
    let ids = |k: usize| -> Vec<u16> { t.tcs[k].entries.iter().map(|e| e.id).collect() };
    assert_eq!(ids(1), [1, 5], "bow3: sbw, tbw");
    assert_eq!(ids(2), [2], "bow6: hbw only");
}

/// `atol` values (sign, whitespace, i32 minimum) cut to u16, and
/// the keys `cr`, `cs`, `cg`.
#[test]
fn item_string_atol_and_keys() {
    let mut s = src();
    s.tcx = vec![tcx(
        "R",
        1,
        0,
        &[
            ("gld,mul=+512", 1),
            ("gld,cm=-1", 1),
            ("gld,cm=-2147483648", 1),
            ("gld,mul=   ,cm=3", 1),
            ("gld,cr=1,cs=2,cg=6", 1),
        ],
    )];
    let t = s.build().unwrap();
    let e = &t.tcs[33].entries;
    assert_eq!(e[0].row, 512);
    assert_eq!(e[1].mods[0], 0xFFFF);
    assert_eq!(e[2].mods[0], 0);
    assert_eq!((e[3].row, e[3].mods[0]), (0, 3));
    assert_eq!(e[4].mods, [0, 1, 2, 0, 0, 6]);
}

/// Item index order: weapons, armor, misc.
#[test]
fn item_list_order() {
    let mut w: Weapons = zeroed();
    w.code = code("w");
    let mut a: Armor = zeroed();
    a.code = code("a");
    let mut m: Misc = zeroed();
    m.code = code("m");
    let l = item_list(&[w], &[a], &[m]);
    let codes: Vec<[u8; 4]> = l.iter().map(|i| i.code).collect();
    assert_eq!(codes, [code("w"), code("a"), code("m")]);
}

/// The item type test: `type2` counts only when
/// nonzero, and then only when it is equivalent.
#[test]
fn item_type_test_type2() {
    let m = equiv(60, &[(0, 12)]);
    let mut it = item("x", 10, 1);
    assert!(!runtime::item_is_type(&m, &it, 12), "type2 0");
    it.type2 = 11;
    assert!(!runtime::item_is_type(&m, &it, 12), "type2 11");
    it.type2 = 12;
    assert!(runtime::item_is_type(&m, &it, 12));
}

#[test]
fn tc_set_emptiness() {
    assert!(!classes(vec![]).is_empty());
    let none = TreasureClasses {
        tcs: Vec::new(),
        group_offset: 0,
        chest: [None; 45],
        notes: Vec::new(),
    };
    assert!(none.is_empty());
}

/// No stepping for `lvl` ≤ 0 or group 0.
#[test]
fn get_steps_only_grouped_positive_level() {
    let mk = |group, level| {
        let mut t = tc(1, 0, &[(0, 1, 0)]);
        t.group = group;
        t.level = level;
        t
    };
    let t = classes(vec![mk(5, 0), mk(5, 0), mk(0, 0), mk(0, 0)]);
    assert_eq!(t.get(1, 0), Some(1));
    assert_eq!(t.get(1, 1), Some(2));
    assert_eq!(t.get(3, 5), Some(3));
}

// ----------------------------------------------------------- drops

/// The area level column per game mode and difficulty.
#[test]
fn area_level_columns() {
    let mut ls: Vec<Levels> = (0..3).map(|_| zeroed()).collect();
    let r = &mut ls[1];
    (r.monlvl1, r.monlvl2, r.monlvl3) = (11, 12, 13);
    (r.monlvl1ex, r.monlvl2ex, r.monlvl3ex) = (21, 22, 23);
    for (exp, d, want) in [
        (false, 0, 11),
        (false, 1, 12),
        (false, 2, 13),
        (true, 0, 21),
        (true, 1, 22),
        (true, 2, 23),
    ] {
        assert_eq!(area_level(&ls, 1, d, exp), want, "{exp} {d}");
    }
}

/// Lo 10, hi 30: s = 21 / 3 = 7; tier 0 below 17, 1 below 24.
#[test]
fn chest_tier_boundaries() {
    let mut ls: Vec<Levels> = (0..38).map(|_| zeroed()).collect();
    ls[2].monlvl1ex = 10;
    ls[37].monlvl1ex = 30;
    for (lvl, cur) in [(3, 16), (4, 17), (5, 23), (6, 24)] {
        ls[lvl].monlvl1ex = cur;
    }
    for (lvl, want) in [(3, 0), (4, 1), (5, 1), (6, 2)] {
        assert_eq!(chest_tier(&ls, 0, lvl, 0, true), Ok(want), "level {lvl}");
    }
}

// ---------------------------------------------------------- quality

fn quality_world() -> World {
    World::new(vec![], vec![item("q", 10, 30)])
}

/// `Class Specific` = itemtypes `class` < 7; an uber weapon (not
/// type 38) reads the uber row. The matching row here returns unique
/// without a draw; the other row draws.
#[test]
fn quality_row_class_and_uber() {
    let mut instant = ratio_row();
    (instant.unique, instant.uniquemin) = (0, 0);
    for (class, cs) in [(3u8, true), (7, false), (0xFF, false)] {
        let mut w = quality_world();
        w.itemtypes[10].class = class;
        let mut row = instant.clone();
        row.class_specific = 1;
        w.ratio = vec![ratio_row(), row];
        let mut seed = Seed::default();
        roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut seed).unwrap();
        assert_eq!(seed == Seed::default(), cs, "class {class}");
    }
    let mut row = instant;
    row.uber = 1;
    for ultra in [false, true] {
        let mut w = quality_world();
        w.equiv = equiv(60, &[(10, 45)]);
        if ultra {
            w.items[0].ultracode = w.items[0].code;
        } else {
            w.items[0].ubercode = w.items[0].code;
        }
        w.ratio = vec![ratio_row(), row.clone()];
        let mut seed = Seed::default();
        let q = roll_quality(&w.data(), 0, 50, 0, &[0; 6], &mut seed);
        assert_eq!(q, Ok(7), "ultra {ultra}");
        assert_eq!(seed, Seed::default(), "ultra {ultra}");
    }
}

/// `roll(chance)` < 128 → the step's result; 128 is not.
#[test]
fn quality_roll_threshold_128() {
    let mut w = quality_world();
    let mut r: Itemratio = zeroed();
    r.version = 1;
    (r.unique, r.uniquedivisor) = (8, 1); // b = 8 × 128 = 1024 at D = 0
    (r.setdivisor, r.raredivisor, r.magicdivisor) = (1, 1, 1);
    (r.hiqualitydivisor, r.normaldivisor) = (1, 1);
    w.ratio = vec![r];
    for (roll, want) in [(127, 7), (128, 5)] {
        let s = (0u32..)
            .find(|&x| Seed::new(x, 666).roll(1024) == roll)
            .unwrap();
        let mut seed = Seed::new(s, 666);
        let q = roll_quality(&w.data(), 0, 30, 0, &[0; 6], &mut seed).unwrap();
        assert_eq!(q, want, "roll {roll}");
    }
}

// ------------------------------------------------------------- walk

/// A TC index at the count is no TC.
#[test]
fn walk_tc_index_at_count() {
    let w = World::new(vec![tc(1, 0, &[(0, 1, 0)])], abcd());
    let a = args(w.tcs.tcs.len() as u16, 4);
    let r = walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        None,
        &a,
        &mut Sink::default(),
    );
    assert_eq!(r, Err(TreasureError::NoTc));
}

/// Without a list only `max` < 1 becomes 6; `max` 1 stays 1.
#[test]
fn walk_max_one_without_list() {
    let w = World::new(vec![tc(3, 0, &[(0, 1, 0)])], abcd());
    let a = WalkArgs {
        max: 1,
        ..args(1, 4)
    };
    let out = walk(
        &w.data(),
        &game(true, 1),
        &OBJECT,
        &mut Seed::default(),
        None,
        &a,
        &mut Sink::default(),
    )
    .unwrap();
    assert_eq!(out.len(), 1);
}

/// (lo′ & 0x3FF) < mod sets the flag; equal does not.
#[test]
fn drop_flag_threshold_equal() {
    // One pick (negative picks: no draw), forced quality: the two steps
    // are the drop-flag steps.
    let mut s = Seed::default();
    let a = (s.step() & 0x3FF) as u16;
    let b = (s.step() & 0x3FF) as u16;
    assert!(a > 0 && b > 0);
    for (m5, m6, want) in [(a, b, 0), (a + 1, b + 1, 0x14)] {
        let mut t = tc(-1, 0, &[(0, 1, 0)]);
        t.mods[4] = m5;
        t.mods[5] = m6;
        let w = World::new(vec![t], abcd());
        let mut sink = Sink::default();
        walk(
            &w.data(),
            &game(true, 1),
            &OBJECT,
            &mut Seed::default(),
            None,
            &args(1, 4),
            &mut sink,
        )
        .unwrap();
        assert_eq!(sink.reqs[0].drop_flags, want, "mods {m5} {m6}");
    }
}
