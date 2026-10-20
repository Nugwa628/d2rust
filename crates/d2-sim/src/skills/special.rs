//! The special-value tables: what each `skillcalc` index (jump table
//! 73 entries) and `misscalc` index
//! returns. The machine-readable source is `tables/skills/skillcalc.tsv`
//! and `misscalc.tsv`; [`check_skillcalc`] / [`check_misscalc`] compare
//! these tables with them row by row: each row's `code`
//! and `returns` columns must equal [`SkillSpecial::code`] /
//! [`SkillSpecial::returns`] exactly.

use d2_data::tables::{Missiles, Skills};

/// `tables/skills/skillcalc.tsv`.
pub const SKILLCALC_TSV: &str = include_str!("../../../../tables/skills/skillcalc.tsv");
/// `tables/skills/misscalc.tsv`.
pub const MISSCALC_TSV: &str = include_str!("../../../../tables/skills/misscalc.tsv");

/// A skills formula field a calc-backed special value evaluates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SField {
    AuraLen,
    Calc(u8),
    AuraRange,
    AuraStat(u8),
    Passive(u8),
    PetMax,
    SkPoints,
}

impl SField {
    /// The field's value in a record (an offset into `skillscode`, or
    /// 0xFFFFFFFF).
    pub fn of(self, r: &Skills) -> u32 {
        match self {
            SField::AuraLen => r.auralencalc,
            SField::Calc(1) => r.calc1,
            SField::Calc(2) => r.calc2,
            SField::Calc(3) => r.calc3,
            SField::Calc(_) => r.calc4,
            SField::AuraRange => r.aurarangecalc,
            SField::AuraStat(1) => r.aurastatcalc1,
            SField::AuraStat(2) => r.aurastatcalc2,
            SField::AuraStat(3) => r.aurastatcalc3,
            SField::AuraStat(4) => r.aurastatcalc4,
            SField::AuraStat(5) => r.aurastatcalc5,
            SField::AuraStat(_) => r.aurastatcalc6,
            SField::Passive(1) => r.passivecalc1,
            SField::Passive(2) => r.passivecalc2,
            SField::Passive(3) => r.passivecalc3,
            SField::Passive(4) => r.passivecalc4,
            SField::Passive(_) => r.passivecalc5,
            SField::PetMax => r.petmax,
            SField::SkPoints => r.skpoints,
        }
    }

    fn name(self) -> String {
        match self {
            SField::AuraLen => "auralencalc".into(),
            SField::Calc(n) => format!("calc{n}"),
            SField::AuraRange => "aurarangecalc".into(),
            SField::AuraStat(n) => format!("aurastatcalc{n}"),
            SField::Passive(n) => format!("passivecalc{n}"),
            SField::PetMax => "petmax".into(),
            SField::SkPoints => "skpoints".into(),
        }
    }
}

/// What one `skillcalc` index returns (`skillcalc.tsv` `returns`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSpecial {
    /// `lvl>0 ? ParamX+(lvl-1)*ParamY : 0`.
    Ln(u8, u8),
    /// `DM(lvl,ParamX,ParamY)`, guarded by `lvl>0` when the flag is set.
    Dm(u8, u8, bool),
    /// `ParamN`.
    Par(u8),
    Lvl,
    /// `elem_min(mastery m)`, `>> 8` when `shift`.
    ElemMin {
        mastery: bool,
        shift: bool,
    },
    ElemMax {
        mastery: bool,
        shift: bool,
    },
    ElemLen,
    ToHit,
    Mana,
    Mps,
    /// `mastery(type t)`.
    Mastery(u8),
    /// `lvl>0 ? miss_elem_min|max(descmissile{slot+1}) [>> 8] : 0`.
    MissElem {
        slot: u8,
        max: bool,
        shift: bool,
    },
    MissElemLen {
        slot: u8,
    },
    /// `Range + lvl*LevRange of descmissile{slot+1}`.
    MissRange {
        slot: u8,
    },
    ULvl,
    BLvl,
    Usmc,
    Eval(SField),
}

use SkillSpecial as S;

const fn emn(mastery: bool, shift: bool) -> SkillSpecial {
    S::ElemMin { mastery, shift }
}
const fn emx(mastery: bool, shift: bool) -> SkillSpecial {
    S::ElemMax { mastery, shift }
}
const fn me(slot: u8, max: bool, shift: bool) -> SkillSpecial {
    S::MissElem { slot, max, shift }
}

