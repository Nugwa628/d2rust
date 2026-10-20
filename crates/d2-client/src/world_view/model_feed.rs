//! [`ModelFeed`]: the [`ViewFeed`] hooks the client world model answers
//! (RW2): the local player's position (camera) and unit positions
//! (camera), given the `Levels.txt` rows, BlankScreen of the local
//! player's level, and, once the world view hands
//! it over, the base-game UI's open mode. Every
//! other hook (shake, player seed, unit offsets, map tiles, light,
//! weather) goes to the wrapped feed, [`NoFeed`] by default; why the model
//! cannot answer them yet is [`PENDING`].
//!
//! With a [`Preview`] (the play preview) the map is on and
//! the tile art, unit facts and unit offsets are the labelled fills of
//! [`preview`] (`D2RUST's own, unverified`); without one every answer above
//! is the strict one.
//!
//! Positions are the model's cells (`ClientUnit::position`). A moving
//! unit (players, monsters, missiles) is on a dynamic path: its 16.16
//! position is the cell centre after every placement the message specs
//! state; a unit created at (0, 0) has its path at
//! (0, 0). The walk prediction between messages is not a message rule
//! (open question 2), so the position changes only when a
//! message places the unit. Static units (objects, items, tiles) are at
//! their cell; an item without a cell (not on the ground, until the item
//! stream is specified) has no position.

use std::sync::{Arc, Mutex};

use d2_sim::rng::Seed;

use crate::audio::driver::ClientSeed;
use crate::bridge::drlg::DrlgRoomId;
use crate::bridge::world::{ClientWorld, LevelRow, UnitKey, ITEM, OBJECT, TILE};
use crate::bridge::ClientUnit;
use crate::rules::camera::shake_offsets;
use crate::rules::draw_order::{FadeClock, NearRooms, UnitFacts};
use crate::rules::lighting::records::LightList;
use crate::rules::{MapTile, OpenMode, UnitPosition, ViewSource};

use super::feed::{NoFeed, RunningShake, ViewFeed};
use super::near_rooms::MapState;
use super::preview::{self, Preview};
use super::unit_facts::{self, UnitFactTables};
use super::weather_view::WeatherView;
use super::{UnitPose, ViewAssets, ViewError};

/// The 16.16 position of a dynamic path at the centre of cell `c`.
fn centre(c: u16) -> u32 {
    (u32::from(c) << 16) | 0x8000
}

/// The position of `unit` as the client keeps it (camera).
pub fn unit_position(unit: &ClientUnit) -> Result<UnitPosition, String> {
    let static_kind = matches!(unit.key.unit_type, OBJECT | ITEM | TILE);
    match (static_kind, unit.position) {
        (true, Some((x, y))) => Ok(UnitPosition::Static {
            sx: i32::from(x),
            sy: i32::from(y),
        }),
        (true, None) => Err(format!(
            "unit ({}, {}) has no cell (an item off the ground has no world position, msg-stats-items.md §2 r4)",
            unit.key.unit_type, unit.key.guid
        )),
        (false, p) => {
            let (x, y) = p.unwrap_or((0, 0));
            // A walking monster's precise track position, while it is in
            // the model's cell (`bridge::motion`).
            if let Some((x16, y16)) = unit
                .precise
                .filter(|&(px, py)| p == Some(((px >> 16) as u16, (py >> 16) as u16)))
            {
                return Ok(UnitPosition::Moving { x16, y16 });
            }
            Ok(UnitPosition::Moving {
                x16: centre(x),
                y16: centre(y),
            })
        }
    }
}

