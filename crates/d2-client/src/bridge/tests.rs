//! Test vectors of the bridge, without a window or GPU. Handlers registered
//! here are synthetic: they test the mechanism, not any message's meaning.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_proto::client::{Walk, WalkToUnit};
use d2_proto::schema::FieldType;
use d2_proto::transport::{Classified, SplitError};
use d2_proto::{CLIENT_MESSAGES, PROTOCOL_VERSION};

use super::dispatch::{self, Dispatch, Handle, HandlerError, Message, Mismatch, Row};
use super::intent::IntentError;
use super::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use super::mirror::{BridgePlugin, BridgeResource, MirrorIndex, UnitView};
use super::world::{addressed_unit, ClientUnit, ClientWorld, UnitKey};
use super::{Bridge, BridgeError};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Send(SendQueue, Vec<u8>),
    Pump,
}

/// A scripted server: records sends and pumps, replays ticks and chunks.
#[derive(Default)]
struct Script {
    version: u32,
    events: Vec<Event>,
    ticks: VecDeque<bool>,
    deliveries: VecDeque<Vec<Vec<u8>>>,
    /// The game's tick rate the link reports; the base rate by default.
    rate: d2_sim::tick::TickRate,
}

#[derive(Clone)]
struct ScriptedLink(Arc<Mutex<Script>>);

impl ScriptedLink {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Script {
            version: PROTOCOL_VERSION,
            rate: d2_sim::tick::TickRate::BASE,
            ..Script::default()
        })))
    }

    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.0.lock().unwrap()
    }

    /// Queues one pump's result and the chunks delivered after it.
    fn deliver(&self, ticked: bool, chunks: &[&[u8]]) {
        let mut s = self.script();
        s.ticks.push_back(ticked);
        s.deliveries
            .push_back(chunks.iter().map(|c| c.to_vec()).collect());
    }
}

impl ServerLink for ScriptedLink {
    fn protocol_version(&self) -> u32 {
        self.script().version
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.script().rate
    }

    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.script().events.push(Event::Send(queue, msg.to_vec()));
        Ok(Sent::Queued)
    }

    fn pump(&mut self) -> Result<Pumped, LinkError> {
        let mut s = self.script();
        s.events.push(Event::Pump);
        Ok(Pumped {
            ticked: s.ticks.pop_front().unwrap_or(false),
        })
    }

    fn tick_pending(&mut self) -> bool {
        self.script().ticks.front() == Some(&true)
    }

    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.script().deliveries.pop_front().unwrap_or_default()
    }
}

fn bridge() -> (Bridge<ScriptedLink>, ScriptedLink) {
    let link = ScriptedLink::new();
    (Bridge::new(link.clone()).unwrap(), link)
}

fn sent(link: &ScriptedLink) -> Vec<(SendQueue, Vec<u8>)> {
    link.script()
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Send(q, b) => Some((*q, b.clone())),
            Event::Pump => None,
        })
        .collect()
}

/// The bytes of a C→S message built from its `d2-proto` layout: each
/// named field's value little-endian at its offset.
fn from_layout(id: u8, size: usize, values: &[(&str, u32)]) -> Vec<u8> {
    let desc = &CLIENT_MESSAGES[id as usize];
    let mut b = vec![0; size];
    b[0] = id;
    for &(name, v) in values {
        let f = desc.layout.iter().find(|f| f.name == name).unwrap();
        let off = f.offset.unwrap() as usize;
        let n = match f.ty {
            FieldType::U8 => 1,
            FieldType::U16 => 2,
            FieldType::U32 => 4,
            other => panic!("unexpected field type {other:?}"),
        };
        b[off..off + n].copy_from_slice(&v.to_le_bytes()[..n]);
    }
    b
}

/// open question 10 decided as A: the bridge holds what was
/// queued after 0x28's reserved slot until the UI's case comes back, then
/// sends 0x31 and the rest in the base-game order; an unanswered slot is dropped
/// at the next frame.
#[test]
fn dialog_reply_slot_holds_the_send_order() {
    use crate::bridge::msg::ui_npc::DialogCase;
    use crate::bridge::output::Output;
    use crate::bridge::world::{ClientUnit, MonsterClass, UnitKey, MONSTER};
    let quest = |g: u32| {
        let mut m = vec![0x28, 1];
        m.extend(g.to_le_bytes());
        m.resize(103, 0);
        m
    };
    let (mut b, link) = bridge();
    let mut tables = b.inputs.tables.clone();
    tables.monsters = vec![Some(MonsterClass::default()); 200];
    tables.monsters[148] = Some(MonsterClass {
        interact: true,
        ..MonsterClass::default()
    });
    b.set_tables(tables);
    let k = UnitKey::new(MONSTER, 6);
    let mut u = ClientUnit::new(k);
    u.class = 148;
    b.world_mut().units.insert(k, u);
    let chunk = [quest(6), quest(7)].concat();
    link.deliver(false, &[&chunk]);
    b.frame().unwrap();
    let x2f = vec![0x2F, 1, 0, 0, 0, 6, 0, 0, 0];
    let x30 = vec![0x30, 0, 0, 0, 0, 7, 0, 0, 0];
    assert_eq!(sent(&link), vec![(SendQueue::Game, x2f.clone())]);
    let outs = b.take_outputs();
    let Some(Output::NpcDialog(d)) = outs.first() else {
        panic!("{outs:?}")
    };
    assert_eq!(b.npc_dialog_branch(d, DialogCase::B2 { m: 3 }).unwrap(), 2);
    assert_eq!(
        sent(&link),
        vec![
            (SendQueue::Game, x2f.clone()),
            (SendQueue::Game, vec![0x31, 6, 0, 0, 0, 3, 0, 0, 0]),
            (SendQueue::Game, x30.clone()),
        ]
    );
    assert_eq!(b.world().units[&k].mode, 1);
    // Unanswered: the next frame drops the slot and sends what followed.
    link.deliver(false, &[&chunk]);
    b.frame().unwrap();
    link.deliver(false, &[]);
    b.frame().unwrap();
    let s = sent(&link);
    assert_eq!(s[3..], [(SendQueue::Game, x2f), (SendQueue::Game, x30)]);

    // Catch-up ticks precede the UI's chance to answer. Keep its reply
    // slot and later outgoing messages until the display frame finishes.
    let _ = b.take_outputs();
    let before = sent(&link).len();
    link.deliver(true, &[&chunk]);
    link.deliver(true, &[]);
    b.display_frame(true).unwrap();
    assert_eq!(b.world().server_ticks, 2);
    assert_eq!(sent(&link).len(), before + 1);
    let outs = b.take_outputs();
    let Some(Output::NpcDialog(d)) = outs.first() else {
        panic!("{outs:?}")
    };
    assert_eq!(b.npc_dialog_branch(d, DialogCase::B2 { m: 3 }).unwrap(), 2);
    assert_eq!(sent(&link).len(), before + 3);
}

