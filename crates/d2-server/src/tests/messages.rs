//! Vectors for the `d2-server` message path: the size rules and classifier, the
//! server queues and drain, the duplicate filter, dispatch gates and handler
//! parsing, and the per-client buffers and delivery. Part of the `d2-server` test
//! suite (see `mod.rs`); `size_vectors` is shared with `adapters.rs`, which runs it
//! on the real `d2-proto` tables, and the fakes come from `fakes.rs`.

use std::collections::BTreeMap;

use super::fakes::*;
use crate::buffers::{ClientBuffers, Inbox, QueueError};
use crate::dispatch::{self, bind_hotkey, gate, kind, select_skill, Gate, Kind};
use crate::seams::*;
use crate::transport::*;

/// Size-rule vectors for client and server messages. Generic over
/// the seam so the same vectors run on `d2-proto` once it is wired.
pub(super) fn size_vectors(s: &impl MessageSizes) {
    let chat = |last: u8| {
        let mut m = vec![0x15, 0x01, 0x00, b'h', b'i', 0, b'b', b'o', b'b', 0, last];
        m.resize(20, 0);
        m
    };
    assert_eq!(s.client_size(&chat(0x00)), Ok(11));
    assert_eq!(s.client_size(&chat(0x05)), Ok(16));
    assert_eq!(s.client_size(&chat(0xFF)), Ok(10));
    assert_eq!(s.client_size(&[0x66, 0x05, 0x00]), Ok(8));
    assert_eq!(s.client_size(&[0x66, 0xFD, 0x01]), Ok(512));
    assert_eq!(s.client_size(&[0x66, 0xFE, 0x01]), Ok(3));
    assert_eq!(s.client_size(&[0x6C, 0x10, 0, 0, 0, 0]), Ok(23));
    assert_eq!(s.client_size(&[0x6C, 0x10]), Err(SizeError::Incomplete));
    for id in [0x2C, 0x4A, 0x64] {
        assert_eq!(s.client_size(&[id; 9]), Err(SizeError::Invalid), "{id:#x}");
    }
    let mut m94 = vec![0x94, 0x03];
    m94.resize(9, 0);
    assert_eq!(s.server_size(&m94), Ok(15));
    assert_eq!(s.server_size(&[0xAF, 0x00]), Ok(2));
    assert_eq!(s.server_size(&[0xAF, 0x05]), Ok(6));
    assert_eq!(s.server_size(&[0xAE, 0x10, 0x00]), Ok(19));
    let mut m26 = vec![0x26; 10];
    m26.extend_from_slice(b"a\0bc\0");
    assert_eq!(s.server_size(&m26), Ok(15));
    assert_eq!(
        s.server_size(&[0x16, 0x20, 0x00]),
        Err(SizeError::Incomplete)
    );
    let mut m16 = vec![0x16, 0x20, 0x00];
    m16.resize(13, 0);
    assert_eq!(s.server_size(&m16), Ok(32));
    assert_eq!(s.server_size(&[0xB5]), Err(SizeError::Invalid));
}

// The TSV-driven size rules satisfy the shared size vectors.
#[test]
fn size_rules() {
    size_vectors(&TsvSizes::new());
}

// The classifier's results for complete, partial, empty and oversized messages.
#[test]
fn classifier_vectors() {
    let s = TsvSizes::new();
    let mut ff = vec![0xFF];
    ff.resize(16, 0);
    assert_eq!(classify(&s, &[0x80, 0, 0], true), Classified::Invalid);
    assert_eq!(classify(&s, &ff, true), Classified::Queued(Queue::Admin));
    assert_eq!(classify(&s, &ff, false), Classified::Invalid);
    assert_eq!(classify(&s, &ff[..15], true), Classified::Incomplete);
    assert_eq!(
        classify(&s, &[0x6B], true),
        Classified::Queued(Queue::System)
    );
    assert_eq!(
        classify(&s, &[1, 2, 3, 4, 5], true),
        Classified::Queued(Queue::Game)
    );
    assert_eq!(classify(&s, &[1, 2, 3, 4], true), Classified::Incomplete);
    assert_eq!(classify(&s, &[], true), Classified::Incomplete);
    // Size 0 ids are never queued.
    assert_eq!(classify(&s, &[0x2C; 9], true), Classified::Incomplete);
    // The 0x15 chat with 05 needs 16 bytes: 11 given → 3.
    let chat = [0x15, 0x01, 0x00, b'h', b'i', 0, b'b', b'o', b'b', 0, 5];
    assert_eq!(classify(&s, &chat, true), Classified::Incomplete);
    // 0x71 is invalid on the original wire, 0x72 is the request system
    // message, and the rest of 0x73..=0xFE is invalid (edge case 2).
    for id in 0x71..=0xFE {
        if id == 0x72 {
            assert_eq!(
                classify(&s, &[id; 13], true),
                Classified::Queued(Queue::System)
            );
        } else {
            assert_eq!(classify(&s, &[id; 20], true), Classified::Invalid);
        }
    }
}

