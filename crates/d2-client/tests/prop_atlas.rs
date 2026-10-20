//! Property tests on the atlas packer: random frame sizes (empty, tiny,
//! wide, tall, up to the largest slot, too large, bad pixel counts) in
//! random sets, with random page clears. Invariants: every slot
//! lies inside its page with the 1-pixel gutter of index 0 to the page
//! edge, no two live slots (or their gutters) overlap, each slot reads
//! back the frame's bytes (`Atlas::check` clean), a failed insert leaves
//! the atlas unchanged, and packing depends only on the insert order.

use crate::prop_support;

use d2_client::frames::atlas::MAX_SIDE;
use d2_client::frames::{Atlas, AtlasError, AtlasSlot, IndexFrame, GUTTER, PAGE_SIZE};
use proptest::prelude::*;

use prop_support::{bounded, config};

/// A frame of `w × h` with non-zero pixels (so a slot reads back
/// distinguishable bytes and the gutter check sees any spill).
fn frame(w: u32, h: u32, seed: u8) -> IndexFrame {
    let n = (w * h) as usize;
    let pixels = (0..n).map(|i| (i as u8).wrapping_add(seed) | 1).collect();
    IndexFrame::new(w, h, 0, 0, pixels).unwrap()
}

/// Sides: mostly small, sometimes large, at the limit, or past it.
fn side() -> impl Strategy<Value = u32> {
    prop_oneof![
        8 => 0u32..48,
        2 => 48u32..400,
        1 => 400u32..1200,
        1 => prop_oneof![Just(MAX_SIDE), Just(MAX_SIDE + 1), Just(PAGE_SIZE)],
    ]
}

/// A frame spec: a size, and whether its pixel count is wrong.
fn frame_spec() -> impl Strategy<Value = (u32, u32, bool)> {
    (side(), side(), proptest::bool::weighted(0.03))
        .prop_filter("keep frames small enough to build", |(w, h, _)| {
            u64::from(*w) * u64::from(*h) <= 1 << 21
        })
}

#[derive(Debug, Clone)]
enum Op {
    Insert(Vec<(u32, u32, bool)>),
    Clear(u32),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => proptest::collection::vec(frame_spec(), 0..12).prop_map(Op::Insert),
        1 => (0u32..4).prop_map(Op::Clear),
    ]
}

/// A slot as a gutter-expanded rect, for overlap tests.
fn expanded(s: &AtlasSlot) -> (u32, u32, u32, u32) {
    (
        s.x - GUTTER,
        s.y - GUTTER,
        s.x + s.w + GUTTER,
        s.y + s.h + GUTTER,
    )
}

fn overlap(a: &AtlasSlot, b: &AtlasSlot) -> bool {
    // Interior of `a` against the gutter-expanded `b`: slots may share a
    // gutter column, never a pixel of one's slot with the other's ring.
    let (bx0, by0, bx1, by1) = expanded(b);
    a.page == b.page && a.x < bx1 && bx0 < a.x + a.w && a.y < by1 && by0 < a.y + a.h
}

fn run(max_pages: u32, ops: Vec<Op>) -> Vec<Result<Vec<AtlasSlot>, AtlasError>> {
    let mut atlas = Atlas::new(max_pages).unwrap();
    // Live slots with their frames, per page generation.
    let mut live: Vec<(AtlasSlot, IndexFrame)> = Vec::new();
    let mut results = Vec::new();
    let mut seed = 0u8;
    for op in ops {
        match op {
            Op::Insert(specs) => {
                let frames: Vec<IndexFrame> = specs
                    .iter()
                    .map(|&(w, h, bad)| {
                        seed = seed.wrapping_add(37);
                        let mut f = frame(w, h, seed);
                        if bad {
                            f.pixels.push(7);
                        }
                        f
                    })
                    .collect();
                let before: Vec<_> = atlas.pages().to_vec();
                let r = atlas.insert_set(&frames);
                match &r {
                    Ok(slots) => {
                        assert_eq!(slots.len(), frames.len());
                        for (s, f) in slots.iter().zip(&frames) {
                            if f.is_empty() {
                                assert_eq!(*s, AtlasSlot::EMPTY);
                                assert_eq!(atlas.read(*s).unwrap(), Vec::<u8>::new());
                                continue;
                            }
                            assert_eq!((s.w, s.h), (f.width, f.height));
                            assert!(s.page < max_pages);
                            assert!(s.x >= GUTTER && s.y >= GUTTER, "{s:?}");
                            assert!(s.x + s.w + GUTTER <= PAGE_SIZE, "{s:?}");
                            assert!(s.y + s.h + GUTTER <= PAGE_SIZE, "{s:?}");
                            for (other, _) in &live {
                                assert!(!overlap(s, other), "{s:?} overlaps {other:?}");
                            }
                            assert_eq!(atlas.read(*s).unwrap(), f.pixels);
                            live.push((*s, f.clone()));
                        }
                    }
                    Err(e) => {
                        assert_eq!(
                            atlas.pages(),
                            &before[..],
                            "failed insert changed the atlas ({e})"
                        );
                        let bad = frames.iter().any(|f| {
                            f.pixels.len() as u64 != u64::from(f.width) * u64::from(f.height)
                        });
                        let large = frames
                            .iter()
                            .any(|f| !f.is_empty() && (f.width > MAX_SIDE || f.height > MAX_SIDE));
                        match e {
                            AtlasError::PixelCount { .. } => assert!(bad),
                            AtlasError::TooLarge { .. } => assert!(large),
                            AtlasError::Full { pages } => {
                                assert!(!bad && !large);
                                assert_eq!(*pages, max_pages);
                            }
                            other => panic!("unexpected insert error {other}"),
                        }
                    }
                }
                results.push(r);
            }
            Op::Clear(page) => {
                let r = atlas.clear_page(page);
                if (page as usize) < atlas.pages().len() {
                    r.unwrap();
                    assert!(atlas.pages()[page as usize].pixels.iter().all(|&p| p == 0));
                    live.retain(|(s, _)| s.page != page);
                } else {
                    assert_eq!(r, Err(AtlasError::NoPage(page)));
                }
            }
        }
        assert!(atlas.pages().len() as u32 <= max_pages);
        let report = atlas
            .check(live.iter().map(|(s, f)| (*s, f)))
            .expect("live slots name live pages");
        assert!(report.is_ok(), "{report:?}");
    }
    results
}

proptest! {
    #![proptest_config(config(24))]

    /// Packing invariants under random inserts and clears, and
    /// determinism: the same op sequence packs to the same slots.
    #[test]
    fn packing_invariants(max_pages in 1u32..=3, ops in proptest::collection::vec(op(), 0..10)) {
        bounded(move || {
            let a = run(max_pages, ops.clone());
            let b = run(max_pages, ops);
            assert_eq!(a, b);
        });
    }

    /// Many small frames into one page: the page fills (a `Full` error,
    /// atlas unchanged) without any slot leaving the page.
    #[test]
    fn fill_one_page(w in 1u32..300, h in 1u32..300, n in 1usize..200) {
        bounded(move || {
            let ops = (0..8)
                .map(|_| Op::Insert(vec![(w, h, false); n]))
                .collect();
            run(1, ops);
        });
    }
}
