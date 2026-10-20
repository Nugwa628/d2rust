//! The `skilldesc` line function of the skill hover boxes: one `descline` /
//! `dsc2line` / `dsc3line` cell of a skill's `skilldesc` row, appended to the
//! box's UTF-16 buffer by its line type (types 1–75, jump table), with the
//! shared number and text helpers the types call, the mana cost line
//! and the "has learned a class skill" test.
//!
//! The facts a line reads (records, formulas, skill levels, monster stats
//! by level, strings) come through [`HoverInputs`]; [`ModelInputs`]
//! answers them from the client model of the local player
//! ([`DescUnits`]) and the loaded tables. Nothing here decides an
//! outcome: it only writes text. Numbers are ASCII texts widened unit by
//! unit, as the game widens its `wsprintfA` output.

use std::sync::Arc;

use d2_data::tables::{Missiles, Monlvl, Monstats, Skilldesc, Skills};
use d2_sim::combat::pct;
use d2_sim::skills::levels::{
    elem_len, elem_max, elem_min, eval_desc, eval_skill, miss_elem_max, miss_elem_min, phys_max,
    phys_min, skill_level, to_hit,
};
use d2_sim::skills::{SkillTables, SkillUnits};
use d2_sim::units::UnitType;

use crate::bridge::world::{ClientWorld, UnitKey};
use crate::ui::char_desc::{DescTables, DescUnits};
use crate::ui::panel::StringLookup;

/// Cells of a `skilldesc` row: 0–5 `descline1`–`6`, 6–9 `dsc2line1`–`4`,
/// 10–16 `dsc3line1`–`7`.
pub const LINES: usize = 17;
/// "No formula" in a calc column.
pub const NO_CALC: u32 = u32::MAX;

/// `dummy` (Conventions): every empty `strkey` cell; "an evil force".
pub const STR_DUMMY: u16 = 5382;
/// `ÿc`, the colour code prefix.
pub const STR_COLOR: u16 = 3994;
pub const STR_SPACE: u16 = 3995;
pub const STR_COLON: u16 = 3997;
/// The line feed.
pub const STR_NL: u16 = 3998;
pub const STR_SLASH: u16 = 4000;
pub const STR_PERCENT_SIGN: u16 = 4001;
const STR_DAMAGE: u16 = 4256;
const STR_FIRE_DAMAGE: u16 = 4257;
const STR_COLD_DAMAGE: u16 = 4258;
const STR_LIGHTNING_DAMAGE: u16 = 4259;
const STR_POISON_DAMAGE: u16 = 4260;
const STR_TO_ATTACK: u16 = 4262;
const STR_COLD_LENGTH: u16 = 4265;
const STR_POISON_LENGTH: u16 = 4266;
const STR_SECOND: u16 = 4267;
const STR_SECONDS: u16 = 4268;
const STR_RADIUS: u16 = 4270;
const STR_DURATION: u16 = 4272;
const STR_PERCENT: u16 = 4275;
const STR_YARDS: u16 = 4278;
const STR_PER_SECOND: u16 = 4286;
const STR_YARD: u16 = 4288;
const STR_MAGIC_DAMAGE: u16 = 4291;
const STR_LIFE: u16 = 4294;
const STR_DAMAGE_RETURNED: u16 = 4309;
const STR_AVERAGE: u16 = 4322;
const STR_THORNS: u16 = 4365;
const STR_LIFE_STOLEN: u16 = 3524;
const STR_OVER: u16 = 11022;
const STR_CHARGES: u16 = 21248;
const STR_PAREN_OPEN: u16 = 21252;
const STR_PAREN_CLOSE: u16 = 21253;

/// Stats read by the lines: `damagepercent` (types 34, 39),
/// `item_normaldamage` (34), the local player's `passive_fire_mastery` /
/// `passive_ltng_mastery` (22).
const STAT_DAMAGEPERCENT: u16 = 25;
const STAT_ITEM_NORMALDAMAGE: u16 = 111;
const STAT_FIRE_MASTERY: u16 = 329;
const STAT_LTNG_MASTERY: u16 = 330;
/// `firegolem`, the monstats row of type 41.
const MON_FIRE_GOLEM: i32 = 292;

/// The `skilldesc` fields the boxes and the lines read (Record offsets).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescRow {
    /// `str name` (+0x08), `str long` (+0x0C), `str mana` (+0x10).
    pub name: u16,
    pub long: u16,
    pub mana: u16,
    /// `descmissile1` (+0x3C, i16).
    pub missile: i32,
    /// Line types (+0x42 + i), texta (+0x54 + 2i), textb (+0x76 + 2i),
    /// calca (+0x98 + 4i), calcb (+0xDC + 4i).
    pub kind: [u8; LINES],
    pub texta: [u16; LINES],
    pub textb: [u16; LINES],
    pub calca: [u32; LINES],
    pub calcb: [u32; LINES],
}

impl Default for DescRow {
    /// No lines, every text `dummy`, no formulas.
    fn default() -> Self {
        DescRow {
            name: STR_DUMMY,
            long: STR_DUMMY,
            mana: STR_DUMMY,
            missile: 0,
            kind: [0; LINES],
            texta: [STR_DUMMY; LINES],
            textb: [STR_DUMMY; LINES],
            calca: [NO_CALC; LINES],
            calcb: [NO_CALC; LINES],
        }
    }
}

