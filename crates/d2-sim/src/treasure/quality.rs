//! Drop quality: the ratio row and the quality ladder.
//! [`LADDER`] is `treasure-quality.tsv`; the test `ladder_matches_tsv`
//! fails when the two disagree.

use d2_data::tables::Itemratio;

use super::runtime::{item_is_type, TYPE_TPOT};
use super::{TreasureData, TreasureError};
use crate::rng::Seed;

/// itemtypes records 45 `weap` and 50 `armo`.
const TYPE_WEAP: usize = 45;
const TYPE_ARMO: usize = 50;

/// Item quality codes.
pub const QUALITY_INFERIOR: u8 = 1;
pub const QUALITY_NORMAL: u8 = 2;
pub const QUALITY_SUPERIOR: u8 = 3;
pub const QUALITY_MAGIC: u8 = 4;
pub const QUALITY_SET: u8 = 5;
pub const QUALITY_RARE: u8 = 6;
pub const QUALITY_UNIQUE: u8 = 7;

/// A ladder step's gate (TSV `gate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Always,
    /// itemtypes `rare` set.
    Rare,
    /// itemtypes `magic` set.
    Magic,
}

/// The itemratio column triple a step reads (TSV `ratio`, `divisor`,
/// `min`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ratio {
    Unique,
    Set,
    Rare,
    Magic,
    HiQuality,
    Normal,
}

impl Ratio {
    /// (ratio, divisor, min) of the row; `min` is `None` for the steps
    /// without one.
    fn values(self, r: &Itemratio) -> (i32, i32, Option<i32>) {
        let (a, b, c) = match self {
            Ratio::Unique => (r.unique, r.uniquedivisor, Some(r.uniquemin)),
            Ratio::Set => (r.set, r.setdivisor, Some(r.setmin)),
            Ratio::Rare => (r.rare, r.raredivisor, Some(r.raremin)),
            Ratio::Magic => (r.magic, r.magicdivisor, Some(r.magicmin)),
            Ratio::HiQuality => (r.hiquality, r.hiqualitydivisor, None),
            Ratio::Normal => (r.normal, r.normaldivisor, None),
        };
        (a as i32, b as i32, c.map(|m| m as i32))
    }

    fn columns(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Ratio::Unique => ("Unique", "UniqueDivisor", "UniqueMin"),
            Ratio::Set => ("Set", "SetDivisor", "SetMin"),
            Ratio::Rare => ("Rare", "RareDivisor", "RareMin"),
            Ratio::Magic => ("Magic", "MagicDivisor", "MagicMin"),
            Ratio::HiQuality => ("HiQuality", "HiQualityDivisor", "-"),
            Ratio::Normal => ("Normal", "NormalDivisor", "-"),
        }
    }
}

/// How magic find scales a step (TSV `mf_factor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MfFactor {
    /// `f` = `M`·`k` / (`M` + `k`) + 100 when `M` + 100 > 110.
    Factor(i32),
    /// `f` = `M` + 100.
    Linear,
    /// MF ignored.
    None,
}

/// One ladder step (one TSV row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub result: u8,
    pub gate: Gate,
    /// `None`: the step returns `result` without a draw (TSV
    /// `draw_when` = `never`).
    pub ratio: Option<Ratio>,
    pub mf: MfFactor,
    /// Index into the slot mods (magic 0, rare 1, set 2, unique 3).
    pub tc_slot: Option<usize>,
}

/// `treasure-quality.tsv`, in order. Steps 1–5 are the MF steps
/// (skipped when `M` ≤ −100); step 8 is the fall-through.
pub const LADDER: [Step; 8] = [
    Step {
        result: QUALITY_UNIQUE,
        gate: Gate::Always,
        ratio: Some(Ratio::Unique),
        mf: MfFactor::Factor(250),
        tc_slot: Some(3),
    },
    Step {
        result: QUALITY_SET,
        gate: Gate::Always,
        ratio: Some(Ratio::Set),
        mf: MfFactor::Factor(500),
        tc_slot: Some(2),
    },
    Step {
        result: QUALITY_RARE,
        gate: Gate::Rare,
        ratio: Some(Ratio::Rare),
        mf: MfFactor::Factor(600),
        tc_slot: Some(1),
    },
    Step {
        result: QUALITY_MAGIC,
        gate: Gate::Magic,
        ratio: None,
        mf: MfFactor::None,
        tc_slot: None,
    },
    Step {
        result: QUALITY_MAGIC,
        gate: Gate::Always,
        ratio: Some(Ratio::Magic),
        mf: MfFactor::Linear,
        tc_slot: Some(0),
    },
    Step {
        result: QUALITY_SUPERIOR,
        gate: Gate::Always,
        ratio: Some(Ratio::HiQuality),
        mf: MfFactor::None,
        tc_slot: None,
    },
    Step {
        result: QUALITY_NORMAL,
        gate: Gate::Always,
        ratio: Some(Ratio::Normal),
        mf: MfFactor::None,
        tc_slot: None,
    },
    Step {
        result: QUALITY_INFERIOR,
        gate: Gate::Always,
        ratio: None,
        mf: MfFactor::None,
        tc_slot: None,
    },
];

/// First step run when `M` ≤ −100 (the superior step).
const NO_MF_START: usize = 5;