#[test]
fn intent_bytes_match_layouts() {
    let (mut b, link) = bridge();
    b.send(&Walk {
        x: 0x1234,
        y: 0x5678,
    })
    .unwrap();
    b.send(&WalkToUnit {
        type_: 1,
        id: 0xAABB_CCDD,
    })
    .unwrap();
    let walk = vec![0x01, 0x34, 0x12, 0x78, 0x56];
    let to_unit = vec![0x02, 1, 0, 0, 0, 0xDD, 0xCC, 0xBB, 0xAA];
    assert_eq!(walk, from_layout(0x01, 5, &[("x", 0x1234), ("y", 0x5678)]));
    assert_eq!(
        to_unit,
        from_layout(0x02, 9, &[("type", 1), ("id", 0xAABB_CCDD)])
    );
    assert_eq!(
        sent(&link),
        vec![(SendQueue::Game, walk), (SendQueue::Game, to_unit)]
    );
}

#[test]
fn intent_routing_and_refusals() {
    let (mut b, link) = bridge();
    assert_eq!(b.send_bytes(&[0x6B]).unwrap(), Sent::Queued);
    let refused = |r: Result<Sent, BridgeError>| match r {
        Err(BridgeError::Intent(e)) => e,
        other => panic!("expected an intent error, got {other:?}"),
    };
    assert_eq!(
        refused(b.send_bytes(&[0x80; 5])),
        IntentError::NotSendable(Classified::Invalid)
    );
    assert_eq!(
        refused(b.send_bytes(&[0x01, 0, 0])),
        IntentError::NotSendable(Classified::Incomplete)
    );
    assert_eq!(refused(b.send_bytes(&[0xFF; 16])), IntentError::AdminQueue);
    // Chat with a large trailing byte: the size rule is negative.
    let chat = [&[0x15, 0x01, 0x00][..], b"hi\0bob\0", &[0x80]].concat();
    assert_eq!(
        refused(b.send_bytes(&chat)),
        IntentError::NotSendable(Classified::NegativeSize(11 - 128))
    );
    // Only the system message reached the link.
    assert_eq!(sent(&link), vec![(SendQueue::System, vec![0x6B])]);
}

#[test]
fn game_send_limit() {
    // 0x66 (warden response): u16 at +1 plus 3, capped at 0x1FD.
    let mut big = vec![0x66, 0xFD, 0x01];
    big.resize(0x200, 0);
    assert_eq!(
        super::intent::route(&big),
        Err(IntentError::GameTooLarge(0x200))
    );
    let mut ok = vec![0x66, 0xFC, 0x01];
    ok.resize(0x1FF, 0);
    assert_eq!(super::intent::route(&ok), Ok(SendQueue::Game));
}

#[test]
fn split_and_unowned() {
    let (mut b, _) = bridge();
    // 0x79 (6 bytes) and 0x8B (6 bytes): out-of-scope ids; the shared
    // no-op handler runs, nothing is unowned.
    let r = b
        .receive_chunk(&[0x79, 1, 2, 3, 4, 5, 0x8B, 1, 2, 3, 4, 5])
        .unwrap();
    assert_eq!((r.messages, r.unowned, r.handled), (2, 0, 2));
    let log = b.log();
    assert!(log.unowned.is_empty());
    assert!(log.discarded.is_empty());
    assert_eq!(b.world(), &ClientWorld::default());
}

#[test]
fn unknown_id_ends_split() {
    let (mut b, _) = bridge();
    let r = b.receive_chunk(&[0x61, 0x07, 0x80, 0x61, 0x07]).unwrap();
    assert_eq!((r.messages, r.discarded_bytes), (1, 3));
    assert_eq!(
        b.log().discarded,
        vec![super::receive::Discarded {
            first: 0x80,
            bytes: 3
        }]
    );
}

#[test]
fn fatal_chunks_are_refused_whole() {
    let (mut b, _) = bridge();
    match b.receive_chunk(&[0x61, 0x07, 0x5F, 1]) {
        Err(BridgeError::Split(SplitError::Truncated { at: 2, size: 5 })) => {}
        other => panic!("{other:?}"),
    }
    let mut big = vec![0x16, 0x05, 0x02];
    big.resize(13, 0);
    match b.receive_chunk(&big) {
        Err(BridgeError::Split(SplitError::TooLarge { at: 0, size: 0x205 })) => {}
        other => panic!("{other:?}"),
    }
    // Nothing of either chunk was dispatched.
    assert!(b.log().unowned.is_empty());
}

#[test]
fn addressed_units() {
    let mut m6d = vec![0x6D, 7, 0, 0, 0];
    m6d.resize(10, 0);
    assert_eq!(
        addressed_unit(&m6d),
        Some(UnitKey {
            unit_type: 1,
            guid: 7
        })
    );
    let mut m0e = vec![0x0E, 2, 9, 0, 0, 0];
    m0e.resize(12, 0);
    assert_eq!(
        addressed_unit(&m0e),
        Some(UnitKey {
            unit_type: 2,
            guid: 9
        })
    );
    assert_eq!(addressed_unit(&[0x6E]), None);
    assert_eq!(addressed_unit(&[0x61, 0x07]), None);
}

// Synthetic handlers: add / remove the addressed unit.
fn add_unit(world: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let key = msg.unit.ok_or(HandlerError::Invalid("no unit"))?;
    world.units.insert(key, ClientUnit::new(key));
    Ok(())
}

fn remove_unit(world: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let key = msg.unit.ok_or(HandlerError::Invalid("no unit"))?;
    world.units.remove(&key);
    Ok(())
}

fn refuse(_: &mut ClientWorld, _: &Message<'_>) -> Result<(), HandlerError> {
    Err(HandlerError::Invalid("synthetic refusal"))
}

fn test_dispatch() -> Dispatch {
    let mut d = Dispatch::empty();
    d.set(0x6D, "test", add_unit);
    d.set(0x0E, "test", add_unit);
    d.set(0x0F, "test", remove_unit);
    d.set(0x61, "test", refuse);
    d
}

fn msg(id: u8, size: usize, unit_type: u8, guid: u32) -> Vec<u8> {
    let mut b = vec![id];
    if (0x67..=0x6D).contains(&id) {
        b.extend(guid.to_le_bytes());
    } else {
        b.push(unit_type);
        b.extend(guid.to_le_bytes());
    }
    b.resize(size, 0);
    b
}

#[test]
fn world_updates_from_messages() {
    let link = ScriptedLink::new();
    let mut b = Bridge::with_dispatch(link.clone(), test_dispatch()).unwrap();
    let chunk = [
        msg(0x6D, 10, 1, 7),
        msg(0x0E, 12, 2, 9),
        vec![0x61, 0x07],
        msg(0x0F, 16, 2, 9),
        msg(0x0E, 12, 4, 3),
    ]
    .concat();
    let r = b.receive_chunk(&chunk).unwrap();
    assert_eq!((r.messages, r.handled, r.rejected), (5, 4, 1));
    let keys: Vec<_> = b.world().units.keys().copied().collect();
    let key = |unit_type, guid| UnitKey { unit_type, guid };
    assert_eq!(keys, vec![key(1, 7), key(4, 3)]);
    assert_eq!(b.log().rejected.len(), 1);
    assert_eq!(b.log().rejected[0].id, 0x61);
}