// Queues keep the whole received buffer, and drain in the order sent.
#[test]
fn queues_keep_the_whole_buffer_and_drain_in_queue_order() {
    let s = TsvSizes::new();
    let mut q = ServerQueues::new();
    // A 6-byte 0x01: rule size 5 <= 6, queued with all 6 bytes.
    assert_eq!(
        q.send(&s, 0, &[1, 0, 0, 0, 0, 9]),
        Ok(Classified::Queued(Queue::Game))
    );
    assert_eq!(
        q.send(&s, 0, &[0x6B]),
        Ok(Classified::Queued(Queue::System))
    );
    let mut ff = vec![0xFF];
    ff.resize(16, 0);
    assert_eq!(q.send(&s, 0, &ff), Ok(Classified::Queued(Queue::Admin)));
    assert_eq!(
        q.send(&s, 0, &[0x03, 1, 0, 2, 0]),
        Ok(Classified::Queued(Queue::Game))
    );
    assert_eq!(
        q.send(&s, 0, &[0x69]),
        Ok(Classified::Queued(Queue::System))
    );
    // Dropped silently.
    assert_eq!(q.send(&s, 0, &[0x80]), Ok(Classified::Invalid));
    assert_eq!(q.len(Queue::Game), 2);
    assert_eq!(
        q.send(&s, 0, &[0x66; 0x205]),
        Err(SendError::TooLarge(0x205))
    );
    let d = q.drain();
    let order: Vec<(Queue, u8, usize)> = d.iter().map(|m| (m.queue, m.msg[0], m.size)).collect();
    assert_eq!(
        order,
        [
            (Queue::System, 0x6B, 1),
            (Queue::System, 0x69, 1),
            (Queue::Game, 0x01, 6),
            (Queue::Game, 0x03, 5),
            (Queue::Admin, 0xFF, 16),
        ]
    );
    assert!(q.is_empty());
}

// A long message is copied short on drain; its size stays.
#[test]
fn drain_truncates_long_messages() {
    // 0x66 of 512 bytes is copied short; its size stays.
    let s = TsvSizes::new();
    let mut q = ServerQueues::new();
    let mut m = vec![0x66, 0xFD, 0x01];
    m.resize(512, 0xAB);
    assert_eq!(q.send(&s, 7, &m), Ok(Classified::Queued(Queue::Game)));
    let d = q.drain();
    assert_eq!(d[0].client, 7);
    assert_eq!(d[0].msg.len(), 0x1FC);
    assert_eq!(d[0].size, 512);
    assert_eq!(d[0].msg[..], m[..0x1FC]);
}

// Duplicate filter: which repeated messages pass, depending on content and time.
#[test]
fn duplicate_filter_vectors() {
    let mut f = DuplicateFilter::default();
    let walk = [0x03, 0x10, 0x00, 0x20, 0x00];
    assert_eq!(f.pass(&walk, 1000), Ok(true));
    assert_eq!(f.pass(&walk, 1120), Ok(false));
    // The filtered send did not refresh the time: 200 ms after the first.
    assert_eq!(f.pass(&walk, 1200), Ok(true));
    let skill = [0x0C, 0x10, 0x00, 0x20, 0x00];
    assert_eq!(f.pass(&skill, 2000), Ok(true));
    assert_eq!(f.pass(&skill, 2060), Ok(true));
    assert_eq!(f.pass(&skill, 2070), Ok(false));
    // 0x3A is never filtered.
    assert_eq!(f.pass(&[0x3A, 1, 0], 3000), Ok(true));
    assert_eq!(f.pass(&[0x3A, 1, 0], 3001), Ok(true));
    // A different message passes at once.
    assert_eq!(f.pass(&[0x3B, 1, 0], 3002), Ok(true));
    assert_eq!(
        f.pass(&[0u8; 0x200], 0),
        Err(SendError::GameTooLarge(0x200))
    );
}

