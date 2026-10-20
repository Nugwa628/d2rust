//! The box border (`menu\boxpieces`): a frame of cel pieces drawn
//! around a rectangle, after the box's black backing. The NPC menu box (no-cel
//! branch) and the NPC dialog panel call it.
//!
//! The routine takes the rectangle as (left, top, right, bottom), both edges
//! inclusive, and writes plain cel draws (mode 5) of the file's frames in this
//! order, with one running counter `n` that every edge piece increments before
//! it picks its frame: top edge (frame `n % 6 + 2`), bottom edge (`n % 6 +
//! 16`), left edge (`n % 3 + 10`), right edge (`n % 3 + 13`), then the four
//! corners (frames 0, 1, 8, 9).

use super::draw::{CelCall, ImageRef, ImageRequest, UiDraw, UiDrawSink};
use super::geom::{Point, Rect};
use super::CelLook;

/// The cel file of the border pieces.
pub const BOXPIECES_FILE: &str = "menu\\boxpieces";

/// The files the border needs registered.
pub(super) fn border_files() -> [String; 1] {
    [BOXPIECES_FILE.to_string()]
}

/// One piece: the frame and the cel draw position.
pub type Piece = (u32, i32, i32);

/// The border pieces for the rectangle (l, t, r, b), in draw
/// order.
pub fn border_pieces(l: i32, t: i32, r: i32, b: i32) -> Vec<Piece> {
    let right = r + 1;
    let bottom = b + 1;
    let mut n = 0i32;
    let mut out = Vec::new();
    // Top edge: x from l + 12 while x < right − 13, at y = t + 12.
    let mut x = l + 12;
    while x < right - 13 {
        n += 1;
        out.push(((n % 6 + 2) as u32, x, t + 12));
        x += 12;
    }
    // Bottom edge: x from l + 13, at y = bottom + 10 (a do-while behind a
    // first test).
    let mut x = l + 13;
    if x < right - 13 {
        loop {
            n += 1;
            out.push(((n % 6 + 16) as u32, x, bottom + 10));
            x += 12;
            if x >= right - 13 {
                break;
            }
        }
    }
    // Left edge: y from t + 22 while y < bottom − 1, at x = l − 4.
    let mut y = t + 22;
    while y < bottom - 1 {
        n += 1;
        out.push(((n % 3 + 10) as u32, l - 4, y));
        y += 12;
    }
    // Right edge: y from t + 24 while y < bottom − 1, at x = right − 7.
    let mut y = t + 12 + 12;
    if y < bottom - 1 {
        loop {
            n += 1;
            out.push(((n % 3 + 13) as u32, right - 7, y));
            y += 12;
            if y >= bottom - 1 {
                break;
            }
        }
    }
    out.push((0, l, t + 12));
    out.push((1, right - 12, t + 12));
    out.push((8, l, bottom + 1));
    out.push((9, right - 12, bottom + 1));
    out
}

/// Pushes the border of (l, t, r, b) as cel draws of `file`.
pub fn push_border(file: u32, rect: (i32, i32, i32, i32), clip: Rect, out: &mut dyn UiDrawSink) {
    let (l, t, r, b) = rect;
    for (frame, x, y) in border_pieces(l, t, r, b) {
        out.push(UiDraw::Image(ImageRequest {
            image: ImageRef { file, frame },
            at: Point::new(x, y),
            clip,
            look: CelLook::PLAIN,
            call: CelCall::Draw,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // (border)
    #[test]
    fn pieces_follow_the_original_order_and_counter() {
        // A 60 x 40 box at (100, 50): right = 160, bottom = 90.
        let p = border_pieces(100, 50, 159, 89);
        // Top edge x = 112, 124, 136 (< 147), y = 62; frames 3, 4, 5.
        assert_eq!(&p[..3], &[(3, 112, 62), (4, 124, 62), (5, 136, 62)]);
        // Bottom edge x = 113, 125, 137, y = 100; frames at n = 4, 5, 6.
        assert_eq!(&p[3..6], &[(20, 113, 100), (21, 125, 100), (16, 137, 100)]);
        // Corners last.
        let n = p.len();
        assert_eq!(
            &p[n - 4..],
            &[(0, 100, 62), (1, 148, 62), (8, 100, 91), (9, 148, 91)]
        );
        // Left edge at x = l − 4 = 96, right edge at x = right − 7 = 153.
        assert!(p.iter().any(|&(f, x, _)| (10..13).contains(&f) && x == 96));
        assert!(p.iter().any(|&(f, x, _)| (13..16).contains(&f) && x == 153));
    }
}