/// The 73 skills special values, by index, with their 4-byte codes.
pub const SKILL_SPECIALS: [SkillSpecial; 73] = [
    S::Ln(1, 2),
    S::Dm(1, 2, true),
    S::Ln(3, 4),
    S::Dm(3, 4, true),
    S::Ln(5, 6),
    S::Dm(5, 6, true),
    S::Ln(7, 8),
    S::Dm(7, 8, false),
    S::Par(1),
    S::Par(2),
    S::Par(3),
    S::Par(4),
    S::Par(5),
    S::Par(6),
    S::Par(7),
    S::Par(8),
    S::Lvl,
    emn(false, true),
    emx(false, true),
    S::ElemLen,
    S::ToHit,
    S::Mana,
    S::Mps,
    S::Mastery(0),
    S::Mastery(1),
    S::Mastery(2),
    me(0, false, true),
    me(0, true, true),
    S::MissElemLen { slot: 0 },
    me(1, false, true),
    me(1, true, true),
    S::MissElemLen { slot: 1 },
    me(2, false, true),
    me(2, true, true),
    S::MissElemLen { slot: 2 },
    S::MissRange { slot: 0 },
    S::MissRange { slot: 1 },
    S::MissRange { slot: 2 },
    emn(false, false),
    emx(false, false),
    S::ULvl,
    S::BLvl,
    S::Usmc,
    me(0, false, false),
    me(0, true, false),
    // m2eo / m2ey read descmissile1, me3o / me3y descmissile2 (Edge case 2).
    me(0, false, false),
    me(0, true, false),
    me(1, false, false),
    me(1, true, false),
    emn(true, true),
    emx(true, true),
    S::ElemLen,
    emn(true, false),
    emx(true, false),
    S::Eval(SField::AuraLen),
    S::Eval(SField::Calc(1)),
    S::Eval(SField::Calc(2)),
    S::Eval(SField::Calc(3)),
    S::Eval(SField::Calc(4)),
    S::Eval(SField::AuraRange),
    S::Eval(SField::AuraStat(1)),
    S::Eval(SField::AuraStat(2)),
    S::Eval(SField::AuraStat(3)),
    S::Eval(SField::AuraStat(4)),
    S::Eval(SField::AuraStat(5)),
    S::Eval(SField::AuraStat(6)),
    S::Eval(SField::Passive(1)),
    S::Eval(SField::Passive(2)),
    S::Eval(SField::Passive(3)),
    S::Eval(SField::Passive(4)),
    S::Eval(SField::Passive(5)),
    S::Eval(SField::PetMax),
    S::Eval(SField::SkPoints),
];

/// The 4-byte codes of [`SKILL_SPECIALS`].
pub const SKILLCALC_CODES: [&str; 73] = [
    "ln12", "dm12", "ln34", "dm34", "ln56", "dm56", "ln78", "dm78", "par1", "par2", "par3", "par4",
    "par5", "par6", "par7", "par8", "lvl", "edmn", "edmx", "edln", "toht", "mana", "mps", "math",
    "madm", "macr", "m1en", "m1ex", "m1el", "m2en", "m2ex", "m2el", "m3en", "m3ex", "m3el", "m1rn",
    "m2rn", "m3rn", "edns", "edxs", "ulvl", "blvl", "usmc", "m1eo", "m1ey", "m2eo", "m2ey", "me3o",
    "me3y", "enma", "exma", "edma", "enms", "exms", "len", "clc1", "clc2", "clc3", "clc4", "rng",
    "ast1", "ast2", "ast3", "ast4", "ast5", "ast6", "pst1", "pst2", "pst3", "pst4", "pst5", "pets",
    "skpt",
];