impl DescRow {
    /// The fields of a decoded `skilldesc` record.
    pub fn of(d: &Skilldesc) -> Self {
        DescRow {
            name: d.str_name,
            long: d.str_long,
            mana: d.str_mana,
            missile: i32::from(d.descmissile1 as i16),
            kind: [
                d.descline1,
                d.descline2,
                d.descline3,
                d.descline4,
                d.descline5,
                d.descline6,
                d.dsc2line1,
                d.dsc2line2,
                d.dsc2line3,
                d.dsc2line4,
                d.dsc3line1,
                d.dsc3line2,
                d.dsc3line3,
                d.dsc3line4,
                d.dsc3line5,
                d.dsc3line6,
                d.dsc3line7,
            ],
            texta: [
                d.desctexta1,
                d.desctexta2,
                d.desctexta3,
                d.desctexta4,
                d.desctexta5,
                d.desctexta6,
                d.dsc2texta1,
                d.dsc2texta2,
                d.dsc2texta3,
                d.dsc2texta4,
                d.dsc3texta1,
                d.dsc3texta2,
                d.dsc3texta3,
                d.dsc3texta4,
                d.dsc3texta5,
                d.dsc3texta6,
                d.dsc3texta7,
            ],
            textb: [
                d.desctextb1,
                d.desctextb2,
                d.desctextb3,
                d.desctextb4,
                d.desctextb5,
                d.desctextb6,
                d.dsc2textb1,
                d.dsc2textb2,
                d.dsc2textb3,
                d.dsc2textb4,
                d.dsc3textb1,
                d.dsc3textb2,
                d.dsc3textb3,
                d.dsc3textb4,
                d.dsc3textb5,
                d.dsc3textb6,
                d.dsc3textb7,
            ],
            calca: [
                d.desccalca1,
                d.desccalca2,
                d.desccalca3,
                d.desccalca4,
                d.desccalca5,
                d.desccalca6,
                d.dsc2calca1,
                d.dsc2calca2,
                d.dsc2calca3,
                d.dsc2calca4,
                d.dsc3calca1,
                d.dsc3calca2,
                d.dsc3calca3,
                d.dsc3calca4,
                d.dsc3calca5,
                d.dsc3calca6,
                d.dsc3calca7,
            ],
            calcb: [
                d.desccalcb1,
                d.desccalcb2,
                d.desccalcb3,
                d.desccalcb4,
                d.desccalcb5,
                d.desccalcb6,
                d.dsc2calcb1,
                d.dsc2calcb2,
                d.dsc2calcb3,
                d.dsc2calcb4,
                d.dsc3calcb1,
                d.dsc3calcb2,
                d.dsc3calcb3,
                d.dsc3calcb4,
                d.dsc3calcb5,
                d.dsc3calcb6,
                d.dsc3calcb7,
            ],
        }
    }
}

/// The `skills` fields the boxes and the lines read (Record offsets).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillRow {
    /// `charclass` (+0x0C).
    pub class: u8,
    /// `EType` (+0x1DC).
    pub etype: u8,
    /// `summon` (+0xBC, i16).
    pub summon: i32,
    /// `maxlvl` (+0x12C), as stored.
    pub maxlvl: i32,
    /// `reqlevel` (+0x174, i16).
    pub reqlevel: i32,
    /// `mana` (+0x18A), `lvlmana` (+0x18C), `manashift` (+0x188),
    /// `minmana` (+0x186), all 16-bit signed.
    pub mana: i32,
    pub lvlmana: i32,
    pub manashift: i32,
    pub minmana: i32,
    /// `Param1` (+0x148), `Param2` (+0x14C).
    pub param1: i32,
    pub param2: i32,
    /// (`aurastat1–6`, `aurastatcalc1–6`).
    pub aura: [(u16, u32); 6],
    /// (`passivestat1–5`, `passivecalc1–5`).
    pub passive: [(u16, u32); 5],
}

impl Default for SkillRow {
    fn default() -> Self {
        SkillRow {
            class: 0xFF,
            etype: 0,
            summon: -1,
            maxlvl: 0,
            reqlevel: 0,
            mana: 0,
            lvlmana: 0,
            manashift: 0,
            minmana: 0,
            param1: 0,
            param2: 0,
            aura: [(0xFFFF, NO_CALC); 6],
            passive: [(0xFFFF, NO_CALC); 5],
        }
    }
}

impl SkillRow {
    /// The fields of a decoded `skills` record.
    pub fn of(s: &Skills) -> Self {
        let i16v = |v: u16| i32::from(v as i16);
        SkillRow {
            class: s.charclass,
            etype: s.etype,
            summon: i16v(s.summon),
            maxlvl: i16v(s.maxlvl),
            reqlevel: i16v(s.reqlevel),
            mana: i16v(s.mana),
            lvlmana: i16v(s.lvlmana),
            manashift: i16v(s.manashift),
            minmana: i16v(s.minmana),
            param1: s.param1 as i32,
            param2: s.param2 as i32,
            aura: [
                (s.aurastat1, s.aurastatcalc1),
                (s.aurastat2, s.aurastatcalc2),
                (s.aurastat3, s.aurastatcalc3),
                (s.aurastat4, s.aurastatcalc4),
                (s.aurastat5, s.aurastatcalc5),
                (s.aurastat6, s.aurastatcalc6),
            ],
            passive: [
                (s.passivestat1, s.passivecalc1),
                (s.passivestat2, s.passivecalc2),
                (s.passivestat3, s.passivecalc3),
                (s.passivestat4, s.passivecalc4),
                (s.passivestat5, s.passivecalc5),
            ],
        }
    }

    /// `maxlvl`, or 20 when it is ≤ 0.
    pub fn max_level(&self) -> i32 {
        if self.maxlvl > 0 {
            self.maxlvl
        } else {
            20
        }
    }
}

/// The `missiles` fields the lines read (Record offsets).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MissileRow {
    /// `Range` (+0x96, i16), `LevRange` (+0x98, i16).
    pub range: i32,
    pub lev_range: i32,
    /// `Param1` (+0x38).
    pub param1: i32,
    /// `SubMissile1` (+0x18), −1 for none.
    pub sub1: i32,
    /// `EType` (+0xE4).
    pub etype: u8,
}

impl MissileRow {
    /// The fields of a decoded `missiles` record.
    pub fn of(m: &Missiles) -> Self {
        MissileRow {
            range: i32::from(m.range as i16),
            lev_range: i32::from(m.levrange as i16),
            param1: m.param1 as i32,
            sub1: i32::from(m.submissile1 as i16),
            etype: m.etype,
        }
    }
}

