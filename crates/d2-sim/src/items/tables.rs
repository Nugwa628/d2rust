//! The table columns item creation reads, projected from `d2-data` typed
//! records (`d2_data::tables`) and runtime maps (`d2_data::fixup`): items,
//! item types, affixes, qualities, uniques, sets, gems, runes, properties
//! and the stat columns. Each projection keeps the stored value; signed
//! readings (`link16` as i16, `link32`/`u32` as i32) follow the "< 0" /
//! "< 1" tests of the readers. [`ItemTables`] also carries the game's
//! rules, so every item path sees a mod game's item overrides.

use super::bitstream::Isc;
use crate::rules::Ruleset;
use d2_data::bin::BinTable;
use d2_data::fixup::maps::{EquivMatrix, SkillLists};
use d2_data::fixup::FixedSet;
use d2_data::tables::{
    decode_all, Armor, Automagic, Books, Gems, Itemratio, Itemstatcost, Itemtypes, Lowqualityitems,
    Magicprefix, Magicsuffix, Misc, Properties, Qualityitems, Rareprefix, Raresuffix, Record,
    Runes, Setitems, Sets, Skills, Uniqueitems, Weapons, WrongTable,
};

/// A property record `{code, param, min, max}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PropRec {
    /// properties row; < 0 = none.
    pub code: i32,
    pub param: i32,
    pub min: i32,
    pub max: i32,
}

impl PropRec {
    pub const NONE: PropRec = PropRec {
        code: -1,
        param: 0,
        min: 0,
        max: 0,
    };
    fn of(code: u32, param: u32, min: u32, max: u32) -> Self {
        Self {
            code: code as i32,
            param: param as i32,
            min: min as i32,
            max: max as i32,
        }
    }
}

/// The items columns used (weapons, armor and misc share one layout).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ItemRec {
    pub code: [u8; 4],
    pub version: u16,
    /// Primary type (`type`), i16 reading of the link.
    pub type_: i16,
    pub type2: i16,
    pub level: u8,
    pub magic_lvl: u8,
    pub auto_prefix: u16,
    pub block: u8,
    pub speed: i32,
    pub durability: u8,
    pub nodurability: u8,
    pub minac: u32,
    pub maxac: u32,
    pub mindam: u8,
    pub maxdam: u8,
    pub mindam2: u8,
    pub maxdam2: u8,
    pub minmisdam: u8,
    pub maxmisdam: u8,
    pub stackable: u8,
    pub minstack: u32,
    pub maxstack: u32,
    pub spawnstack: u32,
    pub hasinv: u8,
    /// items.txt `component`: the category byte of the item messages.
    pub component: u8,
    pub gemsockets: u8,
    pub gemapplytype: u8,
    pub gemoffset: i32,
    pub invwidth: u8,
    pub invheight: u8,
    pub unique: u8,
    pub quest: u8,
    pub questdiffcheck: u8,
    pub ubercode: [u8; 4],
    pub ultracode: [u8; 4],
    /// `compactsave` (+0x143).
    pub compactsave: u8,
    /// `normcode` (+0x84): the alt-code record's base code, which the
    /// item bit stream writes for an alt-code item.
    pub normcode: [u8; 4],
    /// `spawnable` (+0x133).
    pub spawnable: u8,
    /// `rarity` (+0xFC, u8).
    pub rarity: u8,
    /// `bitfield1` (+0xDC); bit 0 "may be magic", read by drops and
    /// vendors.
    pub bitfield1: u32,
    /// `rangeadder` (weapons +0x104): the melee reach added by a weapon;
    /// 0 for armor and misc.
    pub rangeadder: u8,
    /// `missiletype` (+0xFA, u16): the missile class
    /// a thrown item flies as.
    pub missiletype: u16,
    /// `wclass` (+0xC0): the COF weapon class of a weapon (`hth`, `1hs`,
    /// ...; the type class).
    pub wclass: [u8; 4],
    /// `dropsound` (+0x124, `sounds.txt` row; read inline by the Act V
    /// reward).
    pub dropsound: u16,
}

