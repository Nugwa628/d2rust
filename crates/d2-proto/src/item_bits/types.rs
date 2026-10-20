//! The data the item reader works with: the table facts it asks for
//! ([`ItemLookup`]), its errors, and the decoded record ([`ItemBits`],
//! [`SaveEntry`]).

use super::hflag;

/// The itemstatcost save columns of one stat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IscSave {
    /// `Save Bits`: width of the stat value on the wire.
    pub save_bits: u8,
    /// `Save Add`: offset added to the value before it is written.
    pub save_add: u32,
    /// `Save Param Bits`: width of the stat's parameter (0: none).
    pub save_param_bits: u32,
}

/// What the tables say about an item code: the type tests (with type
/// equivalence) and the items / itemtypes columns the reader needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeFacts {
    /// Type 50 `armo`.
    pub armor: bool,
    /// Type 45 `weap`.
    pub weapon: bool,
    /// Type 4 `gold`.
    pub gold: bool,
    /// Type 13 `char`.
    pub charm: bool,
    /// Type 40 `body` and not type 7 `play`.
    pub body_part: bool,
    /// Type 22 `scro` or 18 `book`.
    pub scroll_or_book: bool,
    /// Items `stackable` ≠ 0.
    pub stackable: bool,
    /// The primary type's `varinvgfx` ≠ 0.
    pub varinvgfx: bool,
    /// Items `quest` ≠ 0 and `questdiffcheck` ≠ 0.
    pub quest_diff: bool,
}

/// The tables the reader needs.
pub trait ItemLookup {
    /// Facts of an item code; `None`: unknown code (a decode error).
    fn code(&self, code: [u8; 4]) -> Option<CodeFacts>;
    /// Save columns of a stat; `None`: no row (the list ends there, with no
    /// failure; what follows is then misread).
    fn isc(&self, stat: u16) -> Option<IscSave>;
    /// The `setitems` row count (a set record naming no row fails);
    /// `None`: not known, no test.
    fn set_item_rows(&self) -> Option<usize> {
        None
    }
}

/// Why a stream did not decode.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ItemBitsError {
    /// The stream ends at this bit before the record does.
    #[error("stream ends at bit {0}")]
    Short(usize),
    /// The item code is not in the items table.
    #[error("unknown item code {0:?}")]
    UnknownCode([u8; 4]),
    /// A stat the reader must read has no `itemstatcost` row.
    #[error("stat {0} has no itemstatcost row")]
    UnknownStat(u16),
    /// This many bits remain after the record (more than the padding).
    #[error("{0} bits left after the record (more than the padding)")]
    Trailing(usize),
    /// A padding bit (at this position) is not zero.
    #[error("padding bit {0} is set")]
    Padding(usize),
    /// Save format: the 16 bits before the flags are not 0x4D4A.
    #[error("save marker is {0:#06x}, not 0x4D4A (\"JM\")")]
    BadMarker(u32),
}

/// Where the item is, by mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    /// Modes 3 (on the ground) and 5 (being dropped): sub-tile position.
    Ground { x: u16, y: u16 },
    /// Other modes: body location, grid / slot x and y, page + 1.
    Slot { body: u8, x: u8, y: u8, page1: u8 },
}

/// Ear fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ear {
    /// Class of the player the ear came from (3 bits).
    pub class: u8,
    /// Level of that player (7 bits).
    pub level: u8,
    /// Their name, 7 bits per character, as sent.
    pub name: Vec<u8>,
}

/// A stat on the wire: id, param, the written value and its `Save Add`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    /// Stat id.
    pub stat: u16,
    /// Stat parameter (0 when the stat has none).
    pub param: u32,
    /// As written (clamped, `Save Add` included).
    pub raw: u32,
    /// The `Save Add` of the stat's row.
    pub save_add: u32,
}

impl Stat {
    /// The value without `Save Add` (the list value >> `ValShift`).
    pub fn value(&self) -> i64 {
        i64::from(self.raw) - i64::from(self.save_add)
    }
}