/// What the boxes and the line function read, for the local player U. Skill and
/// missile ids are row numbers; a level argument is the line's level.
pub trait HoverInputs {
    /// `D2Lang_GetStringByIndex(id)`; empty when unknown.
    fn string(&self, id: u16) -> Vec<u16>;
    /// The `skills` row count.
    fn skill_count(&self) -> i32;
    /// `skills` row `s`; `None` out of range.
    fn skill(&self, s: i32) -> Option<SkillRow>;
    /// The `skilldesc` row of `R.skilldesc`.
    fn desc(&self, s: i32) -> Option<DescRow>;
    /// `missiles` row `m`.
    fn missile(&self, m: i32) -> Option<MissileRow>;
    /// A skilldesc formula (no formula → 0).
    fn eval_desc(&mut self, calc: u32, s: i32, l: i32) -> i32;
    /// A skills formula (no formula → 0).
    fn eval_skill(&mut self, calc: u32, s: i32, l: i32) -> i32;
    /// `elem_min(U, s, L, 1)` / `elem_max(U, s, L, 1)` (1/256 points).
    fn elem_min(&mut self, s: i32, l: i32) -> i32;
    fn elem_max(&mut self, s: i32, l: i32) -> i32;
    /// `elem_len(U, s, L)` (frames).
    fn elem_len(&mut self, s: i32, l: i32) -> i32;
    /// `phys_min` / `phys_max(U, s, L, use_srcdam)` (<< HitShift).
    fn phys_min(&mut self, s: i32, l: i32, src_dam: bool) -> i32;
    fn phys_max(&mut self, s: i32, l: i32, src_dam: bool) -> i32;
    /// `to_hit(U, s, L)`.
    fn to_hit(&mut self, s: i32, l: i32) -> i32;
    /// Missile elemental min / max `(0, U, m, L)` (in
    /// 1/256 points).
    fn missile_elem_min(&mut self, m: i32, l: i32) -> i32;
    fn missile_elem_max(&mut self, m: i32, l: i32) -> i32;
    /// The local player's stat total.
    fn stat(&self, stat: u16) -> i32;
    /// U's entry for `s`: (`skill_level(U, e,
    /// 0)`, `skill_level(U, e, 1)`); `None` without an entry.
    fn levels(&self, s: i32) -> Option<(i32, i32)>;
    /// U's class, `None` for no player.
    fn class(&self) -> Option<u8>;
    /// The difficulty byte.
    fn difficulty(&self) -> u8;
    /// The expansion flag.
    fn expansion(&self) -> bool;
    /// `AiCurseDivisor` of the `difficultylevels` row of the difficulty
    /// (clamped to the rows); 0 without rows.
    fn curse_divisor(&self) -> i32;
    /// The `monstats` row count.
    fn monstats_count(&self) -> i32;
    /// monstats `Level[d]` (+0xAA + 2d) of row `m` for the difficulty.
    fn monster_level(&self, m: i32) -> Option<i32>;
    /// (minHP, maxHP); `None` when
    /// the helper fails.
    fn monster_life(&self, m: i32, level: i32) -> Option<(i32, i32)>;
    /// The A1 (min, max) damage;
    /// `None` when the helper fails.
    fn monster_a1(&self, m: i32, level: i32) -> Option<(i32, i32)>;
}

// ---------------------------------------------------------------- numbers

/// `N(v, plus)`: `%d`, or `+%d` with `plus` and v ≥ 0.
pub fn num(v: i32, plus: bool) -> String {
    if plus && v >= 0 {
        format!("+{v}")
    } else {
        format!("{v}")
    }
}

/// `F(i, f, plus)`: `%d.%d` (the sign rule of [`num`] on
/// i); f is not zero-padded (Edge case 4).
pub fn frac(i: i32, f: i32, plus: bool) -> String {
    format!("{}.{f}", num(i, plus))
}

/// `"%d"` (a = b) or `"%d-%d"`: the shared number range text (`rng`).
pub fn rng(a: i32, b: i32) -> String {
    if a == b {
        format!("{a}")
    } else {
        format!("{a}-{b}")
    }
}

/// `elabel(t)`: the element's damage label, 0 for none.
pub fn elabel(t: i32) -> u16 {
    match t {
        1 => STR_FIRE_DAMAGE,
        2 => STR_LIGHTNING_DAMAGE,
        3 => STR_MAGIC_DAMAGE,
        4 => STR_COLD_DAMAGE,
        5 => STR_POISON_DAMAGE,
        6 => STR_LIFE_STOLEN,
        _ => 0,
    }
}

/// `elabelF(t)`: [`elabel`] with fire for every other type.
pub fn elabel_fire(t: i32) -> u16 {
    match elabel(t) {
        0 => STR_FIRE_DAMAGE,
        s => s,
    }
}

/// `"%d.%d"` with tenths (or `d`ths) of a 1/256 value: (q, r) with q =
/// v / 256 and r = trunc((v rem 256) × d / 256), signed.
fn split256(v: i32, d: i32) -> (i32, i32) {
    (v / 256, (v % 256).wrapping_mul(d) / 256)
}

/// One argument of [`wsp`].
#[derive(Clone, Copy, Debug)]
pub enum Arg<'a> {
    Int(i32),
    Str(&'a [u16]),
}

/// D2's wide `sprintf` into a buffer of `size` units (the last one holds the
/// terminator) (`wsp`). `%d`, `%u`, `%s` and `%%` (a `%`
/// that still uses up one argument slot). A `%s` whose text is empty or does
/// not fit copies what fits and ends the formatting. Any other character after
/// `%` is a fatal error in the game; here the formatting
/// ends there instead (D2RUST's own, no live cell has one). A missing argument
/// reads as 0 / an empty text, and a text where a number is expected as 0
/// (D2RUST's own: the game would print the pointer).
pub fn wsp(size: usize, fmt: &[u16], args: &[Arg]) -> Vec<u16> {
    let cap = size.saturating_sub(1);
    let mut out: Vec<u16> = Vec::new();
    let mut slot = 0usize;
    let mut i = 0usize;
    let int = |a: Option<&Arg>| match a {
        Some(Arg::Int(v)) => *v,
        _ => 0,
    };
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != u16::from(b'%') {
            if out.len() >= cap {
                break;
            }
            out.push(c);
            continue;
        }
        let Some(&k) = fmt.get(i) else {
            break;
        };
        i += 1;
        let arg = args.get(slot);
        slot += 1;
        let room = cap - out.len();
        let text: Vec<u16> = match u8::try_from(k).map(char::from) {
            Ok('d') => int(arg).to_string().encode_utf16().collect(),
            Ok('u') => (int(arg) as u32).to_string().encode_utf16().collect(),
            Ok('%') => vec![u16::from(b'%')],
            Ok('s') => {
                let s: &[u16] = match arg {
                    Some(Arg::Str(s)) => s,
                    _ => &[],
                };
                if s.is_empty() || s.len() > room {
                    out.extend_from_slice(&s[..s.len().min(room)]);
                    break;
                }
                s.to_vec()
            }
            _ => break,
        };
        if text.len() > room {
            out.extend_from_slice(&text[..room]);
            break;
        }
        out.extend(text);
    }
    out
}

// ---------------------------------------------------------------- writer

/// The buffer of one line call and the inputs it reads.
struct Writer<'a> {
    inp: &'a mut dyn HoverInputs,
    out: &'a mut Vec<u16>,
}

