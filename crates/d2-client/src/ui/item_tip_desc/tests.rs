use super::*;
use std::collections::HashMap;

/// Synthetic tables with the game's English texts the vectors name.
pub(crate) struct Names {
    pub strings: HashMap<u16, String>,
    pub stats: HashMap<u16, StatDesc>,
}

impl DescNames for Names {
    fn string(&self, id: u16) -> Vec<u16> {
        self.strings.get(&id).map_or_else(Vec::new, |s| w(s))
    }
    fn stat_desc(&self, s: u16) -> Option<StatDesc> {
        self.stats.get(&s).copied()
    }
    fn skill_name(&self, skill: u32) -> Option<u16> {
        (skill == 36).then_some(900)
    }
    fn skill_class(&self, skill: u32) -> Option<i8> {
        (skill == 36).then_some(1)
    }
    fn skill_count(&self) -> u32 {
        357
    }
    fn class_strings(&self, class: u32) -> Option<ClassStrings> {
        match class {
            1 => Some(ClassStrings {
                all_skills: 801,
                tabs: [811, 812, 813],
                class_only: 821,
            }),
            3 => Some(ClassStrings {
                all_skills: 803,
                ..ClassStrings::default()
            }),
            5 => Some(ClassStrings {
                all_skills: 805,
                ..ClassStrings::default()
            }),
            _ => None,
        }
    }
    fn montype_name(&self, _: u32) -> Option<u16> {
        None
    }
    fn monstats_name(&self, _: u32) -> Option<u16> {
        None
    }
}

pub(crate) fn names() -> Names {
    let strings = [
        (sid::PCT, "%"),
        (sid::PLUS, "+"),
        (sid::TO, "to"),
        (sid::SP, " "),
        (sid::COLON, ":"),
        (sid::NL, "\n"),
        (sid::EVIL, "an evil force"),
        (sid::BASED, "(Based on Character Level)"),
        (sid::REPAIR_SEC, "Repairs %d durability per second"),
        (sid::REPAIR_IN, "Repairs %d durability in %d seconds"),
        (sid::LEVEL, "Level"),
        (700, "to Strength"),
        (701, "Fire Resist"),
        (702, "better chance of getting magic item"),
        (703, "%d%% Chance to cast level %d %s on attack"),
        (704, "(%d/%d Charges)"),
        (705, "Defense"),
        (706, "Target Defense"),
        (707, "Hit blinds target"),
        (801, "to Sorceress Skill Levels"),
        (803, "to Paladin Skill Levels"),
        (805, "to Druid Skills"),
        (811, "+%d to Fire Skills"),
        (821, "(Sorceress Only)"),
        (900, "Fire Bolt"),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_owned()))
    .collect();
    Names {
        strings,
        stats: HashMap::new(),
    }
}

fn shape(func: u8, val: u8, pos: u16, str2: u16) -> Shape {
    Shape {
        func,
        val,
        pos,
        neg: pos,
        str2,
    }
}

fn text(n: &Names, v: &Viewer, sh: Shape, value: i64, layer: u32) -> Option<String> {
    line(n, v, &sh, value, layer).map(|t| String::from_utf16_lossy(&t))
}

// The spec's single-line vectors.
#[test]
fn spec_vectors_one_line() {
    let n = names();
    let v = Viewer::default();
    let s = |sh, value, layer| text(&n, &v, sh, value, layer);
    assert_eq!(
        s(shape(1, 1, 700, 0), 5, 0).as_deref(),
        Some("+5 to Strength")
    );
    assert_eq!(
        s(shape(1, 1, 700, 0), -3, 0).as_deref(),
        Some("-3 to Strength")
    );
    assert_eq!(
        s(shape(4, 2, 701, 0), 30, 0).as_deref(),
        Some("Fire Resist +30%")
    );
    assert_eq!(
        s(shape(2, 1, 702, 0), 25, 0).as_deref(),
        Some("25% better chance of getting magic item")
    );
    assert_eq!(
        s(shape(20, 1, 706, 0), 25, 0).as_deref(),
        Some("-25% Target Defense")
    );
    // f 12 dv 2 with v = 1: empty value part after the SP.
    assert_eq!(
        s(shape(12, 2, 707, 0), 1, 0).as_deref(),
        Some("Hit blinds target ")
    );
}

#[test]
fn f11_repair_lines() {
    let n = names();
    let v = Viewer::default();
    let s = |value| text(&n, &v, shape(11, 0, 0, 0), value, 0);
    assert_eq!(s(1).as_deref(), Some("Repairs 1 durability in 100 seconds"));
    assert_eq!(s(100).as_deref(), Some("Repairs 1 durability per second"));
    assert_eq!(s(-2).as_deref(), Some("Repairs 25 durability per second"));
}

