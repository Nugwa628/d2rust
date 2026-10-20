//! The codec's field rules and the error answer's round trip.

use super::*;

#[test]
fn fields_round_trip() {
    let body = Writer::new(0x20)
        .u8(7)
        .u16(0x1234)
        .u32(0xdead_beef)
        .u64(u64::MAX - 1)
        .bool(true)
        .array(&[9u8; 16])
        .str8("Ünïcode name")
        .bytes8(&[1, 2, 3])
        .opt(Some(&5u32), |w, v| {
            w.u32(*v);
        })
        .opt(None::<&u32>, |w, v| {
            w.u32(*v);
        })
        .finish();
    let (k, mut r) = Reader::new(&body).unwrap();
    assert_eq!(k, 0x20);
    assert_eq!(r.u8().unwrap(), 7);
    assert_eq!(r.u16().unwrap(), 0x1234);
    assert_eq!(r.u32().unwrap(), 0xdead_beef);
    assert_eq!(r.u64().unwrap(), u64::MAX - 1);
    assert!(r.bool().unwrap());
    assert_eq!(r.array::<16>().unwrap(), [9; 16]);
    assert_eq!(r.str8(64).unwrap(), "Ünïcode name");
    assert_eq!(r.bytes8(8).unwrap(), [1, 2, 3]);
    assert_eq!(r.opt(|r| r.u32()).unwrap(), Some(5));
    assert_eq!(r.opt(|r| r.u32()).unwrap(), None);
    r.finish().unwrap();
}

/// Each field rule of contracts section 3.2 refuses what breaks it.
#[test]
fn bad_fields_are_malformed() {
    let read = |b: &[u8], f: fn(&mut Reader) -> Result<(), NetError>| {
        let (_, mut r) = Reader::new(b).unwrap();
        f(&mut r)
    };
    let bad = Err(NetError::Malformed);
    // Short body.
    assert_eq!(read(&[1, 2], |r| r.u32().map(drop)), bad);
    // A bool that is neither 0 nor 1.
    assert_eq!(read(&[1, 2], |r| r.bool().map(drop)), bad);
    // An optional marker that is neither 0 nor 1.
    assert_eq!(read(&[1, 2, 0], |r| r.opt(|r| r.u8()).map(drop)), bad);
    // A string longer than its field's limit.
    assert_eq!(
        read(&[1, 3, b'a', b'b', b'c'], |r| r.str8(2).map(drop)),
        bad
    );
    // A string that is not UTF-8.
    assert_eq!(read(&[1, 1, 0xff], |r| r.str8(8).map(drop)), bad);
    // A string with a control character.
    assert_eq!(read(&[1, 2, b'a', b'\n'], |r| r.str8(8).map(drop)), bad);
    // Bytes left after the last field.
    assert_eq!(read(&[1, 5, 6], |r| r.u8().map(drop)), Ok(()));
    let (_, mut r) = Reader::new(&[1, 5, 6]).unwrap();
    r.u8().unwrap();
    assert_eq!(r.finish(), bad);
    // An empty body has no kind.
    assert_eq!(Reader::new(&[]).map(drop), bad);
}

#[test]
fn long_strings_are_cut_on_a_character() {
    let s = "é".repeat(200); // 400 bytes
    let body = Writer::new(1).str8(&s).finish();
    assert_eq!(body[1], 254);
    let (_, mut r) = Reader::new(&body).unwrap();
    assert_eq!(r.str8(255).unwrap(), "é".repeat(127));
}

#[test]
fn error_round_trips() {
    let mut e = ErrorMsg::new(0x21, ErrorCode::RATE_LIMITED);
    e.wait_s = 42;
    assert_eq!(ErrorMsg::parse(&e.body()).unwrap(), e);
    // A code this build doesn't know still reads, with the server's text.
    let e = ErrorMsg {
        request_kind: 0x45,
        code: ErrorCode(500),
        wait_s: 0,
        text: "new rule".into(),
    };
    let got = ErrorMsg::parse(&e.body()).unwrap();
    assert_eq!(got, e);
    assert_eq!(got.code.name(), "Unknown");
    // Another kind is not an error answer.
    assert_eq!(ErrorMsg::parse(&[0x20]), Err(NetError::Malformed));
}

/// The table of contracts section 3.8: codes are unique, every code has
/// its own words, and the texts fit an error answer.
#[test]
fn error_codes_are_unique_and_worded() {
    let mut seen = std::collections::HashSet::new();
    for c in ErrorCode::ALL {
        assert!(seen.insert(c.0), "code {} twice", c.0);
        assert_ne!(c.name(), "Unknown");
        assert!(c.text().len() <= MAX_ERROR_TEXT);
    }
    assert_eq!(ErrorCode::DIFFICULTY_LOCKED.0, 66);
    assert_eq!(ErrorCode::TICKET_INVALID.0, 71);
    assert_eq!(ErrorCode::EXPANSION_CLASS.name(), "ExpansionClass");
    // The names are the contracts' (the MP test tool's scenarios use them).
    assert_eq!(ErrorCode::WRONG_LOGIN.name(), "WrongLogin");
    assert_eq!(ErrorCode::DIFFICULTY_LOCKED.name(), "DifficultyLocked");
    assert_eq!(ErrorCode::TICKET_INVALID.to_string(), "TicketInvalid (71)");
}

#[test]
fn kind_ranges_do_not_overlap() {
    let ranges = [kind::SESSION, kind::ACCOUNT, kind::LOBBY, kind::MOD];
    for (i, a) in ranges.iter().enumerate() {
        for b in &ranges[i + 1..] {
            assert!(a.end() < b.start());
        }
    }
    assert!(kind::SESSION.contains(&kind::ERROR));
    // Below the session range are the wire's own kinds (up to the game
    // stream's ATTACH 0x0A and FRAME_END 0x0B).
    assert!(*kind::SESSION.start() > 0x0B);
    assert!(*kind::SESSION.start() > crate::wire::kind::EXT);
}
