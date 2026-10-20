//! Experience on a kill and on death: the player gain, the distribution of a kill's
//! experience (with the party share), the death penalties and the corpse experience.
//!
//! The hireling's share of a kill is `crate::world::hirelings::level`; here it is the
//! seam [`ExpShare::hireling_share`]. Nothing in this file is random.
//!
//! The party share's float32 arithmetic (an integer quotient rounded to a float32, then
//! the level times that float32 truncated toward zero) is reproduced exactly in integers
//! ([`party_quotient`], [`party_share`]): the quotient is rounded to float32 (nearest,
//! ties to even) and the product is exact. The product is taken at full precision, which
//! is exact for every level and float32 quotient.

use super::{add_experience_at, level_factor, stat, VitalsTables, VitalsUnits};
use crate::combat::pct;
use crate::units::UnitType;

/// The experience cap of one gain.
pub const GAIN_CAP: i32 = 0x7F_FFFF;
/// Stat 85 `item_addexperience`.
pub const ADDEXPERIENCE: u16 = 85;
/// Stat 14 `gold`.
pub const GOLD: u16 = 14;
/// Stat 15 `goldbank`.
pub const GOLDBANK: u16 = 15;
/// Stat 175 `goldlost`.
pub const GOLDLOST: u16 = 175;
/// The party share's member limit (a 9th member is a fatal assert in `party_share`).
pub const MAX_PARTY: usize = 8;
/// Squared distance limit of a party member to the defender in the party share.
pub const PARTY_RANGE_SQ: u32 = 6400;

impl VitalsTables {
    /// The `ExpRatio` for level `L`: no table → 0; L < 1 → the `MaxLvl` row's `ExpRatio`;
    /// 1 ≤ L ≤ that row's class-0 value → level L's `ExpRatio` (row L + 1); above → 0.
    pub fn exp_ratio(&self, level: i32) -> i32 {
        let Some(head) = self.experience.first() else {
            return 0;
        };
        if level < 1 {
            return head.expratio as i32;
        }
        if level as u32 > head.amazon {
            return 0;
        }
        self.experience
            .get(level as usize + 1)
            .map_or(0, |r| r.expratio as i32)
    }

    /// Scales the gain `g` by the `ExpRatio` of the attacker's level `alvl`.
    pub fn apply_exp_ratio(&self, g: i32, alvl: i32) -> i32 {
        if g <= 0 {
            return g;
        }
        let r = self.exp_ratio(alvl);
        let s = self.exp_ratio(0);
        if (s as u32).wrapping_sub(1) >= 31 {
            return g;
        }
        let sh = (r >> s).wrapping_add(s) as u32 & 31;
        let limit = 0x7FFF_FFFFi32 >> sh;
        if g > limit {
            (g >> s).wrapping_mul(r)
        } else {
            r.wrapping_mul(g) >> s
        }
    }
}

/// The experience gain of one kill for a **player** gainer `u` (a player keeps the gain
/// as computed; the cap for non-player gainers is in the hireling code).
pub fn player_gain<W: VitalsUnits>(
    w: &W,
    t: &VitalsTables,
    e: i32,
    u: W::Unit,
    alvl: i32,
    dlvl: i32,
) -> i32 {
    // Cap the experience; a non-positive value gains 1.
    let e = if e > GAIN_CAP {
        GAIN_CAP
    } else if e <= 0 {
        return 1;
    } else {
        e
    };
    // A gainer at its class's maximum level gains nothing.
    if alvl >= t.max_level_in(w.rules(), w.class_id(u)) as i32 {
        return 0;
    }
    // Level factor, then the `ExpRatio` step, then the `item_addexperience` bonus.
    let mut g = level_factor(e, alvl, dlvl);
    g = t.apply_exp_ratio(g, alvl);
    let x = w.stat(u, ADDEXPERIENCE);
    if x != 0 {
        g = g.wrapping_add(pct(g, x, 100));
    }
    g
}

/// The world calls the distribution needs beside [`VitalsUnits`].
pub trait ExpShare: VitalsUnits {
    /// The credited player of a monster attacker: its owner, replaced by the owner of a
    /// flag-0x800 stat list on the attacker, then on the defender. `None` unless that unit
    /// is a player.
    fn credited_player(&self, attacker: Self::Unit, defender: Self::Unit) -> Option<Self::Unit>;
    /// Gives the hireling share of player `p`'s hireling for `attacker` killing
    /// `defender` with experience `e`.
    fn hireling_share(&mut self, p: Self::Unit, attacker: Self::Unit, defender: Self::Unit, e: i32);
    /// True when the player is in a party.
    fn in_party(&self, p: Self::Unit) -> bool;
    /// The party members that share the kill, in order: `p` alone when `p` has no room or
    /// no party; otherwise the party members on `p`'s level, alive, within range of the
    /// defender.
    fn party_members(&self, p: Self::Unit, defender: Self::Unit) -> Vec<Self::Unit>;
}

