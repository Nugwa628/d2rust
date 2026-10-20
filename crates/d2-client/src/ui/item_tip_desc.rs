//! One property line of the item tool tip ([`super::item_tip`]): the
//! value of the line and the 28 `descfunc` shapes, with every fixed
//! phrase taken from the string tables by id.
//!
//! Text is UTF-16 as the game builds it; numbers are `%i` decimal.
//! Unverified until the text capture cases run.

use super::wformat::{format, Arg};

/// Fixed string ids of the property lines (English text in the comments).
pub mod sid {
    /// `%`
    pub const PCT: u16 = 4001;
    /// `+`
    pub const PLUS: u16 = 4002;
    /// `to`
    pub const TO: u16 = 4003;
    /// space
    pub const SP: u16 = 3995;
    /// `-`
    pub const DASH: u16 = 3996;
    /// `:`
    pub const COLON: u16 = 3997;
    /// LF
    pub const NL: u16 = 3998;
    /// `an evil force`: the name of a missing skill.
    pub const EVIL: u16 = 5382;
    /// `(Based on Character Level)`
    pub const BASED: u16 = 11091;
    /// `Repairs %d durability per second`
    pub const REPAIR_SEC: u16 = 21241;
    /// `Repairs %d durability in %d seconds`
    pub const REPAIR_IN: u16 = 21242;
    /// `Level`
    pub const LEVEL: u16 = 21249;
    /// By-time period lines for p = 0 … 3 (`descfunc` 17).
    pub const PERIODS: [u16; 4] = [21235, 21237, 21234, 21236];
}

/// The description columns of an `itemstatcost` row that the lines read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatDesc {
    pub priority: u16,
    pub func: u8,
    pub val: u8,
    pub pos: u16,
    pub neg: u16,
    pub str2: u16,
    pub dgrp: u16,
    pub dgrpfunc: u8,
    pub dgrpval: u8,
    pub dgrppos: u16,
    pub dgrpneg: u16,
    pub dgrpstr2: u16,
    pub op: u8,
    pub op_param: u8,
    pub op_base: u16,
    pub valshift: u8,
    /// `op stat1` … `op stat3` (the op targets).
    pub op_stats: [u16; 3],
}

/// The `charstats` strings of a class (`descfunc` 13, 14, 27).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClassStrings {
    pub all_skills: u16,
    pub tabs: [u16; 3],
    pub class_only: u16,
}

/// What the shapes read beyond the stat's own row.
pub trait DescNames {
    /// String `id`'s text (empty when missing).
    fn string(&self, id: u16) -> Vec<u16>;
    /// The `itemstatcost` description row of stat `s` (`None`: invalid
    /// stat).
    fn stat_desc(&self, s: u16) -> Option<StatDesc>;
    /// The skilldesc `str name` id of `skill`; `None` on
    /// any miss.
    fn skill_name(&self, skill: u32) -> Option<u16>;
    /// The skills row's `charclass` (+0x0C, signed; −1 classless);
    /// `None`: no skills row.
    fn skill_class(&self, skill: u32) -> Option<i8>;
    /// Rows of the skills table.
    fn skill_count(&self) -> u32;
    /// The charstats strings of class row `class`.
    fn class_strings(&self, class: u32) -> Option<ClassStrings>;
    /// Montype row `row`'s `strplur` (+0x0A).
    fn montype_name(&self, row: u32) -> Option<u16>;
    /// Monstats row `row`'s `NameStr`.
    fn monstats_name(&self, row: u32) -> Option<u16>;
    /// The description list: stats with
    /// `descfunc` ≠ 0 by ascending `descpriority`.
    fn desc_list(&self) -> Vec<u16> {
        Vec::new()
    }
    /// The stats whose `dgrp` is `g`.
    fn group_members(&self, g: u16) -> Vec<u16> {
        let _ = g;
        Vec::new()
    }
}

/// The units a line may read (op base, `descfunc` 17 / 18 / 28).
#[derive(Clone, Copy, Default)]
pub struct Viewer<'a> {
    /// The local player P's total of (stat, layer) (`None`: no player).
    pub player: Option<&'a dyn Fn(u16, u16) -> i32>,
    /// P's class (0–6).
    pub player_class: Option<u8>,
    /// The described unit's class when it is a player.
    pub unit_class: Option<u8>,
    /// The client act's base time for by-time values (`None`: no act).
    pub act_time: Option<i32>,
}

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// `%i` of `v`.
pub fn num(v: i64) -> Vec<u16> {
    w(&v.to_string())
}

