//! Skill tree (ui 4; right slot): the class art and its tab frames, the skill
//! icons of the current tab with their level numbers, the tab and icon mouse
//! handlers and the close button.
//!
//! Game facts (class, skills, free points) come from a [`SkillTreeView`]
//! the caller fills from the client world; the panel decides nothing: a
//! spent point leaves as C→S 0x3B, a close as `SetUIState`.
//!
//! Open questions:
//! - The class icon file prefix (`CC` of `spells\CCskillicon`) is
//!   not specified; the icon file id comes from the view
//!   ([`SkillTreeView::icon_file`]). No icon is drawn without it.
//! - The remap `k` of an icon is implemented literally in the stated order;
//!   the exact remap `k` per state is not measured yet. Unverified.
//!   The icon's [`UiDraw::Image`] carries it as `look.remap =
//!   Remap::Palette(k)` (the colored cel draw).
//! - The close-button offset table is only known for the
//!   amazon; other classes have no close button here
//!   ([`close_offset`] returns `None`).
//! - The no-points mouse-down message (9 bytes), the
//!   free-points box and the tab tool tips: not implemented.
//! - Whether the close button is pressed by a mouse down (the general
//!   panel rule; the skill tree names only the hover / release rectangle), whether a
//!   release elsewhere clears it, and the order of the close test against
//!   the tab / icon tests (the close rectangle overlaps column 3 row 6):
//!   not stated; implemented as press-sets / release-clears, tabs and
//!   icons first.
//! - what the release handler does besides not consuming a click
//!   at x ≤ W / 2 is not stated; here it only clears the pressed icon.
//! - Draw order icons → level numbers → close button follows the order of
//!   the panel's parts, not a measured draw.

use d2_proto::client::{AddSkillPoint, SelectSkill};

use super::{cel, emit_static_draws, text, utf16, PanelEnv, PanelOutput, PanelTables};
use crate::ui::draw::{UiDraw, UiDrawSink};
use crate::ui::geom::{Point, Rect};
use crate::ui::layout::{Cond, PanelKey, Screen};
use crate::ui::panel::ClientIntent;

/// UI state id of the skill tree.
pub const UI_SKILLTREE: u8 = 4;
/// Font16 (`text-fonts.tsv` id 1): level numbers below 10.
pub const FONT16: u16 = 1;
/// FontFormal10 (`text-fonts.tsv` id 4): level numbers ≥ 10.
pub const FONT_FORMAL10: u16 = 4;
/// Close button file, path under `data\global\ui\`.
pub const CLOSE_FILE: &str = "panel\\buysellbtn";
/// Size of the close button's hover / release rectangle (0x22 × 0x22).
pub const CLOSE_SIZE: i32 = 0x22;

/// Icon x offsets for columns 1–3: `X = W − sx − c`.
const COLUMN_X: [i32; 3] = [305, 236, 167];
/// Icon y offsets for rows 1–6: `Y = H + sy − r`.
const ROW_Y: [i32; 6] = [418, 350, 282, 214, 145, 77];

/// One class skill as the skill tree reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillEntry {
    /// skills.txt row (the 0x3B payload).
    pub skill: u16,
    /// skilldesc `SkillPage` (+2): the tab 1–3 (0: not on the tree).
    pub page: u8,
    /// skilldesc `SkillRow` (+3), 1–6.
    pub row: u8,
    /// skilldesc `SkillColumn` (+4), 1–3.
    pub column: u8,
    /// skilldesc `IconCel` (+7).
    pub icon_cel: u32,
    /// The skill level shown (`%d`).
    pub level: i32,
    /// Hard points (skill entry +0x28).
    pub hard_points: i32,
    /// The bonus part.
    pub bonus: i32,
    /// The skill's flag byte (+5) tested against [`SkillTreeView::flag_mask`].
    pub flags: u8,
    /// A point would be accepted (start remap 0 rather than
    /// 5 for a level-0 skill).
    pub learnable: bool,
    /// The skill is `passive` (`skills.txt` flags byte +4 has the
    /// `passive` bit of the skill flag mask).
    pub passive: bool,
    /// The required-level check passes.
    pub req_level_ok: bool,
    /// Below the skill's max level.
    pub below_max: bool,
}

/// The game facts the skill tree draws and tests.
pub trait SkillTreeView {
    /// Player class 0–6 (amazon … assassin, [`super::CLASS_LETTERS`]).
    fn class(&self) -> Option<u8>;
    /// The class icon file id (`spells\CCskillicon`, prefix unspecified:
    /// Open, module doc).
    fn icon_file(&self) -> Option<u32>;
    /// Free skill points (stat 5).
    fn free_points(&self) -> i32;
    /// The skill flag mask: an icon whose flag byte has no bit of it uses
    /// remap 1.
    fn flag_mask(&self) -> u8;
    /// The class skills in the class skill list's order.
    fn skills(&self) -> &[SkillEntry];
}

/// One icon draw with what [`UiDraw::Image`] cannot carry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconDraw {
    pub skill: u16,
    /// The cel draw (frame = `IconCel` + 1 while pressed).
    pub image: UiDraw,
    /// Remap `k` of the colored cel draw (unverified).
    pub remap: u8,
    /// The level number, when drawn.
    pub level: Option<UiDraw>,
}

/// A release's result: whether the skill tree consumed it and
/// what it asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Release {
    pub consumed: bool,
    pub out: Vec<PanelOutput>,
}

/// Icon draw position (X, Y) of a skilldesc column 1–3 / row 1–6.
pub fn icon_pos(s: &Screen, column: u8, row: u8) -> Option<Point> {
    let cx = COLUMN_X.get(usize::from(column).checked_sub(1)?)?;
    let ry = ROW_Y.get(usize::from(row).checked_sub(1)?)?;
    Some(Point::new(s.panel_right() - cx, s.h + s.sy() - ry))
}

/// Icon hit test (strict): `X < mx < X + 48`, `Y − 48 < my < Y`.
pub fn icon_hit(at: Point, p: Point) -> bool {
    icon_rect(at).contains(p)
}

/// The integer interior of the strict icon hit test.
fn icon_rect(at: Point) -> Rect {
    Rect::new(at.x + 1, at.y - 47, 47, 47)
}

/// The tab a mouse down at `p` selects, if any: only when
/// my ≤ H − 48; x in [`W − sx − 88`, `W − sx`]; y strictly inside the
/// tab's band, the tab-1 bottom bound `H − 49` without `sy`.
pub fn tab_at(s: &Screen, p: Point) -> Option<u8> {
    (1..=3).find(|&tab| tab_rect(s, tab).is_some_and(|r| r.contains(p)))
}

/// Integer interior of the tab hit bands, clipped above the control panel
/// and at the frame's right edge: in a wide frame the right panel ends at
/// the frame's last column, so the band's last column (`panel_right`) is
/// one past it.
fn tab_rect(s: &Screen, tab: u8) -> Option<Rect> {
    let b = s.h + s.sy();
    let (lo, hi) = match tab {
        1 => (b - 156, s.h - 49),
        2 => (b - 264, b - 157),
        3 => (b - 372, b - 265),
        _ => return None,
    };
    let bottom = (hi - 1).min(s.h - 48);
    let x = s.panel_right() - 88;
    let right = (s.panel_right() + 1).min(s.frame_right());
    Some(Rect::new(
        x,
        lo + 1,
        u16::try_from(right - x).ok()?,
        u16::try_from(bottom - lo).ok()?,
    ))
}

