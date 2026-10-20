//! The automap: client-only presentation. Per automap
//! layer (leveldefs `Layer`) four cell trees — floors, walls, units, town
//! art — filled from drawn tile records and units as the player
//! moves, town art grids, persisted per character in `.map` /
//! `.ma<k>` files, shown full screen or as a mini map by a
//! pass of cel draws, markers and header text.
//!
//! Plain Rust, no Bevy: the pass emits [`AutomapDraw`]s; tables, the
//! registry and the files come in through small seams
//! ([`AutomapLevels`], [`place::UnitCels`], [`options::OptionStore`],
//! [`persist::MaFile`]).

pub mod cells;
pub mod draw;
pub mod header;
pub mod markers;
pub mod options;
pub mod persist;
pub mod picker;
pub mod place;
pub mod session;
pub mod town;
pub mod view;

#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod tests;

use d2_sim::rng::Seed;

use crate::rules::camera::ClientPos;
use crate::rules::draw_order::{NearRooms, Room, REC_DRAWN, REC_HIDDEN};

pub use cells::{Cell, CellTree, LayerCells, TreeKind};
pub use draw::{AutomapDraw, Label, TextAlign};
pub use options::{CelFile, OptionStore, Options};
pub use persist::{CelLimits, MaFile, MaRecord};
pub use picker::CelPicker;
pub use place::{AutomapUnit, UnitCels};
pub use town::TownKind;
pub use view::{Bounds, FrameFacts, View};

/// An automap failure: one of the game's fatal checks, a bad file, or
/// a fact the caller did not supply.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AutomapError {
    #[error("automap fatal {code:#x} ({rule}): {detail}")]
    Fatal {
        code: u32,
        rule: &'static str,
        detail: String,
    },
    #[error("automap file: {0}")]
    File(String),
    #[error("automap: {0}")]
    Unresolved(String),
}

impl AutomapError {
    pub fn fatal(code: u32, rule: &'static str, detail: String) -> Self {
        AutomapError::Fatal { code, rule, detail }
    }
}

/// The automap seed at table load: `{0, 666}`.
pub const SEED: Seed = Seed::new(0, 666);
/// Reveal distance.
pub const REVEAL_DISTANCE: i32 = 0x50;
/// Frames without reveal after a placement.
pub const PLACEMENT_COUNTDOWN: u32 = 2;

/// The level facts the automap reads (leveldefs `Layer` +0x08; `LevelType`
/// +0x34; the act).
pub trait AutomapLevels {
    fn layer(&self, level: u32) -> Option<u32>;
    fn level_type(&self, level: u32) -> Option<u32>;
    fn act(&self, level: u32) -> Option<u8>;
}

/// The tables of the cell sources.
#[derive(Clone, Copy)]
pub struct AutomapTables<'a> {
    pub picker: &'a CelPicker,
    pub levels: &'a dyn AutomapLevels,
    pub cels: &'a dyn UnitCels,
}

impl AutomapTables<'_> {
    fn layer(&self, level: u32) -> Result<u32, AutomapError> {
        self.levels
            .layer(level)
            .ok_or_else(|| AutomapError::Unresolved(format!("leveldefs Layer of level {level}")))
    }
}

/// The near rooms of the player's room with each room's unit list
/// in the form reads (`units[i]` belongs to `near.rooms[i]`).
pub struct RevealRooms<'a> {
    pub near: &'a mut NearRooms,
    pub units: &'a mut [Vec<AutomapUnit>],
}

/// The reveal state: the countdown and the last reveal position
/// (initially 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RevealState {
    pub countdown: u32,
    pub last: (i32, i32),
}

impl RevealState {
    /// Whether this frame reveals; a running countdown is
    /// decremented instead. Stores the position when it reveals.
    pub fn step(&mut self, player: ClientPos) -> bool {
        if self.countdown != 0 {
            self.countdown -= 1;
            return false;
        }
        let dx = (self.last.0 - player.x).abs();
        let dy = (self.last.1 - player.y).abs();
        let d = (2 * dx.max(dy) + dx.min(dy)) / 2;
        if d < REVEAL_DISTANCE {
            return false;
        }
        self.last = (player.x, player.y);
        true
    }
}

