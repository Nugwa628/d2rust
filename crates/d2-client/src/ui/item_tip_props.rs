//! The property block of the item tool tip: the shown list L, the lines of its
//! stats in description-list order, the `dgrp` groups and the damage groups,
//! the undead and indestructible lines and the label form. Unverified until the
//! `text-0002` capture cases run.

use super::item_tip_desc::{self as desc, sid, DescNames, Shape, Viewer};
use super::wformat::{format, Arg};

/// More fixed string ids.
pub mod pid {
    /// `,` (with SP: the one-line separator).
    pub const COMMA: u16 = 3852;
    /// `Damage to Undead`
    pub const UNDEAD: u16 = 3554;
    /// `Indestructible`
    pub const INDESTRUCTIBLE: u16 = 21240;
    /// `Enhanced Damage`
    pub const ENHANCED: u16 = 10023;
    /// `Adds %d-%d damage`
    pub const ADDS: u16 = 3623;
    /// Fire, lightning, magic, cold: (one value, range).
    pub const ELEMENTS: [(u16, u16); 4] = [(3612, 3613), (3616, 3617), (3618, 3619), (3614, 3615)];
    /// Poison: (one value, range).
    pub const POISON: (u16, u16) = (3620, 3621);
    /// Elixir lines (table): (stat, string).
    pub const ELIXIRS: [(u32, u16); 6] = [
        (0, 3498),
        (1, 3500),
        (2, 3499),
        (3, 3501),
        (9, 3502),
        (7, 3503),
    ];
}

/// One stat list: (stat, layer, list value), at most one entry per
/// (stat, layer), kept in (stat, layer) order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatList(Vec<(u16, u32, i32)>);

impl StatList {
    /// Adds `v` to (stat, layer).
    pub fn add(&mut self, stat: u16, layer: u32, v: i32) {
        match self.0.binary_search_by_key(&(stat, layer), |e| (e.0, e.1)) {
            Ok(i) => self.0[i].2 = self.0[i].2.wrapping_add(v),
            Err(i) => self.0.insert(i, (stat, layer, v)),
        }
    }

    /// Adds every entry of `other`.
    pub fn add_list(&mut self, other: &StatList) {
        for &(s, l, v) in &other.0 {
            self.add(s, l, v);
        }
    }

    pub fn get(&self, stat: u16, layer: u32) -> i32 {
        self.0
            .binary_search_by_key(&(stat, layer), |e| (e.0, e.1))
            .map_or(0, |i| self.0[i].2)
    }

    /// Whether the list holds stat `s` (any layer, any value).
    pub fn has(&self, stat: u16) -> bool {
        self.layers(stat).next().is_some()
    }

    /// (layer, value) of stat `s` in list order.
    pub fn layers(&self, stat: u16) -> impl Iterator<Item = (u32, i32)> + '_ {
        let from = self.0.partition_point(|e| e.0 < stat);
        self.0[from..]
            .iter()
            .take_while(move |e| e.0 == stat)
            .map(|e| (e.1, e.2))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The list values of one stream list: (stat, param, value). A stat's own entry
/// is sent as value >> `ValShift` (`valshift`); the partners written after 17,
/// 48, 50, 52, 54, 57 are sent unshifted.
pub fn stream_values(
    stats: &[d2_proto::item_bits::Stat],
    valshift: impl Fn(u16) -> u8,
) -> Vec<(u16, u32, i32)> {
    let mut out = Vec::with_capacity(stats.len());
    let mut partners: &[u16] = &[];
    for s in stats {
        let partner = partners.first() == Some(&s.stat);
        partners = if partner {
            &partners[1..]
        } else {
            match s.stat {
                17 => &[18],
                48 => &[49],
                50 => &[51],
                52 => &[53],
                54 => &[55, 56],
                57 => &[58, 59],
                _ => &[],
            }
        };
        let v = s.value() as i32;
        out.push((
            s.stat,
            s.param,
            if partner { v } else { v << valshift(s.stat) },
        ));
    }
    out
}

/// What reads of the item beyond its lists.
#[derive(Clone, Copy, Debug, Default)]
pub struct PropItem {
    /// Primary type 11 `elix`: (file index, stat 71).
    pub elixir: Option<(u32, i32)>,
    /// Is-a 57 `blun`.
    pub blunt: bool,
    /// I's stat 122.
    pub undead_stat: i32,
    /// R6's test: an item, `nodurability` 0, `durability` ≠ 0, stat
    /// 152 < 1 and max durability 0.
    pub indestructible: bool,
}

/// The arguments of the property line builders.
#[derive(Clone, Copy, Debug)]
pub struct PropArgs<'a> {
    /// Undead line wanted (`undead` 1).
    pub undead: bool,
    /// Multi-line (else one line with `, `).
    pub multi: bool,
    /// Label put before the text (empty: none).
    pub label: &'a [u16],
}