#[test]
fn frame_order_and_counters() {
    let (mut b, link) = bridge();
    link.deliver(true, &[&[0x61, 0x07]]);
    link.deliver(false, &[]);
    let r1 = b.frame().unwrap();
    assert!(r1.ticked);
    assert_eq!((r1.chunks, r1.messages), (1, 1));
    b.send(&Walk { x: 1, y: 2 }).unwrap();
    let r2 = b.frame().unwrap();
    assert!(!r2.ticked);
    assert_eq!((b.world().frames, b.world().server_ticks), (2, 1));
    // The act video's 0x48 and the intent of frame 1
    // reach the link before pump 2.
    assert_eq!(
        link.script().events,
        vec![
            Event::Pump,
            Event::Send(SendQueue::Game, vec![0x48]),
            Event::Send(SendQueue::Game, vec![0x01, 1, 0, 2, 0]),
            Event::Pump,
        ]
    );
}

#[test]
fn version_mismatch_is_refused() {
    let link = ScriptedLink::new();
    link.script().version = PROTOCOL_VERSION + 1;
    match Bridge::new(link) {
        Err(BridgeError::Version { client, server }) => {
            assert_eq!((client, server), (PROTOCOL_VERSION, PROTOCOL_VERSION + 1));
        }
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("accepted a link of another version"),
    }
}

#[test]
fn dispatch_table_matches_spec() {
    let rows = dispatch::parse(dispatch::TSV).unwrap();
    assert_eq!(rows.len(), dispatch::IDS);
    assert_eq!(dispatch::check(&rows, dispatch::HANDLERS), vec![]);
    assert!(Dispatch::from_spec().is_ok());
}

/// A changed row is reported, and only that row.
#[test]
fn dispatch_check_catches_perturbations() {
    let rows = dispatch::parse(dispatch::TSV).unwrap();
    let mut owned = rows.clone();
    // A handler-less id given an owner has no handler to match.
    let mut no_handler = dispatch::HANDLERS.to_vec();
    no_handler.retain(|h| h.id != 0x79);
    assert_eq!(
        dispatch::check(&owned, &no_handler),
        vec![Mismatch::NoHandler { id: 0x79 }]
    );
    owned[0x75].owner = Some("specs/client/x.md".into());
    assert_eq!(
        dispatch::check(&owned, dispatch::HANDLERS),
        vec![Mismatch::Owner {
            id: 0x75,
            tsv: "specs/client/x.md".into(),
            code: "specs/client/msg-units.md".into()
        }]
    );
    let mut moved = rows.clone();
    moved[0x1A].owner = Some("specs/client/x.md".into());
    assert_eq!(
        dispatch::check(&moved, dispatch::HANDLERS),
        vec![Mismatch::Owner {
            id: 0x1A,
            tsv: "specs/client/x.md".into(),
            code: "specs/client/msg-stats-items.md".into(),
        }]
    );
    let mut renamed = rows.clone();
    renamed[0x5F].name = "Renamed".into();
    assert_eq!(
        dispatch::check(&renamed, dispatch::HANDLERS),
        vec![Mismatch::Name {
            id: 0x5F,
            tsv: "Renamed".into(),
            proto: rows[0x5F].name.clone(),
        }]
    );
    let tbd: Vec<Row> = rows
        .iter()
        .map(|r| Row {
            owner: None,
            ..r.clone()
        })
        .collect();
    let handler = [dispatch::Handler {
        id: 0x5F,
        owner: "specs/client/y.md",
        handle: Handle::General(add_unit),
    }];
    assert_eq!(
        dispatch::check(&tbd, &handler),
        vec![Mismatch::Unowned { id: 0x5F }]
    );
    let mut both: Vec<Row> = tbd.clone();
    both[0x5F].owner = Some("specs/client/z.md".into());
    assert_eq!(
        dispatch::check(&both, &handler),
        vec![Mismatch::Owner {
            id: 0x5F,
            tsv: "specs/client/z.md".into(),
            code: "specs/client/y.md".into(),
        }]
    );
    // The handler kind follows the receive table:
    // a general handler for a unit-handler id, and the reverse.
    let mut kinds: Vec<dispatch::Handler> = dispatch::HANDLERS.to_vec();
    let i = kinds.iter().position(|h| h.id == 0x6D).unwrap();
    kinds[i].handle = Handle::General(add_unit);
    let j = kinds.iter().position(|h| h.id == 0x15).unwrap();
    kinds[j].handle = Handle::Unit(super::msg::units::queued);
    assert_eq!(
        dispatch::check(&rows, &kinds),
        vec![Mismatch::Kind { id: 0x15 }, Mismatch::Kind { id: 0x6D }]
    );
    // Strict parse: a missing row, a wrong id.
    let short: String = dispatch::TSV
        .lines()
        .take(10)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(dispatch::parse(&short), Err(dispatch::TsvError::Rows(9)));
    let swapped = dispatch::TSV.replacen("0x01\t", "0x02\t", 1);
    assert_eq!(
        dispatch::parse(&swapped),
        Err(dispatch::TsvError::Id {
            line: 3,
            expected: 1
        })
    );
}

#[test]
fn bevy_mirror_follows_the_model() {
    let link = ScriptedLink::new();
    link.deliver(
        false,
        &[&[msg(0x6D, 10, 1, 7), msg(0x0E, 12, 2, 9)].concat()],
    );
    link.deliver(true, &[&msg(0x0F, 16, 2, 9)]);
    link.deliver(false, &[]);
    let bridge = Bridge::with_dispatch(Box::new(link.clone()) as _, test_dispatch()).unwrap();
    let mut app = App::new();
    app.add_plugins(BridgePlugin)
        .insert_resource(BridgeResource(bridge));

    let views = |app: &mut App| -> Vec<UnitKey> {
        let mut q = app.world_mut().query::<&UnitView>();
        let mut keys: Vec<_> = q.iter(app.world()).map(|v| v.key).collect();
        keys.sort();
        keys
    };
    let key = |unit_type, guid| UnitKey { unit_type, guid };

    app.update();
    assert_eq!(views(&mut app), vec![key(1, 7), key(2, 9)]);
    let index: Vec<_> = app
        .world()
        .resource::<MirrorIndex>()
        .0
        .keys()
        .copied()
        .collect();
    assert_eq!(index, vec![key(1, 7), key(2, 9)]);

    app.update();
    assert_eq!(views(&mut app), vec![key(1, 7)]);
    app.update();
    assert_eq!(views(&mut app), vec![key(1, 7)]);
    let world = app.world().resource::<BridgeResource>().0.world();
    assert_eq!((world.frames, world.server_ticks), (3, 1));
}

// Gap tests.

