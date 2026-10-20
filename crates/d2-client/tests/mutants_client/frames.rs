//! Mutation-testing gaps of `frames` (derived frame sets and the atlas).

use d2_client::frames::FrameSource;
use d2_formats::dt1::Dt1;

/// A DT1 is keyed by tile, so its part count is its tile count.
#[test]
fn part_count_of_a_dt1_is_its_tile_count() {
    let empty = Dt1 {
        version: 7,
        minor_version: 6,
        tiles: Vec::new(),
    };
    assert_eq!(FrameSource::Dt1(&empty).part_count(), 0);
}

/// Atlas: a slot is empty when either side is 0.
#[test]
fn atlas_slot_is_empty_when_either_side_is_zero() {
    use d2_client::frames::AtlasSlot;

    let slot = |w, h| AtlasSlot {
        page: 0,
        x: 1,
        y: 1,
        w,
        h,
    };
    assert!(AtlasSlot::EMPTY.is_empty());
    assert!(slot(0, 5).is_empty());
    assert!(slot(3, 0).is_empty());
    assert!(!slot(3, 5).is_empty());
}

/// Atlas: slots are separated by a 1-pixel gutter of
/// index 0, also between frames sharing a shelf.
#[test]
fn frames_on_one_shelf_keep_a_one_pixel_gutter() {
    use d2_client::frames::{Atlas, IndexFrame, GUTTER};

    let mut atlas = Atlas::new(1).unwrap();
    assert_eq!(atlas.max_pages(), 1);
    let frame = || IndexFrame::new(3, 2, 0, 0, vec![7; 6]).unwrap();
    let frames = [frame(), frame(), frame()];
    let slots = atlas.insert_set(&frames).unwrap();
    for pair in slots.windows(2) {
        assert_eq!((pair[0].page, pair[0].y), (pair[1].page, pair[1].y));
        assert_eq!(pair[1].x, pair[0].x + pair[0].w + GUTTER, "{slots:?}");
    }
    let report = atlas.check(slots.iter().copied().zip(&frames)).unwrap();
    assert!(report.is_ok(), "{report:?}");
    let wide = Atlas::new(64).unwrap();
    assert_eq!(wide.max_pages(), 64);
}

/// The atlas check reports frame and gutter mismatches together.
#[test]
fn check_report_sums_both_kinds() {
    use d2_client::frames::CheckReport;

    let r = CheckReport {
        frame_mismatches: 2,
        gutter_mismatches: 3,
        first: Some((0, 1, 1)),
    };
    assert_eq!(r.mismatches(), 5);
    assert!(!r.is_ok());
}

/// A page of 2048 with a 1-pixel gutter on each side holds a side of
/// 2046 and no more.
#[test]
fn largest_frame_side_is_page_minus_two_gutters() {
    use d2_client::frames::{Atlas, AtlasError, IndexFrame, GUTTER, PAGE_SIZE};

    let side = PAGE_SIZE - 2 * GUTTER;
    let mut atlas = Atlas::new(1).unwrap();
    let fits = IndexFrame::new(side, 1, 0, 0, vec![1; side as usize]).unwrap();
    let slots = atlas.insert_set(&[fits]).unwrap();
    assert_eq!((slots[0].x, slots[0].w), (GUTTER, side));
    let tall = IndexFrame::new(1, side, 0, 0, vec![1; side as usize]).unwrap();
    assert!(Atlas::new(1).unwrap().insert_set(&[tall]).is_ok());
    let over = IndexFrame::new(side + 1, 1, 0, 0, vec![1; side as usize + 1]).unwrap();
    assert!(matches!(
        atlas.insert_set(&[over]),
        Err(AtlasError::TooLarge { .. })
    ));
}

