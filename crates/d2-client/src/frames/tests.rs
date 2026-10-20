//! Frame tests: synthetic files of each format cut into frame sets, frame
//! keys, the shelf packer and the atlas check, the page upload edge, and an
//! opt-in run over the real game files.

use bevy::asset::Assets;
use bevy::image::Image;
use d2_formats::dc6::{Dc6, Dc6Frame, Dc6Header};
use d2_formats::dcc::{Dcc, DccDirection, DccFrame};
use d2_formats::dt1::{Dt1, Dt1Block, Dt1Tile, ISO_FORMAT};

use super::atlas::MAX_SIDE;
use super::upload::{upload_dirty, AtlasTextures};
use super::*;

fn frame(w: u32, h: u32, seed: u8) -> IndexFrame {
    // Nonzero, position-dependent bytes so misplaced copies show.
    let pixels = (0..w * h)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) | 1)
        .collect();
    IndexFrame::new(w, h, 0, 0, pixels).unwrap()
}

fn dc6_frame(w: u32, h: u32, ox: i32, oy: i32, fill: u8) -> Dc6Frame {
    Dc6Frame {
        flip: 0,
        width: w,
        height: h,
        offset_x: ox,
        offset_y: oy,
        unknown: 0,
        next_block: 0,
        pixels: vec![fill; (w * h) as usize],
    }
}

fn dc6(dirs: u32, per: u32) -> Dc6 {
    let frames = (0..dirs * per)
        .map(|i| dc6_frame(2 + i, 3, i as i32, -(i as i32), i as u8 + 1))
        .collect();
    Dc6 {
        header: Dc6Header {
            version: 6,
            flags: 1,
            encoding: 0,
            termination: [0xEE; 4],
            directions: dirs,
            frames_per_direction: per,
        },
        frames,
    }
}

fn dcc_frame(w: u32, h: u32, x_min: i32, y_min: i32, fill: u8) -> DccFrame {
    DccFrame {
        variable0: 0,
        width: w,
        height: h,
        x_offset: x_min,
        y_offset: y_min + h as i32 - 1,
        coded_bytes: 0,
        bottom_up: false,
        optional_data: Vec::new(),
        x_min,
        y_min,
        pixels: vec![fill; (w * h) as usize],
    }
}

fn dcc(dirs: &[Vec<DccFrame>]) -> Dcc {
    Dcc {
        version: 6,
        frames_per_direction: dirs.first().map_or(0, |d| d.len() as u32),
        tag: 1,
        final_dc6_size: 0,
        directions: dirs
            .iter()
            .map(|frames| DccDirection {
                outsize_coded: 0,
                compression_flags: 0,
                x_min: 0,
                y_min: 0,
                width: 0,
                height: 0,
                frames: frames.clone(),
                pcd_leftover_bits: 0,
            })
            .collect(),
    }
}

fn iso_block(x: i16, y: i16, fill: u8) -> Dt1Block {
    Dt1Block {
        x,
        y,
        unknown1: 0,
        grid_x: 0,
        grid_y: 0,
        format: ISO_FORMAT,
        unknown2: 0,
        pixels: vec![fill; 32 * 15],
    }
}

fn dt1_tile(blocks: Vec<Dt1Block>) -> Dt1Tile {
    Dt1Tile {
        light_direction: 0,
        roof_height: 0,
        material_flags: 0,
        height: 0,
        width: 0,
        unknown_height: 0,
        orientation: 0,
        main_index: 0,
        sub_index: 0,
        rarity: 0,
        unknown_color: 0,
        subtile_flags: [0; 25],
        unknown_58: 0,
        cache_index: 0,
        unknown_5c: 0,
        blocks,
    }
}

// ---- IndexFrame, FrameSetKey ------------------------------------------