/// Distributes the experience of defender `d` killed by attacker `a`. The kill runs it
/// when the defender lacks unit flag 0x04000000.
pub fn distribute<W: ExpShare>(w: &mut W, t: &VitalsTables, a: W::Unit, d: W::Unit) {
    // Only players and monsters earn experience, and only for a worth-something defender.
    let a_ty = w.unit_type(a);
    if !matches!(a_ty, UnitType::Player | UnitType::Monster) {
        return;
    }
    let e = w.base_stat(d, stat::EXPERIENCE);
    if e <= 0 {
        return;
    }
    // The credited player: the attacker itself, or the player behind a monster attacker.
    let p = if a_ty == UnitType::Player {
        Some(a)
    } else {
        w.credited_player(a, d)
    };
    let Some(p) = p.filter(|&p| w.unit_type(p) == UnitType::Player) else {
        return;
    };
    // The hireling's share is handed out first.
    let dl = w.base_stat(d, stat::LEVEL);
    w.hireling_share(p, a, d, e);
    // A party splits the experience; a lone player takes the full gain.
    let pl = w.base_stat(p, stat::LEVEL);
    if w.in_party(p) {
        party_share(w, t, p, d, e, pl, dl);
    } else {
        let g = player_gain(w, t, e, p, pl, dl);
        add_experience_at(w, t, p, pl, g);
    }
}

/// Splits the kill's experience `e` among the party members, each gaining by their own level.
pub fn party_share<W: ExpShare>(
    w: &mut W,
    t: &VitalsTables,
    p: W::Unit,
    d: W::Unit,
    e: i32,
    pl: i32,
    dl: i32,
) {
    let members = w.party_members(p, d);
    assert!(
        members.len() <= MAX_PARTY,
        "party share: a 9th member (assert 0xF25)"
    );
    // Total experience is boosted per extra member, then divided by the sum of levels.
    let levels: Vec<i32> = members.iter().map(|&m| w.stat(m, stat::LEVEL)).collect();
    let n = members.len() as i32;
    let s = levels.iter().fold(0i32, |a, &l| a.wrapping_add(l));
    if n <= 0 || s <= 0 {
        return;
    }
    if n == 1 {
        let g = player_gain(w, t, e, p, pl, dl);
        add_experience_at(w, t, p, pl, g);
        return;
    }
    let total = e.wrapping_add(n.wrapping_sub(1).wrapping_mul(e).wrapping_mul(89) / 256);
    let q = party_quotient(total, s);
    for (&m, &l) in members.iter().zip(&levels) {
        let share = party_member_share(l, q);
        let g = player_gain(w, t, share, m, l, dl);
        add_experience_at(w, t, m, l, g);
    }
}

/// A float32 value as `sign · mantissa · 2^-shift` (mantissa < 2^24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct F32 {
    /// The sign.
    pub negative: bool,
    /// The 24-bit mantissa (0 for zero).
    pub mantissa: u64,
    /// The power of two the mantissa is divided by.
    pub shift: i32,
}

/// `t / s` stored to a float32: round to nearest, ties to even. `s` > 0.
pub fn party_quotient(t: i32, s: i32) -> F32 {
    let a = u64::from(t.unsigned_abs());
    let b = u64::from(s.unsigned_abs());
    let negative = (t < 0) != (s < 0);
    if a == 0 {
        return F32 {
            negative,
            mantissa: 0,
            shift: 0,
        };
    }
    // Choose `shift` so 2^23 ≤ a · 2^shift / b < 2^24.
    let mut shift = 23 - (63 - a.leading_zeros() as i32) + (63 - b.leading_zeros() as i32);
    let scaled = |k: i32| -> (u128, u128) {
        let (num, den) = (u128::from(a), u128::from(b));
        if k >= 0 {
            (num << k, den)
        } else {
            (num, den << -k)
        }
    };
    loop {
        let (num, den) = scaled(shift);
        let m = num / den;
        if m < 1 << 23 {
            shift += 1;
        } else if m >= 1 << 24 {
            shift -= 1;
        } else {
            break;
        }
    }
    let (num, den) = scaled(shift);
    let mut m = num / den;
    let r = num % den;
    if 2 * r > den || (2 * r == den && m & 1 == 1) {
        m += 1;
    }
    if m == 1 << 24 {
        m = 1 << 23;
        shift -= 1;
    }
    F32 {
        negative,
        mantissa: m as u64,
        shift,
    }
}