/// Atlas check : a frame with pixels on an empty slot counts
/// all its pixels as mismatches; an empty frame on an empty slot none.
#[test]
fn check_counts_a_frame_given_an_empty_slot() {
    use d2_client::frames::{Atlas, AtlasSlot, IndexFrame};

    let atlas = Atlas::new(1).unwrap();
    let f = IndexFrame::new(2, 3, 0, 0, vec![1; 6]).unwrap();
    let r = atlas.check([(AtlasSlot::EMPTY, &f)]).unwrap();
    assert_eq!((r.frame_mismatches, r.gutter_mismatches), (6, 0));
    let empty = IndexFrame::new(0, 3, 0, 0, Vec::new()).unwrap();
    assert!(atlas.check([(AtlasSlot::EMPTY, &empty)]).unwrap().is_ok());
}

/// Atlas check: the gutter ring includes the row above the slot.
/// A slot claimed one row below a placed frame sees that frame's last row
/// in its top ring.
#[test]
fn check_reads_the_ring_row_above_the_slot() {
    use d2_client::frames::{Atlas, AtlasSlot, IndexFrame};

    let mut atlas = Atlas::new(1).unwrap();
    let f = IndexFrame::new(3, 2, 0, 0, vec![7; 6]).unwrap();
    let s = atlas.insert_set(std::slice::from_ref(&f)).unwrap()[0];
    let row = IndexFrame::new(3, 1, 0, 0, vec![7; 3]).unwrap();
    let below = AtlasSlot {
        y: s.y + 1,
        h: 1,
        ..s
    };
    let r = atlas.check([(below, &row)]).unwrap();
    assert_eq!((r.frame_mismatches, r.gutter_mismatches), (0, 3));
    assert_eq!(r.first, Some((s.page, s.x, s.y)));
}

/// The atlas shelf packer (first fit, in order): a new
/// shelf opens on the page while it fits above the bottom gutter, and the
/// next page takes the first frame that does not.
#[test]
fn shelves_fill_a_page_to_its_bottom_gutter() {
    use d2_client::frames::{Atlas, IndexFrame, GUTTER, PAGE_SIZE};

    let (w, h) = (PAGE_SIZE - 2 * GUTTER, 1022);
    let frame = || IndexFrame::new(w, h, 0, 0, vec![3; (w * h) as usize]).unwrap();
    let frames = [frame(), frame(), frame()];
    let mut atlas = Atlas::new(2).unwrap();
    let slots = atlas.insert_set(&frames).unwrap();
    let at: Vec<(u32, u32, u32)> = slots.iter().map(|s| (s.page, s.x, s.y)).collect();
    // Shelf 2 ends at 1024 + 1022 = 2046, leaving the 1-pixel gutter
    // and one spare row: a third shelf of 1022 does not fit.
    assert_eq!(at, vec![(0, 1, 1), (0, 1, 1 + h + GUTTER), (1, 1, 1)]);
    let report = atlas.check(slots.iter().copied().zip(&frames)).unwrap();
    assert!(report.is_ok(), "{report:?}");
}

/// The gutter around the page edge: a shelf that would end on the page's
/// last row (no bottom gutter) does not open there; the frame goes to the
/// next page.
#[test]
fn a_shelf_keeps_the_bottom_gutter() {
    use d2_client::frames::{Atlas, IndexFrame, GUTTER, PAGE_SIZE};

    let w = PAGE_SIZE - 2 * GUTTER;
    let first = IndexFrame::new(w, 1022, 0, 0, vec![3; (w * 1022) as usize]).unwrap();
    // The second shelf would start at 1024 and end at row 2047, the last.
    let h = PAGE_SIZE - (GUTTER + 1022 + GUTTER);
    let second = IndexFrame::new(w, h, 0, 0, vec![4; (w * h) as usize]).unwrap();
    let mut atlas = Atlas::new(2).unwrap();
    let slots = atlas.insert_set(&[first, second]).unwrap();
    let at: Vec<(u32, u32)> = slots.iter().map(|s| (s.page, s.y)).collect();
    assert_eq!(at, vec![(0, 1), (1, 1)]);
}
