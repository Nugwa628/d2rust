//! A synthetic install on disk, in two stages:
//!
//! 1. **Text install** ([`write_text_install`]): `d2data.mpq` holds
//!    `string.tbl`, every excel `.txt`, the DRLG's DS1 / DT1 files
//!    ([`Synthetic::files`]) and the play client's art and palettes
//!    ([`Synthetic::stored`], [`crate::play`]); `d2exp.mpq` holds
//!    `expansionstring.tbl` and `AnimData.d2`; `patch_d2.mpq` holds
//!    `patchstring.tbl` and `soundenviron.txt`.
//! 2. **Compile and pack** ([`compile_and_pack`]): the text set is
//!    compiled through the archive set by `d2_data::compile_set`, and
//!    `patch_d2.mpq` is rewritten with the compiled `.bin` of every
//!    runtime table, `hitclass.bin` and the four code buffers, the patch
//!    archive being where the game finds its live tables. Then
//!    `d2_data::bin::load` reads the set like an install.
//!
//! The archives use several block layouts on purpose (compressed,
//! imploded, encrypted with FIX_KEY, stored), so every load also
//! exercises the reader's paths. [`Synthetic::stored`] is written
//! stored: compressing its large generated files is slow in
//! debug builds and covers no reader path the small files miss.

use std::path::{Path, PathBuf};

use d2_data::bin::{self, excel_path, read_excel, BinSet, LoadError};
use d2_data::compile_set::{compile_all, CompileSetError, CompiledSet};
use d2_data::schema::schema;
use d2_data::strings::{StringTables, StringsError};
use d2_formats::animdata;
use d2_formats::mpq::writer::{FileOptions, Method, MpqWriter, Pkware};
use d2_formats::mpq::{ArchiveSet, MpqError};

use crate::synth::{Strings, Synthetic};

