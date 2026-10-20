//! Tests of the skilldesc line function and its helpers, on a fake input set
//! ([`Fake`], also used by the box tests of `skill_hover`).

use std::collections::BTreeMap;

use super::*;

/// A fake [`HoverInputs`]: a calc field evaluates to its own value (no
/// formula → 0); the damage functions answer fixed values.
#[derive(Clone, Debug, Default)]
pub(crate) struct Fake {
    pub strings: BTreeMap<u16, Vec<u16>>,
    pub skills: BTreeMap<i32, SkillRow>,
    pub descs: BTreeMap<i32, DescRow>,
    pub missiles: BTreeMap<i32, MissileRow>,
    /// `elem_min` / `elem_max` (1/256), `elem_len`.
    pub elem: (i32, i32),
    pub len: i32,
    /// `phys_min` / `phys_max` without and with `use_srcdam` (1/256).
    pub phys: (i32, i32),
    pub phys_src: (i32, i32),
    pub to_hit: i32,
    pub miss_elem: (i32, i32),
    pub stats: BTreeMap<u16, i32>,
    pub levels: BTreeMap<i32, (i32, i32)>,
    pub class: Option<u8>,
    pub difficulty: u8,
    pub expansion: bool,
    pub curse: i32,
    /// monstats row → (Level[d], life (min, max), A1 (min, max)).
    pub monsters: BTreeMap<i32, Monster>,
    pub monstats_count: i32,
}

/// A fake monstats row: (Level[d], life (min, max), A1 (min, max)).
pub(crate) type Monster = (i32, (i32, i32), (i32, i32));

/// The English texts of the string ids the lines use.
const TEXTS: &[(u16, &str)] = &[
    (STR_COLOR, "ÿc"),
    (STR_SPACE, " "),
    (STR_COLON, ":"),
    (STR_NL, "\n"),
    (STR_SLASH, "/"),
    (STR_PERCENT_SIGN, "%"),
    (STR_DUMMY, "an evil force"),
    (4184, "Quantity : %d"),
    (4187, "Required Level : "),
    (4252, ""),
    (4253, "Next Level"),
    (4254, "Current Skill Level: "),
    (4256, "Damage: "),
    (4257, "Fire Damage: "),
    (4258, "Cold Damage: "),
    (4259, "Lightning Damage: "),
    (4260, "Poison Damage: "),
    (4262, "To Attack Rating: "),
    (4265, "Cold Length: "),
    (4266, "Poison Length: "),
    (4267, " second"),
    (4268, " seconds"),
    (4269, "First Level"),
    (4270, "Radius: "),
    (4272, "Duration: "),
    (4275, " percent"),
    (4278, " yards"),
    (4286, " per second"),
    (4288, " yard"),
    (4291, "Magic Damage: "),
    (4294, "Life: "),
    (4309, " percent damage returned"),
    (4322, "Average "),
    (4365, "Thorns damage"),
    (4369, "Maximum Level Reached"),
    (4370, " (item)"),
    (4371, "You have been granted this skill by an item"),
    (4372, "You have not learned this skill yet"),
    (3524, "Life stolen per hit"),
    (11022, "over "),
    (21248, "Charges"),
    (21252, "("),
    (21253, ")"),
    // Texts of the tests' own cells.
    (100, "ta"),
    (101, "tb"),
    (102, "Mana Cost: "),
];

