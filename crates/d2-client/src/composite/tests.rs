use d2_formats::cof::Cof;

use super::*;
use crate::frames::FramePart;
use crate::scene::{compose, FrameImage, MapId, MapTable};

/// A synthetic COF: header, `layers` as (component, weapon class), the `events` block,
/// then the draw `order` (directions × frames × layers bytes).
fn cof_bytes(f: u8, d: u8, layers: &[(u8, &[u8; 3])], events: &[u8], order: &[u8]) -> Vec<u8> {
    let l = u8::try_from(layers.len()).unwrap();
    let mut v = vec![l, f, d, 20, 0, 0, 0, 0];
    for x in [-10i32, 10, -20, 0] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&25u32.to_le_bytes());
    for (i, (c, wc)) in layers.iter().enumerate() {
        // shadow, selectable, override translucency, new level: distinct
        // per layer so tests can see which record a slot got.
        let i = u8::try_from(i).unwrap();
        v.extend_from_slice(&[*c, i, 1, 0, i]);
        v.extend_from_slice(&wc[..]);
        v.push(0);
    }
    v.extend_from_slice(events);
    v.extend_from_slice(order);
    v
}

fn parse(bytes: &[u8]) -> Cof {
    Cof::parse(bytes).expect("synthetic COF parses")
}

/// Three layers HD(0), TR(1), RH(5); two directions, two frames. The
/// order differs per (d, f) so a wrong index shows.
fn three_layer() -> Cof {
    let order = [
        1, 0, 5, // d0 f0: TR HD RH
        5, 1, 0, // d0 f1: RH TR HD
        0, 5, 1, // d1 f0: HD RH TR
        1, 5, 0, // d1 f1: TR RH HD
    ];
    parse(&cof_bytes(
        2,
        2,
        &[(0, b"hth"), (1, b"hth"), (5, b"1hs")],
        &[1, 0],
        &order,
    ))
}

fn components(slots: &[Slot]) -> Vec<u8> {
    slots.iter().map(|s| s.component).collect()
}

#[test]
fn slot_order_reads_draw_order_per_direction_and_frame() {
    let cof = three_layer();
    assert_eq!(components(&slot_order(&cof, 0, 0).unwrap()), [1, 0, 5]);
    assert_eq!(components(&slot_order(&cof, 0, 1).unwrap()), [5, 1, 0]);
    assert_eq!(components(&slot_order(&cof, 1, 0).unwrap()), [0, 5, 1]);
    assert_eq!(components(&slot_order(&cof, 1, 1).unwrap()), [1, 5, 0]);
    let s = slot_order(&cof, 1, 0).unwrap();
    assert_eq!(s.iter().map(|s| s.slot).collect::<Vec<_>>(), [0, 1, 2]);
    // Layer records are found by component, not by slot position.
    assert_eq!(s.iter().map(|s| s.layer).collect::<Vec<_>>(), [0, 2, 1]);
}

#[test]
fn one_layer_two_frames() {
    // The smallest shape: one layer, two frames, one direction, events 01 00, order 01 01.
    let cof = parse(&cof_bytes(2, 1, &[(1, b"hth")], &[1, 0], &[1, 1]));
    for f in 0..2 {
        let s = slot_order(&cof, 0, f).unwrap();
        assert_eq!(
            s,
            [Slot {
                slot: 0,
                component: 1,
                layer: 0
            }]
        );
    }
}

#[test]
fn out_of_range_direction_and_row_past_end_are_errors() {
    let cof = three_layer();
    assert_eq!(
        slot_order(&cof, 2, 0),
        Err(CompositeError::Direction {
            dir: 2,
            directions: 2
        })
    );
    // d1, frame 2 starts at the file end.
    assert_eq!(
        slot_order(&cof, 1, 2),
        Err(CompositeError::RowPastEnd { dir: 1, frame: 2 })
    );
    // Zero directions: nothing is in range.
    let empty = parse(&cof_bytes(1, 0, &[(1, b"hth")], &[0], &[]));
    assert!(matches!(
        slot_order(&empty, 0, 0),
        Err(CompositeError::Direction { .. })
    ));
}

#[test]
fn frame_past_cof_frames_reads_the_next_direction_row() {
    // Frames are not bound-checked: d0 f2 is d1 f0's row.
    let cof = three_layer();
    assert_eq!(components(&slot_order(&cof, 0, 2).unwrap()), [0, 5, 1]);
    assert_eq!(components(&slot_order(&cof, 0, 3).unwrap()), [1, 5, 0]);
}

