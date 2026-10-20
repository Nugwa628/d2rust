//! Animation rate and frame bonus: the pure
//! arithmetic of the speed (+0x4C) and the sequence speed (+0x3C).
//!
//! The facts the routine reads from the world (the draw identity after
//! the disguise substitution, the AnimData speed, the item values, the
//! used skill, the attack weapon, the path's velocity-modifier test)
//! arrive resolved in [`RateInput`]; this module
//! owns the step order and the formulas. The rate draws nothing.

/// Constant `c` of the `animstat` rows of the table (every base-game
/// row has flag 1).
pub const E_C: [i32; 5] = [120, 120, 120, 120, 150];
/// Row 0: fast attack rate (stat 93).
pub const E_ATTACK: usize = 0;
/// Row 1: fast get-hit rate (stat 99).
pub const E_GETHIT: usize = 1;
/// Row 2: fast cast rate (stat 105).
pub const E_CAST: usize = 2;
/// Row 3: fast block rate (stat 102).
pub const E_BLOCK: usize = 3;
/// Row 4: fast move velocity (stat 96).
pub const E_MOVE: usize = 4;

/// Maximum speed.
pub const SPEED_MAX: i32 = 0x7FFF;

/// E(r): the diminished item bonus of `animstat` row `row` for the
/// unit's raw item/skill value `v` (`c·v / (c + v)`, i32 truncating, when
/// `v ≠ 0`).
pub fn diminished(row: usize, v: i32) -> i32 {
    if v == 0 {
        return 0;
    }
    let c = E_C[row];
    c.wrapping_mul(v).wrapping_div(c.wrapping_add(v))
}

/// D(s, f): `s·f / 100` with the product taken as i32 and divided as
/// unsigned 32-bit, then 0 if ≤ 0 (signed) and at most 0x7FFF. A
/// negative `f` therefore gives 0x7FFF.
pub fn d(s: i32, f: i32) -> i32 {
    let q = (s.wrapping_mul(f) as u32 / 100) as i32;
    if q <= 0 {
        0
    } else {
        q.min(SPEED_MAX)
    }
}

/// Which of the mode table's columns (V-skill, V, A-skill, A, fixed) are
/// non-zero for a mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModeRow {
    pub v_skill: bool,
    pub v: bool,
    pub a_skill: bool,
    pub a: bool,
    pub fixed: bool,
}

/// The mode table row of draw type `t` (0 player, 1 monster), class `c`
/// and mode `m`. Modes beyond the table (players 0–19, monsters 0–15)
/// have an all-zero row.
pub fn mode_row(t: u8, c: u32, m: u32) -> ModeRow {
    let mut r = ModeRow::default();
    match t {
        0 if m <= 19 => {
            r.v = matches!(m, 2 | 3 | 6);
            r.a = matches!(m, 7 | 8 | 11 | 12);
            r.v_skill = matches!(m, 13..=16 | 18);
            r.a_skill = r.v_skill;
            r.fixed = matches!(m, 0 | 17);
        }
        1 if m <= 15 => {
            r.v = matches!(m, 2 | 15) || (c < 410 && matches!(m, 8..=11));
            r.a = matches!(m, 4 | 5);
            r.v_skill = matches!(m, 8..=11 | 14);
            r.a_skill = r.v_skill;
            r.fixed = matches!(m, 0 | 12);
        }
        _ => {}
    }
    r
}

/// The facts the routine reads (see the module docs).
#[derive(Clone, Copy, Debug, Default)]
pub struct RateInput {
    /// Step 1: the unit exists and is of type 0–3 (not an item, not ≥ 5).
    pub applies: bool,
    /// Steps 7 and 8 assert (fatal) for object and missile units.
    pub is_object_or_missile: bool,
    /// Draw type after the disguise substitution (0 player, 1 monster).
    pub t: u8,
    pub c: u32,
    pub m: u32,
    /// s: the AnimData speed (+0x0C).
    pub s: i32,
    /// Raw item/skill values of stats 93, 99, 105, 102, 96 (E rows 0–4).
    pub item: [i32; 5],
    /// Unit total of stat 67 `velocitypercent`.
    pub velocitypercent: i32,
    /// Unit total of stat 68 `attackrate`.
    pub attackrate: i32,
    /// Unit total of stat 69 `other_animrate`.
    pub other_animrate: i32,
    /// The used skill's `seqtrans`, when a skill is in use.
    pub used_seqtrans: Option<i32>,
    /// The used skill's row has `UseAttackRate` (flag bit 19).
    pub use_attack_rate: bool,
    /// The unit has state 101 `holyshield`.
    pub holyshield: bool,
    /// The unit has a path (+0x2C).
    pub has_path: bool,
    /// w of the velocity read `(T, C, M)` (knockback / velocity speed).
    pub w: i32,
    /// The velocity-modifier test.
    pub velocity_mode: bool,
    /// Dual wield: stat 68 totals of the items at body locations 4 and 5
    /// when the unit can dual-wield and both are usable `weap` items.
    pub dual: Option<(i32, i32)>,
    /// U itself is a player in mode 18 (+0x10).
    pub player_mode_18: bool,
    /// When U has a state of group 38.
    pub were_speed: Option<i32>,
}

