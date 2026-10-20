//! Vendors: store lists, store generation, trade and gamble open, gamble
//! lists, store refresh, buy and sell, repair and prices.
//!
//! Table data is a projection of `d2-data` typed records
//! ([`VendorTables::from_fixed`]). The vendor part of an NPC record
//! ([`VendorRecord`]) is owned here and is meant
//! to be embedded in the NPC record of `world::npc`; the NPC-control seed
//! stays with that record and is passed in. Items, stats, inventories and
//! messages are reached through [`VendorWorld`]; NPC interaction plumbing
//! (`world::npc`) through the narrow [`NpcLink`].
//! Every draw goes through [`Seed`] in a fixed order (determinism).

pub mod gamble;
pub mod price;
pub mod provider;
pub mod store;
pub mod trade;

#[cfg(test)]
mod mod_store_tests;
#[cfg(test)]
mod tests;

use d2_data::bin::BinTable;
use d2_data::fixup::maps::EquivMatrix;
use d2_data::fixup::FixedSet;
use d2_data::tables::{
    decode_all, text, Armor, Automagic, Books, Difficultylevels, Itemstatcost, Itemtypes,
    Lowqualityitems, Magicprefix, Magicsuffix, Misc, Monstats, Npc, Record, Setitems, Skills,
    Uniqueitems, Weapons, WrongTable,
};

use crate::units::UnitId;

pub use price::{cost, PriceCtx, PriceItem};
pub use provider::{StoreProvider, StoreRoles};

// ------------------------------------------------------------- constants

/// Vendor columns.
pub const COLUMNS: usize = 17;
/// Byte offset of `<Vendor>Min` for column 0; Max, MagicMin, MagicMax,
/// MagicLvl follow 17 bytes apart.
pub const COLUMN_BASE: usize = 326;
/// Store item level caps per act in Normal.
pub const ACT_CAPS: [i32; 5] = [12, 20, 28, 36, 45];
/// Failure limit of store generation.
pub const FAIL_LIMIT: u32 = 32;
/// Gamble list length.
pub const GAMBLE_ITEMS: u32 = 14;
/// Refresh time in milliseconds.
pub const REFRESH_MS: u32 = 240_000;
/// Charged-skill cost base.
pub const CHARGE_BASE: i32 = 10_000;
/// Town levels by act.
pub const TOWNS: [u16; 5] = [1, 40, 75, 103, 109];
/// Spaces: an empty code field.
pub const NO_CODE: [u8; 4] = *b"    ";
/// "No upgrade" code of `NightmareUpgrade` / `HellUpgrade`.
pub const XXX: [u8; 4] = *b"xxx ";
/// Fixed codes (Constants).
pub const CQV: [u8; 4] = *b"cqv ";
pub const AQV: [u8; 4] = *b"aqv ";
pub const RIN: [u8; 4] = *b"rin ";
pub const AMU: [u8; 4] = *b"amu ";
pub const HP4: [u8; 4] = *b"hp4 ";
pub const HP5: [u8; 4] = *b"hp5 ";
pub const MP4: [u8; 4] = *b"mp4 ";
pub const MP5: [u8; 4] = *b"mp5 ";
/// Low-quality name that marks a destroyed try.
pub const CRACKED: &[u8] = b"Cracked";

/// Transactions (price kinds).
pub mod tx {
    pub const BUY: u32 = 0;
    pub const SELL: u32 = 1;
    pub const GAMBLE: u32 = 2;
    pub const REPAIR: u32 = 3;
}

/// Monstats classes the vendor code tests.
pub mod class {
    pub const GHEED: u16 = 147;
    pub const AKARA: u16 = 148;
    pub const KASHYA: u16 = 150;
    pub const CHARSI: u16 = 154;
    pub const DROGNAN: u16 = 177;
    pub const FARA: u16 = 178;
    pub const GREIZ: u16 = 198;
    pub const ELZIX: u16 = 199;
    pub const LYSANDER: u16 = 202;
    pub const ASHEARA: u16 = 252;
    pub const HRATLI: u16 = 253;
    pub const ALKOR: u16 = 254;
    pub const ORMUS: u16 = 255;
    pub const HALBU: u16 = 257;
    pub const JAMELLA: u16 = 405;
    pub const LARZUK: u16 = 511;
    pub const DREHYA: u16 = 512;
    pub const MALAH: u16 = 513;
    pub const NIHLATHAK: u16 = 514;
    pub const QUAL_KEHK: u16 = 515;
}