#[test]
fn draw_order_component_without_layer_draws_nothing() {
    // Layers HD, TR; frame 0 draws LG (2), which has no record: the slot
    // is left out, the others keep their index.
    let cof = parse(&cof_bytes(1, 1, &[(0, b"hth"), (1, b"hth")], &[0], &[2, 1]));
    assert_eq!(
        slot_order(&cof, 0, 0).unwrap(),
        [Slot {
            slot: 1,
            component: 1,
            layer: 1
        }]
    );
}

#[test]
fn padded_event_block_shifts_the_row_early() {
    // Shaped like `f9NUHTH.COF`: 42 bytes, one layer, one frame, one direction, a 4-byte
    // event block. Layer TR; the stored order byte is 1 (TR), but the row is read 3 bytes
    // early at offset 38, which holds 0 (HD). HD has no layer record: nothing is drawn.
    let bytes = cof_bytes(1, 1, &[(1, b"hth")], &[0, 0, 0, 0], &[1]);
    assert_eq!(bytes.len(), 42);
    let cof = parse(&bytes);
    assert_eq!(cof.event_padding.len(), 3);
    assert_eq!(draw_order_row(&cof, 0, 0).unwrap(), [0]);
    assert_eq!(slot_order(&cof, 0, 0).unwrap(), []);
}

#[test]
fn s7_slot_has_no_own_graphic_and_is_the_inline_slot() {
    // Layers TR, S7 (14); order TR, S7.
    let cof = parse(&cof_bytes(
        1,
        1,
        &[(1, b"hth"), (14, b"hth")],
        &[0],
        &[1, 14],
    ));
    assert_eq!(components(&slot_order(&cof, 0, 0).unwrap()), [1]);
    assert_eq!(inline_slot(&cof, 0, 0).unwrap(), Some(1));
    assert_eq!(inline_slot(&three_layer(), 0, 0).unwrap(), None);
}

/// A duplicate layer record is not an error; every slot of the component uses the first
/// record.
#[test]
fn duplicate_layer_records_use_the_first_match() {
    let cof = parse(&cof_bytes(1, 1, &[(1, b"hth"), (1, b"1hs")], &[0], &[1, 1]));
    let slots = slot_order(&cof, 0, 0).unwrap();
    assert_eq!(components(&slots), [1, 1]);
    assert!(slots.iter().all(|s| s.layer == 0));
}

#[test]
fn inconsistent_hand_built_cof_is_an_error() {
    let mut cof = three_layer();
    cof.draw_order.pop();
    assert_eq!(
        slot_order(&cof, 0, 0),
        Err(CompositeError::DrawOrderLength {
            len: 11,
            expected: 12
        })
    );
    let mut cof = three_layer();
    cof.layers.pop();
    assert_eq!(
        slot_order(&cof, 0, 0),
        Err(CompositeError::LayerCount {
            layers: 3,
            count: 2
        })
    );
    let mut cof = three_layer();
    cof.layers[2].component = 16;
    assert_eq!(
        slot_order(&cof, 0, 0),
        Err(CompositeError::LayerComponent {
            index: 2,
            component: 16
        })
    );
}

/// Test resolver: component `c` draws frame set `c<c>.dcc` direction `dir`,
/// frame `frame`, scene frame `FrameId(c)`, at `(10·c, 20 + frame)`. These
/// are test fixtures, not the game rules.
struct Fixture {
    resident: bool,
    shade: Option<MapId>,
}

impl Fixture {
    fn unresolved(req: &ComponentRequest<'_>, what: &'static str) -> CompositeError {
        CompositeError::Unresolved {
            slot: req.slot.slot,
            component: req.slot.component,
            what,
            message: "not resident".into(),
        }
    }
}

impl ComponentResolver for Fixture {
    fn frame(&self, req: &ComponentRequest<'_>) -> Result<ComponentFrame, CompositeError> {
        let dir = u8::try_from(req.dir).unwrap();
        Ok(ComponentFrame {
            set: FrameSetKey::new(format!("c{}.dcc", req.slot.component), FramePart::Dir(dir))
                .unwrap(),
            index: req.frame,
        })
    }

    fn frame_id(
        &self,
        req: &ComponentRequest<'_>,
        _: &ComponentFrame,
    ) -> Result<FrameId, CompositeError> {
        if self.resident {
            Ok(FrameId(u32::from(req.slot.component)))
        } else {
            Err(Self::unresolved(req, "frame_id"))
        }
    }

    fn place(
        &self,
        req: &ComponentRequest<'_>,
        frame: &ComponentFrame,
    ) -> Result<(i32, i32), CompositeError> {
        let c = i32::from(req.slot.component);
        Ok((10 * c, 20 + i32::try_from(frame.index).unwrap()))
    }