/// A layer of the list; its cells are in memory only while it is
/// current.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerInfo {
    pub id: u32,
    pub town: TownKind,
}

/// The automap of one game.
#[derive(Debug, Clone)]
pub struct Automap {
    /// The layer list, head first (prepend).
    layers: Vec<LayerInfo>,
    /// The current layer.
    current: Option<u32>,
    /// The current layer's trees.
    pub cells: LayerCells,
    /// The automap seed (it outlives a game: reset only at
    /// table load).
    pub seed: Seed,
    pub reveal: RevealState,
    pub options: Options,
    pub view: View,
    /// The open `.ma<k>` file; `None` before one is attached.
    file: Option<MaFile>,
    /// The act record's second u32 (S→C 0x03 u32@8).
    act2: u32,
    /// Cel counts of the loaded files, checked by the load.
    pub limits: CelLimits,
}

impl Automap {
    /// UI init (options; the groups are a constant table;
    /// the cel files are the caller's, see [`options::cel_paths`]).
    pub fn new(store: &dyn OptionStore, seed: Seed) -> Self {
        let options = Options::init(store);
        Automap {
            layers: Vec::new(),
            current: None,
            cells: LayerCells::default(),
            seed,
            reveal: RevealState::default(),
            view: View::new(&options),
            options,
            file: None,
            act2: 0,
            limits: CelLimits::default(),
        }
    }

    /// Attaches the data file the index opened and the act
    /// record's second u32.
    pub fn attach_file(&mut self, file: MaFile, act2: u32) {
        self.file = Some(file);
        self.act2 = act2;
    }

    pub fn file(&self) -> Option<&MaFile> {
        self.file.as_ref()
    }

    pub fn current(&self) -> Option<u32> {
        self.current
    }

    pub fn layers(&self) -> &[LayerInfo] {
        &self.layers
    }

    /// The current layer's town kind.
    pub fn town(&self) -> TownKind {
        self.current
            .and_then(|c| self.layers.iter().find(|l| l.id == c))
            .map_or(TownKind::None, |l| l.town)
    }

    /// Find or create layer `l`; when it is not the
    /// current one, save the current layer, free every cell, make `l`
    /// current and load its cells.
    pub fn switch(&mut self, l: u32) -> Result<(), AutomapError> {
        if !self.layers.iter().any(|x| x.id == l) {
            self.layers.insert(
                0,
                LayerInfo {
                    id: l,
                    town: TownKind::None,
                },
            );
        }
        if self.current == Some(l) {
            return Ok(());
        }
        self.save()?;
        self.cells.clear();
        self.current = Some(l);
        self.load(l)
    }

    /// Appends the current layer's unsaved cells,
    /// each tree in in-order; none at all: nothing is appended.
    pub fn save(&mut self) -> Result<(), AutomapError> {
        let (Some(l), Some(file)) = (self.current, self.file.as_mut()) else {
            return Ok(());
        };
        let town = self
            .layers
            .iter()
            .find(|x| x.id == l)
            .map_or(TownKind::None, |x| x.town);
        let blobs = TreeKind::ALL.map(|k| {
            self.cells
                .tree(k)
                .in_order()
                .into_iter()
                .filter(|c| !c.saved)
                .collect()
        });
        // No unsaved cell in any tree, no record.
        if blobs.iter().all(Vec::is_empty) {
            return Ok(());
        }
        file.append(&MaRecord {
            layer: l,
            town_kind: town as u32,
            act2: self.act2,
            blobs,
        })?;
        Ok(())
    }