impl Writer<'_> {
    /// Appends str(id).
    fn s(&mut self, id: u16) {
        let t = self.inp.string(id);
        self.out.extend(t);
    }
    /// "ta?" / "tb?": str(id) only when id ≠ 5382.
    fn s_opt(&mut self, id: u16) {
        if id != STR_DUMMY {
            self.s(id);
        }
    }
    /// Appends an ASCII number text.
    fn a(&mut self, t: &str) {
        self.out.extend(t.encode_utf16());
    }
    fn units(&mut self, t: &[u16]) {
        self.out.extend_from_slice(t);
    }
    fn nl(&mut self) {
        self.s(STR_NL);
    }
    /// `:` + space.
    fn colon_space(&mut self) {
        self.s(STR_COLON);
        self.s(STR_SPACE);
    }

    /// `sec(f, label, plus)`: frames as seconds with
    /// tenths; "second" only for exactly 1.
    fn sec(&mut self, f: i32, label: u16, plus: bool) {
        let q = f / 25;
        let t = (f - 25 * q) * 10 / 25;
        if q == 0 && t == 0 {
            return;
        }
        self.s(label);
        self.a(&if t != 0 {
            frac(q, t, plus)
        } else {
            num(q, plus)
        });
        self.s(if q == 1 && t == 0 {
            STR_SECOND
        } else {
            STR_SECONDS
        });
        self.nl();
    }

    /// `yards2` (k = 20) / `yards1` (k = 10): u = k × v / 3 tenths of a yard,
    /// the integer part at least 1.
    fn yards(&mut self, v: i32, label: u16, k: i32) {
        let u = k.wrapping_mul(v) / 3;
        let mut q = u / 10;
        let r = u - 10 * q;
        if q < 1 {
            q = 1;
        }
        self.s(label);
        self.a(&if r != 0 {
            format!("{q}.{r}")
        } else {
            format!("{q}")
        });
        self.s(if q == 1 { STR_YARD } else { STR_YARDS });
        self.nl();
    }

    /// `elemline(a, b, label, etype)`: a label of 0 (low
    /// 16 bits) falls back to [`elabel`]; still 0 → nothing.
    fn elemline(&mut self, a: i32, b: i32, label: u16, etype: i32) {
        let label = if label == 0 { elabel(etype) } else { label };
        if label == 0 {
            return;
        }
        self.s(label);
        self.a(&rng(a, b));
        self.nl();
    }

    /// `avgline(a, b, label)`.
    fn avgline(&mut self, a: i32, b: i32, label: u16) {
        if a == 0 && b == 0 {
            return;
        }
        self.s(STR_AVERAGE);
        self.s(label);
        self.a(&rng(a, b));
        self.s(STR_PER_SECOND);
        self.nl();
    }

    /// `TVT(v, s1, s2, plus)`.
    fn tvt(&mut self, v: i32, s1: u16, s2: u16, plus: bool) {
        if v == 0 {
            return;
        }
        self.s(s1);
        self.a(&num(v, plus));
        self.s(s2);
        self.nl();
    }
    /// `TV(v, s, plus)`.
    fn tv(&mut self, v: i32, s: u16, plus: bool) {
        if v == 0 {
            return;
        }
        self.s(s);
        self.a(&num(v, plus));
        self.nl();
    }
    /// `VT(v, s, plus)`.
    fn vt(&mut self, v: i32, s: u16, plus: bool) {
        if v == 0 {
            return;
        }
        self.a(&num(v, plus));
        self.s(s);
        self.nl();
    }
    /// `VTT(v, s1, s2, plus)`.
    fn vtt(&mut self, v: i32, s1: u16, s2: u16, plus: bool) {
        if v == 0 {
            return;
        }
        self.a(&num(v, plus));
        self.s(s1);
        self.s(s2);
        self.nl();
    }

    /// `"%d-%d"` / `"+%d-%d"`.
    fn pair(&mut self, a: i32, b: i32, plus: bool) {
        self.a(&if plus {
            format!("+{a}-{b}")
        } else {
            format!("{a}-{b}")
        });
    }
    /// `range2`: texts around the range, dummies skipped.
    fn range2(&mut self, a: i32, b: i32, s1: u16, s2: u16, plus: bool) {
        if a == b {
            return self.tvt(a, s1, s2, false);
        }
        self.s_opt(s1);
        self.pair(a, b, plus);
        self.s_opt(s2);
        self.nl();
    }
    /// `range3`: the range after both texts.
    fn range3(&mut self, a: i32, b: i32, s1: u16, s2: u16, plus: bool) {
        if a == b {
            return self.tvt(a, s1, s2, false);
        }
        self.s(s1);
        self.s(s2);
        self.pair(a, b, plus);
        self.nl();
    }
    /// `range4`: one text, an optional space, the range.
    fn range4(&mut self, a: i32, b: i32, s: u16, plus: bool, sp: bool) {
        if a == b {
            return self.tv(a, s, plus);
        }
        self.s(s);
        if sp {
            self.s(STR_SPACE);
        }
        self.pair(a, b, plus);
        self.nl();
    }

    /// `fix(v, d, s1, s2, plus)`: a 1/256 value with `d`ths.
    fn fix(&mut self, v: i32, d: i32, s1: u16, s2: u16, plus: bool) {
        if v == 0 {
            return;
        }
        let (q, r) = split256(v, d);
        self.s_opt(s1);
        if r != 0 {
            self.a(&frac(q, r, plus));
        } else if q != 0 {
            self.a(&num(q, plus));
        }
        self.s_opt(s2);
        self.nl();
    }

    /// `colon(v, s1, s2, plus, pc)`: "s1: +v% s2".
    fn colon(&mut self, v: i32, s1: u16, s2: u16, plus: bool, pc: bool) {
        if v == 0 {
            return;
        }
        if s1 != STR_DUMMY {
            self.s(s1);
            self.colon_space();
        }
        self.a(&num(v, plus));
        if pc {
            self.s(STR_PERCENT_SIGN);
        }
        self.s(STR_SPACE);
        self.s(s2);
        self.nl();
    }

    /// `col(t, k)`: `ÿc` + ('0' + k) before a non-empty t.
    fn col(&self, t: &mut Vec<u16>, k: i32) {
        color_prefix(&*self.inp, t, k);
    }

    /// The coloured tail of types 18, 25, 40, 46: t in colour A (A > 0),
    /// then a line feed in the box's base colour `flag`.
    fn colored(&mut self, mut t: Vec<u16>, a: i32, flag: i32) {
        if a > 0 {
            self.col(&mut t, a);
        }
        self.units(&t);
        let mut nl = self.inp.string(STR_NL);
        self.col(&mut nl, flag);
        self.units(&nl);
    }

    /// Type 9 without its texts: the physical damage with
    /// A percent and B points added.
    fn phys_line(&mut self, s: i32, l: i32, a_pct: i32, b_add: i32) {
        let mut a = self.inp.phys_min(s, l, false) >> 8;
        let mut b = self.inp.phys_max(s, l, false) >> 8;
        a = a.wrapping_add(pct(a, a_pct, 100)).wrapping_add(b_add);
        b = b.wrapping_add(pct(b, a_pct, 100)).wrapping_add(b_add);
        if a == 0 && b == 0 {
            return;
        }
        self.s(STR_DAMAGE);
        self.a(&if a != b {
            format!("{a}-{b}")
        } else {
            num(a, true)
        });
        self.nl();
    }

    /// Type 10 / 24 without its texts.
    fn elem_line(&mut self, s: i32, l: i32, label: u16, etype: i32) {
        let m = self.inp.elem_min(s, l);
        let mx = self.inp.elem_max(s, l);
        if m == 0 && mx == 0 {
            return;
        }
        self.elemline(m >> 8, mx >> 8, label, etype);
    }

    /// Type 11 without its texts: cold / poison length.
    fn length_line(&mut self, s: i32, l: i32, etype: i32) {
        let n = self.inp.elem_len(s, l);
        if n == 0 {
            return;
        }
        match etype {
            4 => self.sec(n, STR_COLD_LENGTH, false),
            5 => self.sec(n, STR_POISON_LENGTH, false),
            _ => {}
        }
    }

    /// The to-hit line of type 8 and type 48.
    fn attack_line(&mut self, s: i32, l: i32) {
        let t = self.inp.to_hit(s, l);
        self.tvt(t, STR_TO_ATTACK, STR_PERCENT, true);
    }

    /// Types 34 / 39: the summon's A1 damage
    /// with the skill's `damagepercent` (and, for 34, `item_normaldamage`)
    /// aura / passive stats.
    fn summon_damage(&mut self, r: &SkillRow, s: i32, l: i32, normal: bool) {
        if !(0..self.inp.monstats_count()).contains(&r.summon) {
            return;
        }
        let level = self.inp.monster_level(r.summon).unwrap_or(0);
        let (mut a, mut b) = self.inp.monster_a1(r.summon, level).unwrap_or((0, 0));
        let mut p = 0i32;
        for &(stat, calc) in r.aura.iter().chain(r.passive.iter()) {
            if stat == STAT_DAMAGEPERCENT {
                p = p.wrapping_add(self.inp.eval_skill(calc, s, l));
            } else if normal && stat == STAT_ITEM_NORMALDAMAGE {
                let v = self.inp.eval_skill(calc, s, l);
                a = a.wrapping_add(v);
                b = b.wrapping_add(v);
            }
        }
        if p != 0 {
            p = p.max(-90);
            a = a.wrapping_add(pct(a, p, 100));
            b = b.wrapping_add(pct(b, p, 100));
        }
        self.elemline(a, b, STR_DAMAGE, 0);
    }

    /// A range of two 1/256 values with `d`ths (types 43,
    /// 44).
    fn range256(&mut self, a: i32, b: i32, d: i32, ta: u16, tb: u16) {
        let (qa, ra) = split256(a, d);
        let (qb, rb) = split256(b, d);
        if qa == 0 && ra == 0 && qb == 0 && rb == 0 {
            return;
        }
        self.s_opt(ta);
        self.a(&if ra == 0 && rb == 0 && (qa != 0 || qb != 0) {
            format!("{qa}-{qb}")
        } else {
            format!("{qa}.{ra}-{qb}.{rb}")
        });
        self.s_opt(tb);
        self.nl();
    }

    /// Type 1: the mana cost.
    fn mana(&mut self, r: &SkillRow, d: &DescRow, p4: i32, l: i32, a: i32) {
        if r.mana == 0 && r.lvlmana == 0 {
            return;
        }
        if p4 != 0 {
            // "(n Charges)".
            self.s(STR_PAREN_OPEN);
            self.a(&format!("{p4}"));
            self.s(STR_SPACE);
            self.s(STR_CHARGES);
            self.s(STR_PAREN_CLOSE);
            self.nl();
            return;
        }
        let lbl = d.mana;
        let shift = (r.manashift & 31) as u32;
        let mut m = r
            .mana
            .wrapping_add((l - 1).wrapping_mul(r.lvlmana))
            .wrapping_shl(shift);
        m = m.max(r.minmana.wrapping_mul(256));
        if r.manashift == 8 || r.lvlmana.wrapping_shl(shift) > 256 || m > 6400 {
            self.tv(m >> 8, lbl, false);
        } else if a > 10 {
            self.fix(m, a, lbl, STR_DUMMY, false);
        } else {
            // Tenths.
            let (q, r) = split256(m, 10);
            if q == 0 && r <= 0 {
                return;
            }
            self.s(lbl);
            self.a(&if r != 0 {
                frac(q, r, false)
            } else {
                num(q, false)
            });
            self.nl();
        }
    }
}

