//! Visual A/B diff for D2RUST screenshots.
//!
//! A lane takes the same shots from two builds (for example `main` and its
//! branch, with `play --offscreen --shot-at ...`) and compares them here.
//! Shots are game art, so they stay in local artifact folders. Q6 can retain
//! local reference captures; neither references nor diffs enter the repository.
//!
//! A pixel counts as changed when any of its R, G, B or A channels differs
//! by more than the colour tolerance. A pair passes when the changed share of
//! the compared pixels is at most the allowed percentage. Masks remove named
//! screen regions (the HUD, the belt, a border strip) from the comparison,
//! so a lane can say "everything outside the HUD is unchanged".
//!
//! The library holds the rules (masks, compare, verdict); `main.rs` is the
//! command line and the folder walk.

use image::{Rgba, RgbaImage};
use serde_json::{json, Value};

pub mod check;
pub mod probe;

/// A pixel rectangle, `x`/`y` its top left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x - self.x < self.w && y - self.y < self.h
    }

    /// This rectangle cut to a `width` x `height` image.
    fn clip(self, width: u32, height: u32) -> Rect {
        let x = self.x.min(width);
        let y = self.y.min(height);
        Rect {
            x,
            y,
            w: self.w.min(width - x),
            h: self.h.min(height - y),
        }
    }

    fn json(&self) -> Value {
        json!({ "x": self.x, "y": self.y, "w": self.w, "h": self.h })
    }
}

/// The game draws a frame 600 lines high and scales it to the window; the
/// HUD and panels are laid out in an 800-column box centred in that frame.
/// The named masks are rectangles in that 800 x 600 box, scaled to the
/// shot's height. In a frame wider than the box (16:9) the control panel's
/// stone bar runs on to both sides of the frame, so `hud` spans the full
/// shot width; `belt` stays in the box.
const UI_BOX_W: u32 = 800;
const UI_BOX_H: u32 = 600;

/// The control panel with the life and mana globes: the bottom 104 lines
/// of the frame, widened to the full shot width (see above). Generous on
/// purpose: it also covers the skill buttons and the run/walk and
/// mini-panel buttons.
const HUD_BOX: Rect = Rect {
    x: 0,
    y: 496,
    w: 800,
    h: 104,
};

/// The belt and its pop-up rows: four columns right of the centre and up
/// to four rows of boxes above the panel.
const BELT_BOX: Rect = Rect {
    x: 420,
    y: 456,
    w: 132,
    h: 144,
};

/// A region left out of the comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mask {
    /// The control panel and globes ([`HUD_BOX`], full shot width).
    Hud,
    /// The belt and its pop-up rows ([`BELT_BOX`]).
    Belt,
    /// A border `N` pixels wide on all four sides of the shot.
    Edges(u32),
    /// An exact rectangle in shot pixels: `rect:X,Y,W,H`.
    Rect(Rect),
}

impl Mask {
    /// Parses one mask name: `hud`, `belt`, `edges:N`, `rect:X,Y,W,H`.
    pub fn parse(s: &str) -> Result<Mask, String> {
        let s = s.trim();
        if s == "hud" {
            return Ok(Mask::Hud);
        }
        if s == "belt" {
            return Ok(Mask::Belt);
        }
        if let Some(n) = s.strip_prefix("edges:") {
            return n
                .parse()
                .map(Mask::Edges)
                .map_err(|_| format!("bad edge width in mask `{s}`"));
        }
        if let Some(r) = s.strip_prefix("rect:") {
            let v: Vec<u32> = r
                .split(',')
                .map(|p| p.trim().parse())
                .collect::<Result<_, _>>()
                .map_err(|_| format!("bad numbers in mask `{s}`"))?;
            if let [x, y, w, h] = v[..] {
                return Ok(Mask::Rect(Rect { x, y, w, h }));
            }
            return Err(format!("mask `{s}` needs rect:X,Y,W,H"));
        }
        Err(format!(
            "unknown mask `{s}` (hud, belt, edges:N, rect:X,Y,W,H)"
        ))
    }

