//! Tests of the S→C builders and parser: sizes against `server-messages.tsv`,
//! full byte coverage of each layout, the audit against the TSV, the parser
//! and the built types, and fixed byte vectors of real messages. Each check
//! has a perturbation test that proves the check can fail.

use super::parse::PARSED_IDS;
use super::*;
use crate::schema::Size;
use crate::transport::{server_message, server_size};

/// A built type's layout, as data.
struct Layout {
    id: u8,
    name: &'static str,
    size: usize,
    fields: &'static [(&'static str, usize, usize)],
    consts: &'static [(usize, u8)],
    unwritten: &'static [usize],
}

macro_rules! layouts {
    ($($t:ident),* $(,)?) => {
        vec![$(Layout {
            id: <$t as ServerMsg>::ID,
            name: stringify!($t),
            size: <$t as ServerMsg>::SIZE,
            fields: <$t as ServerMsg>::FIELDS,
            consts: <$t as ServerMsg>::CONSTS,
            unwritten: <$t as ServerMsg>::UNWRITTEN,
        }),*]
    };
}

fn built() -> Vec<Layout> {
    layouts!(
        PlayerStop,
        QuestInfo,
        GameQuestInfo,
        NpcTransaction,
        MercForHire,
        QuestSpecial,
        QuestLogInfo,
        OpenUi,
        QuestItemState,
        WaypointMenu,
        TradeAction,
        UniqueEvent,
        NpcWantsInteract,
        NpcGossipAct,
        Unknown9B,
        GameHandshake,
        AssignPlayer,
    )
}

/// Ids whose type size differs from the TSV's fixed size.
fn size_mismatches(layouts: &[Layout]) -> Vec<u8> {
    layouts
        .iter()
        .filter(|l| server_message(l.id).and_then(|m| m.size.fixed()) != Some(l.size))
        .map(|l| l.id)
        .collect()
}

/// Bytes 1..size not covered exactly once by fields, constants and
/// unwritten bytes (or covered past the end).
fn coverage_errors(l: &Layout) -> Vec<usize> {
    let mut n = vec![0u32; l.size.max(1)];
    let mut past = Vec::new();
    let mut mark = |o: usize| match n.get_mut(o) {
        Some(c) => *c += 1,
        None => past.push(o),
    };
    for &(_, off, len) in l.fields {
        (off..off + len).for_each(&mut mark);
    }
    l.consts.iter().for_each(|&(o, _)| mark(o));
    l.unwritten.iter().for_each(|&o| mark(o));
    let mut bad: Vec<usize> = (1..l.size).filter(|&o| n[o] != 1).collect();
    bad.extend(past);
    bad
}

#[test]
fn sizes_match_tsv() {
    let mut l = built();
    assert_eq!(size_mismatches(&l), Vec::<u8>::new());
    // Perturbation: one wrong size is reported, and only it.
    l[5].size += 1;
    assert_eq!(size_mismatches(&l), vec![0x50]);
}

#[test]
fn layouts_cover_every_byte() {
    for l in built() {
        assert_eq!(coverage_errors(&l), Vec::<usize>::new(), "0x{:02X}", l.id);
    }
    // Perturbation: dropping 0x2A's `guid` leaves exactly bytes 7..11 uncovered;
    // a field overlapping another is reported too.
    let mut l = built().remove(3);
    assert_eq!(l.id, 0x2A);
    l.fields = &[("kind", 1, 1), ("code", 2, 1), ("gold", 11, 4)];
    assert_eq!(coverage_errors(&l), vec![7, 8, 9, 10]);
    l.fields = &[
        ("kind", 1, 1),
        ("code", 2, 2),
        ("guid", 7, 4),
        ("gold", 11, 4),
    ];
    assert_eq!(coverage_errors(&l), vec![3]);
}

