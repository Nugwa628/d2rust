//! Widgets: plain structs with integer rects that emit draw
//! requests and answer hit tests. No retained GPU state.
//!
//! Hit tests are by rect. Whether the game tests a control's rect or
//! its opaque pixels, and every size, position and frame a widget uses,
//! come from the owning panel through the caller.

use super::draw::{ImageRef, ImageRequest, TextRequest, TextStyle, UiDraw, UiDrawSink};
use super::geom::{Point, Rect};
use super::layout::Screen;
use super::panel::WidgetId;
use super::text::{width_a, width_b, GlyphLookup, TextError, TextOpts};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WidgetError {
    #[error("widget {0:?}: zero size")]
    ZeroSize(WidgetId),
    #[error("grid {0:?}: rect overflows i32")]
    TooLarge(WidgetId),
}

/// What every widget answers.
pub trait Widget {
    fn id(&self) -> WidgetId;
    fn rect(&self) -> Rect;
    fn draw(&self, out: &mut dyn UiDrawSink);
    fn hit(&self, p: Point) -> Option<WidgetId> {
        self.rect().contains(p).then_some(self.id())
    }
}

/// A clickable rect with an optional image. The image changes only with the
/// pressed flag, never on hover (e.g. close buttons frame 10 released / 11
/// pressed): `pressed_image` (when set) is drawn while `pressed`, else `image`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub id: WidgetId,
    pub rect: Rect,
    pub image: Option<ImageRef>,
    pub pressed_image: Option<ImageRef>,
    pub pressed: bool,
}

impl Button {
    /// The image drawn now: the pressed frame while pressed.
    pub fn current_image(&self) -> Option<ImageRef> {
        if self.pressed {
            self.pressed_image.or(self.image)
        } else {
            self.image
        }
    }
}

impl Widget for Button {
    fn id(&self) -> WidgetId {
        self.id
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn draw(&self, out: &mut dyn UiDrawSink) {
        if let Some(image) = self.current_image() {
            out.push(UiDraw::Image(ImageRequest {
                image,
                at: self.rect.origin(),
                clip: Screen::play().rect(),
                look: crate::ui::CelLook::PLAIN,
                call: crate::ui::draw::CelCall::Draw,
            }));
        }
    }
}

/// A static frame image (panel art) over a rect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameImage {
    pub id: WidgetId,
    pub rect: Rect,
    pub image: ImageRef,
}

impl Widget for FrameImage {
    fn id(&self) -> WidgetId {
        self.id
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn draw(&self, out: &mut dyn UiDrawSink) {
        out.push(UiDraw::Image(ImageRequest {
            image: self.image,
            at: self.rect.origin(),
            clip: Screen::play().rect(),
            look: crate::ui::CelLook::PLAIN,
            call: crate::ui::draw::CelCall::Draw,
        }));
    }
}

/// A text label. The pen is never the rect origin: each text rule gives it, y
/// being the bottom row of the glyph cell and x given or centered over a span.
/// `rect` is only the hit / clip area.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    pub id: WidgetId,
    pub rect: Rect,
    /// The pen (x, y) of the text rule.
    pub pen: Point,
    pub text: Vec<u16>,
    pub style: TextStyle,
}

impl Widget for Label {
    fn id(&self) -> WidgetId {
        self.id
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn draw(&self, out: &mut dyn UiDrawSink) {
        out.push(UiDraw::Text(TextRequest {
            text: self.text.clone(),
            at: self.pen,
            style: self.style,
            opts: TextOpts::default(),
            clip: Screen::play().rect(),
        }));
    }
}

/// A grid cell, column and row from the top-left.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    pub col: u16,
    pub row: u16,
}

/// A grid of equal cells (inventory, stash, cube, belt), from a grid layout
/// record (gridX × gridY cells of cellW × cellH at
/// (left, top)): cells are adjacent (pitch = cell size) and have no art of
/// their own (the grid lines are panel background), so the grid draws nothing
/// itself; the tints and item graphics are drawn by the grid's owner from
/// [`CellGrid::footprint`] and [`CellGrid::item_draw_point`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellGrid {
    id: WidgetId,
    origin: Point,
    cols: u16,
    rows: u16,
    cell_w: u16,
    cell_h: u16,
}

