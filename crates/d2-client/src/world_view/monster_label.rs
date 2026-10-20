//! The mouse-over label of a hovered monster.
//!
//! A town NPC (`monstats` `interact`): the control panel's pop-up text
//! (text, then the box and glyphs) at
//! the unit's screen point, `PixHeight` above its feet, centred.
//!
//! A hostile monster (`interact` clear, `killable` set, `NameStr` ≠ 0):
//! the name with its health bar at the top of the screen
//! (row 0x20), centred on the part of the screen the open panels leave
//! free. The text is the space string `0xF9B`, again it,
//! the name, and two more of it; a corpse gets `Corpse` (0x90B) and one
//! space instead of the last two and is red (colour 1); a corpse in town
//! gets no label. The text colour is 4 for a unique monster, 3 for a
//! champion (it wins), 1 for a corpse, and 4 again for the classes 0xF3,
//! 0x14D, 0xF2, 0xD3, 0x220, 0x23A, 0xE5 and 0x2C0–0x2C5. The bar is
//! `value` / 128 of the box in dark red, `value` = life · 128 / max life
//! clamped to 2–128; a monster in mode 0 or 0xC has value 0
//! (the plain box).
//!
//! Under the pop-up (right after), two lines in
//! font 6, centred on the same free span: the monster type (`Demon` /
//! `Undead` and a space, then `DescStr`) in the label colour at y =
//! height(font 1) + 0x27, and the immunities (the
//! resistances of 100 or more, each in its own colour code) one font-1
//! line lower when the type line is not empty. A corpse has neither.
// A champion's name has its umod prefix (`champion_name`; measured on a
// hovered champion: "Champion Fallen"). The modifier line (built, drawn)
// is not drawn, only the shift it gives the immunity line: neither the
// recorded champion hover nor the unique one has such a row. PROVISIONAL
// (settled by a recorded superunique hover and the label's name
// branches): a unique's own
// name (monster data +0x2C) is not in the client model, the class `NameStr`
// is shown. Not modelled: the party-roster life percent and the unit flag
// 0x10000.

use crate::bridge::hover::unit_feet;
use crate::bridge::world::{ClientWorld, KindData, UnitKey};
use crate::rules::camera::Camera;
use crate::ui::draw::{TextRequest, TextStyle, UiDraw, UiDrawSink};
use crate::ui::original::hud_tips::{self, PopupBar};
use crate::ui::original::FontMeasure;
use crate::ui::text::TextOpts;
use crate::ui::{Point, StringLookup};

/// The unit type of a monster.
pub const MONSTER: u8 = 1;
/// String 0xF9B: a single space.
pub const STR_SPACE: u16 = 0xF9B;
/// String 0x90B: `Corpse`.
pub const STR_CORPSE: u16 = 0x90B;
/// The label's row (`push 0x20`).
pub const LABEL_Y: i32 = 0x20;
/// String 0x275E `Demon`, 0x275D `Undead`.
pub const STR_DEMON: u16 = 0x275E;
pub const STR_UNDEAD: u16 = 0x275D;
/// `DescStr` 0x1506: no description.
pub const DESC_NONE: u16 = 0x1506;
/// String 0xF9A: the colour-code lead; 0xF0C: the
/// immunity separator.
pub const STR_COLOR: u16 = 0xF9A;
pub const STR_SEPARATOR: u16 = 0xF0C;
/// The font of the type and immunity lines (`push 6`).
pub const LINE_FONT: u16 = 6;
/// The first line's offset below the font-1 height.
const LINE_Y: i32 = 0x27;
/// The immunity table (stat, string, colour code).
const IMMUNITIES: [(u16, u16, u8); 6] = [
    (36, 0x2765, 4),
    (37, 0x2766, 8),
    (39, 0x2761, 1),
    (41, 0x2763, 9),
    (43, 0x2762, 3),
    (45, 0x2764, 2),
];
/// The classes with no immunity line.
const NO_IMMUNITY_CLASSES: std::ops::RangeInclusive<u32> = 0x2C0..=0x2C5;
/// Monster mode 0 (death) and 0xC (dead).
const MODE_DEATH: u32 = 0;
const MODE_DEAD: u32 = 0xC;
/// Monster data type flags: champion, unique.
const FLAG_CHAMPION: u8 = 0x04;
const FLAG_UNIQUE: u8 = 0x08;
/// The classes whose label is gold (...).
const GOLD_CLASSES: [u32; 13] = [
    0xF3, 0x14D, 0xF2, 0xD3, 0x220, 0x23A, 0xE5, 0x2C4, 0x2C3, 0x2C2, 0x2C1, 0x2C5, 0x2C0,
];