/// Stat ids read or written.
pub mod stat {
    pub const LEVEL: u16 = 12;
    pub const GOLD: u16 = 14;
    pub const GOLD_BANK: u16 = 15;
    pub const ARMORCLASS: u16 = 31;
    pub const QUANTITY: u16 = 70;
    pub const DURABILITY: u16 = 72;
    pub const MAXDURABILITY: u16 = 73;
    pub const REDUCED_PRICES: u16 = 87;
    pub const ITEM_SINGLESKILL: u16 = 107;
    pub const INDESTRUCTIBLE: u16 = 152;
    pub const CHARGED_SKILL: u16 = 204;
    pub const REPLENISH_DURABILITY: u16 = 252;
    pub const REPLENISH_QUANTITY: u16 = 253;
    pub const EXTRA_STACK: u16 = 254;
}

/// Item flags.
pub mod flag {
    pub const NEW: u32 = 0x1;
    pub const TARGET: u32 = 0x2;
    pub const IDENTIFIED: u32 = 0x10;
    pub const BROKEN: u32 = 0x100;
    pub const NOSELL: u32 = 0x1000;
    pub const EAR: u32 = 0x10000;
    pub const STARTITEM: u32 = 0x20000;
    pub const ETHEREAL: u32 = 0x40_0000;
    pub const PERSONALIZED: u32 = 0x100_0000;
}

/// Vendor bits of a unit's flags.
pub mod unit_flag {
    pub const VENDOR: u32 = 0x4;
    pub const TAKEN: u32 = 0x10;
}

/// Item types the vendor code tests (`itemtypes` rows).
pub mod ty {
    pub const PLAY: u16 = 7;
    pub const BOOK: u16 = 18;
    pub const SCRO: u16 = 22;
    pub const QUEST: u16 = 39;
    pub const BODY: u16 = 40;
    pub const ARMO: u16 = 50;
}

/// Item modes the vendor code uses.
pub mod mode {
    pub const STORED: u32 = 0;
    pub const CURSOR: u32 = 4;
}

// --------------------------------------------------- per-NPC constants

/// The column a trader's record copies at game creation.
/// Nihlathak gets Larzuk's 15.
pub fn column_of(class: u16) -> Option<usize> {
    use class::*;
    Some(match class {
        GHEED => 1,
        AKARA => 0,
        CHARSI => 2,
        DROGNAN => 5,
        FARA => 3,
        ELZIX => 9,
        LYSANDER => 4,
        ASHEARA => 10,
        HRATLI => 6,
        ALKOR => 7,
        ORMUS => 8,
        HALBU => 12,
        JAMELLA => 13,
        LARZUK | NIHLATHAK => 15,
        DREHYA => 16,
        MALAH => 14,
        _ => return None,
    })
}

/// The column built in the global lists: as [`column_of`], but Nihlathak
/// builds column 16.
pub fn global_column_of(class: u16) -> Option<usize> {
    match class {
        class::NIHLATHAK => Some(16),
        c => column_of(c),
    }
}

/// Classes whose record has gamble lists.
pub const GAMBLERS: [u16; 6] = [
    class::GHEED,
    class::ELZIX,
    class::ALKOR,
    class::JAMELLA,
    class::DREHYA,
    class::NIHLATHAK,
];

/// Classes whose record gets the two flag bytes.
pub const FLAGGED: [u16; 9] = [
    class::GHEED,
    class::CHARSI,
    class::FARA,
    class::HRATLI,
    class::ASHEARA,
    class::HALBU,
    class::JAMELLA,
    class::LARZUK,
    class::DREHYA,
];

/// Classes that make a hire list at trade open.
pub const HIRE_CLASSES: [u16; 4] = [
    class::KASHYA,
    class::QUAL_KEHK,
    class::GREIZ,
    class::ASHEARA,
];

/// Classes whose store a trade open never refreshes.
pub const NO_STORE_REFRESH: [u16; 3] = [class::KASHYA, class::QUAL_KEHK, class::GREIZ];

/// Repairing NPCs.
pub const REPAIRERS: [u16; 5] = [
    class::CHARSI,
    class::FARA,
    class::HRATLI,
    class::HALBU,
    class::LARZUK,
];

/// Store item level: L_p + 5, capped by the act in Normal.
pub fn store_level(player_level: i32, difficulty: u8, act: u8) -> i32 {
    let ilvl = player_level.wrapping_add(5);
    match (difficulty, ACT_CAPS.get(usize::from(act))) {
        (0, Some(&cap)) => ilvl.min(cap),
        _ => ilvl,
    }
}

// ----------------------------------------------------------------- tables