pub(crate) fn w16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Fake {
    /// The standard strings, skill 1 (class 0) with an empty `skilldesc`
    /// row whose `str mana` is "Mana Cost: ", and no other facts.
    pub fn new() -> Self {
        let mut f = Fake {
            strings: TEXTS.iter().map(|&(k, t)| (k, w16(t))).collect(),
            class: Some(0),
            ..Fake::default()
        };
        f.skills.insert(
            1,
            SkillRow {
                class: 0,
                ..SkillRow::default()
            },
        );
        f.descs.insert(
            1,
            DescRow {
                mana: 102,
                ..DescRow::default()
            },
        );
        f
    }

    pub fn text(&mut self, id: u16, t: &str) {
        self.strings.insert(id, w16(t));
    }

    /// Sets cell `i` of skill 1: type, texts and the values of A and B.
    pub fn cell(&mut self, i: usize, kind: u8, ta: u16, tb: u16, a: i32, b: i32) {
        let d = self.descs.get_mut(&1).expect("skill 1");
        d.kind[i] = kind;
        d.texta[i] = ta;
        d.textb[i] = tb;
        d.calca[i] = a as u32;
        d.calcb[i] = b as u32;
    }

    pub fn row(&mut self) -> &mut SkillRow {
        self.skills.get_mut(&1).expect("skill 1")
    }
}

fn calc(c: u32) -> i32 {
    if c == NO_CALC {
        0
    } else {
        c as i32
    }
}

