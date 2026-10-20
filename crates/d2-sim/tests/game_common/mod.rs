//! Loads the player's extracted tables for the `#[ignore]` game-file tests,
//! as the in-crate game tests do (`skills::tests::game_loader`): the
//! `.bin` files of `D2_GAME_DIR/extracted/patch_d2/data/global/excel/`
//! (`mpq-tool extract`). Tests may read game files (`CLAUDE.md`
//! conventions); the sim never does.
#![allow(dead_code)]

use d2_data::bin::BinTable;
use d2_data::tables::{decode_all, Record};

/// Reads one extracted excel file.
#[allow(clippy::disallowed_methods)]
pub fn read(file: &str) -> Vec<u8> {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let path = format!("{dir}/extracted/patch_d2/data/global/excel/{file}");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// `<name>.bin` as a table of `size`-byte records.
pub fn table(name: &str, size: usize) -> BinTable {
    let file = format!("{name}.bin");
    BinTable::parse(name, "patch_d2.mpq", &file, &read(&file), size).expect("table parses")
}

/// The raw table of a typed record.
pub fn raw<T: Record>() -> BinTable {
    table(T::TABLE, T::SIZE)
}

/// Every record of a typed table.
pub fn rows<T: Record>() -> Vec<T> {
    decode_all(&raw::<T>()).expect("typed table decodes")
}

/// A spec TSV as rows of cells (header row dropped).
pub fn tsv_rows(tsv: &str) -> Vec<Vec<&str>> {
    tsv.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('\t').collect())
        .collect()
}
