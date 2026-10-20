//! Window → frame coordinate mapping.
//!
//! The game draws one fixed frame (800 × 600, or 640 × 480 under
//! `play --res 640x480`, [`Presentation::for_frame`]; the frame's size is
//! always taken from the frame itself). The present stage shows it in the
//! window the way the player's [`ScaleMode`] asks:
//!
//! - **fit** (the default): the largest scale that keeps the frame's shape,
//!   fractional if need be, bars on one axis only. The sharp scaler
//!   ([`crate::world_view::sharp`]) keeps every game pixel a solid block of
//!   even width;
//! - **integer**: the largest whole-number scale, centred with black bars,
//!   every game pixel exactly the same size. A window smaller than the
//!   frame has none, and shows the frame as fit does
//!   ([`super::edge::presentation`]).
//!
//! All of it is in physical pixels, so Windows display scaling never
//! changes where the frame lands.
//!
//! A cursor position in window pixels maps back by the inverse: subtract
//! the bar, then take the frame pixel under the window pixel's centre, with
//! integer maths, so the pointer always picks the game pixel drawn under
//! it. Positions in the bars are outside the frame. The input edge and the
//! present stage both build their [`Presentation`] from the same window and
//! [`scale_mode`], so the two directions agree.

use std::sync::atomic::{AtomicU8, Ordering};

use super::geom::{Point, FRAME_H, FRAME_W};
pub use crate::app::config::ScaleMode;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// Integer scaling has no whole-number scale below 1, so a window
    /// smaller than the frame can't show it that way; and no scale shows a
    /// frame in an empty window.
    #[error("window {w}×{h} is smaller than the frame")]
    TooSmall { w: u32, h: u32 },
}

/// Where a window position lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramePos {
    Inside(Point),
    /// In a black bar or outside the window.
    Outside,
}

/// The scale mode in force: picked on the Video page, stored in
/// `settings.toml`, and read by the input edge and the present stage
/// (0 fit, 1 integer).
static SCALE_MODE: AtomicU8 = AtomicU8::new(0);

/// The scale mode the window presents with.
pub fn scale_mode() -> ScaleMode {
    match SCALE_MODE.load(Ordering::Relaxed) {
        1 => ScaleMode::Integer,
        _ => ScaleMode::Fit,
    }
}

/// Sets the scale mode the window presents with (at startup and on each
/// Video page change).
pub fn set_scale_mode(mode: ScaleMode) {
    let n = match mode {
        ScaleMode::Fit => 0,
        ScaleMode::Integer => 1,
    };
    SCALE_MODE.store(n, Ordering::Relaxed);
}

/// Placement of the frame in a window, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Presentation {
    pub window_w: u32,
    pub window_h: u32,
    /// The frame `fw × fh`, from the frame itself.
    pub frame_w: u32,
    pub frame_h: u32,
    pub mode: ScaleMode,
    /// The whole-number part of the scale, at least 1. Under integer
    /// scaling it is the scale itself; things drawn at the frame's exact
    /// size (the hardware cursor) use `sx`/`sy` instead.
    pub scale: u32,
    /// The window pixels the frame covers, `image_w × image_h`.
    pub image_w: u32,
    pub image_h: u32,
    /// The scale on each axis, `image / frame`. Fit keeps the two within
    /// a rounding pixel of each other.
    pub sx: f32,
    pub sy: f32,
    /// Left bar width: `(w − image_w) / 2`, rounded down.
    pub left: u32,
    /// Top bar height: `(h − image_h) / 2`, rounded down.
    pub top: u32,
}

impl Presentation {
    /// The 800 × 600 frame in a `window_w × window_h` window, integer
    /// scaled.
    pub fn new(window_w: u32, window_h: u32) -> Result<Self, FrameError> {
        Self::for_frame(window_w, window_h, u32::from(FRAME_W), u32::from(FRAME_H))
    }

    /// A `frame_w × frame_h` frame in a `window_w × window_h` window,
    /// integer scaled.
    pub fn for_frame(
        window_w: u32,
        window_h: u32,
        frame_w: u32,
        frame_h: u32,
    ) -> Result<Self, FrameError> {
        Self::with_mode(window_w, window_h, frame_w, frame_h, ScaleMode::Integer)
    }

    /// A `frame_w × frame_h` frame in a `window_w × window_h` window, scaled
    /// as `mode` asks.
    pub fn with_mode(
        window_w: u32,
        window_h: u32,
        frame_w: u32,
        frame_h: u32,
        mode: ScaleMode,
    ) -> Result<Self, FrameError> {
        let (fw, fh) = (frame_w.max(1), frame_h.max(1));
        let too_small = FrameError::TooSmall {
            w: window_w,
            h: window_h,
        };
        let whole = (window_w / fw).min(window_h / fh);
        let (image_w, image_h) = match mode {
            ScaleMode::Integer => {
                if whole == 0 {
                    return Err(too_small);
                }
                (fw * whole, fh * whole)
            }
            ScaleMode::Fit => {
                // Width-limited when w / fw ≤ h / fh; the other side is
                // rounded to the nearest pixel and kept inside the window.
                let (w, h, fw64, fh64) = (
                    u64::from(window_w),
                    u64::from(window_h),
                    u64::from(fw),
                    u64::from(fh),
                );
                if w * fh64 <= h * fw64 {
                    (window_w, ((fh64 * w + fw64 / 2) / fw64).min(h) as u32)
                } else {
                    (((fw64 * h + fh64 / 2) / fh64).min(w) as u32, window_h)
                }
            }
        };
        if image_w == 0 || image_h == 0 {
            return Err(too_small);
        }
        Ok(Self {
            window_w,
            window_h,
            frame_w: fw,
            frame_h: fh,
            mode,
            scale: whole.max(1),
            image_w,
            image_h,
            sx: image_w as f32 / fw as f32,
            sy: image_h as f32 / fh as f32,
            left: (window_w - image_w) / 2,
            top: (window_h - image_h) / 2,
        })
    }