impl CellGrid {
    pub fn new(
        id: WidgetId,
        origin: Point,
        cols: u16,
        rows: u16,
        cell_w: u16,
        cell_h: u16,
    ) -> Result<Self, WidgetError> {
        if cols == 0 || rows == 0 || cell_w == 0 || cell_h == 0 {
            return Err(WidgetError::ZeroSize(id));
        }
        let w = u32::from(cols) * u32::from(cell_w);
        let h = u32::from(rows) * u32::from(cell_h);
        // Every pixel of the grid must be an i32 point, so cell rects and
        // hit math stay in range.
        let past_end = |o: i32, size: u32| i64::from(o) + i64::from(size) > i64::from(i32::MAX) + 1;
        if w > u32::from(u16::MAX)
            || h > u32::from(u16::MAX)
            || past_end(origin.x, w)
            || past_end(origin.y, h)
        {
            return Err(WidgetError::TooLarge(id));
        }
        Ok(Self {
            id,
            origin,
            cols,
            rows,
            cell_w,
            cell_h,
        })
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// The cell under `p` (integer division).
    pub fn cell_at(&self, p: Point) -> Option<Cell> {
        if !self.rect().contains(p) {
            return None;
        }
        Some(Cell {
            col: ((p.x - self.origin.x) / i32::from(self.cell_w)) as u16,
            row: ((p.y - self.origin.y) / i32::from(self.cell_h)) as u16,
        })
    }

    /// The cell under `p` (no cursor item):
    /// c = (mouseX − left) / cellW, r = (mouseY − top) / cellH, unsigned
    /// division of the wrapped difference (a point left of or above the
    /// grid gives a huge, out-of-grid value). The pair is returned as is;
    /// [`CellGrid::cell_at`] is the in-grid test.
    pub fn mouse_cell(&self, p: Point) -> (u32, u32) {
        let c = (p.x.wrapping_sub(self.origin.x) as u32) / u32::from(self.cell_w);
        let r = (p.y.wrapping_sub(self.origin.y) as u32) / u32::from(self.cell_h);
        (c, r)
    }

    /// The cells of a w × h footprint at (c, r) that get a tint (a cell is
    /// tinted only when its top-left corner is inside [0, clipW) × [0, clipH)),
    /// in row-major order.
    pub fn footprint(&self, c: u16, r: u16, w: u16, h: u16, clip_w: i32, clip_h: i32) -> Vec<Rect> {
        let mut out = Vec::new();
        for dr in 0..h {
            for dc in 0..w {
                let x = self.origin.x + i32::from(c + dc) * i32::from(self.cell_w);
                let y = self.origin.y + i32::from(r + dr) * i32::from(self.cell_h);
                if (0..clip_w).contains(&x) && (0..clip_h).contains(&y) {
                    out.push(Rect::new(x, y, self.cell_w, self.cell_h));
                }
            }
        }
        out
    }

    /// The hover anchor of an item at (c, r) of w × h cells: x = left + cellW ·
    /// c + (w · cellW) / 2, top = top + cellH · r, bottom = top + cellH · (r +
    /// h).
    pub fn hover_anchor(&self, c: u16, r: u16, w: u16, h: u16) -> (i32, i32, i32) {
        let cw = i32::from(self.cell_w);
        let ch = i32::from(self.cell_h);
        let x = self.origin.x + cw * i32::from(c) + (i32::from(w) * cw) / 2;
        let top = self.origin.y + ch * i32::from(r);
        let bottom = self.origin.y + ch * (i32::from(r) + i32::from(h));
        (x, top, bottom)
    }

    /// The cursor cell of a w × h cursor item whose inventory graphic is
    /// gw × gh. `None`: the handler returns
    /// without change (the footprint would pass the grid's right or
    /// bottom edge); the caller keeps its previous cursor cell.
    pub fn cursor_cell(&self, p: Point, w: u16, h: u16, gw: u32, gh: u32) -> Option<(i32, i32)> {
        let (c, r) = self.drop_cell(p, w, h, gw, gh);
        if w > 1 && i64::from(w) + i64::from(c) > i64::from(self.cols) {
            return None;
        }
        if h > 1 && i64::from(h) + i64::from(r) > i64::from(self.rows) {
            return None;
        }
        Some((c, r))
    }

    /// The grid drop cell: the
    /// Cursor-cell formula recomputed from the click's mouse, without
    /// the overflow returns (an out-of-grid footprint then fails the
    /// placement test, so no 0x18 is sent).
    pub fn drop_cell(&self, p: Point, w: u16, h: u16, gw: u32, gh: u32) -> (i32, i32) {
        let (cw, ch) = (u32::from(self.cell_w), u32::from(self.cell_h));
        let (mut c, mut r) = self.mouse_cell(p);
        let left = self.origin.x as u32;
        let top = self.origin.y as u32;
        if w.is_multiple_of(2) {
            c = (gw >> 2).wrapping_sub(left).wrapping_add(p.x as u32) / cw;
        }
        if h.is_multiple_of(2) {
            r = (gh >> 2).wrapping_sub(top).wrapping_add(p.y as u32) / ch;
        }
        if w == self.cols {
            c = u32::from(self.cols >> 1);
        }
        if h == self.rows {
            r = u32::from(self.rows >> 1);
        }
        let mut c = c as i32;
        let mut r = r as i32;
        if w > 1 {
            c -= i32::from(w >> 1);
            if c < 0 {
                c = 0;
            }
        }
        if h > 1 {
            r -= i32::from(h >> 1);
            if r < 0 {
                r = 0;
            }
        }
        (c, r)
    }

    /// The cel draw point of an item graphic whose footprint's top-left
    /// cell is (c, r): (x, top + h),
    /// h = the frame height, so the frame's top-left sits at the cell
    /// corner for offsets 0.
    pub fn item_draw_point(&self, c: u16, r: u16, frame_h: u32) -> Point {
        let x = self.origin.x + i32::from(c) * i32::from(self.cell_w);
        let top = self.origin.y + i32::from(r) * i32::from(self.cell_h);
        Point::new(x, top + frame_h as i32)
    }

    /// A cell's rect; `None` outside the grid.
    pub fn cell_rect(&self, c: Cell) -> Option<Rect> {
        (c.col < self.cols && c.row < self.rows).then(|| {
            Rect::new(
                self.origin.x + i32::from(c.col) * i32::from(self.cell_w),
                self.origin.y + i32::from(c.row) * i32::from(self.cell_h),
                self.cell_w,
                self.cell_h,
            )
        })
    }
}

impl Widget for CellGrid {
    fn id(&self) -> WidgetId {
        self.id
    }
    fn rect(&self) -> Rect {
        Rect::new(
            self.origin.x,
            self.origin.y,
            self.cols * self.cell_w,
            self.rows * self.cell_h,
        )
    }
    /// Nothing: cells have no art.
    fn draw(&self, _out: &mut dyn UiDrawSink) {}
}

/// A vertical list of equal rows showing `rows_visible` rows from `first`. The
/// owning panel draws the rows; the list answers which row is under a point and
/// keeps `first` in range. No original-UI panel scrolls with the mouse wheel: a
/// wheel event moves the list by [`ScrollList::WHEEL_STEP`] = 0 rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrollList {
    id: WidgetId,
    rect: Rect,
    row_h: u16,
    len: u32,
    first: u32,
}