/// Which UI states leave the screen free for the label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FreeScreen {
    /// One of the states 0x17, 0x19, 0x1A, 0x0C, 0x1C, 0x1B is open: no
    /// label.
    pub blocked: bool,
    /// A left panel is open (states 0x1D, 0x16, 2, 0x0F, 0x24, 0x25, 0x14):
    /// the label's span starts at W / 2.
    pub left: bool,
    /// A right panel is open (states 1, 4): the span ends at W / 2.
    pub right: bool,
}

impl FreeScreen {
    /// The span (left, right) of the free part of a screen of width `w`;
    /// `None` when returns 0.
    ///
    /// An open side panel takes its columns from its edge of the frame
    /// ([`crate::rules::camera::side_panel_span`]: W / 2 at 4:3); a
    /// blocking panel set takes both sides, which at 4:3 leaves nothing
    /// and in a wider frame leaves the world between the panels.
    pub fn span(self, w: i32) -> Option<(i32, i32)> {
        let span = crate::rules::camera::side_panel_span(w);
        let left = if self.left || self.blocked { span } else { 0 };
        let right = if self.right || self.blocked {
            w - span
        } else {
            w
        };
        (left < right).then_some((left, right))
    }
}

/// What the label draw reads besides the world.
pub struct LabelEnv<'a> {
    pub screen: (i32, i32),
    pub fonts: Option<&'a FontMeasure>,
    pub free: FreeScreen,
    /// The palette index of the bar's red (the nearest palette entry to
    /// (0x50, 0, 0));
    /// `None` until the host sets the palette (the plain box).
    pub bar_color: Option<u8>,
}

pub use d2_server::game::rows::MonsterLabelRow;

/// The rows by monster class id.
#[derive(Clone, Debug, Default)]
pub struct MonsterLabels {
    pub rows: Vec<MonsterLabelRow>,
}

/// The health value of a monster: 0 in mode 0 / 0xC,
/// else life · 128 / max life clamped to 2–128.
pub fn health_value(mode: u32, life: i32, max_life: i32) -> i32 {
    if mode == MODE_DEATH || mode == MODE_DEAD {
        return 0;
    }
    if max_life == 0 {
        return 0x80;
    }
    let v = (life << 7) / max_life;
    v.clamp(2, 0x80)
}

/// The champion name prefixes by umod id (the umod handler's table):
/// umod 0x10 "Champion" (`string.tbl` 3220), 0x24
/// "Ghostly", 0x25 "Fanatic", 0x26 "Possessed", 0x27 "Berserker"
/// (`patchstring` 11084-11087).
const CHAMPION_PREFIXES: [(u8, u16); 5] = [
    (0x10, 3220),
    (0x24, 11084),
    (0x25, 11085),
    (0x26, 11086),
    (0x27, 11087),
];

/// The format "%0 %1" (`patchstring` 11072) of a champion's name.
const STR_CHAMPION_FORMAT: u16 = 11072;