/// `level × q` (an integer times a float32) truncated toward zero; a value outside i32
/// gives i32::MIN (0x80000000).
pub fn party_member_share(level: i32, q: F32) -> i32 {
    let mut p = i128::from(level) * i128::from(q.mantissa);
    if q.negative {
        p = -p;
    }
    let v = if q.shift >= 0 {
        p / (1i128 << q.shift.min(126))
    } else {
        p << (-q.shift).min(64)
    };
    i32::try_from(v).unwrap_or(i32::MIN)
}

/// The gold penalty on death as a plan: the amount to drop, the new gold / goldbank
/// (`None` = unchanged) and `goldlost`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoldPenalty {
    /// The gold to drop on the ground, if any.
    pub drop: Option<i32>,
    /// The new carried gold, if it changes.
    pub gold: Option<i32>,
    /// The new banked gold, if it changes.
    pub goldbank: Option<i32>,
    /// The amount recorded as lost (stat 175).
    pub goldlost: i32,
}

/// Computes the gold penalty for a death at level `l` with carried gold `gi` and banked
/// gold `gs` (totals), `pvp`, the game type `game_type` and the stash / gold limits.
pub fn gold_penalty(
    l: i32,
    gi: i32,
    gs: i32,
    pvp: bool,
    game_type: u8,
    stash_limit: i32,
    gold_limit: i32,
) -> GoldPenalty {
    let total = gi.wrapping_add(gs);
    let mut q = l.min(20).wrapping_mul(total) / 100;
    let keep = total.wrapping_sub(q);
    let base = l.wrapping_mul(500);
    let mut out = GoldPenalty {
        drop: None,
        gold: None,
        goldbank: None,
        goldlost: 0,
    };
    let bank = |v: i32| if v < 0 || v > stash_limit { 0 } else { v };
    let gold = |v: i32| if v < 0 || v > gold_limit { 0 } else { v };
    if game_type == 3 {
        if keep < base {
            q = total.wrapping_sub(base).max(0);
        }
        q = q.min(gi);
    }
    if pvp {
        if q > gi {
            out.goldbank = Some(bank(gs.wrapping_sub(q.wrapping_sub(gi))));
            out.gold = Some(gold(q));
        }
        out.drop = Some(q);
    } else if q <= gi {
        out.drop = Some(gi.wrapping_sub(q));
        out.gold = Some(0);
    } else {
        out.goldbank = Some(bank(gs.wrapping_sub(q.wrapping_sub(gi))));
        out.gold = Some(0);
    }
    out.goldlost = q.max(0);
    out
}

/// The experience penalty on death for player `u` with the `DeathExpPenalty` `penalty` of
/// the game's difficulty (the caller skips it for a player-side killer). Returns the loss
/// stored on the client for the corpse, `None` when nothing happens; stat 13 is set.
pub fn death_experience<W: VitalsUnits>(
    w: &mut W,
    t: &VitalsTables,
    u: W::Unit,
    penalty: u32,
) -> Option<i32> {
    let c = w.class_id(u);
    let l = w.stat(u, stat::LEVEL);
    if l <= 1 {
        return None;
    }
    let lo = t.threshold_in(w.rules(), c, (l - 1) as u32);
    let hi = t.threshold_in(w.rules(), c, l as u32);
    let x = w.stat(u, stat::EXPERIENCE);
    let mut loss = penalty.wrapping_mul(hi.wrapping_sub(lo)) / 100;
    if loss == 0 {
        return None;
    }
    let mut new = (x as u32).wrapping_sub(loss);
    if new <= lo {
        loss = x.wrapping_sub(lo as i32).max(0) as u32;
        new = lo.wrapping_add(1);
    }
    let loss = loss as i32;
    assert!(loss >= 0, "experience loss < 0");
    w.set_base_stat(u, stat::EXPERIENCE, new as i32);
    Some(loss)
}

/// The corpse's stat 13 from the client's stored loss `v`: 75 percent of it, as
/// `pct(v, 75, 100)` (v > 0x100000 → (v / 100) × 75, else v × 75 / 100; 32-bit, toward
/// zero). The caller then clears the stored loss.
pub fn corpse_experience(v: i32) -> i32 {
    pct(v, 75, 100)
}

/// Corpse pickup by its own player `p` (`own` = the corpse's owner GUID is `p`'s): the
/// corpse's experience goes back through the add at `p`'s base level. Returns the amount.
pub fn corpse_pickup<W: VitalsUnits>(
    w: &mut W,
    t: &VitalsTables,
    p: W::Unit,
    corpse: W::Unit,
    own: bool,
) -> i32 {
    if !own {
        return 0;
    }
    let x = w.stat(corpse, stat::EXPERIENCE);
    if x == 0 {
        return 0;
    }
    w.set_base_stat(corpse, stat::EXPERIENCE, 0);
    let l0 = w.base_stat(p, stat::LEVEL);
    add_experience_at(w, t, p, l0, x);
    x
}