macro_rules! item_rec {
    ($($t:ty),*) => {$(
        impl From<&$t> for ItemRec {
            fn from(r: &$t) -> Self {
                ItemRec {
                    code: r.code,
                    version: r.version,
                    type_: r.type_ as i16,
                    type2: r.type2 as i16,
                    level: r.level,
                    magic_lvl: r.magic_lvl,
                    auto_prefix: r.auto_prefix,
                    block: r.block,
                    speed: r.speed as i32,
                    durability: r.durability,
                    nodurability: r.nodurability,
                    minac: r.minac,
                    maxac: r.maxac,
                    mindam: r.mindam,
                    maxdam: r.maxdam,
                    mindam2: r.f_2handmindam,
                    maxdam2: r.f_2handmaxdam,
                    minmisdam: r.minmisdam,
                    maxmisdam: r.maxmisdam,
                    stackable: r.stackable,
                    minstack: r.minstack,
                    maxstack: r.maxstack,
                    spawnstack: r.spawnstack,
                    hasinv: r.hasinv,
                    component: r.component,
                    gemsockets: r.gemsockets,
                    gemapplytype: r.gemapplytype,
                    gemoffset: r.gemoffset as i32,
                    invwidth: r.invwidth,
                    invheight: r.invheight,
                    unique: r.unique,
                    quest: r.quest,
                    questdiffcheck: r.questdiffcheck,
                    ubercode: r.ubercode,
                    ultracode: r.ultracode,
                    compactsave: r.compactsave,
                    normcode: r.normcode,
                    spawnable: r.spawnable,
                    rarity: r.rarity,
                    bitfield1: r.bitfield1,
                    rangeadder: r.rangeadder,
                    missiletype: r.missiletype,
                    wclass: r.wclass,
                    dropsound: r.dropsound,
                }
            }
        }
    )*};
}
item_rec!(Weapons, Armor, Misc);

/// A magic affix row (magicsuffix, magicprefix, automagic).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct AffixRec {
    pub version: u16,
    pub spawnable: u8,
    pub level: i32,
    pub maxlevel: i32,
    pub group: i32,
    pub rare: u8,
    pub classspecific: u8,
    pub frequency: u8,
    pub itype: [i16; 7],
    pub etype: [i16; 5],
    pub mods: [PropRec; 3],
    /// `levelreq`, `class` (0xFF none) and `classlevelreq`: the affix's
    /// share of the item's level requirement.
    pub levelreq: i32,
    pub class: u8,
    pub classlevelreq: i32,
}

macro_rules! affix_rec {
    ($($t:ty),*) => {$(
        impl From<&$t> for AffixRec {
            fn from(r: &$t) -> Self {
                AffixRec {
                    version: r.version,
                    spawnable: r.spawnable,
                    level: r.level as i32,
                    maxlevel: r.maxlevel as i32,
                    group: r.group as i32,
                    rare: r.rare,
                    classspecific: r.classspecific,
                    frequency: r.frequency,
                    itype: [r.itype1, r.itype2, r.itype3, r.itype4, r.itype5, r.itype6, r.itype7]
                        .map(|v| v as i16),
                    etype: [r.etype1, r.etype2, r.etype3, r.etype4, r.etype5].map(|v| v as i16),
                    mods: [
                        PropRec::of(r.mod1code, r.mod1param, r.mod1min, r.mod1max),
                        PropRec::of(r.mod2code, r.mod2param, r.mod2min, r.mod2max),
                        PropRec::of(r.mod3code, r.mod3param, r.mod3min, r.mod3max),
                    ],
                    levelreq: i32::from(r.levelreq),
                    class: r.class,
                    classlevelreq: i32::from(r.classlevelreq),
                }
            }
        }
    )*};
}
affix_rec!(Magicsuffix, Magicprefix, Automagic);

/// A rare affix row (raresuffix, rareprefix).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RareRec {
    pub version: u16,
    pub itype: [i16; 7],
    pub etype: [i16; 4],
}

macro_rules! rare_rec {
    ($($t:ty),*) => {$(
        impl From<&$t> for RareRec {
            fn from(r: &$t) -> Self {
                RareRec {
                    version: r.version,
                    itype: [r.itype1, r.itype2, r.itype3, r.itype4, r.itype5, r.itype6, r.itype7]
                        .map(|v| v as i16),
                    etype: [r.etype1, r.etype2, r.etype3, r.etype4].map(|v| v as i16),
                }
            }
        }
    )*};
}
rare_rec!(Raresuffix, Rareprefix);

