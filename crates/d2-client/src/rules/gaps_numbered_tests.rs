//! One synthetic test per rule unit not covered by `tests.rs`: the shake
//! envelope's millisecond time base and the panel cull of wall blocks
//! (camera edge cases), the cel draw path's orientation bit, zero-size
//! frames and the rejected DC6 headers.

use d2_formats::dc6::{Dc6, Dc6Frame, Dc6Header};

use super::camera::*;
use super::placement::{self, Cel};
use crate::frames::{FrameAnchor, FrameError, FrameSet, IndexFrame};
use crate::scene::{self, DrawItem, FrameId, FrameImage, MapTable, Rect};

const W: i32 = 800;

#[test]
fn shake_envelope_is_a_function_of_milliseconds_not_ticks() {
    // Peak 100, attack 100 ms: the envelope changes inside one 40 ms tick,
    // so two wall-clock times of the same tick give different amplitudes.
    let s = Shake::start(100, 100, 100, 100).unwrap();
    assert_eq!(s.amplitude(41), Some(41));
    assert_eq!(s.amplitude(79), Some(79));
    // Both are tick 1 of the client loop (40 ms ticks)...
    assert_eq!(41 / TICK_MS, 79 / TICK_MS);
    // ...and the D2RUST time base samples one of them per tick.
    assert_eq!(Shake::time_of(1), 40);
    assert_eq!(s.amplitude(Shake::time_of(1)), Some(40));
    assert_eq!(s.amplitude(Shake::time_of(2)), Some(80));
}

#[test]
fn panel_cull_drops_wall_blocks_inside_the_frame() {
    let player = ClientPos { x: 1000, y: 2000 };
    let cam = |mode| {
        Camera::new(
            FrameSize::D2RUST,
            OpenMode::new(mode).unwrap(),
            player,
            (0, 0),
        )
    };
    // Mode 1 (left panel): the view is [−200, 600) but blocks are kept
    // only for x in [−32, 400). A block at x = 400..599 is inside the
    // frame and the view, yet culled: those wall pixels go missing.
    let m1 = cam(1);
    assert_eq!((m1.view.left, m1.view.right), (-200, 600));
    assert!(m1.wall_block_visible(W / 2 - 1, 100));
    for x in [W / 2, W / 2 + 32, 599] {
        assert!(!m1.wall_block_visible(x, 100), "x = {x}");
        // Mode 0 keeps the same block.
        assert!(cam(0).wall_block_visible(x, 100), "x = {x}");
    }
    // Mode 2 (right panel): view [200, 1000), blocks kept for [368, 800).
    let m2 = cam(2);
    assert_eq!((m2.view.left, m2.view.right), (200, 1000));
    for x in [200, 300, W / 2 - 33] {
        assert!(!m2.wall_block_visible(x, 100), "x = {x}");
        assert!(cam(0).wall_block_visible(x, 100), "x = {x}");
    }
    assert!(m2.wall_block_visible(W / 2 - 32, 100));
}

#[test]
fn only_bit_0_of_the_orientation_word_is_tested() {
    // A DC6 cel is its frame header: w, h, xoff, yoff, and bit 0 of the
    // orientation word; the other bits change nothing.
    for (flip, top_down) in [
        (0, false),
        (1, true),
        (2, false),
        (3, true),
        (0xFFFE, false),
    ] {
        let c = Cel::dc6(flip, 4, 3, -2, 10);
        assert_eq!(
            (c.width, c.height, c.xoff, c.yoff, c.top_down),
            (4, 3, -2, 10, top_down),
            "flip = {flip:#x}"
        );
    }
    // DCC cels share the path: w, h, xoff, yoff unchanged, bottom-up.
    assert_eq!(
        Cel::dcc(-5, 7, 9, 8),
        Cel {
            width: 9,
            height: 8,
            xoff: -5,
            yoff: 7,
            top_down: false
        }
    );
}

fn dc6(version: i32, flags: u32, frame: Dc6Frame) -> Dc6 {
    Dc6 {
        header: Dc6Header {
            version,
            flags,
            encoding: 0,
            termination: [0xEE; 4],
            directions: 1,
            frames_per_direction: 1,
        },
        frames: vec![frame],
    }
}

fn dc6_frame(width: u32, height: u32) -> Dc6Frame {
    Dc6Frame {
        flip: 0,
        width,
        height,
        offset_x: 0,
        offset_y: 0,
        unknown: 0,
        next_block: 0,
        pixels: vec![9; width as usize * height as usize],
    }
}

#[test]
fn zero_size_frames_draw_nothing() {
    let frame = Rect::new(0, 0, 800, 600);
    // A zero-high cel covers no row (rows Y + yoff − h + 1 … Y + yoff
    // is empty for h = 0); a top-down one gets no rows at all.
    let c = Cel::dc6(0, 5, 0, 0, 10);
    let (first, last) = c.rows(100);
    assert!(first > last);
    let td = Cel::dc6(1, 5, 0, 0, 10);
    assert_eq!(placement::place_cel(&td, 100, 100, frame).clip, None);

    // Zero-size frames build (DC6 and plain) and composing them onto a
    // non-zero base writes no pixel.
    let set = FrameSet::from_dc6(&dc6(6, 1, dc6_frame(0, 0)), 0).unwrap();
    assert!(set.frames[0].is_empty());
    let images = vec![
        FrameImage {
            width: 0,
            height: 0,
            pixels: vec![],
        },
        FrameImage {
            width: 7,
            height: 0,
            pixels: vec![],
        },
        FrameImage {
            width: 0,
            height: 7,
            pixels: vec![],
        },
    ];
    let mut items = Vec::new();
    for (i, img) in images.iter().enumerate() {
        let f = IndexFrame::new(img.width, img.height, 0, 3, vec![])
            .unwrap()
            .with_anchor(FrameAnchor::Bottom);
        let p = placement::place(&f, 10, 10, frame);
        let mut item = DrawItem::new(FrameId(i as u32), p.x, p.y);
        if let Some(clip) = p.clip {
            item.clip = clip;
        }
        items.push(item);
    }
    let view = Rect::new(0, 0, 32, 32);
    let base = vec![5u8; 32 * 32];
    let out = scene::compose_frame(
        &items,
        &images,
        &MapTable::new(),
        view,
        &base,
        scene::FramePlan::NONE,
    )
    .unwrap();
    assert_eq!(out, base);
}

#[test]
fn dc6_version_other_than_6_or_flags_bit_2_is_refused() {
    // The accepted case: version 6, flags without bit 2.
    for flags in [0, 1, 3, 0xFFFF_FFFB] {
        assert!(FrameSet::from_dc6(&dc6(6, flags, dc6_frame(2, 2)), 0).is_ok());
    }
    // The rasterizer rejects these versions as fatal error 0x452 / 0x453.
    for version in [0, 5, 7, -6] {
        assert_eq!(
            FrameSet::from_dc6(&dc6(version, 1, dc6_frame(2, 2)), 0),
            Err(FrameError::Dc6Header { version, flags: 1 })
        );
    }
    for flags in [4, 5, 0xFFFF_FFFF] {
        assert_eq!(
            FrameSet::from_dc6(&dc6(6, flags, dc6_frame(2, 2)), 0),
            Err(FrameError::Dc6Header { version: 6, flags })
        );
    }
    // The parser already refuses another version on the bytes.
    let mut bytes = vec![0u8; 24];
    bytes[0] = 5;
    assert!(Dc6::parse(&bytes).is_err());
}
