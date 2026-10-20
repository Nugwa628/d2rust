//! Waypoint menu (ui 0x14; left slot): background, act tabs, close button, the
//! rows of the current tab, the title and the C→S 0x49 messages.
//!
//! Row and tab data (level names, which waypoints are known, which acts
//! are reachable) come from a [`WaypointView`]; the menu decides nothing:
//! travel and close leave as C→S 0x49.
//!
//! Open points:
//! - Tab and row click rectangles and the tab switch: not
//!   implemented. Callers drive [`WaypointPanel::choose_row`],
//!   [`WaypointPanel::close`], [`WaypointPanel::tab`] and
//!   [`WaypointPanel::selected`] directly. Whether a row must be known to be
//!   chosen is not settled; any present row sends.
//! - The close button's hover effect is a filled rectangle and a
//!   tool tip; [`UiDraw`] has neither, so it is exposed as the query
//!   [`close_hover`] for the root to draw. The rectangle's extent is not
//!   settled; the query returns the hover area.
//! - The self-close mode passed to `SetUIState(0x14, …)` is not
//!   known; `off` is used. Its jump is 0: no call site in the draw
//!   function passes jump = 1. When the travel latch
//!   is cleared is not settled; [`WaypointPanel::open`]
//!   clears it.
//! - Whether a close through [`WaypointPanel::close`] also sends
//!   the close hook's 0x49 ([`WaypointPanel::close_hook`]) a second time, and
//!   the order of the 0x49 and the `SetUIState` call, are not settled.
//! - Row text color: "5 unknown, 0 known, 3 if hovered-selected or current
//!   level" is applied in that order (3 overrides); an unknown selected row is
//!   untested. The current level's frame-0 draw comes after its row
//!   icon and does not depend on the used byte; the current level's used byte
//!   is 0, so in play it is the row's only icon.
//! - Which tab and row are current when the menu opens is not settled.

use d2_proto::client::TakeOrCloseWp;

use super::{cel, emit_static_draws, text, PanelEnv, PanelOutput, PanelTables, TextMeasure};
use crate::ui::draw::UiDrawSink;
use crate::ui::geom::{Point, Rect};
use crate::ui::layout::{Cond, FrameSpec, LayoutRow, PanelKey, RowKind, Screen};
use crate::ui::messages::msg_u32s;
use crate::ui::panel::{ClientIntent, StringLookup};

/// UI state id of the waypoint menu.
pub const UI_WAYPOINT: u8 = 0x14;
/// Font16 (`text-fonts.tsv` id 1): rows and title.
pub const FONT16: u16 = 1;
/// `waypointsheader`.
pub const STR_HEADER: u16 = 3990;
/// `nowaypoints`.
pub const STR_NO_WAYPOINTS: u16 = 3991;
/// `strUiMenu1` "Cancel", the close tool tip.
pub const STR_CANCEL: u16 = 4130;
/// Rows per tab.
pub const MAX_ROWS: usize = 9;
/// Waypoint icons file.
pub const ICONS_FILE: &str = "menu\\waygateicons";

/// One waypoint row of the current tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WpRow {
    /// The level id sent by 0x49.
    pub level: u16,
    /// The player knows this waypoint.
    pub known: bool,
    /// This is the level the player stands in.
    pub current: bool,
    /// Level name, UTF-16.
    pub name: Vec<u16>,
}

/// The game facts the waypoint menu draws.
pub trait WaypointView {
    /// The rows of tab `tab` (at most [`MAX_ROWS`] are used).
    fn rows(&self, tab: u8) -> &[WpRow];
    /// The act of tab 1–4 is reachable (its quest record 7, 15, 23 or
    /// 26 is set).
    fn tab_reachable(&self, tab: u8) -> bool;
    /// Any other waypoint is known.
    fn any_other_known(&self) -> bool;
}

/// The close button's hover result: the root draws the filled
/// rectangle (color 0, mode 2) and queues the tool tip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseHover {
    /// Hover area: x in [`sx + 273`, `sx + 308`], y in [`387 − sy`,
    /// `420 − sy`].
    pub area: Rect,
    /// Tool-tip string id ("Cancel").
    pub tooltip: u16,
    /// Tool tip centered on this x.
    pub tooltip_cx: i32,
    /// Tool tip y.
    pub tooltip_y: i32,
}

/// The close hover at `p`, if the mouse is in the area.
pub fn close_hover(s: &Screen, p: Point) -> Option<CloseHover> {
    let (sx, sy) = (s.sx(), (s.sy() - s.panel_dy()));
    let area = Rect::new(sx + 273, 387 - sy, 36, 34);
    area.contains(p).then_some(CloseHover {
        area,
        tooltip: STR_CANCEL,
        tooltip_cx: sx + 291,
        tooltip_y: 385 - sy,
    })
}

/// Tab frame: the current tab draws `2t`; another draws `2t + 1`
/// if it is tab 0 or its act is reachable; otherwise nothing.
pub fn tab_frame(current: u8, tab: u8, reachable: bool) -> Option<u32> {
    let t = u32::from(tab);
    if tab == current {
        Some(2 * t)
    } else if tab == 0 || reachable {
        Some(2 * t + 1)
    } else {
        None
    }
}

/// Waypoint menu state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WaypointPanel {
    /// Current tab.
    pub tab: u8,
    /// Selected row.
    pub selected: Option<u8>,
    /// Close button pressed.
    pub close_pressed: bool,
    /// The waypoint object's GUID.
    pub guid: u32,
    /// Self-close message sent.
    close_sent: bool,
    /// No player or no room for it: the latched level-0
    /// send needs both.
    pub no_player_room: bool,
}

