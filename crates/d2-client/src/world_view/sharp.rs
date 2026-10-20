//! Sharp scaling of the presented game frame (and the front end's
//! picture).
//!
//! The frame is shown through [`SharpMaterial`] on a quad the size of the
//! frame, scaled to the presentation. Its shader keeps every game pixel a
//! solid block and blends only the one screen pixel where two game pixels
//! meet. At a fractional scale (fit: 1.8× at 1080p, 2.4× at 1440p, 3.6× at
//! 2160p for a 600-line frame) every game pixel then covers the same width
//! on screen, with no smear and no shimmer as the picture moves; at a
//! whole-number scale every screen pixel falls inside one game pixel, so
//! the result is exactly nearest sampling.
//!
//! Fit is a nearest integer prescale (rounded up), then linear sampling
//! to the final size. The shader evaluates both steps in one read, avoiding
//! a large intermediate texture on small GPUs.
//!
//! It is one pass and one texture read per screen pixel, with a plain
//! linear sampler, so it runs on every GPU Bevy runs on.
//!
//! [`sharp_offset`] is the shader's per-axis rule in Rust, so the rule is
//! tested here ([`present_row`] models a whole screen row through it).

use bevy::asset::embedded_asset;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};

/// The frame image, sampled with a linear filter, the scale per axis and
/// the part of the image shown.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SharpMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub frame: Handle<Image>,
    /// xy: presentation scale; zw: gamma exponent and contrast minus one.
    #[uniform(2)]
    pub scale: Vec4,
    /// The part of the image the quad shows, in uv units: xy its origin,
    /// zw its size. [`FULL_IMAGE`] shows the whole frame; a layer that
    /// slides by a fraction of a pixel moves the origin instead.
    #[uniform(3)]
    pub uv_rect: Vec4,
    /// The smooth camera's world layer only: the tick before's world
    /// layer, read where the shifted image has no picture (its uv is
    /// outside the image), at [`Self::prev_uv`] (`super::smooth::prev_uv`;
    /// zero-sized: no fill). `None`: the image's clamped edge.
    #[texture(4)]
    pub prev: Option<Handle<Image>>,
    #[uniform(5)]
    pub prev_uv: Vec4,
    /// Opaque for the game frame; [`AlphaMode2d::Blend`] for a layer with
    /// transparent parts drawn over another (the smooth camera's screen
    /// layer, `super::smooth`).
    pub alpha_mode: AlphaMode2d,
}

/// The whole image: origin (0, 0), size (1, 1).
pub const FULL_IMAGE: Vec4 = Vec4::new(0.0, 0.0, 1.0, 1.0);

impl SharpMaterial {
    /// The whole of `frame`, at scale 1 until the first placement.
    pub fn new(frame: Handle<Image>) -> Self {
        SharpMaterial {
            frame,
            scale: scale(1.0, 1.0),
            uv_rect: FULL_IMAGE,
            prev: None,
            prev_uv: Vec4::ZERO,
            alpha_mode: AlphaMode2d::Opaque,
        }
    }
}

impl Material2d for SharpMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://d2_client/world_view/sharp.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        self.alpha_mode
    }
}

/// Registers the material and its shader.
pub struct SharpPlugin;

impl Plugin for SharpPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sharp.wgsl");
        app.add_plugins(Material2dPlugin::<SharpMaterial>::default());
    }
}

/// Adds [`SharpPlugin`] when the app renders and nothing added it yet (the
/// game's view and the front end both present through it).
pub fn add_sharp(app: &mut App) {
    if app.get_sub_app(bevy::render::RenderApp).is_some() && !app.is_plugin_added::<SharpPlugin>() {
        app.add_plugins(SharpPlugin);
    }
}

/// The shader's scale uniform for a presentation scale `(sx, sy)`, at
/// least 1 (below 1 game pixels are smaller than screen pixels and the
/// blend covers the whole pixel, which is plain linear filtering).
pub fn scale(sx: f32, sy: f32) -> Vec4 {
    let (gamma, contrast) = picture_adjustment();
    Vec4::new(sx.max(1.0), sy.max(1.0), gamma - 1.0, contrast - 1.0)
}

// Packed so a renderer sees Gamma and Contrast from the same change.
static PICTURE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(155 | (100 << 16));
static BLENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Apply picture options without changing the indexed frame or its hashes.
pub fn set_picture_options(s: &crate::app::config::Settings) {
    use std::sync::atomic::Ordering;
    PICTURE.store(
        u32::from(s.gamma) | (u32::from(s.contrast) << 16),
        Ordering::Relaxed,
    );
    BLENDED.store(s.blended_shadows != 0, Ordering::Relaxed);
}

