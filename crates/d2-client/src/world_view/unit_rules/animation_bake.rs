//! Install-time animation bake. The launcher runs `bake-anims` once after the
//! game or a mod is installed or updated; the generated in-between frames go
//! to versioned files beside the install, and play only loads them
//! ([`super::animation_baked`]). Nothing is generated while the game runs.
//!
//! One work item is one player or monster component file. Its pack holds the
//! generated samples of every source facing and frame, named by the pack
//! identity ([`hash_inputs`]). `bake.json` lists the packs with frames and is
//! written last, so a bake that stops early has no index and resumes: packs
//! already written are verified and kept, not generated again.

use super::animation_baked::{encode_baked_pack, read_baked_pack};
use super::animation_deny::AnimationDeny;
use super::animation_inputs::load_component_file;
use super::animation_pack::{BakedPack, LUT_FILE};
use super::frame_pack::{fold_code, FrameKey};
use super::generation_job::{generate_packed, hash_inputs, GENERATOR_SETTINGS};
use super::generator::{ramps_from_source, BlendTables, Generator};
use crate::app::palette::ActPalettes;
use crate::assets::path::FileSource;
use crate::rules::unit_composite::{code, Code, CompositeKind, FileFormat};
use crate::world_view::unit_assets::UnitLooks;
use d2_formats::mpq::ArchiveSet;
use d2_formats::palette::Palette;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// `bake.json` format version.
pub const BAKE_VERSION: u32 = 2;
/// The index file of a bake folder.
pub const INDEX: &str = "bake.json";
/// The pack folder of a bake folder.
pub const PACKS: &str = "packs";
/// A baked pack: one frame pack v2 as a deflate stream.
pub const PACK_EXTENSION: &str = "d2fz";

/// `bake.json`: what was baked, from which data, and the packs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BakeIndex {
    pub version: u32,
    /// The game build that baked (`launch::VERSION`).
    pub game_version: String,
    /// [`GENERATOR_SETTINGS`].
    pub generator: String,
    /// The source data stamp ([`data_stamp`]): a rerun on unchanged data
    /// returns at once.
    pub stamp: String,
    /// The mod layer baked, if any.
    #[serde(default)]
    pub mod_id: Option<String>,
    /// SHA-256 of the presented palette the packs are quantized to.
    pub palette: String,
    /// SHA-256 of the bake folder's `blend.lut` (the shared pair tables).
    #[serde(default)]
    pub blend_lut: String,
    /// The packs with at least one generated frame.
    pub packs: Vec<PackEntry>,
}

/// One baked component file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackEntry {
    /// The source file (archive path).
    pub source: String,
    /// 0 player, 1 monster.
    pub kind: u8,
    pub token: String,
    pub mode: String,
    pub component: u8,
    pub component_token: String,
    pub armor_class: String,
    pub weapon_class: String,
    /// `packs/<identity>.d2fz`.
    pub pack: String,
    pub frames: u32,
    /// Candidates the quality gate rejected (they keep source frames);
    /// `None` for a pack kept from an earlier run.
    #[serde(default)]
    pub rejected: Option<u32>,
    pub bytes: u64,
}

impl PackEntry {
    /// The folded file key ([`FrameKey::file_key`]) of the entry.
    pub fn file_key(&self) -> FrameKey {
        let c = |s: &str| fold_code(&code(s.as_bytes()));
        FrameKey {
            kind: self.kind,
            token: c(&self.token),
            mode: c(&self.mode),
            component: self.component,
            component_token: c(&self.component_token),
            armor_class: c(&self.armor_class),
            weapon_class: c(&self.weapon_class),
            direction: 0,
            frame: 0,
            sub_frame: 0,
        }
    }

    pub fn format(&self) -> FileFormat {
        if self.source.to_ascii_lowercase().ends_with(".dc6") {
            FileFormat::Dc6
        } else {
            FileFormat::Dcc
        }
    }
}

/// The direction keys in the client's 64-direction space that a file with
/// `count` facings presents.
pub fn source_facings(count: u8) -> Vec<u8> {
    let count = count.max(1);
    if 64 % count == 0 {
        (0..count).map(|i| i * (64 / count)).collect()
    } else {
        vec![0]
    }
}

