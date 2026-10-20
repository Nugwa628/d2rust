//! The native source: a converted root (`manifest.toml`, `files.tsv`,
//! `base/`, `mods/`) read through its layer stack. It is the runtime switch
//! between the player's archives and converted native files: an asset
//! loader asks for a canonical path and gets the same *decoded* value
//! ([`NativeAsset`]) either way. [`decode_original`] builds that value from
//! archive bytes, [`NativeSource::read_native`] from native files, so the
//! loaders have one code path for both.
//!
//! Opening a root checks that it is usable as a whole: the manifest's
//! format and every kind version are ones this build reads, the conversion
//! finished, `files.tsv` is the one the manifest describes, and every
//! enabled mod is well formed. A root that fails tells the player to run
//! the converter again ([`RERUN`]).
//!
//! The game tables come through [`NativeTables`] (see its module).

mod tables;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use d2_formats::animdata::AnimData;
use d2_formats::cof::Cof;
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::font::FontTable;
use d2_formats::palette::{Palette, Pl2};
use d2_formats::tbl::StringTable;
use d2_formats::wav::Wav;
use sha2::{Digest, Sha256};

use crate::expfield::ExpFieldData;
use crate::kind::{FileStore, NativeError, NativeKind};
use crate::layers::{Layer, LayerError, LayerStack};
use crate::manifest::{Manifest, FORMAT_VERSION};
use crate::toml_kinds::TextError;
use crate::{animdata, tbl, toml_kinds, wav};

pub use tables::NativeTables;

/// What the player is told to do when the root is unusable.
pub const RERUN: &str =
    "run the converter again: cargo run -p d2-convert -- convert --install <D2 dir> --out <native dir>";

/// Every kind this build reads, with its `native_version`.
pub const KNOWN_KINDS: &[(&str, u32)] = &[
    ("animdata", 1),
    ("cof", 1),
    ("colormap", 1),
    ("dc6", 1),
    ("dcc", 1),
    ("ds1", 1),
    ("dt1", 1),
    ("excel", 1),
    ("expfield", 1),
    ("font", 1),
    ("pal", 1),
    ("pl2", 1),
    ("tbl", 1),
    ("wav", 1),
];

/// The kind of a canonical source path, as the runtime source sees it.
///
/// This is wider than the converter's [`crate::asset::AssetKind`]: sounds,
/// excel tables and item colour maps are served too, though their native
/// file is a plain copy of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssetKind {
    /// A DC6 sprite (`.dc6`).
    Dc6,
    /// A DCC sprite (`.dcc`).
    Dcc,
    /// A COF animation layout (`.cof`).
    Cof,
    /// A DT1 tileset (`.dt1`).
    Dt1,
    /// A DS1 level (`.ds1`).
    Ds1,
    /// A palette (`pal.dat`).
    Pal,
    /// A palette transform (`.pl2`).
    Pl2,
    /// A string table (`.tbl`).
    Tbl,
    /// A font table (a `.tbl` under `data/local/font/`).
    Font,
    /// A sound (`.wav`).
    Wav,
    /// An Excel table source (`.txt` under `data/global/excel/`).
    Excel,
    /// An item colour map (see [`is_colormap`]).
    ColorMap,
    /// The animation data file (`data/global/animdata.d2`).
    AnimData,
    /// The expansion field data file (`expfield.d2`).
    ExpField,
}

impl AssetKind {
    /// The kind of canonical path `p`, or `None` for a kind that is not
    /// converted.
    pub fn of(p: &str) -> Option<AssetKind> {
        let ext = p.rsplit_once('.').map(|(_, e)| e)?;
        Some(match ext {
            "dc6" => AssetKind::Dc6,
            "dcc" => AssetKind::Dcc,
            "cof" => AssetKind::Cof,
            "dt1" => AssetKind::Dt1,
            "ds1" => AssetKind::Ds1,
            "pl2" => AssetKind::Pl2,
            "wav" => AssetKind::Wav,
            "dat" if p.ends_with("/pal.dat") => AssetKind::Pal,
            "dat" if is_colormap(p) => AssetKind::ColorMap,
            "d2" if p == "data/global/animdata.d2" => AssetKind::AnimData,
            "d2" if p.ends_with("/expfield.d2") || p == "expfield.d2" => AssetKind::ExpField,
            "tbl" if p.starts_with("data/local/font/") => AssetKind::Font,
            "tbl" => AssetKind::Tbl,
            "txt" if p.starts_with("data/global/excel/") => AssetKind::Excel,
            _ => return None,
        })
    }

