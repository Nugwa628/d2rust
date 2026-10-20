//! Pure arithmetic of the type init: the percentage helper, player-count
//! bonus, monster level, monlvl base values, hpregen and classic scaling.

use d2_data::tables::{Levels, Monlvl, Monstats};

use super::{s16, GameInfo};

/// V × p / d, signed 32-bit, truncating, with the
/// base game's overflow branches.
pub fn pct(v: i32, p: i32, d: i32) -> i32 {
    if d == 0 {
        return 0;
    }
    let wide = || (i64::from(v) * i64::from(p) / i64::from(d)) as i32;
    if v > 0x10_0000 {
        if d <= v >> 4 {
            (v / d).wrapping_mul(p)
        } else {
            wide()
        }
    } else if p > 0x1_0000 {
        if d <= p >> 4 {
            (p / d).wrapping_mul(v)
        } else {
            wide()
        }
    } else {
        v.wrapping_mul(p).wrapping_div(d)
    }
}

/// The player-count result (the scaling inputs for the monster's stats).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerBonus {
    /// n (at least 1).
    pub players: i32,
    pub hp: i32,
    pub xp: i32,
}

/// Tables (identical).
const BONUS: [i32; 9] = [0, 0, 50, 100, 150, 200, 250, 300, 350];

/// (`align` = monstats `Align`).
pub fn player_bonus(info: &GameInfo, align: u8) -> PlayerBonus {
    if align != 0 {
        return PlayerBonus {
            players: 1,
            hp: 0,
            xp: 0,
        };
    }
    let mut n = info.players;
    if matches!(info.game_type, 1..=3) {
        n = n.max(info.players_x);
    }
    let n = n.max(1);
    let (hp, xp) = match usize::try_from(n) {
        Ok(i) if i < BONUS.len() => (BONUS[i], BONUS[i]),
        _ => ((n - 2) * 50, 10 * n + 260),
    };
    PlayerBonus { players: n, hp, xp }
}

/// The area level:
/// levels `MonLvl<d+1>Ex` or `MonLvl<d+1>`; 1 when the level id is ≤ 0
/// or ≥ the row count, or d ≥ 3.
pub fn area_level(levels: &[Levels], level_id: i32, d: usize, expansion: bool) -> i32 {
    let Some(r) = usize::try_from(level_id)
        .ok()
        .filter(|&i| i > 0)
        .and_then(|i| levels.get(i))
    else {
        return 1;
    };
    let v = match (d, expansion) {
        (0, true) => r.monlvl1ex,
        (1, true) => r.monlvl2ex,
        (2, true) => r.monlvl3ex,
        (0, false) => r.monlvl1,
        (1, false) => r.monlvl2,
        (2, false) => r.monlvl3,
        _ => return 1,
    };
    i32::from(v)
}

/// The monster level for difficulty `d` (already 0 for hirelings).
pub fn monster_level(
    m: &Monstats,
    levels: &[Levels],
    info: &GameInfo,
    d: usize,
    level_id: i32,
) -> i32 {
    match d {
        0 => s16(m.level),
        _ if info.expansion && !m.noratio && !m.boss => {
            area_level(levels, level_id, d, info.expansion)
        }
        1 => s16(m.level_n),
        _ => s16(m.level_h),
    }
}

/// The outputs of the monster-level row read with flags 7.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LevelStats {
    pub min_hp: i32,
    pub max_hp: i32,
    pub ac: i32,
    pub xp: i32,
}

