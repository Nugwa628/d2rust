//! Bevy asset source `mpq://` backed by the user's game files, and asset loaders for the D2
//! formats.
//!
//! This is the client's door to the game's data. The files come either from the player's MPQ
//! archives or from a converted native folder; [`choose_source`] picks which, and the source is
//! registered under the name `mpq://` in both cases. On archives, loaders parse the game's file
//! bytes; on a native folder the source hands the loaders already-decoded typed assets.
//!
//! Paths are canonical ([`path`]): lowercase, `/` separators:
//! `mpq://data/global/palette/act1/pal.dat`. A path that is not canonical is refused, never
//! guessed at. Residency and budgets are the plain-Rust [`cache`]; [`prefetch`] queues loads
//! ahead of need; [`game_files`] bundles table and file reads for the play app.

pub mod cache;
pub mod game_files;
pub mod path;
pub mod prefetch;
pub mod size;
pub mod tbl;

use std::path::Path;
use std::sync::Arc;

use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, PathStream, Reader, VecReader,
};
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use d2_formats::cof::Cof;
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::mpq::ArchiveSet;
use d2_formats::palette::{Palette, Pl2};

use d2_native::source::{NativeAsset, NativeSource};

pub use self::path::{CanonicalPath, FileSource};
pub use self::tbl::TblAsset;

/// Asset source name: `mpq://...`.
pub const SOURCE: &str = "mpq";

/// Archive path (`\` separators) for an asset path (`/` separators).
pub fn archive_name(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

/// Asset path (with source) for an archive path, in canonical spelling:
/// `DATA\Global\X.DC6` → `mpq://data/global/x.dc6`. Total; a path
/// that is not valid ([`CanonicalPath::new`]) is refused by the reader.
pub fn asset_path(archive_path: &str) -> String {
    format!("{SOURCE}://{}", path::fold(archive_path))
}

struct MpqReader<S: ?Sized> {
    archives: Arc<S>,
}

/// A read failure that is not "not found", as Bevy's reader error.
fn io_error(message: String) -> AssetReaderError {
    AssetReaderError::Io(Arc::new(std::io::Error::other(message)))
}

impl<S: FileSource + ?Sized> AssetReader for MpqReader<S> {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let name = path.to_string_lossy();
        if self.archives.is_native() {
            // A presence mark only: the loader reads the typed asset itself.
            let canon =
                CanonicalPath::parse_canonical(&name).map_err(|e| io_error(e.to_string()))?;
            return match self.archives.read_file(&canon.archive_name()) {
                Some(Ok(_)) => Ok(VecReader::new(Vec::new())),
                Some(Err(e)) => Err(io_error(format!("asset {canon}: {e}"))),
                None => Err(AssetReaderError::NotFound(path.to_path_buf())),
            };
        }
        match path::read_asset(&*self.archives, &name) {
            Ok(bytes) => Ok(VecReader::new(bytes)),
            Err(path::ReadError::NotFound(_)) => {
                Err(AssetReaderError::NotFound(path.to_path_buf()))
            }
            Err(e) => Err(io_error(e.to_string())),
        }
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        // Archives hold no .meta files; Bevy then uses default settings.
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}

/// Registers the `mpq://` source. Must be added **before** `DefaultPlugins`.
pub struct MpqSourcePlugin {
    /// The opened archives.
    pub archives: Arc<ArchiveSet>,
}

impl Plugin for MpqSourcePlugin {
    fn build(&self, app: &mut App) {
        register_source(app, self.archives.clone());
    }
}

/// Registers `mpq://` over any [`FileSource`] (a windowless test app, a
/// tool). Must be added **before** `DefaultPlugins`, like
/// [`MpqSourcePlugin`].
pub struct FileSourcePlugin {
    /// The file source to read through.
    pub source: Arc<dyn FileSource>,
}

impl Plugin for FileSourcePlugin {
    fn build(&self, app: &mut App) {
        register_source(app, self.source.clone());
    }
}

/// Registers `mpq://` over a converted native folder (the source name stays `mpq://`, so asset
/// paths are the same on either source). Add
/// **before** `DefaultPlugins` and [`D2AssetsPlugin`].
pub struct NativeSourcePlugin {
    /// The opened native folder.
    pub source: Arc<NativeSource>,
}

impl Plugin for NativeSourcePlugin {
    fn build(&self, app: &mut App) {
        let source: Arc<dyn FileSource> = self.source.clone();
        app.insert_resource(AssetSourceKind {
            native: Some(source.clone()),
        });
        register_source(app, source);
    }
}

/// Which source `mpq://` reads: `native` is set for a native folder.
#[derive(Resource, Clone, Default)]
pub struct AssetSourceKind {
    /// The native source, or `None` when `mpq://` reads archives.
    pub native: Option<Arc<dyn FileSource>>,
}

/// Where the game's files come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceChoice {
    /// A converted native folder.
    Native(std::path::PathBuf),
    /// The archives of `D2_GAME_DIR` (debug builds, or until a native
    /// folder is configured).
    Mpq,
}

