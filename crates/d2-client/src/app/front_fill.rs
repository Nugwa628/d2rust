//! The fill beside the front end's picture on a wide screen.
//!
//! The menus, character screens, credits and loading picture are drawn at
//! 800 × 600, a 4:3 picture. A 16:9 window shows it centred at its own
//! shape ([`super::front_host::front_presentation`]), leaving the window's
//! sides empty. With `[video] menu_sides = "black"` (the default) they stay
//! black. With `"blur"` they show a dim, blurred copy of the picture,
//! stretched to cover the whole window behind it, so the art seems to
//! carry on past its edges instead of stopping at a hard bar.
//!
//! The blur is made on the CPU and costs almost nothing: the picture is
//! averaged into a 40 × 30 grid of 20 × 20 blocks (25 samples each),
//! softened once more with a 3 × 3 box over the grid and darkened. The
//! grid is shown as an ordinary sprite with linear filtering, which every
//! GPU draws, so there is no shader of our own. The picture is sampled
//! before the pointer is drawn, so the pointer leaves no smudge in the
//! fill.
//!
//! A window of 4:3 or narrower never shows the fill: its bars stay black
//! and its output is exactly what it was without the fill.

use crate::app::config::MenuSides;

use super::front_host::{front_presentation, HEIGHT, WIDTH};

/// The fill grid's size: one cell per [`BLOCK`] × [`BLOCK`] picture pixels.
pub const FILL_W: u32 = WIDTH / BLOCK;
pub const FILL_H: u32 = HEIGHT / BLOCK;

/// Picture pixels per grid cell, on each axis.
const BLOCK: u32 = 20;

/// Distance between the samples of a cell, on each axis (5 × 5 samples).
const STEP: u32 = 4;

/// The fill's brightness, `DIM_NUM / DIM_DEN` of the blurred picture: dark
/// enough that the picture itself stays the clear centre of the screen.
const DIM_NUM: u32 = 7;
const DIM_DEN: u32 = 20;

/// Whether the window shows the fill: the player chose it, the window is
/// wider than 4:3 and it holds the picture.
pub fn shown(sides: MenuSides, window_w: u32, window_h: u32) -> bool {
    sides == MenuSides::Blur
        && u64::from(window_w) * u64::from(HEIGHT) > u64::from(window_h) * u64::from(WIDTH)
        && front_presentation(window_w, window_h).is_ok()
}

/// The scale (physical pixels per grid cell) that stretches the grid over
/// the whole `window_w × window_h` window, keeping its 4:3 shape and
/// centred: the larger of the two axis ratios, so it covers the window and
/// overflows on the other axis.
pub fn cover_scale(window_w: u32, window_h: u32) -> f32 {
    (window_w as f32 / FILL_W as f32).max(window_h as f32 / FILL_H as f32)
}

/// The fill grid (RGBA, `FILL_W × FILL_H`, opaque) of the picture whose
/// pixel (x, y) has colour `colour(x, y)`.
pub fn blur(colour: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let (w, h) = (FILL_W as usize, FILL_H as usize);
    // Block averages.
    let mut avg = vec![[0u32; 3]; w * h];
    let per = BLOCK / STEP;
    for (i, cell) in avg.iter_mut().enumerate() {
        let (bx, by) = ((i % w) as u32 * BLOCK, (i / w) as u32 * BLOCK);
        let mut sum = [0u32; 3];
        for sy in 0..per {
            for sx in 0..per {
                let c = colour(bx + STEP / 2 + sx * STEP, by + STEP / 2 + sy * STEP);
                for k in 0..3 {
                    sum[k] += u32::from(c[k]);
                }
            }
        }
        *cell = sum.map(|s| s / (per * per));
    }
    // A 3 × 3 box over the grid (edges repeated), then dimmed.
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 3];
            for dy in [-1, 0, 1] {
                for dx in [-1, 0, 1] {
                    let cx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                    let cy = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
                    for k in 0..3 {
                        sum[k] += avg[cy * w + cx][k];
                    }
                }
            }
            for s in sum {
                out.push((s * DIM_NUM / (9 * DIM_DEN)) as u8);
            }
            out.push(255);
        }
    }
    out
}

