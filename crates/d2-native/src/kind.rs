//! The contract every native kind keeps: a decoded `d2-formats` value is
//! written to native files, read back from them, and compared with the
//! value it came from. The converter runs [`round_trip`] on every file it
//! converts; the runtime native source only calls [`NativeKind::read`].
//!
//! Everything here works on files in memory. A writer returns
//! [`NativeFile`]s (paths relative to the native `base/` folder) and a
//! reader pulls bytes from a [`FileStore`], so the same code serves the
//! converter's scratch folder, a mod layer on disk and the tests. How the
//! bytes reach the disk is the caller's business (the converter writes to
//! `.work/` and renames into place).

use std::collections::BTreeMap;
use std::fmt;

use d2_formats::palette::Rgb;

/// One file a writer produced. `path` is relative to `base/` and built from
/// the source's canonical path `P` plus the kind's suffix (`P.toml`,
/// `P.png`, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeFile {
    /// Path relative to `base/`.
    pub path: String,
    /// The file contents.
    pub bytes: Vec<u8>,
}

/// A load or write error naming the native file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{file}: {message}")]
pub struct NativeError {
    /// The native file concerned.
    pub file: String,
    /// What is wrong with it.
    pub message: String,
}

impl NativeError {
    /// An error for `file` with `message`.
    pub fn new(file: impl Into<String>, message: impl Into<String>) -> Self {
        NativeError {
            file: file.into(),
            message: message.into(),
        }
    }
}

/// Where a reader finds native files, by path relative to `base/`.
pub trait FileStore {
    /// The file's bytes, or `None` when the path does not exist.
    fn read(&self, path: &str) -> Option<Vec<u8>>;
}

/// A store held in memory, keyed by path: the converter reads its freshly
/// written files back through it, and tests build one by hand.
#[derive(Debug, Clone, Default)]
pub struct MemStore(pub BTreeMap<String, Vec<u8>>);

impl MemStore {
    /// A store holding `files`, keyed by path.
    pub fn from_files(files: &[NativeFile]) -> Self {
        MemStore(
            files
                .iter()
                .map(|f| (f.path.clone(), f.bytes.clone()))
                .collect(),
        )
    }
}

impl FileStore for MemStore {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.0.get(path).cloned()
    }
}

/// The colours written to the `PLTE` chunk of every indexed PNG. They only
/// make the pictures viewable in an ordinary image viewer: the pixels are
/// palette indices, and the loaders ignore `PLTE`. The converter passes
/// the act 1 palette.
pub type ViewPalette = [Rgb; 256];

/// A grey ramp (index `i` shows as grey level `i`), for callers with no
/// palette at hand.
pub fn grey_palette() -> ViewPalette {
    std::array::from_fn(|i| {
        let v = i as u8;
        Rgb { r: v, g: v, b: v }
    })
}

/// Where a read-back value first differs from the value that was written:
/// the native file holding the difference, and a description that names
/// the direction, frame and pixel (or the field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// The native file holding the difference.
    pub file: String,
    /// Where the values differ.
    pub detail: String,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file, self.detail)
    }
}

/// Why a round-trip check failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    /// Writing the native files failed.
    #[error("write failed: {0}")]
    Write(NativeError),
    /// The written files could not be read back.
    #[error("read-back failed: {0}")]
    Read(NativeError),
    /// The read-back value differs from the written one.
    #[error("mismatch: {0}")]
    Mismatch(Difference),
}

/// A decoded value with a native form: a sprite sheet, a tile set, a
/// palette and so on. `write` and `read` are exact inverses, which
/// [`round_trip`] proves for every converted file.
pub trait NativeKind: Sized + PartialEq + fmt::Debug {
    /// The `native = "<kind>"` tag of the sidecar.
    const KIND: &'static str;
    /// The per-kind `native_version` this build writes and reads.
    const NATIVE_VERSION: u32 = 1;

    /// The native files of the value whose canonical source path is `p`
    /// (e.g. `data/global/ui/panel/x.dc6`), sorted by path. Deterministic:
    /// the same value always gives the same bytes.
    fn write(&self, p: &str, view: &ViewPalette) -> Result<Vec<NativeFile>, NativeError>;

    /// The value back from the native files of `p`. Strict: anything that
    /// `write` would not have produced (a missing key, a stray key, another
    /// spelling of a number) is an error naming the file, so a hand-edited
    /// file is either exact or refused.
    fn read(p: &str, store: &dyn FileStore) -> Result<Self, NativeError>;

    /// The first difference between `self` (the decoded source value) and
    /// `native` (read back), or `None` when they are equal.
    fn first_difference(&self, native: &Self, p: &str) -> Option<Difference>;
}

/// The round-trip check: writes `original`, reads the files back and
/// compares. Returns the written files when the two values are equal.
pub fn round_trip<K: NativeKind>(
    original: &K,
    p: &str,
    view: &ViewPalette,
) -> Result<Vec<NativeFile>, CheckError> {
    let files = original.write(p, view).map_err(CheckError::Write)?;
    check_files(original, p, &files)?;
    Ok(files)
}

/// The second half of [`round_trip`]: reads `files` back and compares the
/// result with `original`. Also called on its own, on files read back from
/// disk or deliberately altered by a test.
pub fn check_files<K: NativeKind>(
    original: &K,
    p: &str,
    files: &[NativeFile],
) -> Result<(), CheckError> {
    let store = MemStore::from_files(files);
    let back = K::read(p, &store).map_err(CheckError::Read)?;
    if &back == original {
        return Ok(());
    }
    Err(CheckError::Mismatch(
        original.first_difference(&back, p).unwrap_or(Difference {
            file: p.to_string(),
            detail: "decoded values differ".into(),
        }),
    ))
}

/// A file that must exist in `store`, or an error naming it.
pub(crate) fn need(store: &dyn FileStore, path: &str) -> Result<Vec<u8>, NativeError> {
    store
        .read(path)
        .ok_or_else(|| NativeError::new(path, "missing native file"))
}

/// Compares the listed fields of two structs in order and returns the first
/// one that differs, from the enclosing function, as a [`Difference`] in
/// `$file` (`$what` names the struct in the message).
macro_rules! diff_fields {
    ($file:expr, $what:expr, $a:expr, $b:expr, [$($f:ident),+ $(,)?]) => {
        $(
            if $a.$f != $b.$f {
                return Some($crate::kind::Difference {
                    file: $file.to_string(),
                    detail: format!(
                        "{}: {} {:?} != {:?}",
                        $what,
                        stringify!($f),
                        $a.$f,
                        $b.$f
                    ),
                });
            }
        )+
    };
}
pub(crate) use diff_fields;

/// The first differing pixel of two index buffers laid out `width` wide,
/// as `pixel (x, y): index a != b`, or the two pixel counts when the
/// buffers differ in length.
pub(crate) fn pixel_diff(a: &[u8], b: &[u8], width: usize) -> Option<String> {
    if a.len() != b.len() {
        return Some(format!("pixel count {} != {}", a.len(), b.len()));
    }
    let i = a.iter().zip(b).position(|(x, y)| x != y)?;
    let w = width.max(1);
    Some(format!(
        "pixel ({}, {}): index {} != {}",
        i % w,
        i / w,
        a[i],
        b[i]
    ))
}