/// A qualityitems row.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct QualityRec {
    pub mods: [PropRec; 2],
    pub armor: u8,
    pub weapon: u8,
    pub shield: u8,
    pub scepter: u8,
    pub wand: u8,
    pub staff: u8,
    pub bow: u8,
    pub boots: u8,
    pub gloves: u8,
    pub belt: u8,
}

impl From<&Qualityitems> for QualityRec {
    fn from(r: &Qualityitems) -> Self {
        QualityRec {
            mods: [
                PropRec::of(r.mod1code, r.mod1param, r.mod1min, r.mod1max),
                PropRec::of(r.mod2code, r.mod2param, r.mod2min, r.mod2max),
            ],
            armor: r.armor,
            weapon: r.weapon,
            shield: r.shield,
            scepter: r.scepter,
            wand: r.wand,
            staff: r.staff,
            bow: r.bow,
            boots: r.boots,
            gloves: r.gloves,
            belt: r.belt,
        }
    }
}

/// A uniqueitems row: the unique quality pick and its properties
/// (property mode 3).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct UniqueRec {
    pub code: [u8; 4],
    pub version: u16,
    pub enabled: bool,
    pub ladder: bool,
    pub nolimit: bool,
    /// Read as 32 bits at +0x30 (the u16 column plus two bytes that are 0
    /// in the game's tables).
    pub rarity: u32,
    pub lvl: u16,
    /// `lvl req` read as i16.
    pub lvl_req: i16,
    pub props: [PropRec; 12],
}

impl UniqueRec {
    /// From the typed record and its raw bytes: `rarity` is the 32-bit
    /// value at +0x30, the u16 column plus the two unwritten bytes after it
    /// (0 in the game's tables).
    pub fn from_record(r: &Uniqueitems, raw: &[u8]) -> Self {
        UniqueRec {
            rarity: u32::from_le_bytes([raw[0x30], raw[0x31], raw[0x32], raw[0x33]]),
            ..UniqueRec::from(r)
        }
    }
}

impl From<&Uniqueitems> for UniqueRec {
    fn from(r: &Uniqueitems) -> Self {
        UniqueRec {
            code: r.code,
            version: r.version,
            enabled: r.enabled,
            ladder: r.ladder,
            nolimit: r.nolimit,
            rarity: u32::from(r.rarity),
            lvl: r.lvl,
            lvl_req: r.lvl_req as i16,
            props: [
                PropRec::of(r.prop1, r.par1, r.min1, r.max1),
                PropRec::of(r.prop2, r.par2, r.min2, r.max2),
                PropRec::of(r.prop3, r.par3, r.min3, r.max3),
                PropRec::of(r.prop4, r.par4, r.min4, r.max4),
                PropRec::of(r.prop5, r.par5, r.min5, r.max5),
                PropRec::of(r.prop6, r.par6, r.min6, r.max6),
                PropRec::of(r.prop7, r.par7, r.min7, r.max7),
                PropRec::of(r.prop8, r.par8, r.min8, r.max8),
                PropRec::of(r.prop9, r.par9, r.min9, r.max9),
                PropRec::of(r.prop10, r.par10, r.min10, r.max10),
                PropRec::of(r.prop11, r.par11, r.min11, r.max11),
                PropRec::of(r.prop12, r.par12, r.min12, r.max12),
            ],
        }
    }
}

/// A setitems row plus the two fields the set fix-up writes
/// (version +0x22, slot +0x2E).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SetItemRec {
    pub item: [u8; 4],
    pub set: i16,
    pub lvl: u16,
    /// `lvl req` read as i16.
    pub lvl_req: i16,
    pub rarity: u32,
    pub add_func: u8,
    pub version: u16,
    pub slot: u16,
    pub props: [PropRec; 9],
    /// `aprop1a`, `aprop1b`, … `aprop5b`.
    pub aprops: [PropRec; 10],
}

