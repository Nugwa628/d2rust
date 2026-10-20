//! The ground-item name labels of Show Items (command 37, ui state 0x0D):
//! one box per item with the item's name, stacked so they do not overlap.
//!
//! [`layout`] is the pure part: the anchor of each label, the 12-try
//! collision shifts, the hover test and the box and text
//! draw. The model feed (names, colours, screen points)
//! is built by the caller.
//!
//! The caller gives the items in the game's walk order (the rooms of the
//! player's room's adjacency array, each room's unit list head first), which
//! decides which of two overlapping labels moves (settled by
//! `bug-b009-show-items-draws`).

use super::draw::{RectRequest, TextRequest, UiDraw};
use super::geom::Point;
use super::text::{TextOpts, TEXT_DRAW_MODE};
use super::TextStyle;

/// The font of the label (the pop-up font, `hud_tips::TIP_FONT`).
pub const LABEL_FONT: u16 = 1;

/// Labels placed at most (the table holds 0x20).
pub const MAX_LABELS: usize = 0x20;
/// Collision tries per label.
pub const TRIES: u32 = 12;

/// The text measure of one label: widest line, height of all lines and the font
/// height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub width: i32,
    pub height: i32,
    pub font_height: i32,
}

/// One item to label.
#[derive(Clone, Debug)]
pub struct LabelIn {
    pub guid: u32,
    /// The item's screen point (`unit_feet`).
    pub feet: (i32, i32),
    pub text: Vec<u16>,
    /// The text colour of the unhovered label.
    pub color: u16,
}

/// A placed label's rectangle (left, top, right, bottom) of the
/// collision pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub guid: u32,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// The labels of one frame.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub draws: Vec<UiDraw>,
    pub placed: Vec<Placed>,
    /// The label under the pointer (becomes the hover unit).
    pub hovered: Option<u32>,
}

impl Layout {
    /// The label whose box covers `(x, y)`, the topmost (last).
    pub fn hit(&self, x: i32, y: i32) -> Option<u32> {
        self.placed
            .iter()
            .rev()
            .find(|p| p.left <= x && x <= p.right && p.top <= y && y <= p.bottom + 4)
            .map(|p| p.guid)
    }
}

/// Clamps the rectangle to the screen and, at the first
/// overlap with a placed label, moves it (tries 0–6 vertically, 7+
/// horizontally). `true` = no overlap, final.
fn settle(r: &mut [i32; 4], try_no: u32, (sw, sh): (i32, i32), placed: &[Placed]) -> bool {
    let [left, top, right, bottom] = r;
    let lim = sh - 0x30;
    if *left < 0 {
        *right += -*left;
        *left = 6;
    }
    if *top < 6 {
        *bottom += 6 - *top;
        *top = 6;
    }
    if *right >= sw {
        let d = *right - sw + 1;
        *right -= d;
        *left -= d;
    }
    if *bottom >= lim {
        let d = *bottom - lim;
        *bottom -= d;
        *top -= d;
    }
    for e in placed {
        let across = e.right >= *left && *right >= e.left;
        let down = e.bottom >= *top && *bottom >= e.top;
        if !(across && down) {
            continue;
        }
        if try_no < 7 {
            let d = if (*bottom - *top) / 2 != 0 && *top <= lim {
                (e.top - *bottom) - 3
            } else {
                (e.bottom - *top) + 5
            };
            *top += d;
            *bottom += d;
        } else {
            let d = if (*right - *left) / 2 != 0 && sw <= *left {
                (e.left - *right) - 1
            } else {
                (e.right - *left) + 1
            };
            *left += d;
            *right += d;
        }
        return false;
    }
    true
}