impl HoverInputs for Fake {
    fn string(&self, id: u16) -> Vec<u16> {
        self.strings.get(&id).cloned().unwrap_or_default()
    }
    fn skill_count(&self) -> i32 {
        self.skills.keys().max().map_or(0, |k| k + 1)
    }
    fn skill(&self, s: i32) -> Option<SkillRow> {
        self.skills.get(&s).cloned()
    }
    fn desc(&self, s: i32) -> Option<DescRow> {
        self.descs.get(&s).cloned()
    }
    fn missile(&self, m: i32) -> Option<MissileRow> {
        self.missiles.get(&m).copied()
    }
    fn eval_desc(&mut self, c: u32, _s: i32, _l: i32) -> i32 {
        calc(c)
    }
    fn eval_skill(&mut self, c: u32, _s: i32, _l: i32) -> i32 {
        calc(c)
    }
    fn elem_min(&mut self, _s: i32, _l: i32) -> i32 {
        self.elem.0
    }
    fn elem_max(&mut self, _s: i32, _l: i32) -> i32 {
        self.elem.1
    }
    fn elem_len(&mut self, _s: i32, _l: i32) -> i32 {
        self.len
    }
    fn phys_min(&mut self, _s: i32, _l: i32, src: bool) -> i32 {
        if src {
            self.phys_src.0
        } else {
            self.phys.0
        }
    }
    fn phys_max(&mut self, _s: i32, _l: i32, src: bool) -> i32 {
        if src {
            self.phys_src.1
        } else {
            self.phys.1
        }
    }
    fn to_hit(&mut self, _s: i32, _l: i32) -> i32 {
        self.to_hit
    }
    fn missile_elem_min(&mut self, _m: i32, _l: i32) -> i32 {
        self.miss_elem.0
    }
    fn missile_elem_max(&mut self, _m: i32, _l: i32) -> i32 {
        self.miss_elem.1
    }
    fn stat(&self, stat: u16) -> i32 {
        self.stats.get(&stat).copied().unwrap_or(0)
    }
    fn levels(&self, s: i32) -> Option<(i32, i32)> {
        self.levels.get(&s).copied()
    }
    fn class(&self) -> Option<u8> {
        self.class
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    fn curse_divisor(&self) -> i32 {
        self.curse
    }
    fn monstats_count(&self) -> i32 {
        self.monstats_count
    }
    fn monster_level(&self, m: i32) -> Option<i32> {
        self.monsters.get(&m).map(|v| v.0)
    }
    fn monster_life(&self, m: i32, _level: i32) -> Option<(i32, i32)> {
        self.monsters.get(&m).map(|v| v.1)
    }
    fn monster_a1(&self, m: i32, _level: i32) -> Option<(i32, i32)> {
        self.monsters.get(&m).map(|v| v.2)
    }
}

/// Runs cell 0 of skill 1 at level `l`, flag `flag`: (return value, text).
fn run_at(f: &mut Fake, l: i32, flag: i32) -> (bool, String) {
    let r = f.skill(1);
    let mut out = Vec::new();
    let ret = line(f, &mut out, r.as_ref(), 1, l, 0, 0, flag);
    (ret, String::from_utf16_lossy(&out))
}

/// One cell of type `kind` at level 1, flag 0.
fn one(kind: u8, ta: u16, tb: u16, a: i32, b: i32) -> (bool, String) {
    let mut f = Fake::new();
    f.cell(0, kind, ta, tb, a, b);
    run_at(&mut f, 1, 0)
}

fn text(kind: u8, ta: u16, tb: u16, a: i32, b: i32) -> String {
    one(kind, ta, tb, a, b).1
}

const TA: u16 = 100;
const TB: u16 = 101;
const D: u16 = STR_DUMMY;

#[test]
fn types_0_and_past_75_return_0_and_the_others_1_even_when_empty() {
    assert_eq!(one(0, TA, TB, 5, 0), (false, String::new()));
    assert_eq!(one(76, TA, TB, 5, 0), (false, String::new()));
    // Types 8–11 with nothing to show: still 1, the dummies skipped.
    for k in [8, 9, 10, 11] {
        assert_eq!(one(k, D, D, 0, 0), (true, String::new()), "type {k}");
    }
    // 53–56: always 0, nothing appended.
    for k in 53..=56 {
        assert_eq!(one(k, TA, TB, 5, 0), (false, String::new()), "type {k}");
    }
}

#[test]
fn no_row_or_no_description_returns_0() {
    let mut f = Fake::new();
    f.cell(0, 2, TA, TB, 5, 0);
    let mut out = Vec::new();
    assert!(!line(&mut f, &mut out, None, 1, 1, 0, 0, 0));
    f.descs.clear();
    let r = f.skill(1);
    assert!(!line(&mut f, &mut out, r.as_ref(), 1, 1, 0, 0, 0));
    // Level 0 still reaches the switch.
    let mut f = Fake::new();
    f.cell(0, 2, TA, TB, 5, 0);
    assert_eq!(run_at(&mut f, 0, 0), (true, "ta+5tb\n".into()));
}

#[test]
fn n_and_f_number_forms() {
    assert_eq!(num(5, true), "+5");
    assert_eq!(num(0, true), "+0");
    assert_eq!(num(-5, true), "-5");
    assert_eq!(num(5, false), "5");
    assert_eq!(frac(1, 5, true), "+1.5");
    assert_eq!(frac(1, 5, false), "1.5");
    assert_eq!(rng(0, 0), "0");
    assert_eq!(rng(1, 4), "1-4");
}

#[test]
fn sec_singular_plural_and_tenths() {
    // Type 12: sec(A, texta, 0); type 57 with plus.
    assert_eq!(text(12, TA, D, 25, 0), "ta1 second\n");
    assert_eq!(text(12, TA, D, 50, 0), "ta2 seconds\n");
    // 37 frames: q 1, t 12 × 10 / 25 = 4.
    assert_eq!(text(12, TA, D, 37, 0), "ta1.4 seconds\n");
    assert_eq!(text(12, TA, D, 5, 0), "ta0.2 seconds\n");
    assert_eq!(text(12, TA, D, 2, 0), "");
    assert_eq!(text(12, TA, TB, 0, 0), "tb");
    assert_eq!(text(57, TA, D, 25, 0), "ta+1 second\n");
    assert_eq!(text(57, TA, D, 30, 0), "ta+1.2 seconds\n");
}

#[test]
fn yards_clamp_the_integer_part_to_1() {
    // Type 19: yards2 (20v / 3 tenths).
    assert_eq!(text(19, TA, D, 0, 0), "ta1 yard\n");
    assert_eq!(text(19, TA, D, 3, 0), "ta2 yards\n");
    assert_eq!(text(19, TA, D, 4, 0), "ta2.6 yards\n");
    assert_eq!(text(19, TA, D, 1, 0), "ta1.6 yard\n");
    // Type 37: yards1 (10v / 3).
    assert_eq!(text(37, TA, D, 3, 0), "ta1 yard\n");
    assert_eq!(text(37, TA, D, 9, 0), "ta3 yards\n");
}

#[test]
fn fix_prints_hundredths_unpadded() {
    // Type 61: fix(A, B, ta, tb, 0); 1 + 13/256 with d 100 → r 5.
    assert_eq!(text(61, TA, TB, 256 + 13, 100), "ta1.5tb\n");
    assert_eq!(text(61, TA, TB, 256, 100), "ta1tb\n");
    // B = 0 → tenths.
    assert_eq!(text(61, TA, TB, 128, 0), "ta0.5tb\n");
    // Dummy texts are left out; v = 0 → nothing.
    assert_eq!(text(61, D, D, 512, 0), "2\n");
    assert_eq!(text(61, TA, TB, 0, 0), "");
    // A value under one hundredth: neither number nor text gap.
    assert_eq!(text(61, TA, TB, 1, 100), "tatb\n");
    assert_eq!(text(60, TA, TB, 256 + 128, 10), "ta+1.5tb\n");
}

fn mana(f: &mut Fake, l: i32) -> String {
    run_at(f, l, 0).1
}

#[test]
fn mana_cost_forms() {
    let mut f = Fake::new();
    // Magic Arrow: mana 12, lvlmana −1, manashift 5; A = 10.
    *f.row() = SkillRow {
        mana: 12,
        lvlmana: -1,
        manashift: 5,
        class: 0,
        ..SkillRow::default()
    };
    f.cell(0, 1, D, D, 10, 0);
    assert_eq!(mana(&mut f, 1), "Mana Cost: 1.5\n");
    // Level 5: (12 − 4) << 5 = 256 → "1".
    assert_eq!(mana(&mut f, 5), "Mana Cost: 1\n");
    // minmana 2 raises m to 512.
    f.row().minmana = 2;
    assert_eq!(mana(&mut f, 5), "Mana Cost: 2\n");
    f.row().minmana = 0;
    // A > 10: A is the denominator (hundredths, unpadded).
    f.cell(0, 1, D, D, 100, 0);
    assert_eq!(mana(&mut f, 1), "Mana Cost: 1.50\n");
    // manashift 8: whole mana.
    *f.row() = SkillRow {
        mana: 7,
        manashift: 8,
        class: 0,
        ..SkillRow::default()
    };
    assert_eq!(mana(&mut f, 1), "Mana Cost: 7\n");
    // m > 6400 with another shift: whole mana too.
    *f.row() = SkillRow {
        mana: 300,
        manashift: 5,
        class: 0,
        ..SkillRow::default()
    };
    f.cell(0, 1, D, D, 10, 0);
    assert_eq!(mana(&mut f, 1), "Mana Cost: 37\n");
    // mana = lvlmana = 0: nothing, still 1.
    *f.row() = SkillRow::default();
    assert_eq!(run_at(&mut f, 1, 0), (true, String::new()));
}

#[test]
fn mana_hundredths_are_not_padded() {
    let mut f = Fake::new();
    // mana 269 at shift 0: m = 1 + 13/256, A 100 → q 1, r 5.
    *f.row() = SkillRow {
        mana: 269,
        manashift: 0,
        class: 0,
        ..SkillRow::default()
    };
    f.cell(0, 1, D, D, 100, 0);
    assert_eq!(mana(&mut f, 1), "Mana Cost: 1.5\n");
}

#[test]
fn range_forms() {
    // range2 (38): equal → TVT without plus; else the dummies skipped.
    assert_eq!(text(38, TA, TB, 3, 3), "ta3tb\n");
    assert_eq!(text(38, TA, TB, 3, 5), "ta3-5tb\n");
    assert_eq!(text(38, D, TB, 3, 5), "3-5tb\n");
    assert_eq!(text(52, TA, TB, 3, 5), "ta+3-5tb\n");
    assert_eq!(text(52, TA, TB, 3, 3), "ta3tb\n");
    // 17: tb? first, then range2 with " per second".
    assert_eq!(text(17, TA, TB, 3, 5), "tbta3-5 per second\n");
    // range3 (62 / 58): the number after both texts.
    assert_eq!(text(62, TA, TB, 3, 5), "tatb3-5\n");
    assert_eq!(text(58, TA, TB, 3, 5), "tatb+3-5\n");
    assert_eq!(text(58, TA, TB, 4, 4), "ta4tb\n");
    // range4 (47): equal → TV.
    assert_eq!(text(47, TA, TB, 4, 4), "ta4\n");
    assert_eq!(text(47, TA, TB, 4, 6), "ta4-6\n");
    // 49: physical with use_srcdam, a space and plus.
    let mut f = Fake::new();
    f.phys_src = (2 << 8, 5 << 8);
    f.cell(0, 49, TA, TB, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "ta +2-5\n");
    f.phys_src = (2 << 8, 2 << 8);
    assert_eq!(run_at(&mut f, 1, 0).1, "ta+2\n");
    // 35: always a range.
    assert_eq!(text(35, TA, TB, 4, 4), "ta: 4-4\n");
}

#[test]
fn colon_forms() {
    assert_eq!(text(63, TA, TB, 12, 0), "ta: +12% tb\n");
    assert_eq!(text(67, TA, TB, 12, 0), "ta: +12 tb\n");
    assert_eq!(text(63, D, TB, 12, 0), "+12% tb\n");
    assert_eq!(text(63, TA, TB, 0, 0), "");
    assert_eq!(text(42, TA, TB, 2, 5), "ta: +2.5 tb\n");
    assert_eq!(text(42, D, TB, 2, 5), "");
    assert_eq!(text(64, TA, TB, 3, 10), "ta: +3/10 tb\n");
    assert_eq!(text(64, TA, TB, 3, 0), "");
    assert_eq!(text(65, TA, TB, 0, 0), "ta: tb\n");
    assert_eq!(text(65, TA, D, 0, 0), "");
    assert_eq!(text(69, TA, TB, 7, 0), "ta: tb 7\n");
    assert_eq!(text(15, TA, TB, 0, 0), "ta: tb\n");
}

#[test]
fn dummy_texts() {
    // Unchecked: printed as "an evil force".
    assert_eq!(text(5, D, D, 3, 0), "an evil force3\n");
    assert_eq!(text(2, TA, D, 3, 0), "ta+3an evil force\n");
    // "tb?": skipped.
    assert_eq!(text(12, TA, D, 25, 0), "ta1 second\n");
    // 51 prints the dummy explicitly.
    assert_eq!(one(51, D, TB, 3, 0), (true, "an evil force\n".into()));
    // 40, 46, 71, 74, 75 return 0 on a dummy texta.
    for k in [40, 46, 74, 75] {
        assert_eq!(one(k, D, TB, 3, 3), (false, String::new()), "type {k}");
    }
    assert_eq!(one(71, TA, D, 3, 0), (false, String::new()));
    // 24: the label is texta even when it is the dummy.
    let mut f = Fake::new();
    f.elem = (256, 512);
    f.cell(0, 24, D, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "an evil force1-2\n");
}

#[test]
fn wsp_conversions() {
    let fmt = w16("Converts %d%% Physical %u %s.");
    let s = w16("x");
    let out = wsp(
        256,
        &fmt,
        &[Arg::Int(1), Arg::Int(9), Arg::Int(-1), Arg::Str(&s)],
    );
    // `%%` uses up the slot of the 9.
    assert_eq!(
        String::from_utf16_lossy(&out),
        "Converts 1% Physical 4294967295 x."
    );
    // An empty `%s` ends the formatting.
    let fmt = w16("a%sb");
    assert_eq!(wsp(256, &fmt, &[Arg::Str(&[])]), w16("a"));
    // A text that does not fit: what fits, then the end.
    let long = w16("abcdef");
    assert_eq!(wsp(5, &fmt, &[Arg::Str(&long)]), w16("aabc"));
    // An unknown conversion stops (fatal in the game).
    let fmt = w16("x%iy");
    assert_eq!(wsp(256, &fmt, &[Arg::Int(3)]), w16("x"));
    // 66: wsp(256, ta, A) + NL.
    let mut f = Fake::new();
    f.text(TA, "Converts %d%% Physical Damage to Magic Damage");
    f.cell(0, 66, TA, D, 1, 0);
    assert_eq!(
        run_at(&mut f, 1, 0).1,
        "Converts 1% Physical Damage to Magic Damage\n"
    );
    // 75: two numbers.
    f.text(TA, "%d to %d");
    f.cell(0, 75, TA, D, 2, 3);
    assert_eq!(run_at(&mut f, 1, 0).1, "2 to 3\n");
    // 71: "ta: " + wsp(100, tb, A).
    f.text(TA, "ta");
    f.text(TB, "%d%% more");
    f.cell(0, 71, TA, TB, 4, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "ta: 4% more\n");
}

#[test]
fn colored_lines_restore_the_base_colour_on_the_line_feed() {
    // 18: colour A on the text, colour `flag` on the line feed.
    let mut f = Fake::new();
    f.cell(0, 18, TA, TB, 3, 0);
    assert_eq!(run_at(&mut f, 1, 1).1, "ÿc3taÿc1\n");
    assert_eq!(run_at(&mut f, 1, 0).1, "ÿc3taÿc0\n");
    // A ≤ 0: the text uncoloured.
    f.cell(0, 18, TA, TB, 0, 0);
    assert_eq!(run_at(&mut f, 1, 1).1, "taÿc1\n");
    // 25: tb joined unless dummy.
    f.cell(0, 25, TA, TB, 2, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "ÿc2tatbÿc0\n");
    f.cell(0, 25, TA, D, 2, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "ÿc2taÿc0\n");
    // 40: the synergy header; 46 gated on B.
    f.text(TA, "%s Receives Bonuses From:");
    f.text(TB, "Fire Arrow");
    f.cell(0, 40, TA, TB, 2, 0);
    assert_eq!(
        run_at(&mut f, 1, 1),
        (true, "ÿc2Fire Arrow Receives Bonuses From:ÿc1\n".into())
    );
    f.cell(0, 46, TA, TB, 2, 0);
    assert_eq!(run_at(&mut f, 1, 1), (false, String::new()));
    f.cell(0, 46, TA, TB, 2, 1);
    assert!(run_at(&mut f, 1, 1).0);
}

#[test]
fn damage_lines() {
    let mut f = Fake::new();
    // 9: phys 1–1 → "+1"; A percent and B points.
    f.phys = (256, 256);
    f.cell(0, 9, D, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "Damage: +1\n");
    f.cell(0, 9, D, D, 100, 1);
    assert_eq!(run_at(&mut f, 1, 0).1, "Damage: +3\n");
    f.phys = (0, 3 << 8);
    f.cell(0, 9, D, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "Damage: 0-3\n");
    // 10: the element's label.
    f.row().etype = 1;
    f.elem = (1 << 8, 4 << 8);
    f.cell(0, 10, D, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "Fire Damage: 1-4\n");
    // An element with no label: nothing.
    f.row().etype = 0;
    assert_eq!(run_at(&mut f, 1, 0).1, "");
    // 11: only cold / poison lengths.
    f.len = 50;
    f.cell(0, 11, D, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "");
    f.row().etype = 4;
    assert_eq!(run_at(&mut f, 1, 0).1, "Cold Length: 2 seconds\n");
    // 14: poison over its length; the damage line after the "over" line.
    f.row().etype = 5;
    f.elem = (256, 512);
    f.len = 50;
    f.cell(0, 14, D, D, 0, 0);
    assert_eq!(
        run_at(&mut f, 1, 0).1,
        "over 2 seconds\nPoison Damage: 50-100\n"
    );
    // 8: the to-hit line.
    f.to_hit = 10;
    f.cell(0, 8, D, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "To Attack Rating: +10 percent\n");
    // 59: A, B as the range, no zero test.
    f.row().etype = 2;
    f.cell(0, 59, 0, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "Lightning Damage: 0\n");
    // 26: average per second (× 25), fire for a label-less element.
    f.row().etype = 0;
    f.elem = (256, 512);
    f.cell(0, 26, D, D, 0, 0);
    assert_eq!(
        run_at(&mut f, 1, 0).1,
        "Average Fire Damage: 25-50 per second\n"
    );
}

#[test]
fn missile_lines() {
    let mut f = Fake::new();
    f.descs.get_mut(&1).unwrap().missile = 7;
    // No missile row: 23, 28, 29, 30 return 0.
    for k in [23, 28, 29, 30] {
        f.cell(0, k, TA, D, 0, 0);
        assert_eq!(run_at(&mut f, 1, 0), (false, String::new()), "type {k}");
    }
    f.missiles.insert(
        7,
        MissileRow {
            range: 50,
            lev_range: 25,
            param1: 3,
            sub1: 8,
            etype: 4,
        },
    );
    // 23: Range + LevRange × L.
    f.cell(0, 23, TA, D, 0, 0);
    assert_eq!(run_at(&mut f, 2, 0).1, "ta4 seconds\n");
    // 28: radius from Param1.
    f.cell(0, 28, TA, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "Radius: 2 yards\n");
    // 29: 2 × (Range + LevRange × (L − 1)) / 3.
    f.cell(0, 29, TA, D, 0, 0);
    assert_eq!(run_at(&mut f, 2, 0).1, "50 yards\n");
    // 30: the sub-missile is missing → 0.
    f.cell(0, 30, TA, D, 0, 0);
    assert!(!run_at(&mut f, 1, 0).0);
    f.missiles.insert(
        8,
        MissileRow {
            range: 25,
            ..MissileRow::default()
        },
    );
    assert_eq!(run_at(&mut f, 1, 0), (true, "ta1 second\n".into()));
    // 50: the missile's element label.
    f.miss_elem = (3 << 8, 6 << 8);
    f.cell(0, 50, 0, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "Cold Damage: 3-6\n");
    // 22: × 75 and the local fire mastery for a fire skill.
    f.row().etype = 1;
    f.stats.insert(329, 100);
    f.miss_elem = (256, 512);
    f.cell(0, 22, D, D, 0, 0);
    assert_eq!(
        run_at(&mut f, 1, 0).1,
        "Average Cold Damage: 150-300 per second\n"
    );
}

#[test]
fn summon_and_curse_lines() {
    let mut f = Fake::new();
    f.monstats_count = 400;
    f.row().summon = 10;
    // 13: no monster stats → nothing.
    f.cell(0, 13, D, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0), (true, String::new()));
    f.monsters.insert(10, (5, (10, 20), (2, 4)));
    f.cell(0, 13, D, D, 100, 1);
    assert_eq!(run_at(&mut f, 1, 0).1, "Life: 31\n");
    // 34: damagepercent and item_normaldamage of the aura list.
    f.row().aura[0] = (25, 100);
    f.row().passive[0] = (111, 1);
    f.cell(0, 34, D, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "Damage: 6-10\n");
    // 39: damagepercent only.
    f.cell(0, 39, D, D, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "Damage: 4-8\n");
    // −90 floor.
    f.row().aura[0] = (25, (-200i32) as u32);
    assert_eq!(run_at(&mut f, 1, 0).1, "Damage: 1\n");
    // 41: the fire golem row (292) at the summon's level.
    f.row().etype = 1;
    f.monsters.insert(292, (0, (0, 0), (10, 20)));
    f.cell(0, 41, D, D, 1, 2);
    assert_eq!(run_at(&mut f, 1, 0).1, "Fire Damage: 11-22\n");
    // Without the fire golem row the helper fails: A, B alone.
    f.monstats_count = 200;
    f.row().summon = 10;
    assert_eq!(run_at(&mut f, 1, 0).1, "Fire Damage: 1-2\n");
    // 31: the curse divisor.
    f.curse = 2;
    f.cell(0, 31, TA, D, 100, 0);
    assert_eq!(run_at(&mut f, 1, 0).1, "ta2 seconds\n");
    f.curse = 0;
    assert_eq!(run_at(&mut f, 1, 0).1, "ta4 seconds\n");
}

