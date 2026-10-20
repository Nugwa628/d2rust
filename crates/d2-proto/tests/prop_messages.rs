//! Robustness properties of `d2_proto`: any C→S bytes a client can send,
//! and any S→C buffer, go through the size rules, the classifier, the
//! buffer split and every typed decode without a panic, and with the
//! results the size rules and descriptor tables give. The server is
//! authoritative, so nothing here may trust the bytes: these properties
//! guard the parsing `d2-server` runs on client input.
//!
//! Default case counts are small so `cargo test` stays fast; set
//! `PROPTEST_CASES` to hunt harder (it overrides every default here).

use d2_proto::client::*;
use d2_proto::schema::{Size, SizeRule};
use d2_proto::server::*;
use d2_proto::transport::{
    classify_client, client_size, server_size, split_server_buffer, Classified, ClientQueue,
    SplitError, ADMIN_SIZE, MAX_MESSAGE,
};
use d2_proto::{DecodeError, FixedMessage, CLIENT_MESSAGES, SERVER_MESSAGES};
use proptest::prelude::*;
use proptest::test_runner::Config;

/// Proptest config with `default` cases, or `PROPTEST_CASES` when set.
fn config(default: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

/// A message-shaped input: a chosen or random id, then `0..max` bytes,
/// biased to the edge values size fields and strings react to.
fn message(max: usize) -> impl Strategy<Value = Vec<u8>> {
    let byte = prop_oneof![
        3 => any::<u8>(),
        1 => Just(0u8),
        1 => Just(0xFFu8),
        1 => Just(0x80u8),
        1 => Just(0x7Fu8),
    ];
    (any::<u8>(), prop::collection::vec(byte, 0..max)).prop_map(|(id, rest)| {
        let mut m = Vec::with_capacity(rest.len() + 1);
        m.push(id);
        m.extend(rest);
        m
    })
}

/// The largest size a C→S rule can give (the C→S size rule): a u16 field
/// times its multiplier plus its addend, or the chat rule (two strings
/// in the bytes, plus a signed byte).
fn rule_bound(rule: &SizeRule, len: usize) -> usize {
    match *rule {
        SizeRule::Fixed(n) => n as usize,
        SizeRule::Field { mul, add, .. } => 0xFFFF * mul as usize + add as usize,
        SizeRule::Chat | SizeRule::Chat26 => len + 127,
        SizeRule::Af => 0x100,
    }
}

proptest! {
    #![proptest_config(config(256))]

    /// The C→S size rule never panics, is bounded by its rule, and reads
    /// only its own fields: once a prefix has a result, every longer
    /// buffer has the same one (truncation is only ever `Incomplete`).
    #[test]
    fn client_size_any_bytes(b in message(0x240)) {
        let full = client_size(&b);
        if let Size::Bytes(n) = full {
            let m = &CLIENT_MESSAGES[b[0] as usize];
            prop_assert!(n <= rule_bound(&m.transport_size, b.len()), "{n} for {:02X}", b[0]);
        }
        if b[0] == 0x71 || b[0] >= 0x73 {
            prop_assert_eq!(full, Size::Invalid);
        }
        for k in 0..b.len() {
            let p = client_size(&b[..k]);
            if p != Size::Incomplete {
                prop_assert_eq!(p, full, "prefix {} of {:02X?}", k, &b);
            }
        }
    }

    /// The classifier on arbitrary bytes and every
    /// truncation: queued only with a size rule ≤ 0x204 that fits the
    /// given bytes; the reserved envelope id and unassigned ids outside the\n    /// tables are always invalid; the queue by id.
    #[test]
    fn classify_any_bytes(b in message(0x240)) {
        for k in 0..=b.len() {
            let m = &b[..k];
            let c = classify_client(m);
            let Some(&id) = m.first() else {
                prop_assert_eq!(c, Classified::Incomplete);
                continue;
            };
            if id == 0x71 || (0x73..=0xFE).contains(&id) {
                prop_assert_eq!(c, Classified::Invalid);
                continue;
            }
            let size = if id == 0xFF { Size::Bytes(ADMIN_SIZE) } else { client_size(m) };
            match c {
                Classified::Queue(q) => {
                    let Size::Bytes(n) = size else {
                        return Err(TestCaseError::fail(format!("queued {size:?}")));
                    };
                    prop_assert!(n != 0 && n <= MAX_MESSAGE && n <= m.len());
                    let want = match id {
                        0..=0x66 => ClientQueue::Game,
                        0xFF => ClientQueue::Admin,
                        _ => ClientQueue::System,
                    };
                    prop_assert_eq!(q, want);
                }
                Classified::NegativeSize(n) => {
                    prop_assert!(n < 0);
                    prop_assert_eq!(size, Size::Negative(n));
                }
                Classified::Incomplete => {
                    prop_assert!(!matches!(size, Size::Bytes(n) if n <= MAX_MESSAGE && n <= m.len()));
                }
                Classified::Invalid => {
                    return Err(TestCaseError::fail(format!("invalid id {id:02X}")));
                }
            }
        }
    }

    /// The S→C size rule never panics and reads only its
    /// own fields.
    #[test]
    fn server_size_any_bytes(b in message(0x240)) {
        let full = server_size(&b);
        prop_assert!(!matches!(full, Size::Negative(_)), "S→C rules never go negative");
        if b[0] == 0xB5 || b[0] >= 0xB7 {
            prop_assert_eq!(full, Size::Invalid);
        }
        for k in 0..b.len() {
            let p = server_size(&b[..k]);
            if p != Size::Incomplete {
                prop_assert_eq!(p, full);
            }
        }
    }

    /// The S→C buffer split on arbitrary buffers: the
    /// messages and the discarded tail are the buffer, each message has
    /// its rule's size, and a failure names a message that is over 0x204
    /// or runs past the end.
    #[test]
    fn split_any_buffer(b in prop::collection::vec(any::<u8>(), 0..0x400)) {
        match split_server_buffer(&b) {
            Ok(s) => {
                let mut joined: Vec<u8> = s.messages.concat();
                joined.extend_from_slice(s.discarded);
                prop_assert_eq!(&joined, &b);
                // A rule may read past the message it sizes (0x16 needs 13
                // bytes for any size), so it is evaluated where the split
                // evaluated it: on the rest of the buffer.
                let mut at = 0;
                for m in &s.messages {
                    prop_assert_eq!(server_size(&b[at..]), Size::Bytes(m.len()));
                    prop_assert!(!m.is_empty() && m.len() <= MAX_MESSAGE);
                    at += m.len();
                }
                if let Some(&id) = s.discarded.first() {
                    prop_assert!(!matches!(server_size(s.discarded), Size::Bytes(_)), "{id:02X}");
                }
            }
            Err(SplitError::TooLarge { at, size }) => {
                prop_assert!(size > MAX_MESSAGE);
                prop_assert_eq!(server_size(&b[at..]), Size::Bytes(size));
            }
            Err(SplitError::Truncated { at, size }) => {
                prop_assert!(size <= MAX_MESSAGE && size > b.len() - at);
                prop_assert_eq!(server_size(&b[at..]), Size::Bytes(size));
            }
        }
    }
}

/// Checks one typed message on `b`: decode succeeds exactly for `SIZE`
/// bytes starting with `ID` and the error names the first
/// failing check; a decoded message re-encodes to `SIZE` bytes that
/// decode to the same message.
fn typed<M: FixedMessage + PartialEq + std::fmt::Debug>(b: &[u8]) -> Result<(), TestCaseError> {
    // The same bytes with the type's id, so the size path is reached.
    let mut own = b.to_vec();
    if let Some(f) = own.first_mut() {
        *f = M::ID;
    }
    for m in [b, &own[..]] {
        let r = M::decode(m);
        let want = match m.first() {
            None => Err(DecodeError::Empty),
            Some(&f) if f != M::ID => Err(DecodeError::WrongId {
                expected: M::ID,
                found: f,
            }),
            _ if m.len() != M::SIZE => Err(DecodeError::WrongSize {
                expected: M::SIZE,
                found: m.len(),
            }),
            _ => Ok(()),
        };
        match (r, want) {
            (Ok(v), Ok(())) => {
                let mut out = vec![0xAA; M::SIZE];
                v.write(&mut out);
                prop_assert_eq!(out[0], M::ID);
                prop_assert_eq!(M::decode(&out), Ok(v));
            }
            (Err(e), Err(w)) => prop_assert_eq!(e, w),
            (r, w) => {
                return Err(TestCaseError::fail(format!(
                    "{:02X}: {r:?}, expected {w:?}",
                    M::ID
                )))
            }
        }
    }
    Ok(())
}

macro_rules! all_typed {
    ($b:expr; $($t:ty),* $(,)?) => {{
        $(typed::<$t>($b)?;)*
    }};
}

fn all_client(b: &[u8]) -> Result<(), TestCaseError> {
    all_typed!(b;
        Walk, WalkToUnit, Run, RunToUnit, ShiftLeftSkill, LeftSkillOnUnit,
        ShiftLeftSkillOnUnit, ShiftLeftSkillHold, LeftSkillOnUnitHold,
        ShiftLeftSkillOnUnitHold, Unused0B, RightSkill, RightSkillOnUnit,
        ShiftRightSkillOnUnit, RightSkillHold, RightSkillOnUnitHold,
        ShiftRightSkillOnUnitHold, EndInferno, InteractWithEntity, PickItem, DropItem,
        InsertItemInBuffer, RemoveItemFromBuffer, EquipItem, Swap2HandedItem,
        RemoveBodyItem, SwapCursorWithBody, Swap1HWith2H, SwapCursorBufferItem,
        UseGridItem, StackItems, UnstackItems, ItemToBelt, ItemFromBelt,
        SwitchBeltItem, UseBeltItem, UseItemAction, SocketItem, ScrollToBook,
        ItemToCube, Unused2D, InitEntityChat, TerminateEntityChat, QuestMessage,
        BuyItem, SellItem, IdentifyWithNpc, Repair, HireMerc, IdentifyGamble,
        EntityAction, AddStatPoint, AddSkillPoint, SelectSkill, HighlightDoor,
        ActivateInifussScroll, PlayAudio, RequestQuestData, Resurrect, Unused42,
        Unused43, StaffInOrifice, MercInteract, MoveMerc, TurnOffBusyState,
        TakeOrCloseWp, RequestEntityUpdate, Transmogrify, PlayNpcMessage,
        ClickButton, DropGold, BindHotkey, StaminaOn, StaminaOff, QuestCompleted,
        MakeEntityMove, SquelchHostile, PartyAction, UpdatePlayerPos, SwapWeapons,
        MercItem, ResurrectMerc, ItemToBeltShift, CreateGame, JoinGame, LeaveGame,
        Sys6A, Sys6B, Ping, Sys6E, Sys70,
    );
    Ok(())
}

fn all_server(b: &[u8]) -> Result<(), TestCaseError> {
    all_typed!(b;
        GameLoading, GameFlags, LoadSuccessful, LoadAct, LoadComplete, UnloadComplete,
        GameExit, MapReveal, MonsterHit, AddExpByte, AddExpWord, AddExpDword,
        SetStatByte, SetStatWord, SetStatDword, StartMercList, PortalFlags,
        Unknown6E, Unknown6F, Unknown70, Unknown71, Unknown72, WeaponSwitch,
        ConnectionTerminated, PlayerStop, PlayerMove, PlayerToTarget, ReassignPlayer,
        WalkVerify,
    );
    Ok(())
}

/// Lengths around every typed size, so the `SIZE` case is hit often.
fn typed_input() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        message(64),
        (0usize..64).prop_flat_map(|n| prop::collection::vec(any::<u8>(), n..=n)),
    ]
}

