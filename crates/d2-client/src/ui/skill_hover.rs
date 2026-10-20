//! The skill description box of a hovered skill icon on the skill tree
//! (playtest B028), and the pop-ups of a hovered skill button.
//!
//! The icon draw builds the learnable box when a point can be spent
//! on the skill, else the plain box (the skill tree's state −1). Both build one
//! UTF-16 text from the skill's `skilldesc` cells ([`skill_hover_lines`]),
//! level headers, the long description and the green name ([`box_text`]): the
//! pop-up draw moves up at each line feed, so the buffer's first line is the
//! box's bottom line. The text is set as the pop-up with colour 0 (white) for
//! the learnable box and 1 (red) for the plain box; its box top is 5 rows under the
//! icon's bottom row `Y` and its left edge 24 px left of the icon.
//!
//! [`skill_hover_lines`]: super::skill_hover_lines

use super::skill_hover_lines::{
    color_prefix, learned_class_skill, line, HoverInputs, STR_DUMMY as DUMMY, STR_NL,
};
use super::{hud_tips, FontMeasure};
use crate::ui::draw::UiDrawSink;
use crate::ui::geom::Point;
use crate::ui::panel::StringLookup;

/// `StrSkill0` (4252, empty): the buffer's first text.
pub const STR_SKILL0: u16 = 4252;
/// "Next Level", "Current Skill Level: ", "First Level".
pub const STR_NEXT_LEVEL: u16 = 4253;
pub const STR_CURRENT_LEVEL: u16 = 4254;
pub const STR_FIRST_LEVEL: u16 = 4269;
/// "Required Level: ", "Maximum Level Reached".
pub const STR_REQUIRED_LEVEL: u16 = 4187;
pub const STR_MAX_LEVEL: u16 = 4369;
/// " (item)", the item note and the not-learned note.
pub const STR_ITEM: u16 = 4370;
pub const STR_ITEM_NOTE: u16 = 4371;
pub const STR_NOT_LEARNED: u16 = 4372;
/// The string id of an empty cell (`dummy`).
pub const STR_DUMMY: u16 = DUMMY;
/// The pop-up lies this far right of the icon's left edge.
pub const X_OFFSET: i32 = -0x18;
/// The pop-up's y argument is the text height plus the icon's bottom row
/// plus this (its box top is then 5 rows under the icon).
pub const Y_ARG: i32 = 3;
/// `D2Win_SetPopUpUnicodeText` clears the pop-up for a text this long.
pub const POPUP_MAX: usize = 0x400;
/// The `dsc3line7` … `dsc3line1` cells (synergy block), in call order.
const SYNERGY: [usize; 7] = [16, 15, 14, 13, 12, 11, 10];
/// `descline1`–`6` and `dsc2line1`–`4`.
const DESC: std::ops::Range<usize> = 0..6;
const DSC2: std::ops::Range<usize> = 6..10;
/// The green of the name (`col(t, 2)`).
const NAME_COLOR: i32 = 2;

/// A skill's `skilldesc` text ids.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkillText {
    /// `str name`.
    pub name: u16,
    /// `str long`.
    pub long: u16,
    /// `str short`.
    pub short: u16,
}

/// The hovered icon of a frame: set by the panel that draws it, drawn by the
/// pop-up layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkillHover {
    pub skill: u16,
    /// The skill level shown (`L`, 0: not learned).
    pub level: i32,
    /// The icon's cel draw point (left, bottom row).
    pub at: Point,
    /// The box lies above the icon (the select list at the screen bottom,
    /// D2RUST's own, unverified).
    pub above: bool,
    /// A point can be spent on the skill now; else the
    /// skill tree's state is −1.
    pub learnable: bool,
}