    /// The window pixels the frame covers.
    pub fn image_size(&self) -> (u32, u32) {
        (self.image_w, self.image_h)
    }

    /// The first window pixel (physical) of a frame pixel: the inverse of
    /// [`Self::to_frame`] (`to_frame(from_frame(p)) == p`).
    pub fn from_frame(&self, fx: i32, fy: i32) -> (i64, i64) {
        (
            i64::from(self.left) + first_pixel(fx, self.image_w, self.frame_w),
            i64::from(self.top) + first_pixel(fy, self.image_h, self.frame_h),
        )
    }

    /// The offset (physical pixels, x right, y up) from the window's
    /// centre to the centre of the frame's image: what a quad centred on
    /// the window must move by so the image sits where [`Self::to_frame`]
    /// maps the cursor. Non-zero (a half pixel) only with an odd leftover
    /// width or height.
    pub fn centre_offset(&self) -> (f32, f32) {
        let cx = self.left as f32 + self.image_w as f32 / 2.0;
        let cy = self.top as f32 + self.image_h as f32 / 2.0;
        (
            cx - self.window_w as f32 / 2.0,
            self.window_h as f32 / 2.0 - cy,
        )
    }

    /// Maps a window pixel (physical, top-left origin) to the frame.
    pub fn to_frame(&self, x: i64, y: i64) -> FramePos {
        // Saturating: a far-off position (the edge floors any float,
        // infinities included) stays outside instead of wrapping.
        let dx = x.saturating_sub(i64::from(self.left));
        let dy = y.saturating_sub(i64::from(self.top));
        let (iw, ih) = (i64::from(self.image_w), i64::from(self.image_h));
        if dx < 0 || dy < 0 || dx >= iw || dy >= ih {
            return FramePos::Outside;
        }
        let (fw, fh) = (i64::from(self.frame_w), i64::from(self.frame_h));
        // The frame pixel under the window pixel's centre, as the present
        // shader shows it. Inside the image it is already below the frame
        // size; the clamp keeps that true by construction.
        let fx = ((2 * dx + 1) * fw / (2 * iw)).clamp(0, fw - 1);
        let fy = ((2 * dy + 1) * fh / (2 * ih)).clamp(0, fh - 1);
        FramePos::Inside(Point::new(fx as i32, fy as i32))
    }
}