/// The item columns vendors read (weapons, armor, misc share one layout;
/// Constants).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VendorItem {
    pub code: [u8; 4],
    pub normcode: [u8; 4],
    pub ubercode: [u8; 4],
    pub ultracode: [u8; 4],
    pub cost: u32,
    pub gamble_cost: u32,
    pub minstack: u32,
    pub maxstack: u32,
    pub bitfield1: u32,
    pub version: u16,
    pub level: u8,
    /// Primary and secondary type (i16 reading of the links).
    pub type_: i16,
    pub type2: i16,
    pub durability: u8,
    pub nodurability: u8,
    pub quest: u8,
    pub stackable: u8,
    pub spawnable: u8,
    pub minac: u32,
    pub maxac: u32,
    pub nightmare_upgrade: [u8; 4],
    pub hell_upgrade: [u8; 4],
    pub perm_store: u8,
    /// Vendor columns: (Min, Max, MagicMin, MagicMax, MagicLvl) per column.
    pub columns: [[u8; 5]; COLUMNS],
}

macro_rules! vendor_item {
    ($name:ident, $t:ty) => {
        fn $name(r: &$t, raw: &[u8]) -> VendorItem {
            VendorItem {
                code: r.code,
                normcode: r.normcode,
                ubercode: r.ubercode,
                ultracode: r.ultracode,
                cost: r.cost,
                gamble_cost: r.gamble_cost,
                minstack: r.minstack,
                maxstack: r.maxstack,
                bitfield1: r.bitfield1,
                version: r.version,
                level: r.level,
                type_: r.type_ as i16,
                type2: r.type2 as i16,
                durability: r.durability,
                nodurability: r.nodurability,
                quest: r.quest,
                stackable: r.stackable,
                spawnable: r.spawnable,
                minac: r.minac,
                maxac: r.maxac,
                nightmare_upgrade: r.nightmareupgrade,
                hell_upgrade: r.hellupgrade,
                perm_store: r.permstoreitem,
                columns: std::array::from_fn(|i| {
                    std::array::from_fn(|f| raw[COLUMN_BASE + 17 * f + i])
                }),
            }
        }
    };
}
vendor_item!(from_weapons, Weapons);
vendor_item!(from_armor, Armor);
vendor_item!(from_misc, Misc);

/// The `itemtypes` columns vendors read (Constants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TypeRec {
    pub repair: u8,
    pub quiver: u16,
    pub throwable: u8,
    pub autostack: u8,
    pub staffmods: u8,
    pub class: u8,
    pub storepage: u8,
}

impl From<&Itemtypes> for TypeRec {
    fn from(r: &Itemtypes) -> Self {
        TypeRec {
            repair: r.repair,
            quiver: r.quiver,
            throwable: r.throwable,
            autostack: r.autostack,
            staffmods: r.staffmods,
            class: r.class,
            storepage: r.storepage,
        }
    }
}

/// A cost multiplier and addend (`magicprefix` +136/+140, `uniqueitems`
/// +124/+128, `setitems` +56/+60, `skills` +564/+568, `itemstatcost`
/// +16/+20).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CostMod {
    pub mult: i32,
    pub add: i32,
}

/// A unique item row: its cost modifier and its flag byte (sell
/// rule 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct UniqueCost {
    pub cost: CostMod,
    pub flags: u8,
}

/// A skill row's cost modifier and `reqlevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SkillCost {
    pub cost: CostMod,
    pub reqlevel: i32,
}

/// An `itemstatcost` row's cost columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StatCost {
    pub cost: CostMod,
    pub valshift: u8,
    pub encode: u8,
}

/// An `npc.txt` row (price multipliers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NpcPrices {
    pub class: u32,
    pub sell: i32,
    pub buy: i32,
    pub rep: i32,
    /// Quests A, B, C: (flag slot, sell, buy, rep multipliers).
    pub quests: [(u32, i32, i32, i32); 3],
    /// `max buy` per difficulty.
    pub max_buy: [i32; 3],
}

impl From<&Npc> for NpcPrices {
    fn from(r: &Npc) -> Self {
        let i = |v: u32| v as i32;
        NpcPrices {
            class: r.npc,
            sell: i(r.sell_mult),
            buy: i(r.buy_mult),
            rep: i(r.rep_mult),
            quests: [
                (
                    r.questflag_a,
                    i(r.questsellmult_a),
                    i(r.questbuymult_a),
                    i(r.questrepmult_a),
                ),
                (
                    r.questflag_b,
                    i(r.questsellmult_b),
                    i(r.questbuymult_b),
                    i(r.questrepmult_b),
                ),
                (
                    r.questflag_c,
                    i(r.questsellmult_c),
                    i(r.questbuymult_c),
                    i(r.questrepmult_c),
                ),
            ],
            max_buy: [i(r.max_buy), i(r.max_buy_n), i(r.max_buy_h)],
        }
    }
}

