//! Captures of a game frame as a high-resolution screen shows it, with and
//! without the native UI pass (`super::native_ui`), drawn on the CPU from
//! the frame's own draw list: `play --dump-draws DIR --dump-native WxH`.
//!
//! For each requested window size, the frame is placed as the game places
//! it (fit, `ui::frame::Presentation`) and two images of the picture's
//! size are written:
//! - `native-WxH-frame.png`: the whole frame (UI included) sharp-scaled,
//!   as the frame is shown without the pass;
//! - `native-WxH-ui.png`: the scene (the frame without its UI items)
//!   sharp-scaled, with the UI drawn over it at the output resolution by
//!   the pass's own algorithm (`gpu_compositor::ui_pass::emulate_ui`), as
//!   the frame is shown with the pass, with the art in use (enhanced text
//!   glyphs, `super::ui_art::current`).
//!
//! The sharp scaling filters in linear light, as the GPU's linear sampler
//! does on the presented sRGB image, and the UI is applied in the
//! palette's own units, as `native_ui.wgsl` does. No window and no GPU
//! are needed, so the captures can be taken at 2160p on any machine.

use std::path::Path;

use d2_formats::palette::Palette;

use crate::gpu_compositor::ui_pass::{emulate_ui, pack_ui, ui_blend, UiDraw, UiPixel};
use crate::scene::{DrawItem, FrameCycle};
use crate::ui::frame::{Presentation, ScaleMode};

use super::native_ui::{is_ui, TableKinds};
use super::sharp::sharp_offset;
use super::ui_art::{CelRef, UiArt};
use super::ViewAssets;

/// Atlas pages the capture may use (as many as the game's GPU path).
const MAX_PAGES: u32 = 64;

/// Parses `--dump-native`'s `WxH[,WxH...]`.
pub fn parse_sizes(text: &str) -> Result<Vec<(u32, u32)>, String> {
    text.split(',')
        .map(|s| {
            let (w, h) = s
                .split_once('x')
                .ok_or_else(|| format!("--dump-native: {s:?} is not WxH"))?;
            let n = |v: &str| {
                v.trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|&n| (1..=16384).contains(&n))
                    .ok_or_else(|| format!("--dump-native: {s:?} is not WxH"))
            };
            Ok((n(w)?, n(h)?))
        })
        .collect()
}