/// The first image pixel whose centre falls in frame pixel `f`: the
/// smallest `d` with `(d + ½) · frame / image ≥ f`, that is
/// `ceil((2 f · image − frame) / (2 frame))`.
pub fn first_pixel(f: i32, image: u32, frame: u32) -> i64 {
    let (f, image, frame) = (i64::from(f), i64::from(image), i64::from(frame));
    (2 * f * image - frame + 2 * frame - 1).div_euclid(2 * frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_matrix_keeps_shape_centres_and_largest_integer() {
        for (fw, fh) in [(800, 600), (1068, 600), (640, 480)] {
            for (w, h) in [
                (1280, 720),
                (1920, 1080),
                (2560, 1440),
                (3840, 2160),
                (1365, 767),
                (1919, 1079),
            ] {
                let whole = (w / fw).min(h / fh);
                for mode in [ScaleMode::Fit, ScaleMode::Integer] {
                    let p = Presentation::with_mode(w, h, fw, fh, mode).unwrap();
                    assert!(p.image_w <= w && p.image_h <= h);
                    assert!(w - p.image_w - 2 * p.left <= 1);
                    assert!(h - p.image_h - 2 * p.top <= 1);
                    assert!(
                        (i64::from(p.image_w) * i64::from(fh)
                            - i64::from(p.image_h) * i64::from(fw))
                        .abs()
                            <= i64::from(fw.max(fh)) / 2
                    );
                    if mode == ScaleMode::Integer {
                        assert_eq!(p.image_size(), (fw * whole, fh * whole));
                        assert!((whole + 1) * fw > w || (whole + 1) * fh > h);
                    } else {
                        assert!(p.image_w == w || p.image_h == h);
                    }
                    assert!(matches!(
                        p.to_frame(i64::from(p.left), i64::from(p.top)),
                        FramePos::Inside(_)
                    ));
                    assert_eq!(
                        p.to_frame(i64::from(p.left + p.image_w), i64::from(p.top)),
                        FramePos::Outside
                    );
                }
            }
        }
    }

    #[test]
    fn fit_fills_the_height_at_1080_1440_and_2160() {
        // The 600-line frames scale by 1.8, 2.4 and 3.6.
        for (w, h, s) in [(1920, 1080, 1.8f32), (2560, 1440, 2.4), (3840, 2160, 3.6)] {
            let p = Presentation::with_mode(w, h, 800, 600, ScaleMode::Fit).unwrap();
            assert_eq!(p.image_h, h);
            assert_eq!(p.image_w, (800.0 * s).round() as u32);
            assert!((p.sx - s).abs() < 1e-4 && (p.sy - s).abs() < 1e-4);
            assert_eq!(p.top, 0);
            // A 16:9 frame (1068 × 600) fills the screen to within 3 lines.
            let q = Presentation::with_mode(w, h, 1068, 600, ScaleMode::Fit).unwrap();
            assert!(w - q.image_w <= 3 && h - q.image_h <= 3, "{q:?}");
        }
    }

    #[test]
    fn integer_keeps_whole_scales_and_bars() {
        let p = Presentation::with_mode(2560, 1440, 800, 600, ScaleMode::Integer).unwrap();
        assert_eq!((p.scale, p.image_w, p.image_h), (2, 1600, 1200));
        assert_eq!((p.left, p.top), (480, 120));
        assert_eq!(
            Presentation::with_mode(640, 480, 800, 600, ScaleMode::Integer),
            Err(FrameError::TooSmall { w: 640, h: 480 })
        );
        // The input edge shows it as fit does then.
        let p = super::super::edge::present_in(640, 480, 800, 600, ScaleMode::Integer).unwrap();
        assert_eq!((p.mode, p.image_w, p.image_h), (ScaleMode::Fit, 640, 480));
    }

    #[test]
    fn fit_at_a_whole_scale_is_integer() {
        let fit = Presentation::with_mode(1600, 1200, 800, 600, ScaleMode::Fit).unwrap();
        let int = Presentation::with_mode(1600, 1200, 800, 600, ScaleMode::Integer).unwrap();
        for (x, y) in [(0, 0), (1, 1), (2, 3), (1599, 1199), (801, 600)] {
            assert_eq!(fit.to_frame(x, y), int.to_frame(x, y), "{x},{y}");
        }
    }

    /// Every frame pixel's first window pixel maps back to it, at every
    /// scale, and every window pixel maps into the frame pixel whose span
    /// holds it.
    #[test]
    fn the_mouse_round_trips_at_every_scale() {
        let cases = [
            (800, 600, 800, 600, ScaleMode::Integer),
            (1600, 1200, 800, 600, ScaleMode::Integer),
            (2400, 1800, 800, 600, ScaleMode::Integer),
            (2560, 1440, 800, 600, ScaleMode::Integer),
            (1920, 1080, 800, 600, ScaleMode::Fit),
            (2560, 1440, 800, 600, ScaleMode::Fit),
            (3840, 2160, 800, 600, ScaleMode::Fit),
            (1920, 1080, 1068, 600, ScaleMode::Fit),
            (2560, 1440, 1068, 600, ScaleMode::Fit),
            (3840, 2160, 1068, 600, ScaleMode::Fit),
            (640, 480, 800, 600, ScaleMode::Fit),
            (1280, 960, 640, 480, ScaleMode::Fit),
        ];
        for (w, h, fw, fh, mode) in cases {
            let p = Presentation::with_mode(w, h, fw, fh, mode).unwrap();
            // Scaled down, some frame pixels have no window pixel of their
            // own; the round trip holds when every one has.
            let up = p.image_w >= p.frame_w;
            for fx in (0..fw as i32).filter(|_| up) {
                let (x, _) = p.from_frame(fx, 0);
                assert_eq!(
                    p.to_frame(x, i64::from(p.top)),
                    FramePos::Inside(Point::new(fx, 0)),
                    "{w}x{h} {fw}x{fh} {mode:?} column {fx}"
                );
            }
            for fy in (0..fh as i32).filter(|_| up) {
                let (_, y) = p.from_frame(0, fy);
                assert_eq!(
                    p.to_frame(i64::from(p.left), y),
                    FramePos::Inside(Point::new(0, fy)),
                    "{w}x{h} {fw}x{fh} {mode:?} row {fy}"
                );
            }
            // Every window column lands in order, with no frame column
            // skipped (the image is at least as wide as the frame).
            let mut last = -1;
            for x in i64::from(p.left)..i64::from(p.left + p.image_w) {
                let FramePos::Inside(at) = p.to_frame(x, i64::from(p.top)) else {
                    panic!("{x} outside");
                };
                assert!(at.x == last || at.x == last + 1 || (!up && at.x > last));
                last = at.x;
            }
            assert_eq!(last, fw as i32 - 1);
            // The bars are outside.
            assert_eq!(p.to_frame(i64::from(p.left) - 1, 0), FramePos::Outside);
            assert_eq!(
                p.to_frame(i64::from(p.left + p.image_w), i64::from(p.top)),
                FramePos::Outside
            );
        }
    }
}
