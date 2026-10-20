//! The panel machine tables: `ui-states.tsv`,
//! `panel-layout.tsv` and `npc-menus.tsv`, their position
//! grammar and conditions, and the screen layout model.
//!
//! The tables are our own spec data (not game files), embedded from
//! `specs/ui/`. Every parser is strict: an unknown word, a short row or a
//! bad number is an error, never a default.

use super::geom::Rect;

/// `tables/ui/ui-states.tsv`.
pub const UI_STATES_TSV: &str = include_str!("../../../../tables/ui/ui-states.tsv");
/// `tables/ui/panel-layout.tsv`.
pub const PANEL_LAYOUT_TSV: &str = include_str!("../../../../tables/ui/panel-layout.tsv");
/// `tables/ui/npc-menus.tsv`.
pub const NPC_MENUS_TSV: &str = include_str!("../../../../tables/ui/npc-menus.tsv");

/// Number of UI states (0x26).
pub const UI_STATE_COUNT: usize = 38;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("{table}: bad header")]
    Header { table: &'static str },
    #[error("{table} line {line}: {what}")]
    Row {
        table: &'static str,
        line: usize,
        what: String,
    },
}

fn row_err(table: &'static str, line: usize, what: impl Into<String>) -> LayoutError {
    LayoutError::Row {
        table,
        line,
        what: what.into(),
    }
}

/// Splits a TSV into its data rows (1-based line number, cells); checks the
/// header and the cell count.
fn rows<'a>(
    table: &'static str,
    text: &'a str,
    header: &[&str],
) -> Result<Vec<(usize, Vec<&'a str>)>, LayoutError> {
    let mut lines = text.lines().enumerate();
    let head: Vec<&str> = lines
        .next()
        .map(|(_, l)| l.split('\t').collect())
        .unwrap_or_default();
    if head != header {
        return Err(LayoutError::Header { table });
    }
    let mut out = Vec::new();
    for (i, l) in lines {
        if l.is_empty() {
            continue;
        }
        let cells: Vec<&str> = l.split('\t').collect();
        if cells.len() != header.len() {
            return Err(row_err(table, i + 1, format!("{} cells", cells.len())));
        }
        out.push((i + 1, cells));
    }
    Ok(out)
}

fn parse_int(s: &str) -> Option<i64> {
    if let Some(h) = s.strip_prefix("0x") {
        i64::from_str_radix(h, 16).ok()
    } else {
        s.parse().ok()
    }
}

// ---------------------------------------------------------------------
// Screen layout model
// ---------------------------------------------------------------------

/// The frame size and resolution mode the panel positions depend on.
///
/// `w` is the native UI box width (800 or 640) and `h` is the committed
/// canvas height: 600 or 480 for the fixed frames, and the exact Integer
/// frame height after an adaptive frame is published. A frame wider than
/// the box centres it `left` pixels in, leaves an odd spare column in
/// `extra_right`, and the side panels open at the frame's own edges: left
/// panels at the frame's first column, right panels ending at its last,
/// with the world visible between them. Positions stay in box space, so a
/// point left of the box has a negative x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub w: i32,
    pub h: i32,
    /// D2GFX `ResolutionMode`: 0 (640 × 480) or 2 (800 × 600).
    pub res_mode: u8,
    /// Columns of frame left of the box: 0 at 4:3, 134 in the 16:9 frame
    /// ([`crate::rules::camera::FrameSize::ui_left`]).
    pub left: i32,
    /// The extra column right of a centred box in an odd-width frame.
    pub extra_right: i32,
}

/// Width of a side panel (the 320 × 432 panels).
pub const SIDE_PANEL_W: i32 = 320;

/// `ScreenShiftX` of the 800 × 600 layout: the left panel slot starts 80
/// columns in, and a stone border fills the 80 columns beside each slot.
const SX_800: i32 = 80;

impl Screen {
    pub const R640: Screen = Screen {
        w: 640,
        h: 480,
        res_mode: 0,
        left: 0,
        extra_right: 0,
    };
    pub const R800: Screen = Screen {
        w: 800,
        h: 600,
        res_mode: 2,
        left: 0,
        extra_right: 0,
    };
    /// The 800 × 600 layout centred in the 16:9 frame (1068 × 600).
    pub const WIDE: Screen = Screen {
        left: 134,
        ..Screen::R800
    };

