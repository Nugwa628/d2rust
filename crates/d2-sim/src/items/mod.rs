//! Item creation: the drop request, the item, the creation pipeline and
//! base stats ([`create`]), the quality roll and dispatch ([`quality`]),
//! magic/rare/crafted affixes ([`affixes`]) and properties to stats
//! ([`props`]). Table data is a projection of `d2-data` typed records
//! ([`tables::ItemTables`]); a mod game's item overrides are in
//! [`provider`].
//!
//! Two traits connect item code to the rest of the game:
//! - [`ItemStats`]: the item's (or an owner's) stats and stat lists.
//! - [`ItemGame`]: game fields read or written during creation (game seed,
//!   difficulty, expansion, ladder flags, unique-dropped bits).

pub mod affixes;
pub mod bitstream;
pub mod create;
pub mod inventory;
pub mod moves;
pub mod props;
pub mod props_legacy;
pub mod provider;
pub mod quality;
pub mod recharge;
pub mod set_state;
pub mod tables;

#[cfg(any(test, feature = "bench-fixtures"))]
#[cfg_attr(not(test), allow(unused, dead_code))]
pub(crate) mod tests;

use crate::rng::Seed;

pub use create::{
    create_item, init_item_stats, replenish_timer, replenish_timer_at, CreateError, Created,
};
pub use tables::ItemTables;

/// Quality ids.
pub mod q {
    pub const NONE: u8 = 0;
    pub const LOW: u8 = 1;
    pub const NORMAL: u8 = 2;
    pub const SUPERIOR: u8 = 3;
    pub const MAGIC: u8 = 4;
    pub const SET: u8 = 5;
    pub const RARE: u8 = 6;
    pub const UNIQUE: u8 = 7;
    pub const CRAFTED: u8 = 8;
    pub const TEMPERED: u8 = 9;
}

/// Item flags.
pub mod flag {
    pub const IDENTIFIED: u32 = 0x10;
    pub const BROKEN: u32 = 0x100;
    pub const SOCKETED: u32 = 0x800;
    pub const NOSELL: u32 = 0x1000;
    pub const INSTORE: u32 = 0x2000;
    pub const NOEQUIP: u32 = 0x4000;
    pub const NAMED: u32 = 0x8000;
    pub const EAR: u32 = 0x10000;
    pub const STARTITEM: u32 = 0x20000;
    pub const INIT: u32 = 0x80000;
    pub const ETHEREAL: u32 = 0x400000;
    pub const PERSONALIZED: u32 = 0x1000000;
    pub const RUNEWORD: u32 = 0x4000000;
}

/// Request flags, flags2.
pub mod req {
    pub const HELLBOVINE: u32 = 0x01;
    pub const NEVER_ETHEREAL: u32 = 0x02;
    pub const ALWAYS_ETHEREAL: u32 = 0x04;
    pub const NO_SOCKETS: u32 = 0x08;
    pub const ALWAYS_SOCKETS: u32 = 0x10;
    pub const STAFFMODS_ILVL: u32 = 0x20;
    pub const SUPERIOR: u32 = 0x40;
}

/// Item type numbers: itemtypes row indices.
pub mod ty {
    pub const SHIE: u16 = 2;
    pub const TORS: u16 = 3;
    pub const GOLD: u16 = 4;
    pub const PLAY: u16 = 7;
    pub const ELIX: u16 = 11;
    pub const CHAR: u16 = 13;
    pub const BOOT: u16 = 15;
    pub const GLOV: u16 = 16;
    pub const BOOK: u16 = 18;
    pub const BELT: u16 = 19;
    pub const GEM: u16 = 20;
    pub const SCRO: u16 = 22;
    pub const SCEP: u16 = 24;
    pub const WAND: u16 = 25;
    pub const STAF: u16 = 26;
    pub const BOW: u16 = 27;
    pub const XBOW: u16 = 35;
    pub const HELM: u16 = 37;
    pub const BODY: u16 = 40;
    pub const WEAP: u16 = 45;
    pub const ARMO: u16 = 50;
    pub const MISC: u16 = 52;
    pub const JEWL: u16 = 58;
    pub const RUNE: u16 = 74;
}