/// The shown value of stat `s` with list value `v` (`blunt`:
/// the item is-a 57 `blun`, for stat 122).
pub fn value(names: &dyn DescNames, viewer: &Viewer, s: u16, v: i32, blunt: bool) -> i64 {
    let Some(d) = names.stat_desc(s) else {
        return i64::from(v);
    };
    let mut v = i64::from(v);
    if (2..=5).contains(&d.op) {
        let base_shift = names.stat_desc(d.op_base).map_or(0, |b| b.valshift);
        let base = viewer.player.map_or(0, |p| p(d.op_base, 0));
        v = ((i64::from(base) >> base_shift) * v) >> d.op_param;
    }
    v >>= d.valshift;
    if s == 122 && blunt {
        v += 50;
    }
    v
}

/// The strings and shape one line is printed with (the group columns when the
/// stat prints its group).
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub func: u8,
    pub val: u8,
    pub pos: u16,
    pub neg: u16,
    pub str2: u16,
}

impl Shape {
    pub fn own(d: &StatDesc) -> Self {
        Shape {
            func: d.func,
            val: d.val,
            pos: d.pos,
            neg: d.neg,
            str2: d.str2,
        }
    }
    pub fn group(d: &StatDesc) -> Self {
        Shape {
            func: d.dgrpfunc,
            val: d.dgrpval,
            pos: d.dgrppos,
            neg: d.dgrpneg,
            str2: d.dgrpstr2,
        }
    }
}

/// `+N` when `plus`, else N.
fn signed(names: &dyn DescNames, v: i64, plus: bool) -> Vec<u16> {
    let mut out = Vec::new();
    if plus {
        out.extend(names.string(sid::PLUS));
    }
    out.extend(num(v));
    out
}

/// Value part and str placed by `descval`.
fn place(names: &dyn DescNames, dv: u8, part: Vec<u16>, s: &[u16]) -> Vec<u16> {
    let sp = names.string(sid::SP);
    match dv {
        0 => s.to_vec(),
        2 => [s, &sp, &part].concat(),
        _ => [&part, &sp, s].concat(),
    }
}