    /// The UI geometry of a committed frame. Art stays at its native size;
    /// the HUD follows the bottom and side panels retain their own slots.
    pub fn for_frame(f: crate::rules::camera::FrameSize) -> Screen {
        if f == crate::rules::camera::FrameSize::LOW {
            Screen::R640
        } else {
            Screen {
                h: f.height,
                left: f.ui_left(),
                extra_right: (f.width - 800).max(0) % 2,
                ..Screen::R800
            }
        }
    }

    pub fn play() -> Screen {
        Self::for_frame(crate::rules::camera::FrameSize::play())
    }

    /// Preserve a pointer's frame pixel across changes to the centered UI box.
    /// Clamp retained coordinates when the new frame is smaller.
    pub fn pointer_from(&self, old: Screen, p: super::geom::Point) -> super::geom::Point {
        let x = (p.x + old.left).clamp(0, self.frame_right() + self.left - 1);
        super::geom::Point::new(x - self.left, p.y.clamp(0, self.h - 1))
    }

    /// The whole UI box as a rectangle.
    pub fn rect(&self) -> Rect {
        Rect::new(0, 0, self.w as u16, self.h as u16)
    }

    /// The whole frame in box space: [`Self::rect`] at 4:3; in a wider
    /// frame it also takes the `left` columns on each side of the box.
    /// What covers or catches the whole screen uses it.
    pub fn frame_rect(&self) -> Rect {
        self.frame_rect_h(self.h)
    }

    /// [`Self::frame_rect`] cut to its top `h` rows.
    pub fn frame_rect_h(&self, h: i32) -> Rect {
        let w = self.w + 2 * self.left + self.extra_right;
        Rect::new(-self.left, 0, w as u16, h.max(0) as u16)
    }

    /// The frame's first column in box space (0 at 4:3).
    pub fn frame_left(&self) -> i32 {
        -self.left
    }

    /// One past the frame's last column in box space (`w` at 4:3).
    pub fn frame_right(&self) -> i32 {
        self.w + self.left + self.extra_right
    }

    pub fn res2(&self) -> bool {
        self.res_mode == 2
    }

    /// `ScreenShiftX`: 80 at resolution mode 2, else 0. In a frame
    /// wider than the box it is −`left`, which puts the left panel slot at
    /// the frame's first column and the right slot against its last one;
    /// every left panel is laid out from `sx` and every right panel from
    /// `W − sx`, so both follow.
    pub fn sx(&self) -> i32 {
        if !self.res2() {
            0
        } else if self.left > 0 || self.extra_right > 0 {
            -self.left
        } else {
            SX_800
        }
    }

    /// How far a wider frame moves the left side's art and rectangles from
    /// their 800 × 600 places (the right side's move the same distance the
    /// other way): the left slot goes from x 80 to the frame's first
    /// column. 0 at 4:3.
    pub fn edge_dx(&self) -> i32 {
        if self.res2() && (self.left > 0 || self.extra_right > 0) {
            self.sx() - SX_800
        } else {
            0
        }
    }

    /// The move of [`Self::edge_dx`] for something laid out for the box at
    /// column `x`: left of the box's middle it goes with the left side,
    /// else with the right side.
    pub fn side_dx(&self, x: i32) -> i32 {
        if x < self.w / 2 {
            self.edge_dx()
        } else {
            -self.edge_dx() + self.extra_right
        }
    }

    /// One past the left panel side's last column: `W / 2` at 4:3 (the
    /// left slot and its border), the slot's right edge in a wider frame.
    pub fn left_end(&self) -> i32 {
        self.sx() + SIDE_PANEL_W
    }

    /// The left side's click area of the full-height left panels (stash,
    /// cube, shop, quest log): from the frame's first column through
    /// [`Self::left_end`] (`[0, W / 2]` at 4:3), the rows above the
    /// control panel (`H − 48`).
    pub fn left_side_rect(&self) -> Rect {
        let w = self.left_end() + 1 - self.frame_left();
        Rect::new(self.frame_left(), 0, w as u16, (self.h - 48) as u16)
    }