/// The labels of `items` on a `screen`-sized frame, the pointer at
/// `pointer`, `hovered` the unit hovered the pass before, `blue` the
/// palette index of RGB (0, 0x40, 0x80).
pub fn layout(
    items: &[LabelIn],
    measure: &dyn Fn(&[u16]) -> Option<Metrics>,
    screen: (i32, i32),
    pointer: (i32, i32),
    hovered: Option<u32>,
    blue: u8,
) -> Layout {
    let (sw, sh) = screen;
    let mut out = Layout::default();
    let mut taken = false;
    for it in items {
        if out.placed.len() >= MAX_LABELS {
            break;
        }
        let Some(m) = measure(&it.text) else { continue };
        let (x, y) = (it.feet.0, it.feet.1 - 16);
        if !(0 <= x && x <= sw && -9 < y && y <= sh - 16) {
            continue;
        }
        let w = m.width + 8;
        let left = x - (w >> 1);
        let mut r = [left, y - m.height + 4, left + w, y];
        let settled = (0..TRIES).any(|t| settle(&mut r, t, screen, &out.placed));
        let [left, top, right, bottom] = r;
        if !(settled && right <= sw && 0 <= left) {
            continue;
        }
        let p = Placed {
            guid: it.guid,
            left,
            top,
            right,
            bottom,
        };
        let inside =
            pointer.0 >= left && pointer.0 <= right && pointer.1 >= top && pointer.1 <= bottom + 4;
        let hot = !taken && (inside || hovered == Some(it.guid));
        if hot {
            taken = true;
            if inside {
                out.hovered = Some(it.guid);
            }
        }
        let (fill, mode, color) = if hot { (blue, 5, 0) } else { (0, 1, it.color) };
        let bx = left.max(0).min(sw - w);
        let t = (bottom + 2).max(m.height);
        let by = if t < sh - 0x1f { t } else { sh - 0x1f };
        out.draws.push(UiDraw::Rect(RectRequest {
            x0: bx,
            y0: by - m.height,
            x1: bx + w,
            y1: by,
            color: fill,
            mode,
        }));
        out.draws.push(UiDraw::Text(TextRequest {
            text: it.text.clone(),
            at: Point::new(bx, by - m.font_height * 3 / 10),
            style: TextStyle {
                font: LABEL_FONT,
                color,
            },
            opts: TextOpts::Draw {
                centered: true,
                block_w: Some(w),
                mode: TEXT_DRAW_MODE,
            },
            clip: super::geom::screen_clip(screen),
        }));
        out.placed.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(_: &[u16]) -> Option<Metrics> {
        Some(Metrics {
            width: 52,
            height: 16,
            font_height: 16,
        })
    }

    fn item(guid: u32, x: i32, y: i32) -> LabelIn {
        LabelIn {
            guid,
            feet: (x, y),
            text: vec![0x41],
            color: 3,
        }
    }

    #[test]
    fn a_label_sits_16_above_the_feet_and_centres_on_x() {
        let l = layout(&[item(1, 400, 300)], &m, (800, 600), (0, 0), None, 9);
        let p = l.placed[0];
        // w = 60, left = 400 - 30; bottom = 284; top = 284 - 16 + 4.
        assert_eq!((p.left, p.top, p.right, p.bottom), (370, 272, 430, 284));
        let UiDraw::Rect(r) = &l.draws[0] else {
            panic!()
        };
        // by = max(284 + 2, 16) = 286; box 286 - 16.. 286, colour 0, mode 1.
        assert_eq!(
            (r.x0, r.y0, r.x1, r.y1, r.color, r.mode),
            (370, 270, 430, 286, 0, 1)
        );
        let UiDraw::Text(t) = &l.draws[1] else {
            panic!()
        };
        assert_eq!((t.at.x, t.at.y, t.style.color), (370, 286 - 4, 3));
    }

    #[test]
    fn an_overlapping_label_moves_above_the_first() {
        let l = layout(
            &[item(1, 400, 300), item(2, 410, 300)],
            &m,
            (800, 600),
            (0, 0),
            None,
            9,
        );
        let (a, b) = (l.placed[0], l.placed[1]);
        // Moved so its bottom is the first's top - 3.
        assert_eq!(b.bottom, a.top - 3);
        assert_eq!(b.bottom - b.top, a.bottom - a.top);
        assert_eq!(b.left, 380);
    }

    #[test]
    fn the_pointer_highlights_one_label() {
        let l = layout(&[item(1, 400, 300)], &m, (800, 600), (400, 280), None, 9);
        assert_eq!(l.hovered, Some(1));
        let UiDraw::Rect(r) = &l.draws[0] else {
            panic!()
        };
        assert_eq!((r.color, r.mode), (9, 5));
        let UiDraw::Text(t) = &l.draws[1] else {
            panic!()
        };
        assert_eq!(t.style.color, 0);
        assert_eq!(l.hit(400, 288), Some(1));
        assert_eq!(l.hit(400, 289), None);
    }

    #[test]
    fn off_screen_items_have_no_label() {
        for (x, y) in [(-1, 300), (801, 300), (400, 601), (400, 6)] {
            let l = layout(&[item(1, x, y)], &m, (800, 600), (0, 0), None, 9);
            assert!(l.placed.is_empty(), "{x},{y}");
        }
    }

    /// On the 16:9 frame an item near the right edge keeps its label, and
    /// the text's clip is the whole 1068-wide frame, not the centred 800;
    /// at 800 × 600 the clip stays the 800 × 600 frame.
    #[test]
    fn a_label_near_the_wide_frame_edge_is_not_clipped() {
        let l = layout(&[item(1, 1040, 300)], &m, (1068, 600), (0, 0), None, 9);
        let p = l.placed[0];
        // 60 wide, moved left to end at column 1067.
        assert_eq!((p.left, p.right), (1007, 1067));
        let UiDraw::Text(t) = &l.draws[1] else {
            panic!()
        };
        assert_eq!(t.clip, crate::ui::Rect::new(0, 0, 1068, 600));
        assert!(t.at.x >= 1000);
        let l = layout(&[item(1, 400, 300)], &m, (800, 600), (0, 0), None, 9);
        let UiDraw::Text(t) = &l.draws[1] else {
            panic!()
        };
        assert_eq!(t.clip, crate::ui::FRAME);
    }
}
