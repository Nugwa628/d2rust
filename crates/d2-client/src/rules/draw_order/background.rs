//! Level backgrounds (pass 1) and the empty pass 8.
//!
//! Two levels draw a backdrop behind the map: the Arcane Sanctuary (level
//! 74) floats in a field of 256 one-pixel stars drifting left, the Arreat
//! Summit (level 120) stands before a scrolling strip of mountain cels with
//! 10 clouds passing. Each backdrop keeps its own seed, taken from the
//! clock at its first use; here that seed is an input (a captured state
//! supplies it), never read from the clock, and so is the millisecond
//! tick of each call. The draw functions return the pass-1 items as plain
//! data in build order; [`keyed`] gives them their `DrawKey` (pass 1,
//! major 0, minor = build order).

use d2_formats::palette::Palette;
use d2_sim::rng::Seed;

use super::OrderKey;
use crate::rules::shading::nearest;
use crate::scene::order::pass;

const SPEC: &str = "render/draw-order-2.md";

/// Level ids that draw a background.
pub const ARCANE_SANCTUARY: u32 = 74;
pub const ARREAT_SUMMIT: u32 = 120;

/// Stars: count and color ramp size.
pub const STARS: usize = 256;
pub const STAR_COLORS: usize = 8;
/// Stars move once more than 40 ms have passed since their last move.
pub const STAR_TICK_MS: u32 = 40;

/// The player x the mountain strip is anchored at: the strip scrolls at
/// 1/8 of the player's movement from there.
pub const SUMMIT_ANCHOR_X: i32 = 10_063;
/// Mountain cel width.
pub const SUMMIT_TILE: i32 = 256;
pub const SUMMIT_FILE: &str = "data\\global\\ui\\summit01";
pub const CLOUD_FILE: &str = "data\\global\\ui\\cloud01";
pub const CLOUDS: usize = 10;
/// Cloud wrap: past `x16 > 16·(W + 368)` a cloud restarts at `x16 =
/// −5,888` (368 pixels left of the frame) with a new height.
pub const CLOUD_WRAP: i32 = 368;
pub const CLOUD_RESET_X16: i32 = -5_888;
/// Mountains draw unlit (light −1) with draw mode 5, clouds with a
/// light grey light and draw mode 3.
pub const SUMMIT_LIGHT: u32 = 0xFFFF_FFFF;
pub const SUMMIT_DRAW_MODE: u8 = 5;
pub const CLOUD_LIGHT: u32 = 0xDDDD_DDDD;
pub const CLOUD_DRAW_MODE: u8 = 3;

/// Whether pass 8 runs: it has no content in D2RUST.
pub const PASS8_RUNS: bool = false;

/// Errors of the background pass: an input the frame lacks or a question
/// the design leaves open (never defaulted).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackgroundError {
    /// The first call of a background needs a seed taken from the clock;
    /// a check run passes it in, so a missing one is an error.
    #[error("TODO(spec: {SPEC} §12): level {level} background first use without a recorded seed")]
    NoSeed { level: u32 },
    /// The star colors need the palette `nearest` searches.
    #[error("TODO(spec: {SPEC} §12 r1): star colors need the palette `nearest` reads")]
    NoPalette,
}

/// One pass-1 draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackgroundDraw {
    /// A line; the stars draw (x, y) → (x, y).
    Line {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: u8,
        alpha: u8,
    },
    /// A cel of `file` (loaded once) at `(x, y)` with light and draw mode.
    Cel {
        file: &'static str,
        frame: u32,
        x: i32,
        y: i32,
        light: u32,
        draw_mode: u8,
    },
}

/// A pass-1 draw with its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackgroundItem {
    pub key: OrderKey,
    pub draw: BackgroundDraw,
}

/// Keys pass-1 draws in build order.
pub fn keyed(draws: Vec<BackgroundDraw>) -> Vec<BackgroundItem> {
    draws
        .into_iter()
        .enumerate()
        .map(|(i, draw)| BackgroundItem {
            key: OrderKey {
                pass: pass::LEVEL_BACKGROUND,
                major: 0,
                minor: i as u32,
            },
            draw,
        })
        .collect()
}

/// The items of pass 8: none, the pass has no content.
pub fn pass8_items() -> Vec<BackgroundItem> {
    Vec::new()
}

// ---------------------------------------------------------------- stars

/// One star.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Star {
    pub x: i32,
    pub y: i32,
    /// −1 … −5.
    pub speed: i32,
    pub color: u8,
}