impl SkillSpecial {
    /// The rule in `skillcalc.tsv` notation (the `returns` column).
    pub fn returns(self) -> String {
        let g = |s: String| format!("lvl>0 ? {s} : 0");
        let shr = |s: String, shift: bool| if shift { format!("{s} >> 8") } else { s };
        match self {
            S::Ln(p, q) => g(format!("Param{p}+(lvl-1)*Param{q}")),
            S::Dm(p, q, guard) => {
                let s = format!("DM(lvl,Param{p},Param{q})");
                if guard {
                    g(s)
                } else {
                    s
                }
            }
            S::Par(n) => format!("Param{n}"),
            S::Lvl => "lvl".into(),
            S::ElemMin { mastery, shift } => {
                shr(format!("elem_min(mastery {})", u8::from(mastery)), shift)
            }
            S::ElemMax { mastery, shift } => {
                shr(format!("elem_max(mastery {})", u8::from(mastery)), shift)
            }
            S::ElemLen => "elem_len".into(),
            S::ToHit => "to_hit".into(),
            S::Mana => g("cost >> 8".into()),
            S::Mps => g("(((mana+lvlmana*(lvl-1))*25/2) << manashift) >> 8".into()),
            S::Mastery(t) => format!("mastery(type {t})"),
            S::MissElem { slot, max, shift } => g(shr(
                format!(
                    "miss_elem_{}(descmissile{})",
                    if max { "max" } else { "min" },
                    slot + 1
                ),
                shift,
            )),
            S::MissElemLen { slot } => g(format!("miss_elem_len(descmissile{})", slot + 1)),
            S::MissRange { slot } => format!("Range + lvl*LevRange of descmissile{}", slot + 1),
            S::ULvl => "unit ? level(12) : 0".into(),
            S::BLvl => "unit ? skill_level(unit, highest entry of this skill, 0) : 0".into(),
            S::Usmc => g("cost".into()),
            S::Eval(f) => format!("eval {}", f.name()),
        }
    }
}

/// A missiles record field read by a missile special value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MField {
    Param(u8),
    CltParam(u8),
    SHitPar(u8),
    CHitPar(u8),
    DParam(u8),
}

impl MField {
    /// The field's value (i32) in a record.
    pub fn of(self, r: &Missiles) -> i32 {
        (match self {
            MField::Param(1) => r.param1,
            MField::Param(2) => r.param2,
            MField::Param(3) => r.param3,
            MField::Param(4) => r.param4,
            MField::Param(_) => r.param5,
            MField::CltParam(1) => r.cltparam1,
            MField::CltParam(2) => r.cltparam2,
            MField::CltParam(3) => r.cltparam3,
            MField::CltParam(4) => r.cltparam4,
            MField::CltParam(_) => r.cltparam5,
            MField::SHitPar(1) => r.shitpar1,
            MField::SHitPar(2) => r.shitpar2,
            MField::SHitPar(_) => r.shitpar3,
            MField::CHitPar(1) => r.chitpar1,
            MField::CHitPar(2) => r.chitpar2,
            MField::CHitPar(_) => r.chitpar3,
            MField::DParam(1) => r.dparam1,
            MField::DParam(_) => r.dparam2,
        }) as i32
    }

    fn name(self) -> String {
        match self {
            MField::Param(n) => format!("Param{n}"),
            MField::CltParam(n) => format!("CltParam{n}"),
            MField::SHitPar(n) => format!("sHitPar{n}"),
            MField::CHitPar(n) => format!("cHitPar{n}"),
            MField::DParam(n) => format!("dParam{n}"),
        }
    }
}

/// What one `misscalc` index returns (`misscalc.tsv` `returns`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissSpecial {
    Par(MField),
    Lvl,
    ElemMin {
        shift: bool,
    },
    ElemMax {
        shift: bool,
    },
    ElemLen,
    PhysMin {
        shift: bool,
    },
    PhysMax {
        shift: bool,
    },
    Range,
    /// `X+(lvl-1)*Y`, no level guard.
    Ln(MField, MField),
    /// `DM(lvl,X,Y)`, no level guard.
    Dm(MField, MField),
}

use MField as F;
use MissSpecial as M;