/// `col(t, k)`: `ÿc` (string 3994) and the character '0' +
/// k before a non-empty t; an empty t is left as is.
pub fn color_prefix(inp: &dyn HoverInputs, t: &mut Vec<u16>, k: i32) {
    if t.is_empty() {
        return;
    }
    let mut p = inp.string(STR_COLOR);
    p.push(0x30u16.wrapping_add(k as u16));
    p.extend_from_slice(t);
    *t = p;
}

/// The line function: cell `i` (0–16) of skill `s`'s
/// `skilldesc` row at level `l`, appended to `out`; `flag` is the box's
/// base colour. Returns the line's return value, which
/// the boxes read only for their block separators.
#[allow(clippy::too_many_arguments)]
pub fn line(
    inp: &mut dyn HoverInputs,
    out: &mut Vec<u16>,
    r: Option<&SkillRow>,
    s: i32,
    l: i32,
    p4: i32,
    i: usize,
    flag: i32,
) -> bool {
    // r1: no row, no description → 0; the level is not tested.
    let Some(r) = r else {
        return false;
    };
    let Some(d) = inp.desc(s) else {
        return false;
    };
    let Some(&kind) = d.kind.get(i) else {
        return false;
    };
    let (ta, tb) = (d.texta[i], d.textb[i]);
    // r3: both formulas, always, before the switch.
    let a = inp.eval_desc(d.calca[i], s, l);
    let b = inp.eval_desc(d.calcb[i], s, l);
    let etype = i32::from(r.etype);
    let mut w = Writer { inp, out };
    match kind {
        1 => w.mana(r, &d, p4, l, a),
        2 => w.tvt(a, ta, tb, true),
        3 => w.tvt(a, ta, tb, false),
        4 => w.tv(a, ta, true),
        5 => w.tv(a, ta, false),
        6 => w.vt(a, ta, true),
        7 => w.vt(a, ta, false),
        8 => {
            w.s_opt(ta);
            w.s_opt(tb);
            w.attack_line(s, l);
        }
        9 => {
            w.s_opt(ta);
            w.s_opt(tb);
            w.phys_line(s, l, a, b);
        }
        10 => {
            w.s_opt(ta);
            w.s_opt(tb);
            w.elem_line(s, l, 0, etype);
        }
        11 => {
            w.s_opt(ta);
            w.s_opt(tb);
            w.length_line(s, l, etype);
        }
        12 => {
            w.s_opt(tb);
            w.sec(a, ta, false);
        }
        13 => {
            // The summon's mean life with A% and B added.
            if (0..w.inp.monstats_count()).contains(&r.summon) {
                let level = w.inp.monster_level(r.summon).unwrap_or(0);
                if let Some((lo, hi)) = w.inp.monster_life(r.summon, level) {
                    let h = lo.wrapping_add(hi) / 2;
                    let v = h.wrapping_add(pct(h, a, 100)).wrapping_add(b);
                    w.tv(v, STR_LIFE, false);
                }
            }
        }
        14 => {
            // Poison over its length; the damage line lands
            // above the "over" line.
            w.s_opt(ta);
            w.s_opt(tb);
            let n = w.inp.elem_len(s, l);
            let lo = w.inp.elem_min(s, l).wrapping_mul(n) >> 8;
            let hi = w.inp.elem_max(s, l).wrapping_mul(n) >> 8;
            w.sec(n, STR_OVER, false);
            let t = elabel(etype);
            if t != 0 {
                w.s(t);
                w.a(&rng(lo, hi));
                w.nl();
            }
        }
        15 => {
            w.s(ta);
            w.colon_space();
            w.s(tb);
            w.nl();
        }
        16 => {
            // Always a range, always plural.
            let qa = a / 25;
            let ra = (a - 25 * qa) * 10 / 25;
            let qb = b / 25;
            let rb = (b - 25 * qb) * 10 / 25;
            if !(qa == 0 && ra == 0 && qb == 0) {
                w.s(STR_DURATION);
                w.a(&if ra == 0 && rb == 0 {
                    format!("{qa}-{qb}")
                } else {
                    format!("{qa}.{ra}-{qb}.{rb}")
                });
                w.s(STR_SECONDS);
                w.nl();
            }
        }
        17 => {
            w.s_opt(tb);
            w.range2(a, b, ta, STR_PER_SECOND, false);
        }
        18 => {
            let t = w.inp.string(ta);
            w.colored(t, a, flag);
        }
        19 => {
            w.s_opt(tb);
            w.yards(a, ta, 20);
        }
        20 | 21 => {
            // No dummy tests.
            if a != 0 {
                w.s(ta);
                w.a(&num(a, kind == 20));
                w.s(STR_PERCENT);
                w.s(tb);
                w.nl();
            }
        }
        22 => {
            // A missile's average damage per second, with the local player's
            // fire / lightning mastery.
            let m = d.missile;
            let mut lo = w.inp.missile_elem_min(m, l).wrapping_mul(75);
            let mut hi = w.inp.missile_elem_max(m, l).wrapping_mul(75);
            let mastery = match r.etype {
                1 => Some(STAT_FIRE_MASTERY),
                2 => Some(STAT_LTNG_MASTERY),
                _ => None,
            };
            if let Some(stat) = mastery {
                let p = w.inp.stat(stat);
                lo = lo.wrapping_add(pct(lo, p, 100));
                hi = hi.wrapping_add(pct(hi, p, 100));
            }
            let me = w.inp.missile(m).map_or(0, |x| i32::from(x.etype));
            w.avgline(lo >> 8, hi >> 8, elabel_fire(me));
        }
        23 => {
            let Some(mis) = w.inp.missile(d.missile) else {
                return false;
            };
            let f = mis.range.wrapping_add(mis.lev_range.wrapping_mul(l));
            w.sec(f, ta, false);
        }
        24 => {
            w.s_opt(tb);
            w.elem_line(s, l, ta, etype);
        }
        25 => {
            let mut t = w.inp.string(ta);
            if tb != STR_DUMMY {
                t.extend(w.inp.string(tb));
            }
            w.colored(t, a, flag);
        }
        26 => {
            w.s_opt(ta);
            w.s_opt(tb);
            let lo = w.inp.elem_min(s, l).wrapping_mul(25) >> 8;
            let hi = w.inp.elem_max(s, l).wrapping_mul(25) >> 8;
            w.avgline(lo, hi, elabel_fire(etype));
        }
        27 => {
            let lo = w.inp.elem_min(s, l).wrapping_mul(75) >> 8;
            let hi = w.inp.elem_max(s, l).wrapping_mul(75) >> 8;
            w.avgline(lo, hi, STR_FIRE_DAMAGE);
        }
        28 => {
            let Some(mis) = w.inp.missile(d.missile) else {
                return false;
            };
            w.yards(mis.param1, STR_RADIUS, 20);
        }
        29 => {
            let Some(mis) = w.inp.missile(d.missile) else {
                return false;
            };
            let v =
                2i32.wrapping_mul(mis.range.wrapping_add(mis.lev_range.wrapping_mul(l - 1))) / 3;
            w.vt(v, STR_YARDS, false);
        }
        30 => {
            let Some(mis) = w.inp.missile(d.missile) else {
                return false;
            };
            let Some(m2) = w.inp.missile(mis.sub1) else {
                return false;
            };
            let f = m2.range.wrapping_add(m2.lev_range.wrapping_mul(l));
            w.sec(f, ta, false);
        }
        31 => {
            let k = w.inp.curse_divisor();
            let v = if k > 0 { a / k } else { a };
            w.sec(v, ta, false);
        }
        32 => {
            w.s(ta);
            w.tvt(a, tb, STR_PERCENT, true);
        }
        33 => {
            if l > 1 {
                w.s(ta);
                w.s(tb);
                w.nl();
            }
        }
        34 => w.summon_damage(r, s, l, true),
        35 => {
            w.s(ta);
            w.colon_space();
            w.a(&format!("{a}-{b}"));
            w.nl();
        }
        36 => {
            // Singular / plural text.
            if a != 0 {
                w.a(&num(a, false));
                w.s(if a == 1 { ta } else { tb });
                w.nl();
            }
        }
        37 => w.yards(a, ta, 10),
        38 => w.range2(a, b, ta, tb, false),
        39 => w.summon_damage(r, s, l, false),
        40 | 46 => {
            // Synergy header; 46 is gated on B.
            if kind == 46 && b == 0 {
                return false;
            }
            if ta == STR_DUMMY {
                return false;
            }
            let fmt = w.inp.string(ta);
            let arg = w.inp.string(tb);
            let t = wsp(256, &fmt, &[Arg::Str(&arg)]);
            w.colored(t, a, flag);
        }
        41 => {
            // The fire golem's A1 damage at the level of the
            // skill's summon.
            if (0..w.inp.monstats_count()).contains(&r.summon) {
                let c = if w.inp.monstats_count() > MON_FIRE_GOLEM {
                    MON_FIRE_GOLEM
                } else {
                    -1
                };
                let level = w.inp.monster_level(r.summon).unwrap_or(0);
                let (lo, hi) = w.inp.monster_a1(c, level).unwrap_or((0, 0));
                let (lo, hi) = (lo.wrapping_add(a), hi.wrapping_add(b));
                let t = elabel(etype);
                if t != 0 {
                    w.s(t);
                    w.a(&rng(lo, hi));
                    w.nl();
                }
            }
        }
        42 => {
            // A is the integer part, B the digit.
            if ta != STR_DUMMY && !(a == 0 && b == 0) {
                w.s(ta);
                w.colon_space();
                w.a(&frac(a, b, true));
                w.s(STR_SPACE);
                w.s(tb);
                w.nl();
            }
        }
        43 => w.range256(a, b, 10, ta, tb),
        44 => w.range256(a, b, 100, ta, tb),
        45 => {
            // Thorns.
            if l > 1 {
                let v = r.param1.wrapping_add((l - 2).wrapping_mul(r.param2));
                w.vt(v, STR_DAMAGE_RETURNED, false);
                w.s(STR_THORNS);
                w.nl();
            }
        }
        47 => w.range4(a, b, ta, false, false),
        48 => {
            // Blessed hammer, no texts.
            if w.inp.expansion() {
                w.length_line(s, l, etype);
                w.elem_line(s, l, 0, etype);
                w.phys_line(s, l, 0, 0);
                w.attack_line(s, l);
            } else {
                // Skill elements: poison × its length.
                let (mut lo, mut hi) = (w.inp.elem_min(s, l), w.inp.elem_max(s, l));
                if etype == 5 {
                    let n = w.inp.elem_len(s, l);
                    lo = lo.wrapping_mul(n);
                    hi = hi.wrapping_mul(n);
                }
                w.range4(lo >> 8, hi >> 8, STR_DAMAGE, false, false);
            }
        }
        49 => {
            let lo = w.inp.phys_min(s, l, true) >> 8;
            let hi = w.inp.phys_max(s, l, true) >> 8;
            w.range4(lo, hi, ta, true, true);
        }
        50 => {
            // A missile's elemental damage.
            let m = d.missile;
            let lo = w.inp.missile_elem_min(m, l) >> 8;
            let hi = w.inp.missile_elem_max(m, l) >> 8;
            if !(lo == 0 && hi == 0) {
                let me = w.inp.missile(m).map_or(0, |x| i32::from(x.etype));
                w.elemline(lo, hi, ta, me);
            }
        }
        51 => {
            // The dummy is printed.
            if ta == STR_DUMMY {
                w.s(STR_DUMMY);
            } else {
                let fmt = w.inp.string(ta);
                let t = wsp(256, &fmt, &[Arg::Int(a)]);
                w.units(&t);
            }
            w.nl();
        }
        52 => w.range2(a, b, ta, tb, true),
        // The quantity pop-up is replaced by the box's own
        // pop-up; the counters always return 0 (Edge case 6).
        53..=56 => return false,
        57 => w.sec(a, ta, true),
        58 => w.range3(a, b, ta, tb, true),
        59 => {
            w.s_opt(tb);
            w.elemline(a, b, ta, etype);
        }
        60 | 61 => {
            let d = if b == 0 { 10 } else { b };
            w.fix(a, d, ta, tb, kind == 60);
        }
        62 => w.range3(a, b, ta, tb, false),
        63 => w.colon(a, ta, tb, true, true),
        64 => {
            if ta != STR_DUMMY && a != 0 && b > 0 {
                w.s(ta);
                w.colon_space();
                w.a(&num(a, true));
                w.s(STR_SLASH);
                w.a(&format!("{b}"));
                w.s(STR_SPACE);
                w.s(tb);
                w.nl();
            }
        }
        65 => {
            if ta != STR_DUMMY && tb != STR_DUMMY {
                w.s(ta);
                w.colon_space();
                w.s(tb);
                w.nl();
            }
        }
        66 => {
            if ta != STR_DUMMY {
                let fmt = w.inp.string(ta);
                let t = wsp(256, &fmt, &[Arg::Int(a)]);
                w.units(&t);
                w.nl();
            }
        }
        67 => w.colon(a, ta, tb, true, false),
        68 => w.vtt(a, ta, tb, false),
        69 => {
            if ta != STR_DUMMY && a != 0 {
                w.s(ta);
                w.colon_space();
                w.s(tb);
                w.s(STR_SPACE);
                w.a(&num(a, false));
                w.nl();
            }
        }
        70 => {
            w.s(ta);
            w.tv(a, tb, true);
        }
        71 => {
            if ta == STR_DUMMY || tb == STR_DUMMY {
                return false;
            }
            w.s(ta);
            w.colon_space();
            let fmt = w.inp.string(tb);
            let t = wsp(100, &fmt, &[Arg::Int(a)]);
            w.units(&t);
            w.nl();
        }
        72 | 73 => {
            if a == 0 || b < 1 {
                return false;
            }
            w.a(&num(a, kind == 72));
            w.s(STR_SLASH);
            w.a(&format!("{b}"));
            w.s(STR_SPACE);
            w.s(ta);
            w.nl();
        }
        74 | 75 => {
            if ta == STR_DUMMY {
                return false;
            }
            let fmt = w.inp.string(ta);
            let t = if kind == 74 {
                wsp(100, &fmt, &[Arg::Int(a)])
            } else {
                wsp(100, &fmt, &[Arg::Int(a), Arg::Int(b)])
            };
            w.units(&t);
            w.nl();
        }
        // r4: type 0 and types past the jump table.
        _ => return false,
    }
    true
}

