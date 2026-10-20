//! Client-side parse of one delivered S→C message: the size comes from the
//! S→C size rule, then the id's typed decode runs. Pure (no I/O).
//!
//! The client feeds it the whole messages that `split_server_buffer` yields;
//! the result is a [`Message`] for the client's handlers.

use super::messages::*;
use super::{audit, ServerMsg, Status};
use crate::generated::server as gen;
use crate::schema::Size;
use crate::transport::server_size;
use crate::wire::{DecodeError, FixedMessage};

/// Why a message did not parse.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    /// No bytes at all.
    #[error("message is empty")]
    Empty,
    /// The first byte is not a valid S→C id (the size rule gives 0).
    #[error("id 0x{id:02X}: not a valid S→C message (size rule gives 0)")]
    Invalid { id: u8 },
    /// The buffer ends before the size rule can give a length.
    #[error("id 0x{id:02X}: too few bytes for its size rule")]
    Incomplete { id: u8 },
    /// The length differs from the size the rule gives.
    #[error("id 0x{id:02X}: {found} bytes, size rule gives {expected}")]
    WrongSize {
        id: u8,
        expected: usize,
        found: usize,
    },
    /// The first byte is not the id the decoder expects.
    #[error("id 0x{found:02X}, expected 0x{expected:02X}")]
    WrongId { expected: u8, found: u8 },
    /// A byte the layout fixes has another value.
    #[error("id 0x{id:02X}: byte {offset} is 0x{found:02X}, the layout fixes 0x{expected:02X}")]
    Const {
        id: u8,
        offset: usize,
        expected: u8,
        found: u8,
    },
    /// A valid message whose layout is not fully known.
    #[error("id 0x{id:02X}: no full layout in the specs ({status:?})")]
    Unbuilt { id: u8, status: Status },
}

/// The size the S→C size rule gives for `b`, which must be exactly that long.
pub(crate) fn expected_size(b: &[u8]) -> Result<usize, ParseError> {
    let id = *b.first().ok_or(ParseError::Empty)?;
    match server_size(b) {
        Size::Bytes(n) if n == b.len() => Ok(n),
        Size::Bytes(n) => Err(ParseError::WrongSize {
            id,
            expected: n,
            found: b.len(),
        }),
        Size::Incomplete => Err(ParseError::Incomplete { id }),
        Size::Invalid | Size::Negative(_) => Err(ParseError::Invalid { id }),
    }
}

/// Decodes a generated fixed layout, its [`DecodeError`] turned into the
/// matching [`ParseError`].
fn generated<M: FixedMessage>(b: &[u8]) -> Result<M, ParseError> {
    M::decode(b).map_err(|e| match e {
        DecodeError::Empty => ParseError::Empty,
        DecodeError::WrongId { expected, found } => ParseError::WrongId { expected, found },
        DecodeError::WrongSize { expected, found } => ParseError::WrongSize {
            id: M::ID,
            expected,
            found,
        },
    })
}

/// Builds [`Message`], [`parse`] and the test list of parsed ids from one
/// id list per source: `generated` ids decode through their TSV layout,
/// `built` ids through their hand-written [`ServerMsg`]. 0x50 and 0xAE have
/// their own arms.
macro_rules! messages {
    (
        generated: [ $( $gid:literal $g:ident ),* $(,)? ]
        built: [ $( $bid:literal $b:ident ),* $(,)? ]
    ) => {
        /// One parsed S→C message.
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub enum Message {
            $( $g(gen::$g), )*
            $( $b($b), )*
            WardenRequest(WardenRequest),
            /// 0x50 in any form [`QuestSpecial`] does not take (a code
            /// other than 1, or bytes 9–14 not zero): the TSV layout,
            /// code u16@1 and five words (`server-messages.tsv`; quest
            /// codes 4, 13, 23, mercenary code 2).
            QuestSpecialForm(gen::QuestSpecial),
        }

        /// Parses one whole S→C message (as `split_server_buffer` yields
        /// it). Ids whose layout is not fully known return
        /// [`ParseError::Unbuilt`].
        pub fn parse(b: &[u8]) -> Result<Message, ParseError> {
            expected_size(b)?;
            let id = b[0];
            // 0x50: the code-1 view when it decodes, else every other
            // form through the TSV layout.
            if id == QuestSpecial::ID {
                if let Ok(m) = <QuestSpecial as ServerMsg>::decode(b) {
                    return Ok(Message::QuestSpecial(m));
                }
                return Ok(Message::QuestSpecialForm(generated::<gen::QuestSpecial>(b)?));
            }
            Ok(match id {
                $( $gid => Message::$g(generated::<gen::$g>(b)?), )*
                $( $bid => Message::$b(<$b as ServerMsg>::decode(b)?), )*
                0xAE => Message::WardenRequest(WardenRequest::decode(b)?),
                _ => return Err(ParseError::Unbuilt { id, status: audit(id).status }),
            })
        }

        /// Ids [`parse`] decodes.
        #[cfg(test)]
        pub(crate) const PARSED_IDS: &[u8] = &[ $( $gid, )* $( $bid, )* 0xAE ];
    };
}