    /// The native file that decides which layer owns the asset. Every other
    /// file of the asset is read from that same layer, so a mod that
    /// replaces a sprite's sidecar also supplies its pixels and the two
    /// halves never mix.
    pub fn primary(self, p: &str) -> String {
        match self {
            AssetKind::Wav | AssetKind::Excel | AssetKind::ColorMap => p.to_owned(),
            AssetKind::Pal => format!("{p}.pal"),
            AssetKind::Tbl | AssetKind::AnimData => format!("{p}.tsv"),
            _ => format!("{p}.toml"),
        }
    }
}

/// An item colour map: a `.dat` directly under `data/global/items/palette/`
/// (see [`NativeAsset::ColorMap`]).
pub fn is_colormap(p: &str) -> bool {
    p.strip_prefix("data/global/items/palette/")
        .is_some_and(|rest| !rest.contains('/') && rest.ends_with(".dat"))
}

/// A decoded asset, as both sources return it.
#[derive(Debug, Clone, PartialEq)]
// One value per load, moved once into its asset: boxing buys nothing.
#[allow(clippy::large_enum_variant)]
pub enum NativeAsset {
    /// A DC6 sprite.
    Dc6(Dc6),
    /// A DCC sprite.
    Dcc(Dcc),
    /// A COF animation layout.
    Cof(Cof),
    /// A DT1 tileset.
    Dt1(Dt1),
    /// A DS1 level.
    Ds1(Ds1),
    /// A palette.
    Pal(Palette),
    /// A palette transform.
    Pl2(Pl2),
    /// A string table.
    Tbl(StringTable),
    /// A font table.
    Font(FontTable),
    /// A sound.
    Wav(Wav),
    /// The `.txt` bytes (tables compile from them).
    Excel(Vec<u8>),
    /// An item colour map's bytes, 21 rows of 256 indices.
    ColorMap(Vec<u8>),
    /// The animation data.
    AnimData(AnimData),
    /// The expansion field data.
    ExpField(ExpFieldData),
}

/// Decodes the archive bytes of canonical path `p` with the `d2-formats`
/// readers: the archive side of the switch. `None` for a kind that is not
/// converted; an error names the path.
pub fn decode_original(p: &str, bytes: &[u8]) -> Option<Result<NativeAsset, String>> {
    let e = |x: d2_formats::FormatError| x.to_string();
    let kind = AssetKind::of(p)?;
    let decoded = match kind {
        AssetKind::Dc6 => Dc6::parse(bytes).map(NativeAsset::Dc6).map_err(e),
        AssetKind::Dcc => Dcc::parse(bytes).map(NativeAsset::Dcc).map_err(e),
        AssetKind::Cof => Cof::parse(bytes).map(NativeAsset::Cof).map_err(e),
        AssetKind::Dt1 => Dt1::parse(bytes).map(NativeAsset::Dt1).map_err(e),
        AssetKind::Ds1 => Ds1::parse(bytes).map(NativeAsset::Ds1).map_err(e),
        AssetKind::Pal => Palette::parse(bytes).map(NativeAsset::Pal).map_err(e),
        AssetKind::Pl2 => Pl2::parse(bytes).map(NativeAsset::Pl2).map_err(e),
        AssetKind::Tbl => StringTable::parse(bytes).map(NativeAsset::Tbl).map_err(e),
        AssetKind::Font => FontTable::parse(bytes).map(NativeAsset::Font).map_err(e),
        AssetKind::Wav => Wav::parse(bytes).map(NativeAsset::Wav).map_err(e),
        AssetKind::Excel => Ok(NativeAsset::Excel(bytes.to_vec())),
        AssetKind::ColorMap => Ok(NativeAsset::ColorMap(bytes.to_vec())),
        AssetKind::AnimData => AnimData::parse(bytes).map(NativeAsset::AnimData).map_err(e),
        AssetKind::ExpField => ExpFieldData::from_bytes(bytes)
            .map(NativeAsset::ExpField)
            .map_err(|x| x.to_string()),
    };
    Some(decoded.map_err(|m| format!("{p}: {m}")))
}