/// Close-button offset `o` (table, index `3 × class + t`): amazon −149, −220,
/// −305; sorceress −305, −305, −149; necromancer −305, −149, −305; paladin
/// −305, −220, −305; barbarian −149, −305, −149; druid −149, −149, −149;
/// assassin −220, −149, −305 for tabs 1, 2, 3.
pub fn close_offset(class: u8, tab: u8) -> Option<i32> {
    const O: [[i32; 3]; 7] = [
        [-149, -220, -305],
        [-305, -305, -149],
        [-305, -149, -305],
        [-305, -220, -305],
        [-149, -305, -149],
        [-149, -149, -149],
        [-220, -149, -305],
    ];
    let t = usize::from(tab).checked_sub(1)?;
    O.get(usize::from(class))?.get(t).copied()
}

/// The close variant: 1, 2, 3 for `o`
/// = −149, −220, −305.
pub fn close_variant(o: i32) -> Option<u8> {
    match o {
        -149 => Some(1),
        -220 => Some(2),
        -305 => Some(3),
        _ => None,
    }
}

/// Close-button draw position (`W − sx + o`, `H + sy − 63`).
pub fn close_pos(s: &Screen, class: u8, tab: u8) -> Option<Point> {
    let o = close_offset(class, tab)?;
    Some(Point::new(s.panel_right() + o, s.h + s.sy() - 63))
}

/// Close hover / release rectangle: x in [`X`, `X + 0x22`], y in
/// [`Y − 0x22`, `Y`], bounds inclusive as written.
pub fn close_hit(s: &Screen, class: u8, tab: u8, p: Point) -> bool {
    close_rect(s, class, tab).is_some_and(|r| r.contains(p))
}

/// Inclusive close bounds converted to the UI's half-open rectangle.
fn close_rect(s: &Screen, class: u8, tab: u8) -> Option<Rect> {
    let at = close_pos(s, class, tab)?;
    let size = (CLOSE_SIZE + 1) as u16;
    Some(Rect::new(at.x, at.y - CLOSE_SIZE, size, size))
}

/// Tab captions per class: string id and
/// the offset K of y = `H + sy + K`; the three header lines come first for
/// every class, in the panel's draw order.
const HEADER_CAPTIONS: [(u16, i32); 3] = [(4227, -455), (4228, -443), (4229, -431)];

fn class_captions(class: u8) -> &'static [(u16, i32)] {
    match class {
        0 => &[
            (4232, -330),
            (4233, -318),
            (4230, -306),
            (4234, -222),
            (4235, -210),
            (4230, -198),
            (4236, -121),
            (4237, -109),
            (4230, -97),
        ],
        1 => &[
            (4249, -324),
            (4231, -312),
            (4250, -216),
            (4231, -204),
            (4251, -110),
            (4231, -98),
        ],
        2 => &[
            (4242, -324),
            (4231, -312),
            (4243, -222),
            (4244, -210),
            (4231, -198),
            (4245, -104),
        ],
        3 => &[
            (4238, -324),
            (4239, -312),
            (4240, -216),
            (4239, -204),
            (4241, -110),
            (4230, -98),
        ],
        4 => &[
            (4246, -318),
            (4247, -216),
            (4248, -204),
            (4247, -110),
            (4230, -98),
        ],
        5 => &[(22512, -318), (22510, -216), (22511, -204), (22509, -110)],
        6 => &[
            (22516, -330),
            (22517, -318),
            (22514, -216),
            (22515, -204),
            (22513, -110),
        ],
        _ => &[],
    }
}

/// The caption lines of the class's tab column: the header lines then the
/// class's own, as (string id, y offset K) (y = `H + sy + K`).
pub fn caption_lines(class: u8) -> impl Iterator<Item = (u16, i32)> {
    HEADER_CAPTIONS
        .into_iter()
        .chain(class_captions(class).iter().copied())
}

/// Span of the captions (the string width from `W − sx − 90`).
pub fn caption_span(s: &Screen) -> (i32, i32) {
    (s.panel_right() - 90, s.panel_right())
}

/// The "no free points" number's span and y ("0" centered in [`W − sx − 65`, `W
/// − sx − 25`] at `H + sy − 400`, color 0).
pub fn zero_points_span(s: &Screen) -> (i32, i32, i32) {
    (
        s.panel_right() - 65,
        s.panel_right() - 25,
        s.h + s.sy() - 400,
    )
}

/// Skill tree panel state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillTreePanel {
    /// Current tab, 1–3.
    pub tab: u8,
    /// The pressed icon's skill.
    pub pressed: Option<u16>,
    /// The close button's pressed flag (frame 11).
    pub close_pressed: bool,
    /// Set by the draw.
    pub drawn: std::cell::Cell<bool>,
}

impl Default for SkillTreePanel {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillTreePanel {
    /// Tab 1, nothing pressed.
    pub fn new() -> Self {
        Self {
            tab: 1,
            pressed: None,
            close_pressed: false,
            drawn: std::cell::Cell::new(false),
        }
    }

    /// Visible controls that can act through the current input handlers.
    pub fn ui_elements(
        &self,
        s: &Screen,
        view: &dyn SkillTreeView,
    ) -> Vec<crate::ui::ui_ids::UiElement> {
        use crate::ui::ui_ids::UiElement;
        let Some(class) = view.class() else {
            return Vec::new();
        };
        if !self.drawn.get() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for tab in 1..=3 {
            if let Some(rect) = tab_rect(s, tab) {
                out.push(
                    UiElement::new(format!("skilltree.tab({tab})"), rect)
                        .idle_if(tab == self.tab, "the tab already shown"),
                );
            }
        }
        if let Some(rect) = close_rect(s, class, self.tab) {
            let mut close = UiElement::new("skilltree.close", rect);
            // The centre can overlap a bottom-row icon. Its bottom strip
            // is still inside the close hit region and below every icon.
            close.at.y = rect.y + i32::from(rect.h) - 2;
            out.push(close);
        }
        if view.icon_file().is_some() {
            for (e, at) in self.tab_skills(s, view) {
                let available = if view.free_points() > 0 {
                    e.learnable
                } else {
                    !e.passive && self.pressed != Some(e.skill)
                };
                if available {
                    let mut element =
                        UiElement::new(format!("skilltree.skill({})", e.skill), icon_rect(at));
                    // Close takes precedence on release. Bottom-row icons still
                    // have an uncovered top edge when their centre is covered.
                    if close_hit(s, class, self.tab, element.at) {
                        element.at.y = element.rect.y;
                    }
                    if !close_hit(s, class, self.tab, element.at) {
                        out.push(element);
                    }
                }
            }
        }
        out
    }

