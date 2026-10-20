//! The game's files for the play app, from the user's archives or from a
//! converted native folder: one value that answers both the table reads
//! (`TableFiles`) and the asset reads (`FileSource`, typed through
//! `read_native`), so `play` runs the same code on either source.
//!
//! On a native folder the excel `.bin` / `.txt` reads come from the
//! compiled [`NativeTables`] (built once at open); the typed assets come
//! from the native files; `.wav` reads are `None` (audio is deferred).

use std::path::Path;
use std::sync::Arc;

use d2_data::bin::{LoadError, TableFiles};
use d2_data::strings::StringsError;
use d2_formats::mpq::ArchiveSet;
use d2_formats::tbl::StringTable;
use d2_native::source::{fold, NativeSource, NativeTables};

use crate::file_source::{CanonicalPath, FileSource};

const EXCEL_DIR: &str = "data/global/excel/";

/// The native folder as a [`FileSource`]: [`NativeSource`] plus the table
/// set compiled once, which serves the excel files (the `.bin` set has no
/// native file of its own).
struct NativeFiles {
    src: Arc<NativeSource>,
    tables: NativeTables,
}

impl FileSource for NativeFiles {
    fn read_file(&self, archive_name: &str) -> Option<Result<Vec<u8>, String>> {
        let p = fold(archive_name);
        match p.strip_prefix(EXCEL_DIR) {
            Some(file) if !file.contains('/') => match self.tables.read_excel(file) {
                Ok(hit) => hit.map(|(_, bytes)| Ok(bytes)),
                Err(e) => Some(Err(e.to_string())),
            },
            _ => FileSource::read_file(&*self.src, archive_name),
        }
    }

    fn read_native(
        &self,
        path: &CanonicalPath,
    ) -> Option<Result<d2_native::source::NativeAsset, String>> {
        FileSource::read_native(&*self.src, path)
    }

    fn is_native(&self) -> bool {
        true
    }
}

/// The user's game files (archives or native), see the module docs.
#[derive(Clone)]
pub struct GameFiles {
    files: Arc<dyn FileSource>,
    tables: Arc<dyn TableFiles + Send + Sync>,
    native: bool,
}

impl std::fmt::Debug for GameFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameFiles")
            .field("native", &self.native)
            .finish_non_exhaustive()
    }
}

impl GameFiles {
    /// Combine an asset reader and its table reader for a headless game or tool.
    pub fn from_sources(
        files: Arc<dyn FileSource>,
        tables: Arc<dyn TableFiles + Send + Sync>,
    ) -> Self {
        let native = files.is_native();
        Self {
            files,
            tables,
            native,
        }
    }

    /// The archives of `D2_GAME_DIR`.
    pub fn archives(set: Arc<ArchiveSet>) -> Self {
        GameFiles {
            files: set.clone(),
            tables: set,
            native: false,
        }
    }

    /// A converted native folder (`SourceError` text names the cause and
    /// says to re-run the converter).
    pub fn native(dir: &Path) -> Result<Self, String> {
        Self::native_with(dir, None)
    }

    /// [`GameFiles::native`] with the caller's layer list
    /// (`NativeSource::open_with`): `Some(mods)` stacks `base/` and those
    /// mods, `None` reads `mods/order.toml`.
    pub fn native_with(dir: &Path, order: Option<&[String]>) -> Result<Self, String> {
        let src = Arc::new(NativeSource::open_with(dir, order).map_err(|e| e.to_string())?);
        Self::from_native(src)
    }

    /// An opened native source: compiles the table set once.
    pub fn from_native(src: Arc<NativeSource>) -> Result<Self, String> {
        let tables = src
            .tables(d2_data::bin::DEFAULT_LANGUAGE)
            .map_err(|e| e.to_string())?;
        let shared = Arc::new(tables.clone());
        Ok(GameFiles {
            files: Arc::new(NativeFiles { src, tables }),
            tables: shared,
            native: true,
        })
    }

    /// `true` for a native folder.
    pub fn is_native(&self) -> bool {
        self.native
    }

    /// The asset reads, shared (`Arc<dyn FileSource>` for the loaders).
    pub fn source(&self) -> Arc<dyn FileSource> {
        self.files.clone()
    }
}

impl FileSource for GameFiles {
    fn read_file(&self, archive_name: &str) -> Option<Result<Vec<u8>, String>> {
        self.files.read_file(archive_name)
    }

    fn read_native(
        &self,
        path: &CanonicalPath,
    ) -> Option<Result<d2_native::source::NativeAsset, String>> {
        self.files.read_native(path)
    }

    fn is_native(&self) -> bool {
        self.native
    }
}

impl TableFiles for GameFiles {
    fn read_excel(&self, file: &str) -> Result<Option<(String, Vec<u8>)>, LoadError> {
        self.tables.read_excel(file)
    }

    fn lod(&self) -> bool {
        self.tables.lod()
    }

    fn string_table(&self, path: &str) -> Result<StringTable, StringsError> {
        self.tables.string_table(path)
    }
}
