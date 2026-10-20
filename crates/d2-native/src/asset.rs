//! The converter's view of the native formats: one enum, [`AssetKind`],
//! over every kind of source file the converter turns into native files,
//! with the same two steps for each:
//!
//! - [`AssetKind::write`] decodes the source bytes and writes the native
//!   files (plus notes for the conversion report);
//! - [`AssetKind::check`] reads those files back from disk, decodes the
//!   source again and compares the two values.
//!
//! The pictures, palettes and the expansion field go through the
//! [`NativeKind`] trait; the text kinds (COF, DS1, fonts, string tables,
//! the animation table) have plain `write_*` / `read_*` functions. This
//! module only dispatches between them: the format rules live in the kind
//! modules.
//!
//! The runtime source has its own, wider kind list
//! ([`crate::source::AssetKind`]: it also serves sounds, excel tables and
//! colour maps, which are copied rather than converted).

use std::fmt::Display;

use d2_formats::animdata::AnimData;
use d2_formats::cof::Cof;
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::font::FontTable;
use d2_formats::palette::{Palette, Pl2};
use d2_formats::tbl::StringTable;
use d2_formats::FormatError;

use crate::expfield::ExpFieldData;
use crate::kind::{self, grey_palette, CheckError, NativeFile, NativeKind, ViewPalette};
use crate::toml_kinds::{self, TextError};
use crate::{animdata, tbl};

/// Canonical path of the animation table.
pub const ANIMDATA_PATH: &str = "data/global/animdata.d2";
/// Canonical path of the expansion field.
pub const EXPFIELD_PATH: &str = "data/global/expfield.d2";

/// A failed step: `check` is the check name written to `files.tsv`
/// (`decode`, `write`, `C-DC6`, `C-STRUCT`, ...), `detail` the decoder's
/// message or the first difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetError {
    /// Check name as written to `files.tsv`.
    pub check: String,
    /// The first difference, or the decoder's message.
    pub detail: String,
}

impl AssetError {
    fn new(check: &str, detail: impl Into<String>) -> AssetError {
        AssetError {
            check: check.to_owned(),
            detail: detail.into(),
        }
    }

    /// The source bytes did not decode.
    fn decode(canon: &str, e: FormatError) -> AssetError {
        AssetError::new("decode", format!("{canon}: {e}"))
    }

    /// The native writer refused the decoded value.
    fn write(e: impl Display) -> AssetError {
        AssetError::new("write", e.to_string())
    }
}

/// What [`AssetKind::write`] produces.
#[derive(Debug, Clone, Default)]
pub struct AssetWritten {
    /// The native files, sorted by path.
    pub files: Vec<NativeFile>,
    /// Lines for the converter's report.
    pub notes: Vec<String>,
}

/// A source file kind the converter turns into native files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssetKind {
    /// `.dc6` sprite sheets.
    Dc6,
    /// `.dcc` directional character animations.
    Dcc,
    /// `.dt1` floor, wall and object tile sets.
    Dt1,
    /// The `pal.dat` palettes.
    Pal,
    /// `.pl2` palette transform files.
    Pl2,
    /// `.cof` animation composition files.
    Cof,
    /// `.ds1` map pieces.
    Ds1,
    /// String tables under `data/local/lng/`.
    Tbl,
    /// Font tables under `data/local/font/`.
    Font,
    /// `animdata.d2`, the animation speed and event table.
    Animdata,
    /// `expfield.d2`, the expansion field grid.
    ExpField,
}

impl AssetKind {
    /// Every kind.
    pub const ALL: [AssetKind; 11] = [
        AssetKind::Dc6,
        AssetKind::Dcc,
        AssetKind::Dt1,
        AssetKind::Pal,
        AssetKind::Pl2,
        AssetKind::Cof,
        AssetKind::Ds1,
        AssetKind::Tbl,
        AssetKind::Font,
        AssetKind::Animdata,
        AssetKind::ExpField,
    ];