// Covers: specs/client/render-pipeline.md §a2-indexed-frames
#[test]
fn index_frame_requires_width_times_height_pixels() {
    assert!(IndexFrame::new(2, 2, 0, 0, vec![0, 5, 7, 0]).is_ok());
    assert_eq!(
        IndexFrame::new(2, 2, 0, 0, vec![0; 3]),
        Err(FrameError::PixelCount {
            width: 2,
            height: 2,
            len: 3
        })
    );
    assert!(IndexFrame::new(0, 7, 0, 0, Vec::new()).unwrap().is_empty());
}

// Covers: specs/client/assets.md §a3-derived-assets
#[test]
fn frame_set_key_accepts_only_canonical_paths() {
    let ok = FrameSetKey::new("data/global/x.dc6", FramePart::Dir(0)).unwrap();
    assert_eq!(ok.path(), "data/global/x.dc6");
    assert_eq!(ok.part(), FramePart::Dir(0));
    for bad in [
        "",
        "DATA/global/x.dc6",
        "data\\global\\x.dc6",
        "/data/global/x.dc6",
        "data/glöbal/x.dc6",
    ] {
        assert_eq!(
            FrameSetKey::new(bad, FramePart::Dir(0)),
            Err(FrameError::NonCanonicalPath(bad.to_string())),
            "{bad:?}"
        );
    }
    // Keys order by path, then part (deterministic BTreeMap order).
    let a = FrameSetKey::new("a.dcc", FramePart::Dir(3)).unwrap();
    let b = FrameSetKey::new("a.dcc", FramePart::Tile(0)).unwrap();
    let c = FrameSetKey::new("b.dcc", FramePart::Dir(0)).unwrap();
    assert!(a < b && b < c);
}

// ---- Frame sets from each format --------------------------------------

// Covers: specs/client/render-pipeline.md §a2-indexed-frames; specs/client/assets.md §a3-derived-assets
#[test]
fn dc6_direction_gives_its_frames_with_offsets_unchanged() {
    let f = dc6(2, 2);
    let set = FrameSet::from_dc6(&f, 1).unwrap();
    assert_eq!(set.frames.len(), 2);
    for (i, fr) in set.frames.iter().enumerate() {
        let src = &f.frames[2 + i];
        assert_eq!((fr.width, fr.height), (src.width, src.height));
        assert_eq!((fr.x_off, fr.y_off), (src.offset_x, src.offset_y));
        assert_eq!(fr.pixels, src.pixels);
    }
    assert_eq!(
        FrameSet::from_dc6(&f, 2),
        Err(FrameError::NoPart {
            part: FramePart::Dir(2),
            count: 2
        })
    );
}

// Covers: specs/render/sprite-placement.md §8, §edge-cases-original-bugs r5
#[test]
fn dc6_frames_carry_the_orientation_bit_and_refuse_other_flips() {
    let mut f = dc6(1, 2);
    f.frames[1].flip = 1;
    let set = FrameSet::from_dc6(&f, 0).unwrap();
    assert_eq!(set.frames[0].anchor, FrameAnchor::Bottom);
    assert_eq!(set.frames[1].anchor, FrameAnchor::TopDown);
    f.frames[1].flip = 2;
    assert_eq!(
        FrameSet::from_dc6(&f, 0),
        Err(FrameError::Dc6Flip {
            dir: 0,
            frame: 1,
            flip: 2
        })
    );
}

// Covers: specs/render/sprite-placement.md §3, §edge-cases-original-bugs r5
#[test]
fn dcc_frames_are_top_anchored_and_refuse_an_odd_variable0() {
    let mut d = dcc(&[vec![dcc_frame(2, 2, -1, -2, 7)]]);
    let set = FrameSet::from_dcc(&d, 0).unwrap();
    assert_eq!(set.frames[0].anchor, FrameAnchor::Top);
    d.directions[0].frames[0].variable0 = 3;
    assert_eq!(
        FrameSet::from_dcc(&d, 0),
        Err(FrameError::DccVariable0 {
            dir: 0,
            frame: 0,
            variable0: 3
        })
    );
}

#[test]
fn dc6_with_missing_frame_is_an_error() {
    let mut f = dc6(1, 2);
    f.frames.pop();
    assert_eq!(
        FrameSet::from_dc6(&f, 0),
        Err(FrameError::MissingFrame { dir: 0, frame: 1 })
    );
}