// The duplicate filter compares only the new message's size.
#[test]
fn duplicate_filter_compares_over_the_new_size() {
    // The store keeps bytes past a shorter message.
    let mut f = DuplicateFilter::default();
    assert_eq!(f.pass(&[0x01, 5, 0, 6, 0], 0), Ok(true));
    assert_eq!(f.pass(&[0x01, 5, 0], 10), Ok(false));
    assert_eq!(f.pass(&[0x01, 5, 1], 20), Ok(true));
}

// The duplicate filter's time window per message id.
#[test]
fn window_table() {
    for id in 0..=0xFFu8 {
        let want = match id {
            0x05..=0x0A | 0x0C..=0x11 => Some(50),
            0x3A => None,
            _ => Some(200),
        };
        assert_eq!(duplicate_window(id), want);
    }
    assert_eq!(duplicate_window(0x0B), Some(200));
}

// ---- dispatch ----

fn run(game: &mut FakeGame, msg: &[u8]) -> ResultCode {
    let s = TsvSizes::new();
    let mut out = ClientBuffers::new();
    out.add_client(0);
    let PlayerLookup::Player(p) = game.player(0) else {
        panic!("no player")
    };
    dispatch::dispatch(game, &s, &mut out, 0, p, msg, msg.len())
}

fn gate_of(mode: u32, uninterruptable: bool) -> PlayerGate {
    PlayerGate {
        mode,
        uninterruptable,
    }
}

// Dispatch rejects ids that are out of range or have no handler.
#[test]
fn dispatch_bad_ids() {
    let mut g = FakeGame::with_player(0, ALIVE);
    assert_eq!(run(&mut g, &[0x00]), ResultCode::Malformed);
    assert_eq!(run(&mut g, &[0x67]), ResultCode::Malformed);
    assert_eq!(run(&mut g, &[0x2B]), ResultCode::Malformed);
    assert!(g.handled.is_empty());
}

// Dispatch gates by player mode: which ids run for a dead or uninterruptable player.
#[test]
fn dispatch_gates() {
    let mut dead = FakeGame::with_player(0, gate_of(0x11, false));
    assert_eq!(run(&mut dead, &[0x41]), ResultCode::Done);
    assert_eq!(dead.handled.len(), 1, "0x41 runs for a dead player");
    let mut alive = FakeGame::with_player(0, ALIVE);
    assert_eq!(run(&mut alive, &[0x41]), ResultCode::Done);
    assert!(alive.handled.is_empty(), "0x41 gate closed for mode 1");

    let walk = [0x01, 100, 0, 100, 0];
    for (gate, runs) in [
        (gate_of(1, true), false),
        (gate_of(0, false), false),
        (gate_of(0x11, false), false),
        (gate_of(1, false), true),
    ] {
        let mut g = FakeGame::with_player(0, gate);
        assert_eq!(run(&mut g, &walk), ResultCode::Done);
        assert_eq!(g.handled.len(), usize::from(runs), "{gate:?}");
    }
    // No gate: 0x3C and 0x14 run with the player dead.
    let mut g = FakeGame::with_player(0, gate_of(0x11, false));
    assert_eq!(
        run(&mut g, &[0x3C, 5, 0, 0, 0x80, 0xFF, 0xFF, 0xFF, 0xFF]),
        ResultCode::Done
    );
    assert_eq!(
        run(&mut g, &[0x14, 0, 0, b'h', b'i', 0, 0]),
        ResultCode::Done
    );
    assert_eq!(g.handled.len(), 2);
}

