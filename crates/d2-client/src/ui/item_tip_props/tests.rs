use super::*;
use crate::ui::item_tip_desc::{ClassStrings, StatDesc};
use std::collections::BTreeMap;

/// Synthetic tables: the game's English texts of the strings the
/// vectors use, and description rows with the shapes lists.
struct Names {
    strings: BTreeMap<u16, String>,
    stats: BTreeMap<u16, StatDesc>,
}

impl DescNames for Names {
    fn string(&self, id: u16) -> Vec<u16> {
        self.strings
            .get(&id)
            .map_or_else(Vec::new, |s| s.encode_utf16().collect())
    }
    fn stat_desc(&self, s: u16) -> Option<StatDesc> {
        self.stats.get(&s).copied()
    }
    fn skill_name(&self, _: u32) -> Option<u16> {
        None
    }
    fn skill_class(&self, _: u32) -> Option<i8> {
        None
    }
    fn skill_count(&self) -> u32 {
        0
    }
    fn class_strings(&self, _: u32) -> Option<ClassStrings> {
        None
    }
    fn montype_name(&self, _: u32) -> Option<u16> {
        None
    }
    fn monstats_name(&self, _: u32) -> Option<u16> {
        None
    }
    fn desc_list(&self) -> Vec<u16> {
        let mut v: Vec<(u16, u16)> = self
            .stats
            .iter()
            .filter(|(_, d)| d.func != 0)
            .map(|(&s, d)| (d.priority, s))
            .collect();
        v.sort();
        v.into_iter().map(|(_, s)| s).collect()
    }
    fn group_members(&self, g: u16) -> Vec<u16> {
        self.stats
            .iter()
            .filter(|(_, d)| d.dgrp == g)
            .map(|(&s, _)| s)
            .collect()
    }
}

fn row(priority: u16, func: u8, val: u8, pos: u16) -> StatDesc {
    StatDesc {
        priority,
        func,
        val,
        pos,
        neg: pos,
        ..StatDesc::default()
    }
}

fn names() -> Names {
    let strings = [
        (sid::PCT, "%"),
        (sid::PLUS, "+"),
        (sid::SP, " "),
        (sid::NL, "\n"),
        (pid::COMMA, ","),
        (pid::UNDEAD, "Damage to Undead"),
        (pid::INDESTRUCTIBLE, "Indestructible"),
        (pid::ENHANCED, "Enhanced Damage\n"),
        (pid::ADDS, "Adds %d-%d damage\n"),
        (3612, "+%d fire damage\n"),
        (3613, "Adds %d-%d fire damage\n"),
        (3620, "+%d poison damage over %d seconds\n"),
        (3621, "Adds %d-%d poison damage over %d seconds\n"),
        (10977, "to all Attributes"),
        (10024, "All Resistances +%d"),
        (700, "to Strength"),
        (701, "to Energy"),
        (702, "to Dexterity"),
        (703, "to Vitality"),
        (710, "Fire Resist"),
        (711, "Lightning Resist"),
        (712, "Cold Resist"),
        (713, "Poison Resist"),
        (720, "to Minimum Damage"),
        (721, "to Maximum Damage"),
        (722, "Enhanced Maximum Damage"),
        (723, "Enhanced Minimum Damage"),
        (724, "to Fire Damage"),
        (725, "to Maximum Fire Damage"),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_owned()))
    .collect();
    let mut stats = BTreeMap::new();
    // dgrp 1: stats 0–3, group shape f 1 dv 1 `to all Attributes`.
    for (s, id) in [(0u16, 700u16), (1, 701), (2, 702), (3, 703)] {
        stats.insert(
            s,
            StatDesc {
                dgrp: 1,
                dgrpfunc: 1,
                dgrpval: 1,
                dgrppos: 10977,
                dgrpneg: 10977,
                ..row(67 - s, 1, 1, id)
            },
        );
    }
    // dgrp 2: resistances, group shape f 19 `All Resistances +%d`.
    for (s, id) in [(39u16, 710u16), (41, 711), (43, 712), (45, 713)] {
        stats.insert(
            s,
            StatDesc {
                dgrp: 2,
                dgrpfunc: 19,
                dgrppos: 10024,
                dgrpneg: 10024,
                ..row(36 - (s - 39) / 2, 4, 2, id)
            },
        );
    }
    stats.insert(17, row(129, 4, 2, 722));
    stats.insert(18, row(130, 4, 2, 723));
    stats.insert(21, row(126, 1, 1, 720));
    stats.insert(22, row(127, 1, 1, 721));
    stats.insert(23, row(126, 1, 1, 720));
    stats.insert(24, row(127, 1, 1, 721));
    stats.insert(48, row(102, 1, 1, 724));
    stats.insert(49, row(101, 1, 1, 725));
    stats.insert(57, row(92, 1, 1, 724));
    stats.insert(58, row(92, 1, 1, 725));
    stats.insert(59, row(0, 0, 0, 0));
    stats.insert(326, row(0, 0, 0, 0));
    Names { strings, stats }
}

fn list(entries: &[(u16, i32)]) -> StatList {
    let mut l = StatList::default();
    for &(s, v) in entries {
        l.add(s, 0, v);
    }
    l
}

