//! View geometry: the marker rectangle and mini origin, the panel
//! side, the drawing origin, plus the size setter and the re-centre.

use super::options::Options;
use crate::rules::camera::{ClientPos, PANEL_HEIGHT};

/// Scale divisor.
pub const DIV_FULL: i32 = 10;
pub const DIV_MINI: i32 = 20;

/// The per-frame facts the view reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameFacts {
    /// Display width W and height H.
    pub width: i32,
    pub height: i32,
    /// The screen open mode 0–3.
    pub open_mode: u8,
    /// ≠ 0: the mini map moves down 96 px ([`mini_down`]).
    pub mini_down: bool,
    /// The unit origin (cx, cy).
    pub unit_origin: ClientPos,
}

impl FrameFacts {
    /// Hp = H − 40.
    pub fn play_height(&self) -> i32 {
        self.height - PANEL_HEIGHT
    }

    /// How far an open side panel moves the map: the view's panel move
    /// ([`crate::rules::camera::FrameSize::side_panel_span`] / 2), `W / 4`
    /// at 4:3.
    pub fn panel_shift(&self) -> i32 {
        crate::rules::camera::FrameSize {
            width: self.width,
            height: self.height,
        }
        .side_panel_span()
            / 2
    }

    /// Whether the automap shows in this open mode: as the world, unless
    /// both side panels cover the frame (open mode 3 at 4:3).
    pub fn draws_world(&self) -> bool {
        let f = crate::rules::camera::FrameSize {
            width: self.width,
            height: self.height,
        };
        let mode = crate::rules::camera::OpenMode::new(self.open_mode).unwrap_or_default();
        f.draws_world(mode)
    }

    /// Both side panels are open over a frame that still shows the world
    /// between them (open mode 3 in a frame wider than 4:3): the mini map
    /// sits centred in that gap.
    pub fn mini_centred(&self) -> bool {
        self.open_mode == 3 && self.draws_world()
    }
}

/// An inclusive rectangle `[left, right] × [top, bottom]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Bounds {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.left <= x && x <= self.right && self.top <= y && y <= self.bottom
    }
}

/// The view state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct View {
    /// Mini (1) or full screen (0).
    pub mini: bool,
    /// The scale divisor.
    pub div: i32,
    /// (ox, oy), the view offset.
    pub offset: (i32, i32),
    /// (mx, my).
    pub mini_origin: (i32, i32),
    /// The marker rectangle.
    pub marker: Bounds,
    /// The values compares with: the last frame facts' result and last `Left` (a
    /// copy made at init).
    last_down: bool,
    last_left: bool,
    /// The last [`FrameFacts::mini_centred`]: a change re-centres too.
    last_centred: bool,
    last_mini: bool,
    last_centers: bool,
    last_size: Option<(i32, i32)>,
    /// The `Left` value the panel side remembers while it forces one.
    saved_left: Option<bool>,
}

impl View {
    /// The view after UI init: full size, divisor 10, offset (0, 0).
    pub fn new(opts: &Options) -> Self {
        View {
            mini: false,
            div: DIV_FULL,
            offset: (0, 0),
            mini_origin: (0, 0),
            marker: Bounds::default(),
            last_down: false,
            last_left: opts.left,
            last_centred: false,
            last_mini: false,
            last_centers: opts.centers,
            last_size: None,
            saved_left: None,
        }
    }

    /// Geometry: mini origin and marker rectangle.
    pub fn compute(&mut self, opts: &Options, f: &FrameFacts) {
        let (w, h) = (f.width, f.height);
        self.mini_origin = if f.mini_centred() {
            ((w - w / 3) / 2, 78)
        } else if !opts.left {
            (2 * w / 3, 78)
        } else {
            (0, if f.mini_down { 96 } else { 0 })
        };
        let (mx, my) = self.mini_origin;
        self.marker = if self.mini {
            Bounds {
                left: mx - 8,
                top: my,
                right: w / 3 + mx,
                bottom: h / 3 + my,
            }
        } else {
            Bounds {
                left: -16,
                top: -16,
                right: w,
                bottom: h,
            }
        };
    }

    /// Runs when `force` or `AutoMap Centers`.
    pub fn recentre(&mut self, force: bool, opts: &Options, f: &FrameFacts) {
        if !force && !opts.centers {
            return;
        }
        self.compute(opts, f);
        let (mx, my) = self.mini_origin;
        self.offset = if self.mini {
            (f.width / 3 - mx - 16, f.height / 3 - my - 16)
        } else {
            (0, 0)
        };
    }

    /// (each automap frame): a change of the mini map's down shift (mini only)
    /// or of `Left` stores them and re-centres with force 0; then the geometry
    /// is recomputed.
    pub fn frame(&mut self, opts: &Options, f: &FrameFacts) {
        let centred = f.mini_centred();
        if (self.mini && f.mini_down != self.last_down)
            || opts.left != self.last_left
            || centred != self.last_centred
            || self.mini != self.last_mini
            || (opts.centers && !self.last_centers)
            || self.last_size.is_some_and(|s| s != (f.width, f.height))
        {
            self.last_down = f.mini_down;
            self.last_left = opts.left;
            self.last_centred = centred;
            self.recentre(false, opts, f);
        }
        self.last_size = Some((f.width, f.height));
        self.last_mini = self.mini;
        self.last_centers = opts.centers;
        self.compute(opts, f);
    }