/// Audit rows that disagree with the TSV, the parser or the built types.
fn audit_errors(rows: &[Audit]) -> Vec<u8> {
    let names: Vec<(u8, &str)> = built().iter().map(|l| (l.id, l.name)).collect();
    let mut bad = Vec::new();
    for (i, a) in rows.iter().enumerate() {
        let m = server_message(i as u8).expect("row per id");
        let parsed = PARSED_IDS.contains(&a.id);
        let parsed_ok = match a.status {
            Status::Built | Status::Generated => parsed,
            Status::Partial => !parsed,
            Status::Unspecified | Status::IdOnly | Status::Never => !parsed,
        };
        let builder_ok = match a.status {
            Status::Built if a.id == 0xAE => a.builder == Some("WardenRequest"),
            Status::Built => names.contains(&(a.id, a.builder.unwrap_or(""))),
            Status::Generated => {
                a.builder == Some(m.name) && (!m.layout.is_empty() || m.size.fixed() == Some(1))
            }
            Status::Partial => a.builder.is_none(),
            Status::Unspecified | Status::IdOnly | Status::Never => a.builder.is_none(),
        };
        let ok = a.id as usize == i
            && (a.status == Status::Never) == m.size.is_never()
            && parsed_ok
            && builder_ok;
        if !ok {
            bad.push(i as u8);
        }
    }
    bad
}

/// Rows whose empty `layout` is complete: the server writes only the id.
const ID_ONLY: [u8; 9] = [0x00, 0x02, 0x04, 0x05, 0x06, 0x4F, 0x97, 0xB0, 0x7E];

// Every id-only row is built or generated, never counted as a gap.
#[test]
fn empty_layout_rows_are_complete() {
    for id in ID_ONLY {
        let m = server_message(id).unwrap();
        assert!(m.layout.is_empty(), "0x{id:02X}");
        let done = matches!(audit(id).status, Status::Generated | Status::IdOnly);
        assert!(done, "0x{id:02X} counted as a gap");
    }
    // Every other empty-layout row with a size is a real gap or never.
    for id in 0..=0xB4u8 {
        let m = server_message(id).unwrap();
        let s = audit(id).status;
        if m.layout.is_empty() && !ID_ONLY.contains(&id) && !m.size.is_never() {
            assert_ne!(s, Status::IdOnly, "0x{id:02X}");
        }
    }
}

#[test]
fn audit_matches_tsv_parser_and_types() {
    assert_eq!(audit_errors(&AUDIT), Vec::<u8>::new());
    for id in 0..=0xB4u8 {
        let m = server_message(id).unwrap();
        // A generated row is exactly a fixed TSV layout row.
        if audit(id).status == Status::Generated {
            assert!(m.size.fixed().is_some(), "0x{id:02X}");
        }
    }
    // Perturbation: a status change, or a builder name change, is reported alone.
    let mut rows = AUDIT;
    rows[0x63].status = Status::Partial;
    rows[0x08].status = Status::Never;
    rows[0x77].builder = Some("Trade");
    assert_eq!(audit_errors(&rows), vec![0x08, 0x63, 0x77]);
}