impl SetItemRec {
    /// From the typed record and its raw bytes (after the fix-ups).
    pub fn from_record(r: &Setitems, raw: &[u8]) -> Self {
        SetItemRec {
            item: r.item,
            set: r.set as i16,
            lvl: r.lvl,
            lvl_req: r.lvl_req as i16,
            rarity: r.rarity,
            add_func: r.add_func,
            version: u16::from_le_bytes([raw[0x22], raw[0x23]]),
            slot: u16::from_le_bytes([raw[0x2E], raw[0x2F]]),
            props: [
                PropRec::of(r.prop1, r.par1, r.min1, r.max1),
                PropRec::of(r.prop2, r.par2, r.min2, r.max2),
                PropRec::of(r.prop3, r.par3, r.min3, r.max3),
                PropRec::of(r.prop4, r.par4, r.min4, r.max4),
                PropRec::of(r.prop5, r.par5, r.min5, r.max5),
                PropRec::of(r.prop6, r.par6, r.min6, r.max6),
                PropRec::of(r.prop7, r.par7, r.min7, r.max7),
                PropRec::of(r.prop8, r.par8, r.min8, r.max8),
                PropRec::of(r.prop9, r.par9, r.min9, r.max9),
            ],
            aprops: [
                PropRec::of(r.aprop1a, r.apar1a, r.amin1a, r.amax1a),
                PropRec::of(r.aprop1b, r.apar1b, r.amin1b, r.amax1b),
                PropRec::of(r.aprop2a, r.apar2a, r.amin2a, r.amax2a),
                PropRec::of(r.aprop2b, r.apar2b, r.amin2b, r.amax2b),
                PropRec::of(r.aprop3a, r.apar3a, r.amin3a, r.amax3a),
                PropRec::of(r.aprop3b, r.apar3b, r.amin3b, r.amax3b),
                PropRec::of(r.aprop4a, r.apar4a, r.amin4a, r.amax4a),
                PropRec::of(r.aprop4b, r.apar4b, r.amin4b, r.amax4b),
                PropRec::of(r.aprop5a, r.apar5a, r.amin5a, r.amax5a),
                PropRec::of(r.aprop5b, r.apar5b, r.amin5b, r.amax5b),
            ],
        }
    }
}

/// A sets row plus its item count (+0x0C).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SetRec {
    pub count: i32,
    /// `version` (+0x04; the format-0 set pick).
    pub version: u16,
    /// `pcode2a`, `pcode2b`, … `pcode5b`.
    pub partial: [PropRec; 8],
    /// `fcode1` … `fcode8`.
    pub full: [PropRec; 8],
}

impl SetRec {
    pub fn from_record(r: &Sets, raw: &[u8]) -> Self {
        SetRec {
            count: i32::from_le_bytes([raw[0x0C], raw[0x0D], raw[0x0E], raw[0x0F]]),
            version: r.version,
            partial: [
                PropRec::of(r.pcode2a, r.pparam2a, r.pmin2a, r.pmax2a),
                PropRec::of(r.pcode2b, r.pparam2b, r.pmin2b, r.pmax2b),
                PropRec::of(r.pcode3a, r.pparam3a, r.pmin3a, r.pmax3a),
                PropRec::of(r.pcode3b, r.pparam3b, r.pmin3b, r.pmax3b),
                PropRec::of(r.pcode4a, r.pparam4a, r.pmin4a, r.pmax4a),
                PropRec::of(r.pcode4b, r.pparam4b, r.pmin4b, r.pmax4b),
                PropRec::of(r.pcode5a, r.pparam5a, r.pmin5a, r.pmax5a),
                PropRec::of(r.pcode5b, r.pparam5b, r.pmin5b, r.pmax5b),
            ],
            full: [
                PropRec::of(r.fcode1, r.fparam1, r.fmin1, r.fmax1),
                PropRec::of(r.fcode2, r.fparam2, r.fmin2, r.fmax2),
                PropRec::of(r.fcode3, r.fparam3, r.fmin3, r.fmax3),
                PropRec::of(r.fcode4, r.fparam4, r.fmin4, r.fmax4),
                PropRec::of(r.fcode5, r.fparam5, r.fmin5, r.fmax5),
                PropRec::of(r.fcode6, r.fparam6, r.fmin6, r.fmax6),
                PropRec::of(r.fcode7, r.fparam7, r.fmin7, r.fmax7),
                PropRec::of(r.fcode8, r.fparam8, r.fmin8, r.fmax8),
            ],
        }
    }
}

/// A gems row: three blocks (weapon, helm, shield) of three records.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GemRec {
    pub mods: [[PropRec; 3]; 3],
}