// Handler size check, and the stub handlers' fixed results.
#[test]
fn handler_size_and_stubs() {
    let mut g = FakeGame::with_player(0, ALIVE);
    assert_eq!(
        run(&mut g, &[0x01, 100, 0, 100, 0, 0]),
        ResultCode::Malformed
    );
    assert_eq!(run(&mut g, &[0x45; 9]), ResultCode::Malformed);
    assert_eq!(run(&mut g, &[0x45]), ResultCode::Malformed);
    assert_eq!(run(&mut g, &[0x42]), ResultCode::Done);
    assert_eq!(run(&mut g, &[0x42, 1, 2]), ResultCode::Done);
    assert_eq!(run(&mut g, &[0x66, 1, 0, 0]), ResultCode::Done);
    // Stubs never reach the handler.
    assert!(g.handled.is_empty());
    // A stub is still gated: 0x2C with the player dead → 0, not 3.
    let mut dead = FakeGame::with_player(0, gate_of(0x11, false));
    assert_eq!(run(&mut dead, &[0x2C]), ResultCode::Done);
    // 0x14 accepts 4..=275 bytes.
    let mut chat = vec![0x14, 0, 0, b'x', 0];
    assert_eq!(run(&mut g, &chat), ResultCode::Done);
    chat.resize(276, 0);
    assert_eq!(run(&mut g, &chat), ResultCode::Malformed);
    assert_eq!(run(&mut g, &[0x14, 0, 0]), ResultCode::Malformed);
}

// Chat string length and terminator checks.
#[test]
fn chat_string_checks() {
    let mut g = FakeGame::with_player(0, ALIVE);
    // strlen 0 → 0 with no effect.
    assert_eq!(run(&mut g, &[0x14, 0, 0, 0, 0]), ResultCode::Done);
    // strlen 255 → accepted; 256 → 2 (inside 275 bytes).
    let mut m = vec![0x14, 0, 0];
    m.extend(std::iter::repeat_n(b'a', 255));
    m.extend([0, 0]);
    assert_eq!(run(&mut g, &m), ResultCode::Done);
    m.insert(3, b'a');
    assert_eq!(run(&mut g, &m), ResultCode::Invalid);
    // No NUL at all → 2.
    assert_eq!(run(&mut g, &[0x14, 0, 0, b'a']), ResultCode::Invalid);
    // 0x15: strlen < 256 and strlen + 4 < size reach the handler; else it is
    // rejected with 2.
    let handled = g.handled.len();
    assert_eq!(
        run(&mut g, &[0x15, 1, 0, b'h', b'i', 0, 0]),
        ResultCode::Done
    );
    assert_eq!(g.handled.len(), handled + 1);
    // strlen + 4 = size: the msg's NUL is the last byte → rejected.
    assert_eq!(
        run(&mut g, &[0x15, 1, 0, b'h', b'i', 0]),
        ResultCode::Invalid
    );
    // No NUL.
    assert_eq!(run(&mut g, &[0x15, 1, 0, b'h', b'i']), ResultCode::Invalid);
    // strlen 255 accepted, 256 rejected.
    let mut m = vec![0x15, 1, 0];
    m.extend(std::iter::repeat_n(b'a', 255));
    m.extend([0, 0]);
    assert_eq!(run(&mut g, &m), ResultCode::Done);
    m.insert(3, b'a');
    assert_eq!(run(&mut g, &m), ResultCode::Invalid);
    assert_eq!(g.handled.len(), handled + 2);
}