thread_local! {
    /// Messages seen by [`record`], in dispatch order (per test thread).
    static SEEN: std::cell::RefCell<Vec<Vec<u8>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Synthetic handler: records the message bytes as the handler got them.
fn record(_: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    SEEN.with(|s| s.borrow_mut().push(msg.bytes.to_vec()));
    Ok(())
}

fn seen() -> Vec<Vec<u8>> {
    SEEN.with(|s| s.borrow().clone())
}

fn recording_dispatch() -> Dispatch {
    let mut d = Dispatch::empty();
    for id in [0x0E, 0x6D, 0x61] {
        d.set(id, "test", record);
    }
    d
}

/// Every line of `src` outside `//` comments.
fn code_lines(src: &str) -> impl Iterator<Item = &str> {
    src.lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .filter(|l| !l.trim().is_empty())
}

#[test]
fn bridge_modules_except_mirror_have_no_bevy_type() {
    let sources = [
        ("mod.rs", include_str!("mod.rs")),
        ("dispatch.rs", include_str!("dispatch.rs")),
        ("intent.rs", include_str!("intent.rs")),
        ("link.rs", include_str!("link.rs")),
        ("local.rs", include_str!("local.rs")),
        ("receive.rs", include_str!("receive.rs")),
        ("world.rs", include_str!("world.rs")),
        ("bits.rs", include_str!("bits.rs")),
        ("check.rs", include_str!("check.rs")),
        ("update.rs", include_str!("update.rs")),
        ("msg/mod.rs", include_str!("msg/mod.rs")),
        ("msg/session.rs", include_str!("msg/session.rs")),
        ("msg/units.rs", include_str!("msg/units.rs")),
        ("msg/stats_items.rs", include_str!("msg/stats_items.rs")),
        ("msg/skills.rs", include_str!("msg/skills.rs")),
        ("msg/ui.rs", include_str!("msg/ui.rs")),
        ("msg/sound.rs", include_str!("msg/sound.rs")),
        ("msg/lighting.rs", include_str!("msg/lighting.rs")),
        ("output.rs", include_str!("output.rs")),
        ("skills.rs", include_str!("skills.rs")),
    ];
    for (name, src) in sources {
        for line in code_lines(src) {
            assert!(
                !line.to_ascii_lowercase().contains("bevy"),
                "{name}: {line}"
            );
        }
    }
    // The mirror is the one that does use Bevy (the scan can fail).
    assert!(code_lines(include_str!("mirror.rs")).any(|l| l.contains("bevy::")));
    // And the plain part runs without a window, GPU or `App`.
    let (mut b, _) = bridge();
    b.send(&Walk { x: 1, y: 1 }).unwrap();
    b.frame().unwrap();
}

#[test]
fn model_changes_only_through_dispatched_messages() {
    let (mut b, link) = bridge();
    // Intents are sent, never predicted: no model change.
    b.send(&Walk { x: 5, y: 6 }).unwrap();
    b.send(&WalkToUnit { type_: 1, id: 7 }).unwrap();
    assert_eq!(b.world(), &ClientWorld::default());
    // Unowned messages (even unit messages) change nothing but the
    // bridge's own counters.
    link.deliver(
        true,
        &[&[msg(0x6D, 10, 1, 7), msg(0x0E, 12, 2, 9)].concat()],
    );
    b.frame().unwrap();
    assert_eq!(
        b.world(),
        &ClientWorld {
            frames: 1,
            server_ticks: 1,
            ..ClientWorld::default()
        }
    );
}

#[test]
fn bytes_cross_the_boundary_unchanged() {
    // Out: typed and raw messages reach the link byte for byte.
    let (mut b, link) = bridge();
    let warden = [0x66, 0x02, 0x00, 0xAB, 0xCD];
    let walk = [0x01, 0x34, 0x12, 0x78, 0x56];
    b.send_bytes(&warden).unwrap();
    b.send_bytes(&walk).unwrap();
    b.send_bytes(&[0x6B]).unwrap();
    assert_eq!(
        sent(&link),
        vec![
            (SendQueue::Game, warden.to_vec()),
            (SendQueue::Game, walk.to_vec()),
            (SendQueue::System, vec![0x6B]),
        ]
    );
    // In: each handler sees exactly its message's bytes of the chunk.
    let mut b = Bridge::with_dispatch(ScriptedLink::new(), recording_dispatch()).unwrap();
    let (m1, m2) = (msg(0x0E, 12, 2, 0xA1B2_C3D4), msg(0x6D, 10, 1, 0x0102_0304));
    let chunk = [m1.clone(), vec![0x61, 0x07], m2.clone()].concat();
    b.receive_chunk(&chunk).unwrap();
    assert_eq!(seen(), vec![m1, vec![0x61, 0x07], m2]);
}

#[test]
fn buffer_and_node_chunks_split_alike() {
    let messages = [msg(0x6D, 10, 1, 7), vec![0x61, 0x07], msg(0x0E, 12, 2, 9)];
    let run = |chunks: &[&[u8]]| {
        let link = ScriptedLink::new();
        link.deliver(false, chunks);
        let mut b = Bridge::with_dispatch(link, test_dispatch()).unwrap();
        let report = b.frame().unwrap();
        (report.messages, b.world().clone(), b.log().rejected.len())
    };
    let buffer = messages.concat();
    let whole = run(&[&buffer]);
    let nodes = run(&[&messages[0], &messages[1], &messages[2]]);
    assert_eq!(whole, nodes);
    assert_eq!((whole.0, whole.1.units.len(), whole.2), (3, 2, 1));
}

#[test]
fn chunks_are_handled_in_delivery_order() {
    let link = ScriptedLink::new();
    let (a, c) = (msg(0x6D, 10, 1, 1), msg(0x0E, 12, 2, 2));
    // Delivered "system list" first, then game list; neither is reordered
    // even though the ids would sort the other way.
    link.deliver(false, &[&[0x61, 0x07], &a, &c]);
    let mut b = Bridge::with_dispatch(link, recording_dispatch()).unwrap();
    assert_eq!(b.frame().unwrap().chunks, 3);
    assert_eq!(seen(), vec![vec![0x61, 0x07], a, c]);
}

#[test]
fn units_iterate_in_type_then_guid_order() {
    let mut b = Bridge::with_dispatch(ScriptedLink::new(), test_dispatch()).unwrap();
    let chunk = [
        msg(0x0E, 12, 4, 1),
        msg(0x6D, 10, 1, 9),
        msg(0x0E, 12, 2, 0),
        msg(0x0E, 12, 1, 0xFFFF_FFFF),
        msg(0x6D, 10, 1, 3),
    ]
    .concat();
    b.receive_chunk(&chunk).unwrap();
    let key = |unit_type, guid| UnitKey { unit_type, guid };
    let keys: Vec<_> = b.world().units.keys().copied().collect();
    assert_eq!(
        keys,
        vec![
            key(1, 3),
            key(1, 9),
            key(1, 0xFFFF_FFFF),
            key(2, 0),
            key(4, 1)
        ]
    );
    assert!(b.world().units.iter().all(|(k, u)| u.key == *k));
}

#[test]
fn owned_rows_are_exactly_the_registered_handlers() {
    let rows = dispatch::parse(dispatch::TSV).unwrap();
    let owned: Vec<(u8, &str)> = rows
        .iter()
        .filter_map(|r| r.owner.as_deref().map(|o| (r.id, o)))
        .collect();
    let registered: Vec<(u8, &str)> = dispatch::HANDLERS.iter().map(|h| (h.id, h.owner)).collect();
    assert_eq!(owned, registered);
    // Every id is owned (the out-of-scope ids have owners and the no-op
    // handler): model 18, msg-units 53,
    // msg-stats-items 26, msg-skills 9, msg-ui 24,
    // 0x2C, 0x53 and 0x89, 49 (steps 6–7).
    assert_eq!(owned.len(), 0xB6);
    let per = |spec: &str| owned.iter().filter(|(_, o)| *o == spec).count();
    assert_eq!(
        [
            super::msg::MODEL,
            super::msg::UNITS,
            super::msg::STATS_ITEMS,
            super::msg::SKILLS,
            super::msg::UI,
            super::msg::TRIGGERS,
            super::msg::LIGHTING,
            super::msg::BRIDGE,
        ]
        .map(per),
        [18, 53, 26, 9, 24, 1, 2, 49]
    );
    for (_, o) in &owned {
        assert!(
            [
                super::msg::BRIDGE,
                super::msg::MODEL,
                super::msg::UNITS,
                super::msg::STATS_ITEMS,
                super::msg::SKILLS,
                super::msg::UI,
                super::msg::TRIGGERS,
                super::msg::LIGHTING
            ]
            .contains(o),
            "{o}"
        );
    }
    let d = Dispatch::from_spec().unwrap();
    for r in &rows {
        assert_eq!(d.get(r.id).is_some(), r.owner.is_some(), "{:#04X}", r.id);
    }
}

#[test]
fn views_are_overwritten_from_the_model() {
    let link = ScriptedLink::new();
    link.deliver(false, &[&msg(0x6D, 10, 1, 7)]);
    let bridge = Bridge::with_dispatch(Box::new(link) as _, test_dispatch()).unwrap();
    let mut app = App::new();
    app.add_plugins(BridgePlugin)
        .insert_resource(BridgeResource(bridge));
    app.update();
    let key = UnitKey {
        unit_type: 1,
        guid: 7,
    };
    let bogus = UnitKey {
        unit_type: 3,
        guid: 99,
    };
    let mut q = app.world_mut().query::<&mut UnitView>();
    for mut v in q.iter_mut(app.world_mut()) {
        v.key = bogus;
    }
    app.update();
    let mut q = app.world_mut().query::<&UnitView>();
    let views: Vec<_> = q.iter(app.world()).map(|v| v.key).collect();
    assert_eq!(views, vec![key]);
    // Never written back: the model is unchanged.
    let world = app.world().resource::<BridgeResource>().0.world();
    assert_eq!(world.units.keys().copied().collect::<Vec<_>>(), vec![key]);
}

#[test]
fn one_pump_per_frame_whether_or_not_it_ticked() {
    let (mut b, link) = bridge();
    for ticked in [true, true, false, true, false, false] {
        link.deliver(ticked, &[]);
    }
    for _ in 0..6 {
        b.frame().unwrap();
    }
    let pumps = link
        .script()
        .events
        .iter()
        .filter(|e| **e == Event::Pump)
        .count();
    assert_eq!(pumps, 6);
    assert_eq!((b.world().frames, b.world().server_ticks), (6, 3));
}

/// A link whose pump fails.
struct BrokenLink;

impl ServerLink for BrokenLink {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }
    fn send(&mut self, _: SendQueue, _: &[u8]) -> Result<Sent, LinkError> {
        Ok(Sent::Queued)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        Err(LinkError::Server("server down".into()))
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        Vec::new()
    }
}