/// The 64-space source facing a file of `count` facings draws for `dir64`
/// (the [`source_facings`] entry of its file direction).
pub fn source_facing(count: u8, dir64: u8) -> u8 {
    let facings = source_facings(count);
    if facings.len() == 1 {
        return 0;
    }
    crate::rules::unit_composite::cof_direction(count, dir64)
        .ok()
        .and_then(|k| facings.get(usize::from(k)).copied())
        .unwrap_or(0)
}

/// One component file to bake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItem {
    pub path: String,
    pub format: FileFormat,
    pub key: FrameKey,
}

/// The player and monster component files among `names`, parsed from their
/// paths `data\global\{chars,monsters}\<T>\<C>\<T><C><armor><mode><weapon>`.
/// Files of denied animations, unknown components or modes, and names that
/// do not parse are left out (they keep their source frames).
pub fn work_items<'a>(
    names: impl IntoIterator<Item = &'a str>,
    looks: &UnitLooks,
    deny: &AnimationDeny,
) -> Vec<WorkItem> {
    let components: Vec<Code> = looks.components.iter().map(fold_code).collect();
    let modes = |kind: CompositeKind| -> Vec<Code> {
        match kind {
            CompositeKind::Player => &looks.player_modes,
            _ => &looks.monster_modes,
        }
        .iter()
        .map(fold_code)
        .filter(|m| m[0] != b' ')
        .collect()
    };
    let (player_modes, monster_modes) =
        (modes(CompositeKind::Player), modes(CompositeKind::Monster));
    let mut out = BTreeMap::new();
    for name in names {
        let lower = name.replace('/', "\\").to_ascii_lowercase();
        let parts: Vec<&str> = lower.split('\\').collect();
        let [data, global, root, token, component, file] = parts.as_slice() else {
            continue;
        };
        if *data != "data" || *global != "global" {
            continue;
        }
        let (kind, kind_code, modes) = match *root {
            "chars" => (CompositeKind::Player, 0u8, &player_modes),
            "monsters" => (CompositeKind::Monster, 1u8, &monster_modes),
            _ => continue,
        };
        let (stem, format) = if let Some(s) = file.strip_suffix(".dcc") {
            (s, FileFormat::Dcc)
        } else if let Some(s) = file.strip_suffix(".dc6") {
            (s, FileFormat::Dc6)
        } else {
            continue;
        };
        let Some(rest) = stem
            .strip_prefix(token)
            .and_then(|r| r.strip_prefix(component))
        else {
            continue;
        };
        let Some(c) = components
            .iter()
            .position(|c| c == &fold_code(&code(component.as_bytes())))
        else {
            continue;
        };
        // armor (1-3) + mode (a known token) + weapon class (1-3).
        let split = (1..=3usize).rev().find_map(|a| {
            let (armor, tail) = (rest.get(..a)?, rest.get(a..)?);
            modes.iter().find_map(|m| {
                let m_text = std::str::from_utf8(crate::rules::unit_composite::part(m)).ok()?;
                let weapon = tail.strip_prefix(m_text)?;
                (!weapon.is_empty() && weapon.len() <= 3).then_some((armor, *m, weapon))
            })
        });
        let Some((armor, mode, weapon)) = split else {
            continue;
        };
        let token_code = fold_code(&code(token.as_bytes()));
        if deny.denies(kind, &token_code, &mode) {
            continue;
        }
        let key = FrameKey {
            kind: kind_code,
            token: token_code,
            mode,
            component: c as u8,
            component_token: fold_code(&code(component.as_bytes())),
            armor_class: fold_code(&code(armor.as_bytes())),
            weapon_class: fold_code(&code(weapon.as_bytes())),
            direction: 0,
            frame: 0,
            sub_frame: 0,
        };
        if key.validate().is_err() {
            continue;
        }
        // One file per key: a `.dcc` wins over a `.dc6` of the same name.
        out.entry(key.clone())
            .and_modify(|w: &mut WorkItem| {
                if format == FileFormat::Dcc {
                    w.format = format;
                    w.path = name.to_string();
                }
            })
            .or_insert(WorkItem {
                path: name.to_string(),
                format,
                key,
            });
    }
    out.into_values().collect()
}

