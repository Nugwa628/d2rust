//! The post-load fix-ups: bytes the loaders fill or correct after a
//! `.bin` table loads, and the runtime maps (lookup lists, bit matrices,
//! name links) built from the fixed-up records. [`apply`] runs them over
//! a copy of the loaded set and returns a [`FixedSet`] that the simulation
//! and server read. [`crate::bin::load`] keeps the shipped bytes (the
//! txt → bin cross-check compares those).
//!
//! Submodules: [`records`] (record bytes), [`maps`] (runtime maps),
//! [`text`] (wide text, tile paths), [`qsort`] (the CRT sort).

/// Runtime maps built from the fixed-up tables.
pub mod maps;
/// The C runtime's `qsort`, reproduced so equal keys sort the same way.
pub mod qsort;
/// Record-byte fix-ups.
pub mod records;
/// Wide text and tile path helpers.
pub mod text;

use d2_formats::animdata::{self, AnimData};
use d2_formats::mpq::ArchiveSet;

use crate::bin::{cstr, item_code_map, u32_at, BinSet, BinTable};
use crate::compile::{code4, special_linker, CodeLinker, NameLinker};
use crate::schema::CalcBuffer;
use crate::strings::StringTables;
use maps::{ActRanges, Automap, EquivKind, EquivMatrix, Gamble, SeqEntry, SkillLists, StateMaps};

/// String id used when a unique or set item name is missing from the string tables.
pub const MISSING_ITEM_NAME: u16 = 5383;
/// Number of hcIdx values the superunique map covers.
pub const HC_INDICES: usize = 66;
/// Number of superunique rows read.
pub const SUPERUNIQUE_ROWS: usize = 512;
/// Maximum number of skills one pettype record lists.
pub const PETTYPE_SKILLS: usize = 15;

/// Fix-ups not applied yet, with what each waits for. Empty: every known
/// fix-up is implemented.
pub const PENDING: &[(&str, &str)] = &[];

/// The fixed-up tables and the runtime maps.
#[derive(Debug, Clone)]
pub struct FixedSet {
    /// The 73 record tables, fixed up, in load order. `gamble` and
    /// `automap` are kept after conversion (gamble with its +0x04/+0x08
    /// writes).
    pub tables: Vec<BinTable>,
    /// The itemtypes code link, rebuilt from the records.
    pub item_types: CodeLinker,
    /// The item code map over weapons, armor, misc.
    pub item_codes: CodeLinker,
    /// Unique and set item name links (add-always, record order).
    pub uniques: NameLinker,
    pub sets: NameLinker,
    /// hcIdx → first superunique row holding it.
    pub superunique_hc: [Option<u16>; HC_INDICES],
    /// itemstatcost record 0 `stuff` (+0x140), 6 when outside 1–8, and
    /// its mask.
    pub stat_stuff: u32,
    /// Mask of the low `stat_stuff` bits: `(1 << stat_stuff) - 1`.
    pub stat_mask: u32,
    /// Type-equivalence matrix of itemtypes.
    pub itemtypes_equiv: EquivMatrix,
    /// Type-equivalence matrix of montype.
    pub montype_equiv: EquivMatrix,
    /// itemstatcost description list.
    pub stat_desc_list: Vec<u16>,
    /// states bitsets and lists.
    pub states: StateMaps,
    /// skills class and passive lists.
    pub skill_lists: SkillLists,
    /// Version-0 item list.
    pub version0_items: Vec<u16>,
    /// gamble index and thresholds.
    pub gamble: Gamble,
    /// monseq index entries.
    pub monseq: Vec<SeqEntry>,
    /// monpreset act ranges.
    pub monpreset: ActRanges,
    /// hireling id tables: first row per Id for version < 100 and for
    /// version ≥ 100, −1 when none.
    pub hireling_first: [[i32; maps::HIRELING_IDS]; 2],
    /// leveldefs portal list.
    pub portals: Vec<u32>,
    /// lvlsub first row per type.
    pub lvlsub_types: Vec<u32>,
    /// automap converted records and ranges.
    pub automap: Automap,
    /// The items code buffer (`itemscode`),
    /// as loaded: the `calc` fields of weapons, armor and misc are byte
    /// offsets into it. Empty when the set has none.
    pub items_code: Vec<u8>,
}

impl FixedSet {
    /// The fixed-up table called `name`, if the set has it.
    pub fn table(&self, name: &str) -> Option<&BinTable> {
        self.tables.iter().find(|t| t.name == name)
    }
}

