//! Exact Integer frames and the shipping boundary for adaptive canvases.
//!
//! Divide both physical dimensions by their largest common whole scale
//! that leaves at least 800 columns and 540 rows. An undersized window uses
//! the configured frame in Fit. Fit always keeps its configured frame.
//! Selection stays separate from publication: a frame is published only
//! after the renderer, UI and room streaming have accepted the change.

use super::camera::FrameSize;
use crate::ui::ScaleMode;

/// Adaptive selection is live on every canvas consumer.
pub const ENABLED: bool = true;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSelection {
    pub frame: FrameSize,
    pub mode: ScaleMode,
}

impl FrameSelection {
    /// Plan a canvas without changing any process-wide renderer state.
    pub fn for_window(width: u32, height: u32, normal: FrameSize, mode: ScaleMode) -> Self {
        if mode == ScaleMode::Fit || width < 800 || height < 540 {
            return Self {
                frame: normal,
                mode: ScaleMode::Fit,
            };
        }
        let mut a = width;
        let mut b = height;
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let limit = (width / 800).min(height / 540);
        let scale = (1..=limit)
            .rev()
            .find(|n| a.is_multiple_of(*n))
            .unwrap_or(1);
        Self {
            frame: FrameSize {
                width: (width / scale) as i32,
                height: (height / scale) as i32,
            },
            mode: ScaleMode::Integer,
        }
    }

    /// Current releases retain the fixed frame until integration is complete.
    pub fn shipping(width: u32, height: u32, normal: FrameSize, mode: ScaleMode) -> Self {
        if ENABLED {
            Self::for_window(width, height, normal, mode)
        } else {
            Self {
                frame: normal,
                mode,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Presentation;

    pub const MATRIX: [(u32, u32, i32, i32, u32); 12] = [
        (1920, 1080, 960, 540, 2),
        (3840, 2160, 960, 540, 4),
        (2560, 1440, 1280, 720, 2),
        (1920, 1200, 960, 600, 2),
        (2560, 1080, 1280, 540, 2),
        (3440, 1440, 1720, 720, 2),
        (5120, 2160, 1280, 540, 4),
        (1365, 767, 1365, 767, 1),
        (1919, 1079, 1919, 1079, 1),
        (800, 1600, 800, 1600, 1),
        (7680, 2160, 1920, 540, 4),
        (801, 541, 801, 541, 1),
    ];

    #[test]
    fn integer_matrix_fills_both_axes_with_exact_nearest_neighbour_blocks() {
        for (w, h, fw, fh, n) in MATRIX {
            let choice = FrameSelection::for_window(w, h, FrameSize::WIDE, ScaleMode::Integer);
            assert_eq!(
                choice.frame,
                FrameSize {
                    width: fw,
                    height: fh
                }
            );
            let p = Presentation::with_mode(w, h, fw as u32, fh as u32, choice.mode).unwrap();
            assert_eq!(
                (p.image_w, p.image_h, p.left, p.top, p.scale),
                (w, h, 0, 0, n)
            );
            for (x, y) in [(0, 0), (fw / 2, fh / 2), (fw - 1, fh - 1)] {
                let (wx, wy) = p.from_frame(x, y);
                assert_eq!(
                    (wx, wy),
                    (i64::from(x) * i64::from(n), i64::from(y) * i64::from(n))
                );
            }
            for larger in n + 1..=(w / 800).min(h / 540) {
                assert!(!w.is_multiple_of(larger) || !h.is_multiple_of(larger));
            }
        }
    }

    #[test]
    fn live_mode_switches_restore_the_normal_fit_frame() {
        for (w, h, ..) in MATRIX {
            for normal in FrameSize::ALL {
                let fit = FrameSelection::for_window(w, h, normal, ScaleMode::Fit);
                assert_eq!(
                    fit,
                    FrameSelection {
                        frame: normal,
                        mode: ScaleMode::Fit
                    }
                );
                let integer = FrameSelection::for_window(w, h, normal, ScaleMode::Integer);
                assert_eq!(
                    FrameSelection::for_window(w, h, normal, ScaleMode::Fit),
                    fit
                );
                assert_eq!(
                    FrameSelection::for_window(w, h, normal, ScaleMode::Integer),
                    integer
                );
            }
        }
    }

    #[test]
    fn undersized_windows_use_fit_even_when_the_normal_frame_could_fit_at_one() {
        for (w, h) in [(799, 600), (1920, 539), (640, 480), (1, 1), (0, 0)] {
            assert_eq!(
                FrameSelection::for_window(w, h, FrameSize::LOW, ScaleMode::Integer),
                FrameSelection {
                    frame: FrameSize::LOW,
                    mode: ScaleMode::Fit
                }
            );
        }
    }

    #[test]
    fn shipping_gate_selects_the_adaptive_frame() {
        assert!(std::hint::black_box(ENABLED));
        for (w, h, ..) in MATRIX {
            for mode in [ScaleMode::Fit, ScaleMode::Integer] {
                assert_eq!(
                    FrameSelection::shipping(w, h, FrameSize::WIDE, mode),
                    FrameSelection::for_window(w, h, FrameSize::WIDE, mode)
                );
            }
        }
    }
}