/// Where the bake reads the game data.
#[derive(Debug, Clone)]
pub enum BakeData {
    /// The player's archives.
    Archives(PathBuf),
    /// A converted native folder: `base/` and the mod layers `mods`, in order.
    Native { dir: PathBuf, mods: Vec<String> },
}

/// Files under `dir`, `(path relative to dir, size, modified seconds)`.
fn walk_stats(dir: &Path, out: &mut Vec<(String, u64, u64)>) {
    let mut todo = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, prefix)) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let rel = format!("{prefix}{}", e.file_name().to_string_lossy());
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                todo.push((e.path(), format!("{rel}/")));
            } else {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                out.push((rel, meta.len(), modified));
            }
        }
    }
}

/// The stamp of the source data: archive names, sizes and modification
/// times; for a native folder its `manifest.toml` and every mod layer file's
/// size and time. Cheap to compute; changed data makes a finished bake rerun
/// (packs whose pictures did not change are kept).
pub fn data_stamp(data: &BakeData) -> Result<String, String> {
    let mut h = Sha256::new();
    match data {
        BakeData::Native { dir, mods } => {
            let manifest = std::fs::read(dir.join("manifest.toml"))
                .map_err(|e| format!("{}: {e}", dir.join("manifest.toml").display()))?;
            h.update(&manifest);
            let source = d2_native::source::NativeSource::open_with(dir, Some(mods))
                .map_err(|e| e.to_string())?;
            for layer in source.layers.layers.iter().skip(1) {
                let mut files = Vec::new();
                walk_stats(&layer.dir, &mut files);
                files.sort();
                h.update(format!("layer {}\n", layer.name).as_bytes());
                for (name, len, modified) in files {
                    h.update(format!("{name}\t{len}\t{modified}\n").as_bytes());
                }
            }
        }
        BakeData::Archives(dir) => {
            let mut files: Vec<(String, u64, u64)> = std::fs::read_dir(dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                    if !name.ends_with(".mpq") {
                        return None;
                    }
                    let meta = e.metadata().ok()?;
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_secs());
                    Some((name, meta.len(), modified))
                })
                .collect();
            files.sort();
            for (name, len, modified) in files {
                h.update(format!("{name}\t{len}\t{modified}\n").as_bytes());
            }
        }
    }
    Ok(format!("{:x}", h.finalize()))
}

/// The bake options of `bake-anims`.
#[derive(Debug, Clone)]
pub struct BakeOptions {
    pub data: BakeData,
    pub out: PathBuf,
    pub jobs: usize,
    /// Only count the work and print the plan.
    pub plan: bool,
    /// Bake at most this many items (sampling for measurements); no index.
    pub limit: Option<usize>,
    /// Earlier bake folders (a mod's base game): a pack of the same identity
    /// there is copied, not generated again.
    pub reuse: Vec<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<BakeOptions, String> {
    let mut game_dir = None;
    let mut native = None;
    let mut mods = Vec::new();
    let mut reuse = Vec::new();
    let mut out = None;
    let mut jobs = None;
    let mut plan = false;
    let mut limit = None;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("missing value for {flag}"));
        match flag.as_str() {
            "--game-dir" => game_dir = Some(PathBuf::from(value()?)),
            "--native" => native = Some(PathBuf::from(value()?)),
            "--mod" => {
                let name = value()?;
                if name.is_empty()
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err("--mod needs a single mod folder name".into());
                }
                mods.push(name.clone());
            }
            "--reuse" => reuse.push(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--jobs" => {
                jobs = Some(
                    value()?
                        .parse::<usize>()
                        .ok()
                        .filter(|&j| j > 0)
                        .ok_or("--jobs must be positive")?,
                )
            }
            "--limit" => {
                limit = Some(
                    value()?
                        .parse::<usize>()
                        .map_err(|_| "--limit must be a number")?,
                )
            }
            "--plan" => plan = true,
            _ => return Err(format!("unknown bake-anims option {flag}")),
        }
    }
    let data = match native {
        Some(dir) => {
            if game_dir.is_some() {
                return Err("choose --native or --game-dir".into());
            }
            BakeData::Native { dir, mods }
        }
        None if !mods.is_empty() => {
            return Err("--mod NAME requires --native DIR (converted base and mods)".into())
        }
        None => BakeData::Archives(
            game_dir
                .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from))
                .ok_or("--game-dir or --native is required")?,
        ),
    };
    let jobs = jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map_or(1, |n| n.get().saturating_sub(1))
            .max(1)
    });
    Ok(BakeOptions {
        data,
        out: out.unwrap_or_else(super::animation_baked::dir),
        jobs,
        plan,
        limit,
        reuse,
    })
}

