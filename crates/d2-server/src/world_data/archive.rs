//! Where the server's world data comes from: the player's MPQ archives or a
//! converted native folder.
//!
//! Reads the fixed-up table set and the DRLG files (DS1 / DT1) through
//! `d2_formats::mpq::ArchiveSet`, or the same table set and typed files from
//! a native folder ([`fixed_tables_native`], [`load_native`]). The results
//! feed [`super::WorldFiles`] and
//! [`super::tables::LevelTables`], which the server hands to the
//! sim's level generator.

use d2_data::bin::{self, TableFiles};
use d2_data::fixup::{self, FixedSet};
use d2_formats::animdata::AnimData;
use d2_formats::mpq::ArchiveSet;
use d2_native::source::{NativeAsset, NativeSource};

use super::tables::LevelTables;
use super::{WorldDataError, WorldFiles};

/// A reader for [`WorldFiles::load`] over an archive set.
pub fn reader(set: &ArchiveSet) -> impl FnMut(&str) -> Result<Vec<u8>, String> + '_ {
    move |name| set.read(name).map_err(|e| e.to_string())
}

/// Loads, fixes up and returns the live table set (`bin::load`,
/// `AnimData.d2`, `fixup::apply`).
pub fn fixed_tables(set: &ArchiveSet) -> Result<FixedSet, WorldDataError> {
    let anim = fixup::read_animdata(set).map_err(|e| err("AnimData.d2", e.to_string()))?;
    fixed_tables_from(set, &anim)
}

fn err(table: &str, detail: String) -> WorldDataError {
    WorldDataError::Table {
        table: table.to_owned(),
        detail,
    }
}

/// [`fixed_tables`] over any table source and a loaded `AnimData.d2`.
pub fn fixed_tables_from(
    tables: &dyn TableFiles,
    anim: &AnimData,
) -> Result<FixedSet, WorldDataError> {
    let data =
        bin::load_from(tables, bin::DEFAULT_LANGUAGE).map_err(|e| err("bin", e.to_string()))?;
    fixup::apply(&data, anim).map_err(|e| err("fixup", e.to_string()))
}

/// The fixed-up table set from a native folder: the native `.txt` compiled
/// (mod patches and overrides applied) and the native `animdata.d2.tsv`.
pub fn fixed_tables_native(src: &NativeSource) -> Result<FixedSet, WorldDataError> {
    let tables = src
        .tables(bin::DEFAULT_LANGUAGE)
        .map_err(|e| err("bin", e.to_string()))?;
    let path = "data/global/animdata.d2";
    let anim = match src.read_native(path) {
        Some(Ok(NativeAsset::AnimData(a))) => a,
        Some(Ok(_)) => unreachable!("animdata path reads as animdata"),
        Some(Err(e)) => return Err(err("AnimData.d2", e)),
        None => return Err(err("AnimData.d2", format!("{path}: no native file"))),
    };
    fixed_tables_from(&tables, &anim)
}

/// The level-type table views and every DRLG file, from the archives.
pub fn load(set: &ArchiveSet) -> Result<(LevelTables, WorldFiles), WorldDataError> {
    let fixed = fixed_tables(set)?;
    let tables = LevelTables::from_fixed(&fixed)?;
    let files = WorldFiles::load(&tables.drlg, &tables.preset, &tables.outdoor, reader(set))?;
    Ok((tables, files))
}

/// The level-type table views and every DRLG file, from a native folder
/// (the typed DS1 / DT1 files).
pub fn load_native(src: &NativeSource) -> Result<(LevelTables, WorldFiles), WorldDataError> {
    let fixed = fixed_tables_native(src)?;
    let tables = LevelTables::from_fixed(&fixed)?;
    let read = |path: &[u8]| -> Result<NativeAsset, WorldDataError> {
        let name = super::file_name(path)?;
        match src.read_native(&d2_native::source::fold(&name)) {
            Some(Ok(a)) => Ok(a),
            Some(Err(detail)) => Err(WorldDataError::Read { path: name, detail }),
            None => Err(WorldDataError::Read {
                path: name,
                detail: "no native file".into(),
            }),
        }
    };
    let files = WorldFiles::load_typed(
        &tables.drlg,
        &tables.preset,
        &tables.outdoor,
        |p| match read(p)? {
            NativeAsset::Ds1(d) => Ok(d),
            _ => Err(WorldDataError::BadPath(p.to_vec())),
        },
        |p| match read(p)? {
            NativeAsset::Dt1(d) => Ok(d),
            _ => Err(WorldDataError::BadPath(p.to_vec())),
        },
    )?;
    Ok((tables, files))
}