/// sRGB byte to linear light.
fn linear(v: u8) -> f32 {
    let c = f32::from(v) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light to sRGB (0..1).
fn gamma(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn byte(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The index image `pixels` (`fw × fh`) through `palette`, sharp-scaled to
/// `ow × oh`, in the palette's units (0..1 per channel).
pub fn sharp_scale(
    pixels: &[u8],
    fw: u32,
    fh: u32,
    palette: &Palette,
    ow: u32,
    oh: u32,
) -> Vec<[f32; 3]> {
    let lin: Vec<[f32; 3]> = pixels
        .iter()
        .map(|&i| {
            let c = palette.colors[usize::from(i)];
            [linear(c.r), linear(c.g), linear(c.b)]
        })
        .collect();
    let (sx, sy) = (ow as f32 / fw as f32, oh as f32 / fh as f32);
    // Per output column (row): the left texel and the right one's weight.
    let axis = |n: u32, s: f32, frame: u32| -> Vec<(i64, i64, f32)> {
        (0..n)
            .map(|o| {
                let t = (o as f32 + 0.5) / s;
                let base = t.floor();
                let p = base + sharp_offset(t - base, s.max(1.0));
                let left = (p - 0.5).floor();
                let w = p - 0.5 - left;
                let clamp = |v: f32| (v as i64).clamp(0, i64::from(frame) - 1);
                (clamp(left), clamp(left + 1.0), w)
            })
            .collect()
    };
    let (xs, ys) = (axis(ow, sx, fw), axis(oh, sy, fh));
    let at = |x: i64, y: i64| lin[(y * i64::from(fw) + x) as usize];
    let mut out = Vec::with_capacity((ow * oh) as usize);
    for &(y0, y1, wy) in &ys {
        for &(x0, x1, wx) in &xs {
            let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
            out.push(std::array::from_fn(|k| {
                let top = a[k] * (1.0 - wx) + b[k] * wx;
                let bottom = c[k] * (1.0 - wx) + d[k] * wx;
                gamma(top * (1.0 - wy) + bottom * wy)
            }));
        }
    }
    out
}

fn write_png(path: &Path, w: u32, h: u32, rgb: &[[f32; 3]]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = rgb.iter().flat_map(|c| c.map(byte)).collect();
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(&data))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes the two captures per window size of `sizes` into `dir` for the
/// frame `items` composed over `cycle` (a copy; the game's is untouched).
pub fn write_shots(
    dir: &Path,
    items: &[DrawItem],
    assets: &ViewAssets,
    cycle: &FrameCycle,
    blank_screen: bool,
    sizes: &[(u32, u32)],
    art: &dyn UiArt,
) -> Result<(), String> {
    let view = cycle.view();
    let compose = |list: &[DrawItem]| -> Result<Vec<u8>, String> {
        let mut c = cycle.clone();
        c.compose(blank_screen, list, &assets.frames, &assets.maps)
            .map(<[u8]>::to_vec)
            .map_err(|e| format!("compose: {e}"))
    };
    let frame = compose(items)?;
    let scene_items: Vec<DrawItem> = items.iter().filter(|i| !is_ui(i)).copied().collect();
    let scene = compose(&scene_items)?;
    let atlas = assets
        .frames
        .atlas(MAX_PAGES)
        .map_err(|e| format!("atlas: {e}"))?;
    let kinds = assets
        .shades
        .as_ref()
        .map(|s| TableKinds::measure(s, &assets.maps, &assets.palette));
    let cleared = cycle.plan(blank_screen).clear_after;
    for &(w, h) in sizes {
        let p = Presentation::with_mode(w, h, view.width, view.height, ScaleMode::Fit)
            .map_err(|e| format!("{w}x{h}: {e:?}"))?;
        let (ow, oh) = (p.image_w, p.image_h);
        let before = sharp_scale(&frame, view.width, view.height, &assets.palette, ow, oh);
        let picture = sharp_scale(&scene, view.width, view.height, &assets.palette, ow, oh);
        let place = crate::gpu_compositor::ui_pass::UiPlacement {
            frame_w: view.width,
            frame_h: view.height,
            out_w: ow,
            out_h: oh,
        };
        let scale = oh as f32 / view.height as f32;
        let draws: Vec<UiDraw<'_>> = if cleared {
            Vec::new()
        } else {
            items
                .iter()
                .filter(|i| is_ui(i))
                .map(|item| UiDraw {
                    item,
                    blend: ui_blend(item.blend, |b| kinds.as_ref().and_then(|k| k.kind(b))),
                    image: art.cel(
                        &CelRef {
                            file: assets.frames.owner(item.frame),
                            frames: &assets.frames,
                            fonts: &assets.fonts,
                        },
                        scale,
                    ),
                })
                .collect()
        };
        let mut packed = pack_ui(
            &draws,
            &assets.frames,
            &atlas.slots,
            atlas.page_count(),
            assets.maps.len() as u32,
            view,
            place,
        )
        .map_err(|e| format!("pack: {e}"))?;
        packed.set_rep(art.atlas());
        let ui: Vec<UiPixel> = emulate_ui(&packed, atlas.pages(), &assets.maps, &assets.palette);
        let after: Vec<[f32; 3]> = picture.iter().zip(&ui).map(|(c, u)| u.apply(*c)).collect();
        write_png(
            &dir.join(format!("native-{w}x{h}-frame.png")),
            ow,
            oh,
            &before,
        )?;
        write_png(&dir.join(format!("native-{w}x{h}-ui.png")), ow, oh, &after)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse() {
        assert_eq!(
            parse_sizes("3840x2160,1920x1080"),
            Ok(vec![(3840, 2160), (1920, 1080)])
        );
        assert!(parse_sizes("3840").is_err());
        assert!(parse_sizes("0x10").is_err());
    }

    #[test]
    fn the_light_conversions_round_trip_every_byte() {
        for v in 0..=255u8 {
            assert_eq!(byte(gamma(linear(v))), v);
        }
    }

    #[test]
    fn sharp_scaling_at_a_whole_number_is_nearest() {
        let mut colors = [d2_formats::palette::Rgb { r: 0, g: 0, b: 0 }; 256];
        for (i, c) in colors.iter_mut().enumerate() {
            *c = d2_formats::palette::Rgb {
                r: i as u8,
                g: i as u8,
                b: i as u8,
            };
        }
        let pal = Palette { colors };
        let px = [10u8, 200, 30, 90];
        let out = sharp_scale(&px, 2, 2, &pal, 6, 6);
        for y in 0..6 {
            for x in 0..6 {
                let want = px[(y / 3) * 2 + x / 3];
                assert_eq!(byte(out[y * 6 + x][0]), want, "({x}, {y})");
            }
        }
    }
}