// Covers: specs/client/render-pipeline.md §a2-indexed-frames; specs/client/assets.md §a3-derived-assets
#[test]
fn dcc_direction_uses_the_frame_box() {
    let f = dcc(&[
        vec![dcc_frame(3, 2, -5, -9, 4)],
        vec![dcc_frame(4, 4, 7, -20, 8), dcc_frame(1, 1, 0, 0, 9)],
    ]);
    let set = FrameSet::from_dcc(&f, 1).unwrap();
    assert_eq!(set.frames.len(), 2);
    assert_eq!(
        set.frames[0],
        IndexFrame::new(4, 4, 7, -20, vec![8; 16]).unwrap()
    );
    assert_eq!(set.frames[1].pixels, [9]);
    assert!(matches!(
        FrameSet::from_dcc(&f, 2),
        Err(FrameError::NoPart { count: 2, .. })
    ));
}

// Covers: specs/client/render-pipeline.md §a2-indexed-frames; specs/client/assets.md §a3-derived-assets; specs/render/shading.md §4 r4
#[test]
fn dt1_tile_uses_the_map_preview_tile_image() {
    // Iso blocks at (0,0) and (64,32) assemble into a 96 × 47 image at
    // offset (0,0), the same image the map preview shows.
    let dt1 = Dt1 {
        version: 7,
        minor_version: 6,
        tiles: vec![
            dt1_tile(vec![iso_block(0, 0, 3), iso_block(64, 32, 4)]),
            dt1_tile(Vec::new()),
        ],
    };
    let set = FrameSet::from_dt1(&dt1, 0).unwrap();
    // The assembled image, then each block alone (for the per-block
    // shading draws).
    assert_eq!(set.frames.len(), 1 + 2);
    let fr = &set.frames[0];
    assert_eq!((fr.width, fr.height, fr.x_off, fr.y_off), (96, 47, 0, 0));
    assert_eq!(fr.pixels.len(), 96 * 47);
    let b = &set.frames[DT1_BLOCK_FRAME0 + 1];
    assert_eq!((b.width, b.height, b.x_off, b.y_off), (32, 15, 64, 32));
    assert_eq!(b.pixels, dt1.tiles[0].blocks[1].pixels);
    // A tile without blocks has no image: an empty set.
    assert!(FrameSet::from_dt1(&dt1, 1).unwrap().frames.is_empty());
    assert!(matches!(
        FrameSet::from_dt1(&dt1, 2),
        Err(FrameError::NoPart { count: 2, .. })
    ));
}

#[test]
fn frame_source_refuses_the_wrong_part_kind() {
    let d6 = dc6(1, 1);
    let dt1 = Dt1 {
        version: 7,
        minor_version: 6,
        tiles: Vec::new(),
    };
    assert_eq!(
        FrameSource::Dc6(&d6).frame_set(FramePart::Tile(0)),
        Err(FrameError::WrongPartKind(FramePart::Tile(0)))
    );
    assert_eq!(
        FrameSource::Dt1(&dt1).frame_set(FramePart::Dir(0)),
        Err(FrameError::WrongPartKind(FramePart::Dir(0)))
    );
    assert_eq!(FrameSource::Dc6(&d6).part_count(), 1);
    assert_eq!(
        FrameSource::Dc6(&d6).frame_set(FramePart::Dir(0)).unwrap(),
        FrameSet::from_dc6(&d6, 0).unwrap()
    );
}

// Covers: specs/client/assets.md §a5-budgets-and-eviction
#[test]
fn frame_set_byte_size_counts_pixels_and_headers() {
    let set = FrameSet {
        frames: vec![frame(3, 2, 0), frame(5, 1, 0)],
    };
    let header = std::mem::size_of::<IndexFrame>() as u64;
    assert_eq!(set.byte_size(), 6 + 5 + 2 * header);
}

// ---- Packing ------------------------------------------------------------