#[test]
fn errors_stop_the_frame_records_do_not() {
    let mut broken = Bridge::new(BrokenLink).unwrap();
    assert!(matches!(broken.frame(), Err(BridgeError::Link(_))));

    // A refused chunk is an error; the chunk after it is not processed.
    let link = ScriptedLink::new();
    link.deliver(false, &[&[0x61, 0x07, 0x5F, 1], &[0x61, 0x07]]);
    let mut b = Bridge::with_dispatch(link.clone(), recording_dispatch()).unwrap();
    assert!(matches!(
        b.frame(),
        Err(BridgeError::Split(SplitError::Truncated { .. }))
    ));
    assert!(seen().is_empty());

    // Unowned ids, discarded bytes and handler rejections are recorded,
    // and the frame succeeds.
    link.deliver(false, &[&[0x5F, 1, 2, 3, 4, 0x61, 0x07, 0x80, 0x00]]);
    let mut b = Bridge::with_dispatch(link, test_dispatch()).unwrap();
    let r = b.frame().unwrap();
    assert_eq!((r.unowned, r.rejected, r.discarded_bytes), (1, 1, 2));
}

#[test]
fn size_zero_ids_end_the_split_and_one_byte_unit_ids_address_nothing() {
    for id in [0x83u8, 0x84, 0x88, 0x80] {
        let (mut b, _) = bridge();
        let r = b.receive_chunk(&[0x61, 0x07, id, 0x61, 0x07]).unwrap();
        assert_eq!((r.messages, r.discarded_bytes), (1, 3), "id {id:#04X}");
        assert_eq!(
            b.log().discarded,
            vec![super::receive::Discarded {
                first: id,
                bytes: 3
            }]
        );
    }
    for id in 0x6Eu8..=0x72 {
        assert!(
            d2_proto::transport::server_message(id)
                .unwrap()
                .client_unit_handler
                .is_some(),
            "{id:#04X} has a unit handler"
        );
        assert_eq!(addressed_unit(&[id]), None);
    }
}