/// The 43 missile special values, by index.
pub const MISS_SPECIALS: [MissSpecial; 43] = [
    M::Par(F::Param(1)),
    M::Par(F::Param(2)),
    M::Par(F::Param(3)),
    M::Par(F::Param(4)),
    M::Par(F::Param(5)),
    M::Par(F::CltParam(1)),
    M::Par(F::CltParam(2)),
    M::Par(F::CltParam(3)),
    M::Par(F::CltParam(4)),
    M::Par(F::CltParam(5)),
    M::Par(F::SHitPar(1)),
    M::Par(F::SHitPar(2)),
    M::Par(F::SHitPar(3)),
    M::Par(F::CHitPar(1)),
    M::Par(F::CHitPar(2)),
    M::Par(F::CHitPar(3)),
    M::Par(F::DParam(1)),
    M::Par(F::DParam(2)),
    M::Lvl,
    M::ElemMin { shift: true },
    M::ElemMax { shift: true },
    M::ElemLen,
    M::ElemMin { shift: false },
    M::ElemMax { shift: false },
    M::PhysMin { shift: true },
    M::PhysMax { shift: true },
    M::PhysMin { shift: false },
    M::PhysMax { shift: false },
    M::Range,
    M::Ln(F::Param(1), F::Param(2)),
    M::Dm(F::Param(1), F::Param(2)),
    M::Ln(F::Param(3), F::Param(4)),
    M::Dm(F::Param(3), F::Param(4)),
    M::Ln(F::CltParam(1), F::CltParam(2)),
    M::Dm(F::CltParam(1), F::CltParam(2)),
    M::Ln(F::CltParam(3), F::CltParam(4)),
    M::Dm(F::CltParam(3), F::CltParam(4)),
    M::Ln(F::SHitPar(1), F::SHitPar(2)),
    M::Dm(F::SHitPar(1), F::SHitPar(2)),
    M::Ln(F::CHitPar(1), F::CHitPar(2)),
    M::Dm(F::CHitPar(1), F::CHitPar(2)),
    M::Ln(F::DParam(1), F::DParam(2)),
    M::Dm(F::DParam(1), F::DParam(2)),
];

/// The codes of [`MISS_SPECIALS`].
pub const MISSCALC_CODES: [&str; 43] = [
    "par1", "par2", "par3", "par4", "par5", "cpa1", "cpa2", "cpa3", "cpa4", "cpa5", "hpa1", "hpa2",
    "hpa3", "chp1", "chp2", "chp3", "dpa1", "dpa2", "lvl", "edmn", "edmx", "edln", "edns", "edxs",
    "damn", "damx", "dmns", "dmxs", "rang", "sl12", "sd12", "sl34", "sd34", "cl12", "cd12", "cl34",
    "cd34", "shl1", "shd1", "chl1", "chd1", "dl12", "dd12",
];

impl MissSpecial {
    /// The rule in `misscalc.tsv` notation (the `returns` column).
    pub fn returns(self) -> String {
        let shr = |s: &str, shift: bool| {
            if shift {
                format!("{s} >> 8")
            } else {
                s.to_string()
            }
        };
        match self {
            M::Par(f) => f.name(),
            M::Lvl => "lvl".into(),
            M::ElemMin { shift } => shr("miss_elem_min", shift),
            M::ElemMax { shift } => shr("miss_elem_max", shift),
            M::ElemLen => "miss_elem_len".into(),
            M::PhysMin { shift } => shr("miss_phys_min", shift),
            M::PhysMax { shift } => shr("miss_phys_max", shift),
            M::Range => "Range + lvl*LevRange".into(),
            M::Ln(a, b) => format!("{}+(lvl-1)*{}", a.name(), b.name()),
            M::Dm(a, b) => format!("DM(lvl,{},{})", a.name(), b.name()),
        }
    }
}

/// One row of a special-value TSV that disagrees with the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// Row index (the `index` column, or the row position if unreadable).
    pub index: usize,
    pub what: String,
}

/// Compares a TSV (columns `index`, `code`, `returns`, …) with the code's
/// (code, returns) rows. Every disagreement is reported: a header other
/// than the expected one, a row count, an index out of order, a code or a rule.
pub fn check_tsv(tsv: &str, rows: &[(&str, String)]) -> Vec<Mismatch> {
    let mut out = Vec::new();
    let mut lines = tsv.lines();
    if lines
        .next()
        .map(|h| h.split('\t').take(3).collect::<Vec<_>>())
        != Some(vec!["index", "code", "returns"])
    {
        out.push(Mismatch {
            index: 0,
            what: "header is not index, code, returns".into(),
        });
    }
    let data: Vec<&str> = lines.filter(|l| !l.is_empty()).collect();
    if data.len() != rows.len() {
        out.push(Mismatch {
            index: data.len().min(rows.len()),
            what: format!("{} rows, code has {}", data.len(), rows.len()),
        });
    }
    for (i, (line, (code, returns))) in data.iter().zip(rows).enumerate() {
        let cols: Vec<&str> = line.split('\t').collect();
        let get = |k: usize| cols.get(k).copied().unwrap_or("");
        if get(0) != i.to_string() {
            out.push(Mismatch {
                index: i,
                what: format!("index column {:?}", get(0)),
            });
        }
        if get(1) != *code {
            out.push(Mismatch {
                index: i,
                what: format!("code {:?}, code has {code:?}", get(1)),
            });
        }
        if get(2) != returns {
            out.push(Mismatch {
                index: i,
                what: format!("returns {:?}, code has {returns:?}", get(2)),
            });
        }
    }
    out
}