/// [`blur`] of an 800 × 600 index frame shown with `palette` (black
/// without one).
pub fn blur_index(index: &[u8], palette: Option<&d2_formats::palette::Palette>) -> Vec<u8> {
    blur(|x, y| {
        let i = index
            .get((y * WIDTH + x) as usize)
            .copied()
            .unwrap_or_default();
        palette.map_or([0; 3], |p| {
            let c = p.colors[usize::from(i)];
            [c.r, c.g, c.b]
        })
    })
}

/// [`blur`] of an 800 × 600 RGBA picture.
pub fn blur_rgba(px: &[u8]) -> Vec<u8> {
    blur(|x, y| {
        let o = ((y * WIDTH + x) * 4) as usize;
        px.get(o..o + 3).map_or([0; 3], |c| [c[0], c[1], c[2]])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fill_shows_only_when_chosen_and_the_window_is_wider_than_4_3() {
        for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160), (1366, 768)] {
            assert!(shown(MenuSides::Blur, w, h), "{w}x{h}");
            assert!(!shown(MenuSides::Black, w, h), "{w}x{h}");
        }
        // 4:3 (and taller) windows keep their black bars.
        for (w, h) in [
            (800, 600),
            (1024, 768),
            (1600, 1200),
            (2400, 1800),
            (1280, 1024),
        ] {
            assert!(!shown(MenuSides::Blur, w, h), "{w}x{h}");
        }
        // A window with no height presents nothing.
        assert!(!shown(MenuSides::Blur, 1920, 0));
    }

    #[test]
    fn the_grid_covers_the_window_and_keeps_its_shape() {
        for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160), (800, 600)] {
            let s = cover_scale(w, h);
            let (gw, gh) = (FILL_W as f32 * s, FILL_H as f32 * s);
            assert!(gw >= w as f32 && gh >= h as f32, "{w}x{h}");
            // One axis fits exactly.
            assert!(gw == w as f32 || gh == h as f32, "{w}x{h}");
        }
        // 16:9: the height overflows, the width fits.
        assert_eq!(cover_scale(1920, 1080), 48.0);
    }

    #[test]
    fn a_flat_picture_blurs_to_its_colour_dimmed() {
        let fill = blur(|_, _| [200, 100, 40]);
        assert_eq!(fill.len(), (FILL_W * FILL_H * 4) as usize);
        for c in fill.chunks(4) {
            assert_eq!(c, [70, 35, 14, 255]);
        }
        assert!(blur(|_, _| [0; 3]).chunks(4).all(|c| c == [0, 0, 0, 255]));
    }

    #[test]
    fn the_blur_follows_the_picture_and_softens_its_edges() {
        // Left half white, right half black.
        let fill = blur(|x, _| if x < WIDTH / 2 { [255; 3] } else { [0; 3] });
        let at = |x: u32, y: u32| fill[((y * FILL_W + x) * 4) as usize];
        let mid = FILL_W / 2;
        assert_eq!(at(0, 5), at(mid - 2, 5));
        assert!(at(mid - 1, 5) < at(mid - 2, 5));
        assert!(at(mid, 5) > 0);
        assert_eq!(at(FILL_W - 1, 5), 0);
    }

    #[test]
    fn index_and_rgba_pictures_blur_alike() {
        let mut pal = d2_formats::palette::Palette {
            colors: [Default::default(); 256],
        };
        pal.colors[7] = d2_formats::palette::Rgb {
            r: 90,
            g: 180,
            b: 30,
        };
        let index: Vec<u8> = (0..WIDTH * HEIGHT)
            .map(|i| if (i % WIDTH) < 300 { 7 } else { 0 })
            .collect();
        let rgba: Vec<u8> = index
            .iter()
            .flat_map(|&i| {
                let c = pal.colors[usize::from(i)];
                [c.r, c.g, c.b, 255]
            })
            .collect();
        assert_eq!(blur_index(&index, Some(&pal)), blur_rgba(&rgba));
        assert!(blur_index(&index, None)
            .chunks(4)
            .all(|c| c == [0, 0, 0, 255]));
    }
}