/// What does with stat `s`.
enum Group {
    Normal,
    Skip,
    Text(Vec<u16>),
}

/// The damage-group state of L.
struct Groups {
    min_max: Option<(i32, i32)>,
    enhanced: bool,
    elements: [Option<(i32, i32)>; 4],
    poison: Option<(i32, i32, i32, i32)>,
    /// The min / max damage line was handled (later 21–23 skipped).
    damage_done: bool,
}

const ELEMENT_STATS: [(u16, u16); 4] = [(48, 49), (50, 51), (52, 53), (54, 55)];

impl Groups {
    fn of(l: &StatList) -> Self {
        let g = |s: u16| l.get(s, 0);
        let on = |a: i32, b: i32| (a > 0 && b > 0).then_some((a, b));
        let min = if g(21) != 0 { g(21) } else { g(23) };
        let max = if g(22) != 0 { g(22) } else { g(24) };
        Groups {
            min_max: on(min, max),
            enhanced: g(18) > 0 && g(17) > 0,
            elements: ELEMENT_STATS.map(|(a, b)| on(g(a), g(b))),
            poison: on(g(57), g(58)).map(|(a, b)| (a, b, g(59), g(326))),
            damage_done: false,
        }
    }

    fn apply(&mut self, names: &dyn DescNames, l: &StatList, s: u16) -> Group {
        let fmt = |id: u16, args: &[Arg<'_>]| {
            format(1024, Some(&names.string(id)), args).unwrap_or_default()
        };
        match s {
            17 if self.enhanced => Group::Skip,
            59 | 58 if self.poison.is_some() => Group::Skip,
            49 | 51 | 53 | 55 if self.elements[usize::from((s - 49) / 2)].is_some() => Group::Skip,
            18 if self.enhanced => {
                let mut t = names.string(sid::PLUS);
                t.extend(desc::num(i64::from(l.get(18, 0))));
                t.extend(names.string(sid::PCT));
                t.extend(names.string(sid::SP));
                t.extend(names.string(pid::ENHANCED));
                Group::Text(t)
            }
            21..=23 if self.min_max.is_some() => {
                if self.damage_done {
                    return Group::Skip;
                }
                self.damage_done = true;
                let (min, max) = self.min_max.unwrap_or_default();
                if min >= max {
                    self.min_max = None;
                    Group::Normal
                } else {
                    Group::Text(fmt(pid::ADDS, &[Arg::Int(min), Arg::Int(max)]))
                }
            }
            24 if self.min_max.is_some() => Group::Skip,
            48 | 50 | 52 | 54 => {
                let k = usize::from((s - 48) / 2);
                let Some((min, max)) = self.elements[k] else {
                    return Group::Normal;
                };
                let (one, range) = pid::ELEMENTS[k];
                Group::Text(if min >= max {
                    fmt(one, &[Arg::Int(max)])
                } else {
                    fmt(range, &[Arg::Int(min), Arg::Int(max)])
                })
            }
            57 => {
                let Some((min, max, length, count)) = self.poison else {
                    return Group::Normal;
                };
                let c = count.max(1);
                let len = length / c;
                let lo = (min.wrapping_mul(len) + 128) >> 8;
                let hi = (max.wrapping_mul(len) + 128) >> 8;
                let sec = len / 25;
                Group::Text(if lo >= hi {
                    fmt(pid::POISON.0, &[Arg::Int(hi), Arg::Int(sec)])
                } else {
                    fmt(pid::POISON.1, &[Arg::Int(lo), Arg::Int(hi), Arg::Int(sec)])
                })
            }
            _ => Group::Normal,
        }
    }
}

