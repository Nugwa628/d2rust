//! Synthetic glyphs exercise the production enhancer without game files.
use super::*;

struct Sheet {
    pixels: Vec<u8>,
    path: &'static str,
}
impl FontSheet for Sheet {
    fn path(&self) -> &str {
        self.path
    }
    fn table(&self) -> Option<&FontTable> {
        None
    }
    fn cel(&self, index: usize) -> Option<GlyphCel<'_>> {
        (index == 0).then_some(GlyphCel {
            width: 3,
            height: 3,
            top: -7,
            pixels: &self.pixels,
        })
    }
}
fn sheet() -> Sheet {
    Sheet {
        pixels: vec![0, 5, 0, 5, 9, 5, 0, 5, 0],
        path: "data/local/font/latin/font16.dc6",
    }
}

#[test]
fn edge_aware_expansion_is_deterministic_and_preserves_indices() {
    let s = sheet();
    let out = scale2x(&s.pixels, 3, 3);
    assert_eq!(out, scale2x(&s.pixels, 3, 3));
    assert!(out.iter().all(|p| s.pixels.contains(p)));
    assert!(out.contains(&5) && out.contains(&9));
    let mut nearest = Vec::new();
    for y in 0..6 {
        for x in 0..6 {
            nearest.push(s.pixels[(y / 2) * 3 + x / 2]);
        }
    }
    assert_ne!(out, nearest, "diagonal corners must be enhanced");
}

#[test]
fn all_font_sheets_work_without_glyph_tables_or_outline_fonts() {
    for path in [
        "data/local/font/latin/font8.dc6",
        "data/local/font/latin/font16.dc6",
        "data/local/font/latin/fontexocet10.dc6",
        "data/local/font/latin/font42.dc6",
    ] {
        let s = Sheet { path, ..sheet() };
        let art = HdTextArt::new();
        assert!(matches!(
            art.glyph(&s, 0, 1.8),
            CelImage::IndexedGlyph { .. }
        ));
    }
}

#[test]
fn glyph_metrics_and_cache_are_stable_at_every_scale() {
    let s = sheet();
    for scale in [1.0, 1.2, 1.8, 2.4, 3.6, 6.0] {
        let art = HdTextArt::new();
        let image = art.glyph(&s, 0, scale);
        let CelImage::IndexedGlyph {
            rect,
            scale: prescale,
            ..
        } = image
        else {
            panic!("not enhanced");
        };
        assert_eq!(
            (rect[2] as f32 / prescale, rect[3] as f32 / prescale),
            (3.0, 3.0)
        );
        assert_eq!(s.cel(0).unwrap().top, -7);
        let atlas = art.atlas().unwrap();
        assert_eq!(art.glyph(&s, 0, scale), image);
        assert_eq!(art.atlas().unwrap().id, atlas.id);
        let other = HdTextArt::new();
        other.glyph(&s, 0, scale);
        assert_eq!(other.atlas().unwrap().pages, atlas.pages);
        art.glyph(&s, 0, scale + 0.1);
        assert_ne!(art.atlas().unwrap().id, atlas.id);
    }
}

#[test]
fn invalid_blank_and_non_font_images_remain_unchanged() {
    let art = HdTextArt::new();
    for scale in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
        assert_eq!(art.glyph(&sheet(), 0, scale), CelImage::Original);
    }
    for s in [
        Sheet {
            pixels: vec![0; 9],
            ..sheet()
        },
        Sheet {
            path: "data/global/ui/panel/invchar6.dc6",
            ..sheet()
        },
    ] {
        assert_eq!(art.glyph(&s, 0, 2.0), CelImage::Original);
    }
    assert!(art.atlas().is_none());
}

#[test]
fn native_item_font_frames_use_the_same_enhancer_without_character_tables() {
    use crate::frames::{FramePart, FrameSet, FrameStore};
    let key = FrameSetKey::new("data/local/font/latin/font8.dc6", FramePart::Dir(0)).unwrap();
    let mut frames = FrameStore::new();
    frames
        .insert(
            key.clone(),
            FrameSet {
                frames: vec![IndexFrame::new(3, 3, -2, 7, sheet().pixels).unwrap()],
            },
        )
        .unwrap();
    let fonts = BTreeMap::new();
    let art = HdTextArt::new();
    let image = art.cel(
        &CelRef {
            file: Some((&key, 0)),
            frames: &frames,
            fonts: &fonts,
        },
        2.0,
    );
    assert!(matches!(
        image,
        CelImage::IndexedGlyph {
            rect: [0, 0, 12, 12],
            scale: 4.0,
            ..
        }
    ));
    assert_eq!(image, art.glyph(&sheet_with_item_path(), 0, 2.0));
}

fn sheet_with_item_path() -> Sheet {
    Sheet {
        path: "data/local/font/latin/font8.dc6",
        ..sheet()
    }
}
