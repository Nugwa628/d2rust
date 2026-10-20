//! Tests written against surviving mutants (mutation testing): every typed
//! message of [`crate::generated`] decodes and encodes exactly the bytes
//! its descriptor's layout lists, and nothing else. Also pins edge cases of
//! the size rules ([`crate::schema`]), the transport classifier and buffer
//! split ([`crate::transport`]) and the typed S→C decode ([`crate::s2c`]).
//! Part of d2-proto's test suite only.

use crate::schema::{Field, FieldType};
use crate::wire::{DecodeError, FixedMessage};
use crate::{CLIENT_MESSAGES, SERVER_MESSAGES};

/// Mask of the bits a layout lists: `u8` / `u16` / `u32` / `cstr16` whole
/// bytes, `uN@k` bits `0..N` and `bitN@k` bit `N` of the u32 at `k`; a
/// packed field its bits, LSB-first from bit 0 of byte 0 (`bits:` layouts).
fn layout_mask(layout: &[Field], size: usize) -> Vec<u8> {
    let mut mask = vec![0u8; size];
    for f in layout {
        if let FieldType::Packed { bit, width } = f.ty {
            // LSB-first from bit 0 of byte 0 (`bits:` layouts).
            for i in usize::from(bit)..usize::from(bit) + usize::from(width) {
                mask[i / 8] |= 1 << (i % 8);
            }
            continue;
        }
        let off = usize::from(f.offset.expect("fixed message field has an offset"));
        let bits: Vec<u8> = match f.ty {
            FieldType::U8 => vec![0xFF],
            FieldType::U16 => vec![0xFF; 2],
            FieldType::U32 => vec![0xFF; 4],
            FieldType::Cstr16 => vec![0xFF; 16],
            FieldType::Bytes(n) => vec![0xFF; usize::from(n)],
            FieldType::Bits(n) => ((1u32 << n) - 1).to_le_bytes().to_vec(),
            FieldType::Bit(n) => (1u32 << n).to_le_bytes().to_vec(),
            FieldType::Packed { .. } => unreachable!("handled above"),
            FieldType::Cstr | FieldType::Tail => panic!("not a fixed field: {f:?}"),
        };
        for (m, b) in mask[off..off + bits.len()].iter_mut().zip(&bits) {
            *m |= b;
        }
    }
    mask
}

/// Decodes a patterned message, checks the re-encoding keeps exactly the
/// layout's bits (id byte included), and checks the decode errors.
fn check<T: FixedMessage + Copy + PartialEq + std::fmt::Debug + Default>(
    layout: &[Field],
    encode: fn(&T) -> Vec<u8>,
) {
    let size = T::SIZE;
    let mask = layout_mask(layout, size);
    // A pattern and its complement: every listed bit is 1 in one of them.
    for invert in [false, true] {
        let mut b: Vec<u8> = (0..size)
            .map(|i| (i as u8).wrapping_mul(37).wrapping_add(0x5A))
            .map(|x| if invert { !x } else { x })
            .collect();
        b[0] = T::ID;
        let m = T::decode(&b).unwrap_or_else(|e| panic!("0x{:02X}: {e}", T::ID));
        let want: Vec<u8> = b
            .iter()
            .zip(&mask)
            .enumerate()
            .map(|(i, (x, k))| if i == 0 { *x } else { x & k })
            .collect();
        assert_eq!(encode(&m), want, "0x{:02X} encode", T::ID);
        let mut out = vec![0xEE; size];
        m.write(&mut out);
        assert_eq!(out, want, "0x{:02X} write", T::ID);
        assert_eq!(T::decode(&want).unwrap(), m, "0x{:02X} re-decode", T::ID);
        if mask.iter().any(|&k| k != 0) {
            assert_ne!(m, T::default(), "0x{:02X} decodes its fields", T::ID);
        }
    }
    // A fixed message must be exactly its size, and start with its id byte.
    assert_eq!(T::decode(&[]), Err(DecodeError::Empty));
    let mut b = vec![0u8; size];
    b[0] = T::ID ^ 0xFF;
    assert_eq!(
        T::decode(&b),
        Err(DecodeError::WrongId {
            expected: T::ID,
            found: T::ID ^ 0xFF
        })
    );
    b[0] = T::ID;
    for len in [size - 1, size + 1] {
        if len == 0 {
            continue;
        }
        b.resize(len, 0);
        assert_eq!(
            T::decode(&b),
            Err(DecodeError::WrongSize {
                expected: size,
                found: len
            }),
            "0x{:02X} size {len}",
            T::ID
        );
    }
}

