//! Table views built from the fixed-up table set (`d2_data::fixup::FixedSet`)
//! for the server: the level types, hirelings, drops and saves.
//!
//! [`LevelTables`] gathers each level view's own `from_tables` /
//! `from_record`, plus the preset counts (monstats and superuniques row
//! counts, monpreset rows and act ranges, the item class of `hdm `). Also
//! here: the hireling tables of the interaction desk ([`hireling_tables`]),
//! the drop tables of the treasure walk ([`drop_tables`],
//! `ActionHooks::object_drops`), the tables of the `.d2s` reader and writer
//! ([`SaveData`], `d2s::SaveTables`) and the columns of the save's
//! appearance bytes ([`appearance_tables`]). The server's world loader and
//! its character save code call these; a missing or malformed table is an
//! error naming it.

use d2_data::bin::BinTable;
use d2_data::fixup::FixedSet;
use d2_data::tables::{
    decode_all, Armor, Armtype, Automagic, Gems, Itemstatcost, Itemtypes, Leveldefs, Lvlmaze,
    Lvlprest, Lvlsub, Lvltypes, Lvlwarp, Magicprefix, Magicsuffix, Misc, Objects, Record, Setitems,
    States, Superuniques, Treasureclassex, Uniqueitems, Weapons,
};
use d2_formats::d2s::appearance::{
    link8_signed, AppearanceTables, ColourTables, IsA, ItemGfx, ReferenceSlots, StateColour,
};
use d2_formats::d2s::{self, Hireling, StatSave};
use d2_sim::drlg::maze::MazeData;
use d2_sim::drlg::outdoor::OutdoorData;
use d2_sim::drlg::preset::{MonPresetRow, PresetData, PresetDef, PresetTables};
use d2_sim::drlg::DrlgData;
use d2_sim::items::bitstream::read::read_save_entry;
use d2_sim::items::ItemTables;
use d2_sim::treasure::class_pick::ClassPicks;
use d2_sim::treasure::{item_list, TcSources, TreasureClasses};
use d2_sim::wiring::economy::DropTables;
use d2_sim::world::hirelings::{HirelingRows, HirelingTables};

use super::WorldDataError;

/// The item code the one-entry DS1 item table holds.
pub const HDM_CODE: [u8; 4] = *b"hdm ";

/// The four table views of the level types.
#[derive(Clone, Debug)]
pub struct LevelTables {
    /// Level definitions, warps and level types.
    pub drlg: DrlgData,
    /// Preset (lvlprest) definitions and the preset counts.
    pub preset: PresetData,
    /// Outdoor level data (lvlsub).
    pub outdoor: OutdoorData,
    /// Maze level data (lvlmaze).
    pub maze: MazeData,
}