/// The [`ViewFeed`] / model hooks the client model cannot fill yet, each
/// with the input it lacks (named, never guessed). [`ModelFeed`]
/// leaves them to `inner` ([`NoFeed`]: none).
pub const PENDING: &[(&str, &str)] = &[
    (
        "ViewFeed::near_rooms (room unit facts)",
        "the unit facts come from the model (`unit_facts.rs`: flags, flag-ex, states 7 / 143 / 146, \
         `unflatDead`, `DrawUnder`, the `LOSDraw` gate) once `unit_tables` is set; without the \
         tables `ViewFeed::unit_facts` refuses; the sight test runs over the client DRLG's collision \
         grids and stays `None` where it cannot be placed (no DRLG, no local room, a class without \
         a size row; corpse and DrawUnder facts match the recorded checks (check w2-02-corpse-barrel: rows 1-240, the \
         world, equal; PROVISIONAL REC-273 stays for the sight test only, oracle: the cause row \
         sight test in causes/run13-l30.tsv and check w2-02-sight-cave, which hides no unit in a \
         cave yet, run-queue w1-d1-draw-facts))",
    ),
    (
        "ViewFeed::tile_art, ViewSource::tile_blocks",
        "each record's DT1 entry (path, index) is known (`rooms.md` §9.3, `MapState::entry`), \
         but the art's shade and blend and the per-block shading need the frame's light \
         (below); so the app runs the feed without the map (`ModelFeed::map` off)",
    ),
    (
        "ViewFeed::light",
        "the app wires the light in the preview (`preview_light`: S→C 0x53 environment, the \
         player and `light_sources` records; \
         `preview_light::tests::the_model_feed_states_the_light_unless_fullbright`); the \
         per-unit look inputs (fade, ghostly, hover, items, remaps) are not in the model, and \
         no unit code creates records in `ClientWorld::lights`",
    ),
    (
        "ViewFeed::weather_frame, ViewFeed::sky_items (strict path)",
        "the strict feed lends no weather state: the weather frame, the water floor and the art of \
         passes 4 / 9 are the play preview's (`ModelFeed::weather`, `weather_view.rs`: the seed, \
         update count, day period, video mode and frame rate are `D2RUST's own, unverified` fills); \
         lightning is never started (`draw-order-2.md` open question 4)",
    ),
    (
        "ViewFeed::player_seed",
        "the model's copy of the local player's seed is read-only here; the shake draws on the \
         draw-phase seed (`ModelFeed::client_seed`, `camera.md` Randomness)",
    ),
];

/// The client world model's answers, over `inner` for the rest.
#[derive(Debug, Clone, Default)]
pub struct ModelFeed<F = NoFeed> {
    pub inner: F,
    /// The `Levels.txt` rows by level id. `None`: BlankScreen goes to
    /// `inner` (the app supplies no rows yet).
    pub levels: Option<Vec<LevelRow>>,
    /// The base-game UI's open mode, once the world
    /// view has handed one over; `None`: `open_mode` goes to `inner`.
    pub ui_open_mode: Option<OpenMode>,
    /// The map from the client DRLG. `None` (the
    /// default): `near_rooms` goes to `inner`. Opt-in ([`Self::with_map`])
    /// while tile art is pending: with near rooms every drawn tile needs
    /// [`ViewFeed::tile_art`] (`PENDING`).
    pub map: Option<MapState>,
    /// The play preview's fills ([`preview`]): tile art,
    /// unit facts and offsets the model lacks, each `D2RUST's own,
    /// unverified`. `None` (the default): the strict path.
    pub preview: Option<Preview>,
    /// The local player's predicted 16.16 position (the walk prediction,
    /// `bridge::predict`); read only with a [`Preview`].
    pub local_at: Option<(UnitKey, (u32, u32))>,
    /// The push-out places the local player this update
    /// (`set_local_push`); read only with a [`Preview`].
    pub local_push: bool,
    /// The table columns of the unit facts (`unit_facts`); `None`: the
    /// facts the model needs them for refuse.
    pub unit_tables: Option<UnitFactTables>,
    /// The skill-move draw offsets of the preview (`skill_motion`).
    /// The shadow offsets of the same units (`set_motion_shadow_offsets`).
    pub motion_shadow_offsets: std::collections::BTreeMap<UnitKey, (i32, i32)>,
    pub motion_offsets: std::collections::BTreeMap<UnitKey, (i32, i32)>,
    /// The play preview's weather (`weather_view`, `D2RUST's own,
    /// unverified`): water floors keep their bit, passes 4 and 9 draw.
    /// `None`: no weather state.
    pub weather: Option<WeatherView>,
    /// The local player's draw-phase client seed (`audio::driver::ClientSeed`),
    /// synced with the model's copy: the shake's two draws step it
    /// (Randomness). [`Self::with_weather`] shares the
    /// weather view's, so the shake comes before the weather update's draws
    /// on the one seed.
    pub client_seed: Arc<Mutex<ClientSeed>>,
}

impl<F> ModelFeed<F> {
    pub fn new(inner: F) -> Self {
        Self {
            inner,
            levels: None,
            ui_open_mode: None,
            map: None,
            preview: None,
            local_at: None,
            local_push: false,
            unit_tables: None,
            motion_offsets: Default::default(),
            motion_shadow_offsets: Default::default(),
            weather: None,
            client_seed: Arc::default(),
        }
    }

