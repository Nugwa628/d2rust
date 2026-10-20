//! Character panel (ui 2; left slot): panel art, close button, the stat-points
//! block with its four add buttons, the 15 labels and the stat values, plus the
//! mouse handlers (down and up).
//!
//! Every draw, text and hit comes from the panel-2 rows of
//! `panel-layout.tsv`, in file order (the order of the panel's
//! parts). The panel never decides an outcome: a stat spend leaves as
//! C→S 0x3A intents, the close button as `SetUIState(2, off)`.
//!
//! Open questions:
//! - experience (13) and next-level (30) use the digit-grouping format
//!   (`char_details::group_digits`); the level (12) is a plain value.
//! - the popup width of a value is not known; it is asked of
//!   [`CharacterView::popup_width`], and a value whose font depends on it
//!   is not drawn while that answer is `None`.
//! - the number format of the other values is assumed `%i` (the format
//!   the panel uses for the stat points); the stat values' format is not known.
//! - compares "value" with "base" for stats 7, 9, 11: read here as the
//!   unshifted stat values (the `>> 8` is named for the display only).
//! - resistance color: both effects active is one case for
//!   [`ResistEffect`] (lowering wins, so `Lowered`); the effect tests
//!   themselves are not wired (the view answers them).
//! - the damage / attack-rating block, the name and class lines and the hover
//!   texts are not drawn; the close button tool tip is
//!   queued by the hover owner.

use super::super::draw::UiDrawSink;
use super::super::geom::{Point, Rect};
use super::super::layout::{ColorSpec, Cond, FrameSpec, LayoutRow, PanelKey, RowKind, Screen};
use super::super::panel::{ClientIntent, StringLookup};
use super::super::text::LF;
use super::{cel, centered_in, text, utf16, PanelEnv, PanelOutput, PanelTables, TextMeasure};

/// The ui id of the character panel.
pub const UI_CHARACTER: u8 = 2;
/// Where [`CharacterPanel::draw_staged`] calls its hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// After the labels (before the close button).
    Labels,
    /// After the close button (before the values).
    Close,
}

const PANEL: PanelKey = PanelKey::Ui(UI_CHARACTER);

/// Stat ids (`itemstatcost.txt` rows) the panel reads.
pub const STAT_STATPTS: u16 = 4;
pub const STAT_HITPOINTS: u16 = 6;
pub const STAT_DEFENSE: u16 = 31;

/// Font ids: Font8 (the panel's "font 0").
pub const FONT8: u16 = 0;

/// Most points one 0x3A message spends (byte +2 is `n − 1`, chunks of at most
/// 32).
pub const STAT_CHUNK: i32 = 32;

/// The add buttons in table order: strength,
/// dexterity, vitality, energy. Index = the button's pressed field.
pub const ADD_BUTTON_STATS: [u16; 4] = [0, 2, 3, 1];

/// The resist stats and their max-resist stats.
const RESISTS: [(u16, u16); 4] = [(39, 40), (43, 44), (41, 42), (45, 46)];

/// Stats colored by the `cmp` rule (besides the resistances).
const CMP_STATS: [u16; 9] = [0, 2, 3, 1, 7, 9, 11, 12, 31];

/// Which resist effect is active on the player (the state tests): drives
/// the resist value color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ResistEffect {
    #[default]
    None,
    /// A resist-raising effect: blue (3).
    Raised,
    /// A resist-lowering effect: red (1).
    Lowered,
}

/// The player facts the character panel reads (bridge snapshot).
pub trait CharacterView {
    /// Stat value of the player, unshifted.
    fn stat(&self, id: u16) -> i32;
    /// Unmodified (base) stat value, unshifted.
    fn base(&self, id: u16) -> i32;
    /// The player is alive (stat 6 shown as at least 1).
    fn alive(&self) -> bool;
    /// Language id (language 6 sends defense through the popup-width test).
    fn language(&self) -> u8;
    /// Resistance penalty of the current difficulty (classic game 0 / 20 / 50;
    /// expansion game the difficulty table value).
    fn resist_penalty(&self) -> i32;
    /// Active resist effect for resist stat `id`.
    fn resist_effect(&self, id: u16) -> ResistEffect;
    /// Popup width of a value string. Not known
    /// yet: `None` (the value is then not drawn when its font depends on
    /// it).
    fn popup_width(&self, _text: &[u16]) -> Option<i32> {
        None
    }
    /// The next-level value (the `experience.txt` entry of the
    /// player's class for its level, or stat 30 at the maximum level);
    /// `None`: stat 30.
    fn next_level(&self) -> Option<u32> {
        None
    }
    /// The defense colour of the `armblue` / `armred` / `holyshield`
    /// states; `None`: the value / base compare.
    fn defense_color(&self) -> Option<u16> {
        None
    }
}