/// What the routine writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    /// Nothing is written.
    Nothing,
    /// Speed, sequence speed and the path velocity.
    Set {
        /// +0x4C.
        speed: i32,
        /// +0x3C, written in the cast and attack cases.
        seq_speed: Option<i32>,
        velocity: Velocity,
    },
}

/// The path velocity half (the path code owns the rules).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Velocity {
    /// Unchanged.
    Keep,
    /// Knockback: 0x1000.
    Fixed(i32),
    /// Base velocity · p / 100.
    Percent(i32),
}

/// The fatal assertions of steps 7 and 8.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RateError {
    #[error("animation rate is invalid for object and missile units")]
    ObjectOrMissile,
}

/// (first match wins).
pub fn anim_rate(i: &RateInput) -> Result<Rate, RateError> {
    // Step 1.
    if !i.applies {
        return Ok(Rate::Nothing);
    }
    let (t, m, s) = (i.t, i.m, i.s);
    let e = |row: usize| diminished(row, i.item[row]);
    let row = mode_row(t, i.c, m);
    let set = |speed: i32, seq: Option<i32>, velocity: Velocity| {
        Ok(Rate::Set {
            speed,
            seq_speed: seq,
            velocity,
        })
    };
    // Step 3: cast.
    if (t == 0 && m == 10)
        || (t == 0 && m == 18 && i.used_seqtrans == Some(10))
        || (t == 1 && m == 7)
    {
        let v = d(s, (100 + e(E_CAST)).min(175));
        return set(v, Some(v), Velocity::Keep);
    }
    // Step 4: block.
    if (t == 0 && m == 9) || (t == 1 && m == 6) {
        let f = if i.holyshield { 100 } else { 50 };
        return set(d(s, f + e(E_BLOCK)).max(1), None, Velocity::Keep);
    }
    // Step 5: get hit.
    if (t == 0 && m == 4) || (t == 1 && m == 3) {
        return set(d(s, 50 + e(E_GETHIT)), None, Velocity::Keep);
    }
    // Step 6: knockback.
    if (t == 0 && m == 19) || (t == 1 && m == 13) {
        return set(i.w.clamp(0, SPEED_MAX), None, Velocity::Fixed(0x1000));
    }
    // Step 7: velocity modes.
    if i.velocity_mode {
        if i.is_object_or_missile {
            return Err(RateError::ObjectOrMissile);
        }
        if !i.has_path {
            return Ok(Rate::Nothing);
        }
        let p = (e(E_MOVE) + i.velocitypercent).max(25);
        let w = i.w.wrapping_mul(p).wrapping_div(100);
        let w = if w <= 0 { 0 } else { w.min(SPEED_MAX) };
        return set(w, None, Velocity::Percent(p));
    }
    // Step 8: attack modes.
    if row.a || (row.a_skill && i.use_attack_rate) {
        if i.is_object_or_missile {
            return Err(RateError::ObjectOrMissile);
        }
        let mut v = e(E_ATTACK) + i.attackrate;
        if let Some((a4, a5)) = i.dual {
            v += (a4 + a5) / 2 - a4;
        }
        if i.player_mode_18 {
            v -= 30;
        }
        let v = v.clamp(15, 175);
        let b = i.were_speed.filter(|&b| b != 0).unwrap_or(s);
        let r = d(b, v);
        return set(r, Some(r), Velocity::Keep);
    }
    // Step 9: fixed.
    if row.fixed {
        return set(s.clamp(0, SPEED_MAX), None, Velocity::Keep);
    }
    // Step 10.
    set(d(s, i.other_animrate.clamp(15, 175)), None, Velocity::Keep)
}

/// Player mode (0…19) → monster mode.
pub const PLAYER_TO_MONSTER: [u32; 20] = [
    0, 1, 2, 15, 3, 1, 2, 4, 5, 6, 7, 4, 11, 8, 9, 10, 11, 12, 14, 13,
];
/// Monster mode (0…15) → player mode.
pub const MONSTER_TO_PLAYER: [u32; 16] = [0, 1, 2, 4, 7, 8, 9, 10, 13, 14, 15, 16, 17, 19, 18, 3];

