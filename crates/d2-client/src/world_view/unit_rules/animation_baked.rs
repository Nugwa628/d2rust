//! Play-time loading of baked animation frames ([`super::animation_bake`]).
//! The game never generates frames while it runs: a loader thread reads the
//! baked pack of each component file a drawn player or monster uses, checks
//! it against the source file, and the view installs it. Missing or stale
//! packs play the source frames; that is logged once.
//!
//! A unit's look (its COF with its equipment) is installed as a whole: the
//! packs of all its component files are held until every one of them is
//! answered, then installed in the same frame. Installed one by one, the
//! unit would draw generated in-between pictures for some components and
//! source cels for the others while its packs arrive, which shows as a
//! flicker of the body for the first moments of each new animation (owner
//! report: a fresh character flickers for about a second after it first
//! moves).

use super::animation_bake::{read_index, BakeIndex, BAKE_VERSION, INDEX};
use super::animation_inputs::load_component_file;
use super::animation_pack::{BakedPack, LUT_BYTES, LUT_FILE};
use super::frame_pack::{FrameKey, FramePack};
use super::generation_job::{hash_inputs, read_bounded, GENERATOR_SETTINGS};
use super::generator::{ramps_from_source, RampLabels};
use super::playback::{AnimationMode, SharedPackFrames};
use crate::assets::path::FileSource;
use crate::bridge::world::{ClientWorld, UnitKey, MONSTER, PLAYER};
use crate::rules::unit_composite::{CofPath, CompositeKind, FileFormat};
use crate::world_view::unit_assets::{component_codes, unit_cof_with, SharedUnitArt, UnitLooks};
use crate::world_view::ViewAssets;
use bevy::prelude::Resource;
use d2_formats::palette::Palette;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

