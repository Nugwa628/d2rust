//! The automap of a running game against the client model: the act load opens
//! the character's `.map` / `.ma<k>` files (the save sub-directory first; the
//! plain save directory when the sub-directory file cannot be opened), each
//! frame runs the reveal on the world view's near rooms, the toggle keys flip
//! UI state 0x0A and the teardown saves the current layer and writes the data
//! file. Plain Rust: the world view calls it (no game logic in Bevy systems).

use std::path::{Path, PathBuf};

use super::persist::{open_files, MaFile};
use super::place::{AutomapUnit, UnitCels};
use super::view::FrameFacts;
use super::{Automap, AutomapError, AutomapLevels, AutomapTables, CelPicker, RevealRooms};
use crate::bridge::world::{ClientWorld, UnitKey, ITEM, OBJECT, PLAYER, TILE};
use crate::rules::camera::{moving_to_client, static_to_client, ClientPos};
use crate::rules::draw_order::NearRooms;

/// Where the character's automap files live: the save directory, the save
/// sub-directory (if any) and the character name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveFiles {
    pub dir: PathBuf,
    pub sub: Option<String>,
    pub name: String,
}

/// On disk with the sub-directory rule: `<dir>\<sub>\<name>.map` when
/// the sub-directory can be opened, else `<dir>\<name>.map`. Returns the
/// data file, its path and its slot.
///
/// Reading: "cannot be opened" is the sub-directory not
/// existing as a directory; an I/O error inside an existing
/// sub-directory is an error, not a fallback.
pub fn open_files_in(
    files: &SaveFiles,
    key: u32,
) -> Result<(MaFile, PathBuf, usize), AutomapError> {
    if let Some(sub) = files.sub.as_deref().filter(|s| !s.is_empty()) {
        let d = files.dir.join(sub);
        if d.is_dir() {
            return open_files(&d, &files.name, key);
        }
    }
    open_files(&files.dir, &files.name, key)
}

/// The tables the session hands to every automap call.
pub struct AutomapSource {
    pub picker: CelPicker,
    pub levels: Box<dyn AutomapLevels + Send + Sync>,
    pub cels: Box<dyn UnitCels + Send + Sync>,
}

/// One game's automap and its files.
pub struct AutomapSession {
    pub map: Automap,
    pub source: AutomapSource,
    /// `None`: no save files (the cells live in memory only).
    pub files: Option<SaveFiles>,
    /// The open data file's path (written at teardown).
    data_path: Option<PathBuf>,
    /// The act record the files were opened for (S→C 0x03 u32@2, u32@8).
    act: Option<(u32, u32)>,
    /// UI state 0x0A (automap shown).
    pub open: bool,
    /// The client DRLG (act, init seed) whose town-art calls were taken,
    /// and how many ([`Self::drlg_towns`]).
    towns_taken: (Option<(u8, u32)>, usize),
    /// `ClientWorld::placed_units` at the last frame.
    placed_seen: u32,
}

impl AutomapSession {
    /// The automap's UI init with its tables and files.
    pub fn new(map: Automap, source: AutomapSource, files: Option<SaveFiles>) -> Self {
        AutomapSession {
            map,
            source,
            files,
            data_path: None,
            act: None,
            open: false,
            towns_taken: (None, 0),
            placed_seen: 0,
        }
    }

    /// The act load (S→C 0x03): a new act record
    /// tears the old one down (its layer saved and written) and
    /// opens the index for key `init_seed`, the data file then
    /// attached with the act record's second u32 `f8`.
    pub fn act_load(&mut self, init_seed: u32, f8: u32) -> Result<(), AutomapError> {
        if self.act == Some((init_seed, f8)) {
            return Ok(());
        }
        if self.act.is_some() {
            // The old act's file not written loses its cells, not the game.
            if let Err(e) = self.teardown() {
                bevy::log::warn!("automap: the last act's file was not saved: {e}");
            }
        }
        self.act = Some((init_seed, f8));
        // Files that cannot be opened or written (a save directory that is
        // missing, a read-only one) leave this act's automap in memory
        // only: an empty automap, never a stopped game.
        let file = match &self.files {
            Some(files) => match open_files_in(files, init_seed) {
                Ok((file, path, _)) => {
                    self.data_path = Some(path);
                    file
                }
                Err(e @ AutomapError::File(_)) => {
                    bevy::log::warn!("automap: kept in memory only: {e}");
                    self.data_path = None;
                    MaFile::default()
                }
                Err(e) => return Err(e),
            },
            None => MaFile::default(),
        };
        self.map.attach_file(file, f8);
        Ok(())
    }

    /// The act load of the model's current act record, if any (S→C 0x03 runs
    /// before the client DRLG it starts, so the file is open, before the DRLG
    /// callbacks switch layers, and their switch loads the layer's stored
    /// cells).
    pub fn act(&mut self, world: &ClientWorld) -> Result<(), AutomapError> {
        match world.act {
            Some(act) => self.act_load(act.init_seed, act.f8),
            None => Ok(()),
        }
    }