/// U has a class skill (a `skills` row of U's
/// class) with a base level ≠ 0. The game latches a true answer
/// and never clears it there (who
/// resets it is not read); this is the test without the latch, evaluated
/// at each call.
pub fn learned_class_skill(inp: &dyn HoverInputs) -> bool {
    let Some(class) = inp.class() else {
        return false;
    };
    (0..inp.skill_count()).any(|s| {
        inp.skill(s).is_some_and(|r| r.class == class)
            && inp.levels(s).is_some_and(|(base, _)| base != 0)
    })
}

// ---------------------------------------------------------------- model

/// The extra tables of the summon and curse lines (types 13, 31, 34, 39,
/// 41): `monstats`, `monlvl` and `difficultylevels` `AiCurseDivisor`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HoverTables {
    pub monstats: Arc<Vec<Monstats>>,
    pub monlvl: Arc<Vec<Monlvl>>,
    /// `AiCurseDivisor` per `difficultylevels` row.
    pub curse_divisors: Vec<i32>,
}

/// [`HoverInputs`] of the client model: the local player `key` through
/// [`DescUnits`], the `d2-sim` skills tables, `skilldesccode`, the
/// strings and [`HoverTables`].
pub struct ModelInputs<'a> {
    units: DescUnits<'a>,
    t: &'a SkillTables,
    code: &'a [u8],
    key: UnitKey,
    strings: &'a dyn StringLookup,
    extra: &'a HoverTables,
    difficulty: u8,
    expansion: bool,
}