    /// On change, divisor 20 mini / 10 full and a
    /// re-centre with force 0. Returns whether the size changed (the cel
    /// files are then reloaded).
    pub fn set_size(&mut self, mini: bool, opts: &Options, f: &FrameFacts) -> bool {
        if self.mini == mini {
            return false;
        }
        self.mini = mini;
        self.div = if mini { DIV_MINI } else { DIV_FULL };
        self.recentre(false, opts, f);
        true
    }

    /// Open mode 1 forces `Left`:= 1, mode 2 forces 0, modes 0 and 3 restore
    /// it. Returns s: ±W/4 in full mode (+W/4 in open mode 1, −W/4 in open mode
    /// 2), else 0.
    ///
    /// Reading: the old value is remembered when a force starts and
    /// restored once; the size setter is not involved (no registry
    /// write).
    pub fn panel_side(&mut self, opts: &mut Options, f: &FrameFacts) -> i32 {
        match f.open_mode {
            1 | 2 => {
                if self.saved_left.is_none() {
                    self.saved_left = Some(opts.left);
                }
                opts.left = f.open_mode == 1;
            }
            _ => {
                if let Some(v) = self.saved_left.take() {
                    opts.left = v;
                }
            }
        }
        if self.mini {
            return 0;
        }
        match f.open_mode {
            1 => f.panel_shift(),
            2 => -f.panel_shift(),
            _ => 0,
        }
    }

    /// (Ax, Ay) for panel side `s`.
    pub fn origin(&self, s: i32, f: &FrameFacts) -> (i32, i32) {
        let (cx, cy) = (f.unit_origin.x, f.unit_origin.y);
        (
            self.offset.0 + 40 + (cx / self.div - f.width / 2 + s),
            self.offset.1 + 15 + (cy / self.div - f.play_height() / 2),
        )
    }

    /// The screen (X, Y) of cell (x, y) for origin `a`.
    pub fn cell_screen(&self, x: i16, y: i16, a: (i32, i32)) -> (i32, i32) {
        (
            i32::from(x) * 10 / self.div - a.0,
            i32::from(y) * 10 / self.div - a.1,
        )
    }
}

/// The party-portrait state ≠ 2 (the portrait pass is skipped at
/// 2), so the left mini map moves down 96 px whenever the portraits are not
/// hidden. Gives [`FrameFacts::mini_down`].
pub fn mini_down(portrait_state: u32) -> bool {
    portrait_state != 2
}

#[cfg(test)]
mod adaptive_size_tests {
    use super::*;

    fn facts(w: i32, h: i32, open_mode: u8) -> FrameFacts {
        FrameFacts {
            width: w,
            height: h,
            open_mode,
            mini_down: false,
            unit_origin: ClientPos { x: 1000, y: 1000 },
        }
    }

    #[test]
    fn size_changes_recentre_mini_and_full_views_after_a_manual_pan() {
        let opts = Options::default();
        assert!(opts.centers);
        let frames = [
            (800, 600),
            (960, 540),
            (1280, 720),
            (800, 1600),
            (1720, 720),
        ];
        for (w, h) in frames {
            let f = facts(w, h, 0);
            let mut v = View::new(&opts);
            v.set_size(true, &opts, &f);
            v.frame(&opts, &f);
            assert_eq!(v.mini_origin, (0, 0));
            assert_eq!(v.offset, (w / 3 - 16, h / 3 - 16));
            v.offset = (999, 999);
            let idx = frames.iter().position(|x| *x == (w, h)).unwrap();
            let (nw, nh) = frames[(idx + 1) % frames.len()];
            let g = facts(nw, nh, 0);
            v.frame(&opts, &g);
            assert_eq!(v.mini_origin, (0, 0));
            assert_eq!(v.offset, (nw / 3 - 16, nh / 3 - 16));
            assert_eq!(
                v.marker,
                Bounds {
                    left: -8,
                    top: 0,
                    right: nw / 3,
                    bottom: nh / 3
                }
            );
        }
    }

    #[test]
    fn open_panels_shift_the_view_and_centre_the_mini_map_on_wide_frames() {
        let mut opts = Options::default();
        for (w, h) in [(960, 540), (1280, 720), (1720, 720)] {
            let f = facts(w, h, 1);
            let mut v = View::new(&opts);
            v.set_size(false, &opts, &f);
            let s = v.panel_side(&mut opts, &f);
            assert_eq!(s, f.panel_shift());
            assert_eq!(
                v.origin(s, &f),
                (
                    v.offset.0 + 40 + (f.unit_origin.x / v.div - w / 2 + s),
                    v.offset.1 + 15 + (f.unit_origin.y / v.div - f.play_height() / 2)
                )
            );
            let centred = facts(w, h, 3);
            assert!(centred.mini_centred());
            let mut mini = View::new(&opts);
            mini.set_size(true, &opts, &centred);
            mini.frame(&opts, &centred);
            assert_eq!(mini.mini_origin, ((w - w / 3) / 2, 78));
            assert_eq!(
                mini.offset,
                (
                    w / 3 - mini.mini_origin.0 - 16,
                    h / 3 - mini.mini_origin.1 - 16
                )
            );
        }
    }
}