macro_rules! all {
    ($table:ident, $module:ident: $($t:ident),* $(,)?) => {
        $(
            {
                use crate::generated::$module::$t;
                let d = $table
                    .iter()
                    .find(|d| d.id == <$t as FixedMessage>::ID)
                    .expect("descriptor");
                assert_eq!(d.name, stringify!($t));
                check::<$t>(d.layout, |m| m.encode().to_vec());
            }
        )*
    };
}

// Every typed message keeps exactly the bits its layout lists.
#[test]
fn every_typed_message_follows_its_layout() {
    all!(CLIENT_MESSAGES, client:
        Walk, WalkToUnit, Run, RunToUnit, ShiftLeftSkill, LeftSkillOnUnit,
        ShiftLeftSkillOnUnit, ShiftLeftSkillHold, LeftSkillOnUnitHold,
        ShiftLeftSkillOnUnitHold, Unused0B, RightSkill, RightSkillOnUnit,
        ShiftRightSkillOnUnit, RightSkillHold, RightSkillOnUnitHold,
        ShiftRightSkillOnUnitHold, EndInferno, InteractWithEntity, PickItem,
        DropItem, InsertItemInBuffer, RemoveItemFromBuffer, EquipItem,
        Swap2HandedItem, RemoveBodyItem, SwapCursorWithBody, Swap1HWith2H,
        SwapCursorBufferItem, UseGridItem, StackItems, UnstackItems,
        ItemToBelt, ItemFromBelt, SwitchBeltItem, UseBeltItem, UseItemAction,
        SocketItem, ScrollToBook, ItemToCube, Unused2D, InitEntityChat,
        TerminateEntityChat, QuestMessage, BuyItem, SellItem, IdentifyWithNpc,
        Repair, HireMerc, IdentifyGamble, EntityAction, AddStatPoint,
        AddSkillPoint, SelectSkill, HighlightDoor, ActivateInifussScroll,
        PlayAudio, RequestQuestData, Resurrect, Unused42, Unused43,
        StaffInOrifice, MercInteract, MoveMerc, TurnOffBusyState,
        TakeOrCloseWp, RequestEntityUpdate, Transmogrify, PlayNpcMessage,
        ClickButton, DropGold, BindHotkey, StaminaOn, StaminaOff,
        QuestCompleted, MakeEntityMove, SquelchHostile, PartyAction,
        UpdatePlayerPos, SwapWeapons, MercItem, ResurrectMerc,
        ItemToBeltShift, CreateGame, JoinGame, LeaveGame, Sys6A, Sys6B, Ping,
        Sys6E, Sys70,
    );
    all!(SERVER_MESSAGES, server:
        GameLoading, GameFlags, LoadSuccessful, LoadAct, LoadComplete,
        UnloadComplete, GameExit, MapReveal, MonsterHit, PlayerStop,
        PlayerMove, PlayerToTarget, ReassignPlayer, SmallGoldPickup,
        AddExpByte, AddExpWord, AddExpDword, SetStatByte, SetStatWord,
        SetStatDword, UseStackableItem, ClearCursor, Relator1, Relator2,
        StartMercList, PortalFlags, Unknown6E, Unknown6F, Unknown70,
        Unknown71, Unknown72, SetItemState, WalkVerify, WeaponSwitch,
        ConnectionTerminated,
    );
}