impl From<&Gems> for GemRec {
    fn from(r: &Gems) -> Self {
        GemRec {
            mods: [
                [
                    PropRec::of(
                        r.weaponmod1code,
                        r.weaponmod1param,
                        r.weaponmod1min,
                        r.weaponmod1max,
                    ),
                    PropRec::of(
                        r.weaponmod2code,
                        r.weaponmod2param,
                        r.weaponmod2min,
                        r.weaponmod2max,
                    ),
                    PropRec::of(
                        r.weaponmod3code,
                        r.weaponmod3param,
                        r.weaponmod3min,
                        r.weaponmod3max,
                    ),
                ],
                [
                    PropRec::of(
                        r.helmmod1code,
                        r.helmmod1param,
                        r.helmmod1min,
                        r.helmmod1max,
                    ),
                    PropRec::of(
                        r.helmmod2code,
                        r.helmmod2param,
                        r.helmmod2min,
                        r.helmmod2max,
                    ),
                    PropRec::of(
                        r.helmmod3code,
                        r.helmmod3param,
                        r.helmmod3min,
                        r.helmmod3max,
                    ),
                ],
                [
                    PropRec::of(
                        r.shieldmod1code,
                        r.shieldmod1param,
                        r.shieldmod1min,
                        r.shieldmod1max,
                    ),
                    PropRec::of(
                        r.shieldmod2code,
                        r.shieldmod2param,
                        r.shieldmod2min,
                        r.shieldmod2max,
                    ),
                    PropRec::of(
                        r.shieldmod3code,
                        r.shieldmod3param,
                        r.shieldmod3min,
                        r.shieldmod3max,
                    ),
                ],
            ],
        }
    }
}

/// A runes (runeword) row.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RuneRec {
    pub complete: u8,
    pub server: u8,
    pub itype: [i16; 6],
    pub etype: [i16; 3],
    /// Items combined indices.
    pub runes: [i32; 6],
    pub props: [PropRec; 7],
    /// The name's string id (u16 +0x82, written by the loader's fix-up;
    /// the item bit stream sends it for a runeword).
    pub name_id: u16,
}

impl RuneRec {
    /// From the typed record and its raw bytes (after the fix-ups).
    pub fn from_record(r: &Runes, raw: &[u8]) -> Self {
        RuneRec {
            name_id: u16::from_le_bytes([raw[0x82], raw[0x83]]),
            ..RuneRec::from(r)
        }
    }
}

impl From<&Runes> for RuneRec {
    fn from(r: &Runes) -> Self {
        RuneRec {
            complete: r.complete,
            server: r.server,
            itype: [r.itype1, r.itype2, r.itype3, r.itype4, r.itype5, r.itype6].map(|v| v as i16),
            etype: [r.etype1, r.etype2, r.etype3].map(|v| v as i16),
            runes: [r.rune1, r.rune2, r.rune3, r.rune4, r.rune5, r.rune6].map(|v| v as i32),
            props: [
                PropRec::of(r.t1code1, r.t1param1, r.t1min1, r.t1max1),
                PropRec::of(r.t1code2, r.t1param2, r.t1min2, r.t1max2),
                PropRec::of(r.t1code3, r.t1param3, r.t1min3, r.t1max3),
                PropRec::of(r.t1code4, r.t1param4, r.t1min4, r.t1max4),
                PropRec::of(r.t1code5, r.t1param5, r.t1min5, r.t1max5),
                PropRec::of(r.t1code6, r.t1param6, r.t1min6, r.t1max6),
                PropRec::of(r.t1code7, r.t1param7, r.t1min7, r.t1max7),
            ],
            // Not a typed column: [`RuneRec::from_record`] reads it.
            name_id: 0,
        }
    }
}

/// One properties slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PropSlot {
    pub func: u8,
    pub stat: u16,
    pub set: u8,
    pub val: u16,
}

/// A properties row: its seven slots.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PropertyRec {
    pub slots: [PropSlot; 7],
}

impl From<&Properties> for PropertyRec {
    fn from(r: &Properties) -> Self {
        let s = |func, stat, set, val| PropSlot {
            func,
            stat,
            set,
            val,
        };
        PropertyRec {
            slots: [
                s(r.func1, r.stat1, r.set1, r.val1),
                s(r.func2, r.stat2, r.set2, r.val2),
                s(r.func3, r.stat3, r.set3, r.val3),
                s(r.func4, r.stat4, r.set4, r.val4),
                s(r.func5, r.stat5, r.set5, r.val5),
                s(r.func6, r.stat6, r.set6, r.val6),
                s(r.func7, r.stat7, r.set7, r.val7),
            ],
        }
    }
}