/// `difficultylevels` gamble odds: rare, set, unique, uber, ultra.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GambleOdds {
    pub rare: u32,
    pub set: u32,
    pub unique: u32,
    pub uber: i32,
    pub ultra: i32,
}

impl From<&Difficultylevels> for GambleOdds {
    fn from(r: &Difficultylevels) -> Self {
        GambleOdds {
            rare: r.gamblerare,
            set: r.gambleset,
            unique: r.gambleunique,
            uber: r.gambleuber as i32,
            ultra: r.gambleultra as i32,
        }
    }
}

/// Everything the vendor code reads from the tables.
#[derive(Clone, Debug, Default)]
pub struct VendorTables {
    /// The combined items array: weapons, armor, misc.
    pub items: Vec<VendorItem>,
    pub itemtypes: Vec<TypeRec>,
    /// Itemtypes equivalence.
    pub equiv: EquivMatrix,
    /// Magic affix array (suffixes, prefixes, automagic): cost columns.
    pub magic: Vec<CostMod>,
    pub uniques: Vec<UniqueCost>,
    pub setitems: Vec<CostMod>,
    /// `books.costpercharge` per row.
    pub books: Vec<i32>,
    pub skills: Vec<SkillCost>,
    pub stats: Vec<StatCost>,
    /// Layer split `stuff` and its mask.
    pub stat_shift: u32,
    pub stat_mask: u32,
    /// `monstats` Level, Level(N), Level(H) per class.
    pub monster_levels: Vec<[u16; 3]>,
    /// `monstats` classes with `interact`, in row order.
    pub interact: Vec<u16>,
    pub npc: Vec<NpcPrices>,
    pub difficulty: Vec<GambleOdds>,
    /// Low-quality names (`lowqualityitems` Name text).
    pub lowquality: Vec<Vec<u8>>,
    /// Gamble index and thresholds.
    pub gamble_index: Option<Vec<u32>>,
    pub gamble_thresholds: Vec<u32>,
    /// Mask tested against a unique's flag byte (sell rule 7): 4, the
    /// `carry1` bit.
    pub unique_nosell_mask: u8,
}

/// The `carry1` bit of a uniqueitems row's flag byte.
pub const UNIQUE_NOSELL_MASK: u8 = 0x04;