messages! {
    generated: [
        0x00 GameLoading, 0x01 GameFlags, 0x02 LoadSuccessful, 0x03 LoadAct, 0x04 LoadComplete,
        0x05 UnloadComplete, 0x06 GameExit, 0x07 MapReveal, 0x08 MapHide, 0x09 AssignLevelWarp,
        0x0A RemoveUnit, 0x0C MonsterHit, 0x0E ObjectState, 0x0F PlayerMove,
        0x10 PlayerToTarget, 0x11 ReportKill, 0x15 ReassignPlayer, 0x18 LifeManaUpdate,
        0x19 SmallGoldPickup, 0x1A AddExpByte, 0x1B AddExpWord, 0x1C AddExpDword,
        0x1D SetStatByte, 0x1E SetStatWord, 0x1F SetStatDword, 0x20 StatUpdate,
        0x21 UpdateItemOSkill, 0x22 UpdateItemSkill, 0x23 SetSkill, 0x27 NpcInfo,
        0x2C PlaySound, 0x3F UseStackableItem, 0x40 ItemFlags, 0x42 ClearCursor, 0x47 Relator1,
        0x48 Relator2, 0x4C UnitSkillOnUnit, 0x4D UnitSkillOnPoint, 0x4F StartMercList,
        0x51 AssignObject, 0x53 Darkness, 0x57 NpcEnchants, 0x5A EventMessage, 0x5C PlayerLeft,
        0x5E GameQuestAvailability, 0x5F PortalFlags, 0x60 TownPortalState, 0x61 CanGoToAct,
        0x62 MakeUnitTargetable, 0x65 PlayerKillCount, 0x67 MonsterMove,
        0x68 MonsterMoveToTarget, 0x69 MonsterState, 0x6A Unknown6A, 0x6B MonsterAction,
        0x6C MonsterAttack, 0x6D MonsterStop, 0x6E Unknown6E, 0x6F Unknown6F, 0x70 Unknown70,
        0x71 Unknown71, 0x72 Unknown72, 0x73 Unknown73, 0x74 PlayerCorpseAssign,
        0x75 PlayerPartyInfo, 0x76 PlayerInProximity, 0x78 TradeAccepted, 0x79 GoldInTrade,
        0x7A PetAction, 0x7B AssignHotkey, 0x7C UseScroll, 0x7D SetItemState,
        0x7F AllyPartyInfo, 0x81 AssignMerc, 0x82 PortalOwnership, 0x8B PlayerRelationship,
        0x8C RelationshipUpdate, 0x8D AssignPlayerToParty, 0x8E CorpseAssign, 0x8F Pong,
        0x90 PartyAutomapInfo, 0x92 RemoveItemsDisplay, 0x93 Unknown93, 0x95 LifeManaUpdate2,
        0x96 WalkVerify, 0x97 WeaponSwitch, 0x98 Unknown98, 0x99 SkillTriggered, 0x9A Unknown9A,
        0x9E MercStatByte, 0x9F MercStatWord, 0xA0 MercStatDword, 0xA1 MercAddExpByte,
        0xA2 MercAddExpWord, 0xA3 UnknownA3, 0xA4 BaalWave, 0xA5 UnknownA5, 0xA7 DelayedState,
        0xA9 EndState, 0xAB NpcHeal, 0xB0 ConnectionTerminated, 0xB2 GameList,
        0xB4 ConnectionRefused, 0xB6 ViewReady,
    ]
    built: [
        0x0B GameHandshake, 0x0D PlayerStop, 0x28 QuestInfo, 0x29 GameQuestInfo,
        0x2A NpcTransaction, 0x4E MercForHire, 0x50 QuestSpecial, 0x52 QuestLogInfo,
        0x58 OpenUi, 0x59 AssignPlayer, 0x5D QuestItemState, 0x63 WaypointMenu, 0x77 TradeAction, 0x89 UniqueEvent,
        0x8A NpcWantsInteract, 0x91 NpcGossipAct, 0x9B Unknown9B,
    ]
}