/// The skills columns used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SkillRec {
    pub itypea1: i16,
    pub reqlevel: i32,
    pub maxlvl: i32,
    /// `charclass` (a playerclass row; 0xFF none).
    pub charclass: u8,
}

impl From<&Skills> for SkillRec {
    fn from(r: &Skills) -> Self {
        SkillRec {
            itypea1: r.itypea1 as i16,
            reqlevel: i32::from(r.reqlevel),
            maxlvl: i32::from(r.maxlvl),
            charclass: r.charclass,
        }
    }
}

/// Everything item creation reads from the tables.
#[derive(Clone, Debug, Default)]
pub struct ItemTables {
    /// The combined items array: weapons, armor, misc.
    pub items: Vec<ItemRec>,
    pub itemtypes: Vec<Itemtypes>,
    /// Itemtypes equivalence.
    pub equiv: EquivMatrix,
    pub itemratio: Vec<Itemratio>,
    /// Magic affix array: suffixes, prefixes, automagic.
    pub magic: Vec<AffixRec>,
    pub n_suffix: usize,
    pub n_prefix: usize,
    /// Rare affix array: suffixes then prefixes.
    pub rare: Vec<RareRec>,
    pub n_rare_suffix: usize,
    pub properties: Vec<PropertyRec>,
    /// itemstatcost `valshift` per stat; its length is the stat count.
    pub valshift: Vec<u8>,
    /// itemstatcost save columns per stat; empty
    /// in fixtures that never write a stream (every stat reads as
    /// `Save Bits` 0).
    pub isc: Vec<Isc>,
    /// Layer split: `stuff` and its mask.
    pub stat_shift: u32,
    pub stat_mask: u32,
    pub skills: Vec<SkillRec>,
    pub skill_lists: SkillLists,
    pub books: Vec<([u8; 4], [u8; 4])>,
    pub qualityitems: Vec<QualityRec>,
    pub n_lowquality: usize,
    pub uniques: Vec<UniqueRec>,
    pub setitems: Vec<SetItemRec>,
    pub sets: Vec<SetRec>,
    pub gems: Vec<GemRec>,
    pub runes: Vec<RuneRec>,
    /// The parts of the combined items array (start, count): weapons,
    /// armor, misc; `None`: the part's table is absent.
    pub parts: [Option<(usize, usize)>; 3],
    /// The rules the game runs under: a mod game's item overrides
    /// (`docs/plans/mods/seams.md` MS7, [`super::provider`]) come from
    /// them. [`Ruleset::Original`] from [`ItemTables::from_fixed`]; the
    /// game builder sets a mod game's.
    pub rules: Ruleset,
}

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