/// The Arcane Sanctuary stars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stars {
    pub seed: Seed,
    pub colors: [u8; STAR_COLORS],
    pub stars: [Star; STARS],
    /// Millisecond tick of the last move.
    pub last: u32,
}

impl Stars {
    /// The first call's set-up: `seed` is the clock-derived seed, `last`
    /// the tick of the last move (an input; 0 before the first call).
    /// `w`, `h` are `W`, `H`.
    pub fn new(seed: Seed, last: u32, palette: &Palette, w: i32, h: i32) -> Self {
        let mut s = Stars {
            seed,
            colors: [0; STAR_COLORS],
            stars: [Star::default(); STARS],
            last,
        };
        for i in 0..STAR_COLORS as u32 {
            let base = 128 + (128 * i) / 7;
            let mut c = [0u32; 3];
            for v in &mut c {
                // base ≥ 128, so the sum never goes below 96.
                *v = (base + (s.seed.step() & 63) - 32).min(255);
            }
            s.colors[i as usize] = nearest(palette, c[0], c[1], c[2]);
        }
        for j in 0..STARS {
            s.make(j, false, w, h);
        }
        s
    }

    /// Makes star `j`: x (first set-up: `roll(W)`; a remake: `W − 1 +
    /// (step & 7)`), then y, speed, color.
    fn make(&mut self, j: usize, remake: bool, w: i32, h: i32) {
        let x = if remake {
            w - 1 + (self.seed.step() & 7) as i32
        } else {
            self.seed.roll(w) as i32
        };
        let y = self.seed.roll(h - 40) as i32;
        let speed = -1 - (self.seed.step() % 5) as i32;
        let color = self.colors[(self.seed.step() & 7) as usize];
        self.stars[j] = Star { x, y, speed, color };
    }

    /// Every call: one pixel line per star, then the move when
    /// more than 40 ms passed since `last` (unsigned difference).
    pub fn draw(&mut self, now: u32, w: i32, h: i32) -> Vec<BackgroundDraw> {
        let out = self
            .stars
            .iter()
            .map(|s| BackgroundDraw::Line {
                x0: s.x,
                y0: s.y,
                x1: s.x,
                y1: s.y,
                color: s.color,
                alpha: 0xFF,
            })
            .collect();
        if now.wrapping_sub(self.last) > STAR_TICK_MS {
            for j in 0..STARS {
                self.stars[j].x += self.stars[j].speed;
                if self.stars[j].x < 0 {
                    self.make(j, true, w, h);
                }
            }
            self.last = now;
        }
        out
    }
}

// ---------------------------------------------------------------- summit

/// One cloud: x in sixteenths of a pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Cloud {
    pub x16: i32,
    pub y: i32,
    /// 8 … 23.
    pub speed: i32,
}

/// The mountain strip position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SummitScroll {
    pub q: i32,
    pub c: i32,
    /// 0…3.
    pub t: i32,
    pub x0: i32,
}

/// `d = x − 10,063`, `q = d / 8`, `c = q rem 256`, `t = (q / 256) mod 4`
/// made non-negative, `x0 = −c` (C division and remainder).
pub fn summit_scroll(player_x: i32) -> SummitScroll {
    let d = player_x.wrapping_sub(SUMMIT_ANCHOR_X);
    let q = d / 8;
    let c = q % SUMMIT_TILE;
    let mut t = (q / SUMMIT_TILE) % 4;
    if t < 0 {
        t += 4;
    }
    SummitScroll { q, c, t, x0: -c }
}

/// Mountain cels per row for a frame `w` wide: 4 up to 800 wide, plus one
/// per 256 columns (or part) beyond, so the 16:9 frame's extra width is
/// covered too.
pub fn summit_cels(w: i32) -> i32 {
    4 + ((w - 800).max(0) + SUMMIT_TILE - 1) / SUMMIT_TILE
}

/// The Arreat Summit background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summit {
    pub seed: Seed,
    pub clouds: [Cloud; CLOUDS],
}

impl Summit {
    /// The clouds' first use: `seed` is the clock-derived
    /// seed; per cloud `x16 := roll(16·W)`, `y := step mod 250`, `speed :=
    /// (step & 15) + 8`.
    pub fn new(seed: Seed, w: i32) -> Self {
        let mut s = Summit {
            seed,
            clouds: [Cloud::default(); CLOUDS],
        };
        for c in &mut s.clouds {
            let x16 = s.seed.roll(16 * w) as i32;
            let y = (s.seed.step() % 250) as i32;
            let speed = (s.seed.step() & 15) as i32 + 8;
            *c = Cloud { x16, y, speed };
        }
        s
    }

