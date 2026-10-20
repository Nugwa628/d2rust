//! The belt: belt type and record, the pop-up rows, the
//! slots, the cursor-item highlight, the hit area, the item hover text,
//! the mouse move, the box hit, the belt click and the hover tracking.
//! The belt record (`belts.bin`) and the items are plain inputs: the host
//! reads them from the client world.

use crate::ui::messages::chat::prefix;
use crate::ui::messages::{msg_u32s, LineDraw, RectDraw};
use crate::ui::panel::ClientIntent;

/// The default belt type (no belt item).
pub const DEFAULT_TYPE: u8 = 2;
/// `belts.txt` order: the `Expansion` row is not a record.
pub const BELT_NAMES: [&str; 7] = [
    "belt",
    "sash",
    "default",
    "girdle",
    "light belt",
    "heavy belt",
    "uber belt",
];
/// The item type of belts and the body location read.
pub const BELT_ITEM_TYPE: u32 = 19;
pub const BELT_BODY_LOCATION: u8 = 8;
/// Slot box size drawn.
pub const BOX_SIZE: i32 = 29;
/// Key labels are cut until width A ≤ 28.
pub const LABEL_MAX_W: i32 = 28;
/// The belt draw sets font 1 and never restores it: later
/// text in the frame without its own font uses it.
pub const FONT_AFTER_BELT: u16 = 1;
/// Hover name cut.
pub const HOVER_NAME_MAX: usize = 128;
/// Hover stat lines cut (the stat text buffer holds 0x100 units).
pub const HOVER_STATS_MAX: usize = 0x100;
/// The hover text buffer before the price (384 units).
pub const HOVER_TEXT_MAX: usize = 384;

/// What body location 8 holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeltSlot8 {
    /// No item there: type 2.
    Empty,
    /// An item of another item type: the type is left as it was.
    Other,
    /// A belt item.
    Belt(i32),
}

/// Fatal 0xB07: the belt row count of an item was negative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("belt item gives type {0} (fatal 0xB07)")]
pub struct BeltTypeError(pub i32);

/// One belt box: left, right, top, bottom (inclusive pixel edges).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeltBox {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

/// A `belts.bin` record: box count u8 +4, boxes from +8 (16 bytes each).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeltRecord {
    pub boxes: Vec<BeltBox>,
}

impl BeltRecord {
    /// Reads a record of 0x108 bytes (or the part holding its boxes).
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let n = usize::from(*b.get(4)?);
        let mut boxes = Vec::new();
        for i in 0..n {
            let o = 8 + 16 * i;
            let r = b.get(o..o + 16)?;
            let v = |k: usize| i32::from_le_bytes([r[k], r[k + 1], r[k + 2], r[k + 3]]);
            boxes.push(BeltBox {
                left: v(0),
                right: v(4),
                top: v(8),
                bottom: v(12),
            });
        }
        Some(Self { boxes })
    }
}

/// The record index: `resolution · 7 + type`.
pub fn record_index(resolution: u32, belt_type: u8) -> usize {
    resolution as usize * 7 + usize::from(belt_type)
}

/// The pop-up rows of a belt type: type 0 → rows 0, 1; 1 → row 0;
/// 2 → none; 3 → rows 0, 1, 2; 4 → row 0; 5 → rows 0, 1; other → rows 0,
/// 1, 2.
pub fn popup_rows(belt_type: u8) -> &'static [u8] {
    match belt_type {
        0 | 5 => &[0, 1],
        1 | 4 => &[0],
        2 => &[],
        _ => &[0, 1, 2],
    }
}

/// The row count: types 0–5: 3, 2, 1, 4, 2, 3; else
/// 4.
pub fn row_count(belt_type: u8) -> u8 {
    match belt_type {
        0 => 3,
        1 => 2,
        2 => 1,
        3 => 4,
        4 => 2,
        5 => 3,
        _ => 4,
    }
}

/// A belt colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeltColor {
    /// (128, 0, 0).
    Red,
    /// (0, 128, 0).
    Green,
    /// (0, 0, 128).
    Blue,
    /// (128, 128, 0).
    Yellow,
}

impl BeltColor {
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            BeltColor::Red => (128, 0, 0),
            BeltColor::Green => (0, 128, 0),
            BeltColor::Blue => (0, 0, 128),
            BeltColor::Yellow => (128, 128, 0),
        }
    }
}

/// What the belt knows of an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeltItem {
    pub guid: u32,
    /// = 0, the item is usable and
    /// = 0.
    pub usable: bool,
    /// ≠ 0 (the key label).
    pub has_use: bool,
    /// The item is blocked.
    pub blocked: bool,
    /// The item's x position in the belt grid.
    pub pos_x: usize,
    /// Quality 3.
    pub quality3: bool,
}

/// A belt slot and the key label of its binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotInfo {
    pub item: Option<BeltItem>,
    /// The key name bound to belt slot i (primary else secondary).
    pub key_name: Option<Vec<u16>>,
}

/// A draw of the belt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeltDraw {
    /// `ctrlpnl_popbelt` frame 0 at (x, y), light 0xFF, mode 5.
    PopRow { x: i32, y: i32 },
    /// A 29 × 29 rectangle.
    Box { rect: RectDraw, color: BeltColor },
    /// The belt item `guid` drawn with its top left at (x, y).
    Item { guid: u32, x: i32, y: i32 },
    /// The key label: DrawText at (left + 2, bottom − 2), color 4.
    Label(LineDraw),
}