/// `bake-anims`: prints `bake-anims: <event>` lines for the launcher
/// (`total N`, `progress DONE TOTAL`, `done FILES BYTES`, `error MESSAGE`).
pub fn run_cli(args: &[String]) -> Result<(), String> {
    let result = parse_args(args).and_then(|o| bake(&o));
    if let Err(error) = &result {
        line(&format!("error {}", error.replace('\n', " ")));
    }
    result.map(|_| ())
}

fn line(event: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "bake-anims: {event}");
    let _ = out.flush();
}

/// The baked palette: act 1's presented palette. Unit pictures are palette
/// indices drawn with the act palette in use; the generator only needs one
/// palette to choose in-between indices.
pub fn bake_palette(source: &dyn FileSource) -> Result<Palette, String> {
    let palettes = ActPalettes::live(source)?;
    crate::scene::present_palette(palettes.of(0)).map_err(|e| e.to_string())
}

pub fn palette_hash(palette: &Palette) -> String {
    let mut h = Sha256::new();
    for c in palette.colors {
        h.update([c.r, c.g, c.b]);
    }
    format!("{:x}", h.finalize())
}

/// What a bake did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BakeSummary {
    pub items: usize,
    pub packs: usize,
    pub frames: u64,
    pub bytes: u64,
    /// The bake was already complete for this data.
    pub up_to_date: bool,
}