    /// Parses a comma-separated mask list. `rect:` takes the rest of its
    /// item's commas, so it goes last or in its own `--mask`.
    pub fn parse_list(s: &str) -> Result<Vec<Mask>, String> {
        let mut out = Vec::new();
        let mut rest = s;
        while !rest.is_empty() {
            if rest.starts_with("rect:") {
                out.push(Mask::parse(rest)?);
                break;
            }
            let (item, tail) = rest.split_once(',').unwrap_or((rest, ""));
            out.push(Mask::parse(item)?);
            rest = tail;
        }
        Ok(out)
    }

    /// The pixel rectangles this mask covers in a `width` x `height` shot.
    pub fn rects(&self, width: u32, height: u32) -> Vec<Rect> {
        match self {
            Mask::Hud => {
                let r = ui_box_rect(HUD_BOX, width, height);
                vec![Rect {
                    x: 0,
                    w: width,
                    ..r
                }]
            }
            Mask::Belt => vec![ui_box_rect(BELT_BOX, width, height)],
            Mask::Edges(n) => {
                let n = *n;
                vec![
                    Rect {
                        x: 0,
                        y: 0,
                        w: width,
                        h: n,
                    },
                    Rect {
                        x: 0,
                        y: height.saturating_sub(n),
                        w: width,
                        h: n,
                    },
                    Rect {
                        x: 0,
                        y: 0,
                        w: n,
                        h: height,
                    },
                    Rect {
                        x: width.saturating_sub(n),
                        y: 0,
                        w: n,
                        h: height,
                    },
                ]
            }
            Mask::Rect(r) => vec![*r],
        }
        .into_iter()
        .map(|r| r.clip(width, height))
        .collect()
    }
}

/// The mask as written on the command line.
impl std::fmt::Display for Mask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mask::Hud => write!(f, "hud"),
            Mask::Belt => write!(f, "belt"),
            Mask::Edges(n) => write!(f, "edges:{n}"),
            Mask::Rect(r) => write!(f, "rect:{},{},{},{}", r.x, r.y, r.w, r.h),
        }
    }
}

/// A rectangle of the 800 x 600 UI box placed in a `width` x `height` shot:
/// scaled by `height / 600`, the box centred horizontally.
fn ui_box_rect(r: Rect, width: u32, height: u32) -> Rect {
    let scale = |v: u32| (u64::from(v) * u64::from(height) / u64::from(UI_BOX_H)) as u32;
    let box_w = scale(UI_BOX_W);
    let left = width.saturating_sub(box_w) / 2;
    let x0 = left + scale(r.x);
    let y0 = scale(r.y);
    // Round the far edge up so the scaled rectangle never loses a line.
    let up = |v: u32| (u64::from(v) * u64::from(height)).div_ceil(u64::from(UI_BOX_H)) as u32;
    Rect {
        x: x0,
        y: y0,
        w: (left + up(r.x + r.w)).saturating_sub(x0),
        h: up(r.y + r.h).saturating_sub(y0),
    }
}

/// How strict a comparison is.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Largest per-channel difference that still counts as the same colour.
    pub tol: u8,
    /// Largest share of changed pixels, in percent of the compared pixels,
    /// that still passes.
    pub max_pct: f64,
    /// Regions left out of the comparison.
    pub masks: Vec<Mask>,
}

impl Default for Options {
    /// Strict: exact colours, no changed pixel allowed, nothing masked.
    fn default() -> Self {
        Options {
            tol: 0,
            max_pct: 0.0,
            masks: Vec::new(),
        }
    }
}

/// The result of comparing one pair of shots.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub width: u32,
    pub height: u32,
    /// Pixels outside every mask.
    pub compared: u64,
    /// Compared pixels whose colour differs by more than the tolerance.
    pub changed: u64,
    /// `changed` in percent of `compared` (0 when nothing is compared).
    pub changed_pct: f64,
    /// Largest per-channel difference over the compared pixels.
    pub max_delta: u8,
    /// The smallest rectangle holding every changed pixel.
    pub changed_box: Option<Rect>,
    pub pass: bool,
}