// Covers: specs/client/render-pipeline.md §a2-indexed-frames
#[test]
fn shelf_packer_places_by_first_fit() {
    let mut a = Atlas::new(4).unwrap();
    let s = a
        .insert_set(&[
            frame(10, 8, 1),
            frame(5, 8, 2),
            frame(4, 12, 3),
            frame(3, 4, 4),
        ])
        .unwrap();
    let at = |s: AtlasSlot| (s.page, s.x, s.y, s.w, s.h);
    // First shelf at y=1 (gutter), height 8.
    assert_eq!(at(s[0]), (0, 1, 1, 10, 8));
    assert_eq!(at(s[1]), (0, 12, 1, 5, 8));
    // Taller than shelf 0: new shelf below it, after one gutter row.
    assert_eq!(at(s[2]), (0, 1, 10, 4, 12));
    // Short frame: first shelf tall enough with room, shelf 0.
    assert_eq!(at(s[3]), (0, 18, 1, 3, 4));
}

// Covers: specs/client/render-pipeline.md §a2-indexed-frames
#[test]
fn shelf_packer_wraps_rows_and_pages() {
    let mut a = Atlas::new(2).unwrap();
    // Two 1000-wide frames fill a row (1 + 1000 + 1 + 1000 + 1 = 2003).
    let s = a.insert_set(&vec![frame(1000, 1000, 0); 3]).unwrap();
    assert_eq!((s[0].x, s[0].y), (1, 1));
    assert_eq!((s[1].x, s[1].y), (1002, 1));
    assert_eq!((s[2].page, s[2].x, s[2].y), (0, 1, 1002));
    // Page 0 has room for one more on shelf 1, then page 1 opens.
    let s = a.insert_set(&vec![frame(1000, 1000, 0); 2]).unwrap();
    assert_eq!((s[0].page, s[0].x, s[0].y), (0, 1002, 1002));
    assert_eq!((s[1].page, s[1].x, s[1].y), (1, 1, 1));
    assert_eq!(a.pages().len(), 2);
}

// Covers: specs/client/render-pipeline.md §a2-indexed-frames
#[test]
fn largest_frame_fits_exactly_and_larger_is_an_error() {
    let mut a = Atlas::new(1).unwrap();
    let s = a.insert_set(&[frame(MAX_SIDE, 1, 0)]).unwrap();
    assert_eq!((s[0].x, s[0].w), (1, 2046));
    assert_eq!(
        a.insert_set(&[frame(MAX_SIDE + 1, 1, 0)]),
        Err(AtlasError::TooLarge { w: 2047, h: 1 })
    );
    assert_eq!(Atlas::new(0).err(), Some(AtlasError::NoPages));
}

// Covers: specs/client/assets.md §a5-budgets-and-eviction
#[test]
fn full_atlas_is_an_error_and_leaves_the_atlas_unchanged() {
    let mut a = Atlas::new(1).unwrap();
    a.insert_set(&[frame(2000, 1500, 7)]).unwrap();
    a.take_dirty();
    let before: Vec<u8> = a.pages()[0].pixels.clone();
    // The first frame would fit (shelf 0), the second not: nothing goes in.
    let r = a.insert_set(&[frame(40, 40, 1), frame(2000, 1000, 2)]);
    assert_eq!(r, Err(AtlasError::Full { pages: 1 }));
    assert_eq!(a.pages()[0].pixels, before);
    assert!(a.take_dirty().is_empty());
    // The packer state is unchanged too: the next insert lands where the
    // refused first frame would have (end of shelf 0: 2002 + 40 + 1 ≤ 2048).
    let s = a.insert_set(&[frame(40, 40, 1)]).unwrap();
    assert_eq!((s[0].x, s[0].y), (2002, 1));
}

#[test]
fn empty_frames_take_no_space() {
    let mut a = Atlas::new(1).unwrap();
    let s = a
        .insert_set(&[frame(0, 5, 0), frame(3, 3, 1), frame(4, 0, 0)])
        .unwrap();
    assert_eq!(s[0], AtlasSlot::EMPTY);
    assert_eq!((s[1].x, s[1].y), (1, 1));
    assert_eq!(s[2], AtlasSlot::EMPTY);
    assert_eq!(a.read(s[0]).unwrap(), Vec::<u8>::new());
}