/// The default native root: `<platform data dir>/d2rust/native`.
/// A pack converted before the rename sits in `<platform data dir>/d2rs/native`
/// (legacy-name); that one is used while the new folder has no pack.
pub fn default_native_dir() -> Option<std::path::PathBuf> {
    let var = |k: &str| std::env::var_os(k).map(std::path::PathBuf::from);
    let data = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h.join(".local/share")))
    };
    data.map(|d| native_dir_in(&d))
}

/// The native root under the platform data dir `data`: `d2rust/native`,
/// else the older `d2rs/native` (legacy-name) when only that holds a pack.
pub fn native_dir_in(data: &std::path::Path) -> std::path::PathBuf {
    let new = data.join("d2rust").join("native");
    let old = data.join("d2rs").join("native");
    if !new.join("manifest.toml").is_file() && old.join("manifest.toml").is_file() {
        old
    } else {
        new
    }
}

/// Picks the source, in this order: `--native <dir>`, `D2_NATIVE_DIR`,
/// then the default native root when it holds a `manifest.toml`.
/// `--source mpq` (or `D2_SOURCE=mpq`) forces the archives in a debug
/// build and is refused in a release build. With nothing
/// configured the archives stay the default, so `play` keeps working
/// before the first conversion.
pub fn choose_source(
    native_arg: Option<&std::path::Path>,
    source_arg: Option<&str>,
    env: impl Fn(&str) -> Option<String>,
    default_dir: Option<std::path::PathBuf>,
) -> Result<SourceChoice, String> {
    let source = source_arg.map(str::to_owned).or_else(|| env("D2_SOURCE"));
    match source.as_deref() {
        None | Some("native") => {}
        Some("mpq") if cfg!(debug_assertions) => return Ok(SourceChoice::Mpq),
        Some("mpq") => {
            return Err("--source mpq is for debug builds; a release build reads a native folder (run d2-convert)".into())
        }
        Some(other) => return Err(format!("unknown source {other:?} (native or mpq)")),
    }
    if let Some(d) = native_arg {
        return Ok(SourceChoice::Native(d.to_path_buf()));
    }
    if let Some(d) = env("D2_NATIVE_DIR").filter(|d| !d.is_empty()) {
        return Ok(SourceChoice::Native(d.into()));
    }
    if let Some(d) = default_dir.filter(|d| d.join("manifest.toml").is_file()) {
        return Ok(SourceChoice::Native(d));
    }
    if source.as_deref() == Some("native") {
        return Err(format!(
            "no native folder: pass --native <dir> or set D2_NATIVE_DIR; {}",
            d2_native::source::RERUN
        ));
    }
    Ok(SourceChoice::Mpq)
}

fn register_source<S: FileSource + ?Sized>(app: &mut App, archives: Arc<S>) {
    app.register_asset_source(
        SOURCE,
        AssetSourceBuilder::new(move || {
            Box::new(MpqReader {
                archives: archives.clone(),
            })
        }),
    );
}

