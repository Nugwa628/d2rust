//! [`build_with`]: frame ids from a [`FrameStore`] instead of the
//! resolver's `frame_id` hook.

use d2_formats::cof::Cof;

use super::*;
use crate::frames::{FramePart, FrameSet, IndexFrame};

/// A synthetic COF (as in `tests.rs`): header, layers (component, weapon class), an
/// event block of one byte per frame, then the draw order.
fn cof(frames: u8, dirs: u8, layers: &[u8], order: &[u8]) -> Cof {
    let mut v = vec![layers.len() as u8, frames, dirs, 20, 0, 0, 0, 0];
    for x in [-10i32, 10, -20, 0] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&25u32.to_le_bytes());
    for &c in layers {
        v.extend_from_slice(&[c, 0, 1, 0, 0]);
        v.extend_from_slice(b"hth\0");
    }
    v.extend(std::iter::repeat_n(0, usize::from(frames)));
    v.extend_from_slice(order);
    Cof::parse(&v).expect("synthetic COF parses")
}

fn key(component: u8, dir: u8) -> FrameSetKey {
    FrameSetKey::new(format!("unit/c{component}.dcc"), FramePart::Dir(dir)).unwrap()
}

/// Component `c` draws frame `frame` of `unit/c<c>.dcc`, direction `dir`,
/// at `(c, 0)`; its own `frame_id` must never be asked.
struct Hooks;

impl ComponentResolver for Hooks {
    fn frame(&self, req: &ComponentRequest<'_>) -> Result<ComponentFrame, CompositeError> {
        Ok(ComponentFrame {
            set: key(req.slot.component, req.dir as u8),
            index: req.frame,
        })
    }

    fn frame_id(
        &self,
        _: &ComponentRequest<'_>,
        _: &ComponentFrame,
    ) -> Result<FrameId, CompositeError> {
        panic!("build_with must take ids from the store");
    }

    fn place(
        &self,
        req: &ComponentRequest<'_>,
        _: &ComponentFrame,
    ) -> Result<(i32, i32), CompositeError> {
        Ok((i32::from(req.slot.component), 0))
    }

    fn shade(&self, _: &ComponentRequest<'_>) -> Result<ShadeChain, CompositeError> {
        Ok(ShadeChain::EMPTY)
    }

    fn blend(&self, _: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        Ok(BlendOp::Opaque)
    }
}

/// Like [`Hooks`] but keeps the trait's default `frame_id`.
struct NoIdHook;

impl ComponentResolver for NoIdHook {
    fn frame(&self, req: &ComponentRequest<'_>) -> Result<ComponentFrame, CompositeError> {
        Hooks.frame(req)
    }

    fn place(
        &self,
        req: &ComponentRequest<'_>,
        f: &ComponentFrame,
    ) -> Result<(i32, i32), CompositeError> {
        Hooks.place(req, f)
    }

    fn shade(&self, req: &ComponentRequest<'_>) -> Result<ShadeChain, CompositeError> {
        Hooks.shade(req)
    }

    fn blend(&self, req: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        Hooks.blend(req)
    }
}

fn frames(n: usize) -> FrameSet {
    FrameSet {
        frames: (0..n)
            .map(|i| IndexFrame::new(1, 1, 0, 0, vec![i as u8 + 1]).unwrap())
            .collect(),
    }
}

/// Two layers HD(0), TR(1); one direction, two frames; frame 0 draws
/// TR then HD, frame 1 HD then TR.
fn two_layer() -> Cof {
    cof(2, 1, &[0, 1], &[1, 0, 0, 1])
}

const UNIT: UnitParams = UnitParams {
    pass: 1,
    major: 2,
    minor: 3,
    clip: Rect::FRAME,
    tag: ItemTag::Unit(7),
};

#[test]
fn ids_come_from_the_store() {
    let mut store = FrameStore::new();
    // An unrelated set first, so ids are not the frame indices.
    store.insert(key(9, 0), frames(3)).unwrap();
    store.insert(key(0, 0), frames(2)).unwrap(); // ids 3, 4
    store.insert(key(1, 0), frames(2)).unwrap(); // ids 5, 6
    let cof = two_layer();
    let draws = build_with(&cof, 0, 0, &UNIT, &Hooks, &store).unwrap();
    let got: Vec<_> = draws
        .iter()
        .map(|d| (d.slot.component, d.item.frame))
        .collect();
    assert_eq!(got, [(1, FrameId(5)), (0, FrameId(3))]);
    let draws = build_with(&cof, 0, 1, &UNIT, &Hooks, &store).unwrap();
    let got: Vec<_> = draws
        .iter()
        .map(|d| (d.slot.component, d.item.frame))
        .collect();
    assert_eq!(got, [(0, FrameId(4)), (1, FrameId(6))]);
    // Each item's id reads back the frame set and index the hook chose.
    for d in &draws {
        assert_eq!(
            store.owner(d.item.frame),
            Some((&d.frame.set, d.frame.index))
        );
    }
}

#[test]
fn same_items_as_build_with_a_matching_hook() {
    struct Ids<'a>(&'a FrameStore);
    impl ComponentResolver for Ids<'_> {
        fn frame(&self, r: &ComponentRequest<'_>) -> Result<ComponentFrame, CompositeError> {
            Hooks.frame(r)
        }
        fn frame_id(
            &self,
            _: &ComponentRequest<'_>,
            f: &ComponentFrame,
        ) -> Result<FrameId, CompositeError> {
            Ok(self.0.id(&f.set, f.index).unwrap())
        }
        fn place(
            &self,
            r: &ComponentRequest<'_>,
            f: &ComponentFrame,
        ) -> Result<(i32, i32), CompositeError> {
            Hooks.place(r, f)
        }
        fn shade(&self, r: &ComponentRequest<'_>) -> Result<ShadeChain, CompositeError> {
            Hooks.shade(r)
        }
        fn blend(&self, r: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
            Hooks.blend(r)
        }
    }
    let mut store = FrameStore::new();
    store.insert(key(1, 0), frames(2)).unwrap();
    store.insert(key(0, 0), frames(2)).unwrap();
    let cof = two_layer();
    for f in 0..2 {
        assert_eq!(
            build_with(&cof, 0, f, &UNIT, &NoIdHook, &store),
            build(&cof, 0, f, &UNIT, &Ids(&store))
        );
    }
}

#[test]
fn missing_frame_fails_the_unit() {
    let mut store = FrameStore::new();
    store.insert(key(0, 0), frames(2)).unwrap();
    let cof = two_layer();
    // Slot 0 of frame 0 is TR (component 1): its set is not resident.
    match build_with(&cof, 0, 0, &UNIT, &Hooks, &store) {
        Err(CompositeError::Unresolved {
            slot: 0,
            component: 1,
            what: "frame_id",
            message,
        }) => assert!(message.contains("not resident"), "{message}"),
        other => panic!("{other:?}"),
    }
    // An index past the set's end.
    store.insert(key(1, 0), frames(1)).unwrap();
    match build_with(&cof, 0, 1, &UNIT, &Hooks, &store) {
        Err(CompositeError::Unresolved {
            slot: 1,
            component: 1,
            what: "frame_id",
            message,
        }) => assert!(message.contains("1 frames"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn default_frame_id_hook_refuses() {
    assert!(matches!(
        build(&two_layer(), 0, 0, &UNIT, &NoIdHook),
        Err(CompositeError::Unresolved {
            what: "frame_id",
            ..
        })
    ));
}