/// The cursor, as the belt reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CursorInfo {
    /// The cursor mode (6 and 8 are special).
    pub mode: u8,
    /// The item on the cursor.
    pub item: Option<CursorItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorItem {
    pub guid: u32,
    /// Fits a belt.
    pub fits_belt: bool,
    /// A swap with the hovered belt item is possible.
    pub swap_ok: bool,
    /// Blocked.
    pub blocked: bool,
    /// The put sound id.
    pub put_sound: u32,
}

/// What a belt click asks of the rest of the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeltEffect {
    Send(ClientIntent),
    Call4c21f0(u32),
    Sound(u32),
}

/// The belt's state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeltState {
    /// The belt type of the worn belt (rows of 4 boxes).
    pub belt_type: u8,
    /// The pop-up flag.
    pub popped: bool,
    /// A second pop-up flag: it keeps the rows up when state 0x1F closes and
    /// gives boxes above 3 their hover text.
    pub ef9c: bool,
    /// The boxes above 3 are shown.
    pub extra_boxes: bool,
    /// The belt is hovered.
    pub hovered: bool,
    /// The hovered item.
    pub hover_item: Option<u32>,
    /// The last hovered item.
    pub last_item: Option<u32>,
    /// The hovered box, −1 for none.
    pub hover_box: i32,
    /// The hover text position.
    pub text_pos: (i32, i32),
}

impl Default for BeltState {
    fn default() -> Self {
        Self {
            belt_type: DEFAULT_TYPE,
            popped: false,
            ef9c: false,
            extra_boxes: false,
            hovered: false,
            hover_item: None,
            last_item: None,
            hover_box: -1,
            text_pos: (-1, -1),
        }
    }
}

impl BeltState {
    /// R1, each draw: the item in body location 8 of item type 19 gives
    /// (< 0 fatal 0xB07); no belt item → 2; an item of
    /// another type leaves it unchanged.
    pub fn update_type(&mut self, slot8: BeltSlot8) -> Result<(), BeltTypeError> {
        match slot8 {
            BeltSlot8::Empty => self.belt_type = DEFAULT_TYPE,
            BeltSlot8::Other => {}
            BeltSlot8::Belt(t) if t < 0 => return Err(BeltTypeError(t)),
            BeltSlot8::Belt(t) => self.belt_type = t as u8,
        }
        Ok(())
    }

    /// The pop-up flag is cleared at the belt draw when state 0x1F
    /// is closed and the second pop-up flag is clear, and whenever state 9 is open.
    pub fn clear_popped(&mut self, state_1f_open: bool, state_9_open: bool) {
        if (!state_1f_open && !self.ef9c) || state_9_open {
            self.popped = false;
            // The rows folded, so the extra-boxes flag reads 0 again for the mini
            // panel's sides: the belt draw sets it to 0 at every
            // draw before its popped branch (the mini panel then draws at
            // layout 2 after the belt key closed the rows).
            self.extra_boxes = false;
        }
    }

    /// The state 0x1F draw sets the pop-up flag.
    pub fn state_1f_draw(&mut self) {
        self.popped = true;
    }

    /// The pop-up rows, when popped: row k at `ctrlpnl_popbelt` frame 0 at (W/2 +
    /// 21, H − 41 − 32 k); the extra-boxes flag:= 0 for type 2, else 1.
    pub fn popup_draws(&mut self, w: i32, h: i32) -> Vec<BeltDraw> {
        if !self.popped {
            return Vec::new();
        }
        self.extra_boxes = self.belt_type != 2;
        popup_rows(self.belt_type)
            .iter()
            .map(|&k| BeltDraw::PopRow {
                x: w / 2 + 21,
                y: h - 41 - 32 * i32::from(k),
            })
            .collect()
    }

    /// The slots, boxes > 3 only while the extra-boxes flag is set.
    pub fn slot_draws(&self, rec: &BeltRecord, slots: &[SlotInfo]) -> Vec<BeltDraw> {
        let mut out = Vec::new();
        for (i, b) in rec.boxes.iter().enumerate() {
            if i > 3 && !self.extra_boxes {
                continue;
            }
            let Some(item) = slots.get(i).and_then(|s| s.item) else {
                continue;
            };
            let rect = |color| BeltDraw::Box {
                rect: RectDraw {
                    x: b.left,
                    y: b.top,
                    w: BOX_SIZE,
                    h: BOX_SIZE,
                    color: 0,
                    mode: 0,
                },
                color,
            };
            if item.usable {
                // When it is the hovered item and the belt is hovered: a
                // green rectangle; else none.
                if self.hover_item == Some(item.guid) && self.hovered {
                    out.push(rect(BeltColor::Green));
                }
            } else {
                out.push(rect(BeltColor::Red));
            }
            out.push(BeltDraw::Item {
                guid: item.guid,
                x: b.left,
                y: b.top,
            });
            if i <= 3 && item.has_use {
                if let Some(label) = slots.get(i).and_then(|s| s.key_name.clone()) {
                    out.push(BeltDraw::Label(LineDraw {
                        text: label,
                        x: b.left + 2,
                        y: b.bottom - 2,
                        color: 4,
                    }));
                }
            }
        }
        out
    }