#[test]
fn inconsistent_frame_is_refused_by_the_atlas() {
    let mut a = Atlas::new(1).unwrap();
    let bad = IndexFrame {
        width: 2,
        height: 2,
        x_off: 0,
        y_off: 0,
        anchor: FrameAnchor::Top,
        pixels: vec![1; 3],
    };
    assert!(matches!(
        a.insert_set(&[bad]),
        Err(AtlasError::PixelCount { len: 3, .. })
    ));
    assert!(a.pages().is_empty());
}

/// Deterministic pseudo-random frame sizes (no RNG crate, no state shared
/// with anything else).
fn assorted_frames(n: usize) -> Vec<IndexFrame> {
    let mut x: u32 = 0x1234_5678;
    (0..n)
        .map(|i| {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let w = 1 + (x >> 16) % 300;
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let h = 1 + (x >> 16) % 200;
            frame(w, h, i as u8)
        })
        .collect()
}

// Covers: specs/client/render-pipeline.md §a2-indexed-frames; specs/client/assets.md §a7-threads-and-determinism
#[test]
fn packing_is_deterministic() {
    let frames = assorted_frames(400);
    let run = || {
        let mut a = Atlas::new(64).unwrap();
        let slots: Vec<_> = frames
            .chunks(7)
            .flat_map(|c| a.insert_set(c).unwrap())
            .collect();
        (slots, a)
    };
    let (s1, a1) = run();
    let (s2, a2) = run();
    assert_eq!(s1, s2);
    assert!(a1.pages().len() > 1, "the vector should span pages");
    assert_eq!(a1.pages(), a2.pages());
}

// Covers: specs/client/render-pipeline.md §a2-indexed-frames
#[test]
fn slots_never_overlap_and_keep_their_gutter() {
    let frames = assorted_frames(400);
    let mut a = Atlas::new(64).unwrap();
    let slots = a.insert_set(&frames).unwrap();
    for (i, p) in slots.iter().enumerate() {
        assert!(p.x >= GUTTER && p.y >= GUTTER);
        assert!(p.x + p.w + GUTTER <= PAGE_SIZE && p.y + p.h + GUTTER <= PAGE_SIZE);
        for q in &slots[i + 1..] {
            // Grown by the gutter, rectangles on one page stay apart.
            let apart = p.page != q.page
                || p.x + p.w + GUTTER <= q.x
                || q.x + q.w + GUTTER <= p.x
                || p.y + p.h + GUTTER <= q.y
                || q.y + q.h + GUTTER <= p.y;
            assert!(apart, "{p:?} and {q:?} touch");
        }
    }
    let report = a.check(slots.iter().copied().zip(&frames)).unwrap();
    assert!(report.is_ok(), "{report:?}");
}

// Covers: specs/client/assets.md §a7-threads-and-determinism
#[test]
fn load_order_does_not_change_the_frame_bytes() {
    // §A7: slot positions differ with load order; what a draw reads
    // through each frame's slot does not.
    let frames = assorted_frames(120);
    let mut fwd = Atlas::new(64).unwrap();
    let s_fwd = fwd.insert_set(&frames).unwrap();
    let rev_frames: Vec<_> = frames.iter().rev().cloned().collect();
    let mut rev = Atlas::new(64).unwrap();
    let mut s_rev = rev.insert_set(&rev_frames).unwrap();
    s_rev.reverse();
    assert_ne!(s_fwd, s_rev, "orders should give different slots");
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(fwd.read(s_fwd[i]).unwrap(), f.pixels);
        assert_eq!(rev.read(s_rev[i]).unwrap(), f.pixels);
    }
}