/// The text and pop-up colour of the box of skill `s` at level `l`
/// (the learnable box for a `learnable` skill, else the plain one); `None`
/// when the box returns early or the text is too long for the pop-up.
pub fn box_text(
    inp: &mut dyn HoverInputs,
    s: i32,
    l: i32,
    learnable: bool,
) -> Option<(Vec<u16>, u16)> {
    // r1: no row (s out of range).
    let r = inp.skill(s)?;
    let levels = inp.levels(s);
    // r2: granted only by items.
    let item = levels.is_some_and(|(base, all)| all != 0 && base == 0);
    let maxed = levels.is_some_and(|(base, _)| base >= r.max_level());
    let mut out = inp.string(STR_SKILL0);
    let flag = i32::from(!learnable);
    // One block: every cell is called, the separator follows when any
    // returned non-zero.
    let block = |inp: &mut dyn HoverInputs, out: &mut Vec<u16>, cells: &[usize], lvl: i32| {
        cells.iter().fold(false, |any, &i| {
            line(inp, out, Some(&r), s, lvl, 0, i, flag) | any
        })
    };
    let push = |inp: &dyn HoverInputs, out: &mut Vec<u16>, id: u16| out.extend(inp.string(id));
    let desc: Vec<usize> = DESC.collect();
    let dsc2: Vec<usize> = DSC2.collect();
    if learnable {
        if block(inp, &mut out, &SYNERGY, l + 1) {
            push(inp, &mut out, STR_NL);
        }
        if !maxed && block(inp, &mut out, &desc, l + 1) {
            let head = if l == 0 {
                STR_FIRST_LEVEL
            } else {
                STR_NEXT_LEVEL
            };
            push(inp, &mut out, head);
            push(inp, &mut out, STR_NL);
            push(inp, &mut out, STR_NL);
        }
        if l > 0 && block(inp, &mut out, &desc, l) {
            push(inp, &mut out, STR_CURRENT_LEVEL);
            out.extend(l.to_string().encode_utf16());
            if item {
                push(inp, &mut out, STR_ITEM);
            }
            push(inp, &mut out, STR_NL);
            push(inp, &mut out, STR_NL);
        }
        if block(inp, &mut out, &dsc2, l + 1) {
            push(inp, &mut out, STR_NL);
        }
    } else {
        if block(inp, &mut out, &SYNERGY, l + 1) {
            push(inp, &mut out, STR_NL);
        }
        let lv = l.max(1);
        if block(inp, &mut out, &desc, lv) {
            if l < 1 {
                push(inp, &mut out, STR_FIRST_LEVEL);
            } else {
                push(inp, &mut out, STR_CURRENT_LEVEL);
                out.extend(l.to_string().encode_utf16());
            }
            if item {
                push(inp, &mut out, STR_ITEM);
            }
            push(inp, &mut out, STR_NL);
            push(inp, &mut out, STR_NL);
        }
        if block(inp, &mut out, &dsc2, lv) {
            push(inp, &mut out, STR_NL);
        }
        if maxed {
            push(inp, &mut out, STR_MAX_LEVEL);
            push(inp, &mut out, STR_NL);
        } else {
            let req = r.reqlevel.wrapping_add(l);
            if req != 0 {
                push(inp, &mut out, STR_REQUIRED_LEVEL);
                out.extend(req.to_string().encode_utf16());
                push(inp, &mut out, STR_NL);
            }
        }
    }
    // R9–r12: long description, green name, notes.
    let d = inp.desc(s);
    push(inp, &mut out, d.as_ref().map_or(STR_DUMMY, |d| d.long));
    push(inp, &mut out, STR_NL);
    let mut name = inp.string(d.as_ref().map_or(STR_DUMMY, |d| d.name));
    color_prefix(inp, &mut name, NAME_COLOR);
    out.extend(name);
    if item {
        push(inp, &mut out, STR_NL);
        push(inp, &mut out, STR_NL);
        push(inp, &mut out, STR_ITEM_NOTE);
    }
    if !learned_class_skill(inp) {
        push(inp, &mut out, STR_NL);
        push(inp, &mut out, STR_NL);
        push(inp, &mut out, STR_NOT_LEARNED);
    }
    // A text of 0x400 units or more clears the pop-up.
    (out.len() < POPUP_MAX).then_some((out, u16::from(!learnable)))
}