impl LevelTables {
    /// Builds the four views from the fixed-up set; a missing or malformed
    /// table is an error.
    pub fn from_fixed(set: &FixedSet) -> Result<Self, WorldDataError> {
        let leveldefs: Vec<Leveldefs> = records(set)?;
        let lvlprest: Vec<Lvlprest> = records(set)?;
        let lvlsub: Vec<Lvlsub> = records(set)?;
        let lvlmaze: Vec<Lvlmaze> = records(set)?;
        let lvltypes: Vec<Lvltypes> = records(set)?;
        let lvlwarp: Vec<Lvlwarp> = records(set)?;
        let objects: Vec<Objects> = records(set)?;
        let drlg = DrlgData::from_tables(&leveldefs, &lvlwarp, &lvltypes, &objects);
        let monpreset = table(set, "monpreset")?
            .iter()
            .map(|r| {
                <[u8; 4]>::try_from(r)
                    .map(|b| MonPresetRow::from_record_bytes(&b))
                    .map_err(|_| table_err("monpreset", "record is not 4 bytes"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let preset = PresetData {
            defs: lvlprest.iter().map(PresetDef::from_record).collect(),
            monpreset_acts: set.monpreset,
            monpreset,
            monstats_count: table(set, "monstats")?.count as u32,
            superuniques_count: table(set, "superuniques")?.count as u32,
            hdm_item: set
                .item_codes
                .find(u32::from_le_bytes(HDM_CODE))
                .map_or(-1, |i| i as i32),
            tables: PresetTables::spec().map_err(|e| table_err("preset-tables.tsv", e))?,
        };
        Ok(Self {
            outdoor: OutdoorData::from_tables(&leveldefs, &lvlprest, &lvlsub),
            maze: MazeData::from_tables(&lvlmaze, &lvlprest),
            drlg,
            preset,
        })
    }
}

/// The hireling tables (`hireling` rows, `pettype` row 7, `experience`
/// `MaxLvl` and `ExpRatio`) from the fixed-up set, for
/// `InteractionState::hireling_tables`. A missing or malformed table is an
/// error.
pub fn hireling_tables(set: &FixedSet) -> Result<HirelingTables, WorldDataError> {
    hireling_tables_by(|name| set.table(name))
}

/// [`hireling_tables`] over a table lookup.
pub fn hireling_tables_by<'a>(
    lookup: impl Fn(&str) -> Option<&'a BinTable>,
) -> Result<HirelingTables, WorldDataError> {
    let get = |name: &str| lookup(name).ok_or_else(|| table_err(name, "not loaded"));
    HirelingTables::from_tables(get("hireling")?, get("pettype")?, get("experience")?)
        .map_err(|e| table_err("hireling", e))
}

/// The drop tables of the treasure walk (the drop state of
/// `ActionHooks::object_drops`, `d2_sim::wiring::economy::DeathDrops`):
/// the item tables (`ItemTables::from_fixed`), the runtime treasure
/// classes (`TreasureClasses::build` over
/// `treasureclassex`, `itemtypes`, the item list, the itemtypes
/// equivalence, `uniqueitems` and `setitems`), the item list in its
/// index order (`item_list`: weapons, armor, misc) and the
/// `superuniques` rows. A missing or malformed table, or treasure
/// classes that do not build, is an error.
pub fn drop_tables(set: &FixedSet) -> Result<DropTables, WorldDataError> {
    let items = ItemTables::from_fixed(set).map_err(|e| table_err("items", e))?;
    let treasure_items = item_list(
        &records::<Weapons>(set)?,
        &records::<Armor>(set)?,
        &records::<Misc>(set)?,
    );
    let tcs = TreasureClasses::build(&TcSources {
        treasureclassex: &records::<Treasureclassex>(set)?,
        itemtypes: &records::<Itemtypes>(set)?,
        items: &treasure_items,
        equiv: &set.itemtypes_equiv,
        uniqueitems: &records::<Uniqueitems>(set)?,
        setitems: &records::<Setitems>(set)?,
    })
    .map_err(|e| table_err(Treasureclassex::TABLE, e))?;
    Ok(DropTables {
        items,
        tcs,
        treasure_items,
        superuniques: records::<Superuniques>(set)?,
    })
}

/// The pick columns of the combined items array (weapons, armor, misc)
/// the object and quest drop helpers read: a host sets them on its drop
/// state (`DeathDrops::with_picks`); without them the gold-code lookup finds
/// nothing and no object or quest drop is made.
pub fn class_picks(set: &FixedSet) -> Result<ClassPicks, WorldDataError> {
    Ok(ClassPicks::new(
        &records::<Weapons>(set)?,
        &records::<Armor>(set)?,
        &records::<Misc>(set)?,
    ))
}

/// The tables the `.d2s` reader and writer read (`d2s::SaveTables`): the
/// `itemstatcost` save columns (`CSvBits`, `CSvParam`, `CSvSigned`), the
/// item tables of the item stream (`read_save_entry`: an entry's length
/// with its socketed children) and the hireling rows of the restore test
/// (the hireling is present and its row is found: `Id` at level 1 in the
/// game's version).
#[derive(Clone, Debug)]
pub struct SaveData {
    /// The save columns of every stat, by stat id.
    pub stats: Vec<StatSave>,
    /// The item tables of the item stream.
    pub items: ItemTables,
    /// The hireling rows.
    pub hirelings: HirelingRows,
    /// Expansion game: the hireling rows of the expansion version.
    pub expansion: bool,
}

impl SaveData {
    /// From the fixed-up set for a game of `expansion`; a missing or
    /// malformed table is an error.
    pub fn from_fixed(set: &FixedSet, expansion: bool) -> Result<Self, WorldDataError> {
        let stats = records::<Itemstatcost>(set)?
            .iter()
            .map(|r| StatSave {
                bits: r.csvbits,
                param: r.csvparam,
                signed: r.csvsigned,
            })
            .collect();
        Ok(Self {
            stats,
            items: ItemTables::from_fixed(set).map_err(|e| table_err("items", e))?,
            hirelings: HirelingRows::from_table(table(set, "hireling")?)
                .map_err(|e| table_err("hireling", e))?,
            expansion,
        })
    }
}

impl d2s::SaveTables for SaveData {
    fn stat_save(&self, id: u16) -> Option<StatSave> {
        self.stats.get(usize::from(id)).copied()
    }
    fn item_entry_len(&self, buf: &[u8]) -> Result<usize, String> {
        read_save_entry(buf, &self.items)
            .map(|e| e.len)
            .map_err(|e| e.to_string())
    }
    fn hireling_restored(&self, h: &Hireling) -> bool {
        h.is_present()
            && self
                .hirelings
                .row_at(self.expansion, u32::from(h.id), 1)
                .is_some()
    }
}

macro_rules! item_gfx {
    ($r:expr) => {{
        let r = $r;
        ItemGfx {
            code: r.code,
            alternategfx: r.alternategfx,
            item_type: i32::from(r.type_ as i16),
            component: r.component,
            arm: [r.torso, r.legs, r.rarm, r.larm, r.rspad, r.lspad],
            hasinv: r.hasinv,
            transform: r.transform,
            gemoffset: r.gemoffset as i32,
            wclass: r.wclass,
            wclass2: r.f_2handedwclass,
        }
    }};
}

/// The columns of the save's appearance bytes from the fixed-up set: the
/// items records in record order (weapons, armor, misc), the `armtype` tokens, the itemtypes is-a
/// matrix, the colour columns (magic affixes in the combined order
/// suffixes, prefixes, automagic; `setitems`, `uniqueitems`, `gems`) and
/// the state-colour list (every `states` row whose `itemtype`, read as a
/// signed 16-bit value, is > 0, in row order). The token table is built
/// over `reference`. A missing or malformed table is an error.
pub fn appearance_tables(
    set: &FixedSet,
    reference: &ReferenceSlots,
) -> Result<AppearanceTables, WorldDataError> {
    let mut items: Vec<ItemGfx> = records::<Weapons>(set)?
        .iter()
        .map(|r| item_gfx!(r))
        .collect();
    items.extend(records::<Armor>(set)?.iter().map(|r| item_gfx!(r)));
    items.extend(records::<Misc>(set)?.iter().map(|r| item_gfx!(r)));
    let armtype = records::<Armtype>(set)?.iter().map(|r| r.token).collect();
    let eq = &set.itemtypes_equiv;
    let types = IsA::from_fn(eq.n, |i, j| eq.get(i, j));
    let mut affix: Vec<i32> = records::<Magicsuffix>(set)?
        .iter()
        .map(|r| link8_signed(r.transformcolor))
        .collect();
    affix.extend(
        records::<Magicprefix>(set)?
            .iter()
            .map(|r| link8_signed(r.transformcolor)),
    );
    affix.extend(
        records::<Automagic>(set)?
            .iter()
            .map(|r| link8_signed(r.transformcolor)),
    );
    let states = records::<States>(set)?
        .iter()
        .enumerate()
        .filter(|(_, r)| (r.itemtype as i16) > 0)
        .map(|(row, r)| StateColour {
            state: row as u32,
            item_type: i32::from(r.itemtype as i16),
            itemtrans: link8_signed(r.itemtrans),
        })
        .collect();
    let colours = ColourTables {
        affix,
        set: records::<Setitems>(set)?
            .iter()
            .map(|r| link8_signed(r.chrtransform))
            .collect(),
        unique: records::<Uniqueitems>(set)?
            .iter()
            .map(|r| link8_signed(r.chrtransform))
            .collect(),
        gem: records::<Gems>(set)?
            .iter()
            .map(|r| i32::from(r.transform))
            .collect(),
        states,
    };
    Ok(AppearanceTables::new(
        items, armtype, types, colours, reference,
    ))
}

fn table_err(table: &str, detail: impl ToString) -> WorldDataError {
    WorldDataError::Table {
        table: table.to_owned(),
        detail: detail.to_string(),
    }
}

fn table<'a>(set: &'a FixedSet, name: &str) -> Result<&'a BinTable, WorldDataError> {
    set.table(name).ok_or_else(|| table_err(name, "not loaded"))
}

fn records<T: Record>(set: &FixedSet) -> Result<Vec<T>, WorldDataError> {
    decode_all(table(set, T::TABLE)?).map_err(|e| table_err(T::TABLE, e))
}