    /// The right panel side's first column: `W / 2` at 4:3, the right
    /// slot's left edge in a wider frame.
    pub fn right_start(&self) -> i32 {
        self.panel_right() - SIDE_PANEL_W
    }

    /// How far an open side panel moves the view, and the cursor with it:
    /// half the columns one panel side takes from the frame (`W / 4` at
    /// 4:3, 160 in the 16:9 frame). Equals half of
    /// [`crate::rules::camera::FrameSize::side_panel_span`].
    pub fn panel_shift(&self) -> i32 {
        (self.left_end() - self.frame_left()) / 2
    }

    /// Vertical translation of native 800x600 side-panel art and table boxes.
    pub fn panel_dy(&self) -> i32 {
        if self.res2() {
            self.h - 600
        } else {
            0
        }
    }

    /// Right edge of the right panel, including an odd canvas column.
    pub fn panel_right(&self) -> i32 {
        self.w + self.extra_right - self.sx()
    }

    /// `ScreenShiftY`: −60 at resolution mode 2, else 0.
    pub fn sy(&self) -> i32 {
        if self.res2() {
            -60
        } else {
            0
        }
    }

    /// Left edge `X0` of the left panel slot.
    pub fn left_x0(&self) -> i32 {
        self.sx()
    }

    /// Left edge `X0` of the right panel slot.
    pub fn right_x0(&self) -> i32 {
        self.panel_right() - 320
    }

    /// The four quad draw positions of a 320 × 432 panel with left edge
    /// `x0`, frames `f … f + 3` in this order.
    pub fn quads(&self, x0: i32) -> [(i32, i32); 4] {
        let top = self.h + self.sy() - 224;
        let bottom = self.h + self.sy() - 48;
        [(x0, top), (x0 + 256, top), (x0, bottom), (x0 + 256, bottom)]
    }
}

/// One term of a position expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Term {
    Int(i32),
    W,
    H,
    /// `W / 2`.
    W2,
    Sx,
    Sy,
}

/// A position expression: signed terms (the layout's `x`, `y`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr(pub Vec<(i32, Term)>);

impl Expr {
    pub fn parse(s: &str) -> Option<Expr> {
        if s.is_empty() || s.contains(' ') {
            return None;
        }
        let mut terms = Vec::new();
        let mut sign = 1;
        let mut cur = String::new();
        let mut push = |sign: i32, word: &str| -> Option<()> {
            let t = match word {
                "W" => Term::W,
                "H" => Term::H,
                "W2" => Term::W2,
                "sx" => Term::Sx,
                "sy" => Term::Sy,
                _ if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) => {
                    Term::Int(word.parse().ok()?)
                }
                _ => return None,
            };
            terms.push((sign, t));
            Some(())
        };
        for c in s.chars() {
            if c == '+' || c == '-' {
                if !cur.is_empty() {
                    push(sign, &cur)?;
                    cur.clear();
                } else {
                    // No leading sign and no `+-`: terms are joined by one sign.
                    return None;
                }
                sign = if c == '-' { -1 } else { 1 };
            } else {
                cur.push(c);
            }
        }
        push(sign, &cur)?;
        Some(Expr(terms))
    }

    pub fn eval(&self, s: &Screen) -> i32 {
        self.0
            .iter()
            .map(|&(sign, t)| {
                sign * match t {
                    Term::Int(v) => v,
                    Term::W => s.w + s.extra_right,
                    Term::H => s.h,
                    Term::W2 => s.w / 2,
                    Term::Sx => s.sx(),
                    Term::Sy => s.sy(),
                }
            })
            .sum()
    }
}

// ---------------------------------------------------------------------
// Ui-states.tsv
// ---------------------------------------------------------------------

/// Slot kind of a UI state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Right,
    Left,
    Full,
    Anvil,
    None,
}

/// The gate action `C[i][ui]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// 0: nothing.
    Ignore,
    /// 1: close `i`, continue.
    Close,
    /// 2: refuse.
    Refuse,
    /// 3: refuse; a fatal error when the request is ui 0.
    RefuseFatal0,
    /// 4: end an active NPC interaction, continue.
    EndNpc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiStateRow {
    pub id: u8,
    pub name: String,
    pub flag: u32,
    pub slot: SlotKind,
    pub exp_only: bool,
    /// `C[id][j]` for j = 0 … 37.
    pub conflicts: [Conflict; UI_STATE_COUNT],
}