/// The `--anim-dir` of this run.
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Sets the bake folder for this run (`play --anim-dir DIR`, a dev/test
/// override; the launcher sets `D2_ANIM_DIR` instead).
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// The bake folder: `--anim-dir`, else `$D2_ANIM_DIR`, else
/// `<user folder>/cache/animation/baked`. `bake-anims` writes the same
/// default when it has no `--out`.
pub fn dir() -> PathBuf {
    DIR.get()
        .cloned()
        .or_else(|| {
            crate::launch::env_os("D2_ANIM_DIR")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| crate::user_dir::root().join("cache/animation/baked"))
}

/// A pack the view should load: where it is and the source file it was made from.
#[derive(Debug, Clone)]
struct PackRef {
    key: FrameKey,
    pack: PathBuf,
    source: String,
    format: FileFormat,
}

enum Loaded {
    Pack(FrameKey, FramePack),
    /// The pack is missing, unreadable or made from other source pictures.
    Skipped(FrameKey, String),
}

/// Why no baked frames are in use, once known.
pub fn index_problem(index: Option<&BakeIndex>) -> Option<String> {
    let Some(index) = index else {
        return Some("no baked animation frames".into());
    };
    if index.version != BAKE_VERSION {
        return Some(format!(
            "bake version {} is not {BAKE_VERSION}",
            index.version
        ));
    }
    if index.generator != GENERATOR_SETTINGS {
        return Some("baked by another generator".into());
    }
    if index.blend_lut.is_empty() {
        return Some("bake.json has no blend.lut hash".into());
    }
    None
}

#[derive(Resource)]
pub struct BakedAnimations {
    pub frames: SharedPackFrames,
    looks: Arc<UnitLooks>,
    /// Folded file key -> pack, from `bake.json`.
    packs: BTreeMap<FrameKey, PackRef>,
    requested: BTreeSet<FrameKey>,
    /// Requests the loader has answered (loaded or skipped).
    answered: usize,
    /// Loaded packs waiting for the rest of a look's packs.
    ready: BTreeMap<FrameKey, FramePack>,
    /// Packs installed or skipped: nothing more comes for them.
    settled: BTreeSet<FrameKey>,
    /// The looks whose packs are not all answered yet: each a list of
    /// component file keys.
    looks_waiting: Vec<Vec<FrameKey>>,
    /// Unit appearances already walked: (COF path, hands, equipment).
    seen: BTreeSet<String>,
    jobs: Option<mpsc::Sender<PackRef>>,
    loaded: Mutex<mpsc::Receiver<Loaded>>,
    logged_skip: bool,
}

impl BakedAnimations {
    /// Reads `<dir>/bake.json` and starts the loader thread. Without a usable
    /// index nothing is loaded and the reason is logged once.
    pub fn new(
        source: Arc<dyn FileSource>,
        looks: Arc<UnitLooks>,
        frames: SharedPackFrames,
        dir: &Path,
    ) -> Self {
        let base = source.clone();
        Self::with_inputs(source, looks, frames, dir, move || {
            Ok((
                super::animation_bake::bake_palette(base.as_ref())?,
                ramps_from_source(base.as_ref())?,
            ))
        })
    }

    /// [`BakedAnimations::new`] with the palette and ramps the staleness
    /// check hashes (read on the loader thread, once).
    pub fn with_inputs(
        source: Arc<dyn FileSource>,
        looks: Arc<UnitLooks>,
        frames: SharedPackFrames,
        dir: &Path,
        inputs: impl FnOnce() -> Result<(Palette, RampLabels), String> + Send + 'static,
    ) -> Self {
        let index = read_index(&dir.join(INDEX));
        let lut_hash = index
            .as_ref()
            .filter(|i| index_problem(Some(i)).is_none())
            .map(|i| i.blend_lut.clone())
            .unwrap_or_default();
        let directory = dir.to_path_buf();
        let packs: BTreeMap<FrameKey, PackRef> = match index_problem(index.as_ref()) {
            Some(problem) => {
                bevy::log::info!(
                    "animation frames: {problem} in {}; source frames play (run bake-anims)",
                    dir.display()
                );
                BTreeMap::new()
            }
            None => index
                .into_iter()
                .flat_map(|i| i.packs)
                .map(|p| {
                    let key = p.file_key();
                    let r = PackRef {
                        key: key.clone(),
                        pack: dir.join(&p.pack),
                        format: p.format(),
                        source: p.source,
                    };
                    (key, r)
                })
                .collect(),
        };
        let (job_tx, job_rx) = mpsc::channel::<PackRef>();
        let (done_tx, done_rx) = mpsc::channel();
        let jobs = if packs.is_empty() {
            None
        } else {
            let initialize = move || {
                let (palette, ramps) = inputs()?;
                let lut = read_blend_lut(&directory, &lut_hash)?;
                Ok((palette, ramps, lut))
            };
            let spawned = std::thread::Builder::new()
                .name("animation-load".into())
                .spawn(move || load_worker(source, initialize, job_rx, done_tx));
            spawned.ok().map(|_| job_tx)
        };
        Self {
            frames,
            looks,
            packs,
            requested: BTreeSet::new(),
            answered: 0,
            ready: BTreeMap::new(),
            settled: BTreeSet::new(),
            looks_waiting: Vec::new(),
            seen: BTreeSet::new(),
            jobs,
            loaded: Mutex::new(done_rx),
            logged_skip: false,
        }
    }

    /// Baked packs listed in the index.
    pub fn listed(&self) -> usize {
        self.packs.len()
    }

    /// Packs asked for and not answered yet.
    pub fn waiting(&self) -> usize {
        self.requested.len() - self.answered
    }

    /// Takes in loaded packs and asks for the packs of drawn units the
    /// view has not asked for yet. No generation happens here.
    pub fn update(&mut self, world: &ClientWorld, art: &SharedUnitArt, assets: &mut ViewAssets) {
        let mut loaded = Vec::new();
        {
            let rx = self.loaded.lock().unwrap_or_else(|e| e.into_inner());
            while let Ok(item) = rx.try_recv() {
                loaded.push(item);
            }
        }
        for item in loaded {
            self.receive(item);
        }
        self.install_complete(assets);
        if self.jobs.is_none() || self.frames.mode() != AnimationMode::Smooth {
            return;
        }
        let art = art.read().unwrap_or_else(|e| e.into_inner());
        for key in candidates(world) {
            let Some(unit) = world.units.get(&key) else {
                continue;
            };
            let unit = art.posed(unit);
            let hands = art.hands_of(unit.key);
            let Some(name) = unit_cof_with(&self.looks, &unit, hands) else {
                continue;
            };
            if !matches!(name.kind, CompositeKind::Player | CompositeKind::Monster) {
                continue;
            }
            let Ok(path) = name.path() else {
                continue;
            };
            let equipment = art.equip_of(unit.key).copied();
            let identity = format!("{path}:{hands:?}:{equipment:?}:{:?}", unit.kind);
            if self.seen.contains(&identity) {
                continue;
            }
            let Some(cof) = art.cofs.get(&path) else {
                // The COF is not resident yet; look again next frame.
                continue;
            };
            self.seen.insert(identity);
            let mut look = Vec::new();
            for layer in &cof.layers {
                let Some(codes) =
                    component_codes(&self.looks, &unit, &name, layer, equipment.as_ref())
                else {
                    continue;
                };
                let key =
                    super::animation_inputs::component_key(&codes, layer.component).file_key();
                if self.requested.contains(&key) {
                    continue;
                }
                let Some(pack) = self.packs.get(&key) else {
                    continue;
                };
                look.push(key.clone());
                if !self.requested.insert(key) {
                    continue;
                }
                if let Some(jobs) = &self.jobs {
                    let _ = jobs.send(pack.clone());
                }
            }
            self.wait_for_look(look);
        }
    }

    /// Notes a look's component packs: they are installed together once
    /// all of them are answered ([`Self::install_complete`]).
    fn wait_for_look(&mut self, look: Vec<FrameKey>) {
        if look.iter().any(|k| !self.settled.contains(k)) {
            self.looks_waiting.push(look);
        }
    }

    /// Takes in one answer of the loader thread.
    fn receive(&mut self, item: Loaded) {
        self.answered += 1;
        match item {
            Loaded::Pack(key, pack) => {
                self.ready.insert(key, pack);
            }
            Loaded::Skipped(key, why) => {
                self.skip(&format!("{key:?}: {why}"));
                self.settled.insert(key);
            }
        }
    }

    /// Installs the packs of every look whose packs are all answered, in
    /// one go per look. A skipped pack leaves its component on the source
    /// pictures, as before; the look's other packs still install.
    fn install_complete(&mut self, assets: &mut ViewAssets) {
        let mut waiting = std::mem::take(&mut self.looks_waiting);
        waiting.retain(|look| {
            let answered = look
                .iter()
                .all(|k| self.settled.contains(k) || self.ready.contains_key(k));
            if !answered {
                return true;
            }
            for key in look {
                if let Some(pack) = self.ready.remove(key) {
                    if let Err(error) = self.frames.install(&pack, &mut assets.frames) {
                        self.skip(&format!("install: {error}"));
                    }
                }
                self.settled.insert(key.clone());
            }
            false
        });
        self.looks_waiting = waiting;
    }

    fn skip(&mut self, why: &str) {
        if !self.logged_skip {
            self.logged_skip = true;
            bevy::log::warn!("animation frames: a baked pack was not used ({why}); its source frames play. Rerun bake-anims if the game data changed.");
        }
    }
}

/// Units whose packs may load: the local player first, then other players
/// and monsters.
fn candidates(world: &ClientWorld) -> Vec<UnitKey> {
    let mut out = Vec::new();
    if let Some(key) = world.local_player {
        out.push(key);
    }
    for &key in world.units.keys() {
        if matches!(key.unit_type, PLAYER | MONSTER) && Some(key) != world.local_player {
            out.push(key);
        }
    }
    out
}

/// The loader thread: reads, checks and decodes packs one at a time.
fn load_worker(
    source: Arc<dyn FileSource>,
    inputs: impl FnOnce() -> Result<(Palette, RampLabels, Vec<u8>), String>,
    jobs: mpsc::Receiver<PackRef>,
    done: mpsc::Sender<Loaded>,
) {
    let mut inputs = Some(inputs);
    let mut base = None;
    for job in jobs {
        let base = base.get_or_insert_with(|| inputs.take().map_or(Err(String::new()), |f| f()));
        let result = match base {
            Ok((palette, ramps, lut)) => load_checked(source.as_ref(), &job, palette, ramps, lut),
            Err(e) => Err(e.clone()),
        };
        let message = match result {
            Ok(pack) => Loaded::Pack(job.key, pack),
            Err(why) => Loaded::Skipped(job.key, why),
        };
        if done.send(message).is_err() {
            return;
        }
    }
}

/// A baked pack whose identity matches the source pictures it was made from.
fn load_checked(
    source: &dyn FileSource,
    job: &PackRef,
    palette: &Palette,
    ramps: &RampLabels,
    lut: &[u8],
) -> Result<FramePack, String> {
    let pack = read_baked_pack(&job.pack)?;
    let input = load_component_file(source, &job.source, job.format, job.key.clone())?
        .ok_or("source file missing")?;
    if hash_inputs(std::slice::from_ref(&input), palette, ramps) != pack.input_hash {
        return Err("stale: made from other source pictures".into());
    }
    pack.decode_to_frames(&input, lut)
}

/// The bake folder's `blend.lut`, checked against the manifest's SHA-256.
pub fn read_blend_lut(dir: &Path, expected: &str) -> Result<Vec<u8>, String> {
    let path = dir.join(LUT_FILE);
    let bytes = read_bounded(&path, LUT_BYTES)?;
    if bytes.len() != LUT_BYTES {
        return Err(format!("{} is not {LUT_BYTES} bytes", path.display()));
    }
    if expected.is_empty() {
        return Err("bake.json has no blend.lut hash".into());
    }
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err("blend.lut does not match bake.json".into());
    }
    Ok(bytes)
}

/// A baked pack file: a deflate stream of one pack v2.
pub fn read_baked_pack(path: &Path) -> Result<BakedPack, String> {
    let compressed = read_bounded(path, super::frame_pack::MAX_BYTES)?;
    let mut bytes = Vec::new();
    flate2::read::DeflateDecoder::new(compressed.as_slice())
        .take(super::frame_pack::MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > super::frame_pack::MAX_BYTES {
        return Err("pack exceeds byte limit".into());
    }
    BakedPack::decode(&bytes).map_err(|e| e.to_string())
}

/// Encodes `pack` as a baked pack file's bytes ([`read_baked_pack`]).
pub fn encode_baked_pack(pack: &BakedPack) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let bytes = pack.encode().map_err(|e| e.to_string())?;
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&bytes).map_err(|e| e.to_string())?;
    z.finish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A component file's key (`component`) and a one-picture pack of it.
    fn pack(component: u8) -> (FrameKey, FramePack) {
        let file = FrameKey {
            kind: 0,
            token: *b"QP  ",
            mode: *b"TW  ",
            component,
            component_token: *b"QC  ",
            armor_class: *b"LIT ",
            weapon_class: *b"hth ",
            direction: 0,
            frame: 0,
            sub_frame: 0,
        };
        let mut frame = file.clone();
        frame.sub_frame = 64;
        let picture = crate::frames::IndexFrame::new(1, 1, 0, 0, vec![component + 1]).unwrap();
        let pack = FramePack {
            input_hash: [component; 32],
            frames: [(frame, picture)].into(),
        };
        (file, pack)
    }

    fn loader(frames: SharedPackFrames) -> BakedAnimations {
        let (_, jobs) = mpsc::channel::<Loaded>();
        BakedAnimations {
            frames,
            looks: Arc::new(UnitLooks::default()),
            packs: BTreeMap::new(),
            requested: BTreeSet::new(),
            answered: 0,
            ready: BTreeMap::new(),
            settled: BTreeSet::new(),
            looks_waiting: Vec::new(),
            seen: BTreeSet::new(),
            jobs: None,
            loaded: Mutex::new(jobs),
            logged_skip: false,
        }
    }

    /// A look's packs arrive one at a time from the loader thread; none
    /// is used before all of them are there, so the unit never draws
    /// generated pictures for some components and source cels for the
    /// others. A skipped pack does not hold the look's other packs back.
    #[test]
    fn a_look_switches_to_its_generated_pictures_all_at_once() {
        let frames = SharedPackFrames::default();
        frames.set_mode(AnimationMode::Smooth);
        let mut b = loader(frames.clone());
        let mut assets = ViewAssets::new(Palette {
            colors: [Default::default(); 256],
        });
        let (head, head_pack) = pack(0);
        let (torso, torso_pack) = pack(1);
        b.wait_for_look(vec![head.clone(), torso.clone()]);
        b.receive(Loaded::Pack(head.clone(), head_pack));
        b.install_complete(&mut assets);
        assert_eq!(frames.len(), 0, "half a look is not used");
        b.receive(Loaded::Pack(torso.clone(), torso_pack));
        b.install_complete(&mut assets);
        assert_eq!(frames.len(), 2, "the whole look at once");
        // A look sharing an installed pack waits only for its own.
        let (legs, legs_pack) = pack(2);
        b.wait_for_look(vec![head, legs.clone()]);
        b.install_complete(&mut assets);
        assert_eq!(frames.len(), 2);
        b.receive(Loaded::Pack(legs, legs_pack));
        b.install_complete(&mut assets);
        assert_eq!(frames.len(), 3);
        // A skipped pack: the rest of its look installs once answered.
        let (arms, _) = pack(3);
        let (hands, hands_pack) = pack(4);
        b.wait_for_look(vec![arms.clone(), hands.clone()]);
        b.receive(Loaded::Skipped(arms, "stale".into()));
        b.install_complete(&mut assets);
        assert_eq!(frames.len(), 3);
        b.receive(Loaded::Pack(hands, hands_pack));
        b.install_complete(&mut assets);
        assert_eq!(frames.len(), 4);
        assert!(b.looks_waiting.is_empty());
    }

    #[test]
    fn the_environment_variable_names_the_bake_folder() {
        // The launcher's source: `--anim-dir` (DIR) stays the dev/test
        // override, so this test runs before anything sets it.
        let key = "D2_ANIM_DIR";
        let previous = std::env::var_os(key);
        std::env::set_var(key, "env-bake");
        assert_eq!(dir(), PathBuf::from("env-bake"));
        match previous {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}