proptest! {
    #![proptest_config(config(256))]

    /// Every typed C→S decode on arbitrary bytes.
    #[test]
    fn client_typed_any_bytes(b in typed_input()) {
        all_client(&b)?;
    }

    /// Every typed S→C decode on arbitrary bytes.
    #[test]
    fn server_typed_any_bytes(b in typed_input()) {
        all_server(&b)?;
    }
}

/// Each typed message's id and size are its table row's (the decode
/// size is the transport size), so the typed tests above
/// cover every id with a fixed layout.
#[test]
fn typed_sizes_match_tables() {
    fn c<M: FixedMessage>() {
        let m = &CLIENT_MESSAGES[M::ID as usize];
        assert_eq!(m.transport_size.fixed(), Some(M::SIZE), "{:02X}", M::ID);
    }
    fn s<M: FixedMessage>() {
        let m = &SERVER_MESSAGES[M::ID as usize];
        assert_eq!(m.size.fixed(), Some(M::SIZE), "{:02X}", M::ID);
    }
    c::<Walk>();
    c::<SelectSkill>();
    c::<BindHotkey>();
    c::<CreateGame>();
    c::<Ping>();
    s::<LoadAct>();
    s::<SetStatWord>();
    s::<ConnectionTerminated>();
    s::<WalkVerify>();
}