/// The bytes of a hex string.
fn hx(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

#[test]
fn update_pass_runs_on_ticked_in_game_frames_and_answers_are_sent() {
    let (mut b, link) = bridge();
    let mut join = hx("59 01 00 00 00 01 77 65 72 77 65 72");
    join.resize(0x16, 0);
    join.extend(hx("41 12 c4 11"));
    let obj = hx("51 02 0d 00 00 00 25 00 14 12 c0 11 02 00");
    let state = hx("0e 02 0d 00 00 00 03 00 02 00 00 00");
    let chunk = [
        hx("02"),
        hx("03 00 c4 88 38 10 01 00 61 d1 e0 9f"),
        join,
        hx("0b 00 01 00 00 00"),
        obj,
        state.clone(),
        hx("04"),
    ]
    .concat();
    // Frame 1 does not tick: the 0x0E waits in the object's queue.
    link.deliver(false, &[&chunk]);
    let r = b.frame().unwrap();
    assert_eq!((r.handled, r.queued, r.drained, r.answered), (6, 1, 0, 1));
    let key = UnitKey::new(2, 13);
    assert!(b.world().in_game);
    assert_eq!(b.world().units[&key].queue.len(), 1);
    // The 0x6B answer to 0x02 went through the send path (system queue).
    assert_eq!(sent(&link), vec![(SendQueue::System, vec![0x6B])]);
    assert!(b.world().outgoing.is_empty());
    // Frame 2 ticks: the update pass applies it.
    link.deliver(true, &[]);
    let r = b.frame().unwrap();
    assert_eq!(r.drained, 1);
    let u = &b.world().units[&key];
    assert!(u.queue.is_empty());
    assert_eq!(
        u.last_mode_request.map(|m| (m.code, m.record)),
        Some((3, [0, 2, 0, 0, 0, 0, 0]))
    );
    // Out of game (0x05): a ticked frame drains nothing.
    link.deliver(true, &[&[hx("05"), state].concat()]);
    let r = b.frame().unwrap();
    assert_eq!((r.queued, r.drained), (1, 0));
    assert_eq!(b.world().units[&key].queue.len(), 1);
    assert!(b.log().rejected.is_empty());
}

#[test]
fn outputs_are_handed_over_once_after_the_frame() {
    use super::output::Output;
    let (mut b, link) = bridge();
    // Object 13 (class 37), then 0x2C on it (event 18) and 0x77 0x10.
    let chunk = [
        hx("51 02 0d 00 00 00 25 00 14 12 c0 11 02 00"),
        hx("2c 02 0d 00 00 00 12 00"),
        hx("77 10"),
    ]
    .concat();
    link.deliver(false, &[&chunk]);
    let r = b.frame().unwrap();
    assert_eq!(r.outputs, 2);
    let want = [
        Output::ServerSound {
            unit: UnitKey::new(2, 13),
            class: 37,
            at: Some((0x1214, 0x11C0)),
            event: 18,
        },
        Output::TradeAction {
            code: 0x10,
            dead_or_absent: true,
        },
    ];
    assert_eq!(b.outputs(), want);
    assert_eq!(b.take_outputs(), want);
    assert!(b.take_outputs().is_empty(), "handed over once");
    // A frame with no outputs: the list stays empty.
    link.deliver(false, &[]);
    assert_eq!(b.frame().unwrap().outputs, 0);
    assert!(b.outputs().is_empty());
}

#[test]
fn the_bevy_frame_hands_the_outputs_over() {
    use super::mirror::FrameOutputs;
    use super::output::Output;
    let link = ScriptedLink::new();
    link.deliver(false, &[&hx("77 15")]);
    link.deliver(false, &[]);
    let bridge = Bridge::new(Box::new(link) as _).unwrap();
    let mut app = App::new();
    app.add_plugins(BridgePlugin)
        .insert_resource(BridgeResource(bridge));
    app.update();
    assert_eq!(
        app.world().resource::<FrameOutputs>().0,
        [Output::TradeAction {
            code: 0x15,
            dead_or_absent: true
        }]
    );
    assert!(app
        .world()
        .resource::<BridgeResource>()
        .0
        .outputs()
        .is_empty());
    // The next frame replaces them (nothing new).
    app.update();
    assert!(app.world().resource::<FrameOutputs>().0.is_empty());
}

/// The UI layer chooses 0x28's dialog branch case from the NPC text list
/// of the 0x27 before it, and the dispatcher hands it to the bridge in
/// the same frame: C→S 0x31 goes out right after the 0x2F, before the
/// later 0x30 (Test vectors: A seq 37351, 37353; the
/// recorded client sends `2f 01000000 06000000`, then `31 06000000
/// 25000000`). A holding case (B0, B1) sends no 0x31; an m the table does
/// not give hands no case back, so the 0x30 behind the slot waits for
/// the next frame and no 0x31 is ever sent.
#[test]
fn ui_answers_npc_dialog_with_0x31_in_order() {
    use crate::bridge::world::{MonsterClass, MONSTER, PLAYER};
    use crate::ui::layout::Screen;
    use crate::ui::original::{OriginalUi, UiConfig};
    use crate::world_view::present::deliver;
    fn hexs(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    let npc_text = |count: u8, kind0: u8| {
        let mut m = hexs("27 01 06000000");
        m.extend([count, 0, kind0, 0, 0x25, 0]);
        m.resize(40, 0);
        m
    };
    let quest = |g: u32| {
        let mut m = vec![0x28, 1];
        m.extend(g.to_le_bytes());
        m.extend([0, 1]);
        m.resize(103, 0);
        m
    };
    // One run: a bridge with the local player (cursor item or not) and
    // the NPC (1, 6) class 148; the chunk 0x27, 0x28 (1, 6), 0x28 (1, 7)
    // absent; one frame and its delivery. Returns the sends of the frame
    // and of the next (empty) frame.
    let run = |text: Vec<u8>, b0: u8, cursor: bool| {
        let (mut b, link) = bridge();
        let mut tables = b.inputs.tables.clone();
        tables.monsters = vec![Some(MonsterClass::default()); 200];
        tables.monsters[148] = Some(MonsterClass {
            interact: true,
            npc: true,
            ..MonsterClass::default()
        });
        b.set_tables(tables);
        let p = UnitKey::new(PLAYER, 1);
        let mut pu = ClientUnit::new(p);
        pu.mode = 1;
        if cursor {
            let data = crate::bridge::world::PlayerData {
                cursor_item: Some(9),
                ..Default::default()
            };
            pu.kind = crate::bridge::world::KindData::Player(data);
        }
        b.world_mut().units.insert(p, pu);
        b.world_mut().local_player = Some(p);
        let k = UnitKey::new(MONSTER, 6);
        let mut u = ClientUnit::new(k);
        u.class = 148;
        b.world_mut().units.insert(k, u);
        let mut ui = OriginalUi::new(
            UiConfig {
                screen: Screen::R800,
                expansion_installed: true,
            },
            None,
        )
        .unwrap();
        ui.set_ui_7c0c68(b0);
        let chunk = [text, quest(6), quest(7)].concat();
        link.deliver(false, &[&chunk]);
        b.frame().unwrap();
        let outs = b.take_outputs();
        deliver(&mut b, &outs, Some(&mut ui)).unwrap();
        let first = sent(&link);
        link.deliver(false, &[]);
        b.frame().unwrap();
        let all = sent(&link);
        let mode = b.world().units[&k].mode;
        let next = all[first.len()..].to_vec();
        (first, next, mode)
    };
    let g = |s: &str| (SendQueue::Game, hexs(s));
    let x2f = g("2f 01000000 06000000");
    let x30 = g("30 00000000 07000000");
    // B2: m = 0x25 from the list; 0x31 between 0x2F and 0x30, same frame.
    let (first, next, mode) = run(npc_text(1, 0), 0, false);
    assert_eq!(first, [x2f.clone(), g("31 06000000 25000000"), x30.clone()]);
    assert!(next.is_empty());
    assert_eq!(mode, 1);
    // B0 (≠ 0): no 0x31; the slot is cleared at once.
    let (first, next, mode) = run(npc_text(1, 0), 1, false);
    assert_eq!(first, [x2f.clone(), x30.clone()]);
    assert!(next.is_empty());
    assert_eq!(mode, 0);
    // B1 (cursor item): no 0x31; U takes mode 1 (an `interact` class).
    let (first, _, mode) = run(npc_text(1, 0), 0, true);
    assert_eq!(first, [x2f.clone(), x30.clone()]);
    assert_eq!(mode, 1);
    // M is the smallest kind-0 string id; the
    // helper's second entry is (kind 0, id 0), so m = 0.
    let (first, next, mode) = run(npc_text(2, 0), 0, false);
    assert_eq!(first, [x2f, g("31 06000000 00000000"), x30]);
    assert!(next.is_empty());
    assert_eq!(mode, 1);
}

/// The skill fallback is the last step of a tick frame while `in_game`;
/// a frame without a tick does not run it.
#[test]
fn skill_fallback_runs_only_in_a_tick_frame() {
    use super::skills::{SkillEntry, SkillList, NATIVE};
    use super::world::{SkillRow, PLAYER};
    let p = UnitKey::new(PLAYER, 1);
    let native = |skill, base| SkillEntry {
        skill,
        base,
        owner: NATIVE,
        ..SkillEntry::default()
    };
    let (mut b, link) = bridge();
    b.inputs.tables.skills = vec![SkillRow::default(); 37];
    let mut u = ClientUnit::new(p);
    u.skills = Some(SkillList {
        entries: vec![native(0, 1), native(36, 0)],
        left: Some(1),
        right: Some(0),
        ..SkillList::default()
    });
    b.world.units.insert(p, u);
    b.world.local_player = Some(p);
    b.world.in_game = true;
    let left = |b: &Bridge<ScriptedLink>| {
        let l = b.world.units[&p].skills.as_ref().unwrap();
        l.left_entry().map(|e| e.skill)
    };
    link.deliver(false, &[]);
    b.frame().unwrap();
    assert_eq!(left(&b), Some(36), "no tick: no fallback");
    link.deliver(true, &[]);
    let r = b.frame().unwrap();
    assert_eq!(r.rejected, 0);
    assert_eq!(left(&b), Some(0), "tick frame: the level-0 hand fell back");
}

/// B024: a left attack on a monster out of reach walks first and leaves
/// a pending (6, type, GUID); on arrival the preview runs it as the action
/// itself: the local attack mode request and C→S 0x06, not the interact
/// sender (which sends nothing for a monster).
#[test]
fn a_pending_attack_sends_the_skill_on_arrival() {
    use super::skills::{SkillEntry, SkillList, NATIVE};
    use super::world::{SkillRow, MONSTER, PLAYER};
    use crate::controls::click::Pending;
    let p = UnitKey::new(PLAYER, 1);
    let m = UnitKey::new(MONSTER, 77);
    let (mut b, link) = bridge();
    b.inputs.tables.skills = vec![SkillRow {
        ingame: true,
        anim: 7,
        ..SkillRow::default()
    }];
    let mut u = ClientUnit::new(p);
    u.skills = Some(SkillList {
        entries: vec![SkillEntry {
            skill: 0,
            base: 1,
            owner: NATIVE,
            ..SkillEntry::default()
        }],
        left: Some(0),
        right: Some(0),
        ..SkillList::default()
    });
    b.world.units.insert(p, u);
    b.world.units.insert(m, ClientUnit::new(m));
    b.world.local_player = Some(p);
    b.world.in_game = true;
    b.pending_action(Pending { code: 6, target: m }).unwrap();
    let mut want = vec![0x06];
    want.extend(1u32.to_le_bytes());
    want.extend(77u32.to_le_bytes());
    assert_eq!(sent(&link), [(SendQueue::Game, want)]);
    assert_eq!(b.world.units[&p].mode, 7, "the attack mode started");
}

/// Save and Exit: C→S 0x69 on the system
/// queue and `exit_requested`; the client stays in game until the
/// server's 0x05. `play::leave_game` (the window close) sends it
/// for a client still in game and runs frames until the 0x05.
#[test]
fn save_and_exit_sends_0x69_and_waits_for_the_server() {
    let (mut b, link) = bridge();
    b.world.in_game = true;
    b.save_and_exit().unwrap();
    assert_eq!(sent(&link), [(SendQueue::System, vec![0x69])]);
    assert!(b.world.exit_requested && b.world.in_game);
    link.deliver(false, &[&[0x05], &[0x06]]);
    b.frame().unwrap();
    assert!(!b.world.in_game && b.world.unloaded && b.world.exit_requested);

    // The window close: one 0x69, frames until the 0x05.
    let (mut b, link) = bridge();
    b.world.in_game = true;
    link.deliver(false, &[]);
    link.deliver(false, &[&[0x05], &[0x06]]);
    assert!(crate::app::play::leave_game(&mut b).unwrap());
    assert_eq!(sent(&link), [(SendQueue::System, vec![0x69])]);
    assert_eq!(
        link.script()
            .events
            .iter()
            .filter(|e| **e == Event::Pump)
            .count(),
        2
    );
    // A client already out of the game sends nothing.
    let (mut b, link) = bridge();
    assert!(crate::app::play::leave_game(&mut b).unwrap());
    assert!(sent(&link).is_empty());
}

/// B035: `App::run` moves the world out of the app, so the leave must run
/// inside the app on the frame carrying `AppExit`. The exit system sends
/// C→S 0x69 once and reports the result through the shared report.
#[test]
fn the_exit_system_leaves_while_the_world_exists() {
    use crate::app::play::{leave_on_exit, ExitReport};
    let link = ScriptedLink::new();
    link.deliver(false, &[]);
    link.deliver(false, &[&[0x05], &[0x06]]);
    let report = ExitReport::default();
    let mut app = App::new();
    app.insert_resource(report.clone())
        .insert_resource(BridgeResource(
            Bridge::new(Box::new(link.clone()) as super::mirror::DynLink).unwrap(),
        ))
        .add_systems(Last, leave_on_exit);
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .world_mut()
        .in_game = true;
    // No AppExit yet: nothing happens.
    app.update();
    assert!(sent(&link).is_empty());
    assert!(report.0.lock().unwrap().is_none());
    app.world_mut().write_message(AppExit::Success);
    app.update();
    assert_eq!(sent(&link), [(SendQueue::System, vec![0x69])]);
    let out = report.0.lock().unwrap().clone().expect("the leave ran");
    assert_eq!(out.left, Ok(true));
    assert!(!out.to_menu, "a window close ends the program");
    // The message lives two updates: the leave does not run twice.
    app.update();
    assert_eq!(sent(&link).len(), 1);
}

/// The paused pass: with UI
/// state 9 or 11 open and the local player in a room, a frame runs no
/// pump and no receive, only the skill fallback, once; unpaused, the
/// next frame pumps and receives what waited. Without a placed local
/// player, or once Save and Exit was asked,
/// the frame runs as usual.
#[test]
fn a_paused_frame_runs_no_pump_and_only_the_skill_fallback() {
    use super::skills::{SkillEntry, SkillList, NATIVE};
    use super::world::{SkillRow, PLAYER};
    let p = UnitKey::new(PLAYER, 1);
    let native = |skill, base| SkillEntry {
        skill,
        base,
        owner: NATIVE,
        ..SkillEntry::default()
    };
    let (mut b, link) = bridge();
    b.inputs.tables.skills = vec![SkillRow::default(); 37];
    let mut u = ClientUnit::new(p);
    u.skills = Some(SkillList {
        entries: vec![native(0, 1), native(36, 0)],
        left: Some(1),
        right: Some(0),
        ..SkillList::default()
    });
    b.world.units.insert(p, u);
    b.world.local_player = Some(p);
    b.world.in_game = true;
    let left = |b: &Bridge<ScriptedLink>| {
        let l = b.world.units[&p].skills.as_ref().unwrap();
        l.left_entry().map(|e| e.skill)
    };
    let pumps = |link: &ScriptedLink| {
        link.script()
            .events
            .iter()
            .filter(|e| **e == Event::Pump)
            .count()
    };
    b.set_paused(true);
    // No room yet: an ordinary frame.
    link.deliver(false, &[]);
    let r = b.frame().unwrap();
    assert!(!r.paused);
    assert_eq!(pumps(&link), 1);
    // In a room: paused, no pump, the fallback ran.
    b.world.units.get_mut(&p).unwrap().position = Some((10, 10));
    link.deliver(true, &[&[0x05]]);
    let frames = b.world.frames;
    let r = b.frame().unwrap();
    assert!(r.paused && !r.ticked && r.chunks == 0, "{r:?}");
    assert_eq!(pumps(&link), 1, "no pump");
    assert_eq!(b.world.frames, frames + 1);
    assert_eq!(left(&b), Some(0), "the fallback ran once");
    assert!(b.world.in_game, "the waiting 0x05 is not received");
    // Unpaused: the next frame pumps and receives.
    b.set_paused(false);
    let r = b.frame().unwrap();
    assert!(!r.paused && r.chunks == 1);
    assert_eq!(pumps(&link), 2);
    assert!(!b.world.in_game);
}

#[test]
fn own_c2s_ids_can_be_dropped() {
    let (mut b, link) = bridge();
    b.set_drop_own(vec![0x5F]);
    b.world_mut().outgoing.push(vec![0x5F, 9, 0x13, 0x84, 0x10]);
    b.world_mut().outgoing.push(vec![0x6B]);
    assert_eq!(b.send_outgoing().unwrap(), 2);
    assert_eq!(sent(&link), vec![(SendQueue::System, vec![0x6B])]);
    assert_eq!(b.take_dropped(), vec![vec![0x5F, 9, 0x13, 0x84, 0x10]]);
    assert!(b.take_dropped().is_empty());
}

// Local player: the walk-end restart leaves a cast's animation running.
#[test]
fn the_walk_end_restart_keeps_a_cast_running() {
    let (mut b, _link) = bridge();
    let key = UnitKey::new(super::world::PLAYER, 1);
    let mut u = ClientUnit::new(key);
    u.mode = 10;
    u.frame = 256;
    u.frame_count = 3584;
    u.speed = Some(256);
    b.world.units.insert(key, u);
    b.world.local_player = Some(key);
    b.restart_local_neutral();
    let u = &b.world.units[&key];
    assert_eq!((u.mode, u.frame, u.frame_count), (10, 256, 3584));
    // A neutral player still starts its neutral over.
    b.world.units.get_mut(&key).unwrap().mode = 1;
    b.restart_local_neutral();
    assert_eq!(b.world.units[&key].frame, 0);
}

#[test]
fn the_client_pings_at_its_first_frame_then_every_125_server_ticks() {
    let (mut b, link) = bridge();
    b.set_ping(true);
    for _ in 0..250 {
        link.deliver(true, &[]);
        b.frame().unwrap();
    }
    let at = pings(&link);
    assert_eq!(at.len(), 2, "server ticks 0 and 125: {at:?}");
    let ev = link.script().events.clone();
    // each ping goes before its frame's pump, so the server reads it before that tick
    assert!(at.iter().all(|&i| matches!(ev[i + 1], Event::Pump)));
    let pumps_before = |i: usize| ev[..i].iter().filter(|e| matches!(e, Event::Pump)).count();
    assert_eq!(
        at.iter().map(|&i| pumps_before(i)).collect::<Vec<_>>(),
        [0, 125]
    );
    assert!(matches!(&ev[at[0]], Event::Send(_, m) if m[..] == super::PING_MESSAGE[..]));
    // off (the default, and a tool replaying recorded pings): no 0x6D
    let (mut b, link) = bridge();
    link.deliver(true, &[]);
    b.frame().unwrap();
    assert!(pings(&link).is_empty());
}

/// Indexes of the pings (0x6D) among the link's recorded events.
fn pings(link: &ScriptedLink) -> Vec<usize> {
    link.script()
        .events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Event::Send(_, m) if m[0] == 0x6D => Some(i),
            _ => None,
        })
        .collect()
}