/// The line of stat `s` at `layer` with list value `v`.
fn stat_line(
    names: &dyn DescNames,
    viewer: &Viewer,
    l: &StatList,
    item: &PropItem,
    s: u16,
    layer: u32,
    v: i32,
) -> Option<Vec<u16>> {
    let d = names.stat_desc(s)?;
    let val = desc::value(names, viewer, s, v, item.blunt);
    let mut shape = Shape::own(&d);
    if d.dgrp != 0 {
        let members = names.group_members(d.dgrp);
        let same = members
            .iter()
            .all(|&m| desc::value(names, viewer, m, l.get(m, 0), item.blunt) == val);
        if same {
            if members.iter().min() != Some(&s) {
                return None;
            }
            shape = Shape::group(&d);
        }
    }
    desc::line(names, viewer, &shape, val, layer)
}

/// The elixir text.
fn elixir_text(names: &dyn DescNames, file_index: u32, value: i32) -> Vec<u16> {
    let mut t = Vec::new();
    for &(stat, id) in &pid::ELIXIRS {
        if stat != file_index {
            continue;
        }
        let v = if (6..=11).contains(&stat) {
            value >> 8
        } else {
            value
        };
        if v == 0 {
            continue;
        }
        let mut n = desc::num(i64::from(v));
        n.truncate(8);
        t.extend(names.string(id));
        t.extend(names.string(sid::SP));
        if v > 0 {
            t.extend(names.string(sid::PLUS));
        }
        t.extend(n);
        t.extend(names.string(sid::NL));
    }
    t
}

/// The property text T of list `l` (already summed per
/// R2).
pub fn property_text(
    names: &dyn DescNames,
    viewer: &Viewer,
    item: &PropItem,
    l: &StatList,
    args: &PropArgs,
) -> Vec<u16> {
    if let Some((file_index, value)) = item.elixir {
        return elixir_text(names, file_index, value);
    }
    let nl = names.string(sid::NL);
    let sep = [names.string(pid::COMMA), names.string(sid::SP)].concat();
    let mut t = Vec::new();
    let mut first = true;
    let mut push = |t: &mut Vec<u16>, line: Vec<u16>| {
        if args.multi {
            t.extend(line);
            t.extend(&nl);
        } else {
            if !first {
                t.extend(&sep);
            }
            t.extend(line);
        }
        first = false;
    };
    // r3.
    if args.undead && item.blunt && item.undead_stat == 0 {
        let mut line = names.string(sid::PLUS);
        line.extend(desc::num(50));
        line.extend(names.string(sid::PCT));
        line.extend(names.string(sid::SP));
        line.extend(names.string(pid::UNDEAD));
        push(&mut t, line);
    }
    // r4–r5.
    let mut groups = Groups::of(l);
    for s in names.desc_list() {
        for (layer, v) in l.layers(s).filter(|e| e.1 != 0) {
            match groups.apply(names, l, s) {
                Group::Skip => continue,
                Group::Text(g) => {
                    t.extend(g);
                    continue;
                }
                Group::Normal => {}
            }
            let Some(line) = stat_line(names, viewer, l, item, s, layer, v) else {
                continue;
            };
            if (s == 23 && l.has(21)) || (s == 24 && l.has(22)) {
                continue;
            }
            push(&mut t, line);
        }
    }
    // r6.
    if item.indestructible {
        push(&mut t, names.string(pid::INDESTRUCTIBLE));
    }
    t
}

/// T with the property block label, to append to the block.
pub fn property_block(
    names: &dyn DescNames,
    viewer: &Viewer,
    item: &PropItem,
    l: &StatList,
    args: &PropArgs,
) -> Vec<u16> {
    let t = property_text(names, viewer, item, l, args);
    if args.label.is_empty() || t.is_empty() {
        return t;
    }
    let lf = u16::from(b'\n');
    let inner = &t[..t.len() - 1];
    let Some(k) = inner.iter().rposition(|&u| u == lf) else {
        return [args.label, &t].concat();
    };
    let last = if t.last() == Some(&lf) {
        &t[k + 1..t.len() - 1]
    } else {
        &t[k + 1..]
    };
    [
        &t[..k],
        &names.string(sid::NL)[..],
        args.label,
        last,
        &names.string(pid::COMMA),
        &names.string(sid::NL),
    ]
    .concat()
}

#[cfg(test)]
mod tests;