/// The 38 rows of `ui-states.tsv`, index = id.
pub fn ui_states() -> Result<Vec<UiStateRow>, LayoutError> {
    parse_ui_states(UI_STATES_TSV)
}

pub fn parse_ui_states(text: &str) -> Result<Vec<UiStateRow>, LayoutError> {
    const T: &str = "ui-states.tsv";
    let mut out = Vec::new();
    for (line, c) in rows(
        T,
        text,
        &["id", "name", "flag", "slot", "exp_only", "conflicts"],
    )? {
        let id = c[0].parse::<u8>().map_err(|_| row_err(T, line, "id"))?;
        if usize::from(id) != out.len() {
            return Err(row_err(T, line, "ids not 0, 1, 2, …"));
        }
        let flag = parse_int(c[2])
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| row_err(T, line, "flag"))?;
        let slot = match c[3] {
            "right" => SlotKind::Right,
            "left" => SlotKind::Left,
            "full" => SlotKind::Full,
            "anvil" => SlotKind::Anvil,
            "none" => SlotKind::None,
            _ => return Err(row_err(T, line, "slot")),
        };
        let exp_only = match c[4] {
            "0" => false,
            "1" => true,
            _ => return Err(row_err(T, line, "exp_only")),
        };
        if c[5].len() != UI_STATE_COUNT {
            return Err(row_err(T, line, "conflicts length"));
        }
        let mut conflicts = [Conflict::Ignore; UI_STATE_COUNT];
        for (j, b) in c[5].bytes().enumerate() {
            conflicts[j] = match b {
                b'0' => Conflict::Ignore,
                b'1' => Conflict::Close,
                b'2' => Conflict::Refuse,
                b'3' => Conflict::RefuseFatal0,
                b'4' => Conflict::EndNpc,
                _ => return Err(row_err(T, line, "conflict digit")),
            };
        }
        out.push(UiStateRow {
            id,
            name: c[1].to_string(),
            flag,
            slot,
            exp_only,
            conflicts,
        });
    }
    if out.len() != UI_STATE_COUNT {
        return Err(row_err(T, out.len() + 1, "not 38 states"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// Panel-layout.tsv
// ---------------------------------------------------------------------

/// The `panel` column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PanelKey {
    Ui(u8),
    Border,
    CtrlPnl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Draw,
    Text,
    Hit,
}

/// The `frame` column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameSpec {
    None,
    /// A frame index (`draw`) or a string id (`text`).
    Index(u32),
    /// `a/b`: string alternatives (`text`).
    Alt(u32, u32),
    /// A word the panel resolves (`iconcel`, `tabstate`, `rowstate`,
    /// `value`, `level`, `tabstr`).
    Word(String),
}

/// The `color` column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorSpec {
    None,
    Index(u16),
    /// `cmp` or a state word (`tabstate`, `rowstate`).
    Word(String),
}

/// One `cond` word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cond {
    Always,
    Res2,
    ResNot2,
    ModeL,
    ModeR,
    Exp,
    Classic,
    Released,
    Pressed,
    StatPts,
    Tab(u8),
    SkillHere,
    Tab1Bottom,
    TabShown,
    TabVisible,
    RowUsed,
    RowKnown,
    Centered,
    SplitLf,
    HalfCentered,
}

impl Cond {
    fn parse(s: &str) -> Option<Cond> {
        Some(match s {
            "always" => Cond::Always,
            "res2" => Cond::Res2,
            "res_not2" => Cond::ResNot2,
            "mode_l" => Cond::ModeL,
            "mode_r" => Cond::ModeR,
            "exp" => Cond::Exp,
            "classic" => Cond::Classic,
            "released" => Cond::Released,
            "pressed" => Cond::Pressed,
            "statpts" => Cond::StatPts,
            "tab1" => Cond::Tab(1),
            "tab2" => Cond::Tab(2),
            "tab3" => Cond::Tab(3),
            "skill_here" => Cond::SkillHere,
            "tab1_bottom" => Cond::Tab1Bottom,
            "tab_shown" => Cond::TabShown,
            "tab_visible" => Cond::TabVisible,
            "row_used" => Cond::RowUsed,
            "row_known" => Cond::RowKnown,
            "centered" => Cond::Centered,
            "split_lf" => Cond::SplitLf,
            "half_centered" => Cond::HalfCentered,
            _ => return None,
        })
    }