// Covers: specs/client/assets.md §a5-budgets-and-eviction
#[test]
fn clear_page_frees_it_and_restarts_packing_there() {
    let mut a = Atlas::new(2).unwrap();
    a.insert_set(&[frame(2000, 2000, 1), frame(50, 50, 2)])
        .unwrap();
    assert_eq!(a.pages().len(), 2);
    a.take_dirty();
    a.clear_page(0).unwrap();
    assert_eq!(a.pages()[0].generation, 1);
    assert_eq!(a.pages()[1].generation, 0);
    assert!(a.pages()[0].pixels.iter().all(|&p| p == 0));
    assert_eq!(a.take_dirty(), [0]);
    let s = a.insert_set(&[frame(50, 50, 3)]).unwrap();
    assert_eq!((s[0].page, s[0].x, s[0].y), (0, 1, 1));
    assert_eq!(a.clear_page(2), Err(AtlasError::NoPage(2)));
}

// ---- The atlas check can fail (M08) -------------------------------------

#[test]
fn check_reports_exactly_the_perturbed_frame_pixels() {
    let frames = assorted_frames(50);
    for n in [1u32, 2, 17] {
        let mut a = Atlas::new(8).unwrap();
        let slots = a.insert_set(&frames).unwrap();
        let s = *slots.iter().find(|s| s.w * s.h >= 17).unwrap();
        let page = a.page_pixels_mut(s.page);
        for k in 0..n {
            let (x, y) = (s.x + k % s.w, s.y + k / s.w);
            page[(y * PAGE_SIZE + x) as usize] ^= 0x80;
        }
        let r = a.check(slots.iter().copied().zip(&frames)).unwrap();
        assert_eq!(r.frame_mismatches, u64::from(n));
        assert_eq!(r.gutter_mismatches, 0);
        assert_eq!(r.first, Some((s.page, s.x, s.y)));
    }
}

#[test]
fn check_reports_a_dirty_gutter_pixel() {
    let frames = assorted_frames(50);
    let mut a = Atlas::new(8).unwrap();
    let slots = a.insert_set(&frames).unwrap();
    // A pixel just left of a slot at x = 1 is page column 0, the page-edge
    // gutter, which no other slot's ring includes.
    let s = *slots.iter().find(|s| s.x == 1).unwrap();
    a.page_pixels_mut(s.page)[(s.y * PAGE_SIZE) as usize] = 9;
    let r = a.check(slots.iter().copied().zip(&frames)).unwrap();
    assert_eq!((r.frame_mismatches, r.gutter_mismatches), (0, 1));
    assert_eq!(r.first, Some((s.page, 0, s.y)));
}

#[test]
fn check_reports_a_frame_given_the_wrong_slot() {
    let frames = assorted_frames(4);
    let mut a = Atlas::new(1).unwrap();
    let slots = a.insert_set(&frames).unwrap();
    let r = a.check([(slots[1], &frames[0])]).unwrap();
    assert!(!r.is_ok());
}

// ---- Upload edge (no GPU: Assets<Image> only) ---------------------------

// Covers: specs/client/render-pipeline.md §a2-indexed-frames
#[test]
fn upload_creates_one_texture_per_page_and_reuses_handles() {
    let mut a = Atlas::new(4).unwrap();
    let mut images = Assets::<Image>::default();
    let mut tex = AtlasTextures::default();
    a.insert_set(&[frame(2000, 2000, 1), frame(50, 50, 2)])
        .unwrap();
    assert_eq!(upload_dirty(&mut a, &mut images, &mut tex).unwrap(), [0, 1]);
    assert_eq!(tex.pages.len(), 2);
    for (i, h) in tex.pages.iter().enumerate() {
        let img = images.get(h).unwrap();
        assert_eq!(img.width(), PAGE_SIZE);
        assert_eq!(img.height(), PAGE_SIZE);
        assert_eq!(
            img.texture_descriptor.format,
            bevy::render::render_resource::TextureFormat::R8Uint
        );
        assert_eq!(img.data.as_deref(), Some(&a.pages()[i].pixels[..]));
    }
    // Nothing changed: nothing uploaded.
    assert!(upload_dirty(&mut a, &mut images, &mut tex)
        .unwrap()
        .is_empty());
    // A change to page 1 re-uploads it under the same handle.
    let s = a.insert_set(&[frame(60, 60, 3)]).unwrap();
    assert_eq!(s[0].page, 1);
    let h1 = tex.pages[1].clone();
    assert_eq!(upload_dirty(&mut a, &mut images, &mut tex).unwrap(), [1]);
    assert_eq!(tex.pages[1], h1);
    assert_eq!(
        images.get(&h1).unwrap().data.as_deref(),
        Some(&a.pages()[1].pixels[..])
    );
}