impl ScrollList {
    pub fn new(id: WidgetId, rect: Rect, row_h: u16) -> Result<Self, WidgetError> {
        if row_h == 0 || rect.h < row_h || rect.w == 0 {
            return Err(WidgetError::ZeroSize(id));
        }
        Ok(Self {
            id,
            rect,
            row_h,
            len: 0,
            first: 0,
        })
    }

    pub fn rows_visible(&self) -> u32 {
        u32::from(self.rect.h / self.row_h)
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn first(&self) -> u32 {
        self.first
    }

    fn max_first(&self) -> u32 {
        self.len.saturating_sub(self.rows_visible())
    }

    /// Sets the row count; `first` is clamped to the new range.
    pub fn set_len(&mut self, len: u32) {
        self.len = len;
        self.first = self.first.min(self.max_first());
    }

    /// Rows one wheel event scrolls an original-UI list.
    pub const WHEEL_STEP: i64 = 0;

    /// A mouse-wheel event: moves by [`Self::WHEEL_STEP`] rows
    /// per event whatever the delta, so the list does not move.
    pub fn wheel(&mut self, delta: i32) {
        self.scroll(Self::WHEEL_STEP * i64::from(delta.signum()));
    }

    /// Moves `first` by `rows` (negative: up), clamped to the range.
    pub fn scroll(&mut self, rows: i64) {
        let f = i64::from(self.first)
            .saturating_add(rows)
            .clamp(0, i64::from(self.max_first()));
        self.first = f as u32;
    }

    /// The list row under `p`; `None` past the last row.
    pub fn row_at(&self, p: Point) -> Option<u32> {
        if !self.rect.contains(p) {
            return None;
        }
        let r = ((p.y - self.rect.y) / i32::from(self.row_h)) as u32;
        (r < self.rows_visible() && self.first + r < self.len).then_some(self.first + r)
    }
}

impl Widget for ScrollList {
    fn id(&self) -> WidgetId {
        self.id
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn draw(&self, _out: &mut dyn UiDrawSink) {}
}

/// A single-line text field holding UTF-16 code units. Which characters
/// are accepted and the maximum length come from the owner of the field
/// (chat: not settled). The caret is
/// ([`TextInput::draw_caret`]); the text pointer is the end of the text
/// (this field only appends and deletes at the end) and the field keeps
/// no selection (the selection applies only to boxes with E +0x00 = 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInput {
    pub id: WidgetId,
    pub rect: Rect,
    pub style: TextStyle,
    max_len: usize,
    text: Vec<u16>,
}

impl TextInput {
    pub fn new(id: WidgetId, rect: Rect, style: TextStyle, max_len: usize) -> Self {
        Self {
            id,
            rect,
            style,
            max_len,
            text: Vec::new(),
        }
    }