fn multi(n: &Names, item: &PropItem, l: &StatList) -> String {
    let args = PropArgs {
        undead: true,
        multi: true,
        label: &[],
    };
    String::from_utf16_lossy(&property_text(n, &Viewer::default(), item, l, &args))
}

#[test]
fn damage_groups_print_one_line_per_pair() {
    let n = names();
    let item = PropItem::default();
    assert_eq!(
        multi(&n, &item, &list(&[(48, 10), (49, 16)])),
        "Adds 10-16 fire damage\n"
    );
    assert_eq!(
        multi(&n, &item, &list(&[(48, 16), (49, 16)])),
        "+16 fire damage\n"
    );
    assert_eq!(
        multi(&n, &item, &list(&[(18, 50), (17, 50)])),
        "+50% Enhanced Damage\n"
    );
    assert_eq!(
        multi(
            &n,
            &item,
            &list(&[(57, 256), (58, 512), (59, 75), (326, 0)])
        ),
        "Adds 75-150 poison damage over 3 seconds\n"
    );
    assert_eq!(
        multi(&n, &item, &list(&[(21, 3), (22, 9)])),
        "Adds 3-9 damage\n"
    );
    // A half pair prints its normal line.
    assert_eq!(multi(&n, &item, &list(&[(48, 10)])), "+10 to Fire Damage\n");
}

// min ≥ max clears the pair: the normal lines print (r5's 23 / 24
// skips still apply).
#[test]
fn min_at_least_max_prints_the_normal_lines() {
    let n = names();
    let item = PropItem::default();
    assert_eq!(
        multi(&n, &item, &list(&[(21, 9), (22, 9), (23, 9), (24, 9)])),
        "+9 to Minimum Damage\n+9 to Maximum Damage\n"
    );
}

#[test]
fn dgrp_groups_print_once_when_all_values_match() {
    let n = names();
    let item = PropItem::default();
    let res = |c: i32| list(&[(39, 15), (41, 15), (43, c), (45, 15)]);
    assert_eq!(multi(&n, &item, &res(15)), "All Resistances +15\n");
    assert_eq!(
        multi(&n, &item, &res(10)),
        "Poison Resist +15%\nCold Resist +10%\nLightning Resist +15%\nFire Resist +15%\n"
    );
    assert_eq!(
        multi(&n, &item, &list(&[(0, 5), (1, 5), (2, 5), (3, 5)])),
        "+5 to all Attributes\n"
    );
}

#[test]
fn undead_separator_indestructible_and_label() {
    let n = names();
    let blunt = PropItem {
        blunt: true,
        indestructible: true,
        ..PropItem::default()
    };
    let l = list(&[(0, 5)]);
    assert_eq!(
        multi(&n, &blunt, &l),
        "+50% Damage to Undead\n+5 to Strength\nIndestructible\n"
    );
    let one = PropArgs {
        undead: false,
        multi: false,
        label: &[],
    };
    let t = property_text(&n, &Viewer::default(), &blunt, &l, &one);
    assert_eq!(
        String::from_utf16_lossy(&t),
        "+5 to Strength, Indestructible"
    );
    // Label on a one-line text goes in front; on several lines the top
    // line takes the label and a trailing `,`.
    let label: Vec<u16> = "Weapons: ".encode_utf16().collect();
    let lab = PropArgs {
        undead: false,
        multi: false,
        label: &label,
    };
    let t = property_block(&n, &Viewer::default(), &PropItem::default(), &l, &lab);
    assert_eq!(String::from_utf16_lossy(&t), "Weapons: +5 to Strength");
    let lab = PropArgs { multi: true, ..lab };
    let l2 = list(&[(0, 5), (39, 7)]);
    let t = property_block(&n, &Viewer::default(), &PropItem::default(), &l2, &lab);
    assert_eq!(
        String::from_utf16_lossy(&t),
        "Fire Resist +7%\nWeapons: +5 to Strength,\n"
    );
}

#[test]
fn elixir_text_by_file_index() {
    let mut n = names();
    n.strings.insert(3503, "Elixir of Life".into());
    let item = PropItem {
        elixir: Some((7, 512)),
        ..PropItem::default()
    };
    assert_eq!(multi(&n, &item, &list(&[(0, 5)])), "Elixir of Life +2\n");
    let none = PropItem {
        elixir: Some((7, 0)),
        ..PropItem::default()
    };
    assert_eq!(multi(&n, &none, &StatList::default()), "");
}

// Own entries are shifted back by `ValShift`; the partners after a
// grouped stat are not.
#[test]
fn stream_values_shift_own_entries_not_partners() {
    let st = |stat, raw| d2_proto::item_bits::Stat {
        stat,
        param: 0,
        raw,
        save_add: 0,
    };
    let shift = |s: u16| if s == 7 || s == 49 { 8 } else { 0 };
    let got = stream_values(&[st(48, 10), st(49, 16), st(7, 3), st(49, 2)], shift);
    assert_eq!(
        got,
        vec![(48, 0, 10), (49, 0, 16), (7, 0, 3 << 8), (49, 0, 2 << 8)]
    );
}