/// The pop-up point of an icon at `at` for a text `height` high.
pub fn popup_at(hover: &SkillHover, height: i32) -> Point {
    if hover.above {
        Point::new(hover.at.x + X_OFFSET, hover.at.y - 48)
    } else {
        Point::new(hover.at.x + X_OFFSET, height + hover.at.y + Y_ARG)
    }
}

/// Pushes the box of `hover` in its colour (nothing without its text or
/// the font).
pub fn draw(
    hover: &SkillHover,
    inp: &mut dyn HoverInputs,
    fonts: Option<&FontMeasure>,
    screen: (i32, i32),
    out: &mut dyn UiDrawSink,
) {
    let skill = i32::from(hover.skill);
    let Some((t, color)) = box_text(inp, skill, hover.level, hover.learnable) else {
        return;
    };
    let Some(f) = fonts else {
        return;
    };
    let Some(height) = f.text_height(hud_tips::TIP_FONT, &t) else {
        return;
    };
    hud_tips::push_popup(
        t,
        popup_at(hover, height),
        color,
        false,
        screen,
        Some(f),
        out,
    );
}

/// The skill button pop-up's first string (4252) and the line break (3998).
pub const STR_BUTTON_HEAD: u16 = 0x109C;
pub const STR_NEWLINE: u16 = 0x0F9E;
/// "Finishing Move - ", the Assassin's prefix before Attack's text.
pub const STR_FINISHING_MOVE: u16 = 0x5722;
/// The text of both lines when the skill has no `skilldesc` record.
pub const STR_NO_DESC: u16 = 0x1506;
/// `charclass` of the Assassin.
pub const CLASS_ASSASSIN: u8 = 6;

/// The hovered skill button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonHover {
    pub skill: u16,
    /// The button's cel draw point (left, bottom row).
    pub at: Point,
    /// The button state `k`; the pop-up colour is `k == 1`.
    pub k: u8,
    /// The player's `charclass` (for the Assassin prefix).
    pub class: u8,
    /// The entry's quantity (+0x30) when it picks the quantity pop-up,
    /// else 0.
    pub quantity: i32,
}

/// The text of the button's pop-up: string 4252, for the Assassin's
/// Attack (skill 0, class 6) string 0x5722, then `str short`, a line
/// break and `str name`; a skill without a `skilldesc` record gives
/// string 0x1506 for both lines. The skilldesc line types
/// are not modelled.
pub fn button_description(
    strings: &dyn StringLookup,
    hover: &ButtonHover,
    text: Option<SkillText>,
) -> Vec<u16> {
    let get = |id: u16| strings.get_id(id).map(<[u16]>::to_vec).unwrap_or_default();
    let (short, name) = text.map_or((STR_NO_DESC, STR_NO_DESC), |t| (t.short, t.name));
    let mut out = get(STR_BUTTON_HEAD);
    if hover.skill == 0 && hover.class == CLASS_ASSASSIN {
        out.extend(get(STR_FINISHING_MOVE));
    }
    out.extend(get(short));
    out.extend(get(STR_NEWLINE));
    out.extend(get(name));
    out
}

/// "Quantity " (`skilldesc4`) and the colon (3997) of the quantity pop-up.
pub const STR_QUANTITY: u16 = 0x105C;
pub const STR_COLON: u16 = 0x0F9D;
/// Widens at most 8 characters of the count.
const QUANTITY_DIGITS: usize = 8;

/// The text of the quantity pop-up: `str short`, a line
/// break, "Quantity " + ":" + the quantity (`%i`, at most 8 characters),
/// a line break and `str name`; a skill without a `skilldesc` record
/// gives string 0x1506 for both.
pub fn quantity_description(
    strings: &dyn StringLookup,
    text: Option<SkillText>,
    quantity: i32,
) -> Vec<u16> {
    let get = |id: u16| strings.get_id(id).map(<[u16]>::to_vec).unwrap_or_default();
    let (short, name) = text.map_or((STR_NO_DESC, STR_NO_DESC), |t| (t.short, t.name));
    let mut out = get(short);
    out.extend(get(STR_NEWLINE));
    out.extend(get(STR_QUANTITY));
    out.extend(get(STR_COLON));
    out.extend(
        quantity
            .to_string()
            .bytes()
            .take(QUANTITY_DIGITS)
            .map(u16::from),
    );
    out.extend(get(STR_NEWLINE));
    out.extend(get(name));
    out
}