/// Stat ids written or read here.
pub mod stat {
    pub const LEVEL: u16 = 12;
    pub const GOLD: u16 = 14;
    pub const ARMOR_PERCENT: u16 = 16;
    pub const MAXDAMAGE_PERCENT: u16 = 17;
    pub const MINDAMAGE_PERCENT: u16 = 18;
    pub const TOBLOCK: u16 = 20;
    pub const MINDAMAGE: u16 = 21;
    pub const MAXDAMAGE: u16 = 22;
    pub const SECONDARY_MINDAMAGE: u16 = 23;
    pub const SECONDARY_MAXDAMAGE: u16 = 24;
    pub const ARMORCLASS: u16 = 31;
    pub const POISONMAXDAM: u16 = 58;
    pub const VELOCITYPERCENT: u16 = 67;
    pub const ATTACKRATE: u16 = 68;
    pub const QUANTITY: u16 = 70;
    pub const VALUE: u16 = 71;
    pub const DURABILITY: u16 = 72;
    pub const MAXDURABILITY: u16 = 73;
    pub const ITEM_SINGLESKILL: u16 = 107;
    pub const INDESTRUCTIBLE: u16 = 152;
    pub const THROW_MINDAMAGE: u16 = 159;
    pub const THROW_MAXDAMAGE: u16 = 160;
    pub const NUMSOCKETS: u16 = 194;
    pub const REPLENISH_DURABILITY: u16 = 252;
    pub const REPLENISH_QUANTITY: u16 = 253;
    pub const EXTRA_STACK: u16 = 254;
    pub const POISON_COUNT: u16 = 326;
    pub const QUESTITEMDIFFICULTY: u16 = 356;
}

/// Item format of generated items in an expansion game.
pub const FORMAT_EXPANSION: u16 = 101;
/// Item format of generated items in a classic game.
pub const FORMAT_CLASSIC: u16 = 2;

/// Flags of the item's property stat list, where properties land.
pub const LIST_FLAGS: u32 = 0x40;

/// Which stat list of a unit: the list with
/// this state and flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListKey {
    pub state: u16,
    pub flags: u32,
}

impl ListKey {
    /// State 0, flags 0x40: the list affixes, qualities and uniques write.
    pub const ITEM: ListKey = ListKey {
        state: 0,
        flags: LIST_FLAGS,
    };
}

/// A unit's stats and stat lists, answered by the units' stat lists. A
/// stat is `(id, layer)` with a signed 32-bit value.
pub trait ItemStats {
    /// Whether the unit owns a stat list at all (the "has durability"
    /// test needs one).
    fn has_stats(&self) -> bool;
    /// Unit total of a stat.
    fn stat(&self, id: u16, layer: u16) -> i32;
    /// Unit base stat.
    fn base(&self, id: u16, layer: u16) -> i32;
    /// Sets the unit base stat.
    fn set_base(&mut self, id: u16, layer: u16, value: i32);
    /// Whether the list `key` exists.
    fn has_list(&self, key: ListKey) -> bool;
    /// Sets a stat in list `key`, creating the list if missing.
    fn list_set(&mut self, key: ListKey, id: u16, layer: u16, value: i32);
    /// Adds to a stat in list `key`, creating the list if missing.
    fn list_add(&mut self, key: ListKey, id: u16, layer: u16, value: i32);
    /// A stat's value in list `key` (0 when the list or stat is missing).
    fn list_get(&self, key: ListKey, id: u16, layer: u16) -> i32;
}

/// Bits of the game's unique-dropped array: 4,097 bits, one per unique
/// already dropped in this game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniqueBits(pub [u32; 129]);

