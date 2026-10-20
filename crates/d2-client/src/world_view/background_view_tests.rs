//! The level 120 background layer with synthetic cel files in a memory
//! source: the summit rows of the background test vector reach the frame first.

use super::*;
use crate::assets::path::MemorySource;
use crate::bridge::drlg::DrlgRoomId;
use crate::bridge::world::{ActiveRoom, ClientUnit, UnitKey, PLAYER};
use crate::rules::camera::{Camera, FrameSize, OpenMode};
use crate::rules::draw_order::background::{CLOUD_FILE, SUMMIT_FILE, SUMMIT_TILE};
use crate::scene::order::pass;
use d2_formats::palette::{Palette, Rgb};

/// A DC6 of one direction, `frames` frames of `w × h` literal pixels,
/// offsets 0, bottom-up.
fn dc6(frames: u32, w: u32, h: u32) -> Vec<u8> {
    let mut rows = Vec::new();
    for _ in 0..h {
        rows.push(w as u8);
        rows.extend((0..w).map(|i| 1 + i as u8));
        rows.push(0x80);
    }
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&frames.to_le_bytes());
    let mut at = d.len() + 4 * frames as usize;
    let mut body = Vec::new();
    for _ in 0..frames {
        d.extend_from_slice(&(at as u32).to_le_bytes());
        for v in [0u32, w, h, 0, 0, 0, 0, rows.len() as u32] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&rows);
        body.extend_from_slice(&[0xEE; 3]);
        at += 32 + rows.len() + 3;
    }
    d.extend(body);
    d
}

fn assets() -> ViewAssets {
    let mut colors = [Rgb::default(); 256];
    for (i, c) in colors.iter_mut().enumerate() {
        *c = Rgb {
            r: i as u8,
            g: i as u8,
            b: i as u8,
        };
    }
    ViewAssets::new(Palette { colors })
}

/// The local player in a room of `level`.
fn world(level: u16) -> ClientWorld {
    let mut w = ClientWorld::default();
    let p = UnitKey::new(PLAYER, 1);
    let mut u = ClientUnit::new(p);
    u.position = Some((5000, 5000));
    w.units.insert(p, u);
    w.local_player = Some(p);
    w.active_rooms = Some(vec![ActiveRoom {
        x0: 4900,
        y0: 4900,
        w: 200,
        h: 200,
        level,
        room: DrlgRoomId(1),
    }]);
    w.room_units.place(p, Some(DrlgRoomId(1)));
    w
}

fn framed() -> WorldFrame {
    WorldFrame {
        camera: Some(Camera::new(
            FrameSize::D2RUST,
            OpenMode::NONE,
            crate::rules::camera::ClientPos { x: 0, y: 0 },
            (0, 0),
        )),
        ..WorldFrame::default()
    }
}

fn view() -> BackgroundView {
    let mut src = MemorySource::default();
    src.insert(&format!("{SUMMIT_FILE}.dc6"), dc6(12, 4, 2));
    src.insert(&format!("{CLOUD_FILE}.dc6"), dc6(2, 4, 2));
    BackgroundView::new(Arc::new(src), Some(Seed::new(7, 666)))
}

#[test]
fn the_summit_draws_its_rows_first_in_the_frame() {
    let w = world(120);
    let mut a = assets();
    let mut frame = framed();
    // A world item already in the frame (pass 2).
    let mut wall = DrawItem::new(crate::scene::FrameId(0), 0, 0);
    wall.key = DrawKey::new(2, 0, 0, 0).unwrap();
    frame.items.push(wall);
    let mut v = view();
    // test vector: player client x 10,063 + 2,056 → frames 1, 2, 3, 0
    // at x −1, 255, 511, 767.
    let log = v.add_to_frame(&w, 0, 10_063 + 2_056, &mut a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    // Rows y 256, 512 and (resolution mode 2) 768; the clouds (mode 3)
    // need the shade tables, which these assets lack.
    let bg: Vec<&DrawItem> = frame
        .items
        .iter()
        .filter(|i| i.key.pass() == pass::LEVEL_BACKGROUND)
        .collect();
    assert_eq!(bg.len(), 12);
    // Pass 1 sorts before every world pass.
    assert!(frame.items[..12]
        .iter()
        .all(|i| i.key.pass() == pass::LEVEL_BACKGROUND));
    let set = FrameSetKey::new(
        CanonicalPath::new(&format!("{SUMMIT_FILE}.dc6"))
            .unwrap()
            .as_str(),
        FramePart::Dir(0),
    )
    .unwrap();
    for (row, (y, add)) in [(256, 0), (512, 4), (768, 8)].into_iter().enumerate() {
        for (k, frame_no) in [1usize, 2, 3, 0].into_iter().enumerate() {
            let index = frame_no + add;
            let image = a.frame(&set, index).unwrap();
            let (x, y) = draw_position(image, -1 + SUMMIT_TILE * k as i32, y);
            let it = bg[4 * row + k];
            assert_eq!(
                (it.frame, it.x, it.y, it.blend),
                (a.id(&set, index).unwrap(), x, y, BlendOp::Opaque)
            );
        }
    }
    // The kept state: the clouds were placed on the first use.
    assert!(v.state().summit.is_some());
}

#[test]
fn only_level_120_draws_and_never_while_exiting() {
    let mut a = assets();
    for (w, mode) in [(world(119), 0), (world(120), 3)] {
        let mut frame = framed();
        view().add_to_frame(&w, mode, 10_063, &mut a, &mut frame);
        assert!(frame.items.is_empty());
    }
    let mut w = world(120);
    w.exit_requested = true;
    let mut frame = framed();
    let mut v = view();
    v.add_to_frame(&w, 0, 10_063, &mut a, &mut frame);
    assert!(frame.items.is_empty());
    // No seed: needs it; logged, nothing drawn.
    let mut frame = framed();
    let mut v = BackgroundView::new(Arc::new(MemorySource::default()), None);
    let log = v.add_to_frame(&world(120), 0, 10_063, &mut a, &mut frame);
    assert!(frame.items.is_empty() && log.len() == 1, "{log:?}");
}

#[test]
fn seeds_mix_the_clock_and_the_shake_start() {
    use super::{stars_seed_at, summit_seed_at};
    // time 100, tick 7: 100 + 7 + 7 = 114.
    assert_eq!(
        summit_seed_at(100, 7),
        Seed::init_low(d2_sim::rng::time_value(114))
    );
    // The stars add the shake start instead of a second tick count.
    assert_eq!(
        stars_seed_at(100, 7, 50),
        Seed::init_low(d2_sim::rng::time_value(157))
    );
    assert_eq!(
        stars_seed_at(100, 7, 0),
        Seed::init_low(d2_sim::rng::time_value(107))
    );
}

#[test]
fn level_74_initialises_the_stars_once_with_last_zero() {
    let mut a = assets();
    let mut v = view();
    let mut frame = framed();
    v.add_to_frame(&world(74), 0, 0, &mut a, &mut frame);
    assert!(v.state().stars.is_some());
    // 256 one-pixel line items in pass 1 and their `DrawLine` facts.
    assert_eq!(frame.items.len(), 256);
    assert_eq!(frame.sky.len(), 256);
    assert!(frame
        .items
        .iter()
        .all(|i| i.key.pass() == pass::LEVEL_BACKGROUND));
}
