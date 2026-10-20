//! Synthetic fixtures only: an invented object token, COF and DC6.

use std::sync::Arc;

use d2_formats::palette::{Palette, Rgb};

use super::*;
use crate::assets::path::MemorySource;
use crate::bridge::world::{ClientWorld, UnitKey, OBJECT};
use crate::rules::camera::{ClientPos, OpenMode};
use crate::rules::unit_composite::code;
use crate::world_view::unit_assets::UnitArtLoader;
use crate::world_view::ViewAssets;

/// COF bytes: one direction, one frame, the given
/// layers (component ids, weapon class `hth`); box x −10…10, y −20…0.
fn cof_bytes(layers: &[u8]) -> Vec<u8> {
    let mut v = vec![layers.len() as u8, 1, 1, 20, 0, 0, 0, 0];
    for x in [-10i32, 10, -20, 0] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&256u32.to_le_bytes());
    for c in layers {
        v.extend_from_slice(&[*c, 0, 1, 0, 0]);
        v.extend_from_slice(b"hth\0");
    }
    v.push(0);
    v.extend_from_slice(layers);
    v
}

/// A DC6 of one direction, one 4 × 2 frame at offset (0, 0).
fn dc6() -> Vec<u8> {
    let rows = [4u8, 1, 2, 3, 4, 0x80, 4, 1, 2, 3, 4, 0x80];
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&((d.len() + 4) as u32).to_le_bytes());
    for v in [0u32, 4, 2, 0, 0, 0, 0, rows.len() as u32] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&rows);
    d.extend_from_slice(&[0xEE; 3]);
    d
}

/// Object 342 = `QO` (a DC6 object row), mode
/// 0 `QN`, component 1 `QT`.
fn looks() -> UnitLooks {
    let mut l = UnitLooks {
        object_modes: vec![code(b"QN")],
        components: vec![code(b"QH"), code(b"QT")],
        ..Default::default()
    };
    l.objects.insert(342, code(b"QO"));
    l
}

const COF: &str = "data\\global\\objects\\QO\\cof\\QOQNhth.cof";
const TR_FILE: &str = "data\\global\\objects\\QO\\QT\\QOQTlitQNhth.dc6";

/// The predicate over the art `files` load for one object, with `camera`.
fn predicate(files: &[(&str, Vec<u8>)], camera: Option<Camera>) -> (ViewVisibility, ClientUnit) {
    let mut src = MemorySource::default();
    for (name, bytes) in files {
        src.insert(name, bytes.clone());
    }
    let looks = Arc::new(looks());
    let art = SharedUnitArt::default();
    let loader = UnitArtLoader {
        source: Arc::new(src),
        looks: looks.clone(),
        art: art.clone(),
    };
    let mut u = ClientUnit::new(UnitKey {
        unit_type: OBJECT,
        guid: 9,
    });
    u.class = 342;
    u.position = Some((10, 10));
    let mut world = ClientWorld::default();
    world.units.insert(u.key, u.clone());
    let mut assets = ViewAssets::new(Palette {
        colors: [Rgb::default(); 256],
    });
    // A missing COF is one log line (not an error).
    loader.ensure(&world, &mut assets);
    let v = ViewVisibility {
        looks,
        art,
        camera: Arc::new(RwLock::new(camera)),
    };
    (v, u)
}

/// The player at client (1000, 1000), open mode 0, no shake: unit origin
/// (600, 716), shiftX 0, so a point (a, b) is
/// screen (a − 600, b − 708).
fn camera() -> Camera {
    Camera::new(
        FrameSize::D2RUST,
        OpenMode::NONE,
        ClientPos { x: 1000, y: 1000 },
        (0, 0),
    )
}

#[test]
fn the_cof_box_then_the_cel_box_at_the_camera_origin() {
    let (v, u) = predicate(&[(COF, cof_bytes(&[1])), (TR_FILE, dc6())], Some(camera()));
    let path = crate::assets::path::CanonicalPath::new(TR_FILE).unwrap();
    // The cel fields as the DC6 frame header stores them.
    assert_eq!(
        v.art.read().unwrap().cels.get(&path),
        Some(&vec![vec![CelBox {
            w: 4,
            h: 2,
            ox: 0,
            oy: 0
        }]])
    );
    // Screen (400, 292): both boxes inside the 800 × 600 frame.
    assert!(v.visible(&u, 1000, 1000));
    // Screen X 811: the COF box's x_min + X < W − 1 fails.
    assert!(!v.visible(&u, 1411, 1000));
    // Screen X −6: the COF box passes (x_max + X = 4 ≥ 0), the cel box's
    // left + w = −2 ≥ 0 fails; X −4 passes both.
    assert!(!v.visible(&u, 594, 1000));
    assert!(v.visible(&u, 596, 1000));
    // Screen Y −1: y_max + Y ≥ 0 fails.
    assert!(!v.visible(&u, 1000, 707));
    // The bridge's form answers the same.
    let f = v.clone().into_fn();
    assert!(f.visible(&u, 1000, 1000));
    assert!(!f.visible(&u, 594, 1000));
}

#[test]
fn no_cof_no_torso_layer_or_no_torso_cel_is_not_visible() {
    // No COF resident: step 2 has no box.
    let (v, u) = predicate(&[], Some(camera()));
    assert!(!v.visible(&u, 1000, 1000));
    // The COF has no TR layer (only HD): the cel request fails.
    let (v, u) = predicate(&[(COF, cof_bytes(&[0]))], Some(camera()));
    assert!(!v.visible(&u, 1000, 1000));
    // The TR file is in no archive: the cel load fails.
    let (v, u) = predicate(&[(COF, cof_bytes(&[1]))], Some(camera()));
    assert!(!v.visible(&u, 1000, 1000));
    // A frame +0x44 >> 8 past the file: no cel.
    let (v, mut u) = predicate(&[(COF, cof_bytes(&[1])), (TR_FILE, dc6())], Some(camera()));
    u.frame = 5 << 8;
    assert!(!v.visible(&u, 1000, 1000));
}

#[test]
fn before_the_first_drawn_frame_the_origin_is_zero() {
    // No camera → origin (0, 0), shiftX 0 (the globals read 0 before the
    // first drawn frame, measured): screen
    // (a, b + 8).
    let (v, u) = predicate(&[(COF, cof_bytes(&[1])), (TR_FILE, dc6())], None);
    assert!(v.visible(&u, 400, 292));
    assert!(!v.visible(&u, 1000, 1000));
    // The shared camera is read on each call: once a frame is drawn, its
    // origin applies.
    *v.camera.write().unwrap() = Some(camera());
    assert!(v.visible(&u, 1000, 1000));
    assert!(!v.visible(&u, 400, 292));
}