/// Why a D2 asset failed to load.
#[derive(Debug, thiserror::Error)]
pub enum D2LoadError {
    /// Reading the bytes failed.
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    /// The bytes did not parse as the expected format.
    #[error(transparent)]
    Format(#[from] d2_formats::FormatError),
    /// A native file that is missing, refused by its reader, or of another
    /// kind; the message names the path.
    #[error("{0}")]
    Native(String),
}

/// The typed read behind every loader on a native source.
fn read_typed<A>(
    src: &dyn FileSource,
    path: &Path,
    pick: impl FnOnce(NativeAsset) -> Option<A>,
) -> Result<A, D2LoadError> {
    let name = path.to_string_lossy();
    let canon =
        CanonicalPath::parse_canonical(&name).map_err(|e| D2LoadError::Native(e.to_string()))?;
    match src.read_native(&canon) {
        None => Err(D2LoadError::Native(format!("{canon}: no native file"))),
        Some(Err(e)) => Err(D2LoadError::Native(e)),
        Some(Ok(a)) => {
            pick(a).ok_or_else(|| D2LoadError::Native(format!("{canon}: not of this asset type")))
        }
    }
}

/// Declares an asset wrapper and a loader that parses it with `parse`, or
/// takes the decoded `NativeAsset::$variant` from a native source.
macro_rules! d2_asset {
    ($asset:ident, $loader:ident, $inner:ty, $parse:path, $variant:ident, [$($ext:literal),*]) => {
        #[derive(Asset, TypePath, Debug)]
        pub struct $asset(pub $inner);

        d2_loader!(
            $asset,
            $loader,
            |bytes| Ok($asset($parse(bytes)?)),
            |native| match native {
                NativeAsset::$variant(x) => Some($asset(x)),
                _ => None,
            },
            [$($ext),*]
        );
    };
}

/// Declares a loader that builds `$asset` from the file bytes, or from the
/// typed asset when the source is native.
macro_rules! d2_loader {
    ($asset:ident, $loader:ident, |$bytes:ident| $build:expr, |$native:ident| $typed:expr, [$($ext:literal),*]) => {
        #[derive(Default, TypePath)]
        pub struct $loader {
            /// The native source, when the game runs on one.
            pub native: Option<Arc<dyn FileSource>>,
        }

        impl AssetLoader for $loader {
            type Asset = $asset;
            type Settings = ();
            type Error = D2LoadError;

            async fn load(
                &self,
                reader: &mut dyn Reader,
                _settings: &(),
                load_context: &mut LoadContext<'_>,
            ) -> Result<$asset, D2LoadError> {
                let mut buf = Vec::new();
                reader.read_to_end(&mut buf).await?;
                if let Some(src) = &self.native {
                    return read_typed(&**src, load_context.path().path(), |$native| $typed);
                }
                let $bytes: &[u8] = &buf;
                $build
            }

            fn extensions(&self) -> &[&str] {
                &[$($ext),*]
            }
        }
    };
}

d2_asset!(Ds1Asset, Ds1Loader, Ds1, Ds1::parse, Ds1, ["ds1"]);
d2_asset!(Dt1Asset, Dt1Loader, Dt1, Dt1::parse, Dt1, ["dt1"]);
d2_asset!(
    PaletteAsset,
    PaletteLoader,
    Palette,
    Palette::parse,
    Pal,
    ["dat"]
);
d2_asset!(Dc6Asset, Dc6Loader, Dc6, Dc6::parse, Dc6, ["dc6"]);
d2_asset!(DccAsset, DccLoader, Dcc, Dcc::parse, Dcc, ["dcc"]);
d2_asset!(Pl2Asset, Pl2Loader, Pl2, Pl2::parse, Pl2, ["pl2"]);
d2_asset!(CofAsset, CofLoader, Cof, Cof::parse, Cof, ["cof"]);
d2_loader!(
    TblAsset,
    TblLoader,
    |bytes| Ok(TblAsset::parse(bytes)?),
    |native| match native {
        NativeAsset::Font(f) => Some(TblAsset::Font(f)),
        NativeAsset::Tbl(t) => Some(TblAsset::Strings(t)),
        _ => None,
    },
    ["tbl"]
);

/// Registers the D2 asset types and loaders. Add **after** `DefaultPlugins`.
pub struct D2AssetsPlugin;

impl Plugin for D2AssetsPlugin {
    fn build(&self, app: &mut App) {
        // A native source registered before this plugin gives the loaders
        // their typed reads.
        let native = app
            .world()
            .get_resource::<AssetSourceKind>()
            .and_then(|k| k.native.clone());
        macro_rules! register {
            ($($asset:ty => $loader:ident),*) => {
                $(app.init_asset::<$asset>();)*
                $(app.register_asset_loader($loader { native: native.clone() });)*
            };
        }
        register!(
            Ds1Asset => Ds1Loader,
            Dt1Asset => Dt1Loader,
            PaletteAsset => PaletteLoader,
            Dc6Asset => Dc6Loader,
            DccAsset => DccLoader,
            Pl2Asset => Pl2Loader,
            CofAsset => CofLoader,
            TblAsset => TblLoader
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::LoadState;

    #[test]
    fn path_conversion() {
        assert_eq!(
            archive_name(Path::new("data/global/palette/act1/pal.dat")),
            r"data\global\palette\act1\pal.dat"
        );
        assert_eq!(
            asset_path(r"data\global\tiles\ACT1\TOWN\floor.dt1"),
            "mpq://data/global/tiles/act1/town/floor.dt1"
        );
    }

    #[test]
    fn a_pack_in_the_old_native_folder_is_still_found() {
        let data = std::env::temp_dir().join(format!("d2rust-native-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let (new, old) = (data.join("d2rust/native"), data.join("d2rs/native"));
        assert_eq!(native_dir_in(&data), new);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("manifest.toml"), b"x").unwrap();
        assert_eq!(native_dir_in(&data), old);
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("manifest.toml"), b"x").unwrap();
        assert_eq!(native_dir_in(&data), new);
        let _ = std::fs::remove_dir_all(&data);
    }

    /// A windowless app with `mpq://` over `files`.
    fn app(files: path::MemorySource) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            FileSourcePlugin {
                source: Arc::new(files),
            },
            AssetPlugin::default(),
            D2AssetsPlugin,
        ));
        app
    }