pub fn bake(o: &BakeOptions) -> Result<BakeSummary, String> {
    let start = Instant::now();
    let stamp = data_stamp(&o.data)?;
    let mod_id = match &o.data {
        BakeData::Native { mods, .. } if !mods.is_empty() => Some(mods.join("+")),
        _ => None,
    };
    let index_path = o.out.join(INDEX);
    if !o.plan && o.limit.is_none() {
        if let Some(index) = read_index(&index_path) {
            if index.version == BAKE_VERSION
                && index.game_version == crate::launch::VERSION
                && index.generator == GENERATOR_SETTINGS
                && index.stamp == stamp
                && index.mod_id == mod_id
            {
                let bytes = index.packs.iter().map(|p| p.bytes).sum();
                let frames = index.packs.iter().map(|p| u64::from(p.frames)).sum();
                line(&format!("total {}", index.packs.len()));
                line(&format!("progress {0} {0}", index.packs.len()));
                line(&format!("done {} {bytes}", index.packs.len()));
                return Ok(BakeSummary {
                    items: index.packs.len(),
                    packs: index.packs.len(),
                    frames,
                    bytes,
                    up_to_date: true,
                });
            }
        }
    }
    let (source, names) = open_data(&o.data)?;
    let looks = UnitLooks::live(source.as_ref())?;
    let deny = AnimationDeny::data();
    let mut items = work_items(names.iter().map(String::as_str), &looks, &deny);
    let palette = bake_palette(source.as_ref())?;
    let ramps = ramps_from_source(source.as_ref())?;
    if o.plan {
        plan(source.as_ref(), &items, &palette)?;
        return Ok(BakeSummary {
            items: items.len(),
            ..Default::default()
        });
    }
    if let Some(limit) = o.limit {
        // A spread sample across players and monsters for measurements.
        let step = (items.len() / limit.max(1)).max(1);
        items = items.into_iter().step_by(step).take(limit).collect();
    }
    let packs_dir = o.out.join(PACKS);
    std::fs::create_dir_all(&packs_dir).map_err(|e| format!("{}: {e}", packs_dir.display()))?;
    // Temporary files of a bake that was stopped.
    for entry in std::fs::read_dir(&packs_dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        if entry.file_name().to_string_lossy().ends_with(".partial") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    // A stale index must not be read while packs change under it.
    if index_path.exists() {
        std::fs::remove_file(&index_path).map_err(|e| e.to_string())?;
    }
    let generator = Generator::new(&palette, &ramps);
    let tables = generator.tables();
    let lut_hash = format!("{:x}", Sha256::digest(tables.bytes()));
    write_atomic(&o.out.join(LUT_FILE), tables.bytes())?;
    let total = items.len();
    line(&format!("total {total}"));
    line(&format!("progress 0 {total}"));
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let entries = Mutex::new(Vec::new());
    let errors = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..o.jobs.min(total.max(1)) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else {
                    break;
                };
                match bake_item(
                    source.as_ref(),
                    item,
                    &generator,
                    &tables,
                    &packs_dir,
                    &o.reuse,
                ) {
                    Ok(Some(entry)) => entries
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(entry),
                    Ok(None) => {}
                    Err(error) => errors
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(format!("{}: {error}", item.path)),
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                line(&format!("progress {n} {total}"));
            });
        }
    });
    let mut packs = entries.into_inner().unwrap_or_else(|e| e.into_inner());
    packs.sort_by_key(PackEntry::file_key);
    let errors = errors.into_inner().unwrap_or_else(|e| e.into_inner());
    for error in errors.iter().take(20) {
        crate::say_err!("bake-anims: skipped {error}");
    }
    let frames = packs.iter().map(|p| u64::from(p.frames)).sum();
    let bytes = packs.iter().map(|p| p.bytes).sum();
    let summary = BakeSummary {
        items: total,
        packs: packs.len(),
        frames,
        bytes,
        up_to_date: false,
    };
    if o.limit.is_none() {
        let index = BakeIndex {
            version: BAKE_VERSION,
            game_version: crate::launch::VERSION.to_string(),
            generator: GENERATOR_SETTINGS.to_string(),
            stamp,
            mod_id,
            palette: palette_hash(&palette),
            blend_lut: lut_hash,
            packs,
        };
        let json = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
        write_atomic(&index_path, &json)?;
    }
    crate::say!(
        "bake-anims: {} files, {} packs, {} frames, {} bytes, {} skipped, {:.1}s",
        total,
        summary.packs,
        frames,
        bytes,
        errors.len(),
        start.elapsed().as_secs_f64()
    );
    line(&format!("done {} {bytes}", summary.packs));
    Ok(summary)
}

/// The source files and the unit component file names of `data`.
fn open_data(data: &BakeData) -> Result<(Arc<dyn FileSource>, Vec<String>), String> {
    match data {
        BakeData::Archives(dir) => {
            let set =
                Arc::new(ArchiveSet::open_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?);
            let names = d2_formats::mpq::names::known_names(set.archives())
                .map_err(|e| e.to_string())?
                .into_values()
                .collect();
            Ok((set, names))
        }
        BakeData::Native { dir, mods } => {
            let files = crate::assets::game_files::GameFiles::native_with(dir, Some(mods))?;
            let source = d2_native::source::NativeSource::open_with(dir, Some(mods))
                .map_err(|e| e.to_string())?;
            // A sprite's native file is `<path>.toml` in the layer that owns it.
            let mut names = std::collections::BTreeSet::new();
            for layer in &source.layers.layers {
                for root in ["data/global/chars", "data/global/monsters"] {
                    let mut found = Vec::new();
                    walk_stats(&layer.dir.join(root), &mut found);
                    for (rel, _, _) in found {
                        let lower = rel.to_ascii_lowercase();
                        if lower.ends_with(".dcc.toml") || lower.ends_with(".dc6.toml") {
                            names.insert(format!("{root}/{}", &rel[..rel.len() - 5]));
                        }
                    }
                }
            }
            Ok((files.source(), names.into_iter().collect()))
        }
    }
}