/// A champion's name (through the umod handlers): the format filled
/// with the prefix of the last umod byte
/// in the table and the class name; every other monster the class name.
/// The unique and superunique names stay the class name (PROVISIONAL, see
/// above).
fn champion_name(kind: &KindData, class_name: &[u16], strings: &dyn StringLookup) -> Vec<u16> {
    let KindData::Monster(m) = kind else {
        return class_name.to_vec();
    };
    let prefix = (m.flags & FLAG_CHAMPION != 0)
        .then(|| {
            m.umods
                .iter()
                .rev()
                .find_map(|u| CHAMPION_PREFIXES.iter().find(|(id, _)| id == u))
        })
        .flatten();
    let (Some((_, id)), Some(fmt)) = (prefix, strings.get_id(STR_CHAMPION_FORMAT)) else {
        return class_name.to_vec();
    };
    let Some(p) = strings.get_id(*id) else {
        return class_name.to_vec();
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < fmt.len() {
        if fmt[i] == u16::from(b'%') && i + 1 < fmt.len() {
            match fmt[i + 1] {
                0x30 => out.extend_from_slice(p),
                0x31 => out.extend_from_slice(class_name),
                c => out.extend([0x25, c]),
            }
            i += 2;
        } else {
            out.push(fmt[i]);
            i += 1;
        }
    }
    out
}

/// The label colour (module doc).
pub fn label_color(class: u32, flags: u8, dead: bool) -> u16 {
    let mut c = 0;
    if flags & FLAG_UNIQUE != 0 {
        c = 4;
    }
    if flags & FLAG_CHAMPION != 0 {
        c = 3;
    }
    if dead {
        c = 1;
    }
    if GOLD_CLASSES.contains(&class) {
        c = 4;
    }
    c
}

impl MonsterLabels {
    /// The pop-up draws of the hovered unit `hover`: the NPC name, or the
    /// hostile monster's name with its health bar; nothing for other
    /// units.
    pub fn draw(
        &self,
        world: &ClientWorld,
        cam: &Camera,
        hover: Option<UnitKey>,
        strings: &dyn StringLookup,
        env: &LabelEnv<'_>,
    ) -> Vec<UiDraw> {
        let mut out: Vec<UiDraw> = Vec::new();
        let Some(key) = hover.filter(|k| k.unit_type == MONSTER) else {
            return out;
        };
        let Some(u) = world.units.get(&key) else {
            return out;
        };
        let Some(row) = self.rows.get(u.class as usize) else {
            return out;
        };
        if row.interact {
            let (Some(cell), Some(text)) = (u.position, strings.get_id(row.name_str)) else {
                return out;
            };
            let (x, y) = unit_feet(cam, key.unit_type, cell);
            let at = Point::new(x, y - i32::from(row.pix_height));
            hud_tips::push_popup(
                text.to_vec(),
                at,
                0,
                true,
                env.screen,
                env.fonts,
                &mut out as &mut dyn UiDrawSink,
            );
            return out;
        }
        if !(row.killable && row.name_str != 0) {
            return out;
        }
        let dead = u.mode == MODE_DEAD;
        if dead && crate::bridge::modes::in_town(world, key) {
            return out;
        }
        let Some(class_name) = strings.get_id(row.name_str) else {
            return out;
        };
        let name = &champion_name(&u.kind, class_name, strings);
        let space = strings
            .get_id(STR_SPACE)
            .map_or_else(|| vec![0x20], <[u16]>::to_vec);
        let mut text = Vec::new();
        text.extend_from_slice(&space);
        text.extend_from_slice(&space);
        text.extend_from_slice(name);
        text.extend_from_slice(&space);
        if dead {
            if let Some(c) = strings.get_id(STR_CORPSE) {
                text.extend_from_slice(c);
            }
            text.extend_from_slice(&space);
        }
        text.extend_from_slice(&space);
        let flags = match &u.kind {
            KindData::Monster(m) => m.flags,
            _ => 0,
        };
        let color = label_color(u.class, flags, dead);
        let Some((left, right)) = env.free.span(env.screen.0) else {
            return out;
        };
        let width = env
            .fonts
            .and_then(|f| f.width_a(hud_tips::TIP_FONT, &text))
            .unwrap_or(0);
        let x = ((left + right) >> 1) - (width >> 1);
        let value = health_value(u.mode, world.total(key, 6, 0), world.total(key, 7, 0));
        hud_tips::push_popup_bar(
            text,
            Point::new(x, LABEL_Y),
            color,
            false,
            env.screen,
            env.fonts,
            env.bar_color.map(|color| PopupBar { value, color }),
            &mut out as &mut dyn UiDrawSink,
        );
        if !dead {
            self.push_lines(world, key, row, flags, color, strings, env, &mut out);
        }
        out
    }

    /// The type and immunity lines (module doc).
    #[allow(clippy::too_many_arguments)]
    fn push_lines(
        &self,
        world: &ClientWorld,
        key: UnitKey,
        row: &MonsterLabelRow,
        flags: u8,
        color: u16,
        strings: &dyn StringLookup,
        env: &LabelEnv<'_>,
        out: &mut Vec<UiDraw>,
    ) {
        let Some(class) = world.units.get(&key).map(|u| u.class) else {
            return;
        };
        let s = |id: u16| strings.get_id(id).unwrap_or(&[]);
        // A champion or unique gets the modifier line instead
        // of the type line.
        let mods = flags & (FLAG_CHAMPION | FLAG_UNIQUE) != 0;
        let mut kind = Vec::new();
        if !mods {
            let type_str = if row.demon {
                STR_DEMON
            } else if row.undead {
                STR_UNDEAD
            } else {
                0
            };
            if type_str != 0 {
                kind.extend_from_slice(s(type_str));
                kind.extend_from_slice(s(STR_SPACE));
            }
            if row.desc_str != DESC_NONE {
                kind.extend_from_slice(s(row.desc_str));
            }
        }
        let mut imm = Vec::new();
        if !NO_IMMUNITY_CLASSES.contains(&class) {
            for (stat, id, code) in IMMUNITIES {
                if world.total(key, stat, 0) < 100 {
                    continue;
                }
                if !imm.is_empty() {
                    imm.extend_from_slice(s(STR_SEPARATOR));
                    imm.extend_from_slice(s(STR_SPACE));
                }
                let text = s(id);
                if !text.is_empty() {
                    imm.extend_from_slice(s(STR_COLOR));
                    imm.push(u16::from(b'0' + code));
                    imm.extend_from_slice(text);
                }
            }
        }
        let height = env
            .fonts
            .and_then(|f| f.font_height(hud_tips::TIP_FONT))
            .unwrap_or(0);
        let y1 = height + LINE_Y;
        let y2 = if !kind.is_empty() || mods {
            y1 + height
        } else {
            y1
        };
        self.push_line(kind, y1, color, env, out);
        self.push_line(imm, y2, 4, env, out);
    }

    /// With (6, y, colour): `text` centred on the free span.
    fn push_line(
        &self,
        text: Vec<u16>,
        y: i32,
        color: u16,
        env: &LabelEnv<'_>,
        out: &mut Vec<UiDraw>,
    ) {
        if text.is_empty() {
            return;
        }
        let Some((left, right)) = env.free.span(env.screen.0) else {
            return;
        };
        let width = env
            .fonts
            .and_then(|f| f.width_a(LINE_FONT, &text))
            .unwrap_or(0);
        let x = ((left + right) >> 1) - (width >> 1);
        out.push(UiDraw::Text(TextRequest {
            text,
            at: Point::new(x, y),
            style: TextStyle {
                font: LINE_FONT,
                color,
            },
            opts: TextOpts::centered(),
            clip: crate::ui::geom::screen_clip(env.screen),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_value_clamps_and_dead_is_zero() {
        assert_eq!(health_value(1, 0x8000, 0x8000), 128);
        assert_eq!(health_value(1, 0x4000, 0x8000), 64);
        // Under 3 -> 2; over 127 -> 128.
        assert_eq!(health_value(1, 0x100, 0x8000), 2);
        assert_eq!(health_value(1, 0x9000, 0x8000), 128);
        assert_eq!(health_value(0, 0x8000, 0x8000), 0);
        assert_eq!(health_value(0xC, 0x8000, 0x8000), 0);
        // No max life: full.
        assert_eq!(health_value(1, 5, 0), 128);
    }

    // (the immunity line colour)
    #[test]
    fn label_colour_order_unique_champion_dead_class() {
        assert_eq!(label_color(1, 0, false), 0);
        assert_eq!(label_color(1, FLAG_UNIQUE, false), 4);
        assert_eq!(label_color(1, FLAG_UNIQUE | FLAG_CHAMPION, false), 3);
        assert_eq!(label_color(1, FLAG_CHAMPION, true), 1);
        // The class list wins over all of them.
        assert_eq!(label_color(0xF3, 0, true), 4);
    }

    #[test]
    fn free_span_follows_the_open_panels() {
        let f = FreeScreen::default();
        assert_eq!(f.span(800), Some((0, 800)));
        assert_eq!(FreeScreen { left: true, ..f }.span(800), Some((400, 800)));
        assert_eq!(FreeScreen { right: true, ..f }.span(800), Some((0, 400)));
        assert_eq!(
            FreeScreen {
                left: true,
                right: true,
                ..f
            }
            .span(800),
            None
        );
        assert_eq!(FreeScreen { blocked: true, ..f }.span(800), None);
        // The 16:9 frame opens its panels at its edges, 320 columns each:
        // the label keeps the world beside and between them.
        assert_eq!(FreeScreen { left: true, ..f }.span(1068), Some((320, 1068)));
        assert_eq!(FreeScreen { right: true, ..f }.span(1068), Some((0, 748)));
        assert_eq!(
            FreeScreen { blocked: true, ..f }.span(1068),
            Some((320, 748))
        );
    }

    use crate::assets::path::MemorySource;
    use crate::bridge::world::{ClientUnit, MonsterData};
    use crate::rules::camera::{ClientPos, FrameSize, OpenMode};
    use crate::ui::draw::RectRequest;
    use crate::ui::font_info;

    struct Strs;
    impl StringLookup for Strs {
        fn get(&self, _: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            // Leaked once per call: a test-only lookup of static text.
            let leak = |t: &str| -> &'static [u16] {
                Box::leak(t.encode_utf16().collect::<Vec<_>>().into_boxed_slice())
            };
            let t: &'static [u16] = match id {
                STR_DEMON => leak("Demon"),
                STR_COLOR => leak("\u{ff}c"),
                STR_SEPARATOR => leak(","),
                0x2761 => leak("Immune to Fire"),
                0x2762 => leak("Immune to Cold"),
                STR_SPACE => Box::leak(vec![0x20u16].into_boxed_slice()),
                STR_CORPSE => Box::leak(
                    "Corpse"
                        .encode_utf16()
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                ),
                7 => Box::leak(
                    "Fallen"
                        .encode_utf16()
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                ),
                _ => return None,
            };
            Some(t)
        }
    }

    /// A `.tbl`: 256 records of width 6.
    fn font_tbl() -> Vec<u8> {
        let mut d = b"Woo!".to_vec();
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&256u16.to_le_bytes());
        d.extend_from_slice(&[10, 0]);
        for i in 0..256u16 {
            d.extend_from_slice(&i.to_le_bytes());
            d.extend_from_slice(&[0, 6, 10, 0, 0, 0]);
            d.extend_from_slice(&i.to_le_bytes());
            d.extend_from_slice(&[0; 4]);
        }
        d
    }

    fn fonts() -> FontMeasure {
        let mut src = MemorySource::default();
        for id in crate::ui::original::CHARACTER_FONTS {
            src.insert(font_info(id).unwrap().tbl_path, font_tbl());
        }
        FontMeasure::load(&src, &crate::ui::original::CHARACTER_FONTS).unwrap()
    }

    const MON: UnitKey = UnitKey {
        unit_type: MONSTER,
        guid: 9,
    };

    /// A Fallen (class 3, `NameStr` 7) with `life` of 128 at mode `mode`.
    fn world(mode: u32, life: i32, flags: u8) -> ClientWorld {
        let mut w = ClientWorld::default();
        let mut u = ClientUnit::new(MON);
        u.class = 3;
        u.mode = mode;
        u.position = Some((1000, 1000));
        u.stats.insert(7, 0x8000);
        u.stats.insert(6, life << 8);
        u.kind = KindData::Monster(Box::new(MonsterData {
            flags,
            ..Default::default()
        }));
        w.units.insert(MON, u);
        w
    }

    fn labels() -> MonsterLabels {
        let row = |killable| MonsterLabelRow {
            name_str: 7,
            interact: false,
            killable,
            pix_height: 40,
            desc_str: DESC_NONE,
            demon: true,
            undead: false,
        };
        MonsterLabels {
            rows: vec![row(false), row(false), row(false), row(true)],
        }
    }

    fn draw(w: &ClientWorld, free: FreeScreen, fonts: &FontMeasure) -> Vec<UiDraw> {
        let cam = Camera::new(
            FrameSize::play(),
            OpenMode::NONE,
            ClientPos { x: 0, y: 0 },
            (0, 0),
        );
        let env = LabelEnv {
            screen: (800, 600),
            fonts: Some(fonts),
            free,
            bar_color: Some(109),
        };
        labels().draw(w, &cam, Some(MON), &Strs, &env)
    }

    fn rects(d: &[UiDraw]) -> Vec<RectRequest> {
        d.iter()
            .filter_map(|d| match d {
                UiDraw::Rect(r) => Some(*r),
                _ => None,
            })
            .collect()
    }

    fn texts(d: &[UiDraw]) -> Vec<TextRequest> {
        d.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    fn text(d: &[UiDraw]) -> TextRequest {
        d.iter()
            .find_map(|d| match d {
                UiDraw::Text(t) => Some(t.clone()),
                _ => None,
            })
            .expect("a text draw")
    }

    // (playtest B010)
    #[test]
    fn a_hovered_hostile_monster_gets_its_name_and_a_health_bar() {
        let f = fonts();
        // Half life: the left half of the box in the bar colour (mode 1),
        // the rest in the translucent black (mode 2).
        let d = draw(&world(1, 64, 0), FreeScreen::default(), &f);
        let r = rects(&d);
        assert_eq!(r.len(), 2, "{d:?}");
        let width = r[1].x1 - r[0].x0;
        assert_eq!((r[0].color, r[0].mode), (109, 1));
        assert_eq!(r[0].x1 - r[0].x0, (64 * width) >> 7);
        assert_eq!((r[1].color, r[1].mode, r[1].x0), (0, 2, r[0].x1));
        // The name between spaces: "  Fallen  ", colour 0.
        let t = text(&d);
        assert_eq!(String::from_utf16_lossy(&t.text), "  Fallen  ");
        assert_eq!(t.style.color, 0);
        // The text is centred on the screen (span [0, 800]); the box is
        // the text's width + 8 from the text's left: 4 px right of centre.
        assert_eq!((r[0].x0 + r[1].x1) / 2, 400 + 4);
        assert_eq!(r[0].y0.max(r[0].y1), 0x20 + 2);
    }

    #[test]
    fn full_life_fills_the_box_and_a_champion_is_blue() {
        let f = fonts();
        let d = draw(
            &world(1, 128, FLAG_CHAMPION | FLAG_UNIQUE),
            FreeScreen::default(),
            &f,
        );
        let r = rects(&d);
        assert_eq!(r.len(), 1, "the whole box is the bar: {r:?}");
        assert_eq!((r[0].color, r[0].mode), (109, 1));
        assert_eq!(text(&d).style.color, 3);
    }

    // (a corpse has no bar, reads Corpse and is red.)
    #[test]
    fn a_corpse_is_red_with_the_plain_box() {
        let f = fonts();
        let d = draw(&world(0xC, 0, 0), FreeScreen::default(), &f);
        let r = rects(&d);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].color, r[0].mode), (0, 2));
        let t = text(&d);
        assert_eq!(String::from_utf16_lossy(&t.text), "  Fallen Corpse  ");
        assert_eq!(t.style.color, 1);
    }

    #[test]
    fn an_open_panel_moves_or_removes_the_label() {
        let f = fonts();
        let w = world(1, 128, 0);
        let both = FreeScreen {
            left: true,
            right: true,
            ..FreeScreen::default()
        };
        assert!(draw(&w, both, &f).is_empty());
        let blocked = FreeScreen {
            blocked: true,
            ..FreeScreen::default()
        };
        assert!(draw(&w, blocked, &f).is_empty());
        // A left panel: the span is [400, 800], the box centres on 600.
        let left = FreeScreen {
            left: true,
            ..FreeScreen::default()
        };
        let r = rects(&draw(&w, left, &f));
        assert_eq!((r[0].x0 + r[r.len() - 1].x1) / 2, 600 + 4);
    }

    #[test]
    fn a_class_that_is_not_killable_has_no_label() {
        let f = fonts();
        let mut w = world(1, 128, 0);
        w.units.get_mut(&MON).unwrap().class = 0;
        assert!(draw(&w, FreeScreen::default(), &f).is_empty());
    }

    // (draws-fx-frozen-armor-flight t302: "Demon " in font 6 at y 49)
    #[test]
    fn a_demon_gets_its_type_line_under_the_name() {
        let f = fonts();
        let d = draw(&world(1, 128, 0), FreeScreen::default(), &f);
        let t = texts(&d);
        assert_eq!(t.len(), 2, "{d:?}");
        assert_eq!(String::from_utf16_lossy(&t[1].text), "Demon ");
        assert_eq!((t[1].style.font, t[1].style.color), (LINE_FONT, 0));
        // y = font-1 height 10 + 0x27; x = 400 - (6 * 6 >> 1).
        assert_eq!(t[1].at, Point::new(400 - 18, 49));
        assert_eq!(t[1].opts, TextOpts::centered());
        // A left panel centres it on 600.
        let left = FreeScreen {
            left: true,
            ..FreeScreen::default()
        };
        assert_eq!(texts(&draw(&world(1, 128, 0), left, &f))[1].at.x, 600 - 18);
    }

    #[test]
    fn immunities_follow_the_type_line_in_their_colours() {
        let f = fonts();
        let mut w = world(1, 128, 0);
        let u = w.units.get_mut(&MON).unwrap();
        u.stats.insert(39, 100);
        u.stats.insert(43, 150);
        u.stats.insert(41, 99);
        let t = texts(&draw(&w, FreeScreen::default(), &f));
        assert_eq!(t.len(), 3);
        assert_eq!(
            String::from_utf16_lossy(&t[2].text),
            "\u{ff}c1Immune to Fire, \u{ff}c3Immune to Cold"
        );
        assert_eq!((t[2].style.font, t[2].style.color, t[2].at.y), (6, 4, 59));
        // A champion: no type line, the immunities still one line down
        // (the modifier line's place).
        let u = w.units.get_mut(&MON).unwrap();
        u.kind = KindData::Monster(Box::new(MonsterData {
            flags: FLAG_CHAMPION,
            ..Default::default()
        }));
        let t = texts(&draw(&w, FreeScreen::default(), &f));
        assert_eq!(t.len(), 2);
        assert_eq!(t[1].at.y, 59);
    }

    #[test]
    fn a_corpse_or_a_plain_monster_has_no_second_line() {
        let f = fonts();
        assert_eq!(
            texts(&draw(&world(0xC, 0, 0), FreeScreen::default(), &f)).len(),
            1
        );
        let mut l = labels();
        l.rows[3].demon = false;
        let cam = Camera::new(
            FrameSize::play(),
            OpenMode::NONE,
            ClientPos { x: 0, y: 0 },
            (0, 0),
        );
        let env = LabelEnv {
            screen: (800, 600),
            fonts: Some(&f),
            free: FreeScreen::default(),
            bar_color: None,
        };
        let d = l.draw(&world(1, 128, 0), &cam, Some(MON), &Strs, &env);
        assert_eq!(texts(&d).len(), 1);
    }

    #[test]
    fn a_champion_name_has_its_umod_prefix() {
        struct S;
        impl StringLookup for S {
            fn get(&self, _: &str) -> Option<&[u16]> {
                None
            }
            fn get_id(&self, id: u16) -> Option<&[u16]> {
                let leak = |t: &str| -> &'static [u16] {
                    Box::leak(t.encode_utf16().collect::<Vec<_>>().into_boxed_slice())
                };
                Some(match id {
                    11072 => leak("%0 %1"),
                    3220 => leak("Champion"),
                    11084 => leak("Ghostly"),
                    _ => return None,
                })
            }
        }
        let name: Vec<u16> = "Fallen".encode_utf16().collect();
        let kind = |flags: u8, umods: [u8; 9]| {
            KindData::Monster(Box::new(MonsterData {
                flags,
                umods,
                ..Default::default()
            }))
        };
        let text = |k: &KindData| String::from_utf16_lossy(&champion_name(k, &name, &S));
        let mut m = [0u8; 9];
        m[0] = 16;
        assert_eq!(text(&kind(FLAG_CHAMPION, m)), "Champion Fallen");
        m[1] = 0x24;
        assert_eq!(text(&kind(FLAG_CHAMPION, m)), "Ghostly Fallen");
        // Not a champion, or no prefixed umod: the class name.
        assert_eq!(text(&kind(0, m)), "Fallen");
        assert_eq!(
            text(&kind(FLAG_CHAMPION, [5, 0, 0, 0, 0, 0, 0, 0, 0])),
            "Fallen"
        );
    }
}
