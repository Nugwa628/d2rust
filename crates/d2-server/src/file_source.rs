//! Canonical asset paths and the plain-Rust reads behind the `mpq://` source.
//!
//! Archive names are case-insensitive and accept either slash; Bevy asset paths are
//! not. Every path is therefore canonicalized (ASCII lowercase, `/` separators, no
//! leading `/`) before it reaches the `AssetServer`, so two spellings of one file give
//! one handle. The reader accepts canonical paths only: folding there would quietly
//! load a second handle for a file that is already loaded.
//!
//! A [`FileSource`] is where the bytes come from: the player's archives, a converted
//! native folder, or an in-memory fake for tests. The `read_*` functions give the decoded
//! file the same way from any of them.

use std::fmt;

use d2_formats::mpq::ArchiveSet;
use d2_native::source::{decode_original, AssetKind, NativeAsset, NativeSource};

/// An archive path in canonical form, without the `mpq://` prefix:
/// `data/global/palette/act1/pal.dat`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalPath(String);

/// Why a path cannot be canonical. Every variant names the path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("asset path is empty")]
    Empty,
    #[error("asset path {0:?} contains a non-ASCII character")]
    NonAscii(String),
    #[error("asset path {0:?} starts with a separator")]
    Leading(String),
    #[error("asset path {0:?} has an empty, `.` or `..` component")]
    BadComponent(String),
    /// `#` starts an asset label in Bevy paths; `:` would read as a source.
    #[error("asset path {0:?} contains a reserved character (`#` or `:`)")]
    Reserved(String),
    #[error("asset path {0:?} is not canonical (expected {1:?})")]
    NotCanonical(String, String),
}