// Point messages: range check against the player, and resync when out of range.
#[test]
fn point_range_and_resync() {
    let mut g = FakeGame::with_player(0, ALIVE);
    g.frame = 30;
    let at = |x: u16, y: u16| {
        let (x, y) = (x.to_le_bytes(), y.to_le_bytes());
        [0x01, x[0], x[1], y[0], y[1]]
    };
    assert_eq!(run(&mut g, &at(150, 50)), ResultCode::Done);
    assert_eq!(g.point_state(0).unwrap().last_accept, 30);
    // 25 frames since the accept: refused, no resync.
    g.frame = 55;
    assert_eq!(run(&mut g, &at(151, 100)), ResultCode::Refused);
    assert!(g.resyncs.is_empty());
    // 26 frames: refused and resynced; the accept frame is unchanged.
    g.frame = 56;
    assert_eq!(run(&mut g, &at(100, 49)), ResultCode::Refused);
    assert_eq!(g.resyncs, [0]);
    assert_eq!(g.point_state(0).unwrap().last_accept, 30);
    assert_eq!(g.handled.len(), 1);
    // No player data → 2.
    g.players.get_mut(&0).unwrap().as_mut().unwrap().point = None;
    assert_eq!(run(&mut g, &at(100, 100)), ResultCode::Invalid);
}

// A point resync queues its S→C message.
#[test]
fn point_resync_queues_its_message() {
    let s = TsvSizes::new();
    let mut g = FakeGame::with_player(0, ALIVE);
    g.frame = 100;
    let mut out = ClientBuffers::new();
    out.add_client(0);
    let code = dispatch::dispatch(&mut g, &s, &mut out, 0, ALIVE, &[0x03, 0, 1, 0, 0], 5);
    assert_eq!(code, ResultCode::Refused);
    assert_eq!(out.buffers(0).unwrap()[0], [0x15; 11]);
}

// Messages that target a unit: missing, wrong act or valid targets.
#[test]
fn unit_targets() {
    let mut g = FakeGame::with_player(0, ALIVE);
    let p = Pos { x: 100, y: 100 };
    g.units.insert(
        (1, 7),
        UnitTarget::At {
            player: p,
            target: Pos { x: 150, y: 50 },
        },
    );
    g.units.insert(
        (1, 8),
        UnitTarget::At {
            player: p,
            target: Pos { x: 49, y: 100 },
        },
    );
    g.units.insert((4, 9), UnitTarget::OwnedItem);
    g.units.insert((1, 10), UnitTarget::OtherAct);
    let unit = |t: u32, id: u32| {
        let mut m = vec![0x02];
        m.extend(t.to_le_bytes());
        m.extend(id.to_le_bytes());
        m
    };
    assert_eq!(run(&mut g, &unit(1, 7)), ResultCode::Done);
    assert_eq!(run(&mut g, &unit(1, 8)), ResultCode::Refused);
    assert_eq!(run(&mut g, &unit(4, 9)), ResultCode::Done);
    assert_eq!(run(&mut g, &unit(1, 10)), ResultCode::Invalid);
    assert_eq!(run(&mut g, &unit(1, 11)), ResultCode::Refused);
    assert_eq!(run(&mut g, &unit(6, 7)), ResultCode::Invalid);
    assert_eq!(g.handled.len(), 2);
}

// Decoding of the skill and hotkey fields.
#[test]
fn skill_field_decoders() {
    let s = select_skill(&[0x3C, 0x05, 0x00, 0x00, 0x80, 0xFF, 0xFF, 0xFF, 0xFF]).unwrap();
    assert_eq!((s.skill, s.left, s.item), (5, true, u32::MAX));
    let b = bind_hotkey(&[0x51, 0x06, 0x80, 0x03, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]).unwrap();
    assert_eq!((b.skill, b.left, b.slot, b.item), (6, true, 3, u32::MAX));
}

// Dispatch entry: drops for clients not in game, and the per-client record.
#[test]
fn entry_drops_and_records() {
    let s = TsvSizes::new();
    let mut g = FakeGame::with_player(0, ALIVE);
    g.players.insert(1, None);
    let mut records = BTreeMap::new();
    let mut out = ClientBuffers::new();
    let walk = [0x01, 100, 0, 100, 0];
    let mut entry = |g: &mut FakeGame, records: &mut BTreeMap<_, _>, c| {
        dispatch::process_game_message(g, &s, records, &mut out, c, &walk, 5, 1234)
    };
    // Not in a game: dropped before the record is needed.
    assert_eq!(
        entry(&mut g, &mut records, 9),
        Ok(dispatch::Outcome::NotInGame)
    );
    // In a game without a client record: an error (asserted).
    assert_eq!(
        entry(&mut g, &mut records, 0),
        Err(dispatch::DispatchError::NoClientRecord(0))
    );
    records.insert(0, dispatch::ClientRecord::default());
    records.insert(1, dispatch::ClientRecord::default());
    assert_eq!(
        entry(&mut g, &mut records, 1),
        Ok(dispatch::Outcome::NoPlayer)
    );
    assert_eq!(records[&1].last_message_ms, 1234);
    assert_eq!(
        entry(&mut g, &mut records, 0),
        Ok(dispatch::Outcome::Dispatched(ResultCode::Done))
    );
    assert_eq!(records[&0].last_message_ms, 1234);
}