/// A fix-up could not run (a table it needs is absent, a name has a
/// byte ≥ 0x80, or data that would make a fix-up read or write outside an
/// array).
#[derive(Debug, thiserror::Error)]
#[error("{table}: {detail}")]
pub struct FixupError {
    /// Table (or file) the failure is in.
    pub table: String,
    /// What went wrong.
    pub detail: String,
}

fn err(table: &str, detail: impl Into<String>) -> FixupError {
    FixupError {
        table: table.to_owned(),
        detail: detail.into(),
    }
}

fn set_u16(r: &mut [u8], o: usize, v: u16) {
    r[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

fn get_u16(r: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([r[o], r[o + 1]])
}

fn i16_at(r: &[u8], o: usize) -> i16 {
    get_u16(r, o) as i16
}

fn i32_at(r: &[u8], o: usize) -> i32 {
    u32_at(r, o) as i32
}

fn rec(t: &mut BinTable, k: usize) -> &mut [u8] {
    let size = t.record_size;
    &mut t.records[k * size..(k + 1) * size]
}

fn records_mut(t: &mut BinTable) -> impl Iterator<Item = &mut [u8]> {
    t.records.chunks_exact_mut(t.record_size.max(1))
}

/// `name` (NUL-terminated at `from`) → string id at `to` (u16), with the
/// given miss value.
fn name_id(t: &mut BinTable, strings: &StringTables, from: usize, to: usize, miss: u16) {
    let end = t.record_size;
    for r in records_mut(t) {
        let id = match strings.id(cstr(r, from..end)) {
            0 => miss,
            id => id as u16,
        };
        set_u16(r, to, id);
    }
}

/// Reads and parses `AnimData.d2` from the archive set.
pub fn read_animdata(set: &ArchiveSet) -> Result<AnimData, FixupError> {
    let bytes = set
        .read(animdata::PATH)
        .map_err(|e| err("AnimData.d2", e.to_string()))?;
    AnimData::parse(&bytes).map_err(|e| err("AnimData.d2", e.to_string()))
}

/// Applies the fix-ups of every table and builds the runtime maps, in
/// load order. `anim` is the loaded `AnimData.d2` (monstats speeds).
pub fn apply(data: &BinSet, anim: &AnimData) -> Result<FixedSet, FixupError> {
    let mut tables = data.tables.clone();
    let strings = &data.strings;
    let index = |tables: &[BinTable], n: &str| {
        tables
            .iter()
            .position(|t| t.name == n)
            .ok_or_else(|| err(n, "table not loaded"))
    };
    let items = |tables: &[BinTable]| -> Vec<BinTable> {
        ["weapons", "armor", "misc"]
            .iter()
            .filter_map(|n| tables.iter().find(|t| t.name == *n).cloned())
            .collect()
    };
    let mut out = FixedSet {
        tables: Vec::new(),
        item_types: CodeLinker::default(),
        item_codes: CodeLinker::default(),
        uniques: NameLinker::default(),
        sets: NameLinker::default(),
        superunique_hc: [None; HC_INDICES],
        stat_stuff: 0,
        stat_mask: 0,
        itemtypes_equiv: EquivMatrix::default(),
        montype_equiv: EquivMatrix::default(),
        stat_desc_list: Vec::new(),
        states: StateMaps::default(),
        skill_lists: SkillLists::default(),
        version0_items: Vec::new(),
        gamble: Gamble {
            index: None,
            thresholds: [0; maps::GAMBLE_LEVELS],
        },
        monseq: Vec::new(),
        monpreset: ActRanges::default(),
        hireling_first: [[-1; maps::HIRELING_IDS]; 2],
        portals: Vec::new(),
        lvlsub_types: Vec::new(),
        automap: Automap {
            records: Vec::new(),
            ranges: Vec::new(),
        },
        items_code: data
            .code
            .get(&CalcBuffer::ItemsCode)
            .map(|c| c.bytes.clone())
            .unwrap_or_default(),
    };
    for i in 0..tables.len() {
        let (earlier, rest) = tables.split_at_mut(i);
        let t = &mut rest[0];
        match t.name.as_str() {
            "itemtypes" => {
                for r in t.iter() {
                    out.item_types.add(u32::from_le_bytes(code4(&r[..4])));
                }
                out.itemtypes_equiv = maps::equiv_matrix(t, EquivKind::ItemTypes)?;
            }
            "montype" => out.montype_equiv = maps::equiv_matrix(t, EquivKind::MonType)?,
            "itemstatcost" => {
                let stuff = t.iter().next().map_or(0, |r| i32_at(r, 0x140));
                out.stat_stuff = if (1..=8).contains(&stuff) {
                    stuff as u32
                } else {
                    6
                };
                out.stat_mask = (1 << out.stat_stuff) - 1;
                records::stat_ops(t);
                out.stat_desc_list = maps::desc_list(t);
            }
            "missiles" => {
                for r in records_mut(t) {
                    r[0x183] = r[0x183].min(8);
                }
            }
            "states" => out.states = maps::states(t),
            "skills" => {
                out.skill_lists = maps::skill_lists(t);
                let p = index(earlier, "pettype")?;
                append_pet_skills(t, &mut earlier[p]);
            }
            "charstats" => records::charstats(t, strings)?,
            "magicsuffix" | "magicprefix" | "automagic" | "lowqualityitems" => {
                name_id(t, strings, 0x00, 0x20, 0)
            }
            "raresuffix" | "rareprefix" => name_id(t, strings, 0x26, 0x0C, 0),
            "runes" => name_id(t, strings, 0x00, 0x82, 0),
            "qualityitems" => {
                name_id(t, strings, 0x2C, 0x6C, 0);
                name_id(t, strings, 0x4C, 0x6E, 0);
            }
            "uniqueitems" | "setitems" => {
                let unique = t.name == "uniqueitems";
                for (n, r) in records_mut(t).enumerate() {
                    set_u16(r, 0, n as u16);
                }
                let to = if unique { 0x22 } else { 0x24 };
                name_id(t, strings, 0x02, to, MISSING_ITEM_NAME);
                let (linker, _) = special_linker(t.iter(), 40, if unique { 52 } else { 48 })
                    .ok_or_else(|| err(&t.name, "item name byte >= 0x80"))?;
                if unique {
                    out.uniques = linker;
                } else {
                    out.sets = linker;
                    let s = index(earlier, "sets")?;
                    records::attach_set_items(t, &mut earlier[s])?;
                }
            }
            "gems" => records::gems(t, earlier, strings)?,
            "gamble" => {
                let items = items(earlier);
                let refs: Vec<&BinTable> = items.iter().collect();
                out.gamble = maps::gamble(t, &out.item_codes, &refs)?;
            }
            "monseq" => out.monseq = maps::monseq(t)?,
            "monstats" => {
                let m2 = index(earlier, "monstats2")?;
                let mm = index(earlier, "monmode")?;
                records::monstats_chains(t)?;
                records::monstats_speeds(t, &earlier[m2], &earlier[mm], anim)?;
            }
            "monumod" => records::clamp_monumod(t),
            "superuniques" => {
                for (n, r) in t.iter().take(SUPERUNIQUE_ROWS).enumerate() {
                    let hc = u32_at(r, 0x08) as usize;
                    if hc < HC_INDICES && out.superunique_hc[hc].is_none() {
                        out.superunique_hc[hc] = Some(n as u16);
                    }
                }
            }
            "monpreset" => out.monpreset = maps::monpreset(t)?,
            "hireling" => {
                name_id(t, strings, 0xD3, 0x114, 0);
                name_id(t, strings, 0xF3, 0x116, 0);
                out.hireling_first = maps::hireling_first(t)?;
            }
            "monequip" => {
                let m = index(earlier, "monstats")?;
                records::link_monequip(t, &mut earlier[m], &out.item_codes);
            }
            "levels" => records::levels(t, strings)?,
            "leveldefs" => out.portals = maps::portals(t),
            "lvltypes" | "lvlprest" => records::tile_paths(t, data.lod)?,
            "lvlsub" => {
                out.lvlsub_types = maps::lvlsub_types(t)?;
                records::tile_paths(t, data.lod)?;
            }
            "automap" => out.automap = maps::automap(t)?,
            "objects" => records::objects(t, strings)?,
            _ => {}
        }
        if t.name == "misc" {
            out.item_codes = item_code_map(&tables[..=i]);
            let items = items(&tables[..=i]);
            let refs: Vec<&BinTable> = items.iter().collect();
            out.version0_items = maps::version0_items(&refs);
        }
    }
    out.tables = tables;
    Ok(out)
}

/// skills → pettype: each skill whose i8 `pettype`
/// (+0xBE) names a pettype record is appended to that record's list (u32
/// count +0xBC, u16 skill indices from +0xC0), at most 15 per record.
fn append_pet_skills(skills: &BinTable, pettype: &mut BinTable) {
    for (s, r) in skills.iter().enumerate() {
        let p = r[0xBE] as i8;
        if p < 0 || p as usize >= pettype.count {
            continue;
        }
        let rec = rec(pettype, p as usize);
        let n = u32_at(rec, 0xBC) as usize;
        if n < PETTYPE_SKILLS {
            set_u16(rec, 0xC0 + 2 * n, s as u16);
            rec[0xBC..0xC0].copy_from_slice(&(n as u32 + 1).to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests;