/// Pressed state of the character panel (the panel's pressed flag and the pressed
/// fields of table).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CharacterPanel {
    pub close_pressed: bool,
    /// Pressed field per add button, in [`ADD_BUTTON_STATS`] order.
    pub stat_pressed: [bool; 4],
}

/// The add-button index of a `btn_statN` item.
fn button_index(item: &str) -> Option<usize> {
    let stat: u16 = item.strip_prefix("btn_stat")?.parse().ok()?;
    ADD_BUTTON_STATS.iter().position(|&s| s == stat)
}

/// The stat id of a `value_statN` item.
fn value_stat(item: &str) -> Option<u16> {
    item.strip_prefix("value_stat")?.parse().ok()
}

/// The centering span `[x, x + w − 1]` of a `centered` text row.
fn span(r: &LayoutRow, s: &Screen) -> Option<(i32, i32)> {
    let a = r.x.eval(s);
    Some((a, a + r.w? - 1))
}

/// The character panel's click area: x in
/// [`sx`, `W / 2 − 1`] at 4:3 (the panel slot; `Screen::left_end`), y in
/// [`sy`, `H + sy − 49`].
pub fn area(s: &Screen) -> Rect {
    let w = u16::try_from(s.left_end() - s.sx()).unwrap_or(0);
    let h = u16::try_from(s.h - 48).unwrap_or(0);
    Rect::new(s.sx(), s.sy(), w, h)
}

/// The shown value and color of a non-resist value row (font handled by
/// the caller). `None`: not drawn (open: thousands grouping).
fn plain_value(view: &dyn CharacterView, stat: u16) -> Option<(i32, u16)> {
    // Experience and next-level are grouped by the digit-grouping format and
    // drawn by `draw_text_row`.
    if matches!(stat, 13 | 30) {
        return None;
    }
    let value = view.stat(stat);
    let base = view.base(stat);
    let mut shown = if (6..=11).contains(&stat) {
        value >> 8
    } else {
        value
    };
    if stat == STAT_HITPOINTS && view.alive() && shown < 1 {
        shown = 1;
    }
    let color = if stat == STAT_DEFENSE && view.defense_color().is_some() {
        view.defense_color().unwrap_or(0)
    } else if CMP_STATS.contains(&stat) {
        cmp_color(value, base)
    } else {
        0
    };
    Some((shown, color))
}

/// 3 (blue) when value > base, 1 (red) when value < base, else 0.
fn cmp_color(value: i32, base: i32) -> u16 {
    match value.cmp(&base) {
        std::cmp::Ordering::Greater => 3,
        std::cmp::Ordering::Less => 1,
        std::cmp::Ordering::Equal => 0,
    }
}

/// Shown resist = stat − penalty, clamped to [−100, cap], cap =
/// min(75 + max-resist stat, 95). Colour in this order:
/// 3 when a raising effect is active, then 1
/// when a lowering one is (lowering wins); then at the cap, colour 4
/// unless it is 3; below it, a shown value < 0 is 1 (also over 3).
fn resist_value(view: &dyn CharacterView, stat: u16, max_stat: u16) -> (i32, u16) {
    let cap = (75 + view.stat(max_stat)).min(95);
    let mut color = match view.resist_effect(stat) {
        ResistEffect::Raised => 3,
        ResistEffect::Lowered => 1,
        ResistEffect::None => 0,
    };
    let v = (view.stat(stat) - view.resist_penalty()).max(-100);
    let shown = if v >= cap {
        if color != 3 {
            color = 4;
        }
        cap
    } else {
        if v < 0 {
            color = 1;
        }
        v
    };
    (shown, color)
}

impl CharacterPanel {
    /// Clickable controls from the hit rows used by press and release.
    /// `statpts` is zero when the stat block is unavailable.
    pub fn ui_elements(
        &self,
        t: &PanelTables,
        s: &Screen,
        statpts: i32,
    ) -> Vec<crate::ui::ui_ids::UiElement> {
        use crate::ui::ui_ids::UiElement;
        let mut out = Vec::new();
        if let Some(r) = close_rect(t, s) {
            out.push(UiElement::new("character.close", r));
        }
        if statpts > 0 {
            for r in t.rows(PANEL).filter(|r| r.kind == RowKind::Hit) {
                if let Some(i) = button_index(&r.item) {
                    if let Some(rect) = r.hit_rect(s) {
                        out.push(UiElement::new(
                            format!("character.stat({})", ADD_BUTTON_STATS[i]),
                            rect,
                        ));
                    }
                }
            }
        }
        out
    }

    /// The pressed flag a draw row tests (`released` / `pressed`).
    fn row_pressed(&self, r: &LayoutRow) -> bool {
        if r.item == "close" {
            self.close_pressed
        } else if let Some(i) = button_index(&r.item) {
            self.stat_pressed[i]
        } else {
            false
        }
    }