    fn shade(&self, _: &ComponentRequest<'_>) -> Result<ShadeChain, CompositeError> {
        Ok(match self.shade {
            Some(m) => ShadeChain::new(&[m]).unwrap(),
            None => ShadeChain::EMPTY,
        })
    }

    fn blend(&self, req: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        // Exposes the layer record the hook saw.
        assert_eq!(req.layer.component, req.slot.component);
        assert_eq!(req.layer as *const _, &req.cof.layers[req.slot.layer]);
        Ok(BlendOp::Opaque)
    }
}

const UNIT: UnitParams = UnitParams {
    pass: 3,
    major: 1000,
    minor: 77,
    clip: Rect::new(0, 0, 400, 300),
    tag: ItemTag::Unit(42),
};

const FIXTURE: Fixture = Fixture {
    resident: true,
    shade: None,
};

#[test]
fn build_gives_one_item_per_slot_in_slot_order() {
    let cof = three_layer();
    let draws = build(&cof, 1, 1, &UNIT, &FIXTURE).unwrap();
    // d1 f1: TR RH HD.
    let comps: Vec<u8> = draws.iter().map(|d| d.slot.component).collect();
    assert_eq!(comps, [1, 5, 0]);
    for (s, d) in draws.iter().enumerate() {
        let c = d.slot.component;
        assert_eq!(usize::from(d.slot.slot), s);
        assert_eq!(
            d.frame,
            ComponentFrame {
                set: FrameSetKey::new(format!("c{c}.dcc"), FramePart::Dir(1)).unwrap(),
                index: 1,
            }
        );
        let it = &d.item;
        assert_eq!(it.frame, FrameId(u32::from(c)));
        assert_eq!((it.x, it.y), (10 * i32::from(c), 21));
        assert_eq!(it.key.pass(), 3);
        assert_eq!(it.key.major(), 1000);
        assert_eq!(it.key.minor(), 77);
        assert_eq!(usize::from(it.key.sub()), s);
        assert_eq!(it.clip, UNIT.clip);
        assert_eq!(it.tag, ItemTag::Unit(42));
        assert_eq!(it.blend, BlendOp::Opaque);
        assert_eq!(it.shade, ShadeChain::EMPTY);
        assert!(!it.flip_x);
    }
    // List order is key order: sorting changes nothing.
    let mut items: Vec<DrawItem> = draws.iter().map(|d| d.item).collect();
    let before = items.clone();
    crate::scene::order(&mut items);
    assert_eq!(items, before);
}

#[test]
fn build_passes_hook_shade_through() {
    let cof = three_layer();
    let r = Fixture {
        resident: true,
        shade: Some(MapId(4)),
    };
    let draws = build(&cof, 0, 0, &UNIT, &r).unwrap();
    assert!(draws.iter().all(|d| d.item.shade.maps() == [MapId(4)]));
}

#[test]
fn composed_unit_draws_back_to_front() {
    // Two components drawn at the same spot: HD (0) frame all 3s, TR (1)
    // all 7s. Frame 0 order TR, HD (HD on top); frame 1 HD, TR.
    let cof = parse(&cof_bytes(
        2,
        1,
        &[(0, b"hth"), (1, b"hth")],
        &[0, 0],
        &[1, 0, 0, 1],
    ));
    struct Same;
    impl ComponentResolver for Same {
        fn frame(&self, req: &ComponentRequest<'_>) -> Result<ComponentFrame, CompositeError> {
            FIXTURE.frame(req)
        }
        fn frame_id(
            &self,
            req: &ComponentRequest<'_>,
            f: &ComponentFrame,
        ) -> Result<FrameId, CompositeError> {
            FIXTURE.frame_id(req, f)
        }
        fn place(
            &self,
            _: &ComponentRequest<'_>,
            _: &ComponentFrame,
        ) -> Result<(i32, i32), CompositeError> {
            Ok((1, 1))
        }
        fn shade(&self, req: &ComponentRequest<'_>) -> Result<ShadeChain, CompositeError> {
            FIXTURE.shade(req)
        }
        fn blend(&self, req: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
            FIXTURE.blend(req)
        }
    }
    let frames = vec![
        FrameImage {
            width: 2,
            height: 2,
            pixels: vec![3; 4],
        },
        FrameImage {
            width: 2,
            height: 2,
            pixels: vec![7; 4],
        },
    ];
    let view = Rect::new(0, 0, 4, 4);
    let maps = MapTable::new();
    let top = |f| {
        let mut items: Vec<DrawItem> = build(&cof, 0, f, &UNIT, &Same)
            .unwrap()
            .into_iter()
            .map(|d| d.item)
            .collect();
        crate::scene::order(&mut items);
        compose(&items, &frames, &maps, view).unwrap()[4 + 1]
    };
    assert_eq!(top(0), 3);
    assert_eq!(top(1), 7);
}