fn hex(s: &str) -> Vec<u8> {
    let s: String = s.split_whitespace().collect();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Offsets where two messages differ, except `unwritten` ones.
fn diff_offsets(a: &[u8], b: &[u8], unwritten: &[usize]) -> Vec<usize> {
    assert_eq!(a.len(), b.len());
    (0..a.len())
        .filter(|&i| a[i] != b[i] && !unwritten.contains(&i))
        .collect()
}

fn wp_record(first: u16) -> [u8; WAYPOINT_RECORD] {
    let mut r = [0; WAYPOINT_RECORD];
    r[0] = 2;
    r[1] = 1;
    r[2..4].copy_from_slice(&first.to_le_bytes());
    r
}

#[test]
fn recorded_waypoint_messages() {
    // Byte vectors of real waypoint-menu messages.
    for (bytes, guid, first) in [
        (
            "63 0b000000 0201 0100 0000 0000 0000 0000 0000 0000",
            0x0B,
            1,
        ),
        (
            "63 0a000000 0201 0300 0000 0000 0000 0000 0000 0000",
            0x0A,
            3,
        ),
        (
            "63 33000000 0201 0300 0000 0000 0000 0000 0000 0000",
            0x33,
            3,
        ),
    ] {
        let m = WaypointMenu {
            object_guid: guid,
            record: wp_record(first),
        };
        assert_eq!(m.encode().to_vec(), hex(bytes));
        assert_eq!(parse(&hex(bytes)), Ok(Message::WaypointMenu(m)));
    }
    for (bytes, x, y) in [
        ("0d 00 01000000 01 2013 8413 00 00", 0x1320, 0x1384),
        ("0d 00 01000000 01 5d12 b311 00 00", 0x125D, 0x11B3),
    ] {
        let m = PlayerStop {
            unit_type: 0,
            unit_guid: 1,
            f6: 1,
            x,
            y,
            f11: 0,
            f12: 0,
        };
        assert_eq!(m.encode().to_vec(), hex(bytes));
        assert_eq!(parse(&hex(bytes)), Ok(Message::PlayerStop(m)));
    }
    // 0x07 (generated layout) from the same frames.
    let m = MapReveal {
        x: 0x3D0,
        y: 0x3E0,
        level: 3,
    };
    assert_eq!(parse(&hex("07 d003 e003 03")), Ok(Message::MapReveal(m)));
    assert_eq!(m.encode().to_vec(), hex("07 d003 e003 03"));
}

#[test]
fn recorded_npc_and_quest_messages() {
    // Bytes 3–6 carry no value (arbitrary bytes here), masked.
    for (bytes, kind, code, guid, gold) in [
        ("2a 03 01 05a4f619 07000000 f4010000", 3, 1, 7, 500),
        ("2a 04 00 056cf619 36000000 bc010000", 4, 0, 0x36, 0x1BC),
    ] {
        let m = NpcTransaction {
            kind,
            code,
            guid,
            gold,
        };
        assert_eq!(
            diff_offsets(&m.encode(), &hex(bytes), NpcTransaction::UNWRITTEN),
            Vec::<usize>::new()
        );
        assert_eq!(parse(&hex(bytes)), Ok(Message::NpcTransaction(m)));
    }
    // Known prefixes of 0x28 (the record bytes are not pinned).
    for (prefix, unit_type, unit_guid) in [("28 06 00000000 00", 6, 0), ("28 01 06000000 00", 1, 6)]
    {
        let m = QuestInfo {
            unit_type,
            unit_guid,
            record: [0; QUEST_RECORD],
        };
        assert_eq!(m.encode()[..7].to_vec(), hex(prefix));
    }
    // Known 0x5D vectors and its fixed forms.
    let m = QuestItemState {
        chain: 1,
        flags: 0,
        status: 1,
        extra: 0,
    };
    assert_eq!(m.encode().to_vec(), hex("5d 01 00 01 0000"));
    assert_eq!(
        parse(&hex("5d 01 00 01 0000")),
        Ok(Message::QuestItemState(m))
    );
    let fixed = |flags, status| QuestItemState {
        chain: 9,
        flags,
        status,
        extra: 0,
    };
    assert_eq!(fixed(2, 0).encode().to_vec(), hex("5d 09 02 00 0000"));
    assert_eq!(fixed(0, 0x0C).encode().to_vec(), hex("5d 09 00 0c 0000"));
    // 0x8A vector.
    let m = NpcWantsInteract { npc_guid: 7 };
    assert_eq!(m.encode().to_vec(), hex("8a 01 07000000"));
    assert_eq!(
        parse(&hex("8a 01 07000000")),
        Ok(Message::NpcWantsInteract(m))
    );
    // 0x9B after a mercenary resurrection.
    let m = Unknown9B { f1: 0xFFFF, f3: 0 };
    assert_eq!(m.encode().to_vec(), hex("9b ffff 00000000"));
    // The cube result actions.
    for a in [0x0C, 0x11, 0x15] {
        assert_eq!(TradeAction { action: a }.encode(), [0x77, a]);
        assert_eq!(
            parse(&[0x77, a]),
            Ok(Message::TradeAction(TradeAction { action: a }))
        );
    }
}

#[test]
fn recorded_vector_perturbation_is_reported() {
    // Perturbation: one flipped byte of a real message is reported at its
    // offset; a flipped unwritten byte is not.
    let real = hex("63 0a000000 0201 0300 0000 0000 0000 0000 0000 0000");
    let built = WaypointMenu {
        object_guid: 0x0A,
        record: wp_record(3),
    }
    .encode();
    for i in 0..real.len() {
        let mut p = real.clone();
        p[i] ^= 0x40;
        assert_eq!(diff_offsets(&built, &p, &[]), vec![i]);
    }
    let real = hex("2a 03 01 05a4f619 07000000 f4010000");
    let built = NpcTransaction {
        kind: 3,
        code: 1,
        guid: 7,
        gold: 500,
    }
    .encode();
    for i in 0..real.len() {
        let mut p = real.clone();
        p[i] ^= 0x40;
        let want = if (3..7).contains(&i) { vec![] } else { vec![i] };
        assert_eq!(diff_offsets(&built, &p, NpcTransaction::UNWRITTEN), want);
    }
}

// 0x0B and 0x59 byte vectors of a single-player join.
#[test]
fn recorded_join_messages() {
    // 0x0B names player GUID 1.
    let b = hex("0b 00 01000000");
    let m = GameHandshake {
        unit_type: 0,
        unit_guid: 1,
    };
    assert_eq!(m.encode().to_vec(), b);
    assert_eq!(parse(&b), Ok(Message::GameHandshake(m)));
    // 0x59: GUID @1, class @5, name @6 (16 bytes, zero-padded), x @0x16,
    // y @0x18; the joining player is at (0, 0).
    let mut name = [0u8; 16];
    name[..6].copy_from_slice(b"werwer");
    let m = AssignPlayer {
        guid: 1,
        class: 1,
        name,
        x: 0,
        y: 0,
    };
    let mut want = hex("59 01000000 01 7765727765 72");
    want.resize(26, 0);
    assert_eq!(m.encode().to_vec(), want);
    assert_eq!(parse(&want), Ok(Message::AssignPlayer(m)));
    let m = AssignPlayer {
        x: 4673,
        y: 4548,
        ..m
    };
    assert_eq!(m.encode()[0x16..].to_vec(), hex("4112 c411"));
}

#[test]
fn recorded_messages_of_the_layout_batch_parse() {
    // Real 0x15 / 0x51 messages and a 0x27 prefix: sizes agree with the TSV
    // and the generated layouts read the fields.
    for (bytes, x, y) in [
        ("15 00 01000000 1d13 8113 01", 0x131D, 0x1381),
        ("15 00 01000000 5a12 b011 01", 0x125A, 0x11B0),
    ] {
        let b = hex(bytes);
        assert_eq!(server_size(&b), Size::Bytes(11));
        let Ok(Message::ReassignPlayer(m)) = parse(&b) else {
            panic!("{bytes}");
        };
        assert_eq!((m.type_, m.guid, m.x, m.y, m.flag), (0, 1, x, y, 1));
    }
    for (bytes, guid, x, y, mode) in [
        (
            "51 02 17000000 7700 1e13 8213 01 00",
            0x17,
            0x131E,
            0x1382,
            1,
        ),
        (
            "51 02 49000000 7700 5b12 b111 02 00",
            0x49,
            0x125B,
            0x11B1,
            2,
        ),
    ] {
        let b = hex(bytes);
        assert_eq!(server_size(&b), Size::Bytes(14));
        let Ok(Message::AssignObject(m)) = parse(&b) else {
            panic!("{bytes}");
        };
        assert_eq!(
            (m.type_, m.guid, m.class, m.x, m.y, m.mode, m.interact),
            (2, guid, 0x77, x, y, mode, 0)
        );
    }
    let mut b = hex("27 01 06000000 01000000 25 00");
    b.resize(40, 0);
    let Ok(Message::NpcInfo(m)) = parse(&b) else {
        panic!("0x27");
    };
    assert_eq!(
        (m.type_, m.guid, m.count, m.kind0, m.str0),
        (1, 6, 1, 0, 0x25)
    );
    // 0x50 mercenary form (u16 2 at 1): the TSV layout, name u16@3.
    let mut b = vec![0x50, 2, 0, 0x2A, 0];
    b.resize(15, 0);
    assert_eq!(
        parse(&b),
        Ok(Message::QuestSpecialForm(
            crate::generated::server::QuestSpecial {
                code: 2,
                v0: 0x2A,
                ..Default::default()
            }
        ))
    );
    // Code 1 with a word past byte 9 set: not the code-1 view either.
    let mut b = vec![0x50, 1, 0];
    b.resize(15, 0);
    b[9] = 7;
    assert_eq!(
        parse(&b),
        Ok(Message::QuestSpecialForm(
            crate::generated::server::QuestSpecial {
                code: 1,
                v3: 7,
                ..Default::default()
            }
        ))
    );
}

fn samples() -> Vec<Message> {
    let mut record = [0u8; QUEST_RECORD];
    record
        .iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b = i as u8 ^ 0x5A);
    let mut list = [0u8; QUEST_LOG_ENTRIES];
    list.iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b = i as u8 + 1);
    let mut slots = [0xFFFFu16; GOSSIP_SLOTS];
    slots[0] = 0x9A;
    slots[1] = 0x94;
    vec![
        Message::PlayerStop(PlayerStop {
            unit_type: 0,
            unit_guid: 0x1234_5678,
            f6: 1,
            x: 0x2D,
            y: 0x17,
            f11: 2,
            f12: 3,
        }),
        Message::QuestInfo(QuestInfo {
            unit_type: 1,
            unit_guid: 6,
            record,
        }),
        Message::GameQuestInfo(GameQuestInfo { record }),
        Message::NpcTransaction(NpcTransaction {
            kind: 5,
            code: 0,
            guid: u32::MAX,
            gold: 123_456,
        }),
        Message::MercForHire(MercForHire {
            name: 0x1234,
            seed: 0xDEAD_BEEF,
        }),
        Message::QuestSpecial(QuestSpecial {
            den_left: 17,
            staff_tomb: -2,
            barbarians_left: 5,
        }),
        Message::QuestLogInfo(QuestLogInfo { list }),
        Message::OpenUi(OpenUi {
            npc_guid: 9,
            result: 5,
            effect: 1,
        }),
        Message::QuestItemState(QuestItemState {
            chain: 3,
            flags: 4,
            status: 5,
            extra: 0x0102,
        }),
        Message::WaypointMenu(WaypointMenu {
            object_guid: 0x33,
            record: wp_record(0x7FFF),
        }),
        Message::TradeAction(TradeAction { action: 0x15 }),
        Message::UniqueEvent(UniqueEvent { event: 0 }),
        Message::NpcWantsInteract(NpcWantsInteract { npc_guid: 0x10 }),
        Message::NpcGossipAct(NpcGossipAct { act: 1, slots }),
        Message::Unknown9B(Unknown9B {
            f1: 0xFFFF,
            f3: 0x0102_0304,
        }),
        Message::WardenRequest(WardenRequest {
            data: vec![1, 2, 3],
        }),
        Message::GameHandshake(GameHandshake {
            unit_type: 0,
            unit_guid: 0x0102_0304,
        }),
        Message::AssignPlayer(AssignPlayer {
            guid: 7,
            class: 4,
            name: *b"abcdefghijklmnop",
            x: 0x1234,
            y: 0x5678,
        }),
    ]
}

