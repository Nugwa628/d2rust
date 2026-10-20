//! Frame store: ids in insertion order, strict lookups, the CPU frame
//! source and the atlas numbering (`slots[n]` = `FrameId(n)`).

use super::*;
use crate::frames::FramePart;
use crate::scene::FrameSource;

fn key(path: &str, dir: u8) -> FrameSetKey {
    FrameSetKey::new(path, FramePart::Dir(dir)).unwrap()
}

/// A set of `n` frames; frame `i` is `(i + 1)`×2 filled with `base + i`.
fn set(n: usize, base: u8) -> FrameSet {
    FrameSet {
        frames: (0..n)
            .map(|i| {
                let w = i as u32 + 1;
                IndexFrame::new(
                    w,
                    2,
                    i as i32,
                    -(i as i32),
                    vec![base + i as u8; 2 * w as usize],
                )
                .unwrap()
            })
            .collect(),
    }
}

#[test]
fn ids_follow_insertion_order() {
    let mut store = FrameStore::new();
    assert_eq!(store.insert(key("a.dcc", 0), set(3, 10)), Ok(FrameId(0)));
    assert_eq!(store.insert(key("a.dcc", 1), set(2, 20)), Ok(FrameId(3)));
    assert_eq!(store.insert(key("b.dc6", 0), set(1, 30)), Ok(FrameId(5)));
    assert_eq!(store.len(), 6);
    assert_eq!(store.id(&key("a.dcc", 0), 2), Ok(FrameId(2)));
    assert_eq!(store.id(&key("a.dcc", 1), 0), Ok(FrameId(3)));
    assert_eq!(store.id(&key("a.dcc", 1), 1), Ok(FrameId(4)));
    assert_eq!(store.id(&key("b.dc6", 0), 0), Ok(FrameId(5)));
    // Each id reads back its own frame and owner.
    for id in 0..6 {
        let (k, i) = store.owner(FrameId(id)).unwrap();
        assert_eq!(store.id(k, i), Ok(FrameId(id)));
        let f = store.frame(FrameId(id)).unwrap();
        assert_eq!(f.width, i as u32 + 1);
    }
    assert_eq!(store.frame(FrameId(4)).unwrap().pixels[0], 21);
    assert_eq!(store.frame(FrameId(6)), None);
}

#[test]
fn same_insertions_give_same_ids() {
    let build = || {
        let mut s = FrameStore::new();
        s.insert(key("x.dcc", 3), set(2, 1)).unwrap();
        s.insert(key("a.dcc", 0), set(4, 5)).unwrap();
        s
    };
    assert_eq!(build(), build());
    // Order is insertion order, not key order: "x" first.
    assert_eq!(build().id(&key("x.dcc", 3), 0), Ok(FrameId(0)));
}

#[test]
fn lookups_are_strict() {
    let mut store = FrameStore::new();
    store.insert(key("a.dcc", 0), set(2, 1)).unwrap();
    assert_eq!(
        store.insert(key("a.dcc", 0), set(1, 1)),
        Err(StoreError::Duplicate(key("a.dcc", 0)))
    );
    // The failed insert changed nothing.
    assert_eq!(store.len(), 2);
    assert_eq!(
        store.id(&key("a.dcc", 1), 0),
        Err(StoreError::NotResident(key("a.dcc", 1)))
    );
    assert_eq!(
        store.id(&key("a.dcc", 0), 2),
        Err(StoreError::Index {
            key: key("a.dcc", 0),
            index: 2,
            len: 2
        })
    );
}

#[test]
fn empty_set_takes_no_ids() {
    let mut store = FrameStore::new();
    assert_eq!(store.insert(key("t.dt1", 0), set(0, 0)), Ok(FrameId(0)));
    assert!(store.contains(&key("t.dt1", 0)));
    assert!(matches!(
        store.id(&key("t.dt1", 0), 0),
        Err(StoreError::Index { len: 0, .. })
    ));
    assert_eq!(store.insert(key("a.dcc", 0), set(1, 9)), Ok(FrameId(0)));
}

#[test]
fn frame_source_reads_store_pixels() {
    let mut store = FrameStore::new();
    store.insert(key("a.dcc", 0), set(2, 7)).unwrap();
    let v = store.frame(FrameId(1)).map(|f| f.pixels.clone()).unwrap();
    let view = FrameSource::frame(&store, FrameId(1)).unwrap();
    assert_eq!((view.width(), view.height()), (2, 2));
    assert_eq!(view.pixels(), v.as_slice());
    assert_eq!(
        FrameSource::frame(&store, FrameId(2)).err(),
        Some(SceneError::FrameMissing(FrameId(2)))
    );
}

#[test]
fn atlas_slot_n_holds_frame_id_n() {
    let mut store = FrameStore::new();
    store.insert(key("a.dcc", 0), set(3, 40)).unwrap();
    store.insert(key("b.dcc", 0), set(2, 80)).unwrap();
    let atlas = store.atlas(1).unwrap();
    assert_eq!(atlas.slots.len(), store.len());
    for (n, slot) in atlas.slots.iter().enumerate() {
        let f = store.frame(FrameId(n as u32)).unwrap();
        assert_eq!((slot.w, slot.h), (f.width, f.height));
        assert_eq!(atlas.atlas.read(*slot).unwrap(), f.pixels);
    }
}

#[test]
fn atlas_overflow_is_an_error() {
    let mut store = FrameStore::new();
    let big = |v| IndexFrame::new(2000, 2000, 0, 0, vec![v; 4_000_000]).unwrap();
    store
        .insert(
            key("big.dc6", 0),
            FrameSet {
                frames: vec![big(1), big(2)],
            },
        )
        .unwrap();
    assert!(store.atlas(1).is_err());
    assert!(store.atlas(2).is_ok());
}