/// HP, AC and XP.
/// A negative level (or an empty monlvl) gives nothing (all 0).
pub fn stats_by_level(
    m: &Monstats,
    monlvl: &[Monlvl],
    l_flag: bool,
    d: usize,
    level: i32,
) -> LevelStats {
    let d = d.min(2);
    let pick = |a: [u16; 3]| s16(a[d]);
    let min_hp = pick([m.minhp, m.minhp_n, m.minhp_h]);
    let max_hp = pick([m.maxhp, m.maxhp_n, m.maxhp_h]);
    let ac = pick([m.ac, m.ac_n, m.ac_h]);
    let xp = pick([m.exp, m.exp_n, m.exp_h]);
    if level < 0 || monlvl.is_empty() {
        return LevelStats::default();
    }
    if m.noratio {
        return LevelStats {
            min_hp,
            max_hp,
            ac,
            xp,
        };
    }
    let row = &monlvl[(level as usize).min(monlvl.len() - 1)];
    let col = |a: [u32; 3], l: [u32; 3]| (if l_flag { l } else { a })[d] as i32;
    let hp = col(
        [row.hp, row.hp_n, row.hp_h],
        [row.l_hp, row.l_hp_n, row.l_hp_h],
    );
    let lac = col(
        [row.ac, row.ac_n, row.ac_h],
        [row.l_ac, row.l_ac_n, row.l_ac_h],
    );
    let lxp = col(
        [row.xp, row.xp_n, row.xp_h],
        [row.l_xp, row.l_xp_n, row.l_xp_h],
    );
    LevelStats {
        min_hp: pct(hp, min_hp, 100),
        max_hp: pct(hp, max_hp, 100),
        ac: pct(lac, ac, 100),
        xp: pct(lxp, xp, 100),
    }
}

/// (flag 8): the A1
/// damage (min, max): `A1MinD` / `A1MaxD` for d with `noRatio`, else
/// pct(monlvl `DM` / `L-DM` for d, the monstats value, 100). A negative
/// level (or an empty monlvl) gives (0, 0).
pub fn a1_damage(
    m: &Monstats,
    monlvl: &[Monlvl],
    l_flag: bool,
    d: usize,
    level: i32,
) -> (i32, i32) {
    let d = d.min(2);
    let min = s16([m.a1mind, m.a1mind_n, m.a1mind_h][d]);
    let max = s16([m.a1maxd, m.a1maxd_n, m.a1maxd_h][d]);
    if level < 0 || monlvl.is_empty() {
        return (0, 0);
    }
    if m.noratio {
        return (min, max);
    }
    let row = &monlvl[(level as usize).min(monlvl.len() - 1)];
    let dm = (if l_flag {
        [row.l_dm, row.l_dm_n, row.l_dm_h]
    } else {
        [row.dm, row.dm_n, row.dm_h]
    })[d] as i32;
    (pct(dm, min, 100), pct(dm, max, 100))
}

/// monlvl `DM` / `L-DM` for d at `level` clamped to 1..rows−1 (umod 9,
/// umod 36).
pub fn monlvl_dm(monlvl: &[Monlvl], l_flag: bool, d: usize, level: i32) -> i32 {
    if monlvl.len() < 2 {
        return 0;
    }
    let i = level.clamp(1, monlvl.len() as i32 - 1) as usize;
    let r = &monlvl[i];
    let a = if l_flag {
        [r.l_dm, r.l_dm_n, r.l_dm_h]
    } else {
        [r.dm, r.dm_n, r.dm_h]
    };
    a[d.min(2)] as i32
}

/// Hpregen from the ×256 maxhp and `DamageRegen`. `None`
/// when `DamageRegen` is outside 0..0xFFF (the base game's fatal error).
pub fn hp_regen(maxhp: i32, regen: u32) -> Option<i32> {
    if regen > 0xFFF {
        return None;
    }
    let r = regen as i32;
    if r == 0 {
        return Some(0);
    }
    Some(if maxhp > i32::MAX / r {
        (maxhp >> 12) * r
    } else {
        maxhp.wrapping_mul(r) >> 12
    })
}

/// Table: (a, b) for maxhp, armorclass, experience,
/// by d − 1.
const CLASSIC: [[(i32, i32); 3]; 2] = [[(1, 2), (10, 12), (10, 17)], [(1, 2), (10, 12), (10, 26)]];

/// `Some((maxhp, ac, xp, level))` when the scaling
/// applies (not expansion, d > 0, `Align` ≠ 1). Hitpoints are left alone
/// (edge case 11).
pub fn classic_scaling(
    info: &GameInfo,
    d: usize,
    m: &Monstats,
    maxhp: i32,
    ac: i32,
    xp: i32,
) -> Option<(i32, i32, i32, i32)> {
    if info.expansion || d == 0 || m.align == 1 {
        return None;
    }
    let t = CLASSIC[d.min(2) - 1];
    Some((
        pct(maxhp, t[0].0, t[0].1),
        pct(ac, t[1].0, t[1].1),
        pct(xp, t[2].0, t[2].1),
        s16(m.level) + 25 * d as i32,
    ))
}