    /// The feed with the preview's weather (`weather_view`).
    pub fn with_weather(mut self, weather: WeatherView) -> Self {
        self.client_seed = weather.client_seed();
        self.weather = Some(weather);
        self
    }

    /// The feed with the map and the play preview's fills
    /// ([`preview`]).
    pub fn with_preview(mut self, preview: Preview) -> Self {
        self.map.get_or_insert_with(MapState::default);
        self.preview = Some(preview);
        self
    }

    /// The feed with the client DRLG's near rooms.
    pub fn with_map(mut self) -> Self {
        self.map = Some(MapState::default());
        self
    }
}

impl<F> ModelFeed<F> {
    /// The unit's position: with a preview, the local player is at its
    /// predicted position (D2RUST's own, unverified: the walk prediction); otherwise
    /// [`unit_position`].
    fn position_of(&self, unit: &ClientUnit) -> Result<UnitPosition, String> {
        match (&self.preview, self.local_at) {
            (Some(_), Some((key, (x16, y16)))) if key == unit.key => {
                Ok(UnitPosition::Moving { x16, y16 })
            }
            _ => unit_position(unit),
        }
    }
}

impl<F> ModelFeed<F> {
    /// An object's `Xoffset`, `Yoffset` (the draw
    /// adds `+0x148` / `+0x14C` once before the composite
    /// draw and its box test); `(0, 0)` for
    /// other types, outside the preview or without the table
    /// (`Preview::object_offsets`).
    fn object_offset(&self, unit: &ClientUnit) -> (i32, i32) {
        if unit.key.unit_type != OBJECT {
            return (0, 0);
        }
        self.preview
            .as_ref()
            .and_then(|p| p.object_offsets.get(&unit.class).copied())
            .unwrap_or((0, 0))
    }
}

impl<F: ViewSource> ViewSource for ModelFeed<F> {
    fn unit_position(&self, unit: &ClientUnit) -> Result<UnitPosition, String> {
        self.position_of(unit)
    }

    /// Preview: `(0, 0)` ([`preview::unit_offset`]) plus the skill-move
    /// offset and an object's table offset ([`Self::object_offset`]);
    /// else the inner feed's.
    fn unit_offset(&self, unit: &ClientUnit, pose: &UnitPose) -> Result<(i32, i32), String> {
        if self.preview.is_some() {
            let (x, y) = preview::unit_offset();
            let (mx, my) = self
                .motion_offsets
                .get(&unit.key)
                .copied()
                .unwrap_or((0, 0));
            let (ox, oy) = self.object_offset(unit);
            return Ok((x + mx + ox, y + my + oy));
        }
        self.inner.unit_offset(unit, pose)
    }

    /// Preview: as [`Self::unit_offset`] with the motion record's shadow
    /// offset (objects adding their table offset
    /// there too); else the inner feed's.
    fn unit_shadow_offset(&self, unit: &ClientUnit, pose: &UnitPose) -> Result<(i32, i32), String> {
        if self.preview.is_some() {
            let (x, y) = preview::unit_offset();
            let (mx, my) = self
                .motion_shadow_offsets
                .get(&unit.key)
                .copied()
                .unwrap_or((0, 0));
            let (ox, oy) = self.object_offset(unit);
            return Ok((x + mx + ox, y + my + oy));
        }
        self.inner.unit_shadow_offset(unit, pose)
    }

    fn map_tiles(
        &self,
        world: &ClientWorld,
        assets: &ViewAssets,
    ) -> Result<Vec<MapTile>, ViewError> {
        self.inner.map_tiles(world, assets)
    }

    fn tile_blocks(&self, tile: &MapTile) -> Result<Vec<crate::rules::BlockShade>, ViewError> {
        if let Some(p) = &self.preview {
            let shades = p.block_shades_of(tile.key);
            if !shades.is_empty() {
                return Ok(shades);
            }
        }
        self.inner.tile_blocks(tile)
    }

    fn tile_light(&self, tile: &MapTile) -> Option<String> {
        match &self.preview {
            Some(p) => p.tile_light_of(tile.key),
            None => self.inner.tile_light(tile),
        }
    }