    /// The kind name used in `manifest.toml` and `files.tsv`.
    pub fn name(self) -> &'static str {
        match self {
            AssetKind::Dc6 => "dc6",
            AssetKind::Dcc => "dcc",
            AssetKind::Dt1 => "dt1",
            AssetKind::Pal => "pal",
            AssetKind::Pl2 => "pl2",
            AssetKind::Cof => "cof",
            AssetKind::Ds1 => "ds1",
            AssetKind::Tbl => "tbl",
            AssetKind::Font => "font",
            AssetKind::Animdata => "animdata",
            AssetKind::ExpField => "expfield",
        }
    }

    /// Layout version of this kind's native files (1 for every kind).
    pub fn native_version(self) -> u32 {
        1
    }

    /// Conversion phase: string tables convert first (0), everything else
    /// after them (1).
    pub fn phase(self) -> u8 {
        if self == AssetKind::Tbl {
            0
        } else {
            1
        }
    }

    /// The kind that converts the file at canonical path `canon`, if any.
    pub fn for_path(canon: &str) -> Option<AssetKind> {
        AssetKind::ALL.into_iter().find(|k| k.claims(canon))
    }

    /// Whether canonical path `canon` belongs to this kind. String and font
    /// tables are both `.tbl`; they are told apart by their folder (one
    /// language or font folder deep, no further).
    pub fn claims(self, canon: &str) -> bool {
        let one_folder_under = |dir: &str, ext: &str| {
            canon.strip_prefix(dir).is_some_and(|rest| {
                rest.split_once('/')
                    .is_some_and(|(_, f)| !f.contains('/') && f.ends_with(ext))
            })
        };
        match self {
            AssetKind::Dc6 => canon.ends_with(".dc6"),
            AssetKind::Dcc => canon.ends_with(".dcc"),
            AssetKind::Dt1 => canon.ends_with(".dt1"),
            AssetKind::Pal => canon.ends_with("/pal.dat"),
            AssetKind::Pl2 => canon.ends_with(".pl2"),
            AssetKind::Cof => canon.ends_with(".cof"),
            AssetKind::Ds1 => canon.ends_with(".ds1"),
            AssetKind::Tbl => one_folder_under("data/local/lng/", ".tbl"),
            AssetKind::Font => one_folder_under("data/local/font/", ".tbl"),
            AssetKind::Animdata => canon == ANIMDATA_PATH,
            AssetKind::ExpField => canon == EXPFIELD_PATH,
        }
    }

    /// Whether [`AssetKind::references`] can name other files.
    pub fn has_references(self) -> bool {
        self == AssetKind::Ds1
    }

    /// The other files a source file names: a map piece's tile set list.
    /// The embedded names are often the level designer's own paths or name
    /// `.tg1` files; each is cut to the part from `data\global` on and
    /// given the `.dt1` extension, so it names the archive file that is
    /// actually loaded. Names that still are not tile sets are dropped.
    pub fn references(self, _canon: &str, src: &[u8]) -> Vec<String> {
        if self != AssetKind::Ds1 {
            return Vec::new();
        }
        let Ok(ds1) = Ds1::parse(src) else {
            return Vec::new();
        };
        ds1.files
            .iter()
            .filter_map(|name| tile_set_reference(name))
            .collect()
    }

    /// Decodes `src` (canonical path `canon`) and writes its native files,
    /// sorted by path.
    pub fn write(self, canon: &str, src: &[u8]) -> Result<AssetWritten, AssetError> {
        let dec = |e| AssetError::decode(canon, e);
        let view = grey_palette();
        let mut notes = Vec::new();
        let mut files = match self {
            AssetKind::Dc6 => write_kind(&Dc6::parse(src).map_err(dec)?, canon, &view)?,
            AssetKind::Dcc => write_kind(&Dcc::parse(src).map_err(dec)?, canon, &view)?,
            AssetKind::Dt1 => {
                let files = write_kind(&Dt1::parse(src).map_err(dec)?, canon, &view)?;
                let blocks = blocks_layout_tiles(canon, &files);
                if blocks > 0 {
                    notes.push(format!("{blocks} tile(s) use the blocks layout"));
                }
                files
            }
            AssetKind::Pal => write_kind(&Palette::parse(src).map_err(dec)?, canon, &view)?,
            AssetKind::Pl2 => write_kind(&Pl2::parse(src).map_err(dec)?, canon, &view)?,
            AssetKind::ExpField => write_kind(&decode_expfield(src)?, canon, &view)?,
            AssetKind::Cof => text_file(
                canon,
                ".toml",
                toml_kinds::write_cof(canon, &Cof::parse(src).map_err(dec)?),
            )?,
            AssetKind::Ds1 => text_file(
                canon,
                ".toml",
                toml_kinds::write_ds1(canon, &Ds1::parse(src).map_err(dec)?),
            )?,
            AssetKind::Font => text_file(
                canon,
                ".toml",
                toml_kinds::write_font(canon, &FontTable::parse(src).map_err(dec)?),
            )?,
            AssetKind::Animdata => text_file(
                canon,
                ".tsv",
                animdata::write_animdata(canon, &AnimData::parse(src).map_err(dec)?),
            )?,
            AssetKind::Tbl => {
                let table = StringTable::parse(src).map_err(dec)?;
                if tbl::rebuild_differs(&table) {
                    notes.push("hash table differs from a rebuild (§2.6 r3)".into());
                }
                let f = tbl::write_tbl(canon, &table, true).map_err(AssetError::write)?;
                vec![
                    NativeFile {
                        path: format!("{canon}.toml"),
                        bytes: f.toml.into_bytes(),
                    },
                    NativeFile {
                        path: format!("{canon}.tsv"),
                        bytes: f.tsv.into_bytes(),
                    },
                ]
            }
        };
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(AssetWritten { files, notes })
    }

    /// The round-trip check: `native` holds the files as read back from
    /// disk. Decodes `src` again, reads the native files with the native
    /// reader and compares the two values.
    pub fn check(self, canon: &str, src: &[u8], native: &[NativeFile]) -> Result<(), AssetError> {
        let back = ReadBack {
            canon,
            native,
            check: self.check_name(),
        };
        let dec = |e| AssetError::decode(canon, e);
        match self {
            AssetKind::Dc6 => back.kind(&Dc6::parse(src).map_err(dec)?),
            AssetKind::Dcc => back.kind(&Dcc::parse(src).map_err(dec)?),
            AssetKind::Dt1 => back.kind(&Dt1::parse(src).map_err(dec)?),
            AssetKind::Pal => back.kind(&Palette::parse(src).map_err(dec)?),
            AssetKind::Pl2 => back.kind(&Pl2::parse(src).map_err(dec)?),
            AssetKind::ExpField => back.kind(&decode_expfield(src)?),
            AssetKind::Cof => {
                let original = Cof::parse(src).map_err(dec)?;
                let read = toml_kinds::read_cof(canon, &back.text(".toml")?);
                back.same(&original, &read.map_err(|e| back.refused(e))?)
            }
            AssetKind::Ds1 => {
                let original = Ds1::parse(src).map_err(dec)?;
                let read = toml_kinds::read_ds1(canon, &back.text(".toml")?);
                back.same(&original, &read.map_err(|e| back.refused(e))?)
            }
            AssetKind::Font => {
                let original = FontTable::parse(src).map_err(dec)?;
                let read = toml_kinds::read_font(canon, &back.text(".toml")?);
                back.same(&original, &read.map_err(|e| back.refused(e))?)
            }
            AssetKind::Animdata => {
                let original = AnimData::parse(src).map_err(dec)?;
                let read = animdata::read_animdata(canon, &back.text(".tsv")?);
                back.same(&original, &read.map_err(|e| back.refused(e))?)
            }
            AssetKind::Tbl => {
                let original = StringTable::parse(src).map_err(dec)?;
                let read = tbl::read_tbl(canon, &back.text(".tsv")?, &back.text(".toml")?);
                back.string_table(&original, &read.map_err(|e| back.refused(e))?)
            }
        }
    }

    /// The `files.tsv` check name of the round trip: the image and string
    /// table kinds have their own (`C-DC6`, `C-DCC`, `C-DT1`, `C-TBL`),
    /// every other kind is a structure comparison, `C-STRUCT`.
    fn check_name(self) -> String {
        match self {
            AssetKind::Dc6 | AssetKind::Dcc | AssetKind::Dt1 | AssetKind::Tbl => {
                format!("C-{}", self.name().to_ascii_uppercase())
            }
            _ => "C-STRUCT".into(),
        }
    }
}