    /// When the type is not 2 and
    /// state 0x1F is open or the belt is popped, with an item on the
    /// cursor and the belt hovered, the hovered box: empty and the item
    /// fits a belt → green; occupied and a swap is possible → yellow;
    /// else red; a 29 × 29 rectangle, mode 0.
    pub fn cursor_highlight(
        &self,
        rec: &BeltRecord,
        state_1f_open: bool,
        cursor: &CursorInfo,
        occupied: &dyn Fn(usize) -> bool,
    ) -> Option<BeltDraw> {
        if self.belt_type == 2 || !(state_1f_open || self.popped) {
            return None;
        }
        let c = cursor.item.as_ref()?;
        if !self.hovered {
            return None;
        }
        let i = usize::try_from(self.hover_box).ok()?;
        let b = rec.boxes.get(i)?;
        let color = if !occupied(i) && c.fits_belt {
            BeltColor::Green
        } else if occupied(i) && c.swap_ok {
            BeltColor::Yellow
        } else {
            BeltColor::Red
        };
        Some(BeltDraw::Box {
            rect: RectDraw {
                x: b.left,
                y: b.top,
                w: BOX_SIZE,
                h: BOX_SIZE,
                color: 0,
                mode: 0,
            },
            color,
        })
    }

    /// The hit area: popped: x from box (count −
    /// 4).left to box 3.right, y from box (count − 4).top to box
    /// 3.bottom; not popped: y > H − 48, x W/2 + 23 … W/2 + 145, y H − 39
    /// … H − 10 (inclusive).
    pub fn hit_area(&self, rec: &BeltRecord, w: i32, h: i32, x: i32, y: i32) -> bool {
        if self.popped {
            let n = rec.boxes.len();
            let (Some(a), Some(b)) = (
                n.checked_sub(4).and_then(|i| rec.boxes.get(i)),
                rec.boxes.get(3),
            ) else {
                return false;
            };
            (a.left..=b.right).contains(&x) && (a.top..=b.bottom).contains(&y)
        } else {
            strip(w, h, x, y)
        }
    }

    /// The first box i (0 … count − 1) with left ≤ x ≤ right and
    /// top ≤ y ≤ bottom (inclusive); none → −1 (`None`).
    pub fn box_hit(&self, rec: &BeltRecord, x: i32, y: i32) -> Option<usize> {
        rec.boxes
            .iter()
            .position(|b| (b.left..=b.right).contains(&x) && (b.top..=b.bottom).contains(&y))
    }

    /// R12, hover tracking: the box b under the mouse;
    /// the cursor state 6 or 8 → nothing. `slot_item(i)` is the item in
    /// belt slot i.
    pub fn hover_track(
        &mut self,
        rec: &BeltRecord,
        x: i32,
        y: i32,
        cursor: &CursorInfo,
        slot_item: &dyn Fn(usize) -> Option<BeltItem>,
    ) {
        let Some(b) = self.box_hit(rec, x, y) else {
            return;
        };
        if matches!(cursor.mode, 6 | 8) {
            return;
        }
        if cursor.item.is_some() {
            self.hover_box = b as i32;
            self.hovered = true;
            self.hover_item = None;
            self.last_item = None;
            return;
        }
        self.hover_box = b as i32;
        match slot_item(b) {
            None => {
                self.hovered = false;
                self.hover_item = None;
                self.last_item = None;
            }
            Some(e) => {
                self.hovered = true;
                if self.last_item != Some(e.guid) {
                    self.hover_item = Some(e.guid);
                    self.last_item = Some(e.guid);
                    if let Some(own) = rec.boxes.get(e.pos_x) {
                        self.text_pos = (own.left + 14, own.top);
                    }
                }
            }
        }
    }

    /// Hovered box and text position:= −1; belt hovered, hovered and last:= 0.
    pub fn reset_all(&mut self) {
        self.hover_box = -1;
        self.text_pos = (-1, -1);
        self.hovered = false;
        self.hover_item = None;
        self.last_item = None;
    }

    /// Hovered and last:= 0.
    pub fn reset_items(&mut self) {
        self.hover_item = None;
        self.last_item = None;
    }

    /// R9, the mouse move. Returns whether the event is
    /// consumed.
    #[allow(clippy::too_many_arguments)]
    pub fn mouse_move(
        &mut self,
        rec: &BeltRecord,
        w: i32,
        h: i32,
        x: i32,
        y: i32,
        gates: &MoveGates,
        cursor: &CursorInfo,
        slot_item: &dyn Fn(usize) -> Option<BeltItem>,
    ) -> bool {
        // Only in game, input not blocked, P alive, no modal input owner.
        if !(gates.in_game && !gates.input_blocked && gates.alive && !gates.x44bfe0) {
            return false;
        }
        if self.popped {
            if self.hit_area(rec, w, h, x, y) {
                self.hover_track(rec, x, y, cursor, slot_item);
                // Consumed while state 0x1F is open.
                return gates.state_1f_open;
            }
            self.ef9c = false;
            self.hovered = false;
            if !gates.state_1f_open {
                self.popped = false;
                self.extra_boxes = false; // as `clear_popped` (REC-1435)
            }
            return false;
        }
        if strip(w, h, x, y) {
            if !gates.state_9_open {
                self.popped = true;
                self.ef9c = true;
                self.hover_track(rec, x, y, cursor, slot_item);
            }
            return true;
        }
        self.hovered = false;
        false
    }