/// A step of the fixture build failed.
#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("{0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error(transparent)]
    Mpq(#[from] MpqError),
    #[error(transparent)]
    Strings(#[from] StringsError),
    #[error(transparent)]
    Compile(#[from] CompileSetError),
    #[error(transparent)]
    Load(#[from] LoadError),
}

/// `data\local\lng\<lang>\<file>`.
pub fn string_path(file: &str) -> String {
    format!("data\\local\\lng\\{}\\{file}", bin::DEFAULT_LANGUAGE)
}

fn tbl(strings: &Strings) -> Vec<u8> {
    let entries: Vec<(&[u8], &[u8])> = strings
        .iter()
        .map(|(k, v)| (k.as_bytes(), v.as_bytes()))
        .collect();
    crate::tbl::write(&entries)
}

fn write(w: &MpqWriter, path: PathBuf) -> Result<(), FixtureError> {
    w.write(&path).map_err(|e| FixtureError::Io(path, e))
}

/// The patch archive's text files (stage 1), shared with stage 2.
fn patch_archive(data: &Synthetic) -> MpqWriter {
    let mut p = MpqWriter::new().with_listfile();
    p.add(
        &string_path("patchstring.tbl"),
        tbl(&data.strings.patch),
        FileOptions::default(),
    );
    p.add(
        &excel_path("soundenviron.txt"),
        data.soundenviron.clone(),
        FileOptions {
            method: Method::Implode(Pkware::default()),
            ..FileOptions::default()
        },
    );
    p
}

/// Stage 1: writes `d2data.mpq`, `d2exp.mpq` and `patch_d2.mpq` with the
/// text set into `dir` (created if absent).
pub fn write_text_install(dir: &Path, data: &Synthetic) -> Result<(), FixtureError> {
    std::fs::create_dir_all(dir).map_err(|e| FixtureError::Io(dir.to_owned(), e))?;

    let mut d = MpqWriter::new().with_listfile();
    d.add(
        &string_path("string.tbl"),
        tbl(&data.strings.base),
        FileOptions::default(),
    );
    for (i, (name, bytes)) in data.tables.render().into_iter().enumerate() {
        // Alternate layouts: PKWARE binary, PKWARE ASCII, stored.
        let options = match i % 3 {
            0 => FileOptions::default(),
            1 => FileOptions {
                method: Method::Compress(Pkware {
                    ascii: true,
                    dict_bits: 5,
                }),
                ..FileOptions::default()
            },
            _ => FileOptions::stored(),
        };
        d.add(&excel_path(&name), bytes, options);
    }
    for (name, bytes) in &data.files {
        d.add(name, bytes.clone(), FileOptions::default());
    }
    for (name, bytes) in &data.stored {
        d.add(name, bytes.clone(), FileOptions::stored());
    }
    write(&d, dir.join("d2data.mpq"))?;

    let mut x = MpqWriter::new().with_listfile();
    x.add(
        &string_path("expansionstring.tbl"),
        tbl(&data.strings.expansion),
        FileOptions::default(),
    );
    x.add(
        animdata::PATH,
        crate::animdata::write(&data.animdata),
        FileOptions::encrypted_fix_key(),
    );
    write(&x, dir.join("d2exp.mpq"))?;

    write(&patch_archive(data), dir.join("patch_d2.mpq"))
}

/// The `.bin` files of a compiled text set, `(file name, bytes)`: the
/// ones the game reads (every runtime table, `hitclass.bin`, the four
/// code buffers) and the compile-only by-products no runtime table
/// overwrites, which `d2_data::links::load_lookups` reads for linker
/// sizes.
pub fn bin_files(compiled: &CompiledSet) -> Vec<(String, Vec<u8>)> {
    let s = schema();
    let mut out = Vec::new();
    for t in &compiled.tables {
        let def = s
            .table(&t.name)
            .expect("compiled tables come from the schema");
        let overwritten = s.runtime().any(|r| r.bin_name == def.bin_name);
        if def.is_runtime() || !overwritten {
            let mut bytes = (t.compiled.count as u32).to_le_bytes().to_vec();
            bytes.extend_from_slice(&t.compiled.records);
            out.push((def.bin_name.clone(), bytes));
        }
    }
    for (buffer, code) in &compiled.buffers {
        out.push((format!("{}.bin", buffer.name()), code.clone()));
    }
    out
}

/// Stage 2: compiles the text set from the archives in `dir` and rewrites
/// `patch_d2.mpq` with the `.bin` files added.
pub fn compile_and_pack(dir: &Path, data: &Synthetic) -> Result<CompiledSet, FixtureError> {
    let set = ArchiveSet::open_dir(dir)?;
    let strings = StringTables::load(&set, bin::DEFAULT_LANGUAGE, true)?;
    let mut read = |f: &str| read_excel(&set, f).map_err(|e| e.to_string());
    let compiled = compile_all(&mut read, &strings)?;
    drop(set);

    let mut p = patch_archive(data);
    for (i, (name, bytes)) in bin_files(&compiled).into_iter().enumerate() {
        let options = if i % 2 == 0 {
            FileOptions::default()
        } else {
            FileOptions::encrypted_fix_key()
        };
        p.add(&excel_path(&name), bytes, options);
    }
    write(&p, dir.join("patch_d2.mpq"))?;
    Ok(compiled)
}

/// A built synthetic install.
#[derive(Debug)]
pub struct Install {
    pub dir: PathBuf,
    pub archives: ArchiveSet,
    pub compiled: CompiledSet,
    pub loaded: BinSet,
}

/// Builds the install in `dir` (stages 1 and 2) and loads it.
pub fn build(dir: &Path, data: &Synthetic) -> Result<Install, FixtureError> {
    write_text_install(dir, data)?;
    let compiled = compile_and_pack(dir, data)?;
    let archives = ArchiveSet::open_dir(dir)?;
    let loaded = bin::load(&archives, bin::DEFAULT_LANGUAGE)?;
    Ok(Install {
        dir: dir.to_owned(),
        archives,
        compiled,
        loaded,
    })
}