impl Outcome {
    pub fn json(&self) -> Value {
        json!({
            "pass": self.pass,
            "width": self.width,
            "height": self.height,
            "compared": self.compared,
            "changed": self.changed,
            "changed_pct": self.changed_pct,
            "max_delta": self.max_delta,
            "changed_box": self.changed_box.map(|r| r.json()),
        })
    }
}

/// Why a pair could not be compared pixel by pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeMismatch {
    pub a: (u32, u32),
    pub b: (u32, u32),
}

/// Colour of a changed pixel in the diff image.
const CHANGED: Rgba<u8> = Rgba([255, 0, 0, 255]);

/// Compares `a` (before) with `b` (after). Returns the outcome and the diff
/// image: `b` dimmed to grey, changed pixels red, masked pixels darker
/// still and tinted blue so the left-out regions show.
pub fn compare(
    a: &RgbaImage,
    b: &RgbaImage,
    opts: &Options,
) -> Result<(Outcome, RgbaImage), SizeMismatch> {
    let (width, height) = a.dimensions();
    if b.dimensions() != (width, height) {
        return Err(SizeMismatch {
            a: (width, height),
            b: b.dimensions(),
        });
    }
    let masked: Vec<Rect> = opts
        .masks
        .iter()
        .flat_map(|m| m.rects(width, height))
        .collect();
    let mut diff = RgbaImage::new(width, height);
    let (mut compared, mut changed, mut max_delta) = (0u64, 0u64, 0u8);
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (x, y, pb) in b.enumerate_pixels() {
        let pa = a.get_pixel(x, y);
        let grey = luma(pb) / 3 + 40;
        if masked.iter().any(|r| r.contains(x, y)) {
            diff.put_pixel(x, y, Rgba([grey / 3, grey / 3, grey / 2 + 40, 255]));
            continue;
        }
        compared += 1;
        let delta =
            pa.0.iter()
                .zip(pb.0.iter())
                .map(|(&p, &q)| p.abs_diff(q))
                .max()
                .unwrap_or(0);
        max_delta = max_delta.max(delta);
        if delta > opts.tol {
            changed += 1;
            diff.put_pixel(x, y, CHANGED);
            bounds = Some(match bounds {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        } else {
            diff.put_pixel(x, y, Rgba([grey, grey, grey, 255]));
        }
    }
    let changed_pct = if compared == 0 {
        0.0
    } else {
        changed as f64 * 100.0 / compared as f64
    };
    let outcome = Outcome {
        width,
        height,
        compared,
        changed,
        changed_pct,
        max_delta,
        changed_box: bounds.map(|(x0, y0, x1, y1)| Rect {
            x: x0,
            y: y0,
            w: x1 - x0 + 1,
            h: y1 - y0 + 1,
        }),
        pass: changed_pct <= opts.max_pct,
    };
    Ok((outcome, diff))
}

/// Integer luma of a pixel, 0..=255.
fn luma(p: &Rgba<u8>) -> u8 {
    let [r, g, b, _] = p.0;
    ((u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000) as u8
}

/// The pairs of a folder compare: every `.png` name in either folder, in
/// name order, with whether it is in `a` and in `b`. A name in only one
/// folder fails the run (a shot went missing or appeared).
pub fn pair_names(a: &[String], b: &[String]) -> Vec<(String, bool, bool)> {
    let mut all: Vec<String> = a.iter().chain(b.iter()).cloned().collect();
    all.sort();
    all.dedup();
    all.into_iter()
        .map(|n| {
            let (ia, ib) = (a.contains(&n), b.contains(&n));
            (n, ia, ib)
        })
        .collect()
}

#[cfg(test)]
mod tests;
