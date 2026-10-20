//! CPU reference renderer for the map preview: the ground truth the GPU output is compared
//! to.
//!
//! Items are painted in layout order, so later items cover earlier ones. Index 0 is
//! transparent in an image and black where nothing was drawn; every other index is shown as
//! its palette colour exactly, with no lighting.

use d2_formats::palette::Palette;

use super::layout::Layout;
use super::tiles::TileLibrary;

/// A screen-space rectangle to render (y down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

/// Paints the layout into palette indices (0 = background), clipped to `view`.
pub fn render_indexed(layout: &Layout, lib: &TileLibrary, view: View) -> Vec<u8> {
    let (vw, vh) = (view.width as i32, view.height as i32);
    let mut out = vec![0u8; view.width as usize * view.height as usize];
    for item in &layout.items {
        let img = &lib.images[item.image];
        let (iw, ih) = (img.width as i32, img.height as i32);
        let (ox, oy) = (item.x - view.left, item.y - view.top);
        let (cx0, cy0) = (ox.max(0), oy.max(0));
        let (cx1, cy1) = ((ox + iw).min(vw), (oy + ih).min(vh));
        for y in cy0..cy1 {
            let src = ((y - oy) * iw) as usize;
            let dst = (y * vw) as usize;
            for x in cx0..cx1 {
                let p = img.pixels[src + (x - ox) as usize];
                if p != 0 {
                    out[dst + x as usize] = p;
                }
            }
        }
    }
    out
}

/// Maps indices to opaque RGBA; index 0 is the black background.
pub fn to_rgba(indexed: &[u8], palette: &Palette) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(indexed.len() * 4);
    for &p in indexed {
        if p == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 255]);
        } else {
            let c = palette.colors[usize::from(p)];
            rgba.extend_from_slice(&[c.r, c.g, c.b, 255]);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::layout::{DrawItem, Source};
    use crate::map::tiles::{TileImage, TileKey};

    fn item(x: i32, y: i32) -> DrawItem {
        DrawItem {
            image: 0,
            x,
            y,
            key: TileKey {
                orientation: 0,
                main: 0,
                sub: 0,
            },
            cell: (0, 0),
            source: Source::Sprite,
        }
    }

    fn lib() -> TileLibrary {
        let mut lib = TileLibrary::new();
        lib.images.push(TileImage {
            x0: 0,
            y0: 0,
            width: 2,
            height: 2,
            pixels: vec![1, 0, 2, 3],
            roof_height: 0,
        });
        lib
    }

    #[test]
    fn later_items_paint_over_and_zero_is_transparent() {
        let lib = lib();
        let layout = Layout {
            items: vec![item(0, 0), item(1, 0)],
            ..Layout::default()
        };
        let view = View {
            left: 0,
            top: 0,
            width: 3,
            height: 2,
        };
        // Second copy shifted by one: its (0,0)=1 covers first's (1,0)=0;
        // its transparent (1,0) leaves the background at (2,0).
        assert_eq!(render_indexed(&layout, &lib, view), [1, 1, 0, 2, 2, 3]);
    }

    #[test]
    fn clipping_to_view() {
        let lib = lib();
        let layout = Layout {
            items: vec![item(-1, -1)],
            ..Layout::default()
        };
        let view = View {
            left: 0,
            top: 0,
            width: 2,
            height: 2,
        };
        assert_eq!(render_indexed(&layout, &lib, view), [3, 0, 0, 0]);
    }

    #[test]
    fn probe_finds_topmost_opaque_items() {
        let lib = lib();
        let layout = Layout {
            items: vec![item(0, 0), item(1, 0)],
            ..Layout::default()
        };
        // (1,0): second item's pixel (0,0)=1 on top; first item's (1,0) is
        // transparent, so only one hit.
        let hits = layout.probe(&lib, 1, 0);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].0, hits[0].2), (1, 1));
        // (1,1): both opaque, topmost first.
        let hits = layout.probe(&lib, 1, 1);
        assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), [1, 0]);
    }

    #[test]
    fn rgba_mapping() {
        let mut palette = Palette {
            colors: [Default::default(); 256],
        };
        palette.colors[2] = d2_formats::palette::Rgb { r: 9, g: 8, b: 7 };
        assert_eq!(to_rgba(&[0, 2], &palette), [0, 0, 0, 255, 9, 8, 7, 255]);
    }
}