/// Pushes the pop-up of a hovered skill button: x = the button's left
/// edge, y = text height + the button's bottom row + 3, colour = the
/// `k == 1` (the button's state argument), not centred.
/// With a quantity it is the quantity text instead,
/// at y = the bottom row − 50, centred on x, the same colour.
pub fn draw_button(
    hover: &ButtonHover,
    strings: &dyn StringLookup,
    text: Option<SkillText>,
    fonts: Option<&FontMeasure>,
    screen: (i32, i32),
    out: &mut dyn UiDrawSink,
) {
    if hover.quantity != 0 {
        let t = quantity_description(strings, text, hover.quantity);
        let at = Point::new(hover.at.x, hover.at.y - 0x32);
        hud_tips::push_popup(t, at, u16::from(hover.k == 1), true, screen, fonts, out);
        return;
    }
    let t = button_description(strings, hover, text);
    let Some(f) = fonts else {
        return;
    };
    let Some(height) = f.text_height(hud_tips::TIP_FONT, &t) else {
        return;
    };
    let at = Point::new(hover.at.x, height + hover.at.y + Y_ARG);
    hud_tips::push_popup(t, at, u16::from(hover.k == 1), false, screen, Some(f), out);
}

#[cfg(test)]
mod button_tests {
    use super::*;

    struct S;
    impl StringLookup for S {
        fn get(&self, _key: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            const A: [u16; 1] = [0x41];
            const B: [u16; 1] = [0x62];
            const NL: [u16; 1] = [0x0A];
            const F: [u16; 1] = [0x46];
            const D: [u16; 1] = [0x64];
            match id {
                10 => Some(&A),
                11 => Some(&B),
                STR_NEWLINE => Some(&NL),
                STR_FINISHING_MOVE => Some(&F),
                STR_NO_DESC => Some(&D),
                _ => None,
            }
        }
    }

    fn hover(skill: u16, class: u8) -> ButtonHover {
        ButtonHover {
            skill,
            at: Point::new(0, 0),
            k: 0,
            class,
            quantity: 0,
        }
    }

    const T: SkillText = SkillText {
        name: 10,
        long: 0,
        short: 11,
    };

    #[test]
    fn button_text_is_short_newline_name() {
        assert_eq!(
            button_description(&S, &hover(0, 0), Some(T)),
            vec![0x62, 0x0A, 0x41]
        );
    }

    #[test]
    fn the_assassin_attack_has_the_finishing_move_prefix() {
        let t = Some(T);
        assert_eq!(
            button_description(&S, &hover(0, CLASS_ASSASSIN), t),
            vec![0x46, 0x62, 0x0A, 0x41]
        );
        // Only skill 0, only class 6.
        assert_eq!(
            button_description(&S, &hover(1, CLASS_ASSASSIN), t),
            vec![0x62, 0x0A, 0x41]
        );
    }

    #[test]
    fn a_skill_without_a_record_shows_the_dummy_string_twice() {
        assert_eq!(
            button_description(&S, &hover(5, 0), None),
            vec![0x64, 0x0A, 0x64]
        );
    }

    #[test]
    fn quantity_text_is_short_quantity_line_then_name() {
        struct Q;
        impl StringLookup for Q {
            fn get(&self, _key: &str) -> Option<&[u16]> {
                None
            }
            fn get_id(&self, id: u16) -> Option<&[u16]> {
                const A: [u16; 1] = [0x41];
                const B: [u16; 1] = [0x62];
                const NL: [u16; 1] = [0x0A];
                const Q: [u16; 2] = [0x51, 0x20];
                const C: [u16; 1] = [0x3A];
                const D: [u16; 1] = [0x64];
                match id {
                    10 => Some(&A),
                    11 => Some(&B),
                    STR_NEWLINE => Some(&NL),
                    STR_QUANTITY => Some(&Q),
                    STR_COLON => Some(&C),
                    STR_NO_DESC => Some(&D),
                    _ => None,
                }
            }
        }
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        assert_eq!(quantity_description(&Q, Some(T), 12), w("b\nQ :12\nA"));
        assert_eq!(quantity_description(&Q, None, 3), w("d\nQ :3\nd"));
        // Only 8 characters of the count are copied.
        assert_eq!(
            quantity_description(&Q, Some(T), -123_456_789),
            w("b\nQ :-1234567\nA")
        );
    }
}

