//! Compiles every table of the load sequence from the player's `.txt` sheets, in
//! `tables.tsv` execution order, with linkers kept across tables and the hand-built
//! `@range`, `@treasureclass`, `@uniques` and `@sets` linkers built at the point in the
//! sequence where later tables need them.
//!
//! [`compile_all`] takes a reader for `.txt` files (usually backed by the archives) and
//! returns a [`CompiledSet`]: one compiled record block per table, the four calc code
//! buffers, the final linkers and the calc diagnostics. The cross-check and mod-patch
//! checks (`crosscheck`, `patch::check`) consume it.

use std::collections::BTreeMap;

use crate::bin::DataRules;
use crate::calc::{CalcDiag, CalcRegistry, NO_KEYWORDS};
use crate::compile::{
    check_field_list, compile_table_with, range_linker, special_linker, tc_linker, Compiled,
    Linker, Linkers, StdCallbacks, RANGE_LINKER, SETS_LINKER, TC_LINKER, UNIQUES_LINKER,
};
use crate::schema::{schema, CalcBuffer};
use crate::strings::StringTables;
use crate::txt::{ErrorCode, TxtError, TxtTable};

/// One compiled table.
#[derive(Debug, Clone)]
pub struct CompiledTable {
    /// `tables.tsv` name.
    pub name: String,
    /// Archive the `.txt` came from.
    pub txt_source: String,
    /// The compiled records and link data.
    pub compiled: Compiled,
}

/// The whole text compile.
#[derive(Debug, Clone)]
pub struct CompiledSet {
    /// Every called table, in execution order.
    pub tables: Vec<CompiledTable>,
    /// The four code buffers.
    pub buffers: BTreeMap<CalcBuffer, Vec<u8>>,
    /// Calls of unknown table-specific callbacks by callback;
    /// empty for the standard field lists.
    pub unspecified_callbacks: BTreeMap<String, usize>,
    /// Count of each calc diagnostic, by buffer.
    pub calc_diagnostics: BTreeMap<(CalcBuffer, CalcDiag), usize>,
    /// Linkers after the last table.
    pub linkers: Linkers,
}

impl CompiledSet {
    /// The compiled table called `name`, if it was compiled.
    pub fn table(&self, name: &str) -> Option<&CompiledTable> {
        self.tables.iter().find(|t| t.name == name)
    }
}

/// A source of `.txt` files: `(archive, bytes)` for an excel file name,
/// `None` when absent.
pub type TxtReader<'a> = dyn FnMut(&str) -> Result<Option<(String, Vec<u8>)>, String> + 'a;

/// A text compile failure.
#[derive(Debug, thiserror::Error)]
pub enum CompileSetError {
    /// The `.txt` is in none of the archives.
    #[error("{file}: not found in any archive")]
    Missing { file: String },
    /// The reader failed while fetching the file.
    #[error("{file}: {detail}")]
    Read { file: String, detail: String },
    /// The sheet or one of its cells failed to parse or compile.
    #[error(transparent)]
    Txt(#[from] TxtError),
}

/// Compiles every called table (`tables.tsv` order) from its `.txt`.
pub fn compile_all(
    read: &mut TxtReader<'_>,
    strings: &StringTables,
) -> Result<CompiledSet, CompileSetError> {
    compile_all_with(read, strings, &NO_KEYWORDS)
}

/// [`compile_all`] for a mod game: formula cells may use the keywords in `calc`
/// (`docs/plans/mods/seams.md` MS9). With an empty registry the result is the same as
/// [`compile_all`]'s.
pub fn compile_all_with(
    read: &mut TxtReader<'_>,
    strings: &StringTables,
    calc: &CalcRegistry,
) -> Result<CompiledSet, CompileSetError> {
    compile_all_under(read, strings, calc, DataRules::Original)
}

/// [`compile_all_with`] under the data `rules` of the game (MS10d): under
/// [`DataRules::Mod`] out-of-range integer cells are clamped
/// ([`crate::compile::compile_table_with`]).
pub fn compile_all_under(
    read: &mut TxtReader<'_>,
    strings: &StringTables,
    calc: &CalcRegistry,
    rules: DataRules,
) -> Result<CompiledSet, CompileSetError> {
    let mut linkers = Linkers::default();
    let mut callbacks = StdCallbacks::with_calc(strings, calc);
    let mut tables: Vec<CompiledTable> = Vec::new();
    for def in schema().called() {
        if def.name == "skills" {
            // The `@range` linker must exist before the skills table compiles.
            linkers.insert(RANGE_LINKER, range_linker());
        }
        // The field list is checked before the file is read.
        check_field_list(&def.txt_name, &def.fields, def.record_size)?;
        let (source, bytes) = read(&def.txt_name)
            .map_err(|detail| CompileSetError::Read {
                file: def.txt_name.clone(),
                detail,
            })?
            .ok_or_else(|| CompileSetError::Missing {
                file: def.txt_name.clone(),
            })?;
        let file = format!("{source}:{}", def.txt_name);
        let txt = TxtTable::parse(&file, &bytes)?;
        let compiled = compile_table_with(
            &file,
            &txt,
            &def.fields,
            def.record_size,
            &mut linkers,
            &mut callbacks,
            rules,
        )?;
        tables.push(CompiledTable {
            name: def.name.clone(),
            txt_source: source,
            compiled,
        });
        let special = match def.name.as_str() {
            "uniqueitems" => Some((UNIQUES_LINKER, 52)),
            "setitems" => Some((SETS_LINKER, 48)),
            _ => None,
        };
        if let Some((name, lvl_offset)) = special {
            // Register the item names so later tables can link to them.
            let c = &tables.last().expect("just pushed").compiled;
            let (linker, items) =
                special_linker(c.records.chunks_exact(c.record_size), 40, lvl_offset).ok_or_else(
                    || TxtError::new(&file, ErrorCode::E11).with_detail("unique or set name"),
                )?;
            linkers.insert(name, Linker::Name(linker));
            if name == UNIQUES_LINKER {
                callbacks.special.uniques = items;
            } else {
                callbacks.special.sets = items;
            }
        }
        if def.name == "treasureclassex" {
            // The treasure-class linker is built from `itemtypes` and `treasureclassex`.
            let records = |n: &str| {
                let c = &tables
                    .iter()
                    .find(|t| t.name == n)
                    .expect("compiled earlier")
                    .compiled;
                c.records.chunks_exact(c.record_size).collect::<Vec<_>>()
            };
            let tc =
                tc_linker(records("itemtypes"), records("treasureclassex")).ok_or_else(|| {
                    TxtError::new(&file, ErrorCode::E11).with_detail("treasure class name")
                })?;
            linkers.insert(TC_LINKER, Linker::Name(tc));
        }
    }
    Ok(CompiledSet {
        tables,
        buffers: callbacks.buffers,
        unspecified_callbacks: callbacks.unspecified,
        calc_diagnostics: callbacks.calc_diagnostics,
        linkers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compile reads each table's `.txt` (`levels.txt` for
    /// `leveldefs`); a missing file stops it.
    #[test]
    fn missing_txt_is_fatal() {
        assert_eq!(schema().table("leveldefs").unwrap().txt_name, "levels.txt");
        let mut asked = Vec::new();
        let mut read = |f: &str| {
            asked.push(f.to_owned());
            Ok(None)
        };
        match compile_all(&mut read, &StringTables::default()) {
            Err(CompileSetError::Missing { file }) => assert_eq!(file, "compcode.txt"),
            other => panic!("{other:?}"),
        }
        assert_eq!(asked, ["compcode.txt"]);
    }
}
