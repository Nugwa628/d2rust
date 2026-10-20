//! Stats: identity and keys, values, the itemstatcost columns
//! the simulation reads, MulDiv, the op table ([`ops`]), by-time and the
//! derived-stat helpers. Lists, the readers on them and evaluation:
//! [`lists`]; state bits: [`states`].
//!
//! Table data comes from the d2-data records after the load fix-ups
//! (the fix-ups build the op tables at +0x51…+0xF5):
//! [`StatTable::from_fixed`]. Nothing here is a hard-coded copy of a
//! table.

pub mod lists;
pub mod ops;
pub mod states;

#[cfg(test)]
mod gap_tests;
#[cfg(test)]
mod mutant_tests;
#[cfg(test)]
mod prop_tests;
#[cfg(any(test, feature = "bench-fixtures"))]
#[cfg_attr(not(test), allow(unused, dead_code))]
pub(crate) mod tests;

use d2_data::bin::BinTable;
use d2_data::tables::{decode_all, Charstats, Itemstatcost, Monstats, Skills, WrongTable};

pub use lists::{CallbackEvent, ListId, StatHost, StatLists, ValueCallback};
pub use states::StateTable;

/// Stat ids the simulation names (base-game itemstatcost rows).
pub mod stat {
    pub const STRENGTH: u16 = 0;
    pub const ENERGY: u16 = 1;
    pub const DEXTERITY: u16 = 2;
    pub const VITALITY: u16 = 3;
    pub const HITPOINTS: u16 = 6;
    pub const MAXHP: u16 = 7;
    pub const MANA: u16 = 8;
    pub const MAXMANA: u16 = 9;
    pub const STAMINA: u16 = 10;
    pub const MAXSTAMINA: u16 = 11;
    pub const LEVEL: u16 = 12;
    pub const MANARECOVERY: u16 = 26;
    pub const MANARECOVERYBONUS: u16 = 27;
    pub const STAMINARECOVERYBONUS: u16 = 28;
    pub const HPREGEN: u16 = 74;
    pub const ITEM_AURA: u16 = 151;
    pub const LAST_SENT_HP_PCT: u16 = 352;
}

/// "No stat" in op base / op stat cells and the op-base lists.
pub const NO_STAT: u16 = 0xFFFF;

/// Key := (s << 16) + layer, compared as signed i32.
pub const fn key(stat: u16, layer: u16) -> i32 {
    (((stat as u32) << 16) | layer as u32) as i32
}

/// Stat id of a key.
pub const fn key_stat(key: i32) -> u16 {
    ((key as u32) >> 16) as u16
}

/// Layer of a key.
pub const fn key_layer(key: i32) -> u16 {
    key as u32 as u16
}

/// One entry of a stat's op-target table (+0xDE, 6 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpEntry {
    /// Op base stat (0xFFFF: none).
    pub base: u16,
    /// Source stat (the stat whose op targets this one).
    pub source: u16,
    pub op: u8,
    pub param: u8,
}

/// Unused op entry: reads base 0.
pub const UNUSED_ENTRY: OpEntry = OpEntry {
    base: 0,
    source: 0,
    op: 0,
    param: 0,
};

/// The itemstatcost facts of one stat the simulation reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatInfo {
    pub damagerelated: bool,
    pub fmin: bool,
    pub minaccr: i32,
    pub fcallback: bool,
    pub saved: bool,
    pub valshift: u8,
    pub keepzero: bool,
    pub op: u8,
    pub op_param: u8,
    pub op_base: u16,
    pub op_stats: [u16; 3],
    /// `itemevent1` read as i16 (+0x48).
    pub itemevent1: i16,
    /// `itemevent2` (+0x4A), `itemeventfunc1` (+0x4C), `itemeventfunc2`
    /// (+0x4E) as i16.
    pub itemevent2: i16,
    pub itemeventfunc1: i16,
    pub itemeventfunc2: i16,
    /// A51: an op base or has op stats.
    pub a51: bool,
    /// A52: the target of some op.
    pub a52: bool,
    /// A53: op 4 or 5 with a valid base.
    pub a53: bool,
    /// deps(T), the +0x5E list up to the first 0xFFFF (64 at most).
    pub deps: Vec<u16>,
    /// entries(T), the +0xDE table up to the first op 0 (16 at most).
    pub entries: Vec<OpEntry>,
}

impl StatInfo {
    /// Entry `j` of entries(T), or [`UNUSED_ENTRY`].
    pub fn entry(&self, j: usize) -> OpEntry {
        self.entries.get(j).copied().unwrap_or(UNUSED_ENTRY)
    }
}

/// The itemstatcost table as the simulation reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatTable {
    stats: Vec<StatInfo>,
}

fn u16_at(r: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([r[o], r[o + 1]])
}