    /// Words that say how text is placed, not whether a row applies.
    pub fn is_format(&self) -> bool {
        matches!(
            self,
            Cond::Always | Cond::Centered | Cond::SplitLf | Cond::HalfCentered | Cond::Tab1Bottom
        )
    }
}

/// The panel state a row's conditions are tested against. Panel-specific
/// words (`statpts`, `skill_here`, `row_used`, …) are answered by the
/// panel through `extra`.
pub struct CondEnv<'a> {
    pub screen: Screen,
    /// Screen open mode 0–3.
    pub open_mode: u8,
    /// Expansion game with the expansion installed.
    pub exp: bool,
    pub pressed: bool,
    pub extra: &'a dyn Fn(Cond) -> bool,
}

impl CondEnv<'_> {
    pub fn holds(&self, c: Cond) -> bool {
        match c {
            c if c.is_format() => true,
            Cond::Res2 => self.screen.res2(),
            Cond::ResNot2 => !self.screen.res2(),
            Cond::ModeL => matches!(self.open_mode, 2 | 3),
            Cond::ModeR => matches!(self.open_mode, 1 | 3),
            Cond::Exp => self.exp,
            Cond::Classic => !self.exp,
            Cond::Released => !self.pressed,
            Cond::Pressed => self.pressed,
            other => (self.extra)(other),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutRow {
    pub line: usize,
    pub panel: PanelKey,
    pub item: String,
    pub kind: RowKind,
    /// Path under `data\global\ui\` without extension (`draw`).
    pub file: Option<String>,
    pub frame: FrameSpec,
    pub x: Expr,
    pub y: Expr,
    pub w: Option<i32>,
    pub h: Option<i32>,
    pub color: ColorSpec,
    pub font: Option<u16>,
    pub cond: Vec<Cond>,
    pub addr: u32,
}

impl LayoutRow {
    pub fn has(&self, c: Cond) -> bool {
        self.cond.contains(&c)
    }

    pub fn applies(&self, env: &CondEnv) -> bool {
        self.cond.iter().all(|&c| env.holds(c))
    }

    /// The `hit` rectangle: `x … x + w − 1`, `y … y + h − 1`;
    /// with `tab1_bottom` the bottom row is `H − 50`.
    pub fn hit_rect(&self, s: &Screen) -> Option<Rect> {
        if self.kind != RowKind::Hit {
            return None;
        }
        let x = self.x.eval(s);
        let y = self.y.eval(s);
        let w = self.w?;
        let h = if self.has(Cond::Tab1Bottom) {
            s.h - 50 - y + 1
        } else {
            self.h?
        };
        Some(Rect::new(
            x,
            y,
            u16::try_from(w).ok()?,
            u16::try_from(h).ok()?,
        ))
    }
}

/// All rows of `panel-layout.tsv`, in file order.
pub fn panel_layout() -> Result<Vec<LayoutRow>, LayoutError> {
    parse_panel_layout(PANEL_LAYOUT_TSV)
}

pub fn parse_panel_layout(text: &str) -> Result<Vec<LayoutRow>, LayoutError> {
    const T: &str = "panel-layout.tsv";
    let header = [
        "panel", "item", "kind", "file", "frame", "x", "y", "w", "h", "color", "font", "cond",
        "addr",
    ];
    let mut out = Vec::new();
    for (line, c) in rows(T, text, &header)? {
        let e = |what: &str| row_err(T, line, what);
        let panel = match c[0] {
            "border" => PanelKey::Border,
            "ctrlpnl" => PanelKey::CtrlPnl,
            v => PanelKey::Ui(
                v.parse::<u8>()
                    .ok()
                    .filter(|&u| usize::from(u) < UI_STATE_COUNT)
                    .ok_or_else(|| e("panel"))?,
            ),
        };
        let kind = match c[2] {
            "draw" => RowKind::Draw,
            "text" => RowKind::Text,
            "hit" => RowKind::Hit,
            _ => return Err(e("kind")),
        };
        let file = match (kind, c[3]) {
            (RowKind::Draw, "-") => return Err(e("draw without file")),
            (RowKind::Draw, f) => Some(f.to_string()),
            (_, "-") => None,
            _ => return Err(e("file on a non-draw row")),
        };
        let frame = match c[4] {
            "-" => FrameSpec::None,
            v if v.bytes().all(|b| b.is_ascii_digit()) => {
                FrameSpec::Index(v.parse().map_err(|_| e("frame"))?)
            }
            v if v.contains('/') => {
                let (a, b) = v.split_once('/').ok_or_else(|| e("frame"))?;
                FrameSpec::Alt(
                    a.parse().map_err(|_| e("frame"))?,
                    b.parse().map_err(|_| e("frame"))?,
                )
            }
            v @ ("iconcel" | "tabstate" | "rowstate" | "value" | "level" | "tabstr") => {
                FrameSpec::Word(v.to_string())
            }
            _ => return Err(e("frame")),
        };
        let x = Expr::parse(c[5]).ok_or_else(|| e("x"))?;
        let y = Expr::parse(c[6]).ok_or_else(|| e("y"))?;
        let opt_int = |v: &str, what: &str| -> Result<Option<i32>, LayoutError> {
            match v {
                "-" => Ok(None),
                v => v.parse().map(Some).map_err(|_| e(what)),
            }
        };
        let w = opt_int(c[7], "w")?;
        let h = opt_int(c[8], "h")?;
        let color = match c[9] {
            "-" => ColorSpec::None,
            v if v.bytes().all(|b| b.is_ascii_digit()) => {
                ColorSpec::Index(v.parse().map_err(|_| e("color"))?)
            }
            v @ ("cmp" | "tabstate" | "rowstate") => ColorSpec::Word(v.to_string()),
            _ => return Err(e("color")),
        };
        let font = match c[10] {
            "-" => None,
            v => Some(v.parse().map_err(|_| e("font"))?),
        };
        let cond = c[11]
            .split(',')
            .map(|w| Cond::parse(w).ok_or_else(|| e("cond")))
            .collect::<Result<Vec<_>, _>>()?;
        let addr = parse_int(c[12])
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| e("addr"))?;
        if kind == RowKind::Hit
            && (w.is_none() || (h.is_none() && !cond.contains(&Cond::Tab1Bottom)))
        {
            return Err(e("hit without size"));
        }
        out.push(LayoutRow {
            line,
            panel,
            item: c[1].to_string(),
            kind,
            file,
            frame,
            x,
            y,
            w,
            h,
            color,
            font,
            cond,
            addr,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// Npc-menus.tsv
// ---------------------------------------------------------------------

/// The NPC menu option kinds (handler per kind).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OptionKind {
    Talk,
    Trade,
    Gamble,
    Hire,
    TravelWest,
    SailWest,
    Identify,
    Resurrect,
    /// D2RUST's own, unverified: Charsi's runtime insert, never parsed.
    Imbue,
}

impl OptionKind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "talk" => OptionKind::Talk,
            "trade" => OptionKind::Trade,
            "gamble" => OptionKind::Gamble,
            "hire" => OptionKind::Hire,
            "travel_west" => OptionKind::TravelWest,
            "sail_west" => OptionKind::SailWest,
            "identify" => OptionKind::Identify,
            "resurrect" => OptionKind::Resurrect,
            _ => return None,
        })
    }
}