/// The monster-mode fallback of the player → monster map:
/// WL, GH, A1 → NU; A2 → A1; BL → GH;
/// SC → A1; S1 → NU; S2, S3, S4 → S1; DD, KB, SQ → NU; RN → WL; DT and
/// anything else → NU.
pub fn mode_fallback(m: u32) -> u32 {
    match m {
        5 => 4,
        6 => 3,
        7 => 4,
        9..=11 => 8,
        15 => 2,
        _ => 1,
    }
}

/// Frame bonus table (12 type classes × 7 player classes):
/// 1 for type class 0 and 2 for type classes 2–6 on the Amazon (0) and
/// Sorceress (1); everything else, and type class 12 (past the table), 0.
pub fn frame_bonus_table(type_class: u32, player_class: u32) -> i32 {
    if player_class > 1 || type_class > 11 {
        return 0;
    }
    match type_class {
        0 => 1,
        2..=6 => 2,
        _ => 0,
    }
}

/// Draw type `t`, player class `c`, mode `m`;
/// `dual_capable` is the dual-wield test of U; `weapon_type_class` the attack
/// weapon's type class (`None` without a weapon: 0).
pub fn frame_bonus(
    t: u8,
    c: u32,
    m: u32,
    dual_capable: bool,
    weapon_type_class: Option<u32>,
) -> i32 {
    if t != 0 {
        return 0;
    }
    if matches!(m, 7 | 8) || (matches!(m, 15 | 16) && dual_capable) {
        return frame_bonus_table(weapon_type_class.unwrap_or(0), c);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> RateInput {
        RateInput {
            applies: true,
            t: 0,
            s: 256,
            velocitypercent: 100,
            attackrate: 100,
            other_animrate: 100,
            w: 213,
            ..RateInput::default()
        }
    }

    fn set(speed: i32, seq_speed: Option<i32>, velocity: Velocity) -> Result<Rate, RateError> {
        Ok(Rate::Set {
            speed,
            seq_speed,
            velocity,
        })
    }

    fn speed(i: &RateInput) -> i32 {
        match anim_rate(i) {
            Ok(Rate::Set { speed, .. }) => speed,
            r => panic!("{r:?}"),
        }
    }

    fn seq(i: &RateInput) -> Option<i32> {
        match anim_rate(i) {
            Ok(Rate::Set { seq_speed, .. }) => seq_speed,
            r => panic!("{r:?}"),
        }
    }

    #[test]
    fn formulas_d_and_diminished() {
        assert_eq!(d(256, 100), 256);
        assert_eq!(d(256, 50), 128);
        assert_eq!(d(0, 100), 0);
        assert_eq!(d(100_000, 100), 0x7FFF);
        // A negative f gives 0x7FFF (unsigned divide).
        assert_eq!(d(256, -10), 0x7FFF);
        assert_eq!(diminished(0, 0), 0);
        assert_eq!(diminished(0, 60), 120 * 60 / 180);
        assert_eq!(diminished(4, 50), 150 * 50 / 200);
        assert_eq!(diminished(2, -30), 120 * -30 / 90);
        let mut i = base();
        i.applies = false;
        assert_eq!(anim_rate(&i), Ok(Rate::Nothing));
    }

    #[test]
    fn cast_block_and_get_hit() {
        let mut i = base();
        i.m = 10;
        i.item[E_CAST] = 60; // E = 40
        let v = d(256, 140);
        assert_eq!(anim_rate(&i), set(v, Some(v), Velocity::Keep));
        i.item[E_CAST] = 10_000;
        assert_eq!(speed(&i), d(256, 175));
        let mut q = base();
        q.m = 18;
        q.used_seqtrans = Some(10);
        assert!(seq(&q).is_some());
        q.used_seqtrans = Some(9);
        assert!(seq(&q).is_none());
        let mut mo = base();
        mo.t = 1;
        mo.m = 7;
        assert!(seq(&mo).is_some());
        // Block: 50, 100 under holy shield, at least 1.
        let mut b = base();
        b.m = 9;
        assert_eq!(anim_rate(&b), set(128, None, Velocity::Keep));
        b.holyshield = true;
        assert_eq!(speed(&b), 256);
        b.s = 0;
        assert_eq!(speed(&b), 1);
        // Get hit: D(s, 50 + E(1)).
        let mut g = base();
        g.m = 4;
        g.item[E_GETHIT] = 120; // E = 60
        assert_eq!(speed(&g), d(256, 110));
        g.t = 1;
        g.m = 3;
        assert_eq!(speed(&g), d(256, 110));
    }

    #[test]
    fn knockback_and_velocity_modes() {
        let mut k = base();
        k.m = 19;
        assert_eq!(anim_rate(&k), set(213, None, Velocity::Fixed(0x1000)));
        k.w = 100_000;
        assert_eq!(speed(&k), 0x7FFF);
        k.w = -5;
        assert_eq!(speed(&k), 0);
        // Velocity mode: p = max(E(4) + total(67), 25).
        let mut v = base();
        v.m = 2;
        v.velocity_mode = true;
        v.has_path = true;
        v.item[E_MOVE] = 50; // E = 37
        let p = 37 + 100;
        assert_eq!(
            anim_rate(&v),
            set(213 * p / 100, None, Velocity::Percent(p))
        );
        v.velocitypercent = -500;
        v.item[E_MOVE] = 0;
        assert_eq!(
            anim_rate(&v),
            set(213 * 25 / 100, None, Velocity::Percent(25))
        );
        // No path: nothing at all; an object asserts.
        v.has_path = false;
        assert_eq!(anim_rate(&v), Ok(Rate::Nothing));
        v.is_object_or_missile = true;
        assert_eq!(anim_rate(&v), Err(RateError::ObjectOrMissile));
    }

    #[test]
    fn attack_fixed_and_other() {
        let mut a = base();
        a.m = 7;
        a.attackrate = 120;
        a.item[E_ATTACK] = 60; // E = 40
        let v = d(256, 160);
        assert_eq!(anim_rate(&a), set(v, Some(v), Velocity::Keep));
        // Dual wield: v += (a4 + a5)/2 - a4; mode 18: v -= 30; clamp.
        a.dual = Some((10, 31));
        assert_eq!(speed(&a), d(256, 160 + (41 / 2 - 10)));
        a.dual = None;
        a.player_mode_18 = true;
        assert_eq!(speed(&a), d(256, 130));
        a.attackrate = -1000;
        a.item[E_ATTACK] = 0;
        assert_eq!(speed(&a), d(256, 15));
        // The were-form speed replaces s when non-zero.
        a.were_speed = Some(100);
        assert_eq!(speed(&a), d(100, 15));
        a.were_speed = Some(0);
        assert_eq!(speed(&a), d(256, 15));
        // Skill modes need UseAttackRate.
        let mut sk = base();
        sk.m = 13;
        assert!(seq(&sk).is_none());
        sk.use_attack_rate = true;
        assert!(seq(&sk).is_some());
        // Fixed: s clamped.
        let mut f = base();
        f.m = 0;
        f.s = 40_000;
        assert_eq!(anim_rate(&f), set(0x7FFF, None, Velocity::Keep));
        f.m = 17;
        f.s = -3;
        assert_eq!(speed(&f), 0);
        // Otherwise: D(s, clamp(total(69), 15, 175)).
        let mut o = base();
        o.m = 1;
        o.other_animrate = 300;
        assert_eq!(speed(&o), d(256, 175));
        o.other_animrate = 0;
        assert_eq!(speed(&o), d(256, 15));
    }

    #[test]
    fn mode_rows_follow_the_1_14d_tables() {
        assert!(mode_row(0, 0, 2).v && mode_row(0, 0, 12).a && mode_row(0, 0, 17).fixed);
        assert!(mode_row(0, 0, 18).a_skill && mode_row(0, 0, 18).v_skill);
        assert!(!mode_row(0, 0, 20).fixed);
        assert!(mode_row(1, 100, 9).v && !mode_row(1, 410, 9).v);
        assert!(mode_row(1, 410, 9).v_skill);
        assert!(mode_row(1, 0, 4).a && mode_row(1, 0, 12).fixed && !mode_row(1, 0, 16).fixed);
    }

    #[test]
    fn frame_bonus_table_and_modes() {
        assert_eq!(frame_bonus(0, 0, 7, false, None), 1);
        assert_eq!(frame_bonus(0, 1, 8, false, Some(0)), 1);
        assert_eq!(frame_bonus(0, 0, 8, false, Some(2)), 2);
        assert_eq!(frame_bonus(0, 1, 7, false, Some(6)), 2);
        assert_eq!(frame_bonus(0, 0, 7, false, Some(7)), 0);
        assert_eq!(frame_bonus(0, 2, 7, false, Some(2)), 0);
        // Type class 12 reads past the table.
        assert_eq!(frame_bonus(0, 0, 7, false, Some(12)), 0);
        // Modes 15/16 need dual-wield capability; others 0; monsters 0.
        assert_eq!(frame_bonus(0, 0, 15, false, Some(2)), 0);
        assert_eq!(frame_bonus(0, 0, 15, true, Some(2)), 2);
        assert_eq!(frame_bonus(0, 0, 9, true, Some(2)), 0);
        assert_eq!(frame_bonus(1, 0, 7, true, Some(2)), 0);
    }
}