#[cfg(test)]
mod box_tests {
    use super::super::skill_hover_lines::tests::{w16, Fake};
    use super::super::skill_hover_lines::SkillRow;
    use super::*;

    const D: u16 = STR_DUMMY;
    const NAME: u16 = 200;
    const LONG: u16 = 201;

    /// Skill 1 named "Magic Arrow" with a two-line long text, and skill 2
    /// of the same class learned (so no not-learned note).
    fn fake() -> Fake {
        let mut f = Fake::new();
        f.text(NAME, "Magic Arrow");
        f.text(LONG, "Creates a magical arrow\nthat does extra damage");
        let d = f.descs.get_mut(&1).unwrap();
        d.name = NAME;
        d.long = LONG;
        f.skills.insert(
            2,
            SkillRow {
                class: 0,
                ..SkillRow::default()
            },
        );
        f.levels.insert(2, (1, 1));
        f
    }

    fn text(f: &mut Fake, l: i32, learnable: bool) -> (String, u16) {
        let (t, c) = box_text(f, 1, l, learnable).expect("box");
        (String::from_utf16_lossy(&t), c)
    }

    const TAIL: &str = "Creates a magical arrow\nthat does extra damage\nÿc2Magic Arrow";

    // Test vector 1: Magic Arrow, plain box, L = 0.
    #[test]
    fn magic_arrow_unlearned_box() {
        let mut f = fake();
        *f.row() = SkillRow {
            class: 0,
            etype: 3,
            mana: 12,
            lvlmana: -1,
            manashift: 5,
            reqlevel: 1,
            ..SkillRow::default()
        };
        f.text(102, "Mana Cost: ");
        f.text(300, "Converts %d%% Physical Damage to Magic Damage");
        f.cell(0, 1, D, D, 10, 0);
        f.cell(1, 11, D, D, 0, 0);
        f.cell(2, 10, D, D, 0, 0);
        f.cell(3, 9, D, D, 0, 0);
        f.cell(4, 8, D, D, 0, 0);
        f.cell(5, 66, 300, D, 1, 0);
        f.phys = (256, 256);
        f.to_hit = 10;
        let (t, c) = text(&mut f, 0, false);
        assert_eq!(
            t,
            String::new()
                + "Mana Cost: 1.5\n"
                + "Damage: +1\n"
                + "To Attack Rating: +10 percent\n"
                + "Converts 1% Physical Damage to Magic Damage\n"
                + "First Level\n\n"
                + "Required Level : 1\n"
                + TAIL
        );
        assert_eq!(c, 1);
    }

    // Test vector 2: Fire Arrow, plain box, L = 0, with its synergy
    // block (dsc3 cells 11 then 10, at L + 1).
    #[test]
    fn fire_arrow_unlearned_box_with_synergies() {
        let mut f = fake();
        f.text(NAME, "Fire Arrow");
        *f.row() = SkillRow {
            class: 0,
            etype: 1,
            mana: 24,
            manashift: 5,
            reqlevel: 1,
            ..SkillRow::default()
        };
        f.text(301, "Exploding Arrow");
        f.text(302, "Fire Damage per Level");
        f.text(303, "%s Receives Bonuses From:");
        f.text(304, "Fire Arrow");
        f.cell(0, 1, D, D, 10, 0);
        f.cell(1, 11, D, D, 0, 0);
        f.cell(2, 10, D, D, 0, 0);
        f.cell(3, 9, D, D, 0, 0);
        f.cell(11, 63, 301, 302, 12, 0);
        f.cell(10, 40, 303, 304, 2, 0);
        f.elem = (1 << 8, 4 << 8);
        f.len = 0;
        let (t, c) = text(&mut f, 0, false);
        assert_eq!(
            t,
            String::new()
                + "Exploding Arrow: +12% Fire Damage per Level\n"
                + "ÿc2Fire Arrow Receives Bonuses From:ÿc1\n"
                + "\n"
                + "Mana Cost: 3\n"
                + "Fire Damage: 1-4\n"
                + "First Level\n\n"
                + "Required Level : 1\n"
                + "Creates a magical arrow\nthat does extra damage\nÿc2Fire Arrow"
        );
        assert_eq!(c, 1);
    }