impl StatTable {
    /// From the fixed-up itemstatcost table (`d2_data::fixup::apply`, or
    /// `d2_data::fixup::records::stat_ops` on a copy): typed columns
    /// from [`Itemstatcost`], the load-time op data from the record bytes
    /// +0x51…+0xF5 that the fix-up wrote.
    pub fn from_fixed(t: &BinTable) -> Result<Self, WrongTable> {
        let typed = decode_all::<Itemstatcost>(t)?;
        let stats = typed
            .iter()
            .zip(t.iter())
            .map(|(c, r)| {
                let deps = (0..64)
                    .map(|k| u16_at(r, 0x5E + 2 * k))
                    .take_while(|&s| usize::from(s) < t.count)
                    .collect();
                let entries = (0..16)
                    .map(|m| 0xDE + 6 * m)
                    .map(|e| OpEntry {
                        base: u16_at(r, e),
                        source: u16_at(r, e + 2),
                        op: r[e + 4],
                        param: r[e + 5],
                    })
                    .take_while(|e| e.op != 0)
                    .collect();
                StatInfo {
                    damagerelated: c.damagerelated,
                    fmin: c.fmin,
                    minaccr: c.minaccr as i32,
                    fcallback: c.fcallback,
                    saved: c.saved,
                    valshift: c.valshift,
                    keepzero: c.keepzero != 0,
                    op: c.op,
                    op_param: c.op_param,
                    op_base: c.op_base,
                    op_stats: [c.op_stat1, c.op_stat2, c.op_stat3],
                    itemevent1: c.itemevent1 as i16,
                    itemevent2: c.itemevent2 as i16,
                    itemeventfunc1: c.itemeventfunc1 as i16,
                    itemeventfunc2: c.itemeventfunc2 as i16,
                    a51: r[0x51] != 0,
                    a52: r[0x52] != 0,
                    a53: r[0x53] != 0,
                    deps,
                    entries,
                }
            })
            .collect();
        Ok(Self { stats })
    }

    /// Number of stats n.
    pub fn len(&self) -> usize {
        self.stats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stats.is_empty()
    }

    /// The stat, when 0 ≤ s < n.
    pub fn get(&self, s: u16) -> Option<&StatInfo> {
        self.stats.get(usize::from(s))
    }

    /// ValShift of a stat; 0 for an invalid id.
    pub fn valshift(&self, s: u16) -> u8 {
        self.get(s).map_or(0, |i| i.valshift)
    }
}

/// The charstats columns the stat code reads (Inputs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClassStats {
    /// `ManaRegen` (+0x3A).
    pub mana_regen: u8,
    /// `LifePerVitality` (+0x46), quarter points.
    pub life_per_vitality: u8,
    /// `StaminaPerVitality` (+0x47).
    pub stamina_per_vitality: u8,
    /// `ManaPerMagic` (+0x48).
    pub mana_per_magic: u8,
}

impl From<&Charstats> for ClassStats {
    fn from(c: &Charstats) -> Self {
        Self {
            mana_regen: c.manaregen,
            life_per_vitality: c.lifepervitality,
            stamina_per_vitality: c.staminapervitality,
            mana_per_magic: c.manapermagic,
        }
    }
}

/// Every table the stat code reads, read once at game creation.
#[derive(Clone, Debug, Default)]
pub struct StatData {
    pub stats: StatTable,
    /// charstats rows by class.
    pub classes: Vec<ClassStats>,
    pub states: StateTable,
    /// monstats `DamageRegen` (+0x1A0) by class.
    pub damage_regen: Vec<u32>,
    /// skills `aurastate` (+0x80) by skill id (events 5, 9).
    pub aurastate: Vec<u16>,
    /// Precision of the x87 significand in the max-life rescale.
    ///
    /// TODO(open question 1): the FPU precision control at
    /// the call is not confirmed (24, 53 or 64 bits). 53 is the Win32
    /// process default; settle with a recording of max-life changes.
    pub rescale_precision: u32,
}

impl StatData {
    /// From the fixed-up itemstatcost, charstats, states (with the
    /// runtime state maps), monstats and skills tables.
    pub fn new(
        itemstatcost: &BinTable,
        charstats: &BinTable,
        states: StateTable,
        monstats: &BinTable,
        skills: &BinTable,
    ) -> Result<Self, WrongTable> {
        Ok(Self {
            stats: StatTable::from_fixed(itemstatcost)?,
            classes: decode_all::<Charstats>(charstats)?
                .iter()
                .map(ClassStats::from)
                .collect(),
            states,
            damage_regen: decode_all::<Monstats>(monstats)?
                .iter()
                .map(|m| m.damageregen)
                .collect(),
            aurastate: decode_all::<Skills>(skills)?
                .iter()
                .map(|s| s.aurastate)
                .collect(),
            rescale_precision: DEFAULT_RESCALE_PRECISION,
        })
    }
}

/// See [`StatData::rescale_precision`].
pub const DEFAULT_RESCALE_PRECISION: u32 = 53;

