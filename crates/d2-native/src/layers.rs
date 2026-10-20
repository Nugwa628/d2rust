//! Mod layering over the converted native files.
//!
//! The layer stack is `base/` first, then each enabled mod's `files/` tree
//! in the order `mods/order.toml` lists them. A native path resolves to the
//! last layer that has it, so a later mod wins over an earlier one and any
//! mod wins over the base. The runtime native source reads every asset
//! through the stack.
//!
//! Opening the stack checks each mod before any of its files is used:
//! - its `mod.toml` is well formed and targets this build's
//!   `native_format_version`;
//! - every mod it `requires` comes earlier in the order;
//! - it ships no copy of an excel `.txt` or a string table: mods change
//!   those with table patches, so several mods can edit the same table;
//! - every two-part file is complete in the mod's own layer: a sprite,
//!   palette transform or expansion field sidecar (`P.toml`) has its
//!   pixels (`P.png`), and a tile set sidecar has its tile folder
//!   (`P.d/`), and the other way round.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::manifest::FORMAT_VERSION;

/// Name of the base layer in messages.
pub const BASE: &str = "base";
/// The file listing the enabled mods in order.
pub const ORDER_FILE: &str = "mods/order.toml";
/// Where a mod keeps its table patches, under its `files/` tree.
pub const PATCH_DIR: &str = "patches";
/// Where a mod game keeps its own side tables, under its `files/` tree
/// (`d2_data::side`).
pub const SIDE_DIR: &str = "tables";

/// A layer-stack error naming the mod and file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayerError {
    /// A file or folder could not be read: path, message.
    #[error("{0}: {1}")]
    Io(String, String),
    /// A malformed or inconsistent file.
    #[error("{file}: {detail}")]
    Bad { file: String, detail: String },
    /// A mod ships a copy of an excel table or string table.
    #[error("mod `{layer}` ships {file}: excel tables and string tables are changed by patches, not copies (rule 9)")]
    Copy { layer: String, file: String },
    /// One half of a toml / pixel pair is missing from the mod.
    #[error("mod `{layer}`: {file} has no matching {missing} in the same layer")]
    Unpaired {
        layer: String,
        file: String,
        missing: String,
    },
}

fn bad(file: impl Into<String>, detail: impl Into<String>) -> LayerError {
    LayerError::Bad {
        file: file.into(),
        detail: detail.into(),
    }
}

/// A mod's `mod.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModInfo {
    /// The mod's name.
    pub name: String,
    /// The mod's own version string.
    pub version: String,
    /// The native layout version the mod targets; must equal this build's.
    pub native_format_version: u32,
    /// Mods that must come earlier in the order.
    pub requires: Vec<String>,
}