/// A table the projection needs is missing or has the wrong layout.
#[derive(Debug, thiserror::Error)]
pub enum TableError {
    #[error("table {0} missing")]
    Missing(&'static str),
    #[error(transparent)]
    Wrong(#[from] WrongTable),
}

fn get<'a>(f: &'a FixedSet, name: &'static str) -> Result<&'a BinTable, TableError> {
    f.table(name).ok_or(TableError::Missing(name))
}

fn typed<T: Record>(f: &FixedSet) -> Result<Vec<T>, TableError> {
    Ok(decode_all::<T>(get(f, T::TABLE)?)?)
}

fn cost_mod(mult: u32, add: u32) -> CostMod {
    CostMod {
        mult: mult as i32,
        add: add as i32,
    }
}

impl VendorTables {
    /// Projects the fixed-up table set (`d2_data::fixup::apply`).
    pub fn from_fixed(f: &FixedSet) -> Result<Self, TableError> {
        let mut items = Vec::new();
        for (r, raw) in typed::<Weapons>(f)?.iter().zip(get(f, "weapons")?.iter()) {
            items.push(from_weapons(r, raw));
        }
        for (r, raw) in typed::<Armor>(f)?.iter().zip(get(f, "armor")?.iter()) {
            items.push(from_armor(r, raw));
        }
        for (r, raw) in typed::<Misc>(f)?.iter().zip(get(f, "misc")?.iter()) {
            items.push(from_misc(r, raw));
        }
        let mut magic: Vec<CostMod> = typed::<Magicsuffix>(f)?
            .iter()
            .map(|r| cost_mod(r.multiply, r.add))
            .collect();
        magic.extend(
            typed::<Magicprefix>(f)?
                .iter()
                .map(|r| cost_mod(r.multiply, r.add)),
        );
        magic.extend(
            typed::<Automagic>(f)?
                .iter()
                .map(|r| cost_mod(r.multiply, r.add)),
        );
        let monstats = typed::<Monstats>(f)?;
        Ok(VendorTables {
            items,
            itemtypes: typed::<Itemtypes>(f)?.iter().map(TypeRec::from).collect(),
            equiv: f.itemtypes_equiv.clone(),
            magic,
            uniques: typed::<Uniqueitems>(f)?
                .iter()
                .zip(get(f, "uniqueitems")?.iter())
                .map(|(r, raw)| UniqueCost {
                    cost: cost_mod(r.cost_mult, r.cost_add),
                    flags: raw[0x2C],
                })
                .collect(),
            setitems: typed::<Setitems>(f)?
                .iter()
                .map(|r| cost_mod(r.cost_mult, r.cost_add))
                .collect(),
            books: typed::<Books>(f)?
                .iter()
                .map(|r| r.costpercharge as i32)
                .collect(),
            skills: typed::<Skills>(f)?
                .iter()
                .map(|r| SkillCost {
                    cost: cost_mod(r.cost_mult, r.cost_add),
                    // An i16 field.
                    reqlevel: i32::from(r.reqlevel as i16),
                })
                .collect(),
            stats: typed::<Itemstatcost>(f)?
                .iter()
                .map(|r| StatCost {
                    cost: cost_mod(r.multiply, r.add),
                    valshift: r.valshift,
                    encode: r.encode,
                })
                .collect(),
            stat_shift: f.stat_stuff,
            stat_mask: f.stat_mask,
            monster_levels: monstats
                .iter()
                .map(|m| [m.level, m.level_n, m.level_h])
                .collect(),
            interact: monstats
                .iter()
                .enumerate()
                .filter(|(_, m)| m.interact)
                .map(|(i, _)| i as u16)
                .collect(),
            npc: typed::<Npc>(f)?.iter().map(NpcPrices::from).collect(),
            difficulty: typed::<Difficultylevels>(f)?
                .iter()
                .map(GambleOdds::from)
                .collect(),
            lowquality: typed::<Lowqualityitems>(f)?
                .iter()
                .map(|r| text(&r.name).to_vec())
                .collect(),
            gamble_index: f.gamble.index.clone(),
            gamble_thresholds: f.gamble.thresholds.to_vec(),
            unique_nosell_mask: UNIQUE_NOSELL_MASK,
        })
    }

    pub fn item(&self, i: usize) -> Option<&VendorItem> {
        self.items.get(i)
    }

    /// The item code map's find (first record with the code).
    pub fn find_code(&self, code: [u8; 4]) -> Option<usize> {
        self.items.iter().position(|r| r.code == code)
    }

    /// The primary type's itemtypes row of item `i`.
    pub fn type_of(&self, i: usize) -> Option<&TypeRec> {
        let t = self.item(i)?.type_;
        usize::try_from(t).ok().and_then(|t| self.itemtypes.get(t))
    }

    /// "Item is type T" (with the type equivalences).
    pub fn is_type(&self, i: usize, t: u16) -> bool {
        let Some(r) = self.item(i) else { return false };
        let t = usize::from(t);
        let eq = |a: i16| a >= 0 && self.equiv.get(a as usize, t);
        eq(r.type_) || (r.type2 > 0 && eq(r.type2))
    }

    /// The `npc.txt` row of a class.
    pub fn npc_row(&self, class: u16) -> Option<&NpcPrices> {
        self.npc.iter().find(|r| r.class == u32::from(class))
    }

    /// A code field that names an item: ≠ 0 and ≠ four spaces, and found
    /// in the code map.
    pub fn valid_code(&self, code: [u8; 4]) -> Option<usize> {
        if code == [0; 4] || code == NO_CODE {
            return None;
        }
        self.find_code(code)
    }
}

// ------------------------------------------------------------ store lists

/// One item-list entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ColumnEntry {
    pub min: u8,
    pub max: u8,
    pub magic_min: u8,
    pub magic_max: u8,
    pub code: [u8; 4],
    pub magic_lvl: u8,
}

/// One column's two lists.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Column {
    pub items: Vec<ColumnEntry>,
    pub perm: Vec<[u8; 4]>,
}

impl Column {
    /// Builds column `i` from the combined item array.
    pub fn build(t: &VendorTables, i: usize) -> Self {
        let mut c = Column::default();
        for r in &t.items {
            let [min, max, magic_min, magic_max, magic_lvl] = r.columns[i];
            if r.spawnable == 0 || (max == 0 && magic_max == 0) {
                continue;
            }
            if r.perm_store != 0 {
                c.perm.push(r.code);
            } else {
                c.items.push(ColumnEntry {
                    min,
                    max,
                    magic_min,
                    magic_max,
                    code: r.code,
                    magic_lvl,
                });
            }
        }
        c
    }
}