/// One option slot: string id and handler kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuOption {
    pub string: u16,
    pub kind: OptionKind,
}

/// One static record of the option table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NpcMenuRecord {
    pub record: u8,
    pub npc: u32,
    /// Options + 1 for the trailing cancel.
    pub count: u32,
    /// Up to 5 slots; `None` = cleared.
    pub options: [Option<MenuOption>; 5],
    pub flag: u8,
}

/// The 48 static records of `npc-menus.tsv`.
pub fn npc_menus() -> Result<Vec<NpcMenuRecord>, LayoutError> {
    parse_npc_menus(NPC_MENUS_TSV)
}

pub fn parse_npc_menus(text: &str) -> Result<Vec<NpcMenuRecord>, LayoutError> {
    const T: &str = "npc-menus.tsv";
    let mut out = Vec::new();
    for (line, c) in rows(T, text, &["record", "npc", "count", "options", "flag"])? {
        let e = |what: &str| row_err(T, line, what);
        let record: u8 = c[0].parse().map_err(|_| e("record"))?;
        if usize::from(record) != out.len() {
            return Err(e("records not 0, 1, 2, …"));
        }
        let mut options = [None; 5];
        for (i, o) in c[3].split(',').enumerate() {
            let (s, k) = o.split_once(':').ok_or_else(|| e("option"))?;
            let slot = options.get_mut(i).ok_or_else(|| e("more than 5 options"))?;
            *slot = Some(MenuOption {
                string: s.parse().map_err(|_| e("option string"))?,
                kind: OptionKind::parse(k).ok_or_else(|| e("option kind"))?,
            });
        }
        out.push(NpcMenuRecord {
            record,
            npc: c[1].parse().map_err(|_| e("npc"))?,
            count: c[2].parse().map_err(|_| e("count"))?,
            options,
            flag: c[4].parse().map_err(|_| e("flag"))?,
        });
    }
    if out.len() != 48 {
        return Err(row_err(T, out.len() + 1, "not 48 records"));
    }
    Ok(out)
}

