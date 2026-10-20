//! The tables the tool reads, all from the user's own install (or the
//! synthetic one in tests): the fixed-up set (`d2_data::fixup::apply`),
//! the item and inventory projections of `d2-sim`, the vitals tables
//! (charstats, experience) and the itemstatcost save columns.
//!
//! [`Tables`] implements [`SaveTables`]: `CSvBits` / `CSvParam` /
//! `CSvSigned` come from the loaded `itemstatcost` rows (the save code
//! reads these columns from the loaded table), and item entry lengths
//! from `d2_proto::item_bits::save_entry_len` over [`TablesLookup`].

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use d2_data::bin::{self, BinSet};
use d2_data::fixup::{self, FixedSet};
use d2_data::tables::{decode_all, Itemstatcost, Levels, Record};
use d2_formats::d2s::{SaveTables, StatSave};
use d2_formats::mpq::ArchiveSet;
use d2_proto::item_bits::{save_entry_len, SaveEntry};
use d2_server::adapters::item_bits::TablesLookup;
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::items::inventory::InvTables;
use d2_sim::items::ItemTables;

/// Everything loaded from one install.
#[derive(Debug)]
pub struct Tables {
    pub fixed: FixedSet,
    pub items: ItemTables,
    pub inv: InvTables,
    /// Charstats and experience; `None`
    /// when either table does not decode.
    pub vitals: Option<VitalsTables>,
    /// `CSvBits`, `CSvParam`, `CSvSigned` per itemstatcost row.
    pub isc_save: Vec<StatSave>,
    /// Every waypoint index a `levels` row carries (255 = none),
    /// ascending, without duplicates.
    pub waypoint_indices: Vec<u8>,
    /// The waypoint index of each `levels` row (row = level id; 255 =
    /// none; read directly).
    pub level_waypoint: Vec<u8>,
}

fn typed<T: Record>(f: &FixedSet) -> Result<Vec<T>> {
    let t = f
        .table(T::TABLE)
        .ok_or_else(|| anyhow!("table {} missing", T::TABLE))?;
    decode_all::<T>(t).map_err(|e| anyhow!("{}: {e}", T::TABLE))
}

impl Tables {
    /// Loads the install in `dir` (`bin::load`, `AnimData.d2`, fix-ups).
    pub fn load_dir(dir: &Path) -> Result<Self> {
        let set = ArchiveSet::open_dir(dir).map_err(|e| anyhow!("{}: {e}", dir.display()))?;
        let bins = bin::load(&set, bin::DEFAULT_LANGUAGE).map_err(|e| anyhow!("load: {e}"))?;
        let anim = fixup::read_animdata(&set).map_err(|e| anyhow!("AnimData.d2: {e}"))?;
        Self::from_loaded(&bins, &anim)
    }

    /// From a loaded set and its `AnimData.d2`.
    pub fn from_loaded(bins: &BinSet, anim: &d2_formats::animdata::AnimData) -> Result<Self> {
        let fixed = fixup::apply(bins, anim).map_err(|e| anyhow!("fixup: {e}"))?;
        Self::from_fixed(fixed)
    }

    /// From a fixed-up set.
    pub fn from_fixed(fixed: FixedSet) -> Result<Self> {
        let items = ItemTables::from_fixed(&fixed).map_err(|e| anyhow!("item tables: {e}"))?;
        let inv = InvTables::from_fixed(&fixed).map_err(|e| anyhow!("inventory tables: {e}"))?;
        let vitals = match (typed(&fixed), typed(&fixed)) {
            (Ok(charstats), Ok(experience)) => Some(VitalsTables {
                charstats,
                experience,
            }),
            _ => None,
        };
        let isc_save = typed::<Itemstatcost>(&fixed)
            .context("itemstatcost")?
            .iter()
            .map(|r| StatSave {
                bits: r.csvbits,
                param: r.csvparam,
                signed: r.csvsigned,
            })
            .collect();
        let level_waypoint: Vec<u8> = typed::<Levels>(&fixed)
            .context("levels")?
            .iter()
            .map(|r| r.waypoint)
            .collect();
        let mut waypoint_indices: Vec<u8> = level_waypoint
            .iter()
            .copied()
            .filter(|&w| w != 255)
            .collect();
        waypoint_indices.sort_unstable();
        waypoint_indices.dedup();
        Ok(Tables {
            fixed,
            items,
            inv,
            vitals,
            isc_save,
            waypoint_indices,
            level_waypoint,
        })
    }

    /// The decoded item entry at the start of `buf` (item, padding,
    /// socketed children).
    pub fn decode_entry(&self, buf: &[u8]) -> Result<SaveEntry, String> {
        save_entry_len(buf, &TablesLookup(&self.items)).map_err(|e| e.to_string())
    }

    /// The class skill list of `class`.
    pub fn class_skills(&self, class: u8) -> &[u16] {
        let l = &self.fixed.skill_lists;
        let c = usize::from(class);
        match l.counts.get(c) {
            Some(&n) => &l.lists[c * l.max..c * l.max + n as usize],
            None => &[],
        }
    }

    /// The largest class skill count (header +0x2A; 30 with the stock
    /// tables).
    pub fn max_skill_count(&self) -> usize {
        self.fixed.skill_lists.max
    }
}

impl SaveTables for Tables {
    fn stat_save(&self, id: u16) -> Option<StatSave> {
        self.isc_save.get(usize::from(id)).copied()
    }

    fn item_entry_len(&self, buf: &[u8]) -> Result<usize, String> {
        self.decode_entry(buf).map(|e| e.len)
    }
}