    fn tile_alpha(&self, tile: &MapTile) -> Option<u8> {
        match &self.preview {
            Some(p) => p.tile_alpha_of(tile.key),
            None => self.inner.tile_alpha(tile),
        }
    }
}

impl<F: ViewFeed> ViewFeed for ModelFeed<F> {
    /// The local player's position; no local player
    /// → `None`.
    fn player(&self, world: &ClientWorld) -> Result<Option<UnitPosition>, ViewError> {
        let Some(unit) = world.local() else {
            return Ok(None);
        };
        self.position_of(unit)
            .map(Some)
            .map_err(|message| ViewError::Unresolved {
                what: "local player position",
                spec: "client/model.md",
                message,
            })
    }

    /// The UI flags' open mode (camera); without
    /// the base-game UI, the inner feed's answer.
    fn open_mode(&self, world: &ClientWorld) -> Result<OpenMode, ViewError> {
        match self.ui_open_mode {
            Some(m) => Ok(m),
            None => self.inner.open_mode(world),
        }
    }

    fn set_ui_open_mode(&mut self, mode: OpenMode) {
        self.ui_open_mode = Some(mode);
    }

    fn set_local_prediction(&mut self, at: Option<(UnitKey, (u32, u32))>) {
        self.local_at = at;
    }

    fn set_local_push(&mut self, pushed: bool) {
        self.local_push = pushed;
    }

    fn set_unit_fact_tables(&mut self, tables: UnitFactTables) {
        self.unit_tables = Some(tables);
    }

    fn set_motion_offsets(&mut self, offsets: std::collections::BTreeMap<UnitKey, (i32, i32)>) {
        self.motion_offsets = offsets;
    }

    fn set_motion_shadow_offsets(
        &mut self,
        offsets: std::collections::BTreeMap<UnitKey, (i32, i32)>,
    ) {
        self.motion_shadow_offsets = offsets;
    }

    fn set_hover(&mut self, unit: Option<UnitKey>) {
        if let Some(p) = self.preview.as_mut() {
            p.light.set_hover(unit);
        }
    }

    /// The preview places cels cut by the frame edge (the preview).
    fn edge_clip(&self) -> bool {
        self.preview.is_some()
    }

    /// The model's running shake (started by the
    /// model rules: S→C 0x5A code 0x12, client missile functions), else
    /// the inner feed's.
    fn shake(&self, world: &ClientWorld) -> Result<Option<RunningShake>, ViewError> {
        if let Some(s) = world.shake {
            return Ok(Some(RunningShake {
                shake: s.shake,
                start_tick: s.start_tick,
            }));
        }
        self.inner.shake(world)
    }

    fn player_seed(&mut self, world: &ClientWorld) -> Result<&mut Seed, ViewError> {
        self.inner.player_seed(world)
    }

    /// The shake's two draws on the draw-phase seed, synced with the
    /// model's copy first; a local player without a seed goes to `inner`.
    fn shake_draws(&mut self, world: &ClientWorld, a: u32) -> Result<(i32, i32), ViewError> {
        let mut held = self.client_seed.lock().unwrap_or_else(|e| e.into_inner());
        held.sync(world);
        if let Some(seed) = held.seed() {
            return Ok(shake_offsets(a, seed));
        }
        drop(held);
        self.inner.shake_draws(world, a)
    }

    /// 5: BlankScreen of the level of the local
    /// player's room; no room → no level, BlankScreen 0. The level never
    /// comes from 0x03 u16@6 or a 0x07 level byte.
    fn blank_screen(&self, world: &ClientWorld) -> Result<bool, ViewError> {
        let Some(levels) = &self.levels else {
            return self.inner.blank_screen(world);
        };
        let Some(level) = world.player_level() else {
            return Ok(false);
        };
        levels
            .get(usize::from(level))
            .map(|r| r.blank_screen)
            .ok_or(ViewError::Unresolved {
                what: "player level BlankScreen",
                spec: "client/model.md",
                message: format!("level {level} past the Levels rows"),
            })
    }