/// The ratio row: `Class Specific` and `Uber`
/// equal to the flags (as 0/1), `Version` ≤ 100, the highest `Version`,
/// the later record on a tie.
pub fn ratio_row(rows: &[Itemratio], class_specific: bool, uber: bool) -> Option<&Itemratio> {
    let mut best: Option<&Itemratio> = None;
    for r in rows {
        let ok = r.class_specific == u8::from(class_specific)
            && r.uber == u8::from(uber)
            && r.version <= 100;
        if ok && best.is_none_or(|b| r.version >= b.version) {
            best = Some(r);
        }
    }
    best
}

/// `D / divisor` toward zero; a zero divisor is an error (the base game's
/// `idiv` would fault).
fn div(a: i32, b: i32) -> Result<i32, TreasureError> {
    if b == 0 {
        return Err(TreasureError::ZeroDivisor);
    }
    Ok(a.wrapping_div(b))
}

/// The MF divisor `f` of a step, or `None`
/// when MF leaves `b` unchanged.
pub fn mf_divisor(mf: MfFactor, m: i32) -> Option<i32> {
    if m == 0 {
        return None;
    }
    let linear = m.wrapping_add(100);
    match mf {
        MfFactor::None => None,
        MfFactor::Linear => Some(linear),
        MfFactor::Factor(k) if linear > 110 => Some(
            m.wrapping_mul(k)
                .wrapping_div(m.wrapping_add(k))
                .wrapping_add(100),
        ),
        MfFactor::Factor(_) => Some(linear),
    }
}

/// The chance of one ladder step with a ratio: `b` from
/// the ratio and `D`, scaled by MF, raised to the min, reduced by the
/// slot mod.
pub fn step_chance(
    step: &Step,
    ratio: Ratio,
    row: &Itemratio,
    d: i32,
    mf: i32,
    mods: &[u16; 6],
) -> Result<i32, TreasureError> {
    let (base, divisor, min) = ratio.values(row);
    let mut b = base.wrapping_sub(div(d, divisor)?).wrapping_mul(128);
    if let Some(f) = mf_divisor(step.mf, mf) {
        b = div(b.wrapping_mul(100), f)?;
    }
    if let Some(min) = min {
        b = b.max(min);
    }
    Ok(match step.tc_slot {
        Some(s) => b.wrapping_sub(b.wrapping_mul(i32::from(mods[s])) / 1024),
        None => b,
    })
}

/// Drop quality. `l` is the walk's `L`, `mf` the recipient's magic
/// find `M`, `mods` the slot mods. Draws on `seed` (U's seed).
pub fn roll_quality(
    data: &TreasureData,
    item_id: u16,
    l: i32,
    mf: i32,
    mods: &[u16; 6],
    seed: &mut Seed,
) -> Result<u8, TreasureError> {
    let Some(item) = data.items.get(usize::from(item_id)) else {
        return Ok(0);
    };
    let Some(ty) = data.itemtypes.get(usize::from(item.type_)) else {
        return Ok(0);
    };
    if ty.normal != 0 {
        return Ok(QUALITY_NORMAL);
    }
    if item.unique != 0 {
        return Ok(QUALITY_UNIQUE);
    }
    // TODO(treasure OQ-quest-magic): reads "itemtypes `magic`
    // and items `quest` → 7"; implemented as the conjunction.
    if ty.magic != 0 && item.quest != 0 {
        return Ok(QUALITY_UNIQUE);
    }
    let class_specific = ty.class < 7;
    let uber = (item_is_type(data.equiv, item, TYPE_WEAP)
        || item_is_type(data.equiv, item, TYPE_ARMO))
        && (item.code == item.ubercode || item.code == item.ultracode)
        && usize::from(item.type_) != TYPE_TPOT
        && item.quest == 0;
    let row = ratio_row(data.itemratio, class_specific, uber).ok_or(TreasureError::NoRatioRow)?;
    let d = l.wrapping_sub(i32::from(item.level));
    let start = if mf <= -100 { NO_MF_START } else { 0 };
    for step in &LADDER[start..] {
        let gated = match step.gate {
            Gate::Always => true,
            Gate::Rare => ty.rare != 0,
            Gate::Magic => ty.magic != 0,
        };
        if !gated {
            continue;
        }
        let Some(ratio) = step.ratio else {
            return Ok(step.result);
        };
        let chance = step_chance(step, ratio, row, d, mf, mods)?;
        if chance <= 0 || seed.roll(chance) < 128 {
            return Ok(step.result);
        }
    }
    unreachable!("the last ladder step always returns")
}

/// The TSV form of one step (columns of `treasure-quality.tsv` after
/// `step`), for the table check.
pub fn step_tsv(s: &Step) -> String {
    let gate = match s.gate {
        Gate::Always => "always",
        Gate::Rare => "itemtypes.rare",
        Gate::Magic => "itemtypes.magic",
    };
    let (r, d, m) = s.ratio.map_or(("-", "-", "-"), Ratio::columns);
    let mf = match (s.ratio, s.mf) {
        (None, _) => "-".to_string(),
        (_, MfFactor::Factor(k)) => k.to_string(),
        (_, MfFactor::Linear) => "linear".to_string(),
        (_, MfFactor::None) => "none".to_string(),
    };
    let slot = match s.tc_slot {
        Some(0) => "magic",
        Some(1) => "rare",
        Some(2) => "set",
        Some(3) => "unique",
        _ => "-",
    };
    let draw = if s.ratio.is_some() {
        "chance>0"
    } else {
        "never"
    };
    format!(
        "{}\t{gate}\t{r}\t{d}\t{m}\t{mf}\t{slot}\t{draw}\tresult",
        s.result
    )
}