impl Default for UniqueBits {
    fn default() -> Self {
        Self([0; 129])
    }
}

impl UniqueBits {
    /// Highest valid index.
    pub const MAX: u32 = 4096;
    /// Bit `i`; an index above 4096 reads as dropped.
    pub fn get(&self, i: u32) -> bool {
        i > Self::MAX || self.0[(i >> 5) as usize] & 1 << (i & 31) != 0
    }
    pub fn set(&mut self, i: u32) {
        self.0[(i >> 5) as usize] |= 1 << (i & 31);
    }
}

/// Game fields used by creation, answered by the game.
pub trait ItemGame {
    /// The game seed.
    fn seed(&mut self) -> &mut Seed;
    /// Game +0x6D: 0 normal, 1 nightmare, 2 hell.
    fn difficulty(&self) -> u8;
    /// Game +0x70: expansion game.
    fn expansion(&self) -> bool;
    /// Game +0x6A and +0x74 (ladder flags).
    fn ladder_flags(&self) -> (bool, bool);
    /// The game's tick rate: durations stated in base ticks are
    /// converted through it.
    /// No default, so no world runs at the base rate by accident.
    fn tick_rate(&self) -> crate::tick::TickRate;
    /// The unique-dropped bits.
    fn uniques(&mut self) -> &mut UniqueBits;
    /// The item format generated items take (game +0x78): 101 expansion,
    /// 2 classic.
    fn item_format(&self) -> u16 {
        if self.expansion() {
            FORMAT_EXPANSION
        } else {
            FORMAT_CLASSIC
        }
    }
}

/// The player data a request unit carries.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PlayerInfo {
    pub name: [u8; 16],
    /// Stat 12 (level).
    pub level: i32,
    /// The client's hardcore flag, when a client exists.
    pub hardcore: Option<bool>,
}

/// The request's source unit, staged by the caller: only the fields creation reads.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RequestUnit {
    /// Class id (player class or monster class).
    pub class: i32,
    /// Player data, for a player unit that has it.
    pub player: Option<PlayerInfo>,
}

/// The item drop request.
/// Allocation fields (spawn mode, position, room, init flags) belong to
/// the caller's allocation and are not here.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ItemRequest {
    pub unit: Option<RequestUnit>,
    /// Item level; < 1 becomes 1 and is written back.
    pub ilvl: i32,
    /// Items combined index.
    pub item: i32,
    /// Item format (the caller writes the game's value).
    pub format: u16,
    pub force: bool,
    /// 0 = roll it.
    pub quality: u8,
    pub quantity: i32,
    pub min_dur: i32,
    pub max_dur: i32,
    /// Unique/set preference: record + 1, 0 = none (forced: the record).
    pub index: i32,
    pub flags1: u32,
    pub seed: u32,
    pub item_seed: u32,
    pub ear_level: i32,
    pub quantity_override: i32,
    pub name: [u8; 16],
    pub prefix: [i32; 3],
    pub suffix: [i32; 3],
    pub flags2: u32,
}

/// A generated item: the fields creation writes. `S` holds its stats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item<S> {
    /// Items combined index.
    pub record: usize,
    pub format: u16,
    pub ilvl: i32,
    pub quality: u8,
    /// Unique/set/superior/low-quality/elixir index, or −1.
    pub file_index: i32,
    /// Magic affix ids (combined index + 1; 0 none).
    pub prefix: [u16; 3],
    pub suffix: [u16; 3],
    /// Rare affix ids.
    pub rare_prefix: u16,
    pub rare_suffix: u16,
    pub auto_affix: u16,
    pub flags: u32,
    /// Inventory page (0xFF none).
    pub inv_page: u8,
    /// Variable graphics index.
    pub gfx: i32,
    pub unit_seed: Seed,
    /// The unit's init seed (forced requests).
    pub init_seed: u32,
    pub item_seed: Seed,
    pub start_seed: u32,
    pub name: [u8; 16],
    pub ear_level: i32,
    /// The realm data words (`dwRealmData[0..1]`): set only by the save
    /// reader (setter), so
    /// 0 unless the item was read from a save carrying them.
    pub realm_data: [u32; 2],
    /// The ground expiry (absolute frame, 0 = never), set when the item is
    /// placed on the ground and kept by an inactive-store record.
    pub ground_expiry: i32,
    /// A fatal error raised by a legacy property function
    /// (the format-0 crash codes and the by-time checks), which the
    /// property entry points cannot return; the quality dispatch and the
    /// pipeline return it.
    /// Once set, later property records do nothing.
    pub fatal: Option<Fatal>,
    pub stats: S,
}