    /// Runs the app until `id` leaves the loading states.
    fn settle(app: &mut App, id: impl Into<bevy::asset::UntypedAssetId> + Copy) -> LoadState {
        let start = std::time::Instant::now();
        loop {
            app.update();
            let state = app.world().resource::<AssetServer>().load_state(id);
            if !matches!(state, LoadState::NotLoaded | LoadState::Loading) {
                return state;
            }
            assert!(start.elapsed().as_secs() < 30, "asset never settled");
            std::thread::yield_now();
        }
    }

    #[test]
    fn two_spellings_one_handle() {
        // Paths through the AssetServer; and a `tbl` with the
        // font magic loads as `TblAsset::Font`.
        let mut font = b"Woo!".to_vec();
        font.extend_from_slice(&1u16.to_le_bytes());
        font.extend_from_slice(&[0, 0, 0, 0, 16, 12]);
        let mut files = path::MemorySource::default();
        files.insert(r"data\local\font\latin\font16.tbl", font);
        let mut app = app(files);
        let server = app.world().resource::<AssetServer>().clone();
        let a: Handle<TblAsset> = server.load(asset_path(r"DATA\Local\FONT\LATIN\Font16.TBL"));
        let b: Handle<TblAsset> = server.load(asset_path(r"data\local\font\latin\font16.tbl"));
        assert_eq!(a.id(), b.id());
        assert!(settle(&mut app, a.id()).is_loaded());
        let assets = app.world().resource::<Assets<TblAsset>>();
        assert!(matches!(assets.get(&a), Some(TblAsset::Font(_))));
    }