    /// One frame (from the frame): the act load when the model's act record
    /// changed, the countdown reset when a unit was placed since the last
    /// frame, then the reveal on the near rooms of the local player's room. No
    /// act: nothing.
    pub fn frame(
        &mut self,
        world: &ClientWorld,
        near: Option<&mut NearRooms>,
    ) -> Result<(), AutomapError> {
        let Some(act) = world.act else {
            return Ok(());
        };
        self.act_load(act.init_seed, act.f8)?;
        // A unit placed since the last frame resets the countdown.
        if world.placed_units != self.placed_seen {
            self.placed_seen = world.placed_units;
            self.map.placed();
        }
        let player = world.local().map(|p| unit_pos(PLAYER, p.cell()));
        let mut units: Vec<Vec<AutomapUnit>>;
        let rooms = match near {
            Some(near) => {
                units = near
                    .rooms
                    .iter()
                    .map(|r| {
                        r.units
                            .iter()
                            .map(|u| automap_unit(world, u.key, u.facts.flags))
                            .collect()
                    })
                    .collect();
                Some(RevealRooms {
                    near,
                    units: &mut units,
                })
            }
            None => None,
        };
        let AutomapSession { map, source, .. } = self;
        let t = AutomapTables {
            picker: &source.picker,
            levels: source.levels.as_ref(),
            cels: source.cels.as_ref(),
        };
        map.reveal_frame(player, rooms, &t)
    }

    /// Tab / the middle button / the mini-panel button toggle UI
    /// state 0x0A; closing re-centres with force 0.
    pub fn toggle(&mut self, f: &FrameFacts) {
        self.open = !self.open;
        self.map.toggled(self.open, f);
    }

    /// The current layer saved, cells and layers
    /// freed, the data file written to its path.
    pub fn teardown(&mut self) -> Result<(), AutomapError> {
        let file = self.map.teardown()?;
        if let (Some(file), Some(path)) = (file, self.data_path.take()) {
            write(&file, &path)?;
        }
        self.act = None;
        Ok(())
    }

    /// The whole-preset-level DRLG callback (+0x454) for a room
    /// the caller built.
    pub fn preset_room(
        &mut self,
        room: &mut crate::rules::draw_order::Room,
        units: &mut [AutomapUnit],
    ) -> Result<(), AutomapError> {
        let AutomapSession { map, source, .. } = self;
        let t = AutomapTables {
            picker: &source.picker,
            levels: source.levels.as_ref(),
            cels: source.cels.as_ref(),
        };
        map.preset_room(room, units, &t)
    }

    /// The client DRLG's town-art callback calls (+0x488,
    /// `Drlg::automap_towns`) not taken yet, in call order.
    pub fn drlg_towns(&mut self, world: &ClientWorld) -> Result<(), AutomapError> {
        let Some(cd) = world.drlg.as_ref() else {
            return Ok(());
        };
        let key = Some((cd.drlg.act, cd.drlg.init_seed));
        if self.towns_taken.0 != key {
            self.towns_taken = (key, 0);
        }
        let calls = &cd.drlg.automap_towns;
        let new = calls.get(self.towns_taken.1..).unwrap_or(&[]).to_vec();
        self.towns_taken.1 = calls.len();
        for (level, f, centre) in new {
            self.town_art(level, f, centre)?;
        }
        Ok(())
    }

    /// The town-art DRLG callback (+0x488).
    pub fn town_art(&mut self, level: u32, f: u32, centre: (i32, i32)) -> Result<(), AutomapError> {
        let AutomapSession { map, source, .. } = self;
        let t = AutomapTables {
            picker: &source.picker,
            levels: source.levels.as_ref(),
            cels: source.cels.as_ref(),
        };
        map.town_art(level, f, centre, &t)
    }

    /// The data file's path once an act opened it.
    pub fn data_path(&self) -> Option<&Path> {
        self.data_path.as_deref()
    }
}

fn write(file: &MaFile, path: &Path) -> Result<(), AutomapError> {
    file.write_to(path)
        .map_err(|e| AutomapError::File(format!("{}: {e}", path.display())))
}

/// A unit's client position: types
/// 2, 4, 5 project their static path point, the
/// others their model cell's centre.
pub(super) fn unit_pos(unit_type: u8, (x, y): (u16, u16)) -> ClientPos {
    if matches!(unit_type, OBJECT | ITEM | TILE) {
        static_to_client(i32::from(x), i32::from(y))
    } else {
        moving_to_client((u32::from(x) << 16) | 0x8000, (u32::from(y) << 16) | 0x8000)
    }
}

/// What reads of a room unit: type, class and mode from the model,
/// flags from the frame's unit facts.
fn automap_unit(world: &ClientWorld, key: UnitKey, flags: u32) -> AutomapUnit {
    let u = world.units.get(&key);
    AutomapUnit {
        unit_type: key.unit_type,
        class: u.map_or(0, |u| u.class),
        mode: u.map_or(0, |u| u.mode),
        flags,
        pos: u.map_or(ClientPos::default(), |u| unit_pos(key.unit_type, u.cell())),
    }
}