#[test]
fn hook_error_fails_the_whole_unit() {
    let cof = three_layer();
    let r = Fixture {
        resident: false,
        shade: None,
    };
    assert_eq!(
        build(&cof, 0, 1, &UNIT, &r),
        Err(CompositeError::Unresolved {
            slot: 0,
            component: 5,
            what: "frame_id",
            message: "not resident".into()
        })
    );
}

#[test]
fn unit_key_fields_out_of_range_are_errors() {
    let cof = three_layer();
    let unit = UnitParams {
        major: DrawKey::MAJOR_MAX + 1,
        ..UNIT
    };
    assert!(matches!(
        build(&cof, 0, 0, &unit, &FIXTURE),
        Err(CompositeError::Scene {
            slot: 0,
            error: SceneError::KeyField { .. }
        })
    ));
}

/// Game files: every live COF gives a slot order for every direction and
/// frame (no component without a layer, no duplicate layer records).
/// `D2_GAME_DIR=... cargo test -p d2-client --lib composite::tests::all_live_cofs_give_slot_orders -- --ignored --nocapture`
#[test]
#[ignore = "needs D2_GAME_DIR"]
fn all_live_cofs_give_slot_orders() {
    use std::collections::BTreeSet;

    use d2_formats::mpq::ArchiveSet;

    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let set = ArchiveSet::open_dir(&dir).unwrap();
    let mut names = BTreeSet::new();
    for a in set.archives() {
        for n in a.listfile().unwrap().unwrap_or_default() {
            let l = n.to_ascii_lowercase();
            if l.ends_with(".cof") {
                names.insert(l);
            }
        }
    }
    let (mut files, mut parse_errors, mut frames) = (0, Vec::new(), 0u64);
    let mut failures = Vec::new();
    for name in &names {
        let Ok(bytes) = set.read(name) else { continue };
        let cof = match Cof::parse(&bytes) {
            Ok(c) => c,
            Err(_) => {
                parse_errors.push(name.clone());
                continue;
            }
        };
        files += 1;
        'file: for d in 0..usize::from(cof.directions) {
            for f in 0..usize::from(cof.frames) {
                if let Err(e) = slot_order(&cof, d, f) {
                    failures.push(format!("{name} d{d} f{f}: {e}"));
                    break 'file;
                }
                frames += 1;
            }
        }
    }
    println!(
        "cofs {files}, parse errors {parse_errors:?}, frames {frames}, failures {}",
        failures.len()
    );
    for f in failures.iter().take(20) {
        println!("  {f}");
    }
    assert!(failures.is_empty());
}

#[test]
fn slot_without_frame_draws_nothing_and_others_keep_their_sub() {
    // A resolver whose request for TR (1) fails: that slot gives no item,
    // no error; HD and RH keep slot index (= key `sub`) 2 and 1.
    struct NoTorso;
    impl ComponentResolver for NoTorso {
        fn frame(&self, req: &ComponentRequest<'_>) -> Result<ComponentFrame, CompositeError> {
            FIXTURE.frame(req)
        }
        fn slot_frame(
            &self,
            req: &ComponentRequest<'_>,
        ) -> Result<Option<ComponentFrame>, CompositeError> {
            if req.slot.component == 1 {
                return Ok(None);
            }
            self.frame(req).map(Some)
        }
        fn frame_id(
            &self,
            req: &ComponentRequest<'_>,
            f: &ComponentFrame,
        ) -> Result<FrameId, CompositeError> {
            FIXTURE.frame_id(req, f)
        }
        fn place(
            &self,
            req: &ComponentRequest<'_>,
            f: &ComponentFrame,
        ) -> Result<(i32, i32), CompositeError> {
            FIXTURE.place(req, f)
        }
        fn shade(&self, req: &ComponentRequest<'_>) -> Result<ShadeChain, CompositeError> {
            FIXTURE.shade(req)
        }
        fn blend(&self, req: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
            FIXTURE.blend(req)
        }
    }
    let draws = build(&three_layer(), 1, 1, &UNIT, &NoTorso).unwrap();
    // d1 f1: TR RH HD → RH (slot 1), HD (slot 2).
    let got: Vec<(u8, u8)> = draws
        .iter()
        .map(|d| (d.slot.component, d.item.key.sub()))
        .collect();
    assert_eq!(got, [(5, 1), (0, 2)]);
}