#[test]
fn the_ping_keeps_its_five_seconds_at_a_faster_tick_rate() {
    let link = ScriptedLink::new();
    let fast = d2_sim::tick::TickRate::new(60).unwrap();
    link.script().rate = fast;
    let mut b = Bridge::new(link.clone()).unwrap();
    assert_eq!(b.world.tick_rate, fast, "the client learns the link's rate");
    b.set_ping(true);
    for _ in 0..600 {
        link.deliver(true, &[]);
        b.frame().unwrap();
    }
    let ev = link.script().events.clone();
    let pumps_before = |i: usize| ev[..i].iter().filter(|e| matches!(e, Event::Pump)).count();
    // 5 000 ms at 60 Hz is 300 ticks, as 125 ticks are at the base rate
    assert_eq!(
        pings(&link)
            .iter()
            .map(|&i| pumps_before(i))
            .collect::<Vec<_>>(),
        [0, 300]
    );
}

#[test]
fn a_threaded_link_keeps_walk_distance_per_second_at_25_and_60_hz() {
    use super::predict::{Predict, PredictLink, Speeds};
    use super::world::PLAYER;
    use crate::app::server_thread::ThreadLink;
    use d2_sim::tick::TickRate;

    let distance = |rate: TickRate, run: bool| {
        let scripted = ScriptedLink::new();
        scripted.script().rate = rate;
        let server = scripted.clone();
        let threaded = ThreadLink::spawn(move || Ok::<_, String>(server)).unwrap();
        let link = Box::new(PredictLink::new(threaded));
        let walks = link.tap();
        let mut b = Bridge::new(link).unwrap();
        assert_eq!(b.world.tick_rate, rate);
        let key = UnitKey::new(PLAYER, 7);
        let mut unit = ClientUnit::new(key);
        unit.position = Some((100, 100));
        unit.server_point = (100, 100);
        b.world.units.insert(key, unit);
        b.world.local_player = Some(key);
        let mut prediction = Predict::new();
        let speeds = Speeds { walk: 7, run: 11 };
        prediction.frame(&b.world, [], false, speeds);
        let start = prediction.position().unwrap();
        b.send_bytes(&[if run { 3 } else { 1 }, 200, 0, 100, 0])
            .unwrap();
        for _ in 0..rate.hz() {
            // Display passes without a server tick must consume no distance.
            let before = prediction.position();
            scripted.deliver(false, &[]);
            let report = b.frame().unwrap();
            prediction.frame(&b.world, walks.take(), report.ticked, speeds);
            assert_eq!(prediction.position(), before);
            scripted.deliver(true, &[]);
            let report = b.frame().unwrap();
            prediction.frame(&b.world, walks.take(), report.ticked, speeds);
        }
        let end = prediction.position().unwrap();
        assert_eq!(end.1, start.1);
        end.0 - start.0
    };
    for run in [false, true] {
        let base = distance(TickRate::BASE, run);
        assert_eq!(base, 25 * Speeds { walk: 7, run: 11 }.step(run) as u32);
        assert_eq!(distance(TickRate::new(60).unwrap(), run), base);
    }
}