impl WaypointPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// S→C 0x63: store the GUID and open with jump = 1.
    pub fn open(&mut self, guid: u32) -> Vec<PanelOutput> {
        self.guid = guid;
        self.close_sent = false;
        vec![PanelOutput::SetUi {
            ui: UI_WAYPOINT,
            mode: 0,
            jump: true,
        }]
    }

    fn msg(&self, level: u16) -> PanelOutput {
        PanelOutput::Intent(ClientIntent::from_message(&TakeOrCloseWp {
            wp: self.guid,
            level,
        }))
    }

    /// Choosing row `row` of the current tab: C→S 0x49 `[GUID][level]`. Nothing
    /// for a row that is not there. The row choice sets the close latch too, so
    /// the close that follows sends no level-0 message.
    pub fn choose_row(&mut self, view: &dyn WaypointView, row: usize) -> Vec<PanelOutput> {
        let rows = view.rows(self.tab);
        match rows.get(row).filter(|_| row < MAX_ROWS) {
            Some(r) => {
                let m = self.msg(r.level);
                self.close_sent = true;
                vec![m]
            }
            None => Vec::new(),
        }
    }

    /// The latched level-0 send: it needs the latch
    /// clear and a player with its room, and sets the
    /// latch.
    fn latched_close_msg(&mut self) -> Vec<PanelOutput> {
        if self.close_sent || self.no_player_room {
            return Vec::new();
        }
        self.close_sent = true;
        vec![self.msg(0)]
    }

    /// The close paths: the
    /// latched 0x49 with level 0, then `SetUIState(0x14, off, jump = 1)`.
    pub fn close(&mut self) -> Vec<PanelOutput> {
        self.close_pressed = false;
        let mut out = self.latched_close_msg();
        out.push(PanelOutput::SetUi {
            ui: UI_WAYPOINT,
            mode: 1,
            jump: true,
        });
        out
    }

    /// The close hook: the latched 0x49 with
    /// level 0.
    pub fn close_hook(&mut self) -> Vec<PanelOutput> {
        self.close_pressed = false;
        self.latched_close_msg()
    }

    /// The draw-time check: with the player gone (or in a state
    /// Rejects) the menu closes itself and sends 0x49 level 0
    /// once (latch).
    pub fn self_close(&mut self, player_ok: bool) -> Vec<PanelOutput> {
        self.self_close_checked(true, player_ok).outputs
    }

    /// Start of the draw: with no
    /// player or its mode 0x11 (`player_ok` false), `SetUIState(0x14, off,
    /// 0)` (its close hook sends the latched level 0, which needs a player
    /// and its room: `has_player_room`), then the latched send (nothing
    /// left). The draw **continues** for that frame.
    pub fn self_close_checked(&mut self, has_player_room: bool, player_ok: bool) -> SelfClose {
        if player_ok {
            return SelfClose {
                outputs: Vec::new(),
                draw_continues: false,
            };
        }
        self.no_player_room = !has_player_room;
        let mut outputs = vec![PanelOutput::SetUi {
            ui: UI_WAYPOINT,
            mode: 1,
            jump: false,
        }];
        outputs.extend(self.latched_close_msg());
        SelfClose {
            outputs,
            draw_continues: true,
        }
    }

    /// Draws the menu: background, tabs, close button, rows,
    /// title. Text rows whose string or width is missing are left out.
    pub fn draw(
        &self,
        t: &PanelTables,
        env: &PanelEnv,
        view: &dyn WaypointView,
        strings: &dyn StringLookup,
        measure: &dyn TextMeasure,
        out: &mut dyn UiDrawSink,
    ) {
        let s = env.screen;
        let key = PanelKey::Ui(UI_WAYPOINT);
        let cond = env.cond(self.close_pressed, &|c: Cond| matches!(c, Cond::TabShown));
        emit_static_draws(t, key, &cond, None, &|r| r.item.starts_with("art"), out);
        // Tabs.
        for r in t.rows(key) {
            if r.kind != RowKind::Draw || !r.item.starts_with("tab") || !r.applies(&cond) {
                continue;
            }
            let Some(n) = r.item[3..].parse::<u8>().ok() else {
                continue;
            };
            let reachable = n != 0 && view.tab_reachable(n);
            if let (Some(f), Some(file)) =
                (tab_frame(self.tab, n, reachable), t.files.row_file(r, None))
            {
                out.push(cel(file, f, r.x.eval(&s), r.y.eval(&s)));
            }
        }
        // Close button, frame 10 + pressed.
        emit_static_draws(t, key, &cond, None, &|r| r.item == "close", out);
        // Rows.
        let icons = t.files.id(ICONS_FILE);
        for (i, row) in view.rows(self.tab).iter().take(MAX_ROWS).enumerate() {
            let name = format!("row{i}");
            let sel = self.selected == Some(i as u8);
            // Known row (used byte set): frame `sel` on the current level, else
            // `3 + sel`. The current level's used byte is 0, so its one
            // icon is the frame-0 draw that follows, made whatever the used byte.
            if let (Some(file), Some(d)) = (icons, find(t, key, &name, RowKind::Draw)) {
                let (x, y) = (d.x.eval(&s), d.y.eval(&s));
                if row.known {
                    let f = if row.current {
                        u32::from(sel)
                    } else {
                        3 + u32::from(sel)
                    };
                    out.push(cel(file, f, x, y));
                }
                if row.current {
                    out.push(cel(file, 0, x, y));
                }
            }
            if let Some(tr) = find(t, key, &name, RowKind::Text) {
                let color = if (sel && !self.close_pressed) || row.current {
                    3
                } else if row.known {
                    0
                } else {
                    5
                };
                let font = tr.font.unwrap_or(FONT16);
                out.push(text(
                    row.name.clone(),
                    tr.x.eval(&s),
                    tr.y.eval(&s),
                    font,
                    color,
                ));
            }
        }
        // Title.
        if let Some(tr) = find(t, key, "title", RowKind::Text) {
            let id = match tr.frame {
                FrameSpec::Alt(a, b) => {
                    if view.any_other_known() {
                        a
                    } else {
                        b
                    }
                }
                _ => return,
            };
            let font = tr.font.unwrap_or(FONT16);
            let Some(str16) = u16::try_from(id).ok().and_then(|id| strings.get_id(id)) else {
                return;
            };
            let Some(w) = measure.width(font, str16) else {
                return;
            };
            let color = match tr.color {
                crate::ui::layout::ColorSpec::Index(k) => k,
                _ => 0,
            };
            out.push(text(
                str16.to_vec(),
                tr.x.eval(&s) - w / 2,
                tr.y.eval(&s),
                font,
                color,
            ));
        }
    }
}