fn fmt(f: &[u16], args: &[Arg<'_>]) -> Vec<u16> {
    format(1024, Some(f), args).unwrap_or_default()
}

fn skill_text(names: &dyn DescNames, skill: u32) -> Vec<u16> {
    names.string(names.skill_name(skill).unwrap_or(sid::EVIL))
}

/// The line of stat value `v` (already) at `layer` with
/// shape `sh`. `None`: no line.
pub fn line(
    names: &dyn DescNames,
    viewer: &Viewer,
    sh: &Shape,
    v: i64,
    layer: u32,
) -> Option<Vec<u16>> {
    let str_ = names.string(if v >= 0 { sh.pos } else { sh.neg });
    let pct = names.string(sid::PCT);
    let sp = names.string(sid::SP);
    let n = num(v);
    let vi = v as i32;
    let mut text = match sh.func {
        1 | 6 => place(names, sh.val, signed(names, v, v > 0), &str_),
        12 => {
            let part = if v == 1 {
                Vec::new()
            } else {
                signed(names, v, v > 0)
            };
            place(names, sh.val, part, &str_)
        }
        2 | 7 => place(names, sh.val, [&n[..], &pct].concat(), &str_),
        3 | 9 => place(names, sh.val, n, &str_),
        4 | 8 => {
            let part = [&signed(names, v, v >= 0)[..], &pct].concat();
            place(names, sh.val, part, &str_)
        }
        5 | 10 => {
            let part = [&num(v * 100 / 128)[..], &pct].concat();
            place(names, sh.val, part, &str_)
        }
        11 => {
            if v > 0 {
                let t = 2500 / v;
                if t <= 30 {
                    fmt(&names.string(sid::REPAIR_SEC), &[Arg::Int(1)])
                } else {
                    let secs = ((t + 12) / 25) as i32;
                    fmt(
                        &names.string(sid::REPAIR_IN),
                        &[Arg::Int(1), Arg::Int(secs)],
                    )
                }
            } else {
                fmt(&names.string(sid::REPAIR_SEC), &[Arg::Int(25)])
            }
        }
        13 => {
            if v == 0 {
                return None;
            }
            let c = names.class_strings(layer)?;
            let s = names.string(c.all_skills);
            match sh.val {
                0 => Vec::new(),
                dv => place(names, dv, signed(names, v, v > 0), &s),
            }
        }
        14 => {
            let tab = layer & 7;
            if tab > 2 {
                return None;
            }
            let c = names.class_strings(layer >> 3)?;
            let mut out = fmt(&names.string(c.tabs[tab as usize]), &[Arg::Int(vi)]);
            out.extend(&sp);
            out.extend(names.string(c.class_only));
            out
        }
        15 | 24 => {
            let (skill, level) = (layer >> 6, (layer & 0x3F) as i32);
            if skill == 0 || skill >= names.skill_count() {
                return None;
            }
            let name = skill_text(names, skill);
            if sh.func == 15 {
                // `%%` eats the 0.
                fmt(
                    &names.string(sh.pos),
                    &[Arg::Int(vi), Arg::Int(0), Arg::Int(level), Arg::Str(&name)],
                )
            } else {
                let charges = fmt(&str_, &[Arg::Int(vi & 0xFF), Arg::Int(vi >> 8)]);
                [
                    &names.string(sid::LEVEL)[..],
                    &sp,
                    &num(i64::from(level)),
                    &sp,
                    &name,
                    &sp,
                    &charges,
                ]
                .concat()
            }
        }
        16 => {
            let name = skill_text(names, layer);
            fmt(&str_, &[Arg::Int(vi), Arg::Str(&name)])
        }
        17 | 18 => {
            let p = (vi & 3) as usize;
            let mut out = names.string(sid::PERIODS[p]);
            out.extend(names.string(sid::NL));
            let x = match viewer.act_time {
                Some(t) => i64::from(d2_sim::stats::by_time(vi, t)),
                None => i64::from(((vi >> 2) & 0x3FF) - 256),
            };
            let mut part = if x >= 0 {
                signed(names, x, true)
            } else if v < 0 {
                num(x)
            } else {
                Vec::new()
            };
            if sh.func == 18 {
                part.extend(&pct);
            }
            if sh.val != 0 {
                out.extend(place(names, sh.val, part, &str_));
            }
            out
        }
        19 => fmt(&str_, &[Arg::Int(vi)]),
        20 | 21 => {
            let m = -v;
            let part = [&signed(names, m, m >= 0)[..], &pct].concat();
            place(names, sh.val, part, &str_)
        }
        22 => {
            let part = [&signed(names, v, v >= 0)[..], &pct].concat();
            let mut out = place(names, sh.val, part, &str_);
            out.extend(names.string(sid::COLON));
            out.extend(&sp);
            let mt = names.montype_name(layer).or_else(|| names.montype_name(0));
            out.extend(mt.map(|id| names.string(id)).unwrap_or_default());
            out
        }
        23 => {
            let name = names.monstats_name(layer)?;
            let mut out = place(names, sh.val, [&n[..], &pct].concat(), &str_);
            out.extend(&sp);
            out.extend(names.string(name));
            out
        }
        25 | 26 => place(names, sh.val, signed(names, v, v < 0), &str_),
        27 => {
            let name = names.skill_name(layer).filter(|_| v != 0)?;
            let mut out = signed(names, v, v > 0);
            out.extend(&sp);
            out.extend(names.string(sid::TO));
            out.extend(&sp);
            out.extend(names.string(name));
            out.extend(&sp);
            let class = names.skill_class(layer).unwrap_or(-1);
            if (0..=6).contains(&class) {
                if let Some(c) = names.class_strings(class as u32) {
                    out.extend(names.string(c.class_only));
                }
            }
            out
        }
        28 => {
            let class = names.skill_class(layer).filter(|_| v != 0)?;
            let unit = viewer.unit_class.or(viewer.player_class);
            let v = if unit.is_some_and(|u| i32::from(class) == i32::from(u)) && v > 3 {
                3
            } else {
                v
            };
            let mut out = signed(names, v, v > 0);
            out.extend(&sp);
            out.extend(names.string(sid::TO));
            out.extend(&sp);
            out.extend(skill_text(names, layer));
            out
        }
        _ => return None,
    };
    if matches!(sh.func, 6..=10 | 21) {
        text.extend(&sp);
        let id = if sh.str2 == sid::EVIL {
            sid::BASED
        } else {
            sh.str2
        };
        text.extend(names.string(id));
    }
    Some(text)
}

#[cfg(test)]
mod tests;