    #[test]
    fn learnable_box_has_next_and_current_level_blocks_in_white() {
        let mut f = fake();
        f.cell(0, 5, 100, D, 7, 0);
        f.levels.insert(1, (2, 2));
        let (t, c) = text(&mut f, 2, true);
        assert_eq!(
            t,
            "ta7\nNext Level\n\nta7\nCurrent Skill Level: 2\n\n".to_string() + TAIL
        );
        assert_eq!(c, 0);
        // L = 0: "First Level", no current block.
        f.levels.remove(&1);
        assert_eq!(
            text(&mut f, 0, true).0,
            "ta7\nFirst Level\n\n".to_string() + TAIL
        );
        // The dsc2 and synergy blocks get a separating line feed.
        f.cell(6, 5, 100, D, 8, 0);
        f.cell(12, 5, 100, D, 9, 0);
        assert_eq!(
            text(&mut f, 0, true).0,
            "ta9\n\nta7\nFirst Level\n\nta8\n\n".to_string() + TAIL
        );
    }

    #[test]
    fn a_maxed_skill_has_no_next_level_block() {
        let mut f = fake();
        f.cell(0, 5, 100, D, 7, 0);
        f.row().maxlvl = 0;
        f.levels.insert(1, (20, 21));
        assert_eq!(
            text(&mut f, 21, true).0,
            "ta7\nCurrent Skill Level: 21\n\n".to_string() + TAIL
        );
        // The requirement line says so.
        assert_eq!(
            text(&mut f, 21, false).0,
            "ta7\nCurrent Skill Level: 21\n\nMaximum Level Reached\n".to_string() + TAIL
        );
        // Below max: reqlevel + L; 0 → no line.
        f.levels.insert(1, (3, 3));
        f.row().reqlevel = 6;
        assert_eq!(
            text(&mut f, 3, false).0,
            "ta7\nCurrent Skill Level: 3\n\nRequired Level : 9\n".to_string() + TAIL
        );
        f.row().reqlevel = -3;
        assert_eq!(
            text(&mut f, 3, false).0,
            "ta7\nCurrent Skill Level: 3\n\n".to_string() + TAIL
        );
    }

    #[test]
    fn item_and_not_learned_notes() {
        let mut f = fake();
        f.cell(0, 5, 100, D, 7, 0);
        // Granted only by an item, and no class skill learned.
        f.levels.clear();
        f.levels.insert(1, (0, 1));
        assert_eq!(
            text(&mut f, 1, true).0,
            "ta7\nNext Level\n\nta7\nCurrent Skill Level: 1 (item)\n\n".to_string()
                + TAIL
                + "\n\nYou have been granted this skill by an item"
                + "\n\nYou have not learned this skill yet"
        );
        assert_eq!(
            text(&mut f, 1, false).0,
            "ta7\nCurrent Skill Level: 1 (item)\n\nRequired Level : 1\n".to_string()
                + TAIL
                + "\n\nYou have been granted this skill by an item"
                + "\n\nYou have not learned this skill yet"
        );
    }

    #[test]
    fn no_box_for_a_bad_skill_or_a_too_long_text() {
        let mut f = fake();
        assert!(box_text(&mut f, 99, 1, true).is_none());
        assert!(box_text(&mut f, -1, 1, true).is_none());
        f.strings.insert(LONG, w16(&"x".repeat(POPUP_MAX)));
        assert!(box_text(&mut f, 1, 1, true).is_none());
    }
}