/// A size rule needs at least `min` bytes **and** the field's bytes; the TSV
/// rules all have `min` covering the field, so this
/// uses synthetic rules.
#[test]
fn field_rule_needs_the_field_bytes() {
    use crate::schema::{Size, SizeRule, Width};
    let rule = |width| SizeRule::Field {
        width,
        offset: 4,
        mul: 1,
        add: 0,
        cap: None,
        min: 0,
    };
    let u16_rule = rule(Width::U16);
    assert_eq!(u16_rule.eval(&[9, 0, 0, 0, 7]), Size::Incomplete);
    assert_eq!(u16_rule.eval(&[9, 0, 0, 0, 7, 1]), Size::Bytes(0x107));
    let u8_rule = rule(Width::U8);
    assert_eq!(u8_rule.eval(&[9, 0, 0, 0]), Size::Incomplete);
    assert_eq!(u8_rule.eval(&[9, 0, 0, 0, 7]), Size::Bytes(7));
}

/// Entry 0 is "never valid", not a size.
#[test]
fn fixed_zero_is_not_a_size() {
    use crate::schema::SizeRule;
    assert_eq!(SizeRule::Fixed(0).fixed(), None);
    assert!(SizeRule::Fixed(0).is_never());
    assert_eq!(SizeRule::Fixed(5).fixed(), Some(5));
}

/// The chat rule's size L1 + L2 + 6 + c with c a **signed** byte can be
/// negative; the classifier passes it on instead of calling it incomplete
/// (what the caller does with it is its own decision).
#[test]
fn chat_negative_size_reaches_the_caller() {
    use crate::transport::{classify_client, Classified};
    // L1 = 1 ("a"), L2 = 0, c = −10: 1 + 0 + 6 − 10 = −3.
    let b = [0x15, 1, 0, b'a', 0, 0, 0xF6];
    assert_eq!(classify_client(&b), Classified::NegativeSize(-3));
}

/// Local delivery split: a message ending exactly at the
/// buffer's end is kept; a message of exactly 0x204 bytes is accepted,
/// 0x205 is refused as too large.
#[test]
fn split_keeps_messages_up_to_the_limits() {
    use crate::transport::{split_server_buffer, Split, SplitError, MAX_MESSAGE};
    let buf = [0xAF, 0x00, 0xAF, 0x02, 9];
    assert_eq!(
        split_server_buffer(&buf),
        Ok(Split {
            messages: vec![&buf[..2], &buf[2..]],
            discarded: &[],
        })
    );
    // 0x16: size = u16 at +1.
    let mut m16 = vec![0u8; MAX_MESSAGE];
    m16[..3].copy_from_slice(&[0x16, 0x04, 0x02]);
    let s = split_server_buffer(&m16).unwrap();
    assert_eq!(s.messages, [&m16[..]]);
    m16[1] = 0x05;
    m16.push(0);
    assert_eq!(
        split_server_buffer(&m16),
        Err(SplitError::TooLarge { at: 0, size: 0x205 })
    );
}

/// Typed S→C decode: an empty buffer and a wrong id are their own
/// errors; `parse` takes only a message exactly as long as its size rule
/// gives.
#[test]
fn server_msg_decode_and_parse_errors() {
    use crate::s2c::{parse, ParseError, PlayerStop, ServerMsg};
    assert_eq!(PlayerStop::decode(&[]), Err(ParseError::Empty));
    let mut b = [0u8; 13];
    b[0] = 0x0E;
    assert_eq!(
        PlayerStop::decode(&b),
        Err(ParseError::WrongId {
            expected: 0x0D,
            found: 0x0E
        })
    );
    b[0] = 0x0D;
    assert!(PlayerStop::decode(&b).is_ok());
    assert!(parse(&b).is_ok());
    let mut long = b.to_vec();
    long.push(0);
    assert_eq!(
        parse(&long).unwrap_err(),
        ParseError::WrongSize {
            id: 0x0D,
            expected: 13,
            found: 14
        }
    );
}

/// A variable-size S→C message longer than its size rule (0xAE: u16 at
/// +1, + 3) is a wrong size.
#[test]
fn parse_rejects_bytes_past_a_variable_size() {
    use crate::s2c::{parse, ParseError};
    assert!(parse(&[0xAE, 1, 0, 7]).is_ok());
    assert_eq!(
        parse(&[0xAE, 1, 0, 7, 9]).unwrap_err(),
        ParseError::WrongSize {
            id: 0xAE,
            expected: 4,
            found: 5
        }
    );
}