/// Generates one component file's pack, or keeps the existing pack when its
/// identity still matches (an interrupted bake resumes here). Every file gets
/// a pack, an empty one included, so a rerun never generates it again; only
/// packs with frames are listed in the index.
pub(crate) fn bake_item(
    source: &dyn FileSource,
    item: &WorkItem,
    generator: &Generator<'_>,
    tables: &BlendTables,
    packs_dir: &Path,
    reuse: &[PathBuf],
) -> Result<Option<PackEntry>, String> {
    let Some(input) = load_component_file(source, &item.path, item.format, item.key.clone())?
    else {
        return Ok(None);
    };
    let identity = hash_inputs(
        std::slice::from_ref(&input),
        generator.palette(),
        generator.ramps(),
    );
    let name: String = identity.iter().map(|b| format!("{b:02x}")).collect();
    let pack = format!("{name}.{PACK_EXTENSION}");
    let file = packs_dir.join(&pack);
    let earlier = std::iter::once(file.clone())
        .chain(reuse.iter().map(|dir| dir.join(PACKS).join(&pack)))
        .find_map(|path| {
            let existing = read_baked_pack(&path).ok()?;
            (existing.input_hash == identity).then_some((path, existing.accepted()))
        });
    let (frames, rejected) = match earlier {
        Some((path, frames)) => {
            if path != file {
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                write_atomic(&file, &bytes)?;
            }
            (frames, None)
        }
        None => {
            let result = generate_packed(&input, generator, tables, |_| {})?;
            let packed = BakedPack::from_pairs(identity, input.looping, &result.pairs)
                .map_err(|e| e.to_string())?;
            let bytes = encode_baked_pack(&packed)?;
            write_atomic(&file, &bytes)?;
            let candidates = (result.pairs.len() * 3) as u32;
            (
                result.accepted,
                Some(candidates.saturating_sub(result.accepted)),
            )
        }
    };
    if frames == 0 {
        return Ok(None);
    }
    let bytes = std::fs::metadata(&file).map_err(|e| e.to_string())?.len();
    let text =
        |c: &Code| String::from_utf8_lossy(crate::rules::unit_composite::part(c)).into_owned();
    let k = &item.key;
    Ok(Some(PackEntry {
        source: item.path.clone(),
        kind: k.kind,
        token: text(&k.token),
        mode: text(&k.mode),
        component: k.component,
        component_token: text(&k.component_token),
        armor_class: text(&k.armor_class),
        weapon_class: text(&k.weapon_class),
        pack: format!("{PACKS}/{pack}"),
        frames,
        rejected,
        bytes,
    }))
}