#[test]
fn level_gated_and_other_lines() {
    // 33 and 45 need L > 1.
    let mut f = Fake::new();
    f.cell(0, 33, TA, TB, 0, 0);
    assert_eq!(run_at(&mut f, 1, 0), (true, String::new()));
    assert_eq!(run_at(&mut f, 2, 0).1, "tatb\n");
    f.row().param1 = 30;
    f.row().param2 = 10;
    f.cell(0, 45, D, D, 0, 0);
    assert_eq!(
        run_at(&mut f, 3, 0).1,
        "40 percent damage returned\nThorns damage\n"
    );
    // 16: always a range of seconds.
    assert_eq!(text(16, D, D, 25, 50), "Duration: 1-2 seconds\n");
    assert_eq!(text(16, D, D, 30, 50), "Duration: 1.2-2.0 seconds\n");
    assert_eq!(text(16, D, D, 0, 10), "");
    // 36: singular / plural.
    assert_eq!(text(36, TA, TB, 1, 0), "1ta\n");
    assert_eq!(text(36, TA, TB, 2, 0), "2tb\n");
    // 20 / 21.
    assert_eq!(text(20, TA, TB, 5, 0), "ta+5 percenttb\n");
    assert_eq!(text(21, TA, TB, 5, 0), "ta5 percenttb\n");
    // 32, 70: ta then a TVT / TV.
    assert_eq!(text(32, TA, TB, 5, 0), "tatb+5 percent\n");
    assert_eq!(text(70, TA, TB, 0, 0), "ta");
    // 43 / 44: /256 ranges.
    assert_eq!(text(43, TA, TB, 512, 768), "ta2-3tb\n");
    assert_eq!(text(43, TA, TB, 384, 768), "ta1.5-3.0tb\n");
    assert_eq!(text(44, TA, TB, 256 + 13, 512), "ta1.5-2.0tb\n");
    // 72 / 73.
    assert_eq!(one(72, TA, TB, 2, 0), (false, String::new()));
    assert_eq!(text(72, TA, TB, 2, 5), "+2/5 ta\n");
    assert_eq!(text(73, TA, TB, 2, 5), "2/5 ta\n");
    // VT / VTT.
    assert_eq!(text(6, TA, TB, 2, 0), "+2ta\n");
    assert_eq!(text(68, TA, TB, 2, 0), "2tatb\n");
}

#[test]
fn blessed_hammer_by_game_type() {
    let mut f = Fake::new();
    f.row().etype = 3;
    f.elem = (2 << 8, 4 << 8);
    f.to_hit = 0;
    f.cell(0, 48, TA, TB, 0, 0);
    // Classic: skill elements as one range, no texts.
    assert_eq!(run_at(&mut f, 1, 0).1, "Damage: 2-4\n");
    // Expansion: length, element, physical, to-hit lines.
    f.expansion = true;
    assert_eq!(run_at(&mut f, 1, 0).1, "Magic Damage: 2-4\n");
}

#[test]
fn learned_class_skill_needs_a_base_level_in_the_class() {
    let mut f = Fake::new();
    f.skills.insert(
        2,
        SkillRow {
            class: 1,
            ..SkillRow::default()
        },
    );
    assert!(!learned_class_skill(&f));
    // Another class's skill does not count.
    f.levels.insert(2, (3, 3));
    assert!(!learned_class_skill(&f));
    // Only a bonus level does not count.
    f.levels.insert(1, (0, 2));
    assert!(!learned_class_skill(&f));
    f.levels.insert(1, (1, 1));
    assert!(learned_class_skill(&f));
    f.class = None;
    assert!(!learned_class_skill(&f));
}