fn encode(m: &Message) -> Vec<u8> {
    match m {
        Message::PlayerStop(m) => m.encode().to_vec(),
        Message::QuestInfo(m) => m.encode().to_vec(),
        Message::GameQuestInfo(m) => m.encode().to_vec(),
        Message::NpcTransaction(m) => m.encode().to_vec(),
        Message::MercForHire(m) => m.encode().to_vec(),
        Message::QuestSpecial(m) => m.encode().to_vec(),
        Message::QuestLogInfo(m) => m.encode().to_vec(),
        Message::OpenUi(m) => m.encode().to_vec(),
        Message::QuestItemState(m) => m.encode().to_vec(),
        Message::WaypointMenu(m) => m.encode().to_vec(),
        Message::TradeAction(m) => m.encode().to_vec(),
        Message::UniqueEvent(m) => m.encode().to_vec(),
        Message::NpcWantsInteract(m) => m.encode().to_vec(),
        Message::NpcGossipAct(m) => m.encode().to_vec(),
        Message::Unknown9B(m) => m.encode().to_vec(),
        Message::WardenRequest(m) => m.encode(),
        Message::GameHandshake(m) => m.encode().to_vec(),
        Message::AssignPlayer(m) => m.encode().to_vec(),
        other => panic!("not a sample: {other:?}"),
    }
}