    /// The stored cells of layer `l`, saved flag 1.
    fn load(&mut self, l: u32) -> Result<(), AutomapError> {
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        let loaded = file.load(l, self.act2, self.limits)?;
        for (k, cells) in TreeKind::ALL.into_iter().zip(loaded.trees) {
            for c in cells {
                self.cells.tree_mut(k).insert(c);
            }
        }
        Ok(())
    }

    /// The room's floors, then walls,
    /// then units.
    pub fn add_room(
        &mut self,
        room: &mut Room,
        units: &mut [AutomapUnit],
        all: bool,
        t: &AutomapTables,
    ) -> Result<(), AutomapError> {
        let level_type = t.levels.level_type(room.level).ok_or_else(|| {
            AutomapError::Unresolved(format!("leveldefs LevelType of level {}", room.level))
        })?;
        let origin = (room.tiles.x, room.tiles.y);
        let all = all || place::ADD_ALL;
        // Flag 0x8 skips the record even with all = 1.
        let keep = |flags: u32| flags & REC_HIDDEN == 0 && (all || flags & REC_DRAWN != 0);
        for (recs, kind) in [
            (&mut room.floors, TreeKind::Floor),
            (&mut room.walls, TreeKind::Wall),
        ] {
            for rec in recs.iter_mut().filter(|r| keep(r.flags)) {
                place::add_tile(
                    rec,
                    origin,
                    level_type,
                    t.picker,
                    &mut self.seed,
                    self.cells.tree_mut(kind),
                )?;
            }
        }
        let act = t
            .levels
            .act(room.level)
            .ok_or_else(|| AutomapError::Unresolved(format!("act of level {}", room.level)))?;
        place::add_units(
            units,
            place::UnitLevel {
                id: room.level,
                act,
            },
            t.cels,
            self.cells.tree_mut(TreeKind::Unit),
        )
    }

    /// R1, once per frame. `player` is the local
    /// player's client position (none: fatal 0x696); `rooms` the near
    /// rooms of its room (none: stop).
    pub fn reveal_frame(
        &mut self,
        player: Option<ClientPos>,
        rooms: Option<RevealRooms>,
        t: &AutomapTables,
    ) -> Result<(), AutomapError> {
        if self.reveal.countdown != 0 {
            self.reveal.countdown -= 1;
            return Ok(());
        }
        let p =
            player.ok_or_else(|| AutomapError::fatal(0x696, "§5 r1", "no local player".into()))?;
        if !self.reveal.step(p) {
            return Ok(());
        }
        let Some(r) = rooms else { return Ok(()) };
        if r.units.len() != r.near.rooms.len() {
            return Err(AutomapError::Unresolved(format!(
                "§5 r1: {} unit lists for {} near rooms",
                r.units.len(),
                r.near.rooms.len()
            )));
        }
        let l = t.layer(r.near.level.id)?;
        self.switch(l)?;
        for (room, units) in r.near.rooms.iter_mut().zip(r.units.iter_mut()) {
            if t.layer(room.level)? == l {
                self.add_room(room, units, false, t)?;
            }
        }
        Ok(())
    }

    /// (DRLG callback +0x454): a room of a whole preset level (lvlprest
    /// `AutoMap` = 1), every record.
    pub fn preset_room(
        &mut self,
        room: &mut Room,
        units: &mut [AutomapUnit],
        t: &AutomapTables,
    ) -> Result<(), AutomapError> {
        let prev = self.current;
        self.switch(t.layer(room.level)?)?;
        self.add_room(room, units, true, t)?;
        if let Some(p) = prev {
            self.switch(p)?;
        }
        Ok(())
    }

    /// A unit placement sets the countdown to 2.
    pub fn placed(&mut self) {
        self.reveal.countdown = PLACEMENT_COUNTDOWN;
    }