/// The quality-dependent fields (sent for shown items).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QualityFields {
    /// Qualities 1, 3 (3 bits), 5, 7 (12 bits); type `body` (10 bits).
    pub file_index: Option<u32>,
    /// Magic: prefix and suffix slot 0 as sent.
    pub magic: Option<(u16, u16)>,
    /// Rare, crafted, tempered names.
    pub rare_names: Option<(u8, u8)>,
    /// Rare / crafted slots: (prefix i, suffix i) as sent, 0 = none.
    pub rare_slots: Option<[(u16, u16); 3]>,
    /// Charm affix (1 bit: is a prefix; 11 bits id).
    pub charm: Option<(bool, u16)>,
    /// Scroll / book: suffix slot 0 (5 bits).
    pub spell: Option<u8>,
}

/// One decoded record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemBits {
    /// Header flags (see [`hflag`]).
    pub flags: u32,
    /// Item version field (10 bits).
    pub version: u16,
    /// Item mode (3 bits): 3 and 5 are on the ground, the rest in a slot.
    pub mode: u8,
    /// Where the item is.
    pub location: Option<Location>,
    /// Alt-code record: the base code, and nothing after it.
    pub base_code: Option<[u8; 4]>,
    /// The item code (four characters).
    pub code: [u8; 4],
    /// Ear fields, for an ear record.
    pub ear: Option<Ear>,
    /// Personalized name (characters before the 0).
    pub name: Option<Vec<u8>>,
    /// Gold (compact gold, or type `gold`).
    pub gold: Option<u32>,
    /// `questitemdifficulty` (compact).
    pub quest_diff: Option<Stat>,
    // ---- full record
    /// Number of socketed children (3 bits).
    pub filled: u8,
    /// Item level (7 bits).
    pub ilvl: u8,
    /// Quality (4 bits).
    pub quality: u8,
    /// Picture variant (3 bits), when sent.
    pub gfx: Option<u8>,
    /// Automatic affix id (11 bits), when sent.
    pub auto_affix: Option<u16>,
    /// Fields that depend on the quality.
    pub quality_fields: QualityFields,
    /// Runeword id, for a runeword.
    pub runeword: Option<u16>,
    /// Defense (armor only).
    pub defense: Option<Stat>,
    /// Maximum durability (armor and weapons).
    pub max_durability: Option<Stat>,
    /// Current durability (sent when the maximum is not 0).
    pub durability: Option<Stat>,
    /// Stack size (9 bits), for stackable items.
    pub quantity: Option<u16>,
    /// Socket count, for socketed items.
    pub sockets: Option<u8>,
    /// Set mask (quality 5, shown).
    pub set_mask: Option<u8>,
    /// Lists c = −1, 0, …, L − 1 in stream order: `None` for a slot
    /// without its terminator (no list, no runeword).
    pub lists: Vec<Option<Vec<Stat>>>,
    /// Bits used (the stream is these, padded to whole bytes).
    pub bits: usize,
    // ---- save format
    /// Decoded from a save stream: "shown" always.
    pub save: bool,
    /// Full record, save format: an extra 32-bit unit field that only the
    /// save carries, kept as read.
    pub save_unit28: Option<u32>,
    /// Trailer, when its bit is 1: the two 32-bit values (the third is read
    /// and dropped). `None`: bit 0, or no trailer (network, alt-code).
    pub save_trailer: Option<(u32, u32)>,
    /// The record was read to where the reader stops but failed (an
    /// unsupported quality, a set item with no `setitems` row, or a stat id 0
    /// directly after id 0): it makes no item.
    pub failed: bool,
}

impl ItemBits {
    /// "Shown": the quality fields and stat lists are present. Always in the
    /// save format, otherwise only for identified items.
    pub fn shown(&self) -> bool {
        self.save || self.flags & hflag::IDENTIFIED != 0
    }
}

/// One save-format item entry: a record, its padding and its children.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SaveEntry {
    /// Bytes used: the item's padded stream plus its children's entries.
    pub len: usize,
    /// The item's own record.
    pub item: ItemBits,
    /// The socketed children, `item.filled` of them, in stream order.
    pub children: Vec<SaveEntry>,
}