/// [`check_tsv`] of `skillcalc.tsv` text against [`SKILL_SPECIALS`].
pub fn check_skillcalc(tsv: &str) -> Vec<Mismatch> {
    let rows: Vec<(&str, String)> = SKILLCALC_CODES
        .iter()
        .zip(SKILL_SPECIALS)
        .map(|(c, s)| (*c, s.returns()))
        .collect();
    check_tsv(tsv, &rows)
}

/// [`check_tsv`] of `misscalc.tsv` text against [`MISS_SPECIALS`].
pub fn check_misscalc(tsv: &str) -> Vec<Mismatch> {
    let rows: Vec<(&str, String)> = MISSCALC_CODES
        .iter()
        .zip(MISS_SPECIALS)
        .map(|(c, s)| (*c, s.returns()))
        .collect();
    check_tsv(tsv, &rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skillcalc_tsv_matches_code() {
        assert_eq!(check_skillcalc(SKILLCALC_TSV), vec![]);
    }

    #[test]
    fn misscalc_tsv_matches_code() {
        assert_eq!(check_misscalc(MISSCALC_TSV), vec![]);
    }

    /// Changing one row's rule, code or index is reported at exactly
    /// that row; dropping a row is reported as a count.
    #[test]
    fn check_catches_perturbations() {
        let perturb = |tsv: &str, row: usize, col: usize, to: &str| -> String {
            tsv.lines()
                .enumerate()
                .map(|(i, l)| {
                    if i == row + 1 {
                        let mut c: Vec<&str> = l.split('\t').collect();
                        c[col] = to;
                        c.join("\t")
                    } else {
                        l.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let t = perturb(SKILLCALC_TSV, 5, 2, "lvl>0 ? DM(lvl,Param5,Param7) : 0");
        let m = check_skillcalc(&t);
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(m[0].index, 5);
        let t = perturb(SKILLCALC_TSV, 45, 1, "m2ex");
        let m = check_skillcalc(&t);
        assert_eq!((m.len(), m[0].index), (1, 45));
        let t = perturb(MISSCALC_TSV, 30, 0, "31");
        let m = check_misscalc(&t);
        assert_eq!((m.len(), m[0].index), (1, 30));
        let t = perturb(MISSCALC_TSV, 41, 2, "dParam1+(lvl-1)*dParam1");
        let m = check_misscalc(&t);
        assert_eq!((m.len(), m[0].index), (1, 41));
        let short: String = MISSCALC_TSV.lines().take(43).collect::<Vec<_>>().join("\n");
        let m = check_misscalc(&short);
        assert_eq!(m.len(), 1, "{m:?}");
        assert!(m[0].what.contains("42 rows"));
    }

    /// The codes equal the base-game `skillcalc.txt` / `misscalc.txt` `code`
    /// column ("Mechanical check"). Needs the extracted text
    /// tables under `D2_GAME_DIR/extracted/patch_d2/data/global/excel/`
    /// (`mpq-tool extract`).
    #[test]
    #[ignore = "needs the game tables in D2_GAME_DIR"]
    fn codes_match_game_tables() {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        for (file, codes) in [
            ("skillcalc.txt", &SKILLCALC_CODES[..]),
            ("misscalc.txt", &MISSCALC_CODES[..]),
        ] {
            let path = format!("{dir}/extracted/patch_d2/data/global/excel/{file}");
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let mut lines = text.lines();
            let header: Vec<&str> = lines.next().unwrap().split('\t').collect();
            let col = header
                .iter()
                .position(|h| *h == "code")
                .expect("code column");
            let got: Vec<String> = lines
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.split('\t').nth(col).unwrap_or("").to_string())
                .filter(|c| !c.is_empty() && c != "Expansion")
                .collect();
            assert_eq!(got, codes, "{file}");
        }
    }
}
