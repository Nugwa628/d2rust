//! Integer geometry of the 800×600 logical frame.

/// Logical frame width.
pub const FRAME_W: u16 = 800;
/// Logical frame height.
pub const FRAME_H: u16 = 600;
/// The whole logical frame.
pub const FRAME: Rect = Rect::new(0, 0, FRAME_W, FRAME_H);

/// The clip of a draw laid out on a `w × h` screen: the whole screen, and
/// never less than [`FRAME`]. Labels placed over the world of a 16:9
/// frame (1068 wide) reach its side edges; on the 800 × 600 and 640 × 480
/// frames the clip is [`FRAME`] itself.
pub fn screen_clip((w, h): (i32, i32)) -> Rect {
    let wide = |v: i32, min: u16| v.clamp(i32::from(min), i32::from(u16::MAX)) as u16;
    Rect::new(0, 0, wide(w, FRAME_W), wide(h, FRAME_H))
}

/// An integer point in the 800×600 frame (ours; the spec's `IVec2`, kept
/// free of Bevy types).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// An integer rectangle: `x <= px < x + w`, `y <= py < y + h`. Sizes are
/// unsigned, so a negative size cannot exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }

    pub const fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    /// Exclusive right edge, in `i64`: a rect may end past `i32::MAX`.
    pub const fn right(&self) -> i64 {
        self.x as i64 + self.w as i64
    }

    /// Exclusive bottom edge, in `i64` (as [`Self::right`]).
    pub const fn bottom(&self) -> i64 {
        self.y as i64 + self.h as i64
    }

    /// Half-open containment; an empty rect contains nothing.
    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.x
            && (p.x as i64) < self.right()
            && p.y >= self.y
            && (p.y as i64) < self.bottom()
    }
}

#[cfg(test)]
mod screen_clip_tests {
    use super::*;

    /// The 4:3 frames clip to the 800 × 600 frame as before; the 16:9
    /// frame clips to its own 1068 columns.
    #[test]
    fn the_screen_clip_is_the_frame_and_never_less_than_800_x_600() {
        assert_eq!(screen_clip((800, 600)), FRAME);
        assert_eq!(screen_clip((640, 480)), FRAME);
        assert_eq!(screen_clip((1068, 600)), Rect::new(0, 0, 1068, 600));
    }
}