// ---- Game files (opt-in, needs a game folder) ---------------------------------------

/// Every live DCC/DC6/DT1 builds all its frame sets, and every set packs
/// into the default 64-page atlas. Run:
/// `D2_GAME_DIR=... cargo test -p d2-client --lib frames::tests::all_live_frame_sets_build_and_pack -- --ignored --nocapture`
// Covers: specs/client/render-pipeline.md §a2-indexed-frames; specs/client/assets.md §a3-derived-assets
#[test]
#[ignore = "needs D2_GAME_DIR"]
fn all_live_frame_sets_build_and_pack() {
    use std::collections::BTreeSet;

    use d2_formats::mpq::ArchiveSet;

    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let set = ArchiveSet::open_dir(&dir).unwrap();
    let mut names = BTreeSet::new();
    for a in set.archives() {
        for n in a.listfile().unwrap().unwrap_or_default() {
            let l = n.to_ascii_lowercase();
            if l.ends_with(".dcc") || l.ends_with(".dc6") || l.ends_with(".dt1") {
                names.insert(l);
            }
        }
    }
    let (mut files, mut parse_errors, mut sets, mut frames) = (0, 0, 0u64, 0u64);
    let mut max_side = (0, 0, String::new());
    let mut atlas = Atlas::new(64).unwrap();
    for name in &names {
        let Ok(bytes) = set.read(name) else { continue };
        let (dcc, dc6, dt1);
        let src = if name.ends_with(".dcc") {
            match Dcc::parse(&bytes) {
                Ok(f) => dcc = f,
                Err(_) => {
                    parse_errors += 1;
                    continue;
                }
            }
            FrameSource::Dcc(&dcc)
        } else if name.ends_with(".dc6") {
            match Dc6::parse(&bytes) {
                Ok(f) => dc6 = f,
                Err(_) => {
                    parse_errors += 1;
                    continue;
                }
            }
            FrameSource::Dc6(&dc6)
        } else {
            match Dt1::parse(&bytes) {
                Ok(f) => dt1 = f,
                Err(_) => {
                    parse_errors += 1;
                    continue;
                }
            }
            FrameSource::Dt1(&dt1)
        };
        files += 1;
        for p in 0..src.part_count() {
            let part = match src {
                FrameSource::Dt1(_) => FramePart::Tile(p as u32),
                _ => FramePart::Dir(u8::try_from(p).expect("≤ 256 directions")),
            };
            let fs = src
                .frame_set(part)
                .unwrap_or_else(|e| panic!("{name} {part}: {e}"));
            for f in &fs.frames {
                if f.width.max(f.height) > max_side.0.max(max_side.1) {
                    max_side = (f.width, f.height, format!("{name} {part}"));
                }
            }
            let slots = match atlas.insert_set(&fs.frames) {
                Err(AtlasError::Full { .. }) => {
                    for pg in 0..atlas.pages().len() as u32 {
                        atlas.clear_page(pg).unwrap();
                    }
                    atlas.insert_set(&fs.frames)
                }
                r => r,
            }
            .unwrap_or_else(|e| panic!("{name} {part}: {e}"));
            let r = atlas.check(slots.into_iter().zip(&fs.frames)).unwrap();
            assert!(r.is_ok(), "{name} {part}: {r:?}");
            sets += 1;
            frames += fs.frames.len() as u64;
        }
    }
    println!(
        "{files} files ({parse_errors} parse errors), {sets} frame sets, {frames} frames; largest {}x{} in {}",
        max_side.0, max_side.1, max_side.2
    );
    assert!(files > 0);
}