/// One tile set name from a map piece, as the canonical (backslashed) path
/// of the `.dt1` that is loaded, or `None` when it names no tile set.
fn tile_set_reference(name: &[u8]) -> Option<String> {
    let mut s = String::from_utf8_lossy(name)
        .to_ascii_lowercase()
        .replace('\\', "/");
    if let Some(i) = s.find("data/global/") {
        s = s[i..].to_owned();
    } else if s.starts_with("tiles/") {
        s = format!("data/global/{s}");
    }
    if let Some(stem) = s.strip_suffix(".tg1") {
        s = format!("{stem}.dt1");
    }
    s.ends_with(".dt1").then(|| s.replace('/', "\\"))
}

/// How many tiles of a written tile set use the blocks layout, counted in
/// its sidecar `P.toml`.
fn blocks_layout_tiles(canon: &str, files: &[NativeFile]) -> usize {
    let sidecar = format!("{canon}.toml");
    files.iter().find(|f| f.path == sidecar).map_or(0, |f| {
        String::from_utf8_lossy(&f.bytes)
            .matches("layout = \"blocks\"")
            .count()
    })
}

fn decode_expfield(src: &[u8]) -> Result<ExpFieldData, AssetError> {
    ExpFieldData::from_bytes(src).map_err(|e| AssetError::new("decode", e.to_string()))
}