impl ModInfo {
    /// Parses `mod.toml`. Strict: every key present and of its type.
    pub fn from_toml(file: &str, text: &str) -> Result<ModInfo, LayerError> {
        let t: toml::Table = text.parse().map_err(|e| bad(file, format!("{e}")))?;
        let s = |k: &str| {
            t.get(k)
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .ok_or_else(|| bad(file, format!("`{k}` missing or not a string")))
        };
        let v = t
            .get("native_format_version")
            .and_then(|v| v.as_integer())
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| bad(file, "`native_format_version` missing or not an integer"))?;
        let requires = match t.get("requires") {
            None => Vec::new(),
            Some(r) => r
                .as_array()
                .and_then(|a| {
                    a.iter()
                        .map(|x| x.as_str().map(str::to_owned))
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or_else(|| bad(file, "`requires` is not a list of strings"))?,
        };
        for k in t.keys() {
            if !["name", "version", "native_format_version", "requires"].contains(&k.as_str()) {
                return Err(bad(file, format!("unknown key `{k}`")));
            }
        }
        Ok(ModInfo {
            name: s("name")?,
            version: s("version")?,
            native_format_version: v,
            requires,
        })
    }
}

/// Parses `mods/order.toml`: `order = ["modA", "modB"]`.
pub fn parse_order(text: &str) -> Result<Vec<String>, LayerError> {
    let t: toml::Table = text.parse().map_err(|e| bad(ORDER_FILE, format!("{e}")))?;
    let order = t
        .get("order")
        .and_then(|v| v.as_array())
        .and_then(|a| {
            a.iter()
                .map(|x| x.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| bad(ORDER_FILE, "`order` missing or not a list of strings"))?;
    let mut seen = BTreeSet::new();
    for m in &order {
        if !seen.insert(m) {
            return Err(bad(ORDER_FILE, format!("mod `{m}` listed twice")));
        }
    }
    Ok(order)
}

/// One layer: its name and the folder native paths are relative to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// `base` or the mod's name.
    pub name: String,
    /// The folder native paths are relative to.
    pub dir: PathBuf,
    /// `None` for `base`.
    pub info: Option<ModInfo>,
}

/// The ordered layers, `base` first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerStack {
    /// `base`, then the mods in load order.
    pub layers: Vec<Layer>,
}

fn io(path: &Path, e: std::io::Error) -> LayerError {
    LayerError::Io(path.display().to_string(), e.to_string())
}

/// Every file under `dir`, as `/`-joined paths relative to it, sorted.
fn walk(dir: &Path) -> Result<Vec<String>, LayerError> {
    let mut out = Vec::new();
    let mut todo = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, prefix)) = todo.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| io(&d, e))? {
            let e = e.map_err(|e| io(&d, e))?;
            let name = e.file_name().to_string_lossy().into_owned();
            let rel = format!("{prefix}{name}");
            if e.file_type().map_err(|err| io(&e.path(), err))?.is_dir() {
                todo.push((e.path(), format!("{rel}/")));
            } else {
                out.push(rel);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Whether a mod file is a copy of a table that mods may only patch: an
/// excel `.txt`, the `.bin` overrides, or a string table's native files.
fn is_table_copy(f: &str) -> bool {
    (f.starts_with("data/global/excel/") && f.ends_with(".txt"))
        || f.starts_with("data/global/excel/_bin-overrides")
        || (f.starts_with("data/local/lng/")
            && (f.ends_with(".tbl.tsv") || f.ends_with(".tbl.toml")))
}

/// The file-list checks of one mod (`files` sorted, relative to its
/// `files/` tree): no copies of excel or string tables, and every
/// sidecar / pixel pair complete.
pub fn check_mod_files(layer: &str, files: &[String]) -> Result<(), LayerError> {
    let set: BTreeSet<&str> = files.iter().map(String::as_str).collect();
    // A folder is present when some file sorts right after `dir/` and
    // starts with it.
    let has_dir = |d: &str| {
        let p = format!("{d}/");
        set.range(p.as_str()..)
            .next()
            .is_some_and(|f| f.starts_with(&p))
    };
    for f in files {
        if is_table_copy(f) {
            return Err(LayerError::Copy {
                layer: layer.into(),
                file: f.clone(),
            });
        }
        let unpaired = |missing: String| LayerError::Unpaired {
            layer: layer.into(),
            file: f.clone(),
            missing,
        };
        // Sprite sheets, palette transforms and the expansion field:
        // `P.toml` + `P.png`.
        for ext in [".dc6", ".dcc", ".pl2", "expfield.d2"] {
            for (have, want) in [(".toml", ".png"), (".png", ".toml")] {
                if let Some(p) = f.strip_suffix(&format!("{ext}{have}")) {
                    let other = format!("{p}{ext}{want}");
                    if !set.contains(other.as_str()) {
                        return Err(unpaired(other));
                    }
                }
            }
        }
        // Tile sets: `P.toml` + the tile folder `P.d/`.
        if let Some(p) = f.strip_suffix(".dt1.toml") {
            let d = format!("{p}.dt1.d");
            if !has_dir(&d) {
                return Err(unpaired(format!("{d}/")));
            }
        }
        if let Some((p, _)) = f.split_once(".dt1.d/") {
            let t = format!("{p}.dt1.toml");
            if !set.contains(t.as_str()) {
                return Err(unpaired(t));
            }
        }
    }
    Ok(())
}

/// Opens and checks mod `name` of `root`, given the layers enabled before it.
fn open_mod(root: &Path, name: &str, earlier: &[Layer]) -> Result<Layer, LayerError> {
    let dir = root.join("mods").join(name);
    let file = format!("mods/{name}/mod.toml");
    let text = std::fs::read_to_string(dir.join("mod.toml"))
        .map_err(|e| LayerError::Io(file.clone(), e.to_string()))?;
    let info = ModInfo::from_toml(&file, &text)?;
    if info.native_format_version != FORMAT_VERSION {
        return Err(bad(
            &file,
            format!(
                "native_format_version {} is not known (this build reads {FORMAT_VERSION})",
                info.native_format_version
            ),
        ));
    }
    if let Some(r) = info
        .requires
        .iter()
        .find(|r| !earlier.iter().any(|l| &l.name == *r))
    {
        return Err(bad(
            &file,
            format!("requires `{r}`, which is not enabled before it in {ORDER_FILE}"),
        ));
    }
    let files_dir = dir.join("files");
    let files = if files_dir.is_dir() {
        walk(&files_dir)?
    } else {
        Vec::new()
    };
    check_mod_files(name, &files)?;
    Ok(Layer {
        name: name.to_owned(),
        dir: files_dir,
        info: Some(info),
    })
}

impl LayerStack {
    /// The stack of a native root: `base/`, then the mods of
    /// `mods/order.toml` (none when the file is absent).
    pub fn open(root: &Path) -> Result<LayerStack, LayerError> {
        let order_path = root.join(ORDER_FILE);
        let order = match std::fs::read_to_string(&order_path) {
            Ok(text) => parse_order(&text)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(io(&order_path, e)),
        };
        LayerStack::with_mods(root, &order)
    }

    /// The stack of `root` with the mods `order` (folders under `mods/`),
    /// each checked as the module header describes.
    pub fn with_mods(root: &Path, order: &[String]) -> Result<LayerStack, LayerError> {
        let mut layers = vec![Layer {
            name: BASE.into(),
            dir: root.join("base"),
            info: None,
        }];
        for name in order {
            let layer = open_mod(root, name, &layers)?;
            layers.push(layer);
        }
        Ok(LayerStack { layers })
    }

    /// The last layer holding the native file `path` (later wins).
    pub fn find(&self, path: &str) -> Option<&Layer> {
        self.layers
            .iter()
            .rev()
            .find(|l| l.dir.join(path).is_file())
    }

    /// Reads `path` from layer `layer` only (a pair's other half comes
    /// from the same layer).
    pub fn read_in(&self, layer: &Layer, path: &str) -> Option<Result<Vec<u8>, LayerError>> {
        let p = layer.dir.join(path);
        match std::fs::read(&p) {
            Ok(b) => Some(Ok(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => Some(Err(io(&p, e))),
        }
    }

    /// Reads `path` from the last layer that has it.
    pub fn read(&self, path: &str) -> Option<Result<Vec<u8>, LayerError>> {
        self.find(path).and_then(|l| self.read_in(l, path))
    }

    /// Every mod's table patches, in stack order: layer order, then file
    /// name (byte order) within a layer. `(stack path, bytes)`.
    pub fn patches(&self) -> Result<Vec<(String, Vec<u8>)>, LayerError> {
        self.mod_files(PATCH_DIR)
    }

    /// Every mod's side tables (`files/tables/`), in the same order as
    /// [`LayerStack::patches`], for `d2_data::side::SideTables::from_files`.
    pub fn side_tables(&self) -> Result<Vec<(String, Vec<u8>)>, LayerError> {
        self.mod_files(SIDE_DIR)
    }

    /// Every file under folder `sub` of each mod's `files/` tree: layer
    /// order, then file name (byte order). `(stack path, bytes)`.
    fn mod_files(&self, sub: &str) -> Result<Vec<(String, Vec<u8>)>, LayerError> {
        let mut out = Vec::new();
        for l in self.layers.iter().skip(1) {
            let dir = l.dir.join(sub);
            if !dir.is_dir() {
                continue;
            }
            for f in walk(&dir)? {
                let p = dir.join(&f);
                let bytes = std::fs::read(&p).map_err(|e| io(&p, e))?;
                out.push((format!("mods/{}/files/{sub}/{f}", l.name), bytes));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_tables_come_from_each_mods_tables_folder() {
        let root = std::env::temp_dir().join(format!("d2native-side-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let put = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        put("base/tables/ignored.txt", "x");
        put("a/tables/b.txt", "1");
        put("a/tables/a.txt", "2");
        put("a/patches/p.d2patch", "3");
        put("c/tables/a.txt", "4");
        let layer = |name: &str| Layer {
            name: name.into(),
            dir: root.join(name),
            info: None,
        };
        let stack = LayerStack {
            layers: vec![layer("base"), layer("a"), layer("c")],
        };
        let got = stack.side_tables().unwrap();
        let names: Vec<&str> = got.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            names,
            [
                "mods/a/files/tables/a.txt",
                "mods/a/files/tables/b.txt",
                "mods/c/files/tables/a.txt"
            ]
        );
        assert_eq!(got[2].1, b"4");
        assert_eq!(stack.patches().unwrap().len(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn files(list: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn mod_excel_and_tbl_copies_are_refused() {
        for f in [
            "data/global/excel/weapons.txt",
            "data/local/lng/eng/string.tbl.tsv",
        ] {
            let e = check_mod_files("m", &files(&[f])).unwrap_err();
            assert!(e.to_string().contains(f), "{e}");
        }
        check_mod_files("m", &files(&["patches/a.d2patch"])).unwrap();
    }

    #[test]
    fn half_pairs_are_refused() {
        let e = check_mod_files("m", &files(&["data/x.dc6.toml"])).unwrap_err();
        assert!(e.to_string().contains("data/x.dc6.png"), "{e}");
        let e = check_mod_files("m", &files(&["data/x.dcc.png"])).unwrap_err();
        assert!(e.to_string().contains("data/x.dcc.toml"), "{e}");
        let e = check_mod_files("m", &files(&["data/t.dt1.toml"])).unwrap_err();
        assert!(e.to_string().contains("data/t.dt1.d/"), "{e}");
        let e = check_mod_files("m", &files(&["data/t.dt1.d/0.png"])).unwrap_err();
        assert!(e.to_string().contains("data/t.dt1.toml"), "{e}");
        check_mod_files(
            "m",
            &files(&[
                "data/x.dc6.toml",
                "data/x.dc6.png",
                "data/t.dt1.toml",
                "data/t.dt1.d/0.png",
            ]),
        )
        .unwrap();
    }

    #[test]
    fn order_and_mod_toml_are_strict() {
        assert_eq!(
            parse_order("order = [\"a\", \"b\"]").unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(parse_order("order = [\"a\", \"a\"]").is_err());
        assert!(parse_order("order = 3").is_err());
        let ok = "name = \"a\"\nversion = \"1\"\nnative_format_version = 1\n";
        assert!(ModInfo::from_toml("m", ok).unwrap().requires.is_empty());
        assert!(ModInfo::from_toml("m", &format!("{ok}extra = 1\n")).is_err());
        assert!(ModInfo::from_toml("m", "name = \"a\"\n").is_err());
    }
}
