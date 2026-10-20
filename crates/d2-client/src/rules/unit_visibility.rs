//! Whether a unit is on screen: the client's visibility test for one unit.
//!
//! The test runs in three steps. The unit's origin is turned into a screen point, the unit's
//! COF box must overlap the frame ([`cof_box_visible`]), and its torso cel must overlap the
//! frame too ([`cel_box_visible`]). Requesting and loading that cel is the caller's: it
//! passes `None` when either fails, and such a unit is not visible. Plain Rust, integer math.

use d2_formats::cof::Cof;

use super::unit_composite::cof_box_visible;

/// The cel fields the box test reads: width, height and the x and y offsets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CelBox {
    pub w: i32,
    pub h: i32,
    pub ox: i32,
    pub oy: i32,
}

/// The screen point of client pixel point (`a`, `b`) for a unit origin (`cx`, `cy`):
/// `X = a − (cx − shift_x)`, `Y = b − (cy − 8)`.
pub fn screen_point(a: i32, b: i32, cx: i32, cy: i32, shift_x: i32) -> (i32, i32) {
    (a - (cx - shift_x), b - (cy - 8))
}

/// Whether a cel drawn at (`x`, `y`) overlaps a `w` × `h` frame. Its left edge is
/// `left = ox + x` and its anchor row `top = oy + y`; it is visible when `left ≤ w`,
/// `left + cel.w ≥ 0`, `top − cel.h ≤ h` and `top + cel.h ≥ 0` (signed). The vertical span
/// `[top − h, top + h]` is twice the cel's height, so a cel near the top or bottom edge
/// counts as visible a little early.
pub fn cel_box_visible(cel: CelBox, x: i32, y: i32, w: i32, h: i32) -> bool {
    let (left, top) = (cel.ox + x, cel.oy + y);
    left <= w && left + cel.w >= 0 && top - cel.h <= h && top + cel.h >= 0
}

/// The whole test for a unit at client pixel point (`a`, `b`) with origin `origin` in a
/// `w` × `h` frame. `cel` is the unit's loaded torso cel (`None`: its request or load failed,
/// so the unit is not visible).
#[allow(clippy::too_many_arguments)]
pub fn unit_visible(
    cof: &Cof,
    cel: Option<CelBox>,
    a: i32,
    b: i32,
    origin: (i32, i32),
    shift_x: i32,
    w: u32,
    h: u32,
) -> bool {
    let (x, y) = screen_point(a, b, origin.0, origin.1, shift_x);
    if !cof_box_visible(cof, x, y, w, h) {
        return false;
    }
    cel.is_some_and(|c| cel_box_visible(c, x, y, w as i32, h as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_point_subtracts_the_origin() {
        assert_eq!(screen_point(100, 50, 30, 20, 4), (74, 38));
    }

    #[test]
    fn cel_box_edges() {
        let c = CelBox {
            w: 10,
            h: 6,
            ox: -5,
            oy: 0,
        };
        // left = -5 + x; the right edge left + w ≥ 0 fails below x = -5.
        assert!(cel_box_visible(c, -5, 0, 640, 480));
        assert!(!cel_box_visible(c, -6, 0, 640, 480));
        // left ≤ W.
        assert!(cel_box_visible(c, 645, 0, 640, 480));
        assert!(!cel_box_visible(c, 646, 0, 640, 480));
        // top − h ≤ H and top + h ≥ 0 (a 2h-tall span).
        assert!(cel_box_visible(c, 0, 486, 640, 480));
        assert!(!cel_box_visible(c, 0, 487, 640, 480));
        assert!(cel_box_visible(c, 0, -6, 640, 480));
        assert!(!cel_box_visible(c, 0, -7, 640, 480));
    }

    fn cof(x_min: i32, x_max: i32, y_min: i32, y_max: i32) -> Cof {
        Cof {
            layers_count: 0,
            frames: 1,
            directions: 16,
            version: 20,
            unknown: [0; 4],
            x_min,
            x_max,
            y_min,
            y_max,
            animation_rate: 256,
            layers: vec![],
            events: vec![0],
            event_padding: vec![],
            draw_order: vec![],
        }
    }

    #[test]
    fn the_predicate_chains_the_cof_box_and_the_cel_box() {
        let c = cof(-30, 30, -90, 0);
        let cel = Some(CelBox {
            w: 20,
            h: 10,
            ox: -10,
            oy: -5,
        });
        // Origin at (400, 300), a = b = origin: X = shift, Y = 8.
        assert!(unit_visible(&c, cel, 400, 300, (400, 300), 0, 800, 600));
        // COF box fails (x_min + X < W − 1 broken) → not visible.
        assert!(!unit_visible(&c, cel, 400, 300, (400, 300), 900, 800, 600));
        // A failed cel request or load → not visible.
        assert!(!unit_visible(&c, None, 400, 300, (400, 300), 0, 800, 600));
    }
}