// ---------------------------------------------------------------------
// The waypoint menu input (ui 0x14).
// ---------------------------------------------------------------------

/// A `SelfClose` result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelfClose {
    pub outputs: Vec<PanelOutput>,
    /// The draw then **continues** for that frame (reproduced).
    pub draw_continues: bool,
}

/// What the menu input produces, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WpEffect {
    Out(PanelOutput),
    Sound(u32),
    /// The row rebuild.
    RebuildRows,
}

/// Sounds of the tab click, button press and travel.
pub const SOUND_TAB: u32 = 6;
pub const SOUND_PRESS: u32 = 4;
pub const SOUND_TRAVEL: u32 = 0x8B7;

/// Row hit table: `hy` of the 9 rows (x 17).
pub const ROW_HY: [i32; 9] = [60, 96, 132, 168, 205, 241, 277, 313, 349];
/// Tab count.
pub const TAB_COUNT: u8 = 5;
/// Quest record whose bit 0 the tab setter tests, by tab 1–4 (the draw tests 26
/// for tab 4: reproduce).
pub const TAB_QUEST_RECORD: [u32; 5] = [0, 7, 15, 23, 28];
/// The records the tab **draw** tests: tab 4 on
/// record 26, the setter on record 28.
pub const TAB_DRAW_QUEST_RECORD: [u32; 5] = [0, 7, 15, 23, 26];

/// The rows of the open menu: each row's level and used byte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WpRows {
    pub used: [bool; MAX_ROWS],
    pub level: [u32; MAX_ROWS],
}

/// The screen facts of the handlers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WpEnv {
    pub w: i32,
    pub h: i32,
    pub sx: i32,
    pub sy: i32,
    /// One past the panel side's last column (`Screen::left_end`, W / 2 at
    /// 4:3): a press right of it closes the panel.
    pub left_end: i32,
    /// One past the frame's last column (`Screen::frame_right`, W at 4:3).
    pub right: i32,
    /// The expansion is installed and the game is an expansion game.
    pub expansion_game: bool,
}

impl WpEnv {
    pub fn new(s: &Screen, expansion_game: bool) -> Self {
        Self {
            w: s.w,
            h: s.h,
            sx: s.sx(),
            sy: (s.sy() - s.panel_dy()),
            left_end: s.left_end(),
            right: s.frame_right(),
            expansion_game,
        }
    }
}

/// Ui 0x14 and 0x15 (mini panel) both open and the
/// mouse strictly inside x (W/2 + a, W/2 + a + 20 n), y (H − 76, H − 50),
/// with a, n = 56, 7 in single player, else 35, 8.
pub fn mini_panel_blocks(w: i32, h: i32, both_open: bool, bc978: bool, x: i32, y: i32) -> bool {
    if !both_open {
        return false;
    }
    let (a, n) = if bc978 { (56, 7) } else { (35, 8) };
    let l = w / 2 + a;
    l < x && x < l + 20 * n && h - 76 < y && y < h - 50
}

/// The tab hit: none unless y' ≤ 30 and x' ≤ 320;
/// with the expansion tab 0–3 for x' < 64, 128, 192, 256, else 4;
/// otherwise 0, 1, 2 for x' < 80, 160, 240, else 3 (no left or top
/// bound).
pub fn tab_hit(xp: i32, yp: i32, expansion_game: bool) -> Option<u8> {
    if yp > 30 || xp > 320 {
        return None;
    }
    let (step, last) = if expansion_game { (64, 4) } else { (80, 3) };
    Some(
        (0..last)
            .find(|&t| xp < step * (i32::from(t) + 1))
            .unwrap_or(last),
    )
}

/// The row hit: the first used row r with `17 < x' <
/// 297` and `hy < y' < hy + 30` (strict).
pub fn row_hit(xp: i32, yp: i32, rows: &WpRows) -> Option<usize> {
    (0..MAX_ROWS)
        .find(|&r| rows.used[r] && 17 < xp && xp < 297 && ROW_HY[r] < yp && yp < ROW_HY[r] + 30)
}

/// The tab setter: `t` ≥ 5 → 0; then, walking down from `t`: tab 4 needs quest
/// record 28 bit 0, tab 3 record 23, tab 2 record 15, tab 1 record 7; a failed
/// check sets `t`:= that tab − 1.
pub fn set_tab(t: u8, quest_bit0: &dyn Fn(u32) -> bool) -> u8 {
    if t >= TAB_COUNT {
        return 0;
    }
    let mut t = t;
    while t > 0 {
        if quest_bit0(TAB_QUEST_RECORD[usize::from(t)]) {
            return t;
        }
        t -= 1;
    }
    0
}

/// The close button's press rectangle in panel coordinates.
fn in_close_rect(xp: i32, yp: i32) -> bool {
    (273..=308).contains(&xp) && (387..=420).contains(&yp)
}

impl WaypointPanel {
    /// Mouse down. Returns the effects and whether
    /// the event is consumed. `blocked` is [`mini_panel_blocks`].
    pub fn mouse_down(
        &mut self,
        env: &WpEnv,
        x: i32,
        y: i32,
        rows: &WpRows,
        blocked: bool,
        quest_bit0: &dyn Fn(u32) -> bool,
    ) -> (Vec<WpEffect>, bool) {
        if blocked {
            return (Vec::new(), false);
        }
        let consumed = x <= env.right && y <= env.h - 49;
        if x > env.left_end || y > env.h - 49 {
            // The latched close, then SetUIState(0x14, off, jump 1).
            let out = self.close().into_iter().map(WpEffect::Out).collect();
            return (out, consumed);
        }
        let (xp, yp) = (x - env.sx, y + env.sy);
        let mut e = Vec::new();
        if let Some(t) = tab_hit(xp, yp, env.expansion_game) {
            if t != self.tab {
                e.push(WpEffect::Sound(SOUND_TAB));
            }
            self.tab = set_tab(t, quest_bit0);
            e.push(WpEffect::RebuildRows);
        }
        self.selected = None;
        if in_close_rect(xp, yp) {
            self.close_pressed = true;
            e.push(WpEffect::Sound(SOUND_PRESS));
        } else {
            self.selected = row_hit(xp, yp, rows).map(|r| r as u8);
            if self.selected.is_some() {
                e.push(WpEffect::Sound(SOUND_PRESS));
            }
        }
        (e, consumed)
    }