impl<'a> ModelInputs<'a> {
    pub fn new(
        world: &'a ClientWorld,
        key: UnitKey,
        desc: &'a DescTables,
        t: &'a SkillTables,
        strings: &'a dyn StringLookup,
        extra: &'a HoverTables,
    ) -> Self {
        ModelInputs {
            units: DescUnits::new(world, key, desc),
            t,
            code: &desc.desc_code,
            key,
            strings,
            extra,
            difficulty: world.difficulty,
            expansion: world.expansion != 0,
        }
    }

    fn d(&self) -> usize {
        usize::from(self.difficulty.min(2))
    }
}

/// The client's Lflag is the game's flag only for game types 2 and 3, else 1;
/// the D2RUST client plays the single-player game, so 1.
const CLIENT_L_FLAG: bool = true;

impl HoverInputs for ModelInputs<'_> {
    fn string(&self, id: u16) -> Vec<u16> {
        self.strings
            .get_id(id)
            .map(<[u16]>::to_vec)
            .unwrap_or_default()
    }
    fn skill_count(&self) -> i32 {
        i32::try_from(self.t.skills.len()).unwrap_or(i32::MAX)
    }
    fn skill(&self, s: i32) -> Option<SkillRow> {
        self.t.skill(s).map(SkillRow::of)
    }
    fn desc(&self, s: i32) -> Option<DescRow> {
        let r = self.t.skill(s)?;
        self.t.skilldesc_of(r).map(DescRow::of)
    }
    fn missile(&self, m: i32) -> Option<MissileRow> {
        self.t.missile(m).map(MissileRow::of)
    }
    fn eval_desc(&mut self, calc: u32, s: i32, l: i32) -> i32 {
        eval_desc(
            &mut self.units,
            self.t,
            self.code,
            Some(self.key),
            calc,
            s,
            l,
        )
    }
    fn eval_skill(&mut self, calc: u32, s: i32, l: i32) -> i32 {
        eval_skill(&mut self.units, self.t, Some(self.key), calc, s, l)
    }
    fn elem_min(&mut self, s: i32, l: i32) -> i32 {
        elem_min(&mut self.units, self.t, Some(self.key), s, l, true)
    }
    fn elem_max(&mut self, s: i32, l: i32) -> i32 {
        elem_max(&mut self.units, self.t, Some(self.key), s, l, true)
    }
    fn elem_len(&mut self, s: i32, l: i32) -> i32 {
        elem_len(&mut self.units, self.t, Some(self.key), s, l)
    }
    fn phys_min(&mut self, s: i32, l: i32, src_dam: bool) -> i32 {
        phys_min(&mut self.units, self.t, Some(self.key), s, l, src_dam)
    }
    fn phys_max(&mut self, s: i32, l: i32, src_dam: bool) -> i32 {
        phys_max(&mut self.units, self.t, Some(self.key), s, l, src_dam)
    }
    fn to_hit(&mut self, s: i32, l: i32) -> i32 {
        to_hit(&mut self.units, self.t, Some(self.key), s, l)
    }
    fn missile_elem_min(&mut self, m: i32, l: i32) -> i32 {
        miss_elem_min(&mut self.units, self.t, None, Some(self.key), m, l)
    }
    fn missile_elem_max(&mut self, m: i32, l: i32) -> i32 {
        miss_elem_max(&mut self.units, self.t, None, Some(self.key), m, l)
    }
    fn stat(&self, stat: u16) -> i32 {
        self.units.stat(self.key, stat, 0)
    }
    fn levels(&self, s: i32) -> Option<(i32, i32)> {
        let list = self.units.skill_list(self.key);
        let e = list.iter().find(|e| e.skill == s && e.is_native())?;
        let level = |bonus| skill_level(&self.units, self.t, Some(self.key), Some(e), bonus);
        Some((level(false), level(true)))
    }
    fn class(&self) -> Option<u8> {
        (self.units.unit_type(self.key) == UnitType::Player)
            .then(|| u8::try_from(self.units.class_id(self.key)).ok())
            .flatten()
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn curse_divisor(&self) -> i32 {
        let rows = &self.extra.curse_divisors;
        let Some(last) = rows.len().checked_sub(1) else {
            return 0;
        };
        rows[usize::from(self.difficulty).min(last)]
    }
    fn monstats_count(&self) -> i32 {
        i32::try_from(self.extra.monstats.len()).unwrap_or(i32::MAX)
    }
    fn monster_level(&self, m: i32) -> Option<i32> {
        let row = self.extra.monstats.get(usize::try_from(m).ok()?)?;
        let v = [row.level, row.level_n, row.level_h][self.d()];
        Some(i32::from(v as i16))
    }
    fn monster_life(&self, m: i32, level: i32) -> Option<(i32, i32)> {
        let row = self.extra.monstats.get(usize::try_from(m).ok()?)?;
        if level < 0 || self.extra.monlvl.is_empty() {
            return None;
        }
        let s = d2_sim::monsters::init::stats_by_level(
            row,
            &self.extra.monlvl,
            CLIENT_L_FLAG,
            self.d(),
            level,
        );
        Some((s.min_hp, s.max_hp))
    }
    fn monster_a1(&self, m: i32, level: i32) -> Option<(i32, i32)> {
        let row = self.extra.monstats.get(usize::try_from(m).ok()?)?;
        if level < 0 || self.extra.monlvl.is_empty() {
            return None;
        }
        Some(d2_sim::monsters::init::a1_damage(
            row,
            &self.extra.monlvl,
            CLIENT_L_FLAG,
            self.d(),
            level,
        ))
    }
}

#[cfg(test)]
#[path = "skill_hover_lines_tests.rs"]
pub(crate) mod tests;