/// Counts the bake's work: files, facings, source frames and pixels, and how
/// far the five act palettes differ.
fn plan(source: &dyn FileSource, items: &[WorkItem], palette: &Palette) -> Result<(), String> {
    let palettes = ActPalettes::live(source)?;
    for a in 1..5u8 {
        let other = crate::scene::present_palette(palettes.of(a)).map_err(|e| e.to_string())?;
        let differ = palette
            .colors
            .iter()
            .zip(other.colors.iter())
            .filter(|(x, y)| x != y)
            .count();
        crate::say!(
            "plan: act {} palette differs from act 1 in {differ} entries",
            a + 1
        );
    }
    let mut by_kind = BTreeMap::<u8, (usize, u64, u64, u64)>::new();
    let start = Instant::now();
    for item in items {
        let Some(input) = load_component_file(source, &item.path, item.format, item.key.clone())?
        else {
            continue;
        };
        let e = by_kind.entry(item.key.kind).or_default();
        e.0 += 1;
        e.1 += input.directions.len() as u64;
        for frames in input.directions.values() {
            e.2 += frames.len() as u64;
            e.3 += frames.iter().map(|f| f.pixels.len() as u64).sum::<u64>();
        }
    }
    for (kind, (files, facings, frames, pixels)) in by_kind {
        crate::say!(
            "plan: kind {kind}: {files} files, {facings} facings, {frames} source frames, {pixels} source pixels; generated ~{} frames, ~{} MiB raw",
            frames * 3,
            pixels * 3 / (1024 * 1024)
        );
    }
    crate::say!(
        "plan: {} work items, decoded in {:.1}s",
        items.len(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

pub fn read_index(path: &Path) -> Option<BakeIndex> {
    let bytes = super::generation_job::read_bounded(path, 64 * 1024 * 1024).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);
    let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("{}.{nonce}.partial", std::process::id()));
    std::fs::write(&temporary, bytes).map_err(|e| format!("{}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn looks() -> UnitLooks {
        let c = |s: &str| code(s.as_bytes());
        UnitLooks {
            components: ["HD", "TR", "LG", "RA", "LA", "RH", "LH", "SH", "S1"]
                .into_iter()
                .map(c)
                .collect(),
            player_modes: ["DT", "NU", "WL", "RN", "A1"].into_iter().map(c).collect(),
            monster_modes: ["DT", "NU", "WL", "GH", "A1"].into_iter().map(c).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn component_files_parse_into_folded_keys() {
        let names = [
            r"data\global\CHARS\BA\HD\BAHDLITA1HTH.DCC",
            r"data\global\monsters\zm\tr\zmtrlitwl1hs.dcc",
            r"data\global\monsters\zm\tr\zmtrlitwl1hs.dc6",
            r"data\global\chars\ba\cof\baa1hth.cof",
            r"data\global\chars\ba\xx\baxxlita1hth.dcc",
            r"data\global\objects\ob\tr\obtrlitonhth.dcc",
            r"data\global\chars\ba\hd\bahdlitzzhth.dcc",
        ];
        let items = work_items(names, &looks(), &AnimationDeny::default());
        assert_eq!(items.len(), 2, "{items:?}");
        let ba = &items[0];
        assert_eq!(ba.key.kind, 0);
        assert_eq!(&ba.key.token, b"ba  ");
        assert_eq!(&ba.key.mode, b"a1  ");
        assert_eq!(ba.key.component, 0);
        assert_eq!(&ba.key.armor_class, b"lit ");
        assert_eq!(&ba.key.weapon_class, b"hth ");
        let zm = &items[1];
        assert_eq!((zm.key.kind, zm.key.component), (1, 1));
        assert_eq!(&zm.key.mode, b"wl  ");
        assert_eq!(&zm.key.weapon_class, b"1hs ");
        assert_eq!(zm.format, FileFormat::Dcc);
    }

    #[test]
    fn denied_animations_are_not_baked() {
        let deny =
            AnimationDeny::parse("kind\tunit\tmode\treason\nmonster\tzm\twl\ttest\n").unwrap();
        let items = work_items(
            [r"data\global\monsters\zm\tr\zmtrlitwl1hs.dcc"],
            &looks(),
            &deny,
        );
        assert!(items.is_empty());
    }

    #[test]
    fn index_entries_round_trip_their_file_key() {
        let entry = PackEntry {
            source: r"data\global\chars\ba\hd\bahdlita1hth.dcc".into(),
            kind: 0,
            token: "BA".into(),
            mode: "A1".into(),
            component: 0,
            component_token: "hd".into(),
            armor_class: "lit".into(),
            weapon_class: "hth".into(),
            pack: "packs/x.d2frames".into(),
            frames: 3,
            rejected: Some(1),
            bytes: 10,
        };
        let key = entry.file_key();
        assert_eq!(&key.token, b"ba  ");
        assert_eq!(&key.component_token, b"hd  ");
        assert_eq!(entry.format(), FileFormat::Dcc);
    }

    #[test]
    fn source_facings_cover_every_file_direction() {
        assert_eq!(source_facings(0), vec![0]);
        assert_eq!(source_facings(1), vec![0]);
        assert_eq!(source_facings(4), vec![0, 16, 32, 48]);
        assert_eq!(source_facings(8), (0..8).map(|i| i * 8).collect::<Vec<_>>());
        assert_eq!(
            source_facings(16),
            (0..16).map(|i| i * 4).collect::<Vec<_>>()
        );
        assert_eq!(source_facings(3), vec![0]);
    }
}