    /// Preview: the frame's light and look (`preview_light`), unless
    /// full bright; else the inner feed's.
    fn light(&self, world: &ClientWorld) -> Result<Option<super::feed::FeedLight<'_>>, ViewError> {
        if let Some(p) = &self.preview {
            return Ok(p.light.frame().map(|light| super::feed::FeedLight {
                light,
                look: p.light.look(),
            }));
        }
        self.inner.light(world)
    }

    /// from the client DRLG when the map is on
    /// ([`MapState`]); the unit facts the model holds (type, mode, local
    /// player) over the inner feed's for the rest.
    /// Preview: the facts are [`preview::unit_facts`], the floors are
    /// dried ([`preview::dry_floors`]) and a build that fails is logged
    /// once and leaves the frame without the map.
    fn near_rooms(&mut self, world: &ClientWorld) -> Result<Option<&mut NearRooms>, ViewError> {
        let Self {
            inner,
            levels,
            map,
            preview,
            unit_tables,
            weather,
            ..
        } = self;
        let Some(map) = map.as_mut() else {
            return inner.near_rooms(world);
        };
        let Some(preview) = preview.as_ref() else {
            let inner = &*inner;
            let tables = unit_tables.as_ref();
            return map.near_rooms(world, levels.as_deref(), |u| {
                if let Some(t) = tables {
                    return unit_facts::model_facts(world, u, t, levels.as_deref());
                }
                Ok(UnitFacts {
                    unit_type: u.key.unit_type,
                    mode: u.mode,
                    local: world.local_player == Some(u.key),
                    ..inner.unit_facts(world, u)?
                })
            });
        };
        match map.near_rooms(world, levels.as_deref(), |u| {
            preview_facts(world, u, unit_tables.as_ref())
        }) {
            Ok(Some(near)) => {
                if weather.is_none() {
                    preview::dry_floors(near);
                }
                Ok(Some(near))
            }
            Ok(None) => Ok(None),
            Err(e) => {
                preview.log_once(format!("the map this frame: {e}"));
                Ok(None)
            }
        }
    }

    /// The map's undrawn units ([`MapState::units_undrawn`]).
    fn units_undrawn(&mut self, keys: &std::collections::BTreeSet<UnitKey>) {
        match self.map.as_mut() {
            Some(map) => map.units_undrawn(keys),
            None => self.inner.units_undrawn(keys),
        }
    }

    /// The map's automap callback rooms ([`MapState::automap_rooms`]).
    fn automap_rooms(
        &mut self,
        world: &ClientWorld,
    ) -> Result<
        Vec<(
            crate::bridge::drlg::DrlgRoomId,
            crate::rules::draw_order::Room,
        )>,
        ViewError,
    > {
        match self.map.as_mut() {
            Some(map) => map.automap_rooms(world),
            None => self.inner.automap_rooms(world),
        }
    }

    fn automap_marked(
        &mut self,
        id: crate::bridge::drlg::DrlgRoomId,
        room: &crate::rules::draw_order::Room,
        bits: u32,
    ) {
        match self.map.as_mut() {
            Some(map) => map.automap_marked(id, room, bits),
            None => self.inner.automap_marked(id, room, bits),
        }
    }

    fn unit_facts(&self, world: &ClientWorld, unit: &ClientUnit) -> Result<UnitFacts, ViewError> {
        if self.preview.is_some() {
            return preview_facts(world, unit, self.unit_tables.as_ref());
        }
        if let Some(t) = &self.unit_tables {
            return unit_facts::model_facts(world, unit, t, self.levels.as_deref());
        }
        self.inner.unit_facts(world, unit)
    }

    /// Preview: the act's shade tables, the skip frame and the near rooms'
    /// DT1 tiles resident ([`Preview::prepare`]); else the inner feed's.
    fn prepare(&mut self, world: &ClientWorld, assets: &mut ViewAssets) -> Result<(), ViewError> {
        if self.preview.is_none() {
            return self.inner.prepare(world, assets);
        }
        let counts = preview::record_counts(self.near_rooms(world)?.as_deref());
        let entries = match &self.map {
            Some(m) => preview::entries(m, &counts),
            None => Vec::new(),
        };
        let preview = self.preview.as_mut().expect("checked above");
        let r = preview.prepare(world, &entries, assets);
        self.weather_update(world, assets);
        r
    }

    fn follow_frame_schedule(&mut self) {
        if let Some(w) = self.weather.as_mut() {
            w.follow_frame_schedule();
        }
        self.inner.follow_frame_schedule();
    }

    fn level_weather(&self) -> Option<(bool, bool)> {
        self.weather.as_ref().and_then(|w| w.level_flags())
    }

    fn world_rolls(&self) -> bool {
        match &self.weather {
            Some(w) => w.world_rolls(),
            None => self.inner.world_rolls(),
        }
    }

    fn keep_sky(&mut self, tick: u64, sky: crate::rules::draw_order::sky::SkyPasses) {
        match self.weather.as_mut() {
            Some(w) => w.keep_sky(tick, sky),
            None => self.inner.keep_sky(tick, sky),
        }
    }

    fn take_kept_sky(&mut self, tick: u64) -> Option<crate::rules::draw_order::sky::SkyPasses> {
        match self.weather.as_mut() {
            Some(w) => w.take_kept_sky(tick),
            None => self.inner.take_kept_sky(tick),
        }
    }

    fn weather_update(&mut self, world: &ClientWorld, assets: &mut ViewAssets) {
        let local_at = self.local_at;
        let mode = self.ui_open_mode.unwrap_or(OpenMode::NONE);
        let Some(preview) = self.preview.as_mut() else {
            return self.inner.weather_update(world, assets);
        };
        if let Some(w) = self.weather.as_mut() {
            w.set_local_push(self.local_push);
            w.prepare(
                world,
                self.levels.as_deref(),
                local_at.map(|l| l.1),
                mode,
                assets,
            );
            if let Some(m) = w.take_failure() {
                preview.log_once(format!("weather: {m}"));
            }
        }
    }

    /// Preview: the frame's light map from the kept list
    /// ([`super::preview_light::PreviewLight::refresh`]); else the inner
    /// feed's.
    fn light_frame(&mut self, world: &ClientWorld, lights: &mut LightList) {
        let local_at = self.local_at;
        let Some(preview) = self.preview.as_mut() else {
            return self.inner.light_frame(world, lights);
        };
        let tables = preview
            .tiles
            .shades(world.palette_act.unwrap_or(0))
            .copied();
        preview
            .light
            .refresh(world, lights, local_at, tables.as_ref());
    }

    fn set_light_quality(&mut self, q: Option<u8>) {
        match self.preview.as_mut() {
            Some(preview) => preview.light.set_quality(q),
            None => self.inner.set_light_quality(q),
        }
    }

    fn take_unit_orders(&mut self) -> Vec<(DrlgRoomId, Vec<UnitKey>)> {
        match self.map.as_mut() {
            Some(m) => m.take_unit_orders(),
            None => self.inner.take_unit_orders(),
        }
    }

    fn weather_frame(
        &mut self,
        world: &ClientWorld,
    ) -> Result<Option<crate::rules::draw_order::source::WeatherFrame<'_>>, ViewError> {
        if self.weather.is_some() {
            return Ok(self.weather.as_mut().and_then(WeatherView::frame));
        }
        self.inner.weather_frame(world)
    }

    /// Preview weather: the cels and lines of passes 4 and 9
    /// ([`WeatherView::items`]); a failure is logged once and the frame
    /// goes without them.
    fn sky_items(
        &self,
        sky: &crate::rules::draw_order::sky::SkyPasses,
        assets: &ViewAssets,
    ) -> Result<Vec<crate::scene::DrawItem>, ViewError> {
        let (Some(w), Some(p)) = (&self.weather, &self.preview) else {
            return self.inner.sky_items(sky, assets);
        };
        let mode = self.ui_open_mode.unwrap_or(OpenMode::NONE);
        match w.items(sky, mode, assets.shades.as_ref(), assets) {
            Ok(items) => Ok(items),
            Err(e) => {
                p.log_once(format!("weather draws: {e}"));
                Ok(Vec::new())
            }
        }
    }

    fn fade_clock(&self, world: &ClientWorld) -> Result<FadeClock, ViewError> {
        self.inner.fade_clock(world)
    }

    fn tile_art(
        &self,
        tile: &crate::rules::draw_order::OrderedTile,
        assets: &ViewAssets,
    ) -> Result<crate::rules::draw_order::source::TileArt, ViewError> {
        match &self.preview {
            Some(p) => Ok(p.tile_art(self.map.as_ref(), tile)),
            None => self.inner.tile_art(tile, assets),
        }
    }
}

