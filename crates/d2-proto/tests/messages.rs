//! Typed decode/encode of the fixed message layouts, from fixed byte vectors.
//!
//! Integration tests of `d2_proto`'s client and server message types and
//! the generated descriptor tables that `d2-sim`, `d2-net` and the client
//! rely on for wire layout.

use d2_proto::client::{BindHotkey, CreateGame, EquipItem, SelectSkill, Walk, WalkToUnit};
use d2_proto::schema::{FieldType, Gate, HandlerSize, Kind, Scope, SizeRule};
use d2_proto::server::{LoadAct, QuestInfo, SetStatWord};
use d2_proto::{DecodeError, FixedMessage, CLIENT_MESSAGES, SERVER_MESSAGES};

// SelectSkill byte vector.
#[test]
fn select_skill_vector() {
    let b = [0x3C, 0x05, 0x00, 0x00, 0x80, 0xFF, 0xFF, 0xFF, 0xFF];
    let m = SelectSkill::decode(&b).unwrap();
    assert_eq!(
        m,
        SelectSkill {
            skill: 5,
            left: true,
            item: u32::MAX
        }
    );
    assert_eq!(m.encode(), b);
}

// BindHotkey byte vector.
#[test]
fn bind_hotkey_vector() {
    let b = [0x51, 0x06, 0x80, 0x03, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
    let m = BindHotkey::decode(&b).unwrap();
    assert_eq!(
        m,
        BindHotkey {
            skill: 6,
            left: true,
            slot: 3,
            item: u32::MAX
        }
    );
    assert_eq!(m.encode(), b);
}

// Walk and WalkToUnit round-trip.
#[test]
fn walk_and_unit() {
    let b = [0x01, 0x10, 0x00, 0x20, 0x00];
    assert_eq!(Walk::decode(&b).unwrap(), Walk { x: 0x10, y: 0x20 });
    assert_eq!(Walk { x: 0x10, y: 0x20 }.encode(), b);
    let u = WalkToUnit {
        type_: 1,
        id: 0x1234_5678,
    };
    assert_eq!(u.encode(), [0x02, 1, 0, 0, 0, 0x78, 0x56, 0x34, 0x12]);
    assert_eq!(WalkToUnit::decode(&u.encode()).unwrap(), u);
}

// Empty, wrong-id and wrong-size inputs each give their own error.
#[test]
fn decode_errors() {
    assert_eq!(Walk::decode(&[]), Err(DecodeError::Empty));
    assert_eq!(
        Walk::decode(&[0x03, 0, 0, 0, 0]),
        Err(DecodeError::WrongId {
            expected: 0x01,
            found: 0x03
        })
    );
    // Handler 0x01 with size 6: wrong size.
    assert_eq!(
        Walk::decode(&[0x01, 0, 0, 0, 0, 0]),
        Err(DecodeError::WrongSize {
            expected: 5,
            found: 6
        })
    );
}

/// Body location is a u8 inside a 4-byte slot; bytes +6..+8 are ignored
/// and encode as 0.
#[test]
fn body_location_u8() {
    let b = [0x1A, 7, 0, 0, 0, 4, 0xAA, 0xBB, 0xCC];
    let m = EquipItem::decode(&b).unwrap();
    assert_eq!(
        m,
        EquipItem {
            item: 7,
            bodyloc: 4
        }
    );
    assert_eq!(m.encode(), [0x1A, 7, 0, 0, 0, 4, 0, 0, 0]);
}

#[test]
#[should_panic(expected = "does not fit")]
fn bit_field_overflow_panics() {
    let _ = BindHotkey {
        skill: 0x8000,
        left: false,
        slot: 0,
        item: 0,
    }
    .encode();
}

// CreateGame and generated server layouts.
#[test]
fn session_and_server_layouts() {
    let mut name = [0u8; 16];
    name[..3].copy_from_slice(b"Bob");
    let m = CreateGame {
        char_name: name,
        view_gap: 1,
        unk: 2,
        locale: 3,
        ..Default::default()
    };
    let b = m.encode();
    assert_eq!(b.len(), 46);
    assert_eq!(&b[0x15..0x18], b"Bob");
    assert_eq!(&b[0x2B..], &[1, 2, 3]);
    assert_eq!(CreateGame::decode(&b).unwrap(), m);

    let s = SetStatWord {
        stat: 12,
        value: 0x0102,
    };
    assert_eq!(s.encode(), [0x1E, 12, 0x02, 0x01]);
    assert_eq!(LoadAct::SIZE, 12);
    assert_eq!(LoadAct::decode(&[0x03; 12]).unwrap().act, 3);
}

/// Descriptor spot checks: sizes, never-queued ids, gates, stubs, scope.
#[test]
fn descriptors() {
    assert_eq!(CLIENT_MESSAGES.len(), 0x73);
    assert_eq!(SERVER_MESSAGES.len(), 0xB7);
    for (i, m) in CLIENT_MESSAGES.iter().enumerate() {
        assert_eq!(m.id as usize, i);
        // Transport and handler sizes agree for every id.
        if let HandlerSize::Exact(n) = m.handler_size {
            assert_eq!(m.transport_size, SizeRule::Fixed(n));
        }
    }
    for (i, m) in SERVER_MESSAGES.iter().enumerate() {
        assert_eq!(m.id as usize, i);
    }
    // Ids a client never queues.
    let never: Vec<u8> = CLIENT_MESSAGES
        .iter()
        .filter(|m| m.transport_size.is_never())
        .map(|m| m.id)
        .collect();
    assert_eq!(
        never,
        [
            0x00, 0x2B, 0x2C, 0x4A, 0x4E, 0x55, 0x56, 0x57, 0x5A, 0x5B, 0x5C, 0x64, 0x65, 0x6F,
            0x71
        ]
    );
    // Gates.
    let ungated: Vec<u8> = CLIENT_MESSAGES
        .iter()
        .filter(|m| m.gate == Gate::None)
        .map(|m| m.id)
        .collect();
    assert_eq!(ungated, [0x14, 0x15, 0x3C, 0x43, 0x66]);
    assert_eq!(CLIENT_MESSAGES[0x41].gate, Gate::Dead);
    // Stub handlers.
    let stub = |k| -> Vec<u8> {
        CLIENT_MESSAGES
            .iter()
            .filter(|m| m.kind == k)
            .map(|m| m.id)
            .collect()
    };
    assert_eq!(stub(Kind::Stub3), [0x2C, 0x2D, 0x39, 0x45, 0x52]);
    assert_eq!(stub(Kind::Stub0), [0x2E, 0x42, 0x43, 0x66]);
    // 80 intents are in the sim scope.
    let sim = CLIENT_MESSAGES
        .iter()
        .filter(|m| m.scope == Scope::Sim)
        .count();
    assert_eq!(sim, 80);
    assert_eq!(CLIENT_MESSAGES[0x3C].layout[1].ty, FieldType::Bit(31));
    // S→C size-0 ids with builders.
    for id in [0x83, 0x84, 0x88] {
        let m = &SERVER_MESSAGES[id];
        assert!(m.size.is_never() && !m.senders.is_empty());
    }
}

/// S→C 0x28 as sent when talking to an NPC:
/// `28 01 06000000 00 01 00…`, the 96-byte quest record at 7.
#[test]
fn quest_info_carries_its_record_whole() {
    let mut b = [0u8; 103];
    b[0] = 0x28;
    b[1] = 1;
    b[2] = 6;
    b[7] = 1;
    b[102] = 0xAB;
    let m = QuestInfo::decode(&b).unwrap();
    assert_eq!((m.type_, m.guid, m.f6), (1, 6, 0));
    assert_eq!((m.flags[0], m.flags[95]), (1, 0xAB));
    assert_eq!(m.encode(), b);
    assert_eq!(QuestInfo::default().flags, [0; 96]);
    assert_eq!(
        QuestInfo::decode(&b[..102]),
        Err(DecodeError::WrongSize {
            expected: 103,
            found: 102
        })
    );
}