/// MulDiv: all i32, signed compares, divisions
/// truncating toward zero.
pub fn muldiv(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return 0;
    }
    let wide = |a: i32, b: i32| (i64::from(a) * i64::from(b) / i64::from(c)) as i32;
    if a > 0x10_0000 {
        if c <= a >> 4 {
            return (a.wrapping_div(c)).wrapping_mul(b);
        }
        return wide(a, b);
    }
    if b > 0x1_0000 {
        if c <= b >> 4 {
            return (b.wrapping_div(c)).wrapping_mul(a);
        }
        return wide(a, b);
    }
    a.wrapping_mul(b).wrapping_div(c)
}

/// ByTime: `v` the packed source value, `t` the act's
/// base time.
pub fn by_time(v: i32, t: i32) -> i32 {
    let p = v & 3;
    let lo = ((v >> 2) & 0x3FF) - 256;
    let hi = ((v >> 12) & 0x3FF) - 256;
    let d = t.wrapping_sub(90 * p).wrapping_abs();
    let mut a = (d.wrapping_add(7) / 15).wrapping_mul(15);
    if a <= 0 {
        a = 0;
    } else if a >= 359 {
        a = 1;
    } else if a > 180 {
        a = 360 - a;
    }
    hi.wrapping_sub((hi.wrapping_sub(lo)).wrapping_mul(a) / 180)
}

/// The act base time t of the by-time stats from the environment record: dword +0x08
/// divided by dword +0x28, 0 when +0x28 is 0.
pub fn act_base_time(time: i32, period_length: i32) -> i32 {
    if period_length == 0 {
        0
    } else {
        time.wrapping_div(period_length)
    }
}

/// Life fraction: `hp` = total(6), `max` = max life,
/// both raw (1/256 points).
pub fn life_fraction(hp: i32, max: i32) -> i32 {
    let h = hp >> 8;
    let m = max >> 8;
    if m > 0 && h < m {
        (h << 7) / m
    } else {
        128
    }
}

/// Whether a new life fraction is sent: |f − stat 352 low byte|
/// > 4, in 32-bit arithmetic (no overflow panic outside 0–128).
pub fn fraction_changed(f: i32, last_sent: i32) -> bool {
    f.wrapping_sub(last_sent & 0xFF).wrapping_abs() > 4
}

/// The max-rescale of the stat lists without floating point: the
/// x87 quotient `new / o`, its product with `c`, each rounded to
/// `precision` significand bits (round to nearest even), stored as a
/// float32 (24 bits, nearest even) and truncated toward zero.
///
/// The truncation result is the low 32 bits of the 64-bit integer
/// conversion. TODO(open question 1): precision, and the
/// conversion of a result outside i32 (not seen in the base game data).
pub fn x87_rescale(new: i32, o: i32, c: i32, precision: u32) -> i32 {
    if new == 0 || c == 0 || o == 0 {
        return 0;
    }
    let negative = (new < 0) ^ (o < 0) ^ (c < 0);
    let (n, d, c) = (
        u128::from(new.unsigned_abs()),
        u128::from(o.unsigned_abs()),
        u128::from(c.unsigned_abs()),
    );
    let q = Soft::round(n, d, precision);
    let p = Soft::round_shifted(q.m * c, 1, q.e, precision);
    let f = Soft::round_shifted(p.m, 1, p.e, 24);
    let magnitude = if f.e >= 0 { f.m << f.e } else { f.m >> -f.e };
    let v = magnitude as u64 as i64;
    (if negative { -v } else { v }) as i32
}

/// A positive soft-float m · 2^e.
#[derive(Clone, Copy, Debug)]
struct Soft {
    m: u128,
    e: i32,
}

impl Soft {
    /// num / den rounded to `p` significand bits, nearest even.
    fn round(num: u128, den: u128, p: u32) -> Soft {
        Self::round_shifted(num, den, 0, p)
    }

    /// (num / den) · 2^e0 rounded to `p` bits, nearest even.
    fn round_shifted(num: u128, den: u128, e0: i32, p: u32) -> Soft {
        // Choose e with 2^(p−1) ≤ num / den / 2^e < 2^p.
        let bits = |x: u128| 128 - x.leading_zeros() as i32;
        let mut e = bits(num) - bits(den) - p as i32;
        loop {
            let (n, d) = scale(num, den, e);
            let m = n / d;
            if m >= 1u128 << p {
                e += 1;
            } else if m < 1u128 << (p - 1) {
                e -= 1;
            } else {
                let r = n % d;
                let mut m = m;
                if 2 * r > d || (2 * r == d && m & 1 == 1) {
                    m += 1;
                }
                if m == 1u128 << p {
                    return Soft {
                        m: m >> 1,
                        e: e + 1 + e0,
                    };
                }
                return Soft { m, e: e + e0 };
            }
        }
    }
}

/// num / den · 2^−e as a fraction (n, d).
fn scale(num: u128, den: u128, e: i32) -> (u128, u128) {
    if e >= 0 {
        (num, den << e)
    } else {
        (num << -e, den)
    }
}