    /// The skills of the current tab with their icon positions.
    fn tab_skills<'a>(
        &'a self,
        s: &'a Screen,
        view: &'a dyn SkillTreeView,
    ) -> impl Iterator<Item = (&'a SkillEntry, Point)> + 'a {
        view.skills()
            .iter()
            .filter(move |e| e.page == self.tab)
            .filter_map(move |e| Some((e, icon_pos(s, e.column, e.row)?)))
    }

    /// Remap `k` of an icon, the stated order applied literally
    /// (unverified): start 5 (level 0 and not learnable) or 0;
    /// 3 if the mouse is strictly inside; 1 if the flag byte has no bit
    /// of the mask; 3 if no free points, a level and not pressed.
    pub fn icon_remap(
        &self,
        view: &dyn SkillTreeView,
        e: &SkillEntry,
        at: Point,
        mouse: Point,
    ) -> u8 {
        let mut k = if e.level == 0 && !e.learnable { 5 } else { 0 };
        if icon_hit(at, mouse) {
            k = 3;
        }
        if e.flags & view.flag_mask() == 0 {
            k = 1;
        }
        if view.free_points() <= 0 && e.level > 0 && self.pressed != Some(e.skill) {
            k = 3;
        }
        k
    }

    /// The icon of the current tab under the mouse (the strict rectangle): its
    /// skill, shown level and cel draw point.
    pub fn hovered(
        &self,
        s: &Screen,
        view: &dyn SkillTreeView,
        mouse: Point,
    ) -> Option<(u16, i32, Point)> {
        self.tab_skills(s, view)
            .find(|(_, at)| icon_hit(*at, mouse))
            .map(|(e, at)| (e.skill, e.level, at))
    }

    /// The icons of the current tab; empty without the
    /// icon file (Open).
    pub fn icons(&self, s: &Screen, view: &dyn SkillTreeView, mouse: Point) -> Vec<IconDraw> {
        let Some(file) = view.icon_file() else {
            return Vec::new();
        };
        self.tab_skills(s, view)
            .map(|(e, at)| {
                let pressed = self.pressed == Some(e.skill);
                let remap = self.icon_remap(view, e, at, mouse);
                let mut image = cel(file, e.icon_cel + u32::from(pressed), at.x, at.y);
                // The colored cel draw with `k`; k 0 is
                // the plain draw.
                if let UiDraw::Image(r) = &mut image {
                    // Always goes through the colour draw.
                    r.call = crate::ui::draw::CelCall::Color;
                    r.look.remap = crate::ui::Remap::Palette(i32::from(remap));
                }
                IconDraw {
                    skill: e.skill,
                    image,
                    remap,
                    level: level_text(e, at),
                }
            })
            .collect()
    }

    /// Draws the panel: art frames 0–3 and `4t … 4t + 3`, the
    /// icons and level numbers, the close button.
    pub fn draw(
        &self,
        t: &PanelTables,
        env: &PanelEnv,
        view: &dyn SkillTreeView,
        mouse: Point,
        out: &mut dyn UiDrawSink,
    ) {
        self.draw_with_points(t, env, view, mouse, None, &[], out);
    }

    /// [`Self::draw`] with the free-points number (after the background, before
    /// the icons) when `uninterruptable` is given (the player's state 54).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_with_points(
        &self,
        t: &PanelTables,
        env: &PanelEnv,
        view: &dyn SkillTreeView,
        mouse: Point,
        uninterruptable: Option<bool>,
        captions: &[UiDraw],
        out: &mut dyn UiDrawSink,
    ) {
        self.drawn.set(true);
        let tab = self.tab;
        let extra = move |c: Cond| matches!(c, Cond::Tab(n) if n == tab);
        let cond = env.cond(false, &extra);
        let class = view.class();
        emit_static_draws(t, PanelKey::Ui(UI_SKILLTREE), &cond, class, &|_| true, out);
        // The tab captions (and the "0" without points) come
        // before the number and the icons.
        for c in captions {
            out.push(c.clone());
        }
        if let Some((n, at, font, color)) =
            uninterruptable.and_then(|u| self.free_points_number(&env.screen, view, u))
        {
            out.push(text(utf16(&n), at.x, at.y, font, color));
        }
        for icon in self.icons(&env.screen, view, mouse) {
            out.push(icon.image);
            if let Some(l) = icon.level {
                out.push(l);
            }
        }
        if let (Some(c), Some(file)) = (class, t.files.id(CLOSE_FILE)) {
            if let Some(at) = close_pos(&env.screen, c, self.tab) {
                out.push(cel(file, 10 + u32::from(self.close_pressed), at.x, at.y));
            }
        }
    }

    /// Mouse down with the default context (a player, not
    /// over the belt): its outputs. See [`SkillTreePanel::mouse_down_ctx`].
    pub fn mouse_down(
        &mut self,
        env: &PanelEnv,
        view: &dyn SkillTreeView,
        p: Point,
    ) -> Vec<PanelOutput> {
        self.mouse_down_ctx(env, view, p, InputCtx::DEFAULT).out
    }

    /// Mouse down, in this order (not
    /// consumed when over the belt, without a player, or y > `H − 48`):
    /// 1. no draw yet → input reset, consumed, done;
    /// 2. no free points and `W − sx − 320` < x < `W − sx − 88`: for each
    ///    icon of the tab that is hit, not pressed and not passive: C→S
    ///    0x3C [skill, right hand][−1] then `SetUIState(4, off, 0)`; the
    ///    walk goes on; consumed;
    /// 3. tabs (x in [`W − sx − 88`, `W − sx`]): consumed, goes on;
    /// 4. the close rectangle: click sound, pressed; goes on;
    /// 5. the icon column: no points → consumed, done; else the first hit,
    ///    not pressed icon is pressed, click sound, consumed, done; none:
    ///    consumed, done;
    /// 6. otherwise not consumed.
    pub fn mouse_down_ctx(
        &mut self,
        env: &PanelEnv,
        view: &dyn SkillTreeView,
        p: Point,
        ctx: InputCtx,
    ) -> Down {
        let s = env.screen;
        let mut d = Down::default();
        if ctx.over_belt || !ctx.has_player || p.y > s.h - 48 {
            return d;
        }
        if !self.drawn.get() {
            d.consumed = true;
            return d;
        }
        let (col_lo, col_hi) = (s.panel_right() - 320, s.panel_right() - 88);
        let in_col = col_lo < p.x && p.x < col_hi;
        if view.free_points() < 1 && in_col {
            let hits: Vec<u16> = self
                .tab_skills(&s, view)
                .filter(|&(e, at)| icon_hit(at, p) && self.pressed != Some(e.skill) && !e.passive)
                .map(|(e, _)| e.skill)
                .collect();
            for skill in hits {
                d.out.push(PanelOutput::Intent(ClientIntent::from_message(
                    &SelectSkill {
                        skill: u32::from(skill),
                        left: false,
                        item: u32::MAX,
                    },
                )));
                d.out.push(PanelOutput::SetUi {
                    ui: UI_SKILLTREE,
                    mode: 1,
                    jump: false,
                });
            }
            d.consumed = true;
        }
        if col_hi <= p.x && p.x <= s.panel_right() {
            d.consumed = true;
            if let Some(t) = tab_at(&s, p) {
                if t != self.tab {
                    self.tab = t;
                    // Id 6.
                    d.out.push(PanelOutput::Sound(6));
                }
            }
        }
        if view.class().is_some_and(|c| close_hit(&s, c, self.tab, p)) {
            self.close_pressed = true;
            // Id 4.
            d.out.push(PanelOutput::Sound(4));
        }
        if in_col {
            d.consumed = true;
            if view.free_points() >= 1 {
                let hit = self
                    .tab_skills(&s, view)
                    .find(|&(e, at)| icon_hit(at, p) && self.pressed != Some(e.skill))
                    .map(|(e, _)| e.skill);
                if let Some(skill) = hit {
                    self.pressed = Some(skill);
                    d.cursor_press = true;
                    // Id 5.
                    d.out.push(PanelOutput::Sound(5));
                }
            }
        }
        d
    }