/// Differences between the dispatcher's kind/gate tables and the TSV
/// columns (M05).
fn table_mismatches(rows: &[BTreeMap<String, String>]) -> Vec<String> {
    let mut bad = Vec::new();
    for r in rows {
        let id = hex(&r["id"]) as u8;
        if id >= dispatch::GAME_IDS {
            continue;
        }
        let k = match kind(id) {
            Kind::None => "none",
            Kind::Stub0 => "stub0",
            Kind::Stub3 => "stub3",
            Kind::Handler => "handler",
        };
        if k != r["kind"] {
            bad.push(format!("{id:#04x} kind {k} != {}", r["kind"]));
        }
        // The gate of a null handler is never reached ("-" in the TSV).
        let g = match (kind(id), gate(id)) {
            (Kind::None, _) => "-",
            (_, Gate::None) => "none",
            (_, Gate::Dead) => "dead",
            (_, Gate::Alive) => "alive",
        };
        if g != r["gate"] {
            bad.push(format!("{id:#04x} gate {g} != {}", r["gate"]));
        }
        let point = r["layout"].starts_with("x:u16@1 y:u16@3") && r["handler_size"] == "==5";
        let unit = r["layout"].starts_with("type:u32@1 id:u32@5")
            && r["handler_size"] == "==9"
            && (0x02..=0x11).contains(&id);
        if dispatch::is_point(id) && !point || dispatch::is_unit(id) && !unit {
            bad.push(format!("{id:#04x} parser layout {}", r["layout"]));
        }
    }
    bad
}

#[test]
fn tables_match_tsv() {
    assert_eq!(table_mismatches(&rows(CLIENT_TSV)), Vec::<String>::new());
}

#[test]
fn tables_match_tsv_catches_perturbations() {
    // M08: change one cell and the check reports exactly that cell.
    let base = rows(CLIENT_TSV);
    for (id, col, val, want) in [
        (0x41, "gate", "alive", "0x41 gate dead != alive"),
        (0x2E, "kind", "stub3", "0x2e kind stub0 != stub3"),
        (0x2B, "kind", "handler", "0x2b kind none != handler"),
        (0x3C, "gate", "alive", "0x3c gate none != alive"),
        (
            0x05,
            "layout",
            "type:u32@1 id:u32@5",
            "0x05 parser layout type:u32@1 id:u32@5",
        ),
    ] {
        let mut r = base.clone();
        *r[id].get_mut(col).unwrap() = val.to_string();
        assert_eq!(table_mismatches(&r), [want.to_string()]);
    }
}

// Handler and transport sizes agree for every `==N` row.
#[test]
fn handler_sizes_match_transport() {
    // Transport and handler sizes agree for every `==N` row,
    // so the dispatcher's check can use the transport rule.
    let s = TsvSizes::new();
    for r in rows(CLIENT_TSV) {
        if let Some(n) = r["handler_size"].strip_prefix("==") {
            let id = hex(&r["id"]) as u8;
            assert_eq!(s.client_size(&[id; 32]), Ok(n.parse().unwrap()), "{id:#x}");
        }
    }
}

// ---- server → client ----