/// The global lists (built once per server start): a column is
/// built for each `interact` class's global column ([`StoreRoles`]);
/// the others stay empty.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GlobalLists {
    pub columns: Vec<Column>,
}

impl GlobalLists {
    /// D2RUST's global lists.
    pub fn build(t: &VendorTables) -> Self {
        Self::build_for(t, &crate::rules::ORIGINAL)
    }

    /// The global lists under `rules`: each class's
    /// [`StoreRoles::global_column`].
    pub fn build_for(t: &VendorTables, rules: &crate::rules::Ruleset) -> Self {
        let mut columns = vec![Column::default(); COLUMNS];
        let mut built = [false; COLUMNS];
        for &class in &t.interact {
            if let Some(i) = StoreRoles::of(rules, class).global_column {
                if !built[i] {
                    built[i] = true;
                    columns[i] = Column::build(t, i);
                }
            }
        }
        GlobalLists { columns }
    }
}

// ----------------------------------------------------------------- record

/// One player's gamble list at an NPC.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GambleList {
    pub player: u32,
    /// The node's inventory, in placement order.
    pub items: Vec<UnitId>,
}

/// One player's vendor-chain node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ChainNode {
    pub player: u32,
    pub gamble_mode: bool,
}

/// A node of the NPC event list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventNode {
    pub unit: UnitId,
    pub arg: u32,
    pub kind: u32,
    pub deferred: bool,
}

/// The vendor part of an NPC record. `world::npc`
/// embeds it; `class`, `act` and `trader` are copies of the NPC table
/// fields.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct VendorRecord {
    pub class: u16,
    pub act: u8,
    pub trader: bool,
    /// The store items of the NPC inventory, in creation order.
    pub store: Vec<UnitId>,
    /// Gamble lists, newest first.
    pub gamble_lists: Vec<GambleList>,
    /// The class gambles.
    pub has_gamble: bool,
    /// The NPC event list.
    pub events: Vec<EventNode>,
    /// Vendor-chain nodes, newest first.
    pub chain: Vec<ChainNode>,
    /// A trade happened here.
    pub has_traded: bool,
    /// The store was generated.
    pub store_generated: bool,
    /// The two flag bytes (no reader yet).
    pub flag24: bool,
    pub flag25: bool,
    /// A refresh is pending (it runs at a trade open).
    pub refresh_pending: bool,
    /// Store time (host milliseconds).
    pub store_time: u32,
    /// The column copy: item entries and permanent codes.
    pub items: Vec<ColumnEntry>,
    pub perm: Vec<[u8; 4]>,
    /// NPC GUID of the last trade.
    pub last_npc: u32,
}

impl VendorRecord {
    /// The record's vendor data at creation under D2RUST's store roles:
    /// a trader gets the copy of its column and its flags; a
    /// non-trader gets empty lists.
    pub fn new(class: u16, act: u8, trader: bool, globals: &GlobalLists) -> Self {
        Self::new_for(&crate::rules::ORIGINAL, class, act, trader, globals)
    }

    /// As [`Self::new`], under `rules`: the column and flags are the
    /// class's [`StoreRoles`], the gamble lists its NPC roles' `gambles`.
    pub fn new_for(
        rules: &crate::rules::Ruleset,
        class: u16,
        act: u8,
        trader: bool,
        globals: &GlobalLists,
    ) -> Self {
        let mut r = VendorRecord {
            class,
            act,
            trader,
            ..Default::default()
        };
        if !trader {
            return r;
        }
        let roles = StoreRoles::of(rules, class);
        if let Some(c) = roles.column.and_then(|i| globals.columns.get(i)) {
            r.items = c.items.clone();
            r.perm = c.perm.clone();
        }
        r.has_gamble = crate::world::npc::NpcRoles::of(rules, class).gambles;
        r.flag24 = roles.flagged;
        r.flag25 = r.flag24;
        r
    }

    /// The player's gamble list here.
    pub fn gamble_list(&self, player: u32) -> Option<&GambleList> {
        self.gamble_lists.iter().find(|g| g.player == player)
    }

    /// The player's vendor-chain node here.
    pub fn chain_node(&self, player: u32) -> Option<&ChainNode> {
        self.chain.iter().find(|n| n.player == player)
    }

    /// The player's node, created (prepended) on first use.
    pub fn chain_node_mut(&mut self, player: u32) -> &mut ChainNode {
        let i = match self.chain.iter().position(|n| n.player == player) {
            Some(i) => i,
            None => {
                self.chain.insert(
                    0,
                    ChainNode {
                        player,
                        gamble_mode: false,
                    },
                );
                0
            }
        };
        &mut self.chain[i]
    }
}