    /// R11, the belt click.
    pub fn click(
        &self,
        rec: &BeltRecord,
        x: i32,
        y: i32,
        cursor: &CursorInfo,
        slot_item: &dyn Fn(usize) -> Option<BeltItem>,
    ) -> Vec<BeltEffect> {
        let Some(b) = self.box_hit(rec, x, y) else {
            return Vec::new();
        };
        if cursor.mode == 6 {
            return Vec::new();
        }
        let e = slot_item(b);
        let mut out = Vec::new();
        match cursor.item {
            Some(c) if c.fits_belt => {
                if !c.blocked {
                    out.push(BeltEffect::Send(match e {
                        // 0x25 [c GUID][e GUID].
                        Some(e) => msg_u32s(0x25, &[c.guid, e.guid]),
                        // 0x23 [c GUID][b u32].
                        None => msg_u32s(0x23, &[c.guid, b as u32]),
                    }));
                    out.push(BeltEffect::Call4c21f0(c.guid));
                }
                // In every case of this branch the item's put sound.
                out.push(BeltEffect::Sound(c.put_sound));
            }
            None => {
                if let Some(e) = e.filter(|e| !e.blocked) {
                    out.push(BeltEffect::Send(msg_u32s(0x24, &[e.guid])));
                    out.push(BeltEffect::Call4c21f0(e.guid));
                }
            }
            // A cursor item that does not fit: only with `e`, no `c`.
            Some(_) => {}
        }
        out
    }
}

/// The inputs of the mouse move gates.
#[derive(Clone, Copy, Debug, Default)]
pub struct MoveGates {
    /// In a game.
    pub in_game: bool,
    /// ≠ 0.
    pub input_blocked: bool,
    pub alive: bool,
    /// ≠ 0.
    pub x44bfe0: bool,
    pub state_1f_open: bool,
    pub state_9_open: bool,
}

/// The strip when the belt is not popped: y > H − 48, x W/2 + 23 … W/2 +
/// 145, y H − 39 … H − 10 (inclusive).
fn strip(w: i32, h: i32, x: i32, y: i32) -> bool {
    y > h - 48 && (w / 2 + 23..=w / 2 + 145).contains(&x) && (h - 39..=h - 10).contains(&y)
}