// Outgoing messages are packed into 0x200-byte buffers per client.
#[test]
fn buffer_packing() {
    let mut b = ClientBuffers::new();
    b.add_client(0);
    b.queue(0, &[0x9C; 300]).unwrap();
    b.queue(0, &[0x9C; 250]).unwrap();
    let sizes: Vec<usize> = b.buffers(0).unwrap().iter().map(Vec::len).collect();
    assert_eq!(sizes, [300, 250]);

    let mut b = ClientBuffers::new();
    b.add_client(0);
    for n in [200, 200, 112, 1] {
        b.queue(0, &vec![0x9C; n]).unwrap();
    }
    let sizes: Vec<usize> = b.buffers(0).unwrap().iter().map(Vec::len).collect();
    assert_eq!(sizes, [512, 1]);
    // Unknown client: nothing; too large: loud.
    assert_eq!(b.queue(5, &[1]), Ok(()));
    assert!(b.buffers(5).is_none());
    assert_eq!(b.queue(0, &[0; 0x201]), Err(QueueError::TooLarge(0x201)));
}

// Delivery splits a received buffer into messages and routes them.
#[test]
fn delivery_splits_and_routes() {
    let s = TsvSizes::new();
    let mut inbox = Inbox::default();
    // 0x0C (9 bytes), 0xB4 (5), 0x01 (8), 0xAF 00 (2).
    let mut buf = vec![0x0C; 9];
    buf.extend([0xB4; 5]);
    buf.extend([0x01; 8]);
    buf.extend([0xAF, 0x00]);
    assert_eq!(inbox.deliver(&s, &buf), Ok(0));
    assert_eq!(inbox.game.len(), 2);
    assert_eq!(inbox.system.len(), 2);
    let ids: Vec<u8> = inbox.receive().iter().map(|m| m[0]).collect();
    assert_eq!(ids, [0xB4, 0xAF, 0x0C, 0x01]);
}

// A size-0 id ends a split; the rest of the buffer is lost.
#[test]
fn delivery_split_ends_at_a_size_zero_id() {
    // 0x83 (size 0) ends the split; the rest is lost.
    let s = TsvSizes::new();
    let mut inbox = Inbox::default();
    let mut buf = vec![0x0C; 9];
    buf.extend([0x83; 6]);
    buf.extend([0x01; 8]);
    assert_eq!(inbox.deliver(&s, &buf), Ok(14));
    assert_eq!(inbox.receive(), [vec![0x0C; 9]]);
    // 0x80: receive table expects 4 but the size table says 0 (edge 8).
    assert_eq!(inbox.deliver(&s, &[0x80, 0, 0, 0]), Ok(4));
}

// Delivery asserts on sizes that cannot occur.
#[test]
fn delivery_asserts() {
    let mut inbox = Inbox::default();
    assert_eq!(inbox.push(&[0xB5]), Err(QueueError::BadId(0xB5)));
    assert_eq!(inbox.push(&[0xB7; 13]), Err(QueueError::BadId(0xB7)));
    assert_eq!(inbox.push(&[]), Err(QueueError::BadSize(0)));
    assert_eq!(inbox.push(&[0; 0x205]), Err(QueueError::BadSize(0x205)));
}

/// Regression: S→C 0xB6 ViewReady is in the generated table and reaches the
/// system list through the buffer and local delivery (it was rejected as an
/// id past the old hand-written 0xB5 ceiling).
#[test]
fn view_ready_through_local_delivery() {
    let msg = d2_proto::server::ViewReady {
        seq: 1,
        width: 800,
        height: 600,
    }
    .encode();
    let mut buffers = ClientBuffers::new();
    buffers.add_client(0);
    buffers.queue(0, &msg).unwrap();
    let buf = buffers.pop(0).unwrap();
    let mut inbox = Inbox::default();
    assert_eq!(inbox.deliver(&crate::adapters::ProtoSizes, &buf), Ok(0));
    assert_eq!(inbox.receive(), [msg.to_vec()]);
}

/// Regression (CI-only `prop_transport::inbox_any` seed): a split whose
/// size rule gives more than 0x204 bytes, all present, trips the size assert
/// rather than being treated as a discarded tail.
#[test]
fn delivery_asserts_on_an_oversized_split() {
    let mut buf = vec![0x94, 177, 132, 188, 247, 18];
    buf.resize(537, 0);
    let mut inbox = Inbox::default();
    assert_eq!(
        inbox.deliver(&crate::adapters::ProtoSizes, &buf),
        Err(QueueError::BadSize(537))
    );
}