// ------------------------------------------------------------------ seams

/// The NPC transaction message S→C 0x2A, without the
/// unwritten bytes 3–6. `gold` is the player's stat 14 after the
/// transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub kind: u8,
    pub code: u8,
    pub guid: u32,
    pub gold: i32,
}

/// No GUID (−1) in a 0x2A.
pub const NO_GUID: u32 = 0xFFFF_FFFF;

/// Seam: NPC interaction plumbing owned by `world::npc`.
/// Expected provider: the NPC module.
pub trait NpcLink {
    /// The NPC unit with this GUID (a monster with an interaction list).
    fn npc_by_guid(&self, guid: u32) -> Option<UnitId>;
    /// The NPC unit's monstats class.
    fn npc_class(&self, npc: UnitId) -> u16;
    /// Whether `npc` is the player's interact unit.
    fn is_interact_unit(&self, player: UnitId, npc: UnitId) -> bool;
    /// Whether the NPC's interaction list is empty.
    fn interaction_empty(&self, npc: UnitId) -> bool;
    /// "Hire list made" of the class's record.
    fn hire_list_made(&self, class: u16) -> bool;
    fn set_hire_list_made(&mut self, class: u16);
    /// Makes the class's hire list. The list draws from
    /// the NPC-control seed, which the trade open holds for store
    /// generation: it is lent here as `seed`.
    fn make_hire_list(&mut self, class: u16, seed: &mut crate::rng::Seed);
}

/// Seam: everything else vendors reach (game fields, units, stats,
/// items, inventories, messages). Expected providers: game creation,
/// units / stats, the items group (creation, copy, inventory placement)
/// and the transport.
pub trait VendorWorld: NpcLink {
    // ---- game
    /// Difficulty: 0 normal, 1 nightmare, 2 hell.
    fn difficulty(&self) -> u8;
    /// Expansion game.
    fn expansion(&self) -> bool;
    /// The item format.
    fn item_format(&self) -> u16;
    /// Game type (a client leaving skips type 3).
    fn game_type(&self) -> u8;
    /// The rules the game runs under (store roles, MS8e). Default:
    /// D2RUST's own.
    fn rules(&self) -> &crate::rules::Ruleset {
        &crate::rules::ORIGINAL
    }

    // ---- units and stats
    fn guid(&self, unit: UnitId) -> u32;
    fn player_by_guid(&self, guid: u32) -> Option<UnitId>;
    fn item_by_guid(&self, guid: u32) -> Option<UnitId>;
    /// Unit total of a stat.
    fn stat(&self, unit: UnitId, id: u16, layer: u16) -> i32;
    /// Base stat.
    fn base_stat(&self, unit: UnitId, id: u16, layer: u16) -> i32;
    /// Sets the base stat.
    fn set_stat(&mut self, unit: UnitId, id: u16, layer: u16, value: i32);
    /// The player's quest slot word of difficulty `d`.
    fn quest_slot(&self, player: UnitId, difficulty: u8, slot: u32) -> u16;
    /// Living players whose room is in `level`.
    fn players_in_level(&self, level: u16) -> i32;
    /// The player's room level.
    fn player_level_id(&self, player: UnitId) -> u16;
    /// Quest intro hook on entering a town.
    fn town_entered(&mut self, player: UnitId, level: u16);
    /// The player's carried-gold cap and stash cap.
    fn gold_cap(&self, player: UnitId) -> i32;
    fn stash_cap(&self, player: UnitId) -> i32;
    /// Drops a gold pile at the player.
    fn drop_gold(&mut self, player: UnitId, amount: i32);
    /// The GUID of the player's last bought item.
    fn last_bought(&self, player: UnitId) -> u32;
    fn set_last_bought(&mut self, player: UnitId, guid: u32);
    /// Whether the player holds a cursor item.
    fn has_cursor_item(&self, player: UnitId) -> bool;

