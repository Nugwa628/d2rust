// Spec: specs/render/map-preview.md (Palette shading)
//! GPU palette shading: index images drawn through a palette texture.

use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dPlugin};
use d2_formats::palette::Palette;

/// Material that draws `indices` (R8Uint) through `palette` (256×1 sRGB).
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PaletteMaterial {
    #[texture(0, sample_type = "u_int")]
    pub indices: Handle<Image>,
    #[texture(1)]
    pub palette: Handle<Image>,
}

impl Material2d for PaletteMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://d2_client/render/palette.wgsl".into()
    }

    // Index 0 is discarded in the shader; mask mode enables discard and
    // depth-tested ordering by z.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Mask(0.5)
    }
}

pub struct PaletteRenderPlugin;

impl Plugin for PaletteRenderPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "palette.wgsl");
        app.add_plugins(Material2dPlugin::<PaletteMaterial>::default());
    }
}

/// A `width × height` R8Uint texture of palette indices (row-major).
pub fn index_image(width: u32, height: u32, pixels: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::R8Uint,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// A 256×1 sRGB texture of the palette colors (alpha 255).
pub fn palette_image(palette: &Palette) -> Image {
    let mut data = Vec::with_capacity(256 * 4);
    for c in &palette.colors {
        data.extend_from_slice(&[c.r, c.g, c.b, 255]);
    }
    Image::new(
        Extent3d {
            width: 256,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}