#[cfg(test)]
mod screen_tests {
    use super::*;

    /// At 4:3 the side helpers are the 800 × 600 and 640 × 480 rules: the
    /// left side ends and the right side starts at W / 2, the frame is the
    /// box, and a panel moves the view by W / 4.
    #[test]
    fn the_4_3_screens_keep_the_half_screen_sides() {
        for s in [Screen::R800, Screen::R640] {
            assert_eq!(s.left_end(), s.w / 2);
            assert_eq!(s.right_start(), s.w / 2);
            assert_eq!(s.frame_rect(), s.rect());
            assert_eq!((s.frame_left(), s.frame_right()), (0, s.w));
            assert_eq!(s.panel_shift(), s.w / 4);
            assert_eq!(s.edge_dx(), 0);
            assert_eq!((s.side_dx(10), s.side_dx(s.w - 10)), (0, 0));
            assert_eq!(
                s.left_side_rect(),
                Rect::new(0, 0, (s.w / 2 + 1) as u16, (s.h - 48) as u16)
            );
        }
        assert_eq!(Screen::R800.sx(), 80);
        assert_eq!(Screen::R640.sx(), 0);
    }

    /// In the 16:9 frame (box 134 columns in) the left slot starts at the
    /// frame's first column and the right slot ends at its last: in frame
    /// columns, 0..320 and 748..1068. Each side moves 214 columns out from
    /// its 800 × 600 place, and a panel moves the view by 160.
    #[test]
    fn the_wide_screen_opens_the_panels_at_the_frame_edges() {
        let s = Screen::WIDE;
        let frame = |x: i32| x + s.left;
        assert_eq!(frame(s.left_x0()), 0);
        assert_eq!(frame(s.right_x0()), 1068 - SIDE_PANEL_W);
        assert_eq!(frame(s.left_end()), 320);
        assert_eq!(frame(s.right_start()), 748);
        assert_eq!((frame(s.frame_left()), frame(s.frame_right())), (0, 1068));
        assert_eq!(s.frame_rect(), Rect::new(-134, 0, 1068, 600));
        assert_eq!(s.edge_dx(), -214);
        assert_eq!((s.side_dx(154), s.side_dx(419)), (-214, 214));
        assert_eq!(s.panel_shift(), 160);
        assert_eq!(
            s.panel_shift() * 2,
            crate::rules::camera::FrameSize::WIDE.side_panel_span()
        );
        // The layout rows follow: the character art (left) and the
        // inventory art (right) of the 800 × 600 layout, 214 columns out.
        let rows = panel_layout().unwrap();
        let x_of = |panel: PanelKey, s: &Screen| {
            let r = rows
                .iter()
                .find(|r| r.panel == panel && r.item == "art0" && r.kind == RowKind::Draw)
                .unwrap();
            r.x.eval(s)
        };
        assert_eq!(x_of(PanelKey::Ui(2), &Screen::R800), 80);
        assert_eq!(x_of(PanelKey::Ui(2), &s), -134);
        assert_eq!(x_of(PanelKey::Ui(1), &Screen::R800), 400);
        assert_eq!(x_of(PanelKey::Ui(1), &s), 614);
        // Rows anchored at the middle (the control panel) stay centred.
        let base1 = rows
            .iter()
            .find(|r| {
                r.panel == PanelKey::CtrlPnl
                    && r.item == "base1"
                    && r.file.as_deref() == Some("panel\\800ctrlpnl7")
            })
            .unwrap();
        assert_eq!(base1.x.eval(&s), base1.x.eval(&Screen::R800));
    }
}