/// The native files of a [`NativeKind`] value.
fn write_kind<K: NativeKind>(
    value: &K,
    canon: &str,
    view: &ViewPalette,
) -> Result<Vec<NativeFile>, AssetError> {
    value.write(canon, view).map_err(AssetError::write)
}

/// The single text file `canon` + `ext` of a text kind's writer result.
fn text_file(
    canon: &str,
    ext: &str,
    written: Result<String, TextError>,
) -> Result<Vec<NativeFile>, AssetError> {
    let text = written.map_err(AssetError::write)?;
    Ok(vec![NativeFile {
        path: format!("{canon}{ext}"),
        bytes: text.into_bytes(),
    }])
}

/// The native files of one source file as read back from disk, and the
/// check name every failure is reported under.
struct ReadBack<'a> {
    canon: &'a str,
    native: &'a [NativeFile],
    check: String,
}

impl ReadBack<'_> {
    fn fail(&self, detail: impl Into<String>) -> AssetError {
        AssetError::new(&self.check, detail)
    }

    /// A native reader refused the files.
    fn refused(&self, e: TextError) -> AssetError {
        self.fail(e.to_string())
    }

    /// The text of native file `canon` + `ext`.
    fn text(&self, ext: &str) -> Result<String, AssetError> {
        let path = format!("{}{ext}", self.canon);
        let file = self
            .native
            .iter()
            .find(|f| f.path == path)
            .ok_or_else(|| self.fail(format!("{path}: missing native file")))?;
        String::from_utf8(file.bytes.clone()).map_err(|_| self.fail(format!("{path}: not UTF-8")))
    }

    /// The round trip of a [`NativeKind`] value through the read-back files.
    /// A mismatch reports just the difference; a failed read says so.
    fn kind<K: NativeKind>(&self, original: &K) -> Result<(), AssetError> {
        kind::check_files(original, self.canon, self.native).map_err(|e| match e {
            CheckError::Mismatch(d) => self.fail(d.to_string()),
            other => self.fail(other.to_string()),
        })
    }

    /// Equal values, or the first difference.
    fn same<T: PartialEq + std::fmt::Debug>(&self, a: &T, b: &T) -> Result<(), AssetError> {
        if a == b {
            Ok(())
        } else {
            Err(self.fail(format!(
                "{}: {}",
                self.canon,
                toml_kinds::first_difference(a, b)
            )))
        }
    }

    /// A string table round trip. The header's two layout fields (where the
    /// text starts, the file size) follow from the content and are left out
    /// of the comparison; on top of equal contents, every key must be found
    /// in the same hash slot in both tables, so lookups behave the same.
    fn string_table(&self, original: &StringTable, back: &StringTable) -> Result<(), AssetError> {
        let without_layout = |t: &StringTable| {
            let mut t = t.clone();
            t.header.data_start = 0;
            t.header.file_size = 0;
            t
        };
        self.same(&without_layout(original), &without_layout(back))?;
        for i in 0..original.indices.len() {
            let Some(e) = original.element(i) else {
                continue;
            };
            let (x, y) = (original.find_slot(&e.key), back.find_slot(&e.key));
            if x != y {
                return Err(self.fail(format!(
                    "{}: lookup of element {i}: slot {x:?} vs {y:?}",
                    self.canon
                )));
            }
        }
        Ok(())
    }
}