/// A name longer than 15 characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("item name of {0} bytes does not fit 15 characters and a terminator")]
pub struct NameTooLong(pub usize);

impl<S> Item<S> {
    /// Sets the item's name: at most 15 characters and a 0; a longer name
    /// is refused. The name ends at its first 0 byte.
    pub fn set_name(&mut self, name: &[u8]) -> Result<(), NameTooLong> {
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        if len > 15 {
            return Err(NameTooLong(len));
        }
        self.name = [0; 16];
        self.name[..len].copy_from_slice(&name[..len]);
        Ok(())
    }
    /// A fresh item record with the given stats holder, before creation
    /// fills it.
    pub fn new(record: usize, format: u16, stats: S) -> Self {
        Self {
            record,
            format,
            ilvl: 1,
            quality: q::NONE,
            file_index: -1,
            prefix: [0; 3],
            suffix: [0; 3],
            rare_prefix: 0,
            rare_suffix: 0,
            auto_affix: 0,
            flags: 0,
            inv_page: 0xFF,
            gfx: 0,
            unit_seed: Seed::init(),
            init_seed: 0,
            item_seed: Seed::init(),
            start_seed: 0,
            name: [0; 16],
            ear_level: 0,
            realm_data: [0; 2],
            ground_expiry: 0,
            fatal: None,
            stats,
        }
    }

    /// "Item level" helper: a stored value < 1 is set to 1 first.
    pub fn item_level(&mut self) -> i32 {
        if self.ilvl < 1 {
            self.ilvl = 1;
        }
        self.ilvl
    }

    /// "Is magic or better": quality 4–9.
    pub fn magic_or_better(&self) -> bool {
        (q::MAGIC..=q::TEMPERED).contains(&self.quality)
    }

    /// "Clear": affix slots, rare names, file index.
    pub fn clear(&mut self) {
        self.prefix = [0; 3];
        self.suffix = [0; 3];
        self.rare_prefix = 0;
        self.rare_suffix = 0;
        self.file_index = -1;
    }
}

/// Fatal item-creation errors, returned to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Fatal {
    #[error("item record missing (0x686)")]
    NoItemRecord,
    #[error("defense roll above maxac")]
    Defense,
    #[error("no itemratio row")]
    NoRatioRow,
    #[error("charm with no affix (0x372)")]
    Charm,
    #[error("personalized item without player data")]
    NoPlayerData,
    /// An itemratio divisor of 0.
    #[error("itemratio divisor 0")]
    DivideByZero,
    /// Crafted: the group of affix id 0.
    #[error("crafted affix 0 with a filled slot (read at 0x5C)")]
    NullAffixGroup,
    /// A format-0 property code 245–267 (other than
    /// 257–261) would call a non-legacy property function through the
    /// legacy table; D2RUST refuses it with this error.
    #[error("format-0 property code {0} calls a non-legacy function (crash)")]
    LegacyPropertyCrash(i32),
    /// A by-time record out of range
    /// (ERROR 0x444 min > max, 0x44C param > 3, 0x44D min + 256 > 0x3FF,
    /// 0x44E max + 256 > 0x3FF).
    #[error("format-0 by-time property out of range (0x{0:X})")]
    LegacyByTime(u16),
}