#[cfg(test)]
mod adaptive_hud_tests {
    use super::*;
    use crate::rules::adaptive_integer::FrameSelection;
    use crate::rules::camera::FrameSize;
    use crate::ui::panels::control::buttons::{menu_button, menu_rect, run_button, run_rect};
    use crate::ui::panels::{PanelEnv, PanelTables};
    use crate::ui::ScaleMode;

    #[test]
    fn configured_fit_frames_keep_their_exact_ui_geometry() {
        for (frame, screen) in [
            (FrameSize::LOW, Screen::R640),
            (FrameSize::D2RUST, Screen::R800),
            (FrameSize::WIDE, Screen::WIDE),
        ] {
            assert_eq!(Screen::for_frame(frame), screen);
        }
    }

    #[test]
    fn matrix_hud_art_and_hit_regions_follow_the_same_bottom_edge() {
        let tables = PanelTables::load().unwrap();
        for (w, h) in [
            (1920, 1080),
            (3840, 2160),
            (2560, 1440),
            (1920, 1200),
            (2560, 1080),
            (3440, 1440),
            (5120, 2160),
            (1365, 767),
            (1919, 1079),
            (800, 1600),
            (7680, 2160),
            (801, 541),
        ] {
            let frame = FrameSelection::for_window(w, h, FrameSize::WIDE, ScaleMode::Integer).frame;
            let screen = Screen::for_frame(frame);
            assert_eq!(screen.h, frame.height);
            assert_eq!(i32::from(screen.frame_rect().w), frame.width);
            assert_eq!(screen.frame_right() - screen.frame_left(), frame.width);
            let run = run_button(screen.w, screen.h, false, false, (0, 0));
            let hit = run_rect(screen.w, screen.h);
            assert!(hit.contains(super::super::geom::Point::new(run.x, run.y)));
            let menu = menu_button(screen.w, screen.h, false, false, (0, 0));
            assert!(menu_rect(screen.w, screen.h)
                .contains(super::super::geom::Point::new(menu.x, menu.y)));
            let mut draws = Vec::new();
            crate::ui::panels::border::draw_border_and_ctrlpnl(
                &tables,
                &PanelEnv {
                    screen,
                    open_mode: 0,
                    exp: true,
                },
                &mut draws,
            );
            assert!(!draws.is_empty());
            for d in draws {
                if let crate::ui::UiDraw::Image(i) = d {
                    assert_eq!(i.at.y, frame.height);
                }
            }
        }
    }
}

#[cfg(test)]
mod adaptive_pointer_tests {
    use super::*;
    use crate::rules::camera::FrameSize;
    use crate::ui::geom::Point;

    #[test]
    fn pointer_frame_pixels_survive_adaptive_screen_changes() {
        let frames = [
            (960, 540),
            (801, 541),
            (800, 1600),
            (1280, 720),
            (1720, 720),
        ];
        for (ow, oh) in frames {
            let old = Screen::for_frame(FrameSize {
                width: ow,
                height: oh,
            });
            for (nw, nh) in frames {
                let new = Screen::for_frame(FrameSize {
                    width: nw,
                    height: nh,
                });
                for x in [-old.left, 0, old.w / 2, old.w - 1] {
                    for y in [0, old.h / 2, old.h - 1] {
                        let p = Point::new(x, y);
                        let q = new.pointer_from(old, p);
                        let want = (p.x + old.left).clamp(0, new.frame_right() + new.left - 1);
                        assert_eq!(q.x + new.left, want, "{old:?} {new:?} {p:?}");
                        assert_eq!(q.y, p.y.clamp(0, new.h - 1));
                    }
                }
            }
        }
    }
}