#[test]
fn built_messages_round_trip_through_the_size_rule() {
    for m in samples() {
        let b = encode(&m);
        assert_eq!(server_size(&b), Size::Bytes(b.len()), "{m:?}");
        assert_eq!(parse(&b), Ok(m));
    }
    // Spot checks of field placement.
    let b = encode(&samples()[5]);
    assert_eq!(b, hex("50 0100 1100 feff 0500 000000000000"));
    let b = encode(&samples()[13]);
    assert_eq!(b[..6].to_vec(), hex("91 01 9a00 9400"));
    assert_eq!(b[6..], [0xFF; 20]);
}

#[test]
fn parse_rejects_bad_messages() {
    assert_eq!(parse(&[]), Err(ParseError::Empty));
    assert_eq!(
        parse(&[0x80, 0, 0, 0]),
        Err(ParseError::Invalid { id: 0x80 })
    );
    assert_eq!(parse(&[0xB5]), Err(ParseError::Invalid { id: 0xB5 }));
    assert_eq!(
        parse(&[0x16, 0x20, 0x00]),
        Err(ParseError::Incomplete { id: 0x16 })
    );
    assert_eq!(
        parse(&[0x77, 0x0C, 0x00]),
        Err(ParseError::WrongSize {
            id: 0x77,
            expected: 2,
            found: 3
        })
    );
    assert_eq!(
        parse(&hex("8a 02 07000000")),
        Err(ParseError::Const {
            id: 0x8A,
            offset: 1,
            expected: 1,
            found: 2
        })
    );
    let mut b = vec![0x12];
    b.resize(26, 0);
    assert_eq!(
        parse(&b),
        Err(ParseError::Unbuilt {
            id: 0x12,
            status: Status::Unspecified
        })
    );
    // Unwritten bytes are ignored.
    let m = NpcTransaction {
        kind: 1,
        code: 6,
        guid: 9,
        gold: 10,
    };
    let mut b = m.encode();
    b[3..7].copy_from_slice(&[0xCC; 4]);
    assert_eq!(parse(&b), Ok(Message::NpcTransaction(m)));
}

#[test]
fn warden_request_size_rule() {
    // Size rule: `AE 10 00` → 19; a length over 0x1FD gives size 3.
    let m = WardenRequest {
        data: vec![0xAB; 0x10],
    };
    assert_eq!(m.encode().len(), 19);
    let max = WardenRequest {
        data: vec![7; WARDEN_MAX],
    };
    assert_eq!(server_size(&max.encode()), Size::Bytes(0x200));
    assert_eq!(parse(&max.encode()), Ok(Message::WardenRequest(max)));
    assert_eq!(
        parse(&hex("ae fe01")),
        Ok(Message::WardenRequest(WardenRequest { data: vec![] }))
    );
}
