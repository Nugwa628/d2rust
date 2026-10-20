//! Skill description functions (`skilldesc.descdam` / `descatt`): the pure
//! formula parts that need no unit state: range text, the table bounds,
//! the kick / Inferno / Blaze / Smite sums and the attack-rating sum. The
//! stat-reading callers (weapon physical, elements, hand swaps) feed these
//! with already-read values.

/// Entries 1–24 of the `descdam` table are live.
pub const DESCDAM_LIVE: u16 = 24;
/// Entries 1–5 of the `descatt` table are live.
pub const DESCATT_LIVE: u16 = 5;

/// Whether `descdam` `n` selects a function (Edge case 1: ≥ 25 is a data
/// error, 0 is the null entry).
pub fn descdam_live(n: u16) -> bool {
    (1..=DESCDAM_LIVE).contains(&n)
}

/// Whether `descatt` `n` selects a function.
pub fn descatt_live(n: u16) -> bool {
    (1..=DESCATT_LIVE).contains(&n)
}

/// `x × s / 128` as the image computes it.
pub fn scale128(x: i32, s: i32) -> i32 {
    let p = x.wrapping_mul(s);
    p.wrapping_add(if p < 0 { 127 } else { 0 }) >> 7
}

/// `K` rounding.
fn kilo(v: i32) -> i32 {
    (v + 500) / 1000
}

/// Text of a range: max ≤ min → max:= min + 1, so a single value never prints
/// alone.
pub fn range_text(min: i32, max: i32) -> String {
    let max = if max <= min { min + 1 } else { max };
    range_fmt(min, max)
}

/// Text of a range without (the two-line draw).
pub fn range_text_two_line(min: i32, max: i32) -> String {
    if min == max && min < 10000 {
        format!("{min}")
    } else {
        range_fmt(min, max)
    }
}

fn range_fmt(min: i32, max: i32) -> String {
    if min >= 10000 {
        format!("{}K-{}K", kilo(min), kilo(max))
    } else if max >= 10000 {
        format!("{min}-{}K", kilo(max))
    } else {
        format!("{min}-{max}")
    }
}

/// Element color of an `EType`: 1 → 1, 2 → 9, 3 or 4 → 3, 5 → 2.
pub fn etype_color(etype: i32) -> Option<i32> {
    match etype {
        1 => Some(1),
        2 => Some(9),
        3 | 4 => Some(3),
        5 => Some(2),
        _ => None,
    }
}

/// Entry 2 (kick): `(MinDam << ((HitShift − 8) & 31)) + item_kickdamage`.
pub fn kick_value(min_dam: i32, hit_shift: i32, item_kick: i32) -> i32 {
    (min_dam << ((hit_shift - 8) & 31)) + item_kick
}

/// Entry 8 (inferno, arctic blast): `(A × m × 25 / B) >> 8`, a zero
/// `ddam` result counting as 1. Returns (min, max).
pub fn entry8(calc1: i32, calc2: i32, elem_min: i32, elem_max: i32) -> (i32, i32) {
    let a = if calc1 == 0 { 1 } else { calc1 };
    let b = if calc2 == 0 { 1 } else { calc2 };
    ((a * elem_min * 25 / b) >> 8, (a * elem_max * 25 / b) >> 8)
}

/// Entry 9 (blaze, fire wall, firestorm): the `× 75 >> 8` sum, plus the
/// weapon part already computed by the caller.
pub fn entry9(
    min_dam: i32,
    max_dam: i32,
    hit_shift: i32,
    elem_min: i32,
    elem_max: i32,
    weapon: (i32, i32),
) -> (i32, i32) {
    let sh = hit_shift & 31;
    (
        ((((min_dam << sh) + elem_min) * 75) >> 8) + weapon.0,
        ((((max_dam << sh) + elem_max) * 75) >> 8) + weapon.1,
    )
}

/// Entry 10 (smite): shield bytes `b` / `big_b`, bonus percent `p`
/// (clamped at −90 below −89), and stats 18 / 17. Returns (min, max).
pub fn entry10(b: i32, big_b: i32, p: i32, stat18: i32, stat17: i32) -> (i32, i32) {
    let p = if p < -89 { -90 } else { p };
    (
        (p * b + stat18) / 100 + b,
        (p * big_b + stat17) / 100 + big_b,
    )
}

/// Attack-rating sum of `AR` for the final value: base rating
/// `v` plus `v × P / 100`; `P` is the caller's percent sum.
pub fn ar_value(v: i32, p: i32) -> i32 {
    v + v.wrapping_mul(p) / 100
}

/// `AR` color: `attred` wins over `attblue`, which wins over the
/// progressive-to-hit color 3 and the base 0.
pub fn ar_color(progressive: bool, attblue: bool, attred: bool) -> i32 {
    let mut c = 0;
    if progressive {
        c = 3;
    }
    if attblue {
        c = 3;
    }
    if attred {
        c = 1;
    }
    c
}