pub fn blended_shadows() -> bool {
    BLENDED.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn picture_adjustment() -> (f32, f32) {
    let p = PICTURE.load(std::sync::atomic::Ordering::Relaxed);
    picture_adjustment_for((p & 65535) as u16, (p >> 16) as u8)
}

fn picture_adjustment_for(gamma: u16, contrast: u8) -> (f32, f32) {
    (155.0 / gamma.max(1) as f32, f32::from(contrast) / 100.0)
}

/// Where the shader samples within a game pixel, on one axis: `d` is the
/// offset of the screen pixel's centre from the game pixel's left edge
/// (0..1) and `scale` the screen pixels per game pixel. The result is 0.5
/// (the pixel's own centre, a solid colour) except within half a screen
/// pixel of either edge, where it ramps toward the neighbour, reaching the
/// boundary between the two (an even blend) at the edge itself.
///
/// The solid region is widened by [`SOLID_SLACK`] so that at a
/// whole-number scale, where the outermost screen pixel centres sit right
/// on its edge, rounding never lets a sliver of the neighbour in.
pub fn sharp_offset(d: f32, scale: f32) -> f32 {
    let scale = scale.ceil().max(1.0);
    let c = d - 0.5;
    let solid = 0.5 - 0.5 / scale + SOLID_SLACK;
    (c - c.clamp(-solid, solid)) * scale + 0.5
}

/// How far (in game pixels) the solid region reaches past its exact edge;
/// the shader uses the same number.
pub const SOLID_SLACK: f32 = 1.0 / 4096.0;

/// The linear sampler the material reads the frame with: clamped at the
/// edges, so the outermost game pixels stay solid.
pub fn frame_sampler() -> ImageSampler {
    ImageSampler::linear()
}

/// One screen row through the shader, on the CPU: `frame` is a row of
/// game pixel values, `screen_w` the screen pixels it covers. Each screen
/// pixel samples where [`sharp_offset`] says, with the linear filter the
/// GPU applies (clamped at the ends).
pub fn present_row(frame: &[f32], screen_w: u32) -> Vec<f32> {
    let n = frame.len();
    let s = screen_w as f32 / n as f32;
    let at = |i: i64| frame[i.clamp(0, n as i64 - 1) as usize];
    (0..screen_w)
        .map(|x| {
            let texel = (x as f32 + 0.5) / s;
            let base = texel.floor();
            let p = base + sharp_offset(texel - base, s.max(1.0));
            // Linear filter between the texel centres around `p`.
            let left = (p - 0.5).floor();
            let w = p - 0.5 - left;
            at(left as i64) * (1.0 - w) + at(left as i64 + 1) * w
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_shaders_validate_after_material_preprocessing() {
        for shader in [include_str!("sharp.wgsl"), include_str!("native_ui.wgsl")] {
            let shader = shader
                .replace("#import bevy_sprite::mesh2d_vertex_output::VertexOutput", "struct VertexOutput { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, }")
                .replace("#{MATERIAL_BIND_GROUP}", "0");
            let module =
                wgpu::naga::front::wgsl::parse_str(&shader).expect("presentation shader parses");
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .expect("presentation shader validates");
        }
    }

    #[test]
    fn picture_options_have_identity_defaults_and_distinct_slider_values() {
        assert_eq!(picture_adjustment_for(155, 100), (1.0, 1.0));
        for gamma in (55..255).step_by(10) {
            assert!(
                picture_adjustment_for(gamma, 100).0 > picture_adjustment_for(gamma + 10, 100).0
            );
        }
        for contrast in 0..100 {
            assert!(
                picture_adjustment_for(155, contrast).1
                    < picture_adjustment_for(155, contrast + 1).1
            );
        }
    }

    /// How much of each game pixel shows on screen: each screen pixel's
    /// colour split between the game pixels it blends, summed per game
    /// pixel. A row of distinct powers keeps the shares separable.
    fn widths(frame_w: usize, screen_w: u32) -> Vec<f32> {
        let mut out = vec![0.0; frame_w];
        for i in 0..frame_w {
            let mut row = vec![0.0; frame_w];
            row[i] = 1.0;
            out[i] = present_row(&row, screen_w).iter().sum();
        }
        out
    }

    #[test]
    fn fractional_scales_give_every_game_pixel_an_even_width() {
        // 600 lines at 1080, 1440 and 2160 (1.8, 2.4, 3.6), and the
        // 1068-wide 16:9 frame across the same screens.
        for (frame, screen) in [
            (600usize, 1080u32),
            (600, 1440),
            (600, 2160),
            (1068, 1920),
            (1068, 2560),
            (1068, 3840),
        ] {
            let s = screen as f32 / frame as f32;
            for (i, w) in widths(frame, screen).into_iter().enumerate() {
                assert!(
                    (w - s).abs() <= 0.25,
                    "{frame} -> {screen}: pixel {i} is {w} wide, scale {s}"
                );
            }
        }
    }

    #[test]
    fn nearest_at_a_fractional_scale_is_uneven_but_sharp_is_not() {
        // The reason for the sharp scaler: nearest at 1.8x draws game
        // pixels 1 or 2 screen pixels wide.
        let (frame, screen) = (600usize, 1080u32);
        let s = screen as f32 / frame as f32;
        let mut nearest = vec![0u32; frame];
        for x in 0..screen {
            nearest[((x as f32 + 0.5) / s) as usize] += 1;
        }
        assert!(nearest.contains(&1) && nearest.contains(&2));
        let sharp = widths(frame, screen);
        let spread = sharp.iter().cloned().fold(0.0f32, f32::max)
            - sharp.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread < 0.5, "{spread}");
    }

    /// Picture check against a separately implemented two-stage scaler:
    /// nearest integer enlargement, followed by linear sampling.
    #[test]
    fn fit_matches_integer_prescale_then_linear_at_every_target_size() {
        for (fw, fh) in [(800u32, 600u32), (1068, 600), (640, 480)] {
            let frame: Vec<f32> = (0..fw).map(|x| ((x * 37) % 256) as f32 / 255.0).collect();
            for (w, h) in [
                (1280, 720),
                (1920, 1080),
                (2560, 1440),
                (3840, 2160),
                (1365, 767),
                (1919, 1079),
            ] {
                let p = crate::ui::Presentation::with_mode(w, h, fw, fh, crate::ui::ScaleMode::Fit)
                    .unwrap();
                let k = p.sx.ceil().max(1.0) as usize;
                let prescaled: Vec<f32> = frame
                    .iter()
                    .flat_map(|v| std::iter::repeat_n(*v, k))
                    .collect();
                let at = |x: i64| prescaled[x.clamp(0, prescaled.len() as i64 - 1) as usize];
                for (x, actual) in present_row(&frame, p.image_w).iter().enumerate() {
                    let u = (x as f32 + 0.5) * prescaled.len() as f32 / p.image_w as f32 - 0.5;
                    let l = u.floor();
                    let expected = at(l as i64) * (1.0 - (u - l)) + at(l as i64 + 1) * (u - l);
                    assert!(
                        (actual - expected).abs() < 0.002,
                        "{fw}x{fh} -> {w}x{h}, x={x}: {actual} != {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn whole_scales_are_exactly_nearest() {
        let frame: Vec<f32> = (0..800).map(|i| (i * 37 % 256) as f32).collect();
        for k in [1u32, 2, 3] {
            let row = present_row(&frame, 800 * k);
            for (x, v) in row.iter().enumerate() {
                // Exact once written to an 8-bit target.
                assert_eq!(v.round(), frame[x / k as usize], "scale {k} pixel {x}");
            }
        }
    }

    #[test]
    fn a_whole_number_scale_samples_every_pixel_at_its_centre() {
        for s in 1..=5 {
            for k in 0..s {
                let d = (k as f32 + 0.5) / s as f32;
                assert_eq!(sharp_offset(d, s as f32), 0.5, "scale {s} pixel {k}");
            }
        }
    }

    #[test]
    fn the_edge_band_reaches_the_boundary() {
        assert!(sharp_offset(0.0, 2.4).abs() < 1e-3);
        assert!((sharp_offset(1.0, 2.4) - 1.0).abs() < 1e-3);
        assert_eq!(sharp_offset(0.5, 2.4), 0.5);
    }

    #[test]
    fn the_scale_uniform_is_at_least_one() {
        assert_eq!(scale(0.8, 0.8).xy(), Vec2::new(1.0, 1.0));
        assert_eq!(scale(2.4, 2.4).xy(), Vec2::new(2.4, 2.4));
    }
}