impl ItemTables {
    /// Projects the fixed-up table set (`d2_data::fixup::apply`), under
    /// D2RUST's own rules.
    pub fn from_fixed(f: &FixedSet) -> Result<Self, TableError> {
        let mut items: Vec<ItemRec> = typed::<Weapons>(f)?.iter().map(ItemRec::from).collect();
        let n_weapons = items.len();
        items.extend(typed::<Armor>(f)?.iter().map(ItemRec::from));
        let n_armor = items.len() - n_weapons;
        items.extend(typed::<Misc>(f)?.iter().map(ItemRec::from));
        let n_misc = items.len() - n_weapons - n_armor;
        let parts = [
            Some((0, n_weapons)),
            Some((n_weapons, n_armor)),
            Some((n_weapons + n_armor, n_misc)),
        ];
        let mut magic: Vec<AffixRec> = typed::<Magicsuffix>(f)?
            .iter()
            .map(AffixRec::from)
            .collect();
        let n_suffix = magic.len();
        magic.extend(typed::<Magicprefix>(f)?.iter().map(AffixRec::from));
        let n_prefix = magic.len() - n_suffix;
        magic.extend(typed::<Automagic>(f)?.iter().map(AffixRec::from));
        let mut rare: Vec<RareRec> = typed::<Raresuffix>(f)?.iter().map(RareRec::from).collect();
        let n_rare_suffix = rare.len();
        rare.extend(typed::<Rareprefix>(f)?.iter().map(RareRec::from));
        let setitems_raw = get(f, Setitems::TABLE)?;
        let setitems = typed::<Setitems>(f)?
            .iter()
            .zip(setitems_raw.iter())
            .map(|(r, raw)| SetItemRec::from_record(r, raw))
            .collect();
        let sets_raw = get(f, Sets::TABLE)?;
        let sets = typed::<Sets>(f)?
            .iter()
            .zip(sets_raw.iter())
            .map(|(r, raw)| SetRec::from_record(r, raw))
            .collect();
        // Uniques read their `rarity` column as 32 bits.
        let uniques_raw = get(f, Uniqueitems::TABLE)?;
        let uniques = typed::<Uniqueitems>(f)?
            .iter()
            .zip(uniques_raw.iter())
            .map(|(r, raw)| UniqueRec::from_record(r, raw))
            .collect();
        Ok(ItemTables {
            items,
            itemtypes: typed::<Itemtypes>(f)?,
            equiv: f.itemtypes_equiv.clone(),
            itemratio: typed::<Itemratio>(f)?,
            magic,
            n_suffix,
            n_prefix,
            rare,
            n_rare_suffix,
            properties: typed::<Properties>(f)?
                .iter()
                .map(PropertyRec::from)
                .collect(),
            valshift: typed::<Itemstatcost>(f)?
                .iter()
                .map(|r| r.valshift)
                .collect(),
            isc: typed::<Itemstatcost>(f)?
                .iter()
                .map(|r| Isc {
                    valshift: r.valshift,
                    save_bits: r.save_bits,
                    save_add: r.save_add,
                    save_param_bits: r.save_param_bits,
                })
                .collect(),
            stat_shift: f.stat_stuff,
            stat_mask: f.stat_mask,
            skills: typed::<Skills>(f)?.iter().map(SkillRec::from).collect(),
            skill_lists: f.skill_lists.clone(),
            books: typed::<Books>(f)?
                .iter()
                .map(|b| (b.scrollspellcode, b.bookspellcode))
                .collect(),
            qualityitems: typed::<Qualityitems>(f)?
                .iter()
                .map(QualityRec::from)
                .collect(),
            n_lowquality: typed::<Lowqualityitems>(f)?.len(),
            uniques,
            setitems,
            sets,
            gems: typed::<Gems>(f)?.iter().map(GemRec::from).collect(),
            runes: typed::<Runes>(f)?
                .iter()
                .zip(get(f, Runes::TABLE)?.iter())
                .map(|(r, raw)| RuneRec::from_record(r, raw))
                .collect(),
            parts,
            rules: Ruleset::Original,
        })
    }

    pub fn item(&self, i: usize) -> Option<&ItemRec> {
        self.items.get(i)
    }

    /// Itemtypes row `t` (`t` < 0 or out of range → none).
    pub fn itemtype(&self, t: i16) -> Option<&Itemtypes> {
        usize::try_from(t).ok().and_then(|t| self.itemtypes.get(t))
    }

    /// Code lookup: a 4-byte,
    /// space-padded item code to the combined items index; an unknown
    /// code is not found.
    pub fn find_code(&self, code: [u8; 4]) -> Option<usize> {
        self.items.iter().position(|r| r.code == code)
    }

    /// The primary type's itemtypes row of item `i`.
    pub fn itype_of(&self, i: usize) -> Option<&Itemtypes> {
        self.item(i).and_then(|r| self.itemtype(r.type_))
    }

    /// "Item is type T".
    pub fn is_type(&self, i: usize, t: i16) -> bool {
        let Some(r) = self.item(i) else { return false };
        let eq = |a: i16| a >= 0 && t >= 0 && self.equiv.get(a as usize, t as usize);
        eq(r.type_) || (r.type2 > 0 && eq(r.type2))
    }

    /// Combined index of the first automagic row.
    pub fn first_auto(&self) -> usize {
        self.n_suffix + self.n_prefix
    }

    /// Valid stat id.
    pub fn stat_valid(&self, s: u16) -> bool {
        usize::from(s) < self.valshift.len()
    }

    /// The class's skill count and first skill.
    pub fn class_skills(&self, class: usize) -> (u32, Option<u16>) {
        let sl = &self.skill_lists;
        match sl.counts.get(class) {
            Some(&n) if n > 0 => (n, sl.lists.get(class * sl.max).copied()),
            Some(&n) => (n, None),
            None => (0, None),
        }
    }
}