#[test]
fn f13_f14_class_skill_lines() {
    let n = names();
    let v = Viewer::default();
    let s = |sh, value, layer| text(&n, &v, sh, value, layer);
    assert_eq!(
        s(shape(13, 1, 0, 0), 2, 3).as_deref(),
        Some("+2 to Paladin Skill Levels")
    );
    assert_eq!(
        s(shape(13, 1, 0, 0), 2, 5).as_deref(),
        Some("+2 to Druid Skills")
    );
    assert_eq!(s(shape(13, 1, 0, 0), 0, 3), None, "v = 0");
    assert_eq!(s(shape(13, 0, 0, 0), 2, 3).as_deref(), Some(""), "dv 0");
    // Layer 8: Sorceress (8 >> 3 = 1), tab 0.
    assert_eq!(
        s(shape(14, 0, 0, 0), 3, 8).as_deref(),
        Some("+3 to Fire Skills (Sorceress Only)")
    );
    assert_eq!(s(shape(14, 0, 0, 0), 3, 8 + 3), None, "tab > 2");
}

// The layer splits as skill = layer >> 6, level = layer & 0x3F. Replaces
// the old `each_descfunc_gives_its_line` cases, which asserted D2RUST's own
// English shapes.
#[test]
fn f15_f24_split_the_layer_skill_high_level_low() {
    let n = names();
    let v = Viewer::default();
    assert_eq!(
        text(&n, &v, shape(15, 0, 703, 0), 10, (36 << 6) + 3).as_deref(),
        Some("10% Chance to cast level 3 Fire Bolt on attack")
    );
    assert_eq!(
        text(&n, &v, shape(24, 0, 704, 0), 10 * 256 + 7, (36 << 6) + 5).as_deref(),
        Some("Level 5 Fire Bolt (7/10 Charges)")
    );
    // Skill 0 and skills past the table give no line.
    assert_eq!(text(&n, &v, shape(15, 0, 703, 0), 10, 3), None);
    assert_eq!(text(&n, &v, shape(24, 0, 704, 0), 7, (357 << 6) + 1), None);
}

// Per-level value from P's level and `descstr2` 5382 → 11091.
#[test]
fn f6_per_level_value_and_based_on_level() {
    let mut n = names();
    // itemstatcost 214 `item_armor_perlevel`: op 4, op param 3, op base
    // 12 (`level`).
    n.stats.insert(
        214,
        StatDesc {
            func: 6,
            val: 1,
            pos: 705,
            str2: sid::EVIL,
            op: 4,
            op_param: 3,
            op_base: 12,
            ..StatDesc::default()
        },
    );
    n.stats.insert(12, StatDesc::default());
    let level = |s: u16, _: u16| if s == 12 { 20 } else { 0 };
    let v = Viewer {
        player: Some(&level),
        ..Viewer::default()
    };
    let val = value(&n, &v, 214, 12, false);
    assert_eq!(val, 30);
    let d = n.stat_desc(214).unwrap();
    assert_eq!(
        text(&n, &v, Shape::own(&d), val, 0).as_deref(),
        Some("+30 Defense (Based on Character Level)")
    );
}

#[test]
fn f5_scales_f25_signs_from_minus_v_f0_none() {
    let n = names();
    let v = Viewer::default();
    let s = |sh, value| text(&n, &v, sh, value, 0);
    assert_eq!(s(shape(5, 1, 705, 0), 64).as_deref(), Some("50% Defense"));
    assert_eq!(s(shape(5, 1, 705, 0), -64).as_deref(), Some("-50% Defense"));
    assert_eq!(s(shape(25, 1, 705, 0), -3).as_deref(), Some("+-3 Defense"));
    assert_eq!(s(shape(25, 1, 705, 0), 3).as_deref(), Some("3 Defense"));
    assert_eq!(s(shape(0, 1, 705, 0), 3), None);
    assert_eq!(s(shape(29, 1, 705, 0), 3), None);
}

#[test]
fn f27_f28_skill_lines() {
    let mut n = names();
    n.strings.insert(821, "(Sorceress Only)".into());
    let v = Viewer::default();
    assert_eq!(
        text(&n, &v, shape(27, 0, 0, 0), 2, 36).as_deref(),
        Some("+2 to Fire Bolt (Sorceress Only)")
    );
    assert_eq!(text(&n, &v, shape(27, 0, 0, 0), 0, 36), None);
    assert_eq!(text(&n, &v, shape(27, 0, 0, 0), 2, 40), None);
    // f 28: a Sorceress gets at most +3 to her own class's skill.
    let sorc = Viewer {
        player_class: Some(1),
        ..Viewer::default()
    };
    assert_eq!(
        text(&n, &sorc, shape(28, 0, 0, 0), 5, 36).as_deref(),
        Some("+3 to Fire Bolt")
    );
    assert_eq!(
        text(&n, &v, shape(28, 0, 0, 0), 5, 36).as_deref(),
        Some("+5 to Fire Bolt")
    );
}

// The by-time line without an act uses the low bound.
#[test]
fn f17_by_time_without_act() {
    let mut n = names();
    n.strings.insert(21235, "(Increases During Daytime)".into());
    let v = Viewer::default();
    // p = 0, lo = 266 − 256 = 10.
    let packed = 266 << 2;
    assert_eq!(
        text(&n, &v, shape(17, 1, 705, 0), packed, 0).as_deref(),
        Some("(Increases During Daytime)\n+10 Defense")
    );
    assert_eq!(
        text(&n, &v, shape(18, 0, 705, 0), packed, 0).as_deref(),
        Some("(Increases During Daytime)\n")
    );
}