    /// Mouse up.
    pub fn mouse_up(
        &mut self,
        env: &WpEnv,
        x: i32,
        y: i32,
        rows: &WpRows,
        blocked: bool,
    ) -> (Vec<WpEffect>, bool) {
        if blocked {
            return (Vec::new(), false);
        }
        if x > env.right || y > env.h - 49 {
            self.close_pressed = false;
            self.selected = None;
            return (Vec::new(), false);
        }
        let (xp, yp) = (x - env.sx, y + env.sy);
        let mut e = Vec::new();
        if self.close_pressed {
            if in_close_rect(xp, yp) {
                e.push(WpEffect::Out(PanelOutput::SetUi {
                    ui: UI_WAYPOINT,
                    mode: 1,
                    jump: false,
                }));
                // The latched send.
                e.extend(self.latched_close_msg().into_iter().map(WpEffect::Out));
            }
        } else if let Some(r) = self.selected.map(usize::from) {
            if row_hit(xp, yp, rows) == Some(r) {
                // 0x49 [wp GUID][level of the row u32], latch:= 1.
                e.push(WpEffect::Out(PanelOutput::Intent(msg_u32s(
                    0x49,
                    &[self.guid, rows.level[r]],
                ))));
                self.close_sent = true;
                e.extend(self.latched_close_msg().into_iter().map(WpEffect::Out));
                e.push(WpEffect::Out(PanelOutput::SetUi {
                    ui: UI_WAYPOINT,
                    mode: 1,
                    jump: false,
                }));
                e.push(WpEffect::Sound(SOUND_TRAVEL));
            }
        }
        self.close_pressed = false;
        self.selected = None;
        (e, true)
    }

    /// Key close: any key message except `WM_CHAR`
    /// with Tab (0x102 / 9) and `WM_SYSKEYDOWN` with F4 (0x104 / 0x73):
    /// the latched send, `SetUIState(0x14, off, jump 1)`, consumed.
    pub fn key_close(&mut self, message: u32, wparam: u32) -> (Vec<PanelOutput>, bool) {
        if (message == 0x102 && wparam == 9) || (message == 0x104 && wparam == 0x73) {
            return (Vec::new(), false);
        }
        (self.close(), true)
    }
}