    pub fn text(&self) -> &[u16] {
        &self.text
    }

    /// Appends one code unit; `false` (unchanged) when full.
    pub fn insert(&mut self, c: u16) -> bool {
        if self.text.len() >= self.max_len {
            return false;
        }
        self.text.push(c);
        true
    }

    /// Removes the last code unit; `false` when empty.
    pub fn backspace(&mut self) -> bool {
        self.text.pop().is_some()
    }

    /// Returns the text and empties the field.
    pub fn take(&mut self) -> Vec<u16> {
        std::mem::take(&mut self.text)
    }

    /// The caret glyph `_`.
    pub const CARET: u16 = 0x5F;

    /// Whether the caret blinks on at host time `tick_ms`
    /// (`GetTickCount`): visible when focused and `tick_ms / 1000`
    /// is odd (1 s on, 1 s off; client-only wall clock).
    pub fn caret_visible(focused: bool, tick_ms: u32) -> bool {
        focused && (tick_ms / 1000) % 2 == 1
    }

    /// The caret draw, after [`Widget::draw`]: `_` at x +
    /// width of the units before the text pointer (width B), x for
    /// an empty text; nothing when the blink is off.
    pub fn draw_caret(
        &self,
        g: &GlyphLookup<'_>,
        focused: bool,
        tick_ms: u32,
        out: &mut dyn UiDrawSink,
    ) -> Result<(), TextError> {
        if !Self::caret_visible(focused, tick_ms) {
            return Ok(());
        }
        let before = width_b(g, &self.text, self.text.len())?;
        let at = self.rect.origin();
        out.push(UiDraw::Text(TextRequest {
            text: vec![Self::CARET],
            at: Point::new(at.x + before, at.y),
            style: self.style,
            opts: TextOpts::default(),
            clip: Screen::play().rect(),
        }));
        Ok(())
    }

    /// The caret width `wc` (width A of `_`) and the line-fit test: a
    /// unit is added while `width(line) + wc` ≤ the inner width.
    pub fn fits(g: &GlyphLookup<'_>, line: &[u16], inner_w: i32) -> Result<bool, TextError> {
        let wc = width_a(g, &[Self::CARET])?;
        Ok(width_a(g, line)? + wc <= inner_w)
    }
}

impl Widget for TextInput {
    fn id(&self) -> WidgetId {
        self.id
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn draw(&self, out: &mut dyn UiDrawSink) {
        out.push(UiDraw::Text(TextRequest {
            text: self.text.clone(),
            at: self.rect.origin(),
            style: self.style,
            opts: TextOpts::default(),
            clip: Screen::play().rect(),
        }));
    }
}