    /// Mouse up with the default context: see
    /// [`SkillTreePanel::mouse_up_ctx`].
    pub fn mouse_up(&mut self, env: &PanelEnv, view: &dyn SkillTreeView, p: Point) -> Release {
        self.mouse_up_ctx(env, view, p, InputCtx::DEFAULT)
    }

    /// Mouse up: no player → nothing; over the belt → nothing more; no draw yet
    /// → input reset, consumed. Close pressed → pressed:= 0, a release in the
    /// close rectangle → `SetUIState(4, toggle, 0)`; consumed, done (icon flags
    /// untouched). x ≤ `W / 2` → not consumed. Points are re-checked: base stat
    /// 5 ≤ 0 → no flag cleared, consumed. Else the walk: the hit icon with
    /// pressed = 1 → the learn checks, C→S 0x3B, flags:= 0, consumed, done;
    /// every other visited icon gets pressed:= 0.
    pub fn mouse_up_ctx(
        &mut self,
        env: &PanelEnv,
        view: &dyn SkillTreeView,
        p: Point,
        ctx: InputCtx,
    ) -> Release {
        let s = env.screen;
        let mut r = Release::default();
        if !ctx.has_player || ctx.over_belt {
            return r;
        }
        if !self.drawn.get() {
            r.consumed = true;
            return r;
        }
        if self.close_pressed {
            self.close_pressed = false;
            if view.class().is_some_and(|c| close_hit(&s, c, self.tab, p)) {
                r.out.push(PanelOutput::SetUi {
                    ui: UI_SKILLTREE,
                    mode: 2,
                    jump: false,
                });
            }
            r.consumed = true;
            return r;
        }
        if p.x <= s.right_start() {
            return r;
        }
        r.consumed = true;
        if view.free_points() <= 0 {
            return r;
        }
        let pressed = self.pressed;
        let walk: Vec<(u16, bool, bool, bool)> = self
            .tab_skills(&s, view)
            .map(|(e, at)| (e.skill, icon_hit(at, p), e.req_level_ok, e.below_max))
            .collect();
        for (skill, hit, req_ok, below) in walk {
            if hit && pressed == Some(skill) {
                if req_ok && below {
                    r.out.push(PanelOutput::Intent(ClientIntent::from_message(
                        &AddSkillPoint { skill },
                    )));
                }
                self.pressed = None;
                return r;
            }
        }
        self.pressed = None;
        r
    }

    /// The free-points number, only with base stat
    /// 5 ≥ 1: `%i` at (`W − sx − 52`, `H + sy − 400`), Font16, color 1
    /// when the player has state 54 (`uninterruptable`) or the panel's lock flag
    /// ≠ 0 (only ever written 0), else 0.
    pub fn free_points_number(
        &self,
        s: &Screen,
        view: &dyn SkillTreeView,
        uninterruptable: bool,
    ) -> Option<(String, Point, u16, u16)> {
        let n = view.free_points();
        (n >= 1).then(|| {
            (
                n.to_string(),
                Point::new(s.panel_right() - 52, s.h + s.sy() - 400),
                FONT16,
                u16::from(uninterruptable),
            )
        })
    }
}

/// The input context of a press or release.
#[derive(Clone, Copy, Debug)]
pub struct InputCtx {
    /// The mouse is over the belt.
    pub over_belt: bool,
    pub has_player: bool,
}

impl InputCtx {
    pub const DEFAULT: InputCtx = InputCtx {
        over_belt: false,
        has_player: true,
    };
}

/// A press's result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Down {
    pub consumed: bool,
    pub out: Vec<PanelOutput>,
    /// An icon was pressed: the cursor press runs.
    pub cursor_press: bool,
}

/// A tab tool tip: the region top `T`, the string id and the
/// rectangle and text positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabTip {
    pub top: i32,
    pub string: u16,
    /// A filled rectangle (W − sx − 89, T + 5, W − sx + 1, T + 25), colour
    /// (0, 0, 0) nearest, mode 6.
    pub rect: (i32, i32, i32, i32),
    /// Text at (`W − sx − 89`, `T + 20`), color 0, left-aligned.
    pub text_at: Point,
}