    // ---- items
    /// Item creation (owner the NPC of `npc_class`'s record,
    /// mode 4, quality, item level; seeds from the game seed).
    fn create_item(
        &mut self,
        npc_class: u16,
        record: usize,
        quality: u8,
        ilvl: i32,
    ) -> Option<UnitId>;
    /// Item copy.
    fn copy_item(&mut self, item: UnitId) -> Option<UnitId>;
    /// Destroys an item unit.
    fn destroy_item(&mut self, item: UnitId);
    /// The item's combined record index.
    fn item_record(&self, item: UnitId) -> usize;
    fn item_quality(&self, item: UnitId) -> u8;
    fn item_file_index(&self, item: UnitId) -> i32;
    /// Item flags.
    fn item_flags(&self, item: UnitId) -> u32;
    fn set_item_flags(&mut self, item: UnitId, flags: u32);
    /// Unit flag bits ([`unit_flag`]).
    fn or_unit_flags(&mut self, item: UnitId, bits: u32);
    /// Item mode.
    fn item_mode(&self, item: UnitId) -> u32;
    fn set_item_mode(&mut self, item: UnitId, mode: u32);
    fn set_item_page(&mut self, item: UnitId, page: u8);
    /// Has filled sockets.
    fn has_filled_sockets(&self, item: UnitId) -> bool;
    /// The item's price inputs; `None` when not an item.
    fn price_item(&self, item: UnitId) -> Option<PriceItem>;
    /// Recharge; broken-item repair.
    fn recharge(&mut self, item: UnitId);
    fn repair_broken(&mut self, item: UnitId);
    /// Identify.
    fn identify(&mut self, item: UnitId);
    /// S→C 0x3E: an item stat to the player.
    fn send_item_stat(&mut self, player: UnitId, item: UnitId, stat: u16);
    /// S→C 0x2A.
    fn send_transaction(&mut self, player: UnitId, t: Transaction);

    // ---- NPC inventories
    /// A new (empty) store inventory for the record (`npc` = the NPC unit
    /// it is also assigned to, at a refresh).
    fn new_store_inventory(&mut self, npc_class: u16, npc: Option<UnitId>);
    /// Places an item in the store grid on its page.
    fn place_in_store(&mut self, npc_class: u16, item: UnitId) -> bool;
    /// Removes a store item.
    fn remove_store_item(&mut self, npc_class: u16, item: UnitId);
    /// Takes a store item out of the grid (removed, then
    /// re-added to the trade inventory so the client drops it; the caller
    /// sets [`unit_flag::TAKEN`] in between).
    fn take_from_store(&mut self, npc_class: u16, item: UnitId);
    /// New gamble-list inventory; place / remove there.
    fn place_in_gamble(&mut self, npc_class: u16, player: u32, item: UnitId) -> bool;
    fn remove_gamble_item(&mut self, npc_class: u16, player: u32, item: UnitId);
    /// Refresh the NPC inventory.
    fn refresh_npc_inventory(&mut self, npc: UnitId);
    /// Adds the item to the NPC's trade inventory (the client
    /// receives 0x9C action 11).
    fn add_trade_inventory(&mut self, npc_class: u16, item: UnitId);

    // ---- player inventories (buy, sell, repair)
    /// Item belongs to the player.
    fn owns_item(&self, player: UnitId, item: UnitId) -> bool;
    /// Item is in the player's inventory.
    fn in_inventory(&self, player: UnitId, item: UnitId) -> bool;
    /// Equipped items over the 13 body locations.
    fn equipped_items(&self, player: UnitId) -> Vec<UnitId>;
    /// A backpack tome matching the scroll with free space.
    fn find_tome(&mut self, player: UnitId, scroll: UnitId) -> Option<(UnitId, i32)>;
    /// Tome quantity += k.
    fn add_to_tome(&mut self, tome: UnitId, k: i32);
    /// A partial stack of the same item (equipped first,
    /// then backpack) with its free space.
    fn find_partial_stack(&mut self, player: UnitId, item: UnitId) -> Option<(UnitId, i32)>;
    /// Can go to the belt; put it there.
    fn can_belt(&mut self, player: UnitId, item: UnitId) -> bool;
    fn put_in_belt(&mut self, player: UnitId, item: UnitId) -> bool;
    /// Arrow / bolt auto-equip at buy.
    fn equip_ammo(&mut self, player: UnitId, item: UnitId) -> bool;
    /// Auto-place in the backpack.
    fn place_in_backpack(&mut self, player: UnitId, item: UnitId) -> bool;
    /// Takes a cursor item.
    fn take_from_cursor(&mut self, player: UnitId, item: UnitId) -> bool;
    /// Lowers the matching scroll / book skill quantity
    /// (floor 0, right skill cleared, S→C 0x22).
    fn lower_book_skill(&mut self, player: UnitId, item: UnitId, n: i32);
    /// A stored item: cell := page, item update message, removed.
    fn remove_stored(&mut self, player: UnitId, item: UnitId);
    /// Unequip.
    fn unequip(&mut self, player: UnitId, item: UnitId) -> bool;
}
