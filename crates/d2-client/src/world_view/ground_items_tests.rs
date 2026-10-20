//! Ground items with synthetic fixtures: a model with items on the ground,
//! flippy DC6 files in a memory source, a link that keeps what it is sent.

use std::sync::{Arc, Mutex};

use d2_formats::palette::{Palette, Rgb};
use d2_proto::PROTOCOL_VERSION;

use super::*;
use crate::assets::path::MemorySource;
use crate::bridge::dispatch::Dispatch;
use crate::bridge::items::{ItemArtRow, ITEM};
use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent};
use crate::bridge::world::{ClientUnit, ItemData, ItemRecord, KindData, PlayerData, UnitKey};
use crate::ui::Point;
use crate::world_view::model_feed::ModelFeed;
use crate::world_view::NoFeed;

/// A stream head: flags, version, mode, location, code.
fn stream(m: u8, loc: (u16, u16), code: &[u8; 4]) -> Vec<u8> {
    let mut bits: Vec<(u32, u32)> = vec![(0x10, 32), (0x65, 10), (u32::from(m), 3)];
    if matches!(m, 3 | 5) {
        bits.extend([(u32::from(loc.0), 16), (u32::from(loc.1), 16)]);
    } else {
        bits.extend([(0, 4), (u32::from(loc.0), 4), (u32::from(loc.1), 4), (0, 3)]);
    }
    bits.push((u32::from_le_bytes(*code), 32));
    let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0);
    for (v, w) in bits {
        acc |= u64::from(v) << n;
        n += w;
        while n >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    }
    if n > 0 {
        out.push(acc as u8);
    }
    out
}

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

const PLAYER: UnitKey = UnitKey {
    unit_type: 0,
    guid: 1,
};

/// An item fixture: GUID, mode, location, code.
type Fixture<'a> = (u32, u8, (u16, u16), &'a [u8; 4]);