    #[test]
    fn source_choice_order() {
        let none = |_: &str| None::<String>;
        let d = |s: &str| std::path::PathBuf::from(s);
        assert_eq!(
            choose_source(Some(Path::new("/a")), None, none, None),
            Ok(SourceChoice::Native(d("/a")))
        );
        let env = |k: &str| (k == "D2_NATIVE_DIR").then(|| "/b".to_string());
        assert_eq!(
            choose_source(None, None, env, None),
            Ok(SourceChoice::Native(d("/b")))
        );
        assert_eq!(
            choose_source(Some(Path::new("/a")), None, env, None),
            Ok(SourceChoice::Native(d("/a")))
        );
        // Nothing configured, no default root: the archives (transition).
        assert_eq!(choose_source(None, None, none, None), Ok(SourceChoice::Mpq));
        let e = choose_source(None, Some("native"), none, None).unwrap_err();
        assert!(e.contains("d2-convert"), "{e}");
        assert!(choose_source(None, Some("zip"), none, None).is_err());
        let mpq = choose_source(Some(Path::new("/a")), Some("mpq"), none, None);
        if cfg!(debug_assertions) {
            assert_eq!(mpq, Ok(SourceChoice::Mpq));
        } else {
            assert!(mpq.is_err());
        }
    }

    #[test]
    fn native_source_loads_typed_assets() {
        use d2_native::kind::{grey_palette, NativeKind};
        use d2_native::manifest::{write_files_tsv, Manifest};
        use d2_native::source::{sha256_hex, KNOWN_KINDS};
        let root = std::env::temp_dir().join(format!("d2client-native-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let raw: Vec<u8> = (0..768u32).map(|i| (i * 7) as u8).collect();
        let pal = Palette::parse(&raw).unwrap();
        let p = "data/global/palette/act1/pal.dat";
        for f in pal.write(p, &grey_palette()).unwrap() {
            let at = root.join("base").join(&f.path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, f.bytes).unwrap();
        }
        let tsv = write_files_tsv(&[]);
        std::fs::write(root.join("files.tsv"), &tsv).unwrap();
        let m = Manifest {
            converter_version: "0.1.0".into(),
            converter_commit: "test".into(),
            kinds: KNOWN_KINDS.iter().map(|&(k, v)| (k.into(), v)).collect(),
            language: "eng".into(),
            lod: true,
            archives: Vec::new(),
            counts: [("pal".to_string(), Default::default())].into(),
            unnamed_blocks: Default::default(),
            complete: true,
            files_sha256: sha256_hex(tsv.as_bytes()),
        };
        std::fs::write(root.join("manifest.toml"), m.to_toml()).unwrap();
        let src = Arc::new(NativeSource::open(&root).unwrap());

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            NativeSourcePlugin { source: src },
            AssetPlugin::default(),
            D2AssetsPlugin,
        ));
        let server = app.world().resource::<AssetServer>().clone();
        let h: Handle<PaletteAsset> = server.load(asset_path(r"DATA\GLOBAL\PALETTE\ACT1\PAL.DAT"));
        assert!(settle(&mut app, h.id()).is_loaded());
        let got = &app
            .world()
            .resource::<Assets<PaletteAsset>>()
            .get(&h)
            .unwrap()
            .0;
        assert_eq!(got, &pal);
        // A path with no native file fails naming it (no fallback).
        let m: Handle<PaletteAsset> = server.load(asset_path(r"data\global\palette\act2\pal.dat"));
        match settle(&mut app, m.id()) {
            LoadState::Failed(e) => assert!(e.to_string().contains("act2/pal.dat"), "{e}"),
            other => panic!("expected a failure, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_asset_fails_naming_the_path() {
        // No fallback asset.
        let mut app = app(path::MemorySource::default());
        let server = app.world().resource::<AssetServer>().clone();
        let h: Handle<CofAsset> = server.load(asset_path(r"data\global\missing.cof"));
        match settle(&mut app, h.id()) {
            LoadState::Failed(e) => {
                assert!(e.to_string().contains("data/global/missing.cof"), "{e}")
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }
}