    /// Draws the panel in `panel-layout.tsv` row order.
    pub fn draw(
        &self,
        t: &PanelTables,
        env: &PanelEnv,
        view: &dyn CharacterView,
        measure: &dyn TextMeasure,
        strings: &dyn StringLookup,
        out: &mut dyn UiDrawSink,
    ) {
        self.draw_staged(t, env, view, measure, strings, out, &mut |_, _| {});
    }

    /// The panel in draw order:
    /// the art, the 15 labels, the skill damage blocks (`hook(Stage::Labels)`),
    /// the close button, the class and name lines (`hook(Stage::Close)`), then
    /// the values. The stat-points box and the add buttons (open only with
    /// points to spend) follow the art and precede the 15 labels (measured with a
    /// level-5 character with unspent points).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_staged(
        &self,
        t: &PanelTables,
        env: &PanelEnv,
        view: &dyn CharacterView,
        measure: &dyn TextMeasure,
        strings: &dyn StringLookup,
        out: &mut dyn UiDrawSink,
        hook: &mut dyn FnMut(Stage, &mut dyn UiDrawSink),
    ) {
        let s = env.screen;
        // The base `statpts` gates the points box,
        // its labels, the number and the add buttons; the number is the
        // full value.
        let statpts = view.stat(STAT_STATPTS);
        let shown = super::char_details::points_block_drawn(view.base(STAT_STATPTS));
        let extra = move |c: Cond| c == Cond::StatPts && shown;
        // Measured `ui-draws-inv-char-l5-ama`: the points box (art, its two
        // label lines, the number, the add buttons) follows the art and
        // precedes the 15 labels.
        let phase = |r: &LayoutRow| match r.item.as_str() {
            i if i.starts_with("art") => 0,
            _ if r.has(Cond::StatPts) => 1,
            i if i.starts_with("label") => 2,
            "close" => 3,
            _ => 4,
        };
        for ph in 0..5 {
            for r in t.rows(PANEL).filter(|r| phase(r) == ph) {
                let cenv = env.cond(self.row_pressed(r), &extra);
                if !r.applies(&cenv) {
                    continue;
                }
                match r.kind {
                    RowKind::Hit => {}
                    RowKind::Draw => {
                        let FrameSpec::Index(f) = r.frame else {
                            continue;
                        };
                        if let Some(file) = t.files.row_file(r, None) {
                            out.push(cel(file, f, r.x.eval(&s), r.y.eval(&s)));
                        }
                    }
                    RowKind::Text => {
                        self.draw_text_row(r, &s, view, statpts, measure, strings, out)
                    }
                }
            }
            match ph {
                2 => hook(Stage::Labels, out),
                3 => hook(Stage::Close, out),
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text_row(
        &self,
        r: &LayoutRow,
        s: &Screen,
        view: &dyn CharacterView,
        statpts: i32,
        measure: &dyn TextMeasure,
        strings: &dyn StringLookup,
        out: &mut dyn UiDrawSink,
    ) {
        let (Some((a, b)), Some(font)) = (span(r, s), r.font) else {
            return;
        };
        let y = r.y.eval(s);
        let centered = |s16: Vec<u16>, y: i32, font: u16, color: u16, out: &mut dyn UiDrawSink| {
            if let Some(wd) = measure.width(font, &s16) {
                out.push(text(s16, centered_in(a, b, wd), y, font, color));
            }
        };
        match (&r.frame, &r.color) {
            // Statbox labels and stat labels: a string id.
            (FrameSpec::Index(id), ColorSpec::Index(k)) => {
                let Some(s16) = u16::try_from(*id).ok().and_then(|id| strings.get_id(id)) else {
                    return;
                };
                // A LF before the end splits the label; each part
                // centered on its own, at y − 4 and y + 4.
                let lf = s16.iter().position(|&u| u == LF);
                match lf {
                    Some(i) if r.has(Cond::SplitLf) && i + 1 < s16.len() => {
                        centered(s16[..i].to_vec(), y - 4, font, *k, out);
                        centered(s16[i + 1..].to_vec(), y + 4, font, *k, out);
                    }
                    _ => centered(s16.to_vec(), y, font, *k, out),
                }
            }
            // The stat points value, `%i`.
            (FrameSpec::Word(w), ColorSpec::Index(k)) if w == "value" && r.item == "statpts" => {
                centered(utf16(&statpts.to_string()), y, font, *k, out);
            }
            // Stat values.
            (FrameSpec::Word(w), ColorSpec::Word(c)) if w == "value" && c == "cmp" => {
                let Some(stat) = value_stat(&r.item) else {
                    return;
                };
                // Experience (13) and next level (30): unsigned
                // decimal with `,` every 3 digits, Font16, color 0, no
                // Font8 fallback.
                if matches!(stat, 13 | 30) {
                    let v = if stat == 13 {
                        view.stat(13) as u32
                    } else {
                        view.next_level().unwrap_or(view.stat(30) as u32)
                    };
                    let s16 = utf16(&super::char_details::group_digits(v, 128));
                    centered(s16, y, font, 0, out);
                    return;
                }
                let resist = RESISTS.iter().find(|&&(st, _)| st == stat);
                let (shown, color) = match resist {
                    Some(&(st, max)) => resist_value(view, st, max),
                    None => match plain_value(view, stat) {
                        Some(v) => v,
                        None => return,
                    },
                };
                let s16 = utf16(&shown.to_string());
                let Some(font) = value_font(r, stat, shown, font, view, &s16) else {
                    return;
                };
                centered(s16, y, font, color, out);
            }
            _ => {}
        }
    }

    /// Mouse down: sets the pressed field of the close button or of the add
    /// button under `at` (the add buttons only while stat points are left).
    /// Returns whether an add button was pressed: then the cursor press
    /// runs (the close button does not press it).
    pub fn press(&mut self, t: &PanelTables, s: &Screen, at: Point, statpts: i32) -> bool {
        if close_rect(t, s).is_some_and(|r| r.contains(at)) {
            self.close_pressed = true;
        }
        if let Some(i) = add_button_at(t, s, at, statpts) {
            self.stat_pressed[i] = true;
            return true;
        }
        false
    }

    /// Mouse up: the close pressed field is cleared.
    /// With no points left nothing else happens (no add-button flag is cleared,
    /// nothing is sent). Else the four add buttons are walked in table order;
    /// each pressed field is cleared as it is reached and the first button
    /// whose rectangle holds the release spends and the walk **stops**, so the
    /// pressed fields of the buttons after it stay set (not every pressed field is
    /// cleared first). No check that the press
    /// began on the button.
    pub fn release(
        &mut self,
        t: &PanelTables,
        s: &Screen,
        at: Point,
        shift_held: bool,
        statpts: i32,
    ) -> Vec<PanelOutput> {
        self.close_pressed = false;
        let mut outp = Vec::new();
        if close_rect(t, s).is_some_and(|r| r.contains(at)) {
            outp.push(PanelOutput::SetUi {
                ui: UI_CHARACTER,
                mode: 1,
                jump: false,
            });
        }
        if statpts != 0 {
            let hit = add_button_at(t, s, at, statpts);
            for (i, &stat) in ADD_BUTTON_STATS.iter().enumerate() {
                self.stat_pressed[i] = false;
                if hit == Some(i) {
                    let mut count = if shift_held { statpts } else { 1 };
                    while count > 0 {
                        let n = count.min(STAT_CHUNK);
                        outp.push(PanelOutput::Intent(add_stat_point(stat, n)));
                        count -= n;
                    }
                    break;
                }
            }
        }
        outp
    }
}

/// Font of a value: Font8 instead of the row's Font16 for stats
/// 6–11 and, for defense when its value is ≥ 1,000 or the language is 6,
/// if the value is ≥ 1,000 or its popup width is ≥ `x2 − x1`. `None`:
/// the choice needs the unspecified popup width (Open).
fn value_font(
    r: &LayoutRow,
    stat: u16,
    shown: i32,
    font: u16,
    view: &dyn CharacterView,
    s16: &[u16],
) -> Option<u16> {
    let tested = (6..=11).contains(&stat)
        || (stat == STAT_DEFENSE && (shown >= 1000 || view.language() == 6));
    if !tested {
        return Some(font);
    }
    if shown >= 1000 {
        return Some(FONT8);
    }
    // `x2 − x1` = row w − 1 (the span is [x1, x2]).
    let limit = r.w? - 1;
    let w = view.popup_width(s16)?;
    Some(if w >= limit { FONT8 } else { font })
}

/// A C→S 0x3A spending `n` (1–32) points on `stat`: `[0x3A][stat][n − 1]`
/// (proto fields `stat:u8@1`; `repeat:u8@2`).
pub fn add_stat_point(stat: u16, n: i32) -> ClientIntent {
    let repeat = (u16::try_from(n - 1).unwrap_or(0) & 0xFF) as u8;
    ClientIntent::from_message(&d2_proto::client::AddStatPoint {
        stat: (stat & 0xFF) as u8,
        repeat,
    })
}

/// The close rectangle (row `close` `hit`).
pub fn close_rect(t: &PanelTables, s: &Screen) -> Option<Rect> {
    t.item(PANEL, "close", RowKind::Hit)
        .find_map(|r| r.hit_rect(s))
}

/// The add button under `at` (strict `sx + x < mx < sx + x + 40`, `b − 22 < my
/// < b`; the `btn_statN` `hit` rows), only while stat points are left (the
/// rows' `statpts` condition).
fn add_button_at(t: &PanelTables, s: &Screen, at: Point, statpts: i32) -> Option<usize> {
    if statpts == 0 {
        return None;
    }
    t.rows(PANEL)
        .filter(|r| r.kind == RowKind::Hit)
        .find_map(|r| {
            let i = button_index(&r.item)?;
            r.hit_rect(s).filter(|h| h.contains(at)).map(|_| i)
        })
}

#[cfg(test)]
mod tests {
    use super::super::super::draw::UiDraw;
    use super::*;

    struct Fixed(i32);
    impl TextMeasure for Fixed {
        /// Every unit `self.0` pixels wide.
        fn width(&self, _font: u16, text: &[u16]) -> Option<i32> {
            Some(self.0 * text.len() as i32)
        }
    }

    struct Strings(Vec<(u16, Vec<u16>)>);
    impl StringLookup for Strings {
        fn get(&self, _key: &str) -> Option<&[u16]> {
            None
        }
        fn get_id(&self, id: u16) -> Option<&[u16]> {
            self.0
                .iter()
                .find(|(i, _)| *i == id)
                .map(|(_, s)| s.as_slice())
        }
    }

    #[derive(Default)]
    struct View {
        stats: Vec<(u16, i32, i32)>,
        alive: bool,
        penalty: i32,
        effect: ResistEffect,
        popup: Option<i32>,
        language: u8,
    }
    impl CharacterView for View {
        fn stat(&self, id: u16) -> i32 {
            self.stats.iter().find(|s| s.0 == id).map_or(0, |s| s.1)
        }
        fn base(&self, id: u16) -> i32 {
            self.stats.iter().find(|s| s.0 == id).map_or(0, |s| s.2)
        }
        fn alive(&self) -> bool {
            self.alive
        }
        fn language(&self) -> u8 {
            self.language
        }
        fn resist_penalty(&self) -> i32 {
            self.penalty
        }
        fn resist_effect(&self, _id: u16) -> ResistEffect {
            self.effect
        }
        fn popup_width(&self, _t: &[u16]) -> Option<i32> {
            self.popup
        }
    }

    fn tables() -> PanelTables {
        PanelTables::load().expect("tables")
    }

    fn env(screen: Screen) -> PanelEnv {
        PanelEnv {
            screen,
            open_mode: 2,
            exp: true,
        }
    }

    fn draws(p: &CharacterPanel, screen: Screen, v: &View, s: &Strings) -> Vec<UiDraw> {
        let t = tables();
        let mut out: Vec<UiDraw> = Vec::new();
        p.draw(&t, &env(screen), v, &Fixed(5), s, &mut out);
        out
    }

    fn images(d: &[UiDraw]) -> Vec<(u32, u32, i32, i32)> {
        d.iter()
            .filter_map(|d| match d {
                UiDraw::Image(i) => Some((i.image.file, i.image.frame, i.at.x, i.at.y)),
                _ => None,
            })
            .collect()
    }

    fn texts(d: &[UiDraw]) -> Vec<(String, i32, i32, u16, u16)> {
        d.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some((
                    String::from_utf16_lossy(&t.text),
                    t.at.x,
                    t.at.y,
                    t.style.font,
                    t.style.color,
                )),
                _ => None,
            })
            .collect()
    }

    fn bytes(o: &[PanelOutput]) -> Vec<Vec<u8>> {
        o.iter()
            .filter_map(|o| match o {
                PanelOutput::Intent(i) => Some(i.0.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn quads_800() {
        let t = tables();
        let d = draws(
            &CharacterPanel::default(),
            Screen::R800,
            &View::default(),
            &Strings(vec![]),
        );
        let f = t.files.id("panel\\invchar6").unwrap();
        let q: Vec<_> = images(&d).into_iter().filter(|i| i.0 == f).collect();
        assert_eq!(
            q,
            vec![
                (f, 0, 80, 316),
                (f, 1, 336, 316),
                (f, 2, 80, 492),
                (f, 3, 336, 492)
            ]
        );
    }

    #[test]
    fn classic_uses_invchar() {
        let t = tables();
        let mut out: Vec<UiDraw> = Vec::new();
        let e = PanelEnv {
            exp: false,
            ..env(Screen::R640)
        };
        CharacterPanel::default().draw(
            &t,
            &e,
            &View::default(),
            &Fixed(5),
            &Strings(vec![]),
            &mut out,
        );
        let f = t.files.id("panel\\invchar").unwrap();
        assert_eq!(images(&out).iter().filter(|i| i.0 == f).count(), 4);
    }

    #[test]
    fn close_button_frames() {
        let t = tables();
        let btn = t.files.id("panel\\buysellbtn").unwrap();
        let mut p = CharacterPanel::default();
        let d = draws(&p, Screen::R640, &View::default(), &Strings(vec![]));
        assert!(images(&d).contains(&(btn, 10, 128, 420)));
        p.close_pressed = true;
        let d = draws(&p, Screen::R640, &View::default(), &Strings(vec![]));
        assert!(images(&d).contains(&(btn, 11, 128, 420)));
        assert!(!images(&d).iter().any(|i| i.0 == btn && i.1 == 10));
    }

    #[test]
    fn statpts_block() {
        let t = tables();
        let v = View {
            stats: vec![(4, 5, 5)],
            ..View::default()
        };
        let s = Strings(vec![(4075, utf16("Stat")), (4076, utf16("Points"))]);
        let mut p = CharacterPanel::default();
        p.stat_pressed[2] = true; // vitality
        let d = draws(&p, Screen::R640, &v, &s);
        let sp = t.files.id("panel\\skillpoints").unwrap();
        let sock = t.files.id("panel\\levelsocket").unwrap();
        let lvl = t.files.id("panel\\level").unwrap();
        let im = images(&d);
        assert!(im.contains(&(sp, 0, 3, 364)));
        // Buttons in table order str, dex, vit, energy; b = H + sy − 480 + y.
        let btns: Vec<_> = im
            .iter()
            .filter(|i| i.0 == sock || i.0 == lvl)
            .copied()
            .collect();
        assert_eq!(
            btns,
            vec![
                (sock, 0, 122, 110),
                (lvl, 0, 125, 106),
                (sock, 0, 122, 172),
                (lvl, 0, 125, 168),
                (sock, 0, 122, 258),
                (lvl, 1, 125, 254),
                (sock, 0, 122, 320),
                (lvl, 0, 125, 316),
            ]
        );
        let tx = texts(&d);
        // "Stat" 20 wide in [11, 88] (78): 11 + 29; "Points" 30: 11 + 24.
        assert!(tx.contains(&("Stat".into(), 40, 355, 6, 1)));
        assert!(tx.contains(&("Points".into(), 35, 363, 6, 1)));
        // "5" 5 wide in [92, 127] (36): 92 + 15.
        assert!(tx.contains(&("5".into(), 107, 360, 1, 0)));
    }

    #[test]
    fn no_statpts_no_block() {
        let t = tables();
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &View::default(),
            &Strings(vec![]),
        );
        let sp = t.files.id("panel\\skillpoints").unwrap();
        let sock = t.files.id("panel\\levelsocket").unwrap();
        assert!(!images(&d).iter().any(|i| i.0 == sp || i.0 == sock));
    }

    #[test]
    fn the_base_statpts_gates_the_points_block() {
        let t = tables();
        let sp = t.files.id("panel\\skillpoints").unwrap();
        let p = CharacterPanel::default();
        // Full value 3, base 0: no box.
        let v = View {
            stats: vec![(4, 3, 0)],
            ..View::default()
        };
        assert!(!images(&draws(&p, Screen::R640, &v, &Strings(vec![])))
            .iter()
            .any(|i| i.0 == sp));
        // Base 2, full value 3: the box, and the number is the full value.
        let v = View {
            stats: vec![(4, 3, 2)],
            ..View::default()
        };
        let d = draws(&p, Screen::R640, &v, &Strings(vec![]));
        assert!(images(&d).contains(&(sp, 0, 3, 364)));
        assert!(texts(&d).iter().any(|x| x.0 == "3" && x.2 == 360));
    }

    #[test]
    fn split_lf_label_fire_resistance() {
        let s = Strings(vec![(4071, utf16("Fire\nResistance"))]);
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &View::default(),
            &s,
        );
        let tx = texts(&d);
        // [190, 268]: span 79. "Fire" 20 → 190 + 29; "Resistance" 50 → 190 + 14.
        assert!(tx.contains(&("Fire".into(), 219, 342, 6, 0)));
        assert!(tx.contains(&("Resistance".into(), 204, 350, 6, 0)));
    }

    #[test]
    fn trailing_lf_does_not_split() {
        let s = Strings(vec![(4057, utf16("Name\n"))]);
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &View::default(),
            &s,
        );
        let tx = texts(&d);
        // [11, 52]: span 42; width 25 → 11 + 8; drawn whole at y 44.
        assert!(tx.contains(&("Name\n".into(), 19, 44, 6, 0)));
    }

    #[test]
    fn values_shift_cmp_and_min_life() {
        let v = View {
            stats: vec![
                (0, 30, 25),     // strength above base: blue
                (2, 20, 25),     // dexterity below base: red
                (6, 0x80, 0x80), // life 0.5 → 0, shown 1 alive
                (7, 0x6400, 0x6400),
                (8, 0x1400, 0x1400),
                (31, 12, 12),
                (12, 7, 7),
            ],
            alive: true,
            popup: Some(10),
            ..View::default()
        };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        let tx = texts(&d);
        // strength [77, 112] span 36, "30" 10 wide → 77 + 13, y 99.
        assert!(tx.contains(&("30".into(), 90, 99, 1, 3)));
        assert!(tx.contains(&("20".into(), 90, 161, 1, 1)));
        // life [273, 308], y 270: "1", color 0 (not a cmp stat).
        assert!(tx.contains(&("1".into(), 288, 270, 1, 0)));
        // max life 0x64 = 100 at [232, 267], y 270, cmp equal → 0.
        assert!(tx.contains(&("100".into(), 242, 270, 1, 0)));
        // mana 20 at [273, 308], y 308.
        assert!(tx.contains(&("20".into(), 286, 308, 1, 0)));
        // level (12): compare color, `%ld`, centered in [13, 53] at y 59.
        assert!(tx.contains(&("7".into(), 31, 59, 1, 0)));
    }

    // The level (stat 12) takes the compare color: blue above its base,
    // red below.
    #[test]
    fn level_takes_the_compare_color() {
        let v = |total| View {
            stats: vec![(12, total, 7)],
            alive: true,
            ..View::default()
        };
        assert_eq!(plain_value(&v(8), 12), Some((8, 3)));
        assert_eq!(plain_value(&v(6), 12), Some((6, 1)));
        assert_eq!(plain_value(&v(7), 12), Some((7, 0)));
    }

    #[test]
    fn dead_life_not_raised() {
        let v = View {
            stats: vec![(6, 0x80, 0x80)],
            alive: false,
            popup: Some(0),
            ..View::default()
        };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        assert!(texts(&d).contains(&("0".into(), 288, 270, 1, 0)));
    }

    #[test]
    fn font8_fallback() {
        let v = View {
            stats: vec![
                (7, 1500 << 8, 1500 << 8),
                (6, 50 << 8, 50 << 8),
                (31, 1200, 1000),
            ],
            popup: Some(40),
            ..View::default()
        };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        let tx = texts(&d);
        // ≥ 1000 → Font8.
        assert!(tx.iter().any(|t| t.0 == "1500" && t.3 == FONT8));
        // popup width 40 ≥ 35 → Font8.
        assert!(tx.iter().any(|t| t.0 == "50" && t.3 == FONT8));
        // defense ≥ 1000 → Font8, blue.
        assert!(tx.iter().any(|t| t.0 == "1200" && t.3 == FONT8 && t.4 == 3));
        let v = View {
            popup: Some(34),
            ..v
        };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        assert!(texts(&d).iter().any(|t| t.0 == "50" && t.3 == 1));
        // Popup width unknown: life value pending, not drawn.
        let v = View { popup: None, ..v };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        assert!(!texts(&d).iter().any(|t| t.0 == "50"));
    }

    #[test]
    fn defense_popup_test_only_for_language_6() {
        // Defense 500 with a popup width ≥ the span: Font8 only in
        // language 6; any other language keeps Font16 without the test.
        let v = View {
            stats: vec![(31, 500, 500)],
            popup: Some(40),
            language: 6,
            ..View::default()
        };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        assert!(texts(&d).iter().any(|t| t.0 == "500" && t.3 == FONT8));
        let v = View { language: 0, ..v };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        assert!(texts(&d).iter().any(|t| t.0 == "500" && t.3 == 1));
        // Language 6, popup narrower than the span: Font16.
        let v = View {
            language: 6,
            popup: Some(10),
            ..v
        };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        assert!(texts(&d).iter().any(|t| t.0 == "500" && t.3 == 1));
    }

    #[test]
    fn resist_penalty_clamp_and_effect() {
        let mut v = View {
            stats: vec![
                (39, 90, 90),
                (40, 10, 10),
                (43, -200, -200),
                (41, 120, 120),
                (42, 30, 30),
            ],
            penalty: 50,
            effect: ResistEffect::Raised,
            ..View::default()
        };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        let tx = texts(&d);
        // fire: 90 − 50 = 40 ≤ cap 85; y 348.
        assert!(tx.iter().any(|t| t.0 == "40" && t.2 == 348 && t.4 == 3));
        // cold: −250 → −100; y 372.
        assert!(tx.iter().any(|t| t.0 == "-100" && t.2 == 372));
        // lightning: 70, cap min(105, 95) = 95 → 70; y 396.
        assert!(tx.iter().any(|t| t.0 == "70" && t.2 == 396));
        // poison 0 − 50 = −50, cap 75; y 420.
        assert!(tx.iter().any(|t| t.0 == "-50" && t.2 == 420));
        v.effect = ResistEffect::Lowered;
        v.penalty = 0;
        v.stats[0] = (39, 100, 100);
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        // At the cap, colour 4 unless it is 3 (over the lowering 1).
        assert!(texts(&d)
            .iter()
            .any(|t| t.0 == "85" && t.2 == 348 && t.4 == 4));
    }

    #[test]
    fn resist_colour_order() {
        let colour = |effect, value: i32, max: i32| {
            let v = View {
                stats: vec![(39, value, value), (40, max, max)],
                effect,
                ..View::default()
            };
            let d = draws(
                &CharacterPanel::default(),
                Screen::R640,
                &v,
                &Strings(vec![]),
            );
            let tx = texts(&d);
            let t = tx.iter().find(|t| t.2 == 348).expect("fire value drawn");
            (t.0.clone(), t.4)
        };
        // No effect: 0 below the cap, 4 at it, 1 below zero.
        assert_eq!(colour(ResistEffect::None, 30, 0), ("30".into(), 0));
        assert_eq!(colour(ResistEffect::None, 80, 0), ("75".into(), 4));
        assert_eq!(colour(ResistEffect::None, -5, 0), ("-5".into(), 1));
        // Raised: 3, kept at the cap, replaced by 1 below zero.
        assert_eq!(colour(ResistEffect::Raised, 30, 0), ("30".into(), 3));
        assert_eq!(colour(ResistEffect::Raised, 80, 0), ("75".into(), 3));
        assert_eq!(colour(ResistEffect::Raised, -5, 0), ("-5".into(), 1));
        // Lowered: 1, replaced by 4 at the cap.
        assert_eq!(colour(ResistEffect::Lowered, 30, 0), ("30".into(), 1));
        assert_eq!(colour(ResistEffect::Lowered, 99, 5), ("80".into(), 4));
    }

    #[test]
    fn shift_click_vitality_70() {
        let t = tables();
        let s = Screen::R640;
        let mut p = CharacterPanel::default();
        // Vitality button: b = 253, x in (117, 157), y in (231, 253).
        let at = Point::new(130, 245);
        p.press(&t, &s, at, 70);
        assert_eq!(p.stat_pressed, [false, false, true, false]);
        let o = p.release(&t, &s, at, true, 70);
        assert_eq!(
            bytes(&o),
            vec![
                vec![0x3A, 0x03, 0x1F],
                vec![0x3A, 0x03, 0x1F],
                vec![0x3A, 0x03, 0x05]
            ]
        );
        assert_eq!(p, CharacterPanel::default());
    }

    #[test]
    fn click_strength() {
        let t = tables();
        let s = Screen::R800;
        let mut p = CharacterPanel::default();
        // Strength: b = 600 − 60 − 480 + 105 = 165; x in (197, 237).
        let o = p.release(&t, &s, Point::new(198, 164), false, 3);
        assert_eq!(bytes(&o), vec![vec![0x3A, 0x00, 0x00]]);
    }

    #[test]
    fn add_button_edges_strict() {
        let t = tables();
        let s = Screen::R640;
        let mut p = CharacterPanel::default();
        // Strength: x in (117, 157), y in (83, 105).
        for at in [(117, 100), (157, 100), (130, 83), (130, 105)] {
            assert!(p
                .release(&t, &s, Point::new(at.0, at.1), false, 5)
                .is_empty());
        }
        for at in [(118, 84), (156, 104)] {
            assert_eq!(p.release(&t, &s, Point::new(at.0, at.1), false, 5).len(), 1);
        }
        // No points: no hit.
        assert!(p.release(&t, &s, Point::new(130, 100), false, 0).is_empty());
    }

    #[test]
    fn close_release_without_press() {
        let t = tables();
        let s = Screen::R640;
        let mut p = CharacterPanel {
            close_pressed: false,
            stat_pressed: [true, false, false, true],
        };
        // Close rect x [128, 160], y [388, 420]. With points left the walk
        // reaches all four buttons (no hit) and clears them.
        let o = p.release(&t, &s, Point::new(160, 388), false, 1);
        assert_eq!(
            o,
            vec![PanelOutput::SetUi {
                ui: 2,
                mode: 1,
                jump: false
            }]
        );
        assert_eq!(p, CharacterPanel::default());
        p.press(&t, &s, Point::new(128, 420), 0);
        assert!(p.close_pressed);
        assert!(p.release(&t, &s, Point::new(161, 420), false, 0).is_empty());
        assert!(!p.close_pressed);
    }

    #[test]
    fn click_area() {
        let a = area(&Screen::R640);
        assert_eq!(a, Rect::new(0, 0, 320, 432));
        let a = area(&Screen::R800);
        assert!(a.contains(Point::new(80, -60)));
        assert!(a.contains(Point::new(399, 491)));
        assert!(!a.contains(Point::new(400, 491)));
        assert!(!a.contains(Point::new(399, 492)));
        assert!(!a.contains(Point::new(79, 0)));
    }

    #[test]
    fn experience_and_next_level_grouped() {
        let v = View {
            stats: vec![(13, 1_234_567, 0), (30, 999, 0)],
            popup: Some(0),
            ..View::default()
        };
        let d = draws(
            &CharacterPanel::default(),
            Screen::R640,
            &v,
            &Strings(vec![]),
        );
        let tx = texts(&d);
        // experience: [67, 180], 9 units of 5 → 67 + 34; Font16, color 0
        assert!(tx.contains(&("1,234,567".into(), 101, 59, 1, 0)));
        // next level (stat 30 at the maximum level): [195, 308]
        assert!(tx.contains(&("999".into(), 244, 59, 1, 0)));
    }
}
