//! The hand-written S→C messages (NPC, quest, waypoint, cube and player
//! join messages) built with `s2c_message!`, plus the variable-size 0xAE.
//! Their layouts come from the game systems that send them rather than
//! from the TSV `layout` column. Names are the `server-messages.tsv`
//! labels; fields without a known meaning are named by offset (`f6`), the
//! same convention the generator uses.
//!
//! Each field's doc gives its wire type and offset, then its meaning.
//! Re-exported by [`super`]; [`super::parse`] decodes them into
//! [`super::Message`] and the client bridge's tests build them as input.

use super::s2c_message;

/// Size of a quest record.
pub const QUEST_RECORD: usize = 96;
/// Size of a waypoint record.
pub const WAYPOINT_RECORD: usize = 16;
/// Entries of the quest log list.
pub const QUEST_LOG_ENTRIES: usize = 41;
/// NPC slots of 0x91.
pub const GOSSIP_SLOTS: usize = 12;

s2c_message! {
    /// 0x0B GameHandshake (6 bytes): the unit's type and GUID. At a
    /// single-player join it names the joining player.
    0x0B GameHandshake 6 {
        /// `u8` at 1.
        unit_type: u8 = 1,
        /// `u32` at 2.
        unit_guid: u32 = 2,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x0D PlayerStop (13 bytes): unit type, GUID, 1, x, y, 0, 0 on a
    /// waypoint arrival, for example
    /// `0d 00 01000000 01 2013 8413 00 00` (x 0x1320, y 0x1384).
    0x0D PlayerStop 13 {
        /// `u8` at 1.
        unit_type: u8 = 1,
        /// `u32` at 2.
        unit_guid: u32 = 2,
        /// `u8` at 6 (1 on waypoint arrival).
        f6: u8 = 6,
        /// `u16` at 7.
        x: u16 = 7,
        /// `u16` at 9.
        y: u16 = 9,
        /// `u8` at 11 (0 on waypoint arrival).
        f11: u8 = 11,
        /// `u8` at 12 (0 on waypoint arrival).
        f12: u8 = 12,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x28 QuestInfo (103 bytes): unit, a 0 byte, the player's quest
    /// record for the current difficulty.
    0x28 QuestInfo 103 {
        /// `u8` at 1: 1 (the NPC talked to) or 6 (quest code).
        unit_type: u8 = 1,
        /// `u32` at 2: the NPC's GUID, or 0 with type 6.
        unit_guid: u32 = 2,
        /// 96 bytes at 7.
        record: [u8; QUEST_RECORD] = 7,
    }
    consts [6 => 0]
    unwritten []
}

s2c_message! {
    /// 0x29 GameQuestInfo (97 bytes): the game quest record.
    0x29 GameQuestInfo 97 {
        /// 96 bytes at 1.
        record: [u8; QUEST_RECORD] = 1,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x2A NpcTransaction (15 bytes): the result of a buy, sell, repair
    /// or hire request. Bytes 3–6 carry no value.
    0x2A NpcTransaction 15 {
        /// `u8` at 1.
        kind: u8 = 1,
        /// `u8` at 2: result code.
        code: u8 = 2,
        /// `u32` at 7: item or mercenary GUID, or −1.
        guid: u32 = 7,
        /// `u32` at 11: player gold (stat 14) after the transaction.
        gold: u32 = 11,
    }
    consts []
    unwritten [3, 4, 5, 6]
}

s2c_message! {
    /// 0x4E MercForHire (7 bytes): one hire-list offer.
    0x4E MercForHire 7 {
        /// `u16` at 1: name id.
        name: u16 = 1,
        /// `u32` at 3: slot seed.
        seed: u32 = 3,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x50 QuestSpecial (15 bytes), the quest form only (u16 1 at 1).
    /// The mercenary form (u16 2 at 1) has no known bytes 5–14 and is not
    /// built here; [`super::parse`] reads it through the generated layout.
    0x50 QuestSpecial 15 {
        /// `u16` at 3: Den of Evil monsters left.
        den_left: u16 = 3,
        /// `i16` at 5: true tomb level id − 66.
        staff_tomb: i16 = 5,
        /// `u16` at 7: barbarians left.
        barbarians_left: u16 = 7,
    }
    consts [1 => 1, 2 => 0, 9 => 0, 10 => 0, 11 => 0, 12 => 0, 13 => 0, 14 => 0]
    unwritten []
}

s2c_message! {
    /// 0x52 QuestLogInfo (42 bytes): the quest log list, entries 0..40.
    0x52 QuestLogInfo 42 {
        /// 41 bytes at 1.
        list: [u8; QUEST_LOG_ENTRIES] = 1,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x58 OpenUi (7 bytes): NPC service or object-insert result. Byte 6
    /// (`effect`) is meaningful only for result 5; for every other code it
    /// carries no value, D2RUST writes 0, and a byte comparison masks it
    /// for those codes.
    0x58 OpenUi 7 {
        /// `u32` at 1.
        npc_guid: u32 = 1,
        /// `u8` at 5: 6 done, 7 refused; 0, 1, 4, 5 object inserts.
        result: u8 = 5,
        /// `u8` at 6: "accepted with effect" (result 5 only).
        effect: u8 = 6,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x59 AssignPlayer (26 bytes): the first of the messages that add a
    /// player to the client: GUID, class (u8), name (from the player
    /// data), and x, y (the unit's position).
    0x59 AssignPlayer 26 {
        /// `u32` at 1.
        guid: u32 = 1,
        /// `u8` at 5.
        class: u8 = 5,
        /// 16 bytes at 6 (zero-padded).
        name: [u8; 16] = 6,
        /// `u16` at 0x16.
        x: u16 = 0x16,
        /// `u16` at 0x18.
        y: u16 = 0x18,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x5D QuestItemState (6 bytes): one quest's status.
    0x5D QuestItemState 6 {
        /// `u8` at 1.
        chain: u8 = 1,
        /// `u8` at 2: record flags (+0x14).
        flags: u8 = 2,
        /// `u8` at 3.
        status: u8 = 3,
        /// `u16` at 4: monsters / barbarians left, else 0.
        extra: u16 = 4,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x63 WaypointMenu (21 bytes): opens the waypoint menu with the
    /// player's waypoint record.
    0x63 WaypointMenu 21 {
        /// `u32` at 1: waypoint object GUID.
        object_guid: u32 = 1,
        /// 16 bytes at 5: the player's record for the game difficulty.
        record: [u8; WAYPOINT_RECORD] = 5,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x77 TradeAction (2 bytes): trade and cube result; the cube uses
    /// the actions 0x0C, 0x11 and 0x15.
    0x77 TradeAction 2 {
        /// `u8` at 1.
        action: u8 = 1,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x89 UniqueEvent (2 bytes): a one-off quest event.
    0x89 UniqueEvent 2 {
        /// `u8` at 1.
        event: u8 = 1,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x8A NpcWantsInteract (6 bytes): `8A 01 <npc GUID>`.
    0x8A NpcWantsInteract 6 {
        /// `u32` at 2.
        npc_guid: u32 = 2,
    }
    consts [1 => 1]
    unwritten []
}

s2c_message! {
    /// 0x91 NpcGossipAct (26 bytes): introduced NPC class ids packed at
    /// the front, the rest 0xFFFF.
    0x91 NpcGossipAct 26 {
        /// `u8` at 1: the act.
        act: u8 = 1,
        /// 12 × `u16` at 2.
        slots: [u16; GOSSIP_SLOTS] = 2,
    }
    consts []
    unwritten []
}

s2c_message! {
    /// 0x9B Unknown9B (7 bytes): sent after a mercenary resurrection with
    /// u16 0xFFFF at 1 and u32 0 at 3.
    0x9B Unknown9B 7 {
        /// `u16` at 1.
        f1: u16 = 1,
        /// `u32` at 3.
        f3: u32 = 3,
    }
    consts []
    unwritten []
}

/// Largest 0xAE payload: with a longer length the size rule does not give
/// `3 + len`, so such a message cannot be sent.
pub const WARDEN_MAX: usize = 0x1FD;

/// 0xAE WardenRequest (variable: `len` u16 at 1, `len` bytes at 3, the
/// size rule of `server-messages.tsv`). A transport message; the payload
/// is opaque.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WardenRequest {
    /// The `len` payload bytes.
    pub data: Vec<u8>,
}

impl WardenRequest {
    /// The message id.
    pub const ID: u8 = 0xAE;

    /// The message bytes. Panics if `data` is longer than [`WARDEN_MAX`].
    pub fn encode(&self) -> Vec<u8> {
        assert!(self.data.len() <= WARDEN_MAX, "0xAE payload too long");
        let mut b = Vec::with_capacity(3 + self.data.len());
        b.push(Self::ID);
        b.extend_from_slice(&(self.data.len() as u16).to_le_bytes());
        b.extend_from_slice(&self.data);
        b
    }

    /// Decodes one whole message (size from the S→C size rule).
    pub fn decode(b: &[u8]) -> Result<Self, super::ParseError> {
        let expected = super::parse::expected_size(b)?;
        if b[0] != Self::ID {
            return Err(super::ParseError::WrongId {
                expected: Self::ID,
                found: b[0],
            });
        }
        Ok(Self {
            data: b[3..expected].to_vec(),
        })
    }
}