    /// (DRLG callback +0x488): the town art of level `level` with picked file
    /// `f` and centre tile (cx, cy).
    pub fn town_art(
        &mut self,
        level: u32,
        f: u32,
        centre: (i32, i32),
        t: &AutomapTables,
    ) -> Result<(), AutomapError> {
        let prev = self.current;
        let l = t.layer(level)?;
        self.switch(l)?;
        let (kind, cells) = town::town_cells(level, f, centre.0, centre.1)?;
        if let Some(x) = self.layers.iter_mut().find(|x| x.id == l) {
            x.town = kind;
        }
        for c in cells {
            self.cells.tree_mut(TreeKind::Town).insert(c);
        }
        if let Some(p) = prev {
            self.switch(p)?;
        }
        Ok(())
    }

    /// Tab / middle button / mini-panel button toggled UI state
    /// 0x0A; when it is now closed, re-centre with force 0.
    pub fn toggled(&mut self, now_open: bool, f: &FrameFacts) {
        if !now_open {
            self.view.recentre(false, &self.options, f);
        }
    }

    /// F9 (cmd 8), re-centre with force 1.
    pub fn centre(&mut self, f: &FrameFacts) {
        self.view.recentre(true, &self.options, f);
    }

    /// Space (cmd 38) when the clear actually ran.
    pub fn cleared(&mut self, f: &FrameFacts) {
        self.view.recentre(false, &self.options, f);
    }

    /// The size setter; `true` when the cel files must be
    /// reloaded.
    pub fn set_size(&mut self, mini: bool, f: &FrameFacts) -> bool {
        self.view.set_size(mini, &self.options, f)
    }

    /// The automap pass.
    pub fn draw_pass(
        &mut self,
        p: &PassInput,
        store: &mut dyn OptionStore,
    ) -> Result<Vec<AutomapDraw>, AutomapError> {
        let mut out = Vec::new();
        if !p.open || !p.frame.draws_world() || !p.ready {
            return Ok(out);
        }
        let f = &p.frame;
        self.view.frame(&self.options, f);
        // Cells: panel side, origin, blocks.
        let v = draw::pass_fade(&self.view, &mut self.options, store);
        let s = self.view.panel_side(&mut self.options, f);
        let a = self.view.origin(s, f);
        let town = self.town();
        draw::draw_cells(
            &self.cells,
            town,
            &draw::CellPass {
                view: &self.view,
                opts: &self.options,
                a,
                fade: draw::FadeFacts {
                    v,
                    open_mode: f.open_mode,
                    player_byte_18: p.player_byte_18,
                },
                frame: f,
            },
            &mut out,
        );
        let ctx = markers::MarkerCtx {
            local_party: p.local_party,
            party: self.options.party,
            names: self.options.party_names,
            player_gate: p.player_gate,
            mini: self.view.mini,
            div: self.view.div,
            a,
            rect: self.view.marker,
            palette: p.palette,
        };
        markers::unit_markers(p.markers, &ctx, &mut out);
        markers::roster_markers(p.roster, p.local_act, &ctx, &mut out);
        header::header(p.header, f.width, p.strings, &mut out)?;
        Ok(out)
    }

    /// Save the current layer, free cells and layers; current:= none. The seed
    /// is kept. Returns the data file for the caller to write.
    pub fn teardown(&mut self) -> Result<Option<MaFile>, AutomapError> {
        self.save()?;
        self.cells.clear();
        self.layers.clear();
        self.current = None;
        Ok(self.file.take())
    }
}

/// The inputs of one automap pass.
pub struct PassInput<'a> {
    pub frame: FrameFacts,
    /// UI state 0x0A open.
    pub open: bool,
    /// The client act, a local player and its room exist.
    pub ready: bool,
    /// The local player's byte +0x18 (fade 3).
    pub player_byte_18: u8,
    pub markers: &'a [markers::MarkerUnit],
    pub local_party: i16,
    /// ≠ 0 for the player ([`markers::unit_dead`]).
    pub player_gate: bool,
    pub palette: markers::MarkerPalette,
    pub roster: &'a [markers::RosterEntry],
    pub local_act: u8,
    pub header: &'a header::HeaderFacts,
    pub strings: &'a dyn Fn(u16) -> Option<Vec<u16>>,
}