/// The local player at cell (100, 100); items `(guid, mode, (x, y),
/// code)`.
fn populate(w: &mut ClientWorld, items: &[Fixture<'_>]) {
    let mut p = ClientUnit::new(PLAYER);
    p.position = Some((100, 100));
    p.kind = KindData::Player(PlayerData::default());
    w.units.insert(PLAYER, p);
    w.local_player = Some(PLAYER);
    for &(guid, m, loc, code) in items {
        let k = UnitKey::new(ITEM, guid);
        let mut u = ClientUnit::new(k);
        if matches!(m, 3 | 5) {
            u.position = Some(loc);
        }
        // The item animation set-up: on the ground the
        // last frame of 0x1100, dropping from frame 0.
        u.frame = if m == 3 { 0x1000 } else { 0 };
        // The item update: the mode-3 branch sets the
        // selectable flag; a dropping item has not had it yet.
        u.mode = u32::from(m);
        u.flag_2 = (m == 3).then_some(true);
        u.kind = KindData::Item(ItemData {
            last: Some(ItemRecord {
                id: 0x9C,
                action: 0,
                category: 0,
                owner: None,
                seq: 0,
                stream: stream(m, loc, code),
            }),
            ..ItemData::default()
        });
        w.units.insert(k, u);
    }
}

fn rows() -> ItemArtRows {
    let mut r = ItemArtRows::default();
    r.0.insert(
        *b"cap ",
        ItemArtRow {
            inv_w: 2,
            inv_h: 2,
            inv_file: "invcap".into(),
            flippy_file: "flpcap".into(),
            beltable: false,
        },
    );
    r
}

fn source() -> Arc<dyn FileSource> {
    let mut src = MemorySource::default();
    src.insert("data\\global\\items\\flpcap.dc6", dc6(3, 4, 3));
    Arc::new(src)
}

fn assets() -> ViewAssets {
    ViewAssets::new(Palette {
        colors: [Rgb::default(); 256],
    })
}

#[test]
fn a_ground_item_draws_its_flippy_at_its_subtile() {
    let mut w = ClientWorld::default();
    populate(&mut w, &[(5, 3, (102, 100), b"cap ")]);
    let mut g = GroundItems::new(source(), rows());
    let mut a = assets();
    let mut frame = framed(&w);
    let feed = ModelFeed::<NoFeed>::default();
    let log = g.add_to_frame(&w, &feed, &mut a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    assert_eq!(a.frames.len(), 3, "the whole direction is resident");
    // Player: client (0, 1608); unit origin (−400, 1324). Item (102, 100):
    // client (32, 1616) → draw at (432, 300); the 4 × 3 cel is bottom
    // anchored: top-left (432, 298).
    assert_eq!(frame.items.len(), 1);
    let d = frame.items[0];
    assert_eq!((d.x, d.y), (432, 298));
    assert_eq!(d.tag, ItemTag::Unit(5));
    assert_eq!(d.key.0 >> 60, u64::from(pass::SHADOWS));
    // The resting cel: the flippy's last frame.
    let set = FrameSetKey::new("data/global/items/flpcap.dc6", FramePart::Dir(0)).unwrap();
    assert_eq!(d.frame, a.id(&set, 2).unwrap());
    assert_eq!(g.hit(433, 299), Some(5));
    assert_eq!(g.hit(436, 299), None);
    assert_eq!(g.hit(433, 301), None);
}

#[test]
fn a_dropping_item_draws_its_animation_frame() {
    let mut w = ClientWorld::default();
    populate(&mut w, &[(5, 5, (102, 100), b"cap ")]);
    let set = FrameSetKey::new("data/global/items/flpcap.dc6", FramePart::Dir(0)).unwrap();
    let mut g = GroundItems::new(source(), rows());
    let feed = ModelFeed::<NoFeed>::default();
    let mut a = assets();
    for (frame8, cel) in [(0, 0), (0x100, 1), (0x1F0, 1), (0x200, 2), (0x900, 2)] {
        w.units.get_mut(&UnitKey::new(ITEM, 5)).unwrap().frame = frame8;
        let mut frame = framed(&w);
        g.add_to_frame(&w, &feed, &mut a, &mut frame);
        assert_eq!(frame.items.len(), 1);
        assert_eq!(
            frame.items[0].frame,
            a.id(&set, cel).unwrap(),
            "frame {frame8:#x}"
        );
    }
}

#[test]
fn an_off_view_ground_item_keeps_its_cel_call() {
    let mut w = ClientWorld::default();
    populate(&mut w, &[(5, 3, (70, 100), b"cap ")]);
    let mut g = GroundItems::new(source(), rows());
    let mut a = assets();
    let mut frame = framed(&w);
    let feed = ModelFeed::<NoFeed>::default();
    let log = g.add_to_frame(&w, &feed, &mut a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    // 32 sub-tiles left of the item above: draw at (432 − 512, 300 − 256)
    // = (−80, 44); the 4 × 3 cel lies wholly left of the view.
    assert_eq!(frame.items.len(), 1, "no box pre-test for an item");
    let d = frame.items[0];
    assert_eq!((d.x, d.y), (-80, 42));
    // Left of the view: the rows stay in the frame, the
    // columns clip per pixel, so nothing fills and nothing is clickable.
    assert_eq!((d.clip.width, d.clip.height), (800, 600));
    assert_eq!(g.hit(-79, 43), None);
}

#[test]
fn a_ground_item_cel_is_drawn_mode_5_with_no_palette() {
    let mut w = ClientWorld::default();
    populate(&mut w, &[(5, 3, (102, 100), b"cap ")]);
    let mut g = GroundItems::new(source(), rows());
    let mut a = assets();
    let mut frame = framed(&w);
    let feed = ModelFeed::<NoFeed>::default();
    let log = g.add_to_frame(&w, &feed, &mut a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    let call = frame.items[0].call.expect("an item cel has call args");
    assert_eq!(call.mode, 5);
    assert_eq!(call.pal, crate::scene::CelPal::Null);
}

#[test]
fn a_ground_item_moves_with_the_frames_shaken_camera() {
    use crate::rules::camera::{ClientPos, FrameSize, OpenMode};
    let mut w = ClientWorld::default();
    populate(&mut w, &[(5, 3, (102, 100), b"cap ")]);
    let mut g = GroundItems::new(source(), rows());
    let mut a = assets();
    let feed = ModelFeed::<NoFeed>::default();
    // The player at client (0, 1608) (as above), a running shake (3, −2).
    let player = ClientPos { x: 0, y: 1608 };
    let plain = Camera::new(FrameSize::D2RUST, OpenMode::NONE, player, (0, 0));
    let shaken = Camera::new(FrameSize::D2RUST, OpenMode::NONE, player, (3, -2));
    let mut frame = WorldFrame {
        camera: Some(shaken),
        ..WorldFrame::default()
    };
    let log = g.add_to_frame(&w, &feed, &mut a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    let d = frame.items[0];
    // The tiles move by the unit origin's change; so does the item.
    let (dx, dy) = (shaken.unit.x - plain.unit.x, shaken.unit.y - plain.unit.y);
    assert_ne!((dx, dy), (0, 0));
    assert_eq!((d.x, d.y), (432 - dx, 298 - dy));
}

#[test]
fn no_rows_or_missing_art_draw_nothing_and_never_fail() {
    let mut w = ClientWorld::default();
    populate(
        &mut w,
        &[(5, 3, (102, 100), b"cap "), (6, 0, (1, 1), b"cap ")],
    );
    let feed = ModelFeed::<NoFeed>::default();
    let mut frame = framed(&w);
    let mut a = assets();
    // The default: no rows, no source.
    let log = GroundItems::default().add_to_frame(&w, &feed, &mut a, &mut frame);
    assert!(log.is_empty() && frame.items.is_empty());
    // A row whose file no archive holds: logged once, not drawn.
    let mut g = GroundItems::new(Arc::new(MemorySource::default()), rows());
    let log = g.add_to_frame(&w, &feed, &mut a, &mut frame);
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(frame.items.is_empty());
    assert!(g.add_to_frame(&w, &feed, &mut a, &mut frame).is_empty());
    // No local player: no camera, nothing drawn.
    let mut g = GroundItems::new(source(), rows());
    w.local_player = None;
    let mut frame = framed(&w);
    assert!(g.add_to_frame(&w, &feed, &mut a, &mut frame).is_empty());
    assert!(frame.items.is_empty());
}

/// A built frame of `w`: its one camera,
/// `None` without a local player.
fn framed(w: &ClientWorld) -> WorldFrame {
    let mut feed = ModelFeed::<NoFeed>::default();
    WorldFrame {
        camera: crate::world_view::frame_camera(w, &mut feed).unwrap(),
        ..WorldFrame::default()
    }
}

#[derive(Clone, Default)]
struct RecordingLink {
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ServerLink for RecordingLink {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, _: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.sent.lock().unwrap().push(msg.to_vec());
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        Ok(Pumped { ticked: false })
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

fn press(x: i32, y: i32) -> UiEvent {
    UiEvent::Press {
        button: PointerButton::Left,
        at: Point::new(x, y),
    }
}

/// A bridge whose model holds the player and the items, and the ground
/// items drawn for it.
fn scene(items: &[Fixture<'_>]) -> (Bridge<RecordingLink>, RecordingLink, GroundItems) {
    let link = RecordingLink::default();
    let mut b = Bridge::with_dispatch(link.clone(), Dispatch::empty()).unwrap();
    populate(b.world_mut(), items);
    let mut g = GroundItems::new(source(), rows());
    let mut frame = framed(b.world());
    let feed = ModelFeed::<NoFeed>::default();
    g.add_to_frame(b.world(), &feed, &mut assets(), &mut frame);
    (b, link, g)
}

#[test]
fn a_press_on_a_ground_item_sends_pick_item() {
    let (mut b, link, mut g) = scene(&[(5, 3, (102, 100), b"cap ")]);
    let rest = g
        .take_clicks(&mut b, &[press(433, 299), press(10, 10)])
        .unwrap();
    assert_eq!(
        rest,
        [press(10, 10)],
        "the ground click goes on to the walk"
    );
    assert_eq!(
        *link.sent.lock().unwrap(),
        [vec![0x16, 4, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0]]
    );
}

#[test]
fn a_press_on_a_dropping_item_goes_on_to_the_walk() {
    let (mut b, link, mut g) = scene(&[(5, 5, (102, 100), b"cap ")]);
    let rest = g.take_clicks(&mut b, &[press(433, 299)]).unwrap();
    assert_eq!(rest, [press(433, 299)], "not a click target while dropping");
    assert!(link.sent.lock().unwrap().is_empty());
}

#[test]
fn a_world_press_with_a_cursor_item_sends_drop_item() {
    let (mut b, link, mut g) = scene(&[(5, 3, (102, 100), b"cap "), (9, 4, (0, 0), b"cap ")]);
    if let Some(KindData::Player(d)) = b.world_mut().units.get_mut(&PLAYER).map(|u| &mut u.kind) {
        d.cursor_item = Some(9);
    }
    // Even over a ground item: the held item is dropped.
    let rest = g.take_clicks(&mut b, &[press(433, 299)]).unwrap();
    assert!(rest.is_empty());
    assert_eq!(*link.sent.lock().unwrap(), [vec![0x17, 9, 0, 0, 0]]);
}

// ---- gold piles and the pick-up walk (q-gold) --------------------------------------

/// A compact `gld` record on the ground: the head,
/// then the gold flag bit and 12 bits (a 32-bit amount past 4,095).
fn gold_stream(loc: (u16, u16), amount: u32) -> Vec<u8> {
    let mut bits: Vec<(u32, u32)> = vec![
        (0x10 | 0x20_0000, 32),
        (0x65, 10),
        (3, 3),
        (u32::from(loc.0), 16),
        (u32::from(loc.1), 16),
        (u32::from_le_bytes(*b"gld "), 32),
    ];
    if amount < 4096 {
        bits.extend([(0, 1), (amount, 12)]);
    } else {
        bits.extend([(1, 1), (amount, 32)]);
    }
    let (mut out, mut acc, mut n) = (Vec::new(), 0u64, 0);
    for (v, w) in bits {
        acc |= u64::from(v) << n;
        n += w;
        while n >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    }
    if n > 0 {
        out.push(acc as u8);
    }
    out
}

fn put_gold(w: &mut ClientWorld, guid: u32, loc: (u16, u16), amount: u32) {
    let k = UnitKey::new(ITEM, guid);
    let mut u = ClientUnit::new(k);
    u.position = Some(loc);
    u.kind = KindData::Item(ItemData {
        last: Some(ItemRecord {
            id: 0x9C,
            action: 0,
            category: 0,
            owner: None,
            seq: 0,
            stream: gold_stream(loc, amount),
        }),
        ..ItemData::default()
    });
    w.units.insert(k, u);
}

/// A DC6 of four directions with one frame each, `1 + d` pixels wide.
fn dc6_four_directions() -> Vec<u8> {
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&4u32.to_le_bytes());
    d.extend_from_slice(&1u32.to_le_bytes());
    let mut frames = Vec::new();
    for dir in 0..4u32 {
        let w = 1 + dir;
        let mut rows = Vec::new();
        rows.push(w as u8);
        rows.extend((0..w).map(|i| 1 + i as u8));
        rows.push(0x80);
        frames.push([
            [0u32, w, 1, 0, 0, 0, 0, rows.len() as u32]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>(),
            rows,
            vec![0xEE; 3],
        ]);
    }
    let mut at = d.len() + 16;
    for f in &frames {
        d.extend_from_slice(&(at as u32).to_le_bytes());
        at += f.iter().map(Vec::len).sum::<usize>();
    }
    for f in frames {
        for part in f {
            d.extend(part);
        }
    }
    d
}

fn gold_rows() -> ItemArtRows {
    let mut r = ItemArtRows::default();
    r.0.insert(
        *b"gld ",
        ItemArtRow {
            inv_w: 1,
            inv_h: 1,
            inv_file: "invgld".into(),
            flippy_file: "gold".into(),
            beltable: false,
        },
    );
    r
}

#[test]
fn a_gold_pile_draws_the_direction_of_its_amount_class() {
    let mut src = MemorySource::default();
    src.insert("data\\global\\items\\gold.dc6", dc6_four_directions());
    let mut w = ClientWorld::default();
    populate(&mut w, &[]);
    put_gold(&mut w, 7, (102, 100), 50);
    put_gold(&mut w, 8, (100, 102), 700);
    put_gold(&mut w, 9, (98, 98), 6000);
    let mut g = GroundItems::new(Arc::new(src), gold_rows());
    let mut a = assets();
    let mut frame = framed(&w);
    let feed = ModelFeed::<NoFeed>::default();
    let log = g.add_to_frame(&w, &feed, &mut a, &mut frame);
    assert!(log.is_empty(), "{log:?}");
    assert_eq!(frame.items.len(), 3);
    let path = "data/global/items/gold.dc6";
    let dir = |d: u8| FrameSetKey::new(path, FramePart::Dir(d)).unwrap();
    let id = |d: u8| a.id(&dir(d), 0).unwrap();
    let by_guid = |guid: u32| {
        frame
            .items
            .iter()
            .find(|i| i.tag == ItemTag::Unit(guid))
            .unwrap()
            .frame
    };
    // < 100 → direction 0, < 5,000 and ≥ 500 → 2, else 3.
    assert_eq!(by_guid(7), id(0));
    assert_eq!(by_guid(8), id(2));
    assert_eq!(by_guid(9), id(3));
}

#[test]
fn a_compact_gold_record_reads_its_amount() {
    let mut w = ClientWorld::default();
    populate(&mut w, &[]);
    put_gold(&mut w, 7, (1, 1), 4095);
    put_gold(&mut w, 8, (1, 1), 123_456);
    let got: Vec<_> = items::ground_items(&w)
        .iter()
        .map(|i| (i.key.guid, i.gold))
        .collect();
    assert_eq!(got, [(7, Some(4095)), (8, Some(123_456))]);
}

#[test]
fn a_click_on_a_far_item_walks_to_it_then_asks_again() {
    let (mut b, link, mut g) = scene(&[(5, 3, (102, 100), b"cap ")]);
    g.take_clicks(&mut b, &[press(433, 299)]).unwrap();
    let pick = vec![0x16, 4, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(*link.sent.lock().unwrap(), std::slice::from_ref(&pick));
    // The next frame sends the walk to the item's place.
    g.frame(&mut b, false).unwrap();
    let walk = vec![0x01, 102, 0, 100, 0];
    assert_eq!(link.sent.lock().unwrap()[1], walk);
    // While the predicted walk runs nothing is sent; when it has ended the
    // pick-up is asked again.
    g.frame(&mut b, true).unwrap();
    assert_eq!(link.sent.lock().unwrap().len(), 2);
    g.frame(&mut b, false).unwrap();
    assert_eq!(link.sent.lock().unwrap()[2], pick);
    // The item leaving the ground ends the record.
    b.world_mut().units.remove(&UnitKey::new(ITEM, 5));
    g.frame(&mut b, false).unwrap();
    g.frame(&mut b, false).unwrap();
    assert_eq!(link.sent.lock().unwrap().len(), 3);
}

#[test]
fn ground_items_take_their_slot_from_the_draw_order() {
    use crate::rules::draw_order::OrderKey;
    let mut w = ClientWorld::default();
    populate(
        &mut w,
        &[
            (5, 3, (102, 100), b"cap "),
            (6, 5, (101, 100), b"cap "),
            (7, 3, (100, 102), b"cap "),
        ],
    );
    let mut g = GroundItems::new(source(), rows());
    let mut a = assets();
    let feed = ModelFeed::<NoFeed>::default();
    g.add_to_frame(&w, &feed, &mut a, &mut framed(&w));
    // Item 5 flat in the shadow list of cell 40 (pass 5); item 6
    // dropping (mode 5) in the unit list of cell 41 (pass 6); item 7
    // hidden by the sight test (not drawn).
    let flat = OrderKey {
        pass: pass::SHADOWS,
        major: 40,
        minor: 2,
    };
    let dropping = OrderKey {
        pass: pass::WALLS_UNITS,
        major: 41,
        minor: 1,
    };
    let mut frame = WorldFrame {
        slots: Some(BTreeMap::from([
            (UnitKey::new(ITEM, 5), UnitSlot::Drawn(flat)),
            (UnitKey::new(ITEM, 6), UnitSlot::Drawn(dropping)),
            (UnitKey::new(ITEM, 7), UnitSlot::NotDrawn),
        ])),
        ..framed(&w)
    };
    g.add_to_frame(&w, &feed, &mut a, &mut frame);
    let keys: Vec<_> = frame.items.iter().map(|d| (d.tag, d.key)).collect();
    assert_eq!(
        keys,
        vec![
            (ItemTag::Unit(5), DrawKey::new(5, 40, 2, 0).unwrap()),
            (ItemTag::Unit(6), DrawKey::new(6, 41, 1, 0).unwrap()),
        ]
    );
    // An item the order does not list is not drawn either.
    frame.slots = Some(BTreeMap::new());
    frame.items.clear();
    g.add_to_frame(&w, &feed, &mut a, &mut frame);
    assert!(frame.items.is_empty());
}