/// Current tab `t`, current mouse: only for x in
/// [`W − sx − 89`, `W − sx + 1`].
pub fn tab_tool_tip(s: &Screen, tab: u8, m: Point) -> Option<TabTip> {
    let right = s.panel_right();
    if m.x < right - 89 || m.x > right + 1 {
        return None;
    }
    let b = s.h + s.sy();
    let in_closed = |lo: i32, hi: i32| lo <= m.y && m.y <= hi;
    let (top, string) = match tab {
        1 => {
            if in_closed(b - 264, b - 156) {
                (b - 264, 4225)
            } else if in_closed(b - 372, b - 265) {
                (b - 372, 4224)
            } else {
                return None;
            }
        }
        2 => {
            if in_closed(b - 372, b - 264) {
                (b - 372, 4224)
            } else if in_closed(b - 156, b - 48) {
                (b - 156, 4226)
            } else {
                return None;
            }
        }
        3 => {
            if in_closed(b - 264, b - 156) {
                (b - 264, 4225)
            } else if in_closed(b - 155, b - 48) {
                (b - 156, 4226)
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(TabTip {
        top,
        string,
        rect: (right - 89, top + 5, right + 1, top + 25),
        text_at: Point::new(right - 89, top + 20),
    })
}

/// The close tool tip `strClose` position: (`X + 15`, `H + sy −
/// 98`), centered.
pub fn close_tool_tip_at(s: &Screen, class: u8, tab: u8) -> Option<Point> {
    let at = close_pos(s, class, tab)?;
    Some(Point::new(at.x + 15, s.h + s.sy() - 98))
}

/// The panel's draw order.
pub const DRAW_ORDER: [&str; 8] = [
    "font 16, drawn := 1",
    "background frames 0-3 then 4t..4t+3",
    "tab captions",
    "free-points number (points >= 1) then every class skill of the tab",
    "icons only (no points)",
    "close button",
    "close tool tip (queued)",
    "tab tool tip (at once)",
];

/// Level number: drawn when level > 0 or hard points ≠ 0; `%d` in
/// Font16 at (`X + 48`, `Y + 12`), FontFormal10 at x − 4 for ≥ 10; color 3
/// when the bonus part > 0, 1 when < 0, else 0.
fn level_text(e: &SkillEntry, at: Point) -> Option<UiDraw> {
    if e.level <= 0 && e.hard_points == 0 {
        return None;
    }
    let (font, dx) = if e.level >= 10 {
        (FONT_FORMAL10, -4)
    } else {
        (FONT16, 0)
    };
    let color = match e.bonus.signum() {
        1 => 3,
        -1 => 1,
        _ => 0,
    };
    Some(text(
        utf16(&e.level.to_string()),
        at.x + 48 + dx,
        at.y + 12,
        font,
        color,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::{ImageRef, ImageRequest, TextRequest};

    const ICON_FILE: u32 = 9000;

    struct View {
        class: Option<u8>,
        free: i32,
        mask: u8,
        skills: Vec<SkillEntry>,
    }

    impl SkillTreeView for View {
        fn class(&self) -> Option<u8> {
            self.class
        }
        fn icon_file(&self) -> Option<u32> {
            Some(ICON_FILE)
        }
        fn free_points(&self) -> i32 {
            self.free
        }
        fn flag_mask(&self) -> u8 {
            self.mask
        }
        fn skills(&self) -> &[SkillEntry] {
            &self.skills
        }
    }

    fn entry(skill: u16, page: u8, column: u8, row: u8) -> SkillEntry {
        SkillEntry {
            skill,
            page,
            row,
            column,
            icon_cel: 20,
            level: 0,
            hard_points: 0,
            bonus: 0,
            flags: 1,
            learnable: true,
            passive: false,
            req_level_ok: true,
            below_max: true,
        }
    }

    fn view(skills: Vec<SkillEntry>) -> View {
        View {
            class: Some(0),
            free: 1,
            mask: 1,
            skills,
        }
    }

    fn env(screen: Screen) -> PanelEnv {
        PanelEnv {
            screen,
            open_mode: 1,
            exp: true,
        }
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

    fn tables() -> PanelTables {
        PanelTables::load().expect("panel tables")
    }

    // Test vector "amazon skill tree tab 1, 800 × 600": frames 0–3 then
    // 4t … 4t + 3 as right quads (400, 316) …, class letter `a`.
    #[test]
    fn art_frames_per_tab_amazon_800() {
        let t = tables();
        let e = env(Screen::R800);
        let file = t.files.id("spells\\skltree_a_back").expect("amazon art");
        let quads = [(400, 316), (656, 316), (400, 492), (656, 492)];
        for tab in 1..=3u32 {
            let mut p = SkillTreePanel::new();
            p.tab = tab as u8;
            let v = view(Vec::new());
            let mut out: Vec<UiDraw> = Vec::new();
            p.draw(&t, &e, &v, Point::new(0, 0), &mut out);
            let im = images(&out);
            let mut want = Vec::new();
            for (i, q) in quads.iter().enumerate() {
                want.push((file, i as u32, q.0, q.1));
            }
            for (i, q) in quads.iter().enumerate() {
                want.push((file, 4 * tab + i as u32, q.0, q.1));
            }
            assert_eq!(im[..8], want[..], "tab {tab}");
        }
        // Another class substitutes its letter (sorceress `s`).
        let s_file = t.files.id("spells\\skltree_s_back").unwrap();
        let mut v = view(Vec::new());
        v.class = Some(1);
        let mut out: Vec<UiDraw> = Vec::new();
        SkillTreePanel::new().draw(&t, &env(Screen::R640), &v, Point::new(0, 0), &mut out);
        let im = images(&out);
        assert_eq!(im[0], (s_file, 0, 320, 256));
        assert_eq!(im[7], (s_file, 7, 576, 432));
    }

    // Test vector "close button at (571, 477)"; o per tab; hover/release
    // rectangle; release toggles ui 4 with jump 0.
    #[test]
    fn close_button_amazon() {
        let t = tables();
        let e = env(Screen::R800);
        let close = t.files.id(CLOSE_FILE).unwrap();
        let v = view(Vec::new());
        let mut p = SkillTreePanel::new();
        let mut out: Vec<UiDraw> = Vec::new();
        p.draw(&t, &e, &v, Point::new(0, 0), &mut out);
        assert_eq!(*images(&out).last().unwrap(), (close, 10, 571, 477));
        assert_eq!(close_pos(&Screen::R800, 0, 2), Some(Point::new(500, 477)));
        assert_eq!(close_pos(&Screen::R800, 0, 3), Some(Point::new(415, 477)));
        assert_eq!(close_pos(&Screen::R640, 0, 1), Some(Point::new(491, 417)));
        // Rectangle [571, 605] × [443, 477].
        for (x, y, inside) in [
            (571, 443, true),
            (605, 477, true),
            (570, 460, false),
            (606, 460, false),
            (580, 442, false),
            (580, 478, false),
        ] {
            assert_eq!(
                close_hit(&Screen::R800, 0, 1, Point::new(x, y)),
                inside,
                "({x},{y})"
            );
        }
        // Press draws frame 11; release inside toggles.
        let mut v0 = view(Vec::new());
        v0.free = 0;
        assert_eq!(
            p.mouse_down(&e, &v0, Point::new(580, 460)),
            vec![PanelOutput::Sound(4)]
        );
        assert!(p.close_pressed);
        let mut out: Vec<UiDraw> = Vec::new();
        p.draw(&t, &e, &v0, Point::new(0, 0), &mut out);
        assert_eq!(*images(&out).last().unwrap(), (close, 11, 571, 477));
        let r = p.mouse_up(&e, &v0, Point::new(580, 460));
        assert!(r.consumed);
        assert_eq!(
            r.out,
            vec![PanelOutput::SetUi {
                ui: 4,
                mode: 2,
                jump: false
            }]
        );
        assert!(!p.close_pressed);
        // Release outside: nothing.
        assert!(p.mouse_up(&e, &v0, Point::new(620, 460)).out.is_empty());
    }

    #[test]
    fn close_offsets_of_every_class() {
        let want: [[i32; 3]; 7] = [
            [-149, -220, -305],
            [-305, -305, -149],
            [-305, -149, -305],
            [-305, -220, -305],
            [-149, -305, -149],
            [-149, -149, -149],
            [-220, -149, -305],
        ];
        for (class, row) in want.iter().enumerate() {
            for (i, &o) in row.iter().enumerate() {
                assert_eq!(close_offset(class as u8, i as u8 + 1), Some(o));
                assert_eq!(
                    close_variant(o),
                    Some([1, 2, 3][[-149, -220, -305].iter().position(|&x| x == o).unwrap()])
                );
            }
        }
        assert_eq!(close_offset(7, 1), None);
        assert_eq!(close_offset(0, 0), None);
        assert_eq!(close_offset(0, 4), None);
        assert_eq!(close_variant(0), None);
        // sorceress tab 3 at 800 × 600: (571, 477), variant 1 (test vector)
        assert_eq!(close_pos(&Screen::R800, 1, 3), Some(Point::new(571, 477)));
        assert_eq!(close_offset(1, 3).and_then(close_variant), Some(1));
        // druid, any tab: x = W − sx − 149
        assert_eq!(
            close_pos(&Screen::R800, 5, 2),
            Some(Point::new(800 - 80 - 149, 477))
        );
        let t = tables();
        let mut v = view(Vec::new());
        v.class = Some(3);
        let mut out: Vec<UiDraw> = Vec::new();
        SkillTreePanel::new().draw(&t, &env(Screen::R800), &v, Point::new(0, 0), &mut out);
        let close = t.files.id(CLOSE_FILE).unwrap();
        // paladin tab 1: o = −305
        assert!(images(&out).contains(&(close, 10, 800 - 80 - 305, 477)));
    }

    // Test vector "skill in column 2, row 3 at 800 × 600: icon at
    // (484, 258); hit 485–531 × 211–257" (icon position, strict hit).
    #[test]
    fn icon_position_and_strict_hit() {
        let s = Screen::R800;
        assert_eq!(icon_pos(&s, 2, 3), Some(Point::new(484, 258)));
        let at = Point::new(484, 258);
        for x in 480..540 {
            for y in 205..265 {
                let want = (485..=531).contains(&x) && (211..=257).contains(&y);
                assert_eq!(icon_hit(at, Point::new(x, y)), want, "({x},{y})");
            }
        }
        // Every column / row against the layout table's draw and hit rows.
        let t = tables();
        for c in 1..=3u8 {
            for r in 1..=6u8 {
                let name = format!("icon_c{c}_r{r}");
                let draw = t
                    .item(PanelKey::Ui(4), &name, crate::ui::layout::RowKind::Draw)
                    .next()
                    .unwrap();
                let hit = t
                    .item(PanelKey::Ui(4), &name, crate::ui::layout::RowKind::Hit)
                    .next()
                    .unwrap();
                for scr in [Screen::R640, Screen::R800] {
                    let at = icon_pos(&scr, c, r).unwrap();
                    assert_eq!((at.x, at.y), (draw.x.eval(&scr), draw.y.eval(&scr)));
                    let rect = hit.hit_rect(&scr).unwrap();
                    assert!(icon_hit(at, rect.origin()));
                    assert!(!icon_hit(at, Point::new(rect.x - 1, rect.y)));
                    assert!(!icon_hit(at, Point::new(rect.x, rect.y - 1)));
                    let last = Point::new(rect.right() as i32 - 1, rect.bottom() as i32 - 1);
                    assert!(icon_hit(at, last));
                    assert!(!icon_hit(at, Point::new(last.x + 1, last.y)));
                    assert!(!icon_hit(at, Point::new(last.x, last.y + 1)));
                }
            }
        }
        assert_eq!(icon_pos(&s, 0, 1), None);
        assert_eq!(icon_pos(&s, 4, 1), None);
        assert_eq!(icon_pos(&s, 1, 7), None);
    }

    // Only skills of the current page are drawn; frame IconCel (+1 pressed).
    #[test]
    fn icons_of_current_tab() {
        let s = Screen::R800;
        let v = view(vec![
            entry(6, 1, 2, 3),
            entry(7, 2, 1, 1),
            entry(8, 0, 1, 1),
        ]);
        let mut p = SkillTreePanel::new();
        let ic = p.icons(&s, &v, Point::new(0, 0));
        assert_eq!(ic.len(), 1);
        assert_eq!(ic[0].skill, 6);
        assert_eq!(
            images(std::slice::from_ref(&ic[0].image)),
            vec![(ICON_FILE, 20, 484, 258)]
        );
        p.pressed = Some(6);
        let ic = p.icons(&s, &v, Point::new(0, 0));
        assert_eq!(
            images(std::slice::from_ref(&ic[0].image)),
            vec![(ICON_FILE, 21, 484, 258)]
        );
        p.tab = 2;
        let ic = p.icons(&s, &v, Point::new(0, 0));
        assert_eq!(ic.len(), 1);
        assert_eq!(ic[0].skill, 7);
    }

    #[test]
    fn icon_draw_carries_its_remap() {
        let s = Screen::R800;
        let v = view(vec![entry(6, 1, 2, 3)]);
        let p = SkillTreePanel::new();
        let out = Point::new(0, 0);
        let ic = p.icons(&s, &v, out);
        let want = crate::ui::Remap::Palette(i32::from(ic[0].remap));
        let UiDraw::Image(r) = &ic[0].image else {
            panic!("image")
        };
        assert_eq!((r.look.mode, r.look.remap), (5, want));
        // Mouse strictly inside the icon: k = 3.
        let ic = p.icons(&s, &v, Point::new(500, 240));
        let UiDraw::Image(r) = &ic[0].image else {
            panic!("image")
        };
        assert_eq!(ic[0].remap, 3);
        assert_eq!(r.look.remap, crate::ui::Remap::Palette(3));
    }

    #[test]
    fn level_number_font_position_color() {
        let at = Point::new(484, 258);
        let mut e = entry(6, 1, 2, 3);
        assert_eq!(level_text(&e, at), None);
        e.hard_points = 1;
        e.level = 0;
        let d = level_text(&e, at).unwrap();
        let t = &texts(std::slice::from_ref(&d))[0].clone();
        assert_eq!(
            (t.at.x, t.at.y, t.style.font, t.style.color),
            (532, 270, FONT16, 0)
        );
        assert_eq!(t.text, utf16("0"));
        e.hard_points = 0;
        e.level = 9;
        e.bonus = 2;
        let d = level_text(&e, at).unwrap();
        let t = &texts(std::slice::from_ref(&d))[0].clone();
        assert_eq!((t.at.x, t.style.font, t.style.color), (532, FONT16, 3));
        assert_eq!(t.text, utf16("9"));
        e.level = 10;
        e.bonus = -1;
        let d = level_text(&e, at).unwrap();
        let t = &texts(std::slice::from_ref(&d))[0].clone();
        assert_eq!(
            (t.at.x, t.at.y, t.style.font, t.style.color),
            (528, 270, FONT_FORMAL10, 1)
        );
        assert_eq!(t.text, utf16("10"));
    }

    #[test]
    fn tab_mouse_down_bounds() {
        for s in [Screen::R640, Screen::R800] {
            let r = s.panel_right();
            let b = s.h + s.sy();
            let tab = |x: i32, y: i32| tab_at(&s, Point::new(x, y));
            assert_eq!(tab(r - 88, b - 371), Some(3));
            // The band's last column, or the frame's when the band reaches
            // past it (at 640 the right panel ends at the frame's edge).
            assert_eq!(tab(r.min(s.frame_right() - 1), b - 266), Some(3));
            assert_eq!(tab(r - 89, b - 300), None);
            assert_eq!(tab(r + 1, b - 300), None);
            assert_eq!(tab(r - 40, b - 372), None);
            assert_eq!(tab(r - 40, b - 265), None);
            assert_eq!(tab(r - 40, b - 264), None);
            assert_eq!(tab(r - 40, b - 263), Some(2));
            assert_eq!(tab(r - 40, b - 158), Some(2));
            assert_eq!(tab(r - 40, b - 157), None);
            assert_eq!(tab(r - 40, b - 156), None);
            assert_eq!(tab(r - 40, b - 155), Some(1));
            // Tab-1 bottom ignores sy: last row H − 50.
            assert_eq!(tab(r - 40, s.h - 50), Some(1));
            assert_eq!(tab(r - 40, s.h - 49), None);
        }
        // Every band stays inside the frame. In the wide frame the right
        // panel ends at the frame's last column, so the band ends there too
        // (88 columns) and its clicks are unchanged.
        for s in [Screen::R640, Screen::R800, Screen::WIDE] {
            for t in 1..=3 {
                let r = tab_rect(&s, t).expect("band");
                assert!(
                    r.x >= s.frame_left() && r.right() <= i64::from(s.frame_right()),
                    "{s:?} tab {t}: {r:?}"
                );
            }
        }
        let s = Screen::WIDE;
        let b = s.h + s.sy();
        assert_eq!(tab_rect(&s, 3).map(|r| r.w), Some(88));
        assert_eq!(
            tab_at(&s, Point::new(s.frame_right() - 1, b - 300)),
            Some(3)
        );
        assert_eq!(tab_at(&s, Point::new(s.frame_right() - 89, b - 300)), None);
        // my ≤ H − 48 gate (at 640 the bands end above it anyway).
        let s = Screen::R800;
        assert_eq!(tab_at(&s, Point::new(700, 553)), None);
        // A change plays the click sound and sends nothing; same tab: nothing.
        let e = env(s);
        let v = view(Vec::new());
        let mut p = SkillTreePanel::new();
        p.drawn.set(true);
        assert_eq!(
            p.mouse_down(&e, &v, Point::new(700, 200)),
            vec![PanelOutput::Sound(6)]
        );
        assert_eq!(p.tab, 3);
        assert!(p.mouse_down(&e, &v, Point::new(700, 200)).is_empty());
        assert_eq!(
            p.mouse_down(&e, &v, Point::new(700, 500)),
            vec![PanelOutput::Sound(6)]
        );
        assert_eq!(p.tab, 1);
    }

    // With free points: press marks, release on it sends 0x3B.
    #[test]
    fn icon_press_release_sends_add_skill_point() {
        let e = env(Screen::R800);
        let v = view(vec![entry(0x1A, 1, 2, 3)]);
        let mut p = SkillTreePanel::new();
        p.drawn.set(true);
        assert_eq!(
            p.mouse_down(&e, &v, Point::new(500, 230)),
            vec![PanelOutput::Sound(5)]
        );
        assert_eq!(p.pressed, Some(0x1A));
        let r = p.mouse_up(&e, &v, Point::new(510, 240));
        assert!(r.consumed);
        assert_eq!(
            r.out,
            vec![PanelOutput::Intent(ClientIntent(vec![0x3B, 0x1A, 0x00]))]
        );
        assert_eq!(p.pressed, None);
        // Release off the icon clears pressed and sends nothing.
        p.mouse_down(&e, &v, Point::new(500, 230));
        let r = p.mouse_up(&e, &v, Point::new(600, 230));
        assert!(r.out.is_empty());
        assert_eq!(p.pressed, None);
        // Release without a press sends nothing.
        assert!(p.mouse_up(&e, &v, Point::new(500, 230)).out.is_empty());
        // Required level / max level checks.
        for (req, below) in [(false, true), (true, false)] {
            let mut v2 = view(vec![entry(0x1A, 1, 2, 3)]);
            v2.skills[0].req_level_ok = req;
            v2.skills[0].below_max = below;
            p.mouse_down(&e, &v2, Point::new(500, 230));
            assert_eq!(p.pressed, Some(0x1A));
            assert!(p.mouse_up(&e, &v2, Point::new(500, 230)).out.is_empty());
        }
        // No free points: no press; the no-points path selects
        // the skill for the right hand and closes the tree.
        let mut v0 = view(vec![entry(0x1A, 1, 2, 3)]);
        v0.free = 0;
        assert_eq!(
            p.mouse_down(&e, &v0, Point::new(500, 230)),
            vec![
                PanelOutput::Intent(ClientIntent(vec![
                    0x3C, 0x1A, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF
                ])),
                PanelOutput::SetUi {
                    ui: 4,
                    mode: 1,
                    jump: false
                },
            ]
        );
        assert_eq!(p.pressed, None);
        // Icons of another tab are not hit.
        let v3 = view(vec![entry(0x1A, 2, 2, 3)]);
        p.mouse_down(&e, &v3, Point::new(500, 230));
        assert_eq!(p.pressed, None);
    }

    #[test]
    fn release_left_half_not_consumed() {
        let e = env(Screen::R800);
        let v = view(Vec::new());
        let mut p = SkillTreePanel::new();
        p.drawn.set(true);
        assert!(!p.mouse_up(&e, &v, Point::new(400, 300)).consumed);
        assert!(!p.mouse_up(&e, &v, Point::new(0, 300)).consumed);
        assert!(p.mouse_up(&e, &v, Point::new(401, 300)).consumed);
        let e = env(Screen::R640);
        assert!(!p.mouse_up(&e, &v, Point::new(320, 300)).consumed);
        assert!(p.mouse_up(&e, &v, Point::new(321, 300)).consumed);
    }

    // Remap order, literal (unverified).
    #[test]
    fn icon_remap_literal_order() {
        let at = Point::new(484, 258);
        let out = Point::new(0, 0);
        let inside = Point::new(500, 230);
        let mut v = view(vec![]);
        let p = SkillTreePanel::new();
        let mut e = entry(6, 1, 2, 3);
        assert_eq!(p.icon_remap(&v, &e, at, out), 0);
        e.learnable = false;
        assert_eq!(p.icon_remap(&v, &e, at, out), 5);
        assert_eq!(p.icon_remap(&v, &e, at, inside), 3);
        e.flags = 2;
        assert_eq!(p.icon_remap(&v, &e, at, inside), 1);
        e.flags = 1;
        e.level = 1;
        assert_eq!(p.icon_remap(&v, &e, at, out), 0);
        v.free = 0;
        assert_eq!(p.icon_remap(&v, &e, at, out), 3);
        e.flags = 2;
        assert_eq!(p.icon_remap(&v, &e, at, out), 3);
        let mut pp = SkillTreePanel::new();
        pp.pressed = Some(6);
        assert_eq!(pp.icon_remap(&v, &e, at, out), 1);
    }

    fn drawn_panel() -> SkillTreePanel {
        let p = SkillTreePanel::new();
        p.drawn.set(true);
        p
    }

    #[test]
    fn mouse_down_in_the_spec_order() {
        let e = env(Screen::R800);
        let ctx = |over_belt, has_player| InputCtx {
            over_belt,
            has_player,
        };
        let v = view(vec![entry(0x1A, 1, 2, 3)]);
        let mut p = drawn_panel();
        // not consumed over the belt, without a player, or y > H − 48
        let at = Point::new(500, 230);
        for c in [ctx(true, true), ctx(false, false)] {
            assert_eq!(p.mouse_down_ctx(&e, &v, at, c), Down::default());
        }
        assert_eq!(
            p.mouse_down_ctx(&e, &v, Point::new(500, 553), InputCtx::DEFAULT),
            Down::default()
        );
        // 1. no draw yet: consumed, done
        let mut fresh = SkillTreePanel::new();
        let d = fresh.mouse_down_ctx(&e, &v, at, InputCtx::DEFAULT);
        assert_eq!((d.consumed, d.out.len()), (true, 0));
        assert_eq!(fresh.pressed, None);
        // 6. outside the column and the tab strip: not consumed
        let d = p.mouse_down_ctx(&e, &v, Point::new(100, 300), InputCtx::DEFAULT);
        assert!(!d.consumed && d.out.is_empty());
        // 3. a tab press is consumed even where no tab band is hit
        let d = p.mouse_down_ctx(&e, &v, Point::new(700, 10), InputCtx::DEFAULT);
        assert!(d.consumed && d.out.is_empty());
        // 5. an empty spot of the column with points: consumed, no press
        let d = p.mouse_down_ctx(&e, &v, Point::new(450, 100), InputCtx::DEFAULT);
        assert!(d.consumed && d.out.is_empty() && p.pressed.is_none());
        // 4 + 5: one press sets both the close flag and the column-3 /
        // row-6 icon where they overlap (o = −149)
        let ov = view(vec![entry(0x2B, 1, 3, 6)]);
        let d = p.mouse_down_ctx(&e, &ov, Point::new(580, 450), InputCtx::DEFAULT);
        assert!(d.consumed);
        assert!(p.close_pressed);
        assert_eq!(p.pressed, Some(0x2B));
        assert_eq!(d.out, vec![PanelOutput::Sound(4), PanelOutput::Sound(5)]);
        // an already pressed icon is not pressed again
        let mut q = drawn_panel();
        q.pressed = Some(0x1A);
        let d = q.mouse_down_ctx(&e, &v, at, InputCtx::DEFAULT);
        assert!(d.consumed && d.out.is_empty());
        // 2. no points: the right skill is chosen and the tree closes for
        // every hit non-passive, not pressed icon; passive ones are not
        let mut v0 = view(vec![entry(0x1A, 1, 2, 3), entry(0x1B, 1, 2, 3)]);
        v0.free = 0;
        v0.skills[1].passive = true;
        let mut q = drawn_panel();
        let d = q.mouse_down_ctx(&e, &v0, at, InputCtx::DEFAULT);
        assert!(d.consumed);
        assert_eq!(d.out.len(), 2);
        assert_eq!(
            d.out[0],
            PanelOutput::Intent(ClientIntent(vec![
                0x3C, 0x1A, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF
            ]))
        );
        assert_eq!(
            d.out[1],
            PanelOutput::SetUi {
                ui: 4,
                mode: 1,
                jump: false
            }
        );
        // the skill id has bit 31 clear (right hand)
        assert_eq!(((0x1A_u32 | (1 << 31)) >> 31), 1);
        assert_eq!(q.pressed, None);
        // a pressed icon is skipped by the no-points path
        let mut q = drawn_panel();
        q.pressed = Some(0x1A);
        assert!(q
            .mouse_down_ctx(&e, &v0, at, InputCtx::DEFAULT)
            .out
            .is_empty());
    }

    #[test]
    fn mouse_up_in_the_spec_order() {
        let e = env(Screen::R800);
        let v = view(vec![entry(0x1A, 1, 2, 3), entry(0x1B, 1, 1, 1)]);
        let at = Point::new(500, 230);
        // no player / over the belt: nothing
        for c in [
            InputCtx {
                over_belt: false,
                has_player: false,
            },
            InputCtx {
                over_belt: true,
                has_player: true,
            },
        ] {
            let mut p = drawn_panel();
            p.pressed = Some(0x1A);
            assert_eq!(p.mouse_up_ctx(&e, &v, at, c), Release::default());
            assert_eq!(p.pressed, Some(0x1A));
        }
        // no draw yet: consumed
        let mut fresh = SkillTreePanel::new();
        assert!(fresh.mouse_up(&e, &v, at).consumed);
        // close pressed: cleared; a release in the rectangle toggles ui 4;
        // consumed, done, the icon flags untouched
        let mut p = drawn_panel();
        p.close_pressed = true;
        p.pressed = Some(0x1A);
        let r = p.mouse_up(&e, &v, Point::new(580, 460));
        assert!(r.consumed);
        assert_eq!(
            r.out,
            vec![PanelOutput::SetUi {
                ui: 4,
                mode: 2,
                jump: false
            }]
        );
        assert_eq!(p.pressed, Some(0x1A));
        assert!(!p.close_pressed);
        // ... a close release even in the left half is consumed
        let mut p = drawn_panel();
        p.close_pressed = true;
        let r = p.mouse_up(&e, &v, Point::new(10, 10));
        assert!(r.consumed && r.out.is_empty());
        // x ≤ W / 2: not consumed
        let mut p = drawn_panel();
        assert!(!p.mouse_up(&e, &v, Point::new(400, 230)).consumed);
        // points re-checked: none left → no flag cleared, consumed
        let mut v0 = view(vec![entry(0x1A, 1, 2, 3)]);
        v0.free = 0;
        let mut p = drawn_panel();
        p.pressed = Some(0x1A);
        let r = p.mouse_up(&e, &v0, at);
        assert!(r.consumed && r.out.is_empty());
        assert_eq!(p.pressed, Some(0x1A));
        // the walk: the hit pressed icon sends 0x3B and clears its flags
        let mut p = drawn_panel();
        p.pressed = Some(0x1A);
        let r = p.mouse_up(&e, &v, at);
        assert_eq!(
            r.out,
            vec![PanelOutput::Intent(ClientIntent(vec![0x3B, 0x1A, 0]))]
        );
        assert_eq!(p.pressed, None);
        // a pressed icon released elsewhere: cleared, nothing sent
        let mut p = drawn_panel();
        p.pressed = Some(0x1A);
        let r = p.mouse_up(&e, &v, Point::new(450, 100));
        assert!(r.consumed && r.out.is_empty());
        assert_eq!(p.pressed, None);
    }

    #[test]
    fn numbers_tool_tips_and_draw_order() {
        let s = Screen::R800;
        let v = view(Vec::new());
        let p = SkillTreePanel::new();
        // free points: only with ≥ 1; Font16; color 1 with uninterruptable
        let n = p.free_points_number(&s, &v, false).unwrap();
        assert_eq!(
            n,
            ("1".to_string(), Point::new(800 - 80 - 52, 540 - 400), 1, 0)
        );
        assert_eq!(p.free_points_number(&s, &v, true).unwrap().3, 1);
        let mut v0 = view(Vec::new());
        v0.free = 0;
        assert!(p.free_points_number(&s, &v0, false).is_none());
        // tab tool tips (b = 540, right = 720)
        let tip = |tab, x, y| tab_tool_tip(&s, tab, Point::new(x, y));
        let t = tip(1, 700, 300).unwrap();
        assert_eq!((t.top, t.string), (276, 4225));
        assert_eq!(t.rect, (631, 281, 721, 301));
        assert_eq!(t.text_at, Point::new(631, 296));
        assert_eq!(
            tip(1, 700, 200).map(|t| (t.top, t.string)),
            Some((168, 4224))
        );
        assert_eq!(tip(1, 700, 450), None);
        assert_eq!(
            tip(2, 700, 200).map(|t| (t.top, t.string)),
            Some((168, 4224))
        );
        assert_eq!(
            tip(2, 700, 400).map(|t| (t.top, t.string)),
            Some((384, 4226))
        );
        assert_eq!(tip(2, 700, 300), None);
        assert_eq!(
            tip(3, 700, 300).map(|t| (t.top, t.string)),
            Some((276, 4225))
        );
        assert_eq!(
            tip(3, 700, 400).map(|t| (t.top, t.string)),
            Some((384, 4226))
        );
        assert_eq!(tip(3, 700, 200), None);
        // x in [W − sx − 89, W − sx + 1] only
        assert!(tip(1, 631, 300).is_some() && tip(1, 721, 300).is_some());
        assert!(tip(1, 630, 300).is_none() && tip(1, 722, 300).is_none());
        // the close tool tip is queued at (X + 15, H + sy − 98)
        assert_eq!(close_tool_tip_at(&s, 0, 1), Some(Point::new(586, 442)));
        assert_eq!(close_tool_tip_at(&s, 9, 1), None);
        // draw order
        assert_eq!(DRAW_ORDER.len(), 8);
        assert!(DRAW_ORDER[5].starts_with("close button"));
        assert!(DRAW_ORDER[7].starts_with("tab tool tip"));
    }
}
