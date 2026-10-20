use d2_formats::cof::Cof;

use super::*;
use crate::composite::{ComponentFrame, Slot};
use crate::frames::{FramePart, FrameSet};
use crate::rules::blend::unit_shadow_ops;
use crate::rules::shading::ShadeTables;
use crate::scene::{BlendOp, Rect};

fn frame(w: u32, h: u32, x_off: i32, y_off: i32, anchor: FrameAnchor) -> IndexFrame {
    IndexFrame {
        width: w,
        height: h,
        x_off,
        y_off,
        anchor,
        pixels: vec![7; (w * h) as usize],
    }
}

#[test]
fn the_shadow_is_half_height_and_sheared_one_pixel_per_row() {
    // 4×4, cel yoff = -4 + 3 = -1: y0 = trunc(-1/2) = 0, x0 = -2.
    let s = shadow_frame(&frame(4, 4, -2, -4, FrameAnchor::Top));
    assert_eq!((s.width, s.height), (5, 2));
    assert_eq!((s.x_off, s.y_off), (-3, -1));
    assert_eq!(s.pixels, [1, 1, 1, 1, 0, 0, 1, 1, 1, 1]);
}

#[test]
fn a_one_row_cel_and_a_top_down_cel_have_no_shadow() {
    for f in [
        frame(4, 1, 0, 0, FrameAnchor::Top),
        frame(4, 4, 0, 0, FrameAnchor::TopDown),
    ] {
        let s = shadow_frame(&f);
        assert_eq!((s.width, s.height, s.pixels.as_slice()), (1, 1, &[0u8][..]));
    }
}

/// Two layers (components 0 and 1; the shadow byte is the layer index, so
/// only the second casts a shadow), one direction, one frame.
fn cof() -> Cof {
    let mut v = vec![2u8, 1, 1, 20, 0, 0, 0, 0];
    for x in [-10i32, 10, -20, 0] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&25u32.to_le_bytes());
    for i in 0u8..2 {
        v.extend_from_slice(&[i, i, 1, 0, 0]);
        v.extend_from_slice(b"hth\0");
    }
    v.push(0); // events
    v.extend_from_slice(&[0, 1]); // draw order
    Cof::parse(&v).expect("synthetic COF")
}

fn draw(cof: &Cof, slot: u8, set: &FrameSetKey, assets: &ViewAssets) -> ComponentDraw {
    let id = assets.id(set, 0).unwrap();
    let mut item = DrawItem::new(id, 100 - 2, 50 - 4);
    item.clip = Rect::new(0, 0, 640, 480);
    ComponentDraw {
        slot: Slot {
            slot,
            component: slot,
            layer: cof.layers.iter().position(|l| l.component == slot).unwrap(),
        },
        frame: ComponentFrame {
            set: set.clone(),
            index: 0,
        },
        item,
    }
}

#[test]
fn a_layer_with_the_shadow_byte_is_drawn_darkening_at_the_feet() {
    let cof = cof();
    let mut assets = ViewAssets::new(crate::app::play::unspecified_palette());
    let pl2 = d2_formats::palette::Pl2::parse(&super::super::tile_assets::tests::pl2()).unwrap();
    let tables = ShadeTables::push(&mut assets.maps, &pl2);
    let a = FrameSetKey::new("data/a.dcc", FramePart::Dir(0)).unwrap();
    let b = FrameSetKey::new("data/b.dcc", FramePart::Dir(0)).unwrap();
    for k in [&a, &b] {
        let set = FrameSet {
            frames: vec![frame(4, 4, -2, -4, FrameAnchor::Top)],
        };
        let sh = shadow_set(&set);
        assets.frames.insert(k.clone(), set).unwrap();
        assets.frames.insert(shadow_key(k).unwrap(), sh).unwrap();
    }
    let draws_in = [draw(&cof, 0, &a, &assets), draw(&cof, 1, &b, &assets)];
    let at = OrderKey {
        pass: 5,
        major: 3,
        minor: 1,
    };
    // No tables yet: no shadow.
    assert!(draws(&cof, 9, at, &draws_in, &assets).unwrap().is_empty());
    assets.shades = Some(tables);
    let out = draws(&cof, 9, at, &draws_in, &assets).unwrap();
    assert_eq!(out.len(), 1, "only layer 1 has the shadow byte");
    let item = &out[0];
    // Unit origin (X, Y) = (100, 50): the shadow frame's offsets (-3, -1)
    // are added to it; the draw row's X - 2 is cancelled by the shear
    // (measured, r2a).
    assert_eq!((item.x, item.y), (100 - 3, 50 - 1));
    let (chain, blend) = unit_shadow_ops(&tables, true);
    assert_eq!((item.shade, item.blend), (chain, blend));
    assert!(matches!(item.blend, BlendOp::IndexTable(_)));
    assert_eq!(item.key, DrawKey::new(5, 3, 1, 1).unwrap());
}