/// The preview's facts of a unit ([`preview::unit_facts`]); with the
/// unit tables, the model's facts and the sight test of
/// ([`unit_facts::sight_gate`]). Where the test
/// cannot run (no local player, room or level yet), the preview's
/// "visible" stays (`D2RUST's own, unverified`, the preview).
fn preview_facts(
    world: &ClientWorld,
    unit: &ClientUnit,
    tables: Option<&UnitFactTables>,
) -> Result<UnitFacts, ViewError> {
    let mut f = preview::unit_facts(world, unit);
    if let Some(t) = tables {
        unit_facts::fill_model(&mut f, world, unit, t)?;
        if let Some(hidden) = unit_facts::sight_gate(world, unit, t) {
            f.sight_hidden = Some(hidden);
        }
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{MONSTER, PLAYER};
    use crate::bridge::UnitKey;

    fn unit(unit_type: u8, position: Option<(u16, u16)>) -> ClientUnit {
        let mut u = ClientUnit::new(UnitKey::new(unit_type, 1));
        u.position = position;
        u
    }

    #[test]
    fn local_player_position_is_the_cell_centre() {
        let feed = ModelFeed::<NoFeed>::default();
        let mut w = ClientWorld::default();
        assert_eq!(feed.player(&w).unwrap(), None);
        let key = UnitKey::new(PLAYER, 1);
        w.units.insert(key, unit(PLAYER, Some((0x1241, 0x11C4))));
        // A unit, but not the local player: no camera.
        assert_eq!(feed.player(&w).unwrap(), None);
        w.local_player = Some(key);
        assert_eq!(
            feed.player(&w).unwrap(),
            Some(UnitPosition::Moving {
                x16: 0x1241_8000,
                y16: 0x11C4_8000
            })
        );
        // Created at (0, 0): the path is at (0, 0).
        w.units.get_mut(&key).unwrap().position = None;
        assert_eq!(
            feed.player(&w).unwrap(),
            Some(UnitPosition::Moving {
                x16: 0x8000,
                y16: 0x8000
            })
        );
        // Open mode without the UI: 0.
        assert_eq!(feed.open_mode(&w).unwrap(), OpenMode::NONE);
    }

    #[test]
    fn the_shake_is_the_models() {
        let feed = ModelFeed::<NoFeed>::default();
        let mut w = ClientWorld::default();
        assert_eq!(feed.shake(&w).unwrap(), None);
        w.server_ticks = 3;
        w.start_shake(6, 4000, 10000, 4000);
        // t2 = 0 starts nothing: the running one stays.
        w.server_ticks = 4;
        w.start_shake(9, 1, 0, 1);
        let r = feed.shake(&w).unwrap().unwrap();
        assert_eq!(
            (r.shake, r.start_tick),
            (crate::rules::Shake::start(6, 4000, 10000, 4000).unwrap(), 3)
        );
    }

    #[test]
    fn the_shake_draws_on_the_draw_phase_seed() {
        use super::super::feed::frame_shake;
        use crate::rules::camera::shake_offsets;
        use crate::rules::Shake;
        let mut feed = ModelFeed::<NoFeed>::default();
        let mut w = ClientWorld::default();
        let key = UnitKey::new(PLAYER, 1);
        let mut player = unit(PLAYER, Some((0x1241, 0x11C4)));
        player.seed = Some((0x6AC6_935F, 0));
        w.units.insert(key, player);
        w.server_ticks = 3;
        w.start_shake(6, 4000, 10000, 4000);
        // No local player yet: no seed, the shake is unresolved.
        w.server_ticks = 103;
        assert!(frame_shake(&w, &mut feed).is_err());
        w.local_player = Some(key);
        let shake = Shake::start(6, 4000, 10000, 4000).unwrap();
        let mut seed = Seed::new(0x6AC6_935F, 0);
        let a = shake.amplitude(Shake::time_of(100)).unwrap();
        assert_ne!(a, 0);
        assert_eq!(
            frame_shake(&w, &mut feed).unwrap(),
            shake_offsets(a, &mut seed)
        );
        // The next frame goes on from the stepped seed; the model's copy
        // is not touched.
        w.server_ticks = 104;
        let a = shake.amplitude(Shake::time_of(101)).unwrap();
        assert_eq!(
            frame_shake(&w, &mut feed).unwrap(),
            shake_offsets(a, &mut seed)
        );
        assert_eq!(feed.client_seed.lock().unwrap().seed().copied(), Some(seed));
        assert_eq!(w.local().unwrap().seed, Some((0x6AC6_935F, 0)));
    }

    #[test]
    fn open_mode_is_the_uis_once_handed_over() {
        let mut feed = ModelFeed::<NoFeed>::default();
        let w = ClientWorld::default();
        assert_eq!(
            feed.open_mode(&w).unwrap(),
            OpenMode::NONE,
            "no UI: the inner feed's answer"
        );
        feed.set_ui_open_mode(OpenMode::new(3).unwrap());
        assert_eq!(feed.open_mode(&w).unwrap().get(), 3);
        // A feed that ignores the UI keeps its own answer.
        let mut inner = NoFeed;
        inner.set_ui_open_mode(OpenMode::new(1).unwrap());
        assert_eq!(inner.open_mode(&w).unwrap(), OpenMode::NONE);
    }

    /// The pending hooks answer "nothing" through the app's feed, never a
    /// guess.
    #[test]
    fn pending_hooks_answer_nothing() {
        let mut feed = ModelFeed::<NoFeed>::default();
        let w = ClientWorld::default();
        assert!(w.active_rooms.is_none());
        assert!(feed.light(&w).unwrap().is_none());
        assert!(feed.weather_frame(&w).unwrap().is_none());
        assert!(feed.near_rooms(&w).unwrap().is_none());
        assert!(feed
            .map_tiles(
                &w,
                &ViewAssets::new(crate::app::play::unspecified_palette())
            )
            .unwrap()
            .is_empty());
        assert!(feed.player_seed(&w).is_err());
        assert_eq!(PENDING.len(), 5);
    }

    #[test]
    fn unit_facts_come_from_the_tables_once_set() {
        let mut feed = ModelFeed::<NoFeed>::default();
        let w = ClientWorld::default();
        let mut u = unit(MONSTER, None);
        u.class = 1;
        u.mode = 12;
        assert!(feed.unit_facts(&w, &u).is_err());
        feed.set_unit_fact_tables(UnitFactTables {
            unflat_dead: vec![Some(false), Some(true)],
            ..UnitFactTables::default()
        });
        let f = feed.unit_facts(&w, &u).unwrap();
        assert!(f.unflat_dead && !crate::rules::draw_order::is_flat(&f));
        u.class = 0;
        assert!(crate::rules::draw_order::is_flat(
            &feed.unit_facts(&w, &u).unwrap()
        ));
    }

    #[test]
    fn unit_positions_by_path_kind() {
        let feed = ModelFeed::<NoFeed>::default();
        assert_eq!(
            feed.unit_position(&unit(MONSTER, Some((0x121A, 0x11B9))))
                .unwrap(),
            UnitPosition::Moving {
                x16: 0x121A_8000,
                y16: 0x11B9_8000
            }
        );
        assert_eq!(
            feed.unit_position(&unit(OBJECT, Some((0x1214, 0x11C0))))
                .unwrap(),
            UnitPosition::Static {
                sx: 0x1214,
                sy: 0x11C0
            }
        );
        assert!(feed.unit_position(&unit(ITEM, None)).is_err());
    }
    #[test]
    fn preview_objects_take_their_table_offset() {
        let mut preview = Preview::default();
        preview.object_offsets = std::sync::Arc::new([(442, (0, -130))].into_iter().collect());
        let mut feed = ModelFeed::<NoFeed>::default().with_preview(preview);
        let pose = UnitPose {
            sub_frame: 0,
            cof: crate::assets::path::CanonicalPath::new(
                "data\\global\\objects\\2z\\cof\\2znuhth.cof",
            )
            .unwrap(),
            dir: 0,
            frame: 0,
            dir64: 0,
        };
        let mut chandelier = unit(OBJECT, Some((5076, 5016)));
        chandelier.class = 442;
        assert_eq!(feed.unit_offset(&chandelier, &pose).unwrap(), (0, -130));
        // Its shadow takes it too: the draw adds it once before the
        // composite.
        assert_eq!(
            feed.unit_shadow_offset(&chandelier, &pose).unwrap(),
            (0, -130)
        );
        // Another class, or a monster of class 442: no table offset.
        let mut other = chandelier.clone();
        other.class = 441;
        assert_eq!(feed.unit_offset(&other, &pose).unwrap(), (0, 0));
        let mut monster = unit(MONSTER, Some((5076, 5016)));
        monster.class = 442;
        assert_eq!(feed.unit_offset(&monster, &pose).unwrap(), (0, 0));
        // The skill-move offset adds on top.
        feed.motion_offsets.insert(chandelier.key, (3, 4));
        assert_eq!(feed.unit_offset(&chandelier, &pose).unwrap(), (3, -126));
    }
}