/// Folds an archive or asset path to canonical spelling: `\` → `/`, `A`–`Z` → `a`–`z`.
/// Every other character is kept, as archive lookups fold no others. Never fails;
/// [`CanonicalPath::new`] adds the validity checks.
pub fn fold(path: &str) -> String {
    path.chars()
        .map(|c| match c {
            '\\' => '/',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

impl CanonicalPath {
    /// Canonicalizes an archive path (`\` or `/` separators, any case).
    pub fn new(path: &str) -> Result<CanonicalPath, PathError> {
        let folded = fold(path);
        check(&folded)?;
        Ok(CanonicalPath(folded))
    }

    /// Accepts `path` only if it is already canonical (the reader's check).
    pub fn parse_canonical(path: &str) -> Result<CanonicalPath, PathError> {
        let c = CanonicalPath::new(path)?;
        if c.0 != path {
            return Err(PathError::NotCanonical(path.to_string(), c.0));
        }
        Ok(c)
    }

    /// The canonical path, `/` separators.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name to look up in the archive set (`\` separators).
    pub fn archive_name(&self) -> String {
        self.0.replace('/', "\\")
    }

    /// Bevy asset path with the source prefix: `mpq://data/...`.
    pub fn asset_path(&self) -> String {
        format!("mpq://{}", self.0)
    }
}

impl fmt::Display for CanonicalPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The validity rules of a folded path (see [`PathError`]).
fn check(folded: &str) -> Result<(), PathError> {
    let owned = || folded.to_string();
    if folded.is_empty() {
        return Err(PathError::Empty);
    }
    if !folded.is_ascii() {
        return Err(PathError::NonAscii(owned()));
    }
    if folded.starts_with('/') {
        return Err(PathError::Leading(owned()));
    }
    if folded.contains(['#', ':']) {
        return Err(PathError::Reserved(owned()));
    }
    if folded
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(PathError::BadComponent(owned()));
    }
    Ok(())
}

/// Where asset bytes come from: the user's archive set, or a test fake.
pub trait FileSource: Send + Sync + 'static {
    /// Reads `archive_name` (`\` separators). `None` if no archive holds
    /// it; `Some(Err(message))` if one does but reading failed.
    fn read_file(&self, archive_name: &str) -> Option<Result<Vec<u8>, String>>;

    /// The decoded asset at `path`. A native folder stores it decoded already; on the
    /// archives the bytes are decoded here with the `d2-formats` readers, so every caller
    /// has one code path. `None` when the source has no such file, or for a kind that is
    /// not converted.
    fn read_native(&self, path: &CanonicalPath) -> Option<Result<NativeAsset, String>> {
        match self.read_file(&path.archive_name())? {
            Ok(bytes) => decode_original(path.as_str(), &bytes),
            Err(e) => Some(Err(e)),
        }
    }

    /// `true` for a native folder: the loaders then take
    /// [`FileSource::read_native`], and the bytes are only a presence mark.
    fn is_native(&self) -> bool {
        false
    }
}

/// The decoded asset at `archive` (`\` or `/` separators, any case) from `source`,
/// narrowed to one type by `pick`: on a native folder the typed file, on the archives
/// the decoded bytes. `None`: no such file, or a file of another kind.
fn read_typed<S: FileSource + ?Sized, T>(
    source: &S,
    archive: &str,
    pick: fn(NativeAsset) -> Option<T>,
) -> Option<Result<T, String>> {
    let path = match CanonicalPath::new(archive) {
        Ok(p) => p,
        Err(e) => return Some(Err(e.to_string())),
    };
    match source.read_native(&path)? {
        Ok(a) => pick(a).map(Ok),
        Err(e) => Some(Err(e)),
    }
}

/// Declares `pub fn $name(source, archive)`: [`read_typed`] for one `NativeAsset` variant.
macro_rules! typed_reader {
    ($(#[$m:meta])* $name:ident, $variant:ident, $ty:ty) => {
        $(#[$m])*
        pub fn $name<S: FileSource + ?Sized>(
            source: &S,
            archive: &str,
        ) -> Option<Result<$ty, String>> {
            read_typed(source, archive, |a| match a {
                NativeAsset::$variant(x) => Some(x),
                _ => None,
            })
        }
    };
}

typed_reader!(
    /// The DC6 at `archive`, from archives or native alike.
    read_dc6, Dc6, d2_formats::dc6::Dc6
);
typed_reader!(
    /// The DCC at `archive`.
    read_dcc, Dcc, d2_formats::dcc::Dcc
);
typed_reader!(
    /// The DT1 at `archive`.
    read_dt1_file, Dt1, d2_formats::dt1::Dt1
);
typed_reader!(
    /// The COF at `archive`.
    read_cof, Cof, d2_formats::cof::Cof
);
typed_reader!(
    /// The DS1 at `archive`.
    read_ds1, Ds1, d2_formats::ds1::Ds1
);
typed_reader!(
    /// The font `.tbl` at `archive`.
    read_font_table, Font, d2_formats::font::FontTable
);
typed_reader!(
    /// The palette (`pal.dat`) at `archive`.
    read_palette, Pal, d2_formats::palette::Palette
);
typed_reader!(
    /// The PL2 at `archive`.
    read_pl2, Pl2, d2_formats::palette::Pl2
);

/// The converted native folder. Raw bytes exist only for the kinds whose native file is
/// kept verbatim: excel `.txt` tables and the item colour maps. Audio is not converted
/// yet: `.wav` reads are `None`, so the sound code skips them (no sound on a native
/// folder for now).
impl FileSource for NativeSource {
    fn read_file(&self, archive_name: &str) -> Option<Result<Vec<u8>, String>> {
        let p = fold(archive_name);
        match AssetKind::of(&p)? {
            AssetKind::Excel => match self.read_native(&p)? {
                Ok(NativeAsset::Excel(b)) => Some(Ok(b)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            },
            AssetKind::ColorMap => match self.read_native(&p)? {
                Ok(NativeAsset::ColorMap(b)) => Some(Ok(b)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            },
            AssetKind::Wav => None,
            // A typed kind: present (empty bytes) or absent; the loaders
            // read it through `read_native`.
            _ => self.contains(&p).then(|| Ok(Vec::new())),
        }
    }

    fn read_native(&self, path: &CanonicalPath) -> Option<Result<NativeAsset, String>> {
        if AssetKind::of(path.as_str()) == Some(AssetKind::Wav) {
            return None;
        }
        NativeSource::read_native(self, path.as_str())
    }

    fn is_native(&self) -> bool {
        true
    }
}

impl FileSource for ArchiveSet {
    fn read_file(&self, archive_name: &str) -> Option<Result<Vec<u8>, String>> {
        self.find(archive_name)
            .map(|archive| archive.read(archive_name).map_err(|e| e.to_string()))
    }
}

/// A failed asset read. Every variant names the path; there is no fallback asset.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("asset {0} not found in any archive")]
    NotFound(CanonicalPath),
    #[error("asset {0}: {1}")]
    Archive(CanonicalPath, String),
}

/// Reads the asset at `path` (relative to `mpq://`, canonical) from
/// `source`. A non-canonical path is an error, not folded silently: it
/// would load a second handle for the same file.
pub fn read_asset<S: FileSource + ?Sized>(source: &S, path: &str) -> Result<Vec<u8>, ReadError> {
    let path = CanonicalPath::parse_canonical(path)?;
    match source.read_file(&path.archive_name()) {
        None => Err(ReadError::NotFound(path)),
        Some(Err(msg)) => Err(ReadError::Archive(path, msg)),
        Some(Ok(bytes)) => Ok(bytes),
    }
}

/// An in-memory [`FileSource`] keyed case-insensitively like an archive
/// (for tests and tools; holds no game data of its own).
#[derive(Debug, Default, Clone)]
pub struct MemorySource {
    files: std::collections::BTreeMap<String, Vec<u8>>,
}

impl MemorySource {
    pub fn insert(&mut self, archive_name: &str, bytes: Vec<u8>) {
        self.files.insert(mpq_fold(archive_name), bytes);
    }
}

/// Archive name folding: `a`–`z` → `A`–`Z`, `/` → `\`.
fn mpq_fold(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' => '\\',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

impl FileSource for MemorySource {
    fn read_file(&self, archive_name: &str) -> Option<Result<Vec<u8>, String>> {
        self.files.get(&mpq_fold(archive_name)).cloned().map(Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_spellings_one_path() {
        let a = CanonicalPath::new(r"DATA\Global\X.DC6").unwrap();
        let b = CanonicalPath::new(r"data\global\x.dc6").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.as_str(), "data/global/x.dc6");
        assert_eq!(a.asset_path(), "mpq://data/global/x.dc6");
        assert_eq!(a.archive_name(), r"data\global\x.dc6");
    }

    #[test]
    fn invalid_paths_are_errors() {
        assert_eq!(CanonicalPath::new(""), Err(PathError::Empty));
        assert!(matches!(
            CanonicalPath::new(r"\data\x.dc6"),
            Err(PathError::Leading(_))
        ));
        assert!(matches!(
            CanonicalPath::new("data//x.dc6"),
            Err(PathError::BadComponent(_))
        ));
        assert!(matches!(
            CanonicalPath::new("data/../x.dc6"),
            Err(PathError::BadComponent(_))
        ));
        assert!(matches!(
            CanonicalPath::new("data/x.dc6/"),
            Err(PathError::BadComponent(_))
        ));
        assert!(matches!(
            CanonicalPath::new("data/x.dc6#0"),
            Err(PathError::Reserved(_))
        ));
        assert!(matches!(
            CanonicalPath::new("data/é.dc6"),
            Err(PathError::NonAscii(_))
        ));
    }

    fn source() -> MemorySource {
        let mut s = MemorySource::default();
        s.insert(r"data\global\X.dc6", vec![1, 2, 3]);
        s
    }

    #[test]
    fn reader_reads_canonical_paths() {
        assert_eq!(
            read_asset(&source(), "data/global/x.dc6"),
            Ok(vec![1, 2, 3])
        );
    }

    #[test]
    fn missing_path_is_an_error_naming_it() {
        let err = read_asset(&source(), "data/global/missing.dc6").unwrap_err();
        assert!(matches!(err, ReadError::NotFound(_)));
        assert!(err.to_string().contains("data/global/missing.dc6"), "{err}");
    }

    #[test]
    fn reader_rejects_non_canonical_paths() {
        // The same file, spelled with capitals, is refused rather than loaded under a
        // second handle.
        let err = read_asset(&source(), "data/global/X.dc6").unwrap_err();
        assert_eq!(
            err,
            ReadError::Path(PathError::NotCanonical(
                "data/global/X.dc6".into(),
                "data/global/x.dc6".into()
            ))
        );
        assert!(err.to_string().contains("data/global/X.dc6"));
    }

    #[test]
    fn archive_errors_name_the_path() {
        struct Broken;
        impl FileSource for Broken {
            fn read_file(&self, _: &str) -> Option<Result<Vec<u8>, String>> {
                Some(Err("bad sector".into()))
            }
        }
        let err = read_asset(&Broken, "data/x.dc6").unwrap_err();
        assert_eq!(err.to_string(), "asset data/x.dc6: bad sector");
    }
}