/// The item hover text: with the belt hovered, a hovered item, no cursor
/// item and (box ≤ 3 or popped or `ef9c`), T = `Prefix(S, 3)` then
/// `Prefix(N, 0)`: the stat lines S (256 units; empty for quality 3, and an
/// empty part gets no colour code) then the name N (128 units), in a 384-unit
/// buffer; the shop sell price, when non-empty, appended after an LF (string
/// 3998); queued at, color 0, centred (positions must be ≥ 0). Drawn bottom-up
/// as the item tip pop-up, so the stat lines are the lower lines, the name above
/// them, the price on top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverText {
    pub text: Vec<u16>,
    pub x: i32,
    pub y: i32,
    pub color: u8,
    pub centered: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn hover_text(
    s: &BeltState,
    cursor_item: bool,
    item: Option<&BeltItem>,
    name: &[u16],
    stat_lines: &[u16],
    price: Option<&[u16]>,
) -> Option<HoverText> {
    let item = item?;
    let box_ok = s.hover_box >= 0 && (s.hover_box <= 3 || s.popped || s.ef9c);
    if !(s.hovered && s.hover_item.is_some() && !cursor_item && box_ok) {
        return None;
    }
    let name: Vec<u16> = name.iter().copied().take(HOVER_NAME_MAX).collect();
    let stats: Vec<u16> = if item.quality3 {
        Vec::new()
    } else {
        stat_lines.iter().copied().take(HOVER_STATS_MAX).collect()
    };
    let mut text = prefix(&stats, 3);
    text.extend(prefix(&name, 0));
    text.truncate(HOVER_TEXT_MAX);
    if let Some(p) = price.filter(|p| !p.is_empty()) {
        text.push(0x0A);
        text.extend_from_slice(p);
    }
    let (x, y) = s.text_pos;
    (x >= 0 && y >= 0).then_some(HoverText {
        text,
        x,
        y,
        color: 0,
        centered: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> BeltRecord {
        // 640 × 480 default-like record: boxes of 31 px at x 343, 374, 405,
        // 436 (row 0, y 440…470); rows above at y 409 and 378.
        let mut boxes = Vec::new();
        for row in 0..4 {
            for col in 0..4 {
                let left = 343 + 31 * col;
                let top = 440 - 31 * row;
                boxes.push(BeltBox {
                    left,
                    right: left + 30,
                    top,
                    bottom: top + 30,
                });
            }
        }
        BeltRecord { boxes }
    }

    fn item(guid: u32, pos: usize) -> BeltItem {
        BeltItem {
            guid,
            usable: true,
            has_use: true,
            blocked: false,
            pos_x: pos,
            quality3: false,
        }
    }

    fn items(v: Vec<(usize, BeltItem)>) -> impl Fn(usize) -> Option<BeltItem> {
        move |i| v.iter().find(|(k, _)| *k == i).map(|(_, it)| *it)
    }

    #[test]
    fn belt_type_and_record() {
        let mut s = BeltState::default();
        assert_eq!(s.belt_type, 2);
        // A belt item gives its type; another item leaves it; none → 2.
        s.update_type(BeltSlot8::Belt(3)).unwrap();
        assert_eq!(s.belt_type, 3);
        s.update_type(BeltSlot8::Other).unwrap();
        assert_eq!(s.belt_type, 3);
        s.update_type(BeltSlot8::Empty).unwrap();
        assert_eq!(s.belt_type, 2);
        assert_eq!(s.update_type(BeltSlot8::Belt(-1)), Err(BeltTypeError(-1)));
        // The record: resolution · 7 + type; `belts.txt` order.
        assert_eq!(record_index(0, 2), 2);
        assert_eq!(record_index(1, 6), 13);
        assert_eq!(BELT_NAMES[0], "belt");
        assert_eq!(BELT_NAMES[2], "default");
        assert_eq!(BELT_NAMES[6], "uber belt");
        // The record layout: count u8 +4, boxes from +8 (left, right, top,
        // bottom).
        let mut b = vec![0u8; 0x108];
        b[4] = 2;
        for (i, v) in [343i32, 373, 440, 470, 374, 404, 440, 470]
            .iter()
            .enumerate()
        {
            b[8 + 4 * i..12 + 4 * i].copy_from_slice(&(*v).to_le_bytes());
        }
        let r = BeltRecord::from_bytes(&b).unwrap();
        assert_eq!(
            r.boxes,
            vec![
                BeltBox {
                    left: 343,
                    right: 373,
                    top: 440,
                    bottom: 470
                },
                BeltBox {
                    left: 374,
                    right: 404,
                    top: 440,
                    bottom: 470
                }
            ]
        );
        assert!(BeltRecord::from_bytes(&b[..20]).is_none());
    }

    #[test]
    fn popup_flag() {
        let mut s = BeltState {
            popped: true,
            ..Default::default()
        };
        // State 0x1F open: stays; closed and [7BEF9C] = 0: cleared.
        s.clear_popped(true, false);
        assert!(s.popped);
        s.ef9c = true;
        s.clear_popped(false, false);
        assert!(s.popped);
        s.ef9c = false;
        s.clear_popped(false, false);
        assert!(!s.popped);
        // State 9 open: cleared whatever else.
        s.popped = true;
        s.ef9c = true;
        s.clear_popped(true, true);
        assert!(!s.popped);
        // The state 0x1F draw sets it.
        s.state_1f_draw();
        assert!(s.popped);
    }

    // Test vector "640 × 480 belt type 0 popped".
    #[test]
    fn popup_rows_and_row_counts() {
        let mut s = BeltState {
            belt_type: 0,
            popped: true,
            ..Default::default()
        };
        assert_eq!(
            s.popup_draws(640, 480),
            vec![
                BeltDraw::PopRow { x: 341, y: 439 },
                BeltDraw::PopRow { x: 341, y: 407 }
            ]
        );
        assert!(s.extra_boxes);
        // The rows fold (state 0x1F closed): the extra-boxes flag reads 0 again
        // (the mini panel draws at layout 2).
        s.clear_popped(false, false);
        assert!(!s.popped && !s.extra_boxes);
        s.popped = true;
        s.extra_boxes = true;
        // Not popped: no rows.
        s.popped = false;
        assert!(s.popup_draws(640, 480).is_empty());
        // The rows by type and the extra-boxes flag.
        let rows = |t: u8| popup_rows(t).to_vec();
        assert_eq!(rows(0), [0, 1]);
        assert_eq!(rows(1), [0]);
        assert_eq!(rows(2), Vec::<u8>::new());
        assert_eq!(rows(3), [0, 1, 2]);
        assert_eq!(rows(4), [0]);
        assert_eq!(rows(5), [0, 1]);
        assert_eq!(rows(6), [0, 1, 2]);
        assert_eq!(rows(99), [0, 1, 2]);
        let mut s = BeltState {
            belt_type: 2,
            popped: true,
            extra_boxes: true,
            ..Default::default()
        };
        assert!(s.popup_draws(800, 600).is_empty());
        assert!(!s.extra_boxes);
        // Row count: 3, 2, 1, 4, 2, 3, else 4 (rows + 1).
        let counts: Vec<u8> = (0..8).map(row_count).collect();
        assert_eq!(counts, [3, 2, 1, 4, 2, 3, 4, 4]);
        for t in 0..8u8 {
            assert_eq!(usize::from(row_count(t)), rows(t).len() + 1);
        }
    }

    #[test]
    fn slots() {
        let r = rec();
        let mut s = BeltState::default();
        let mut slots = vec![
            SlotInfo {
                item: None,
                key_name: None
            };
            16
        ];
        // Slot 0: a usable potion, hovered; slot 1: a usable potion not
        // hovered; slot 2: not usable (red); slot 4: above the first row.
        slots[0] = SlotInfo {
            item: Some(item(10, 0)),
            key_name: Some(vec![49]),
        };
        slots[1] = SlotInfo {
            item: Some(item(11, 1)),
            key_name: None,
        };
        let mut nu = item(12, 2);
        nu.usable = false;
        nu.has_use = false;
        slots[2] = SlotInfo {
            item: Some(nu),
            key_name: Some(vec![50]),
        };
        slots[4] = SlotInfo {
            item: Some(item(14, 4)),
            key_name: Some(vec![52]),
        };
        s.hover_item = Some(10);
        s.hovered = true;
        let d = s.slot_draws(&r, &slots);
        let sq = |left: i32, top: i32| RectDraw {
            x: left,
            y: top,
            w: 29,
            h: 29,
            color: 0,
            mode: 0,
        };
        assert_eq!(
            d,
            vec![
                // Slot 0: green (hovered usable), the item, the label.
                BeltDraw::Box {
                    rect: sq(343, 440),
                    color: BeltColor::Green
                },
                BeltDraw::Item {
                    guid: 10,
                    x: 343,
                    y: 440
                },
                BeltDraw::Label(LineDraw {
                    text: vec![49],
                    x: 345,
                    y: 468,
                    color: 4
                }),
                // Slot 1: usable, not hovered: no rectangle, no key.
                BeltDraw::Item {
                    guid: 11,
                    x: 374,
                    y: 440
                },
                // Slot 2: not usable: red, no label (no use).
                BeltDraw::Box {
                    rect: sq(405, 440),
                    color: BeltColor::Red
                },
                BeltDraw::Item {
                    guid: 12,
                    x: 405,
                    y: 440
                },
            ]
        );
        // Boxes above 3 only while the extra-boxes flag is set; no label past box 3.
        s.extra_boxes = true;
        let d = s.slot_draws(&r, &slots);
        assert!(d.contains(&BeltDraw::Item {
            guid: 14,
            x: 343,
            y: 409
        }));
        assert!(!d
            .iter()
            .any(|d| matches!(d, BeltDraw::Label(l) if l.text == vec![52])));
        assert_eq!(BeltColor::Red.rgb(), (128, 0, 0));
        assert_eq!(BeltColor::Green.rgb(), (0, 128, 0));
        assert_eq!(BeltColor::Blue.rgb(), (0, 0, 128));
        assert_eq!(BeltColor::Yellow.rgb(), (128, 128, 0));
    }

    fn cursor_item(fits: bool, swap: bool) -> CursorInfo {
        CursorInfo {
            mode: 0,
            item: Some(CursorItem {
                guid: 77,
                fits_belt: fits,
                swap_ok: swap,
                blocked: false,
                put_sound: 12,
            }),
        }
    }

    #[test]
    fn cursor_item_highlight() {
        let r = rec();
        let mut s = BeltState {
            belt_type: 0,
            popped: true,
            hovered: true,
            hover_box: 1,
            ..Default::default()
        };
        let col = |s: &BeltState, c: &CursorInfo, occ: bool| match s.cursor_highlight(
            &r,
            false,
            c,
            &|_| occ,
        ) {
            Some(BeltDraw::Box { color, rect }) => {
                assert_eq!(
                    (rect.x, rect.y, rect.w, rect.h, rect.mode),
                    (374, 440, 29, 29, 0)
                );
                Some(color)
            }
            Some(_) => unreachable!(),
            None => None,
        };
        assert_eq!(
            col(&s, &cursor_item(true, false), false),
            Some(BeltColor::Green)
        );
        assert_eq!(
            col(&s, &cursor_item(false, false), false),
            Some(BeltColor::Red)
        );
        assert_eq!(
            col(&s, &cursor_item(true, true), true),
            Some(BeltColor::Yellow)
        );
        assert_eq!(
            col(&s, &cursor_item(true, false), true),
            Some(BeltColor::Red)
        );
        // Not for type 2; not without the popped belt or state 0x1F; not
        // without a cursor item, a hover, or a box in range.
        s.belt_type = 2;
        assert_eq!(col(&s, &cursor_item(true, false), false), None);
        s.belt_type = 0;
        s.popped = false;
        assert_eq!(col(&s, &cursor_item(true, false), false), None);
        assert!(s
            .cursor_highlight(&r, true, &cursor_item(true, false), &|_| false)
            .is_some());
        s.popped = true;
        assert_eq!(col(&s, &CursorInfo::default(), false), None);
        s.hover_box = 16;
        assert_eq!(col(&s, &cursor_item(true, false), false), None);
        s.hover_box = -1;
        assert_eq!(col(&s, &cursor_item(true, false), false), None);
        s.hover_box = 1;
        s.hovered = false;
        assert_eq!(col(&s, &cursor_item(true, false), false), None);
    }

    #[test]
    fn hit_area() {
        let r = rec();
        let mut s = BeltState::default();
        // Not popped: y > H − 48, x W/2 + 23 … W/2 + 145, y H − 39 … H − 10.
        assert!(s.hit_area(&r, 640, 480, 343, 441));
        assert!(s.hit_area(&r, 640, 480, 465, 470));
        assert!(!s.hit_area(&r, 640, 480, 342, 450));
        assert!(!s.hit_area(&r, 640, 480, 466, 450));
        assert!(!s.hit_area(&r, 640, 480, 400, 440));
        assert!(!s.hit_area(&r, 640, 480, 400, 471));
        assert!(s.hit_area(&r, 640, 480, 400, 441));
        // Popped: from box (count − 4).left … box 3.right,.top … box 3.bottom:
        // count 16 → box 12 (343, 347) to box 3 (…, 470).
        s.popped = true;
        let (a, b) = (r.boxes[12], r.boxes[3]);
        assert!(s.hit_area(&r, 640, 480, a.left, a.top));
        assert!(s.hit_area(&r, 640, 480, b.right, b.bottom));
        assert!(!s.hit_area(&r, 640, 480, a.left - 1, a.top));
        assert!(!s.hit_area(&r, 640, 480, a.left, a.top - 1));
    }

    #[test]
    fn box_hit() {
        let r = rec();
        let s = BeltState::default();
        assert_eq!(s.box_hit(&r, 343, 440), Some(0));
        assert_eq!(s.box_hit(&r, 373, 470), Some(0));
        assert_eq!(s.box_hit(&r, 374, 440), Some(1));
        assert_eq!(s.box_hit(&r, 342, 440), None);
        assert_eq!(s.box_hit(&r, 343, 409), Some(4));
        // The first box wins when edges touch.
        let touch = BeltRecord {
            boxes: vec![
                BeltBox {
                    left: 0,
                    right: 10,
                    top: 0,
                    bottom: 10,
                },
                BeltBox {
                    left: 10,
                    right: 20,
                    top: 0,
                    bottom: 10,
                },
            ],
        };
        assert_eq!(s.box_hit(&touch, 10, 5), Some(0));
    }

    // Test vectors "belt click on box 2 (empty)" and "box 0 holding a
    // potion, no cursor item".
    #[test]
    fn belt_click() {
        let r = rec();
        let s = BeltState::default();
        let none = |_: usize| None;
        // Box 2 empty, a potion on the cursor: C→S 0x23 [GUID][2], the
        // item call, the put sound.
        let e = s.click(&r, 410, 445, &cursor_item(true, false), &none);
        assert_eq!(
            e,
            vec![
                BeltEffect::Send(ClientIntent(vec![0x23, 77, 0, 0, 0, 2, 0, 0, 0])),
                BeltEffect::Call4c21f0(77),
                BeltEffect::Sound(12)
            ]
        );
        // Box 2 occupied: 0x25 [c][e].
        let occ = items(vec![(2, item(30, 2))]);
        let e = s.click(&r, 410, 445, &cursor_item(true, false), &occ);
        assert_eq!(
            e[0],
            BeltEffect::Send(ClientIntent(vec![0x25, 77, 0, 0, 0, 30, 0, 0, 0]))
        );
        // A blocked cursor item: only the put sound.
        let mut c = cursor_item(true, false);
        c.item.as_mut().unwrap().blocked = true;
        assert_eq!(
            s.click(&r, 410, 445, &c, &none),
            vec![BeltEffect::Sound(12)]
        );
        // No cursor item, box 0 holds a potion: C→S 0x24 [GUID], the item
        // call.
        let occ0 = items(vec![(0, item(30, 0))]);
        let e = s.click(&r, 345, 445, &CursorInfo::default(), &occ0);
        assert_eq!(
            e,
            vec![
                BeltEffect::Send(ClientIntent(vec![0x24, 30, 0, 0, 0])),
                BeltEffect::Call4c21f0(30)
            ]
        );
        // The item blocked: nothing. A cursor item that does not fit:
        // nothing. Cursor state 6: nothing. No box: nothing.
        let mut blocked = item(30, 0);
        blocked.blocked = true;
        assert!(s
            .click(
                &r,
                345,
                445,
                &CursorInfo::default(),
                &items(vec![(0, blocked)])
            )
            .is_empty());
        assert!(s
            .click(&r, 345, 445, &cursor_item(false, false), &occ0)
            .is_empty());
        let six = CursorInfo {
            mode: 6,
            ..cursor_item(true, false)
        };
        assert!(s.click(&r, 345, 445, &six, &occ0).is_empty());
        assert!(s.click(&r, 0, 0, &CursorInfo::default(), &occ0).is_empty());
    }

    #[test]
    fn hover_tracking() {
        let r = rec();
        let mut s = BeltState::default();
        let none = |_: usize| None;
        // No box: nothing.
        s.hover_track(&r, 0, 0, &CursorInfo::default(), &none);
        assert_eq!(s.hover_box, -1);
        // Cursor state 6 or 8: nothing.
        for mode in [6, 8] {
            let c = CursorInfo {
                mode,
                ..Default::default()
            };
            s.hover_track(&r, 345, 445, &c, &none);
            assert_eq!(s.hover_box, -1);
        }
        // A cursor item: the box, the belt hovered, hovered and last:= 0.
        s.hover_item = Some(5);
        s.last_item = Some(5);
        s.hover_track(&r, 410, 445, &cursor_item(true, false), &none);
        assert_eq!(
            (s.hover_box, s.hovered, s.hover_item, s.last_item),
            (2, true, None, None)
        );
        // No item in the box: belt hovered:= 0, hovered:= last:= 0.
        s.hover_track(&r, 345, 445, &CursorInfo::default(), &none);
        assert_eq!((s.hover_box, s.hovered), (0, false));
        // An item: belt hovered, and (e ≠ last) hovered:= last:= e and the text
        // position (box.left + 14, box.top) of e's own box.
        let occ = items(vec![(1, item(30, 1))]);
        s.hover_track(&r, 380, 445, &CursorInfo::default(), &occ);
        assert_eq!(
            (s.hovered, s.hover_item, s.last_item, s.text_pos),
            (true, Some(30), Some(30), (374 + 14, 440))
        );
        // The same item again: the text position stays.
        s.text_pos = (1, 2);
        s.hover_track(&r, 380, 445, &CursorInfo::default(), &occ);
        assert_eq!(s.text_pos, (1, 2));
        // The resets.
        s.reset_items();
        assert_eq!((s.hover_item, s.last_item, s.hover_box), (None, None, 1));
        s.reset_all();
        assert_eq!((s.hover_box, s.text_pos, s.hovered), (-1, (-1, -1), false));
    }

    #[test]
    fn mouse_move_state_machine() {
        let r = rec();
        let ok = MoveGates {
            in_game: true,
            alive: true,
            ..Default::default()
        };
        let none = |_: usize| None;
        let cur = CursorInfo::default();
        // Gates: not in game, input blocked, dead, modal input owner.
        for g in [
            MoveGates {
                in_game: false,
                ..ok
            },
            MoveGates {
                input_blocked: true,
                ..ok
            },
            MoveGates { alive: false, ..ok },
            MoveGates {
                x44bfe0: true,
                ..ok
            },
        ] {
            let mut s = BeltState::default();
            assert!(!s.mouse_move(&r, 640, 480, 400, 450, &g, &cur, &none));
            assert!(!s.popped);
        }
        // Not popped, inside the strip: pops up, [7BEF9C]:= 1, tracks,
        // consumed.
        let mut s = BeltState::default();
        assert!(s.mouse_move(&r, 640, 480, 345, 445, &ok, &cur, &none));
        assert!(s.popped && s.ef9c);
        assert_eq!(s.hover_box, 0);
        // State 9 open: no pop, still consumed.
        let mut s = BeltState::default();
        let g9 = MoveGates {
            state_9_open: true,
            ..ok
        };
        assert!(s.mouse_move(&r, 640, 480, 345, 445, &g9, &cur, &none));
        assert!(!s.popped);
        // Elsewhere: [7BEF94]:= 0, not consumed.
        let mut s = BeltState {
            hovered: true,
            ..Default::default()
        };
        assert!(!s.mouse_move(&r, 640, 480, 100, 100, &ok, &cur, &none));
        assert!(!s.hovered);
        // Popped and over the belt: tracking; consumed only while state
        // 0x1F is open.
        let mut s = BeltState {
            popped: true,
            ..Default::default()
        };
        assert!(!s.mouse_move(&r, 640, 480, 345, 445, &ok, &cur, &none));
        let g1f = MoveGates {
            state_1f_open: true,
            ..ok
        };
        assert!(s.mouse_move(&r, 640, 480, 345, 445, &g1f, &cur, &none));
        // Popped and off the belt: [7BEF9C]:= [7BEF94]:= 0 and, with state 0x1F
        // closed, popped:= 0.
        let mut s = BeltState {
            popped: true,
            ef9c: true,
            hovered: true,
            ..Default::default()
        };
        assert!(!s.mouse_move(&r, 640, 480, 100, 100, &g1f, &cur, &none));
        assert!(s.popped && !s.ef9c && !s.hovered);
        assert!(!s.mouse_move(&r, 640, 480, 100, 100, &ok, &cur, &none));
        assert!(!s.popped);
    }

    #[test]
    fn hover_text_rules() {
        let name: Vec<u16> = "Healing Potion".encode_utf16().collect();
        let stats: Vec<u16> = "Heals 45".encode_utf16().collect();
        let it = item(30, 1);
        let mut s = BeltState {
            hovered: true,
            hover_item: Some(30),
            hover_box: 1,
            text_pos: (388, 440),
            ..Default::default()
        };
        let t = hover_text(&s, false, Some(&it), &name, &stats, None).unwrap();
        // The expected text: `ÿc3` + stat lines + `ÿc0` + name, colour 0,
        // centred at the text position.
        let want: Vec<u16> = "\u{ff}c3Heals 45\u{ff}c0Healing Potion"
            .encode_utf16()
            .collect();
        assert_eq!(
            (t.text, t.x, t.y, t.color, t.centered),
            (want, 388, 440, 0, true)
        );
        // NPC trade, price text P: as above + LF + P.
        let price: Vec<u16> = "(10)".encode_utf16().collect();
        let t = hover_text(&s, false, Some(&it), &name, &stats, Some(&price)).unwrap();
        assert_eq!(
            String::from_utf16(&t.text).unwrap(),
            "\u{ff}c3Heals 45\u{ff}c0Healing Potion\n(10)"
        );
        // Quality 3: no stat lines and no colour code for them.
        let mut q3 = it;
        q3.quality3 = true;
        let t = hover_text(&s, false, Some(&q3), &name, &stats, Some(&price)).unwrap();
        assert_eq!(
            String::from_utf16(&t.text).unwrap(),
            "\u{ff}c0Healing Potion\n(10)"
        );
        // Not with a cursor item, without a hover, or for box > 3 unless
        // popped or [7BEF9C].
        assert!(hover_text(&s, true, Some(&it), &name, &stats, None).is_none());
        let mut u = s.clone();
        u.hovered = false;
        assert!(hover_text(&u, false, Some(&it), &name, &stats, None).is_none());
        s.hover_box = 5;
        assert!(hover_text(&s, false, Some(&it), &name, &stats, None).is_none());
        s.popped = true;
        assert!(hover_text(&s, false, Some(&it), &name, &stats, None).is_some());
        s.popped = false;
        s.ef9c = true;
        assert!(hover_text(&s, false, Some(&it), &name, &stats, None).is_some());
        // Positions must be ≥ 0.
        s.text_pos = (-1, -1);
        assert!(hover_text(&s, false, Some(&it), &name, &stats, None).is_none());
        // The name is cut to 128 units.
        s.text_pos = (1, 1);
        let long = vec![65u16; 300];
        let t = hover_text(&s, false, Some(&q3), &long, &stats, None).unwrap();
        assert_eq!(t.text.len(), 3 + 128);
    }
}