fn find<'a>(
    t: &'a PanelTables,
    key: PanelKey,
    item: &'a str,
    kind: RowKind,
) -> Option<&'a LayoutRow> {
    t.item(key, item, kind).next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::{ImageRef, ImageRequest, TextRequest, UiDraw};
    use crate::ui::panels::utf16;

    struct View {
        rows: Vec<WpRow>,
        reach: [bool; 5],
        other: bool,
    }

    impl WaypointView for View {
        fn rows(&self, _tab: u8) -> &[WpRow] {
            &self.rows
        }
        fn tab_reachable(&self, tab: u8) -> bool {
            self.reach[usize::from(tab)]
        }
        fn any_other_known(&self) -> bool {
            self.other
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

    /// Width 7 per code unit.
    struct Measure;

    impl TextMeasure for Measure {
        fn width(&self, _font: u16, text: &[u16]) -> Option<i32> {
            Some(7 * text.len() as i32)
        }
    }

    fn row(level: u16, known: bool, current: bool) -> WpRow {
        WpRow {
            level,
            known,
            current,
            name: utf16(&format!("L{level}")),
        }
    }

    fn env(screen: Screen, exp: bool) -> PanelEnv {
        PanelEnv {
            screen,
            open_mode: 2,
            exp,
        }
    }

    fn strings() -> Strings {
        Strings(vec![
            (3990, utf16("Waypoints")),
            (3991, utf16("No Waypoints")),
        ])
    }

    fn draw(p: &WaypointPanel, e: &PanelEnv, v: &View) -> (PanelTables, Vec<UiDraw>) {
        let t = PanelTables::load().unwrap();
        let mut out: Vec<UiDraw> = Vec::new();
        p.draw(&t, e, v, &strings(), &Measure, &mut out);
        (t, out)
    }

    fn images(d: &[UiDraw]) -> Vec<(u32, u32, i32, i32)> {
        d.iter()
            .filter_map(|d| match d {
                UiDraw::Image(ImageRequest {
                    image: ImageRef { file, frame },
                    at,
                    ..
                }) => Some((*file, *frame, at.x, at.y)),
                _ => None,
            })
            .collect()
    }

    fn texts(d: &[UiDraw]) -> Vec<&TextRequest> {
        d.iter()
            .filter_map(|d| match d {
                UiDraw::Text(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    fn view() -> View {
        View {
            rows: Vec::new(),
            reach: [true, true, false, false, false],
            other: true,
        }
    }

    // Background quads (left slot) and the tab frames per game type.
    #[test]
    fn background_and_tabs() {
        let p = WaypointPanel {
            tab: 1,
            ..Default::default()
        };
        // Expansion, 800 × 600.
        let (t, out) = draw(&p, &env(Screen::R800, true), &view());
        let im = images(&out);
        let bg = t.files.id("menu\\waygatebackground").unwrap();
        let tabs = t.files.id("menu\\expwaygatetabs").unwrap();
        assert_eq!(
            im[..4],
            [
                (bg, 0, 80, 316),
                (bg, 1, 336, 316),
                (bg, 2, 80, 492),
                (bg, 3, 336, 492)
            ]
        );
        // Tab 0 always (2t + 1 = 1), tab 1 current (2), tabs 2–4 unreachable.
        assert_eq!(im[4..6], [(tabs, 1, 85, 94), (tabs, 2, 147, 94)]);
        // Classic, 640 × 480, tab 0 current, tab 2 reachable.
        let mut v = view();
        v.reach = [false, false, true, false, false];
        let p0 = WaypointPanel::default();
        let (t, out) = draw(&p0, &env(Screen::R640, false), &v);
        let im = images(&out);
        let ctabs = t.files.id("menu\\waygatetabs").unwrap();
        assert_eq!(im[4..6], [(ctabs, 0, 3, 33), (ctabs, 5, 159, 33)]);
        assert!(im.iter().all(|i| i.0 != tabs));
        // Expansion fifth tab at sx + 253.
        let mut v = view();
        v.reach = [true; 5];
        let (_, out) = draw(&p0, &env(Screen::R640, true), &v);
        let im = images(&out);
        assert_eq!(
            im[4..9],
            [
                (tabs, 0, 5, 34),
                (tabs, 3, 67, 34),
                (tabs, 5, 129, 34),
                (tabs, 7, 191, 34),
                (tabs, 9, 253, 34)
            ]
        );
        assert_eq!(tab_frame(2, 2, false), Some(4));
        assert_eq!(tab_frame(2, 3, false), None);
        assert_eq!(tab_frame(2, 0, false), Some(1));
    }

    // Close button frame 10 + pressed at (sx + 273, 417 − sy); hover area.
    #[test]
    fn close_button_and_hover() {
        let mut p = WaypointPanel::default();
        let (t, out) = draw(&p, &env(Screen::R800, true), &view());
        let close = t.files.id("panel\\buysellbtn").unwrap();
        assert!(images(&out).contains(&(close, 10, 353, 477)));
        p.close_pressed = true;
        let (_, out) = draw(&p, &env(Screen::R800, true), &view());
        assert!(images(&out).contains(&(close, 11, 353, 477)));
        assert!(!images(&out).contains(&(close, 10, 353, 477)));
        let s = Screen::R800;
        let h = close_hover(&s, Point::new(353, 447)).unwrap();
        assert_eq!((h.tooltip, h.tooltip_cx, h.tooltip_y), (4130, 371, 445));
        assert!(close_hover(&s, Point::new(388, 480)).is_some());
        assert!(close_hover(&s, Point::new(352, 460)).is_none());
        assert!(close_hover(&s, Point::new(389, 460)).is_none());
        assert!(close_hover(&s, Point::new(360, 446)).is_none());
        assert!(close_hover(&s, Point::new(360, 481)).is_none());
        let s = Screen::R640;
        assert!(close_hover(&s, Point::new(273, 387)).is_some());
        assert!(close_hover(&s, Point::new(308, 420)).is_some());
    }

    // The pressed row's text is color 3 only while the close button is
    // not pressed; the current level stays 3 either way.
    #[test]
    fn pressed_row_text_is_plain_while_the_close_button_is_pressed() {
        let mut v = view();
        v.rows = vec![row(1, true, false), row(3, true, true)];
        let p = WaypointPanel {
            selected: Some(0),
            close_pressed: true,
            ..Default::default()
        };
        let (_, out) = draw(&p, &env(Screen::R640, true), &v);
        let colors: Vec<_> = texts(&out)[..2].iter().map(|t| t.style.color).collect();
        assert_eq!(colors, vec![0, 3]);
    }

    // The current level's row has used byte 0: it
    // draws only the frame-0 icon.
    #[test]
    fn the_current_row_draws_only_frame_0() {
        let mut v = view();
        v.rows = vec![
            row(1, false, true),
            row(3, true, false),
            row(4, true, false),
        ];
        let (t, out) = draw(&WaypointPanel::default(), &env(Screen::R640, true), &v);
        let icons = t.files.id(ICONS_FILE).unwrap();
        let im: Vec<_> = images(&out).into_iter().filter(|i| i.0 == icons).collect();
        assert_eq!(
            im,
            vec![(icons, 0, 17, 89), (icons, 3, 17, 125), (icons, 3, 17, 161)]
        );
    }

    // Row icons (frame sel / 3 + sel for a used row, frame 0 for the
    // current level whatever its used byte), positions, text colors.
    #[test]
    fn rows_icons_and_text() {
        let mut v = view();
        // The current level's used byte is 0: only its frame-0 icon.
        v.rows = vec![
            row(1, true, false),
            row(3, false, true),
            row(4, false, false),
            row(5, true, false),
            row(6, true, false),
        ];
        let p = WaypointPanel {
            selected: Some(3),
            ..Default::default()
        };
        let (t, out) = draw(&p, &env(Screen::R640, true), &v);
        let icons = t.files.id(ICONS_FILE).unwrap();
        let im: Vec<_> = images(&out).into_iter().filter(|i| i.0 == icons).collect();
        assert_eq!(
            im,
            vec![
                (icons, 3, 17, 89),
                (icons, 0, 17, 125),
                (icons, 4, 17, 197),
                (icons, 3, 17, 234),
            ]
        );
        let tx = texts(&out);
        let rows: Vec<_> = tx[..5]
            .iter()
            .map(|t| (t.at.x, t.at.y, t.style.font, t.style.color))
            .collect();
        assert_eq!(
            rows,
            vec![
                (80, 84, 1, 0),
                (80, 119, 1, 3),
                (80, 154, 1, 5),
                (80, 189, 1, 3),
                (80, 224, 1, 0)
            ]
        );
        assert_eq!(tx[0].text, utf16("L1"));
        // A used current row (never built by the row rebuild) would draw both icons.
        let mut v2 = view();
        v2.rows = vec![row(3, true, true)];
        let (_, out) = draw(&WaypointPanel::default(), &env(Screen::R640, true), &v2);
        let im: Vec<_> = images(&out).into_iter().filter(|i| i.0 == icons).collect();
        assert_eq!(im, vec![(icons, 0, 17, 89), (icons, 0, 17, 89)]);
        // 800 × 600: + (80, 60).
        let (_, out) = draw(&p, &env(Screen::R800, true), &v);
        let im: Vec<_> = images(&out).into_iter().filter(|i| i.0 == icons).collect();
        assert_eq!(im[0], (icons, 3, 97, 149));
        assert_eq!((texts(&out)[4].at.x, texts(&out)[4].at.y), (160, 284));
        // At most nine rows.
        v.rows = (0..12).map(|l| row(l, true, false)).collect();
        let (_, out) = draw(&WaypointPanel::default(), &env(Screen::R640, true), &v);
        let im: Vec<_> = images(&out).into_iter().filter(|i| i.0 == icons).collect();
        assert_eq!(im.len(), 9);
        assert_eq!(im[8], (icons, 3, 17, 378));
        assert_eq!(texts(&out).len(), 10);
    }

    #[test]
    fn title_header_or_none() {
        let p = WaypointPanel::default();
        let (_, out) = draw(&p, &env(Screen::R800, true), &view());
        let tx = texts(&out);
        let title = tx.last().unwrap();
        assert_eq!(title.text, utf16("Waypoints"));
        // width 63: x = 80 + 160 − 31.
        assert_eq!(
            (title.at.x, title.at.y, title.style.font, title.style.color),
            (209, 108, 1, 0)
        );
        let mut v = view();
        v.other = false;
        let (_, out) = draw(&p, &env(Screen::R640, true), &v);
        let title = *texts(&out).last().unwrap();
        assert_eq!(title.text, utf16("No Waypoints"));
        // width 84: x = 160 − 42.
        assert_eq!((title.at.x, title.at.y), (118, 48));
    }

    #[test]
    fn messages_choose_and_close() {
        let mut p = WaypointPanel::new();
        assert_eq!(
            p.open(0x0B),
            vec![PanelOutput::SetUi {
                ui: 0x14,
                mode: 0,
                jump: true
            }]
        );
        let mut v = view();
        v.rows = vec![row(1, true, false), row(0x1D, true, false)];
        assert_eq!(
            p.choose_row(&v, 1),
            vec![PanelOutput::Intent(ClientIntent(vec![
                0x49, 0x0B, 0, 0, 0, 0x1D, 0, 0, 0
            ]))]
        );
        assert!(p.choose_row(&v, 2).is_empty());
        let close = PanelOutput::Intent(ClientIntent(vec![0x49, 0x0B, 0, 0, 0, 0, 0, 0, 0]));
        let off = PanelOutput::SetUi {
            ui: 0x14,
            mode: 1,
            jump: true,
        };
        // The row choice set the close latch: the
        // close sends no level-0 message after it, and neither does the
        // hook.
        assert_eq!(p.close(), vec![off.clone()]);
        assert!(p.close_hook().is_empty());
        // Without a row: one level-0 message, then the latch holds.
        p.open(0x0B);
        assert_eq!(p.close(), vec![close.clone(), off]);
        assert!(p.close_hook().is_empty());
        p.open(0x0B);
        assert_eq!(p.close_hook(), vec![close]);
    }

    // Player gone → close, 0x49 level 0 once (latch).
    #[test]
    fn self_close_latch() {
        let mut p = WaypointPanel::new();
        p.open(7);
        assert!(p.self_close(true).is_empty());
        let off = PanelOutput::SetUi {
            ui: 0x14,
            mode: 1,
            jump: false,
        };
        let msg = PanelOutput::Intent(ClientIntent(vec![0x49, 7, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(p.self_close(false), vec![off.clone(), msg.clone()]);
        assert_eq!(p.self_close(false), vec![off.clone()]);
        p.open(7);
        assert_eq!(p.self_close(false), vec![off, msg]);
    }

    fn rows_012() -> WpRows {
        let mut r = WpRows::default();
        for i in 0..3 {
            r.used[i] = true;
            r.level[i] = 10 + i as u32;
        }
        r
    }

    fn open_panel() -> WaypointPanel {
        let mut p = WaypointPanel::new();
        p.open(0x0B);
        p
    }

    fn off(jump: bool) -> PanelOutput {
        PanelOutput::SetUi {
            ui: 0x14,
            mode: 1,
            jump,
        }
    }

    fn msg49(level: u32) -> PanelOutput {
        PanelOutput::Intent(ClientIntent(
            [&[0x49u8, 0x0B, 0, 0, 0][..], &level.to_le_bytes()].concat(),
        ))
    }

    fn q_all(_: u32) -> bool {
        true
    }

    #[test]
    fn mini_panel_blocks_the_menu_mouse() {
        // 800 × 600, single player (a 56, n 7): x strictly
        // inside (456, 596), y (524, 550).
        assert!(mini_panel_blocks(800, 600, true, true, 457, 525));
        assert!(mini_panel_blocks(800, 600, true, true, 595, 549));
        assert!(!mini_panel_blocks(800, 600, true, true, 456, 525));
        assert!(!mini_panel_blocks(800, 600, true, true, 596, 525));
        assert!(!mini_panel_blocks(800, 600, true, true, 500, 524));
        assert!(!mini_panel_blocks(800, 600, true, true, 500, 550));
        // Multiplayer: a 35, n 8: (435, 595).
        assert!(mini_panel_blocks(800, 600, true, false, 436, 530));
        assert!(!mini_panel_blocks(800, 600, true, false, 435, 530));
        assert!(mini_panel_blocks(800, 600, true, false, 594, 530));
        assert!(!mini_panel_blocks(800, 600, true, false, 595, 530));
        // Only with both panels open.
        assert!(!mini_panel_blocks(800, 600, false, true, 500, 530));
        // Both handlers ignore the event while it holds.
        let env = WpEnv::new(&Screen::R800, true);
        let mut p = open_panel();
        assert_eq!(
            p.mouse_down(&env, 300, 80, &rows_012(), true, &q_all),
            (vec![], false)
        );
        assert_eq!(
            p.mouse_up(&env, 300, 80, &rows_012(), true),
            (vec![], false)
        );
        assert_eq!(p.tab, 0);
    }

    // Row hit test vectors.
    #[test]
    fn tab_and_row_hit_tests() {
        // 800 × 600, expansion game, mouse (300, 80): x' = 220, y' = 20 →
        // tab 3.
        let env = WpEnv::new(&Screen::R800, true);
        assert_eq!((300 - env.sx, 80 + env.sy), (220, 20));
        assert_eq!(tab_hit(220, 20, true), Some(3));
        // 640 × 480, classic, (100, 10) → tab 1.
        assert_eq!(tab_hit(100, 10, false), Some(1));
        // Edges: expansion 63 → 0, 64 → 1, 255 → 3, 256 → 4; classic 79 →
        // 0, 240 → 3; none for y' > 30 or x' > 320; negative values hit.
        for (x, t) in [
            (63, 0),
            (64, 1),
            (127, 1),
            (128, 2),
            (191, 2),
            (192, 3),
            (255, 3),
            (256, 4),
        ] {
            assert_eq!(tab_hit(x, 0, true), Some(t), "x' {x}");
        }
        for (x, t) in [(79, 0), (80, 1), (159, 1), (160, 2), (239, 2), (240, 3)] {
            assert_eq!(tab_hit(x, 0, false), Some(t), "x' {x}");
        }
        assert_eq!(tab_hit(-50, -50, true), Some(0));
        assert_eq!(tab_hit(100, 31, true), None);
        assert_eq!(tab_hit(321, 30, true), None);
        assert_eq!(tab_hit(320, 30, true), Some(4));
        // Rows: 800 × 600, rows 0–2 used, (200, 200): x' = 120, y' = 140 →
        // row 2 (132 < 140 < 162).
        assert_eq!((200 - env.sx, 200 + env.sy), (120, 140));
        let rows = rows_012();
        assert_eq!(row_hit(120, 140, &rows), Some(2));
        // Strict bounds and used rows only.
        assert_eq!(row_hit(17, 70, &rows), None);
        assert_eq!(row_hit(18, 70, &rows), Some(0));
        assert_eq!(row_hit(296, 70, &rows), Some(0));
        assert_eq!(row_hit(297, 70, &rows), None);
        assert_eq!(row_hit(100, 60, &rows), None);
        assert_eq!(row_hit(100, 61, &rows), Some(0));
        assert_eq!(row_hit(100, 89, &rows), Some(0));
        assert_eq!(row_hit(100, 90, &rows), None);
        assert_eq!(row_hit(100, 170, &rows), None);
        // The hy table.
        assert_eq!(ROW_HY, [60, 96, 132, 168, 205, 241, 277, 313, 349]);
        let all = WpRows {
            used: [true; 9],
            ..Default::default()
        };
        assert_eq!(row_hit(100, 350, &all), Some(8));
    }

    #[test]
    fn tab_setter_walks_down_the_quest_records() {
        let only = |ok: &'static [u32]| move |r: u32| ok.contains(&r);
        // t ≥ 5 → 0.
        assert_eq!(set_tab(5, &q_all), 0);
        assert_eq!(set_tab(9, &q_all), 0);
        // Every record set: the tab itself.
        for t in 0..5 {
            assert_eq!(set_tab(t, &q_all), t);
        }
        // Tab 4 needs record 28 (the draw tests 26: reproduce).
        assert_eq!(set_tab(4, &only(&[26, 23, 15, 7])), 3);
        assert_eq!(set_tab(4, &only(&[28])), 4);
        // A failed check walks down: 3 needs 23, 2 needs 15, 1 needs 7.
        assert_eq!(set_tab(3, &only(&[15, 7])), 2);
        assert_eq!(set_tab(3, &only(&[7])), 1);
        assert_eq!(set_tab(3, &only(&[])), 0);
        assert_eq!(set_tab(2, &only(&[7])), 1);
        assert_eq!(TAB_QUEST_RECORD, [0, 7, 15, 23, 28]);
    }

    #[test]
    fn mouse_down_paths() {
        let env = WpEnv::new(&Screen::R800, true);
        let rows = rows_012();
        // On a tab that differs: sound 6, the tab set, the rows rebuilt;
        // pressed row cleared.
        let mut p = open_panel();
        p.selected = Some(1);
        let (e, c) = p.mouse_down(&env, 300, 80, &rows, false, &q_all);
        assert_eq!(e, vec![WpEffect::Sound(6), WpEffect::RebuildRows]);
        assert!(c);
        assert_eq!((p.tab, p.selected, p.close_pressed), (3, None, false));
        // The same tab again: no sound.
        let (e, _) = p.mouse_down(&env, 300, 80, &rows, false, &q_all);
        assert_eq!(e, vec![WpEffect::RebuildRows]);
        // A quest-gated tab falls back.
        let mut p = open_panel();
        p.mouse_down(&env, 300, 80, &rows, false, &|r| r == 7);
        assert_eq!(p.tab, 1);
        // On a row: pressed row set and sound 4.
        let mut p = open_panel();
        let (e, c) = p.mouse_down(&env, 200, 200, &rows, false, &q_all);
        assert_eq!(e, vec![WpEffect::Sound(4)]);
        assert!(c);
        assert_eq!(p.selected, Some(2));
        // On a gap between rows: nothing pressed, no sound.
        let (e, _) = p.mouse_down(&env, 200, 225, &rows, false, &q_all);
        assert!(e.is_empty());
        assert_eq!(p.selected, None);
        // On the close rectangle: close pressed and sound 4 (x' 273–308,
        // y' 387–420 → (353…388, 447…480)).
        let (e, _) = p.mouse_down(&env, 353 + 5, 447 + 5, &rows, false, &q_all);
        assert_eq!(e, vec![WpEffect::Sound(4)]);
        assert!(p.close_pressed);
        // x > W / 2: the latched close and SetUIState(0x14, off, jump 1);
        // consumed while x ≤ W and y ≤ H − 49.
        let mut p = open_panel();
        let (e, c) = p.mouse_down(&env, 401, 100, &rows, false, &q_all);
        assert_eq!(e, vec![WpEffect::Out(msg49(0)), WpEffect::Out(off(true))]);
        assert!(c);
        // y > H − 49 (a click on the control panel): closes, not consumed.
        let mut p = open_panel();
        let (e, c) = p.mouse_down(&env, 100, 552, &rows, false, &q_all);
        assert_eq!(e.len(), 2);
        assert!(!c);
        // Beyond the right edge: closes, not consumed.
        let mut p = open_panel();
        let (_, c) = p.mouse_down(&env, 801, 100, &rows, false, &q_all);
        assert!(!c);
        // 640 × 480 classic: (100, 10) → tab 1.
        let env = WpEnv::new(&Screen::R640, false);
        let mut p = open_panel();
        p.mouse_down(&env, 100, 10, &rows, false, &q_all);
        assert_eq!(p.tab, 1);
    }

    // Test vector "waypoint: row chosen, then the close hook runs".
    #[test]
    fn latch_gives_one_0x49_per_open() {
        let env = WpEnv::new(&Screen::R800, true);
        let rows = rows_012();
        let mut p = open_panel();
        // Row 2 pressed, released over it: one 0x49 (the row's), the close
        // hook sends no level-0 message.
        p.mouse_down(&env, 200, 200, &rows, false, &q_all);
        let (e, c) = p.mouse_up(&env, 200, 200, &rows, false);
        assert!(c);
        let sent: Vec<_> = e
            .iter()
            .filter(|e| matches!(e, WpEffect::Out(PanelOutput::Intent(_))))
            .collect();
        assert_eq!(sent, vec![&WpEffect::Out(msg49(12))]);
        assert!(p.close_hook().is_empty());
        // A new open clears the latch: one level-0 message at most.
        p.open(0x0B);
        assert_eq!(p.close_hook(), vec![msg49(0)]);
        assert!(p.close_hook().is_empty());
        // No player or no room: the latched send needs both and sends
        // nothing.
        let mut p = open_panel();
        p.no_player_room = true;
        assert!(p.close_hook().is_empty());
        assert_eq!(p.close(), vec![off(true)]);
    }

    #[test]
    fn mouse_up_paths() {
        let env = WpEnv::new(&Screen::R800, true);
        let rows = rows_012();
        // x > W or y > H − 49: clears close pressed and pressed row, not
        // consumed.
        let mut p = open_panel();
        p.close_pressed = true;
        p.selected = Some(1);
        let (e, c) = p.mouse_up(&env, 801, 100, &rows, false);
        assert!(e.is_empty() && !c);
        assert!(!p.close_pressed && p.selected.is_none());
        let mut p = open_panel();
        p.selected = Some(1);
        assert!(!p.mouse_up(&env, 100, 552, &rows, false).1);
        // Close pressed and released inside: SetUIState(0x14, off, 0) and
        // the latched level-0 send.
        let mut p = open_panel();
        p.close_pressed = true;
        let (e, c) = p.mouse_up(&env, 358, 452, &rows, false);
        assert_eq!(e, vec![WpEffect::Out(off(false)), WpEffect::Out(msg49(0))]);
        assert!(c && !p.close_pressed);
        // Close pressed and released outside: nothing, both cleared.
        let mut p = open_panel();
        p.close_pressed = true;
        let (e, c) = p.mouse_up(&env, 200, 200, &rows, false);
        assert!(e.is_empty() && c && !p.close_pressed);
        // Row pressed and released over it: 0x49 [GUID][level], close off
        // (jump 0), sound 0x8B7; the latch is set.
        let mut p = open_panel();
        p.selected = Some(2);
        let (e, c) = p.mouse_up(&env, 200, 200, &rows, false);
        assert_eq!(
            e,
            vec![
                WpEffect::Out(msg49(12)),
                WpEffect::Out(off(false)),
                WpEffect::Sound(0x8B7)
            ]
        );
        assert!(c && p.selected.is_none());
        // Released over another row: nothing, both cleared, consumed.
        let mut p = open_panel();
        p.selected = Some(2);
        let (e, c) = p.mouse_up(&env, 200, 100, &rows, false);
        assert!(e.is_empty() && c && p.selected.is_none());
    }

    #[test]
    fn self_close_keeps_drawing() {
        let mut p = open_panel();
        // A player in a good mode: nothing.
        let r = p.self_close_checked(true, true);
        assert!(r.outputs.is_empty() && !r.draw_continues);
        // Dead (mode 0x11): the close hook sends the latched level 0; the
        // draw continues.
        let r = p.self_close_checked(true, false);
        assert_eq!(r.outputs, vec![off(false), msg49(0)]);
        assert!(r.draw_continues);
        // The latched send after it has nothing left.
        assert!(p.close_hook().is_empty());
        // No player: the menu closes, nothing is sent.
        let mut p = open_panel();
        let r = p.self_close_checked(false, false);
        assert_eq!(r.outputs, vec![off(false)]);
        assert!(r.draw_continues);
    }

    #[test]
    fn key_close_keys() {
        // Any key message closes: latched send, off with jump 1, consumed.
        let mut p = open_panel();
        let (e, c) = p.key_close(0x100, 0x1B);
        assert_eq!(e, vec![msg49(0), off(true)]);
        assert!(c);
        // WM_CHAR with Tab and WM_SYSKEYDOWN with F4 pass.
        let mut p = open_panel();
        assert_eq!(p.key_close(0x102, 9), (vec![], false));
        assert_eq!(p.key_close(0x104, 0x73), (vec![], false));
        // WM_CHAR with another key closes; WM_SYSKEYDOWN with another too.
        let (_, c) = p.key_close(0x102, 0x41);
        assert!(c);
        let mut p = open_panel();
        let (_, c) = p.key_close(0x104, 0x72);
        assert!(c);
    }
}