    /// Every call: mountains row by row (y 256, 512, and 768 in
    /// resolution mode 2), then each cloud moved and drawn. The caller
    /// skips the call while the exit flag is set ([`Backgrounds::pass1`]).
    pub fn draw(&mut self, player_x: i32, w: i32, resolution_mode: u32) -> Vec<BackgroundDraw> {
        let mut out = Vec::new();
        let sc = summit_scroll(player_x);
        let rows: &[(i32, u32)] = if resolution_mode == 2 {
            &[(256, 0), (512, 4), (768, 8)]
        } else {
            &[(256, 0), (512, 4)]
        };
        let cel = |frame: u32, x: i32, y: i32| BackgroundDraw::Cel {
            file: SUMMIT_FILE,
            frame,
            x,
            y,
            light: SUMMIT_LIGHT,
            draw_mode: SUMMIT_DRAW_MODE,
        };
        let cels = summit_cels(w);
        for &(y, add) in rows {
            for k in 0..cels {
                let frame = ((sc.t + k) & 3) as u32 + add;
                out.push(cel(frame, sc.x0 + SUMMIT_TILE * k, y));
            }
            if sc.x0 > 0 {
                let frame = ((sc.t + 3) & 3) as u32 + add;
                out.push(cel(frame, sc.x0 - SUMMIT_TILE, y));
            }
        }
        for i in 0..CLOUDS {
            let mut c = self.clouds[i];
            c.x16 += c.speed;
            if c.x16 > 16 * (w + CLOUD_WRAP) {
                c.x16 = CLOUD_RESET_X16;
                c.y = (self.seed.step() % 300) as i32;
            }
            self.clouds[i] = c;
            let x = c.x16 / 16;
            for (frame, dx) in [(0, 0), (1, SUMMIT_TILE)] {
                out.push(BackgroundDraw::Cel {
                    file: CLOUD_FILE,
                    frame,
                    x: x + dx,
                    y: c.y,
                    light: CLOUD_LIGHT,
                    draw_mode: CLOUD_DRAW_MODE,
                });
            }
        }
        out
    }
}

// ---------------------------------------------------------------- pass 1

/// What pass 1 reads of a frame.
#[derive(Debug, Clone, Copy)]
pub struct BackgroundFrame<'a> {
    /// The local player's level id.
    pub level: u32,
    /// `W`, `H`.
    pub w: i32,
    pub h: i32,
    /// The resolution mode.
    pub resolution_mode: u32,
    /// `GetTickCount()` of this call.
    pub now: u32,
    /// Whether the client is exiting.
    pub exiting: bool,
    /// The local player's client x.
    pub player_x: i32,
    /// The seeds for a first use (level 74, level 120), from the clock.
    pub stars_seed: Option<Seed>,
    pub summit_seed: Option<Seed>,
    /// The star tick `last` before the first call: `None` = 0, the
    /// zero-filled start value.
    pub stars_last: Option<u32>,
    /// The palette `nearest` searches for the star colors.
    pub palette: Option<&'a Palette>,
}

/// The two backgrounds' state, kept across frames (first-call flags
/// for the stars and the summit).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Backgrounds {
    pub stars: Option<Stars>,
    pub summit: Option<Summit>,
}

impl Backgrounds {
    /// Pass 1 of a frame: the background of
    /// level 74 or 120, nothing for other levels.
    pub fn pass1(&mut self, f: &BackgroundFrame) -> Result<Vec<BackgroundItem>, BackgroundError> {
        let draws = match f.level {
            ARCANE_SANCTUARY => {
                if self.stars.is_none() {
                    let seed = f
                        .stars_seed
                        .ok_or(BackgroundError::NoSeed { level: f.level })?;
                    let last = f.stars_last.unwrap_or(0);
                    let palette = f.palette.ok_or(BackgroundError::NoPalette)?;
                    self.stars = Some(Stars::new(seed, last, palette, f.w, f.h));
                }
                let stars = self.stars.as_mut().expect("set above");
                stars.draw(f.now, f.w, f.h)
            }
            ARREAT_SUMMIT if !f.exiting => {
                if self.summit.is_none() {
                    let seed = f
                        .summit_seed
                        .ok_or(BackgroundError::NoSeed { level: f.level })?;
                    self.summit = Some(Summit::new(seed, f.w));
                }
                let summit = self.summit.as_mut().expect("set above");
                summit.draw(f.player_x, f.w, f.resolution_mode)
            }
            _ => Vec::new(),
        };
        Ok(keyed(draws))
    }
}

#[cfg(test)]
#[path = "background_tests.rs"]
mod tests;