/// Why a native root cannot be used. Every message names the root or file
/// and, for the root, tells the player to run the converter again.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    #[error("native assets in {root}: {reason}; {RERUN}")]
    /// The root is missing, unreadable or inconsistent.
    Root { root: String, reason: String },
    #[error(transparent)]
    /// A mod layer failed to load.
    Layer(#[from] LayerError),
}

/// Lowercase hex SHA-256 (the manifest's `files_sha256` spelling).
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Canonical form of an archive name: `\` becomes `/`, letters lowercase.
pub fn fold(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '\\' => '/',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

/// A converted root with its layer stack.
#[derive(Debug, Clone)]
pub struct NativeSource {
    /// The converted root directory.
    pub root: PathBuf,
    /// The root's `manifest.toml`.
    pub manifest: Manifest,
    /// The base layer and the mods, in load order.
    pub layers: LayerStack,
}

/// A [`FileStore`] over one layer only, so the files of one asset never
/// come from two layers.
struct LayerStore<'a> {
    stack: &'a LayerStack,
    layer: &'a Layer,
}

impl FileStore for LayerStore<'_> {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.stack.read_in(self.layer, path).and_then(Result::ok)
    }
}

impl LayerStore<'_> {
    /// The text of native file `name`, or an error naming it.
    fn text(&self, name: String) -> Result<String, String> {
        let bytes = self
            .read(&name)
            .ok_or_else(|| format!("{name}: missing in layer `{}`", self.layer.name))?;
        String::from_utf8(bytes).map_err(|_| format!("{name}: not UTF-8"))
    }

    /// The bytes of a copied file `p` (sounds, excel tables, colour maps).
    fn copy(&self, p: &str) -> Result<Vec<u8>, String> {
        self.read(p).ok_or_else(|| format!("{p}: unreadable"))
    }

    /// Reads and decodes the asset of kind `kind` at canonical path `p`.
    fn decode(&self, kind: AssetKind, p: &str) -> Result<NativeAsset, String> {
        let n = |e: NativeError| e.to_string();
        let t = |e: TextError| e.to_string();
        match kind {
            AssetKind::Dc6 => Dc6::read(p, self).map(NativeAsset::Dc6).map_err(n),
            AssetKind::Dcc => Dcc::read(p, self).map(NativeAsset::Dcc).map_err(n),
            AssetKind::Dt1 => Dt1::read(p, self).map(NativeAsset::Dt1).map_err(n),
            AssetKind::Pal => Palette::read(p, self).map(NativeAsset::Pal).map_err(n),
            AssetKind::Pl2 => Pl2::read(p, self).map(NativeAsset::Pl2).map_err(n),
            AssetKind::ExpField => ExpFieldData::read(p, self)
                .map(NativeAsset::ExpField)
                .map_err(n),
            AssetKind::Cof => toml_kinds::read_cof(p, &self.text(format!("{p}.toml"))?)
                .map(NativeAsset::Cof)
                .map_err(t),
            AssetKind::Font => toml_kinds::read_font(p, &self.text(format!("{p}.toml"))?)
                .map(NativeAsset::Font)
                .map_err(t),
            AssetKind::Ds1 => toml_kinds::read_ds1(p, &self.text(format!("{p}.toml"))?)
                .map(NativeAsset::Ds1)
                .map_err(t),
            AssetKind::Tbl => {
                let tsv = self.text(format!("{p}.tsv"))?;
                let toml = self.text(format!("{p}.toml"))?;
                tbl::read_tbl(p, &tsv, &toml)
                    .map(NativeAsset::Tbl)
                    .map_err(t)
            }
            AssetKind::AnimData => animdata::read_animdata(p, &self.text(format!("{p}.tsv"))?)
                .map(NativeAsset::AnimData)
                .map_err(t),
            AssetKind::Wav => wav::read_wav(p, &self.copy(p)?)
                .map(NativeAsset::Wav)
                .map_err(t),
            AssetKind::Excel => self.copy(p).map(NativeAsset::Excel),
            AssetKind::ColorMap => self.copy(p).map(NativeAsset::ColorMap),
        }
    }
}

impl NativeSource {
    /// Opens `root`: its `manifest.toml` is known and loadable, the
    /// manifest's `files_sha256` is the hash of `files.tsv`, and the layer
    /// stack of `mods/order.toml` opens cleanly.
    pub fn open(root: &Path) -> Result<NativeSource, SourceError> {
        NativeSource::open_with(root, None)
    }

    /// [`NativeSource::open`] with the layer list chosen by the caller:
    /// `Some(mods)` stacks `base/` and exactly those folders under `mods/`,
    /// in that order (`Some(&[])` is `base/` alone), and never reads
    /// `mods/order.toml`; `None` reads the order file as `open` does. A
    /// game's builder passes its own list (`docs/plans/mods/seams.md` MS2).
    pub fn open_with(root: &Path, order: Option<&[String]>) -> Result<NativeSource, SourceError> {
        let fail = |reason: String| SourceError::Root {
            root: root.display().to_string(),
            reason,
        };
        let text = std::fs::read_to_string(root.join("manifest.toml"))
            .map_err(|e| fail(format!("manifest.toml: {e}")))?;
        let manifest = Manifest::from_toml(&text).map_err(|e| fail(e.to_string()))?;
        let known: BTreeMap<String, u32> = KNOWN_KINDS
            .iter()
            .map(|&(k, v)| (k.to_owned(), v))
            .collect();
        manifest
            .check_loadable(&known)
            .map_err(|e| fail(e.to_string()))?;
        let tsv =
            std::fs::read(root.join("files.tsv")).map_err(|e| fail(format!("files.tsv: {e}")))?;
        if sha256_hex(&tsv) != manifest.files_sha256 {
            return Err(fail(
                "files.tsv does not match manifest.toml files_sha256".into(),
            ));
        }
        let layers = match order {
            Some(mods) => LayerStack::with_mods(root, mods)?,
            None => LayerStack::open(root)?,
        };
        Ok(NativeSource {
            root: root.to_path_buf(),
            manifest,
            layers,
        })
    }

    /// The kind of the asset at canonical path `p` and the layer that owns
    /// it, if any layer has it.
    fn owner(&self, p: &str) -> Option<(AssetKind, &Layer)> {
        let kind = AssetKind::of(p)?;
        let layer = self.layers.find(&kind.primary(p))?;
        Some((kind, layer))
    }

    /// Whether the asset at canonical path `p` has a native file.
    pub fn contains(&self, p: &str) -> bool {
        self.owner(p).is_some()
    }

    /// The decoded asset at canonical path `p`, from the last layer that
    /// has it. `None` when no layer has it (the caller reports the missing
    /// file); `Some(Err)` names the file when a reader refuses it.
    pub fn read_native(&self, p: &str) -> Option<Result<NativeAsset, String>> {
        let (kind, layer) = self.owner(p)?;
        let store = LayerStore {
            stack: &self.layers,
            layer,
        };
        Some(store.decode(kind, p))
    }

    /// The table source for `language` (`eng`).
    pub fn tables(&self, language: &str) -> Result<NativeTables, d2_data::bin::LoadError> {
        NativeTables::build(self, language)
    }
}

/// The native format version this build reads (re-exported for callers).
pub const NATIVE_FORMAT_VERSION: u32 = FORMAT_VERSION;
