//! Weather in the play preview: the state the base game keeps
//! ([`Weather`], the floor pass context, the local player's seed), its
//! once-per-update step, and the art of passes 4 and 9 — the splash and
//! bubble cels (`UncompOverlays\Rain1–4`, `bubble1–4`), the particle
//! lines and the lightning flash — turned into scene [`DrawItem`]s.
//!
//! Fills, each `D2RUST's own, unverified` (the model does not hold the input):
//! - the weather seed is the local player's client seed, shared with the
//!   sound draws (the model's steps replayed
//!   first); `Seed::init_low(local player guid)` only while the
//!   model holds none (measured: the rain at a rain start depends on the seed);
//! - the client update count is the server tick; a tick whose draw was
//!   skipped is replayed so that every update steps the weather once
//!   (the base game draws every loop pass, measured on two rain scenes);
//! - the local player's 0x15 placements (`ClientWorld::local_places`)
//!   empty the splash and bubble pools, and the client push-out's
//!   placement does each update it applies; a placement runs
//!   its level entry with (flag) before the frame's act load
//!   (measured: the draws sit in the receive of the base game, here at
//!   the next weather step);
//! - the camera delta is the change of the camera's unit origin between
//!   frames; the video mode 1 (colors of the snow tables); the frame rate
//!   is 25 (≥ 10: the flash draws). The day period is the client
//!   environment record's;
//! - a level change calls the level entry with (0);
//! - lightning is never started: its only caller is an open question.
//!
//! Lines are drawn one pixel per item (a 1×1 frame of index 1 mapped to
//! the line color); the flash is a rectangle
//! frame of the clipped size, mode 5 (opaque).

use crate::audio::driver::{replay_steps, ClientSeed, SoundLink, WeatherSound};
use crate::rules::draw_order::source::SeedCommit;
use crate::rules::draw_order::weather::ThunderSound;
use std::sync::Arc;
use std::sync::Mutex;

use d2_sim::rng::Seed;

use crate::assets::path::{CanonicalPath, FileSource};
use crate::bridge::world::{ClientWorld, LevelRow};
use crate::frames::{FramePart, FrameSet, FrameSetKey, IndexFrame};
use crate::rules::blend::{cel_ops, color_row, gdi_line_pixels, gdi_rectangle};
use crate::rules::camera::{Camera, FrameSize, OpenMode};
use crate::rules::draw_order::sky::{SkyFrame, SkyPasses};
use crate::rules::draw_order::source::WeatherFrame;
use crate::rules::draw_order::weather::{
    span, ActResources, ColorTables, EnvPool, FloorContext, LevelWeather, LocalPlayer, PoolDraw,
    SkyDraw, UpdateInput, Weather, BOTTOM_MARGIN,
};
use crate::rules::placement;
use crate::rules::shading::ShadeTables;
use crate::rules::UnitPosition;
use crate::scene::{DrawItem, DrawKey, MapId, Rect, ShadeChain};

use super::{ViewAssets, ViewError};

/// The pass numbers of the draw order.
const PASS_POOLS: u32 = crate::scene::order::pass::UNIDENTIFIED_4;
const PASS_SKY: u32 = crate::scene::order::pass::UNIDENTIFIED_9;

/// The frame set of the 1×1 pixel (index 1) every line pixel draws.
const PIXEL_PATH: &str = "d2rust/weather/pixel";

/// The act's day period:
/// the client environment record's period type (+0x04), 0 without a
/// record.
fn day_period(world: &ClientWorld) -> u8 {
    world
        .environment
        .map_or(0, |e| u8::try_from(e.kind).unwrap_or(0))
}
/// `D2RUST's own, unverified`: the video mode.
const VIDEO_MODE: u32 = 1;
/// `D2RUST's own, unverified`: frames counted in the last second.
const FRAME_RATE: u32 = 25;

fn unresolved(what: &'static str, message: String) -> ViewError {
    ViewError::Unresolved {
        what,
        spec: "render/draw-order-2.md",
        message,
    }
}

fn pixel_key() -> FrameSetKey {
    FrameSetKey::new(PIXEL_PATH, FramePart::Tile(0)).expect("canonical")
}

/// The clipped size of the flash rectangle of `frame` at `mode`
/// (columns `L … R − 1` clamped to `W − 1`, rows
/// `0 … H − 48`).
pub fn flash_size(frame: FrameSize, mode: OpenMode) -> (u32, u32) {
    let (l, r) = span(frame, mode);
    let w = r.min(frame.width - 1) - l;
    let h = (frame.height - BOTTOM_MARGIN).min(frame.height - 1);
    (w.max(0) as u32, h.max(0) as u32)
}

/// Whether a frame-set path is one of pass 9's call drawings (a line
/// pixel or the flash rectangle): the facts export writes the calls of
/// [`super::WorldFrame::sky`] in their place.
pub fn is_sky_call_path(path: &str) -> bool {
    path == PIXEL_PATH
        || path == super::background_view::STAR_PIXEL_PATH
        || path.starts_with("d2rust/weather/flash/")
}

fn flash_key(frame: FrameSize, mode: OpenMode) -> FrameSetKey {
    let (w, h) = flash_size(frame, mode);
    FrameSetKey::new(format!("d2rust/weather/flash/{w}x{h}"), FramePart::Tile(0))
        .expect("canonical")
}

/// The frame-set key of kind `kind` of `pool`'s cel file.
fn cel_key(pool: EnvPool, kind: u8) -> Result<FrameSetKey, String> {
    let path = CanonicalPath::new(&format!("{}.dc6", pool.cel_file(kind)))
        .map_err(|e| format!("{}: {e}", pool.cel_file(kind)))?;
    FrameSetKey::new(path.as_str(), FramePart::Dir(0)).map_err(|e| e.to_string())
}

/// The weather of the play preview: state, act resources and the
/// per-frame step.
#[derive(Clone)]
pub struct WeatherView {
    source: Option<Arc<dyn FileSource>>,
    weather: Weather,
    floors: FloorContext,
    seed: Seed,
    seeded: bool,
    /// The server tick whose update the last [`WeatherView::prepare`] ran:
    /// the model's steps a later prepare of the same tick syncs come
    /// after the frame's draws ([`WeatherView::frame`]).
    prepared_tick: Option<u64>,
    /// The local player's client seed, shared with the sound draws;
    /// `seed` is the working copy for a frame
    /// and the fallback while the model holds none.
    shared: Arc<Mutex<ClientSeed>>,
    level: Option<LevelWeather>,
    update_count: u32,
    /// Whether a client update with no drawn frame is replayed at the
    /// next frame; off when a check run follows the recorded
    /// frame schedule, where the base game's weather ran only in drawn frames.
    replay_skipped: bool,
    /// How many of `ClientWorld::local_places` were replayed.
    places_seen: usize,
    /// The client push-out places the local player this
    /// update (placement clears).
    local_push: bool,
    origin: Option<(i32, i32)>,
    color_maps: Option<Vec<MapId>>,
    /// Why the weather is off (logged once by the caller).
    failure: Option<String>,
    /// The sound layer the thunder step requests from.
    thunder: Option<SoundLink>,
    /// Passes 4 / 9 a pass whose draw waited for the GPU rolled, with the
    /// server tick they belong to.
    kept: Option<(u64, SkyPasses)>,
}

impl std::fmt::Debug for WeatherView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeatherView")
            .field("level", &self.level)
            .field("failure", &self.failure)
            .finish()
    }
}

impl WeatherView {
    /// The current level's `Rain` and `Mud` flags (`None`: no level yet).
    pub fn level_flags(&self) -> Option<(bool, bool)> {
        self.level.map(|l| (l.rain, l.mud))
    }

    /// Whether the world draw can roll the client seed this frame: any
    /// level the weather runs in (the floor pass, passes 4 / 9
    ///–; a pass without rain or water rolls nothing).
    pub fn world_rolls(&self) -> bool {
        self.level.is_some()
    }

    /// Keeps the passes 4 / 9 rolled ahead for `tick`.
    pub fn keep_sky(&mut self, tick: u64, sky: SkyPasses) {
        self.kept = Some((tick, sky));
    }

    /// The passes kept for `tick`, once; others are dropped.
    pub fn take_kept_sky(&mut self, tick: u64) -> Option<SkyPasses> {
        self.kept.take().filter(|(t, _)| *t == tick).map(|(_, s)| s)
    }

    /// Weather with the overlay cels read from `source`.
    pub fn new(source: Option<Arc<dyn FileSource>>) -> Self {
        WeatherView {
            source,
            weather: Weather::new(),
            floors: FloorContext::default(),
            seed: Seed::init(),
            seeded: false,
            prepared_tick: None,
            shared: Arc::default(),
            level: None,
            update_count: 0,
            replay_skipped: true,
            places_seen: 0,
            local_push: false,
            origin: None,
            color_maps: None,
            failure: None,
            thunder: None,
            kept: None,
        }
    }

    /// Follows a recorded frame schedule: no replay of client updates
    /// without a drawn frame.
    pub fn follow_frame_schedule(&mut self) {
        self.replay_skipped = false;
    }

    /// The local player's draw-phase client seed this view steps, shared
    /// with the sound draws and the shake (Randomness).
    pub fn client_seed(&self) -> Arc<Mutex<ClientSeed>> {
        self.shared.clone()
    }

    /// The sound layer for the thunder step's request of sound 202.
    pub fn with_sound(mut self, link: SoundLink) -> Self {
        self.shared = link.client_seed();
        self.thunder = Some(link);
        self
    }

    pub fn weather(&self) -> &Weather {
        &self.weather
    }

    pub fn weather_mut(&mut self) -> &mut Weather {
        &mut self.weather
    }

    /// Whether the push-out (`walk::local_push_out`) places the local
    /// player in this frame's client update, set before [`Self::prepare`].
    pub fn set_local_push(&mut self, pushed: bool) {
        self.local_push = pushed;
    }

    /// Why the weather stopped, once.
    pub fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }

    fn fail(&mut self, message: String) {
        self.level = None;
        self.failure = Some(message);
    }

    /// The frame's weather state for the floor pass and passes 4 / 9
    /// (`None` before the first level or after a failure).
    pub fn frame(&mut self) -> Option<WeatherFrame<'_>> {
        let level = self.level?;
        let shared = self.shared.clone();
        // The model's steps since this frame's update come after the
        // frame's draws: the base game's per-frame step on the
        // player seed follows the floor pass. A seed no replay reaches
        // (a new one) is taken before the draws.
        let mut after = 0;
        if let Some(s) = shared.lock().unwrap_or_else(|e| e.into_inner()).seed() {
            if *s != self.seed {
                match replay_steps((self.seed.lo, self.seed.hi), (s.lo, s.hi)) {
                    Some(n) => after = n,
                    None => self.seed = *s,
                }
            }
        }
        Some(WeatherFrame {
            weather: &mut self.weather,
            floors: &mut self.floors,
            seed: &mut self.seed,
            update_count: self.update_count,
            mud: level.mud,
            sky: Some(SkyFrame {
                frame: FrameSize::play(),
                mode: OpenMode::NONE,
                shift_x: 0,
                level,
                frame_rate: FRAME_RATE,
                low_quality: false,
            }),
            thunder: self
                .thunder
                .as_mut()
                .map(|t| t as &mut (dyn ThunderSound + Send + Sync)),
            commit: Some(SeedCommit(Box::new(move |s| {
                if let Some(m) = shared.lock().unwrap_or_else(|e| e.into_inner()).seed() {
                    *m = *s;
                    (0..after).for_each(|_| {
                        m.step();
                    });
                }
            }))),
        })
    }

    /// Before the frame's build: the level, the act load, the update step
    /// and the art the frame's draws need resident.
    pub fn prepare(
        &mut self,
        world: &ClientWorld,
        levels: Option<&[LevelRow]>,
        local_at: Option<(u32, u32)>,
        mode: OpenMode,
        assets: &mut ViewAssets,
    ) {
        // The update's draws step the shared client seed, after the
        // model's steps of the receive and the client update.
        // A later prepare of the same tick runs no update: the model's
        // steps it syncs stay on the shared seed for after the frame's
        // draws.
        let shared = self.shared.clone();
        let mut held = shared.lock().unwrap_or_else(|e| e.into_inner());
        held.sync(world);
        let again = self.prepared_tick == Some(world.server_ticks);
        self.prepared_tick = Some(world.server_ticks);
        if again {
            // no update to step: only the level's assets are kept resident
            drop(held);
            let seed = self.seed;
            self.prepare_draws(world, levels, local_at, mode, assets);
            self.seed = seed;
            return;
        }
        if let Some(s) = held.seed() {
            self.seed = *s;
            self.seeded = true;
        }
        drop(held);
        self.prepare_draws(world, levels, local_at, mode, assets);
        let mut held = shared.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = held.seed() {
            *m = self.seed;
        }
    }

    fn prepare_draws(
        &mut self,
        world: &ClientWorld,
        levels: Option<&[LevelRow]>,
        local_at: Option<(u32, u32)>,
        mode: OpenMode,
        assets: &mut ViewAssets,
    ) {
        // The rain rule's input (an open question): not
        // active unless this frame's update ran with a rain level.
        if let Some(l) = &self.thunder {
            l.set_weather(WeatherSound::default());
        }
        if self.failure.is_some() {
            return;
        }
        if let Err(m) = self.ensure_pixels(assets) {
            return self.fail(m);
        }
        let Some(local) = world.local() else {
            self.level = None;
            return;
        };
        let (Some(levels), Some(id)) = (levels, world.player_level()) else {
            self.level = None;
            return;
        };
        let Some(row) = levels.get(usize::from(id)) else {
            return self.fail(format!("level {id} past the Levels rows"));
        };
        let level = LevelWeather {
            level_id: u32::from(id),
            act: row.act,
            rain: row.rain,
            mud: row.mud,
        };
        if !self.seeded {
            self.seed = Seed::init_low(local.key.guid);
            self.seeded = true;
        }
        // The local player's placements (the level entry with (flag), received
        // before this frame's draw): the pools' clear, then the level entry
        // of a non-zero flag.
        while let Some(&flag) = world.local_places.get(self.places_seen) {
            self.places_seen += 1;
            self.weather.placement_clear();
            if flag != 0 {
                if let Err(e) = self
                    .weather
                    .level_entry(u32::from(flag), &mut self.seed, &level)
                {
                    return self.fail(e.to_string());
                }
            }
        }
        let previous = self.level;
        let r = self.enter(previous, level, assets);
        if let Err(m) = r {
            return self.fail(m);
        }
        self.level = Some(level);
        // The base game updates the weather once per client update, in the draw of
        // its loop pass; a pass whose draw was skipped here (the GPU still
        // busy) is replayed so that every client update steps it once
        // (measured: the base game draws every pass). The
        // count is the client update counter, one behind the
        // server frame: the frame drawn after server frame f records
        // `client_update` f − 1 (measured over a rain scene), and
        // the splash gate compares it with an absolute start of 0.
        let now = (world.server_ticks as u32).saturating_sub(1);
        let first = match previous {
            Some(_) if self.replay_skipped && self.update_count < now => self.update_count + 1,
            _ => now,
        };
        self.update_count = now;
        let delta = self.camera_delta(local_at, mode);
        for c in first..=now {
            // The push-out's placement runs in the client player update,
            // before the pass's weather update (placement clears).
            if self.local_push {
                self.weather.placement_clear();
            }
            let input = UpdateInput {
                update_count: c,
                frame: FrameSize::play(),
                camera_delta: if c == now { delta } else { (0, 0) },
                day_period: day_period(world),
                video_mode: VIDEO_MODE,
            };
            let player = LocalPlayer {
                seed: &mut self.seed,
                level,
            };
            if let Err(e) = self.weather.update(Some(player), &input) {
                return self.fail(e.to_string());
            }
        }
        if let Some(l) = &self.thunder {
            let active = !self.weather.snow_mode && level.rain;
            l.set_weather(WeatherSound::new(active, self.weather.intensity_256));
        }
        if self.weather.lightning_on {
            if let Err(m) = self.ensure_flash(mode, assets) {
                self.fail(m);
            }
        }
    }

    /// Act load on the first level and an act change;
    /// a level change inside the act makes no weather draw.
    fn enter(
        &mut self,
        previous: Option<LevelWeather>,
        level: LevelWeather,
        assets: &mut ViewAssets,
    ) -> Result<(), String> {
        let act_changed =
            previous.is_none_or(|p| p.act != level.act) || self.weather.resources().is_none();
        if act_changed {
            let resources = self.load_cels(assets)?;
            let player = LocalPlayer {
                seed: &mut self.seed,
                level,
            };
            return self
                .weather
                .act_load(player, resources)
                .map_err(|e| e.to_string());
        }
        // A level change inside the act re-checks nothing: the base game calls
        // only from the act load, a placement
        // with flag ≠ 0 and rain phase 1.
        Ok(())
    }

    /// The eight cel files resident (`Rain1–4`, `bubble1–4`) and the color
    /// tables of the frame palette.
    fn load_cels(&self, assets: &mut ViewAssets) -> Result<ActResources, String> {
        let mut counts = [[0u32; 4]; 2];
        for (i, pool) in [EnvPool::Splashes, EnvPool::Bubbles]
            .into_iter()
            .enumerate()
        {
            for kind in 0..4u8 {
                counts[i][usize::from(kind)] = self.load_cel(pool, kind, assets)?;
            }
        }
        Ok(ActResources {
            splash_frames: counts[0],
            bubble_frames: counts[1],
            colors: ColorTables::build(&assets.palette),
        })
    }

    fn load_cel(&self, pool: EnvPool, kind: u8, assets: &mut ViewAssets) -> Result<u32, String> {
        let key = cel_key(pool, kind)?;
        if assets.frames.contains(&key) {
            return Ok((0..)
                .take_while(|&i| assets.frames.id(&key, i).is_ok())
                .count() as u32);
        }
        let name = format!("{}.dc6", pool.cel_file(kind));
        let canon = CanonicalPath::new(&name).map_err(|e| format!("{name}: {e}"))?;
        let source = self.source.as_ref().ok_or("no archives")?;
        let dc6 = crate::assets::path::read_dc6(source.as_ref(), &canon.archive_name())
            .ok_or_else(|| format!("{name}: no archive holds it"))?
            .map_err(|e| format!("{name}: {e}"))?;
        let set = FrameSet::from_dc6(&dc6, 0).map_err(|e| format!("{name}: {e}"))?;
        let frames = set.frames.len() as u32;
        assets.frames.insert(key, set).map_err(|e| e.to_string())?;
        Ok(frames)
    }

    /// The pixel frame and one color map row per index, once.
    fn ensure_pixels(&mut self, assets: &mut ViewAssets) -> Result<(), String> {
        if self.color_maps.is_some() {
            return Ok(());
        }
        let key = pixel_key();
        if !assets.frames.contains(&key) {
            let frame = IndexFrame::new(1, 1, 0, 0, vec![1]).map_err(|e| e.to_string())?;
            assets
                .frames
                .insert(
                    key,
                    FrameSet {
                        frames: vec![frame],
                    },
                )
                .map_err(|e| e.to_string())?;
        }
        self.color_maps = Some(
            (0..=255u8)
                .map(|c| assets.maps.push(color_row(c)))
                .collect(),
        );
        Ok(())
    }

    fn ensure_flash(&self, mode: OpenMode, assets: &mut ViewAssets) -> Result<(), String> {
        let key = flash_key(FrameSize::play(), mode);
        if assets.frames.contains(&key) {
            return Ok(());
        }
        let (w, h) = flash_size(FrameSize::play(), mode);
        let frame = IndexFrame::new(w, h, 0, 0, vec![1; w as usize * h as usize])
            .map_err(|e| e.to_string())?;
        assets
            .frames
            .insert(
                key,
                FrameSet {
                    frames: vec![frame],
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// The change of the camera's unit origin since the last frame.
    fn camera_delta(&mut self, local_at: Option<(u32, u32)>, mode: OpenMode) -> (i32, i32) {
        let Some((x16, y16)) = local_at else {
            return (0, 0);
        };
        let player = UnitPosition::Moving { x16, y16 }.client();
        let cam = Camera::new(FrameSize::play(), mode, player, (0, 0)).unit;
        let now = (cam.x, cam.y);
        let delta = self.origin.map_or((0, 0), |o| (now.0 - o.0, now.1 - o.1));
        self.origin = Some(now);
        delta
    }

    /// The draw items of passes 4 and 9.
    pub fn items(
        &self,
        sky: &SkyPasses,
        mode: OpenMode,
        tables: Option<&ShadeTables>,
        assets: &ViewAssets,
    ) -> Result<Vec<DrawItem>, ViewError> {
        let mut out = Vec::new();
        if !sky.pools.is_empty() {
            let tables = tables.ok_or_else(|| {
                unresolved(
                    "weather cels",
                    "the act's shade tables are not resident (draw mode 3 needs the additive table)"
                        .into(),
                )
            })?;
            for d in &sky.pools {
                out.extend(self.pool_item(d, tables, assets)?);
            }
        }
        if !sky.sky.is_empty() {
            let maps = self.color_maps.as_ref().ok_or_else(|| {
                unresolved("weather lines", "the color maps are not resident".into())
            })?;
            let mut minor = 0u32;
            for d in &sky.sky {
                match *d {
                    SkyDraw::Flash { x0, y0, x1, y1 } => {
                        let tables = tables.ok_or_else(|| {
                            unresolved(
                                "lightning flash",
                                "the act's shade tables are not resident".into(),
                            )
                        })?;
                        let g = gdi_rectangle(
                            tables,
                            FrameSize::play(),
                            maps[usize::from(SkyDraw::FLASH_COLOR)],
                            x0,
                            y0,
                            x1,
                            y1,
                            SkyDraw::FLASH_DRAW_MODE,
                        )
                        .map_err(|e| unresolved("lightning flash", e.to_string()))?;
                        if let Some(g) = g {
                            let id = assets.id(&flash_key(FrameSize::play(), mode), 0)?;
                            let mut item = g.item(id);
                            item.key = sky_key(minor)?;
                            minor += 1;
                            out.push(item);
                        }
                    }
                    SkyDraw::Line {
                        x0,
                        y0,
                        x1,
                        y1,
                        color,
                        ..
                    } => {
                        let id = assets.id(&pixel_key(), 0)?;
                        let chain = ShadeChain::new(&[maps[usize::from(color)]]).expect("one map");
                        let pixels = gdi_line_pixels(x0, y0, x1, y1)
                            .map_err(|e| unresolved("weather line", e.to_string()))?;
                        for (x, y) in pixels {
                            if x < 0
                                || y < 0
                                || x >= FrameSize::play().width
                                || y >= FrameSize::play().height
                            {
                                continue;
                            }
                            let mut item = DrawItem::new(id, x, y);
                            item.shade = chain;
                            item.key = sky_key(minor)?;
                            minor = minor.saturating_add(1).min(DrawKey::MINOR_MAX);
                            out.push(item);
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// A splash or bubble cel: frame `d.frame` of the kind's file
    /// at (`x`, `y`), light −1 (unlit), draw mode 3, palette 0.
    fn pool_item(
        &self,
        d: &PoolDraw,
        tables: &ShadeTables,
        assets: &ViewAssets,
    ) -> Result<Option<DrawItem>, ViewError> {
        let key = cel_key(d.pool, d.kind).map_err(|m| unresolved("weather cel", m))?;
        let frame = assets.frame(&key, d.frame as usize)?;
        let id = assets.id(&key, d.frame as usize)?;
        let placed = placement::place(frame, d.x, d.y, Rect::world_clip());
        let Some(clip) = placed.clip else {
            return Ok(None);
        };
        let (shade, blend) = cel_ops(tables, PoolDraw::DRAW_MODE, None, 0xFF);
        let mut item = DrawItem::new(id, placed.x, placed.y);
        item.clip = clip;
        item.shade = shade;
        item.blend = blend;
        // `D2GFX_DrawCelContext` with draw mode 3 and a null palette.
        item.call = Some(crate::scene::CelArgs {
            mode: PoolDraw::DRAW_MODE,
            pal: crate::scene::CelPal::Null,
        });
        item.key = DrawKey::new(PASS_POOLS, d.pool.major(), d.slot as u32, 0)?;
        Ok(Some(item))
    }
}

fn sky_key(minor: u32) -> Result<DrawKey, ViewError> {
    Ok(DrawKey::new(PASS_SKY, 0, minor.min(DrawKey::MINOR_MAX), 0)?)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use d2_formats::palette::{Palette, Rgb};
    use test_fixtures::sprites::{dc6_file, Image};

    use super::*;
    use crate::bridge::drlg::DrlgRoomId;
    use crate::bridge::world::{ActiveRoom, PLAYER};
    use crate::bridge::{ClientUnit, UnitKey};
    use crate::rules::draw_order::weather::PoolRecord;
    use crate::world_view::tile_assets::TileAssets;

    struct Files(BTreeMap<String, Vec<u8>>);

    impl FileSource for Files {
        fn read_file(&self, name: &str) -> Option<Result<Vec<u8>, String>> {
            self.0.get(name).cloned().map(Ok)
        }
    }

    /// `Rain1–4` and `bubble1–4`, three frames each.
    fn overlay_files() -> Arc<dyn FileSource> {
        let img = |w: u32| Image {
            width: w,
            height: 4,
            x: -2,
            y: 3,
            pixels: vec![7; (w * 4) as usize],
        };
        let mut files = BTreeMap::new();
        for stem in ["rain", "bubble"] {
            for k in 1..=4 {
                let name = format!(r"data\global\uncompoverlays\{stem}{k}.dc6");
                files.insert(name, dc6_file(&[img(5), img(6), img(7)], 1, 3));
            }
        }
        Arc::new(Files(files))
    }

    fn assets() -> ViewAssets {
        let mut a = ViewAssets::new(Palette {
            colors: [Rgb::default(); 256],
        });
        let mut tiles = TileAssets::new(
            None,
            Some(std::array::from_fn(|_| {
                crate::world_view::tile_assets::tests::pl2()
            })),
        );
        tiles.ensure_shades(0, &mut a.maps).unwrap();
        a.shades = tiles.shades(0).copied();
        a
    }

    /// Level 2 (act 0) with `Rain`, level 3 without; the local player in
    /// a level-2 room.
    fn world() -> (ClientWorld, Vec<LevelRow>) {
        let mut w = ClientWorld::default();
        let key = UnitKey::new(PLAYER, 1);
        w.units.insert(key, ClientUnit::new(key));
        w.local_player = Some(key);
        let room = DrlgRoomId(5);
        w.room_units.place(key, Some(room));
        w.active_rooms = Some(vec![ActiveRoom {
            x0: 0,
            y0: 0,
            w: 40,
            h: 40,
            level: 2,
            room,
        }]);
        let mut levels = vec![LevelRow::default(); 4];
        levels[2].rain = true;
        (w, levels)
    }

    fn prepared(
        view: &mut WeatherView,
        w: &mut ClientWorld,
        levels: &[LevelRow],
        a: &mut ViewAssets,
        ticks: u64,
    ) {
        w.server_ticks = ticks;
        view.prepare(
            w,
            Some(levels),
            Some((0x100_0000, 0x100_0000)),
            OpenMode::NONE,
            a,
        );
        assert_eq!(view.take_failure(), None);
    }

    #[test]
    fn a_rain_level_produces_particle_lines_per_tick() {
        let (mut w, levels) = world();
        let mut a = assets();
        let mut view = WeatherView::new(Some(overlay_files()));
        let mut lines = 0;
        for tick in 1..=3_000u64 {
            prepared(&mut view, &mut w, &levels, &mut a, tick);
            let mut frame = view.frame().expect("a level with weather");
            let sky = frame.sky_passes().unwrap();
            drop(frame);
            let items = view
                .items(&sky, OpenMode::NONE, a.shades.as_ref(), &a)
                .unwrap();
            if !sky.sky.is_empty() {
                assert!(!items.is_empty(), "tick {tick}");
                assert!(items.iter().all(|i| i.key.pass() == PASS_SKY));
                lines += 1;
            }
        }
        assert!(lines > 0, "rain never drew in 3,000 ticks");
        assert!(view.weather().particles().live() > 0);
    }

    #[test]
    fn a_level_without_rain_draws_no_particles() {
        let (mut w, mut levels) = world();
        levels[2].rain = false;
        let mut a = assets();
        let mut view = WeatherView::new(Some(overlay_files()));
        for tick in 1..=500u64 {
            prepared(&mut view, &mut w, &levels, &mut a, tick);
        }
        assert_eq!(view.weather().particles().live(), 0);
    }

    #[test]
    fn a_level_change_inside_the_act_makes_no_seed_step() {
        let (mut w, mut levels) = world();
        levels[2].rain = false;
        let (mut stay, _) = world();
        let mut a = assets();
        let mut moved = WeatherView::new(Some(overlay_files()));
        let mut kept = WeatherView::new(Some(overlay_files()));
        prepared(&mut moved, &mut w, &levels, &mut a, 1);
        prepared(&mut kept, &mut stay, &levels, &mut a, 1);
        assert_eq!(moved.seed, kept.seed);
        if let Some(rooms) = w.active_rooms.as_mut() {
            rooms[0].level = 3;
        }
        for tick in 2..=4 {
            prepared(&mut moved, &mut w, &levels, &mut a, tick);
            prepared(&mut kept, &mut stay, &levels, &mut a, tick);
        }
        assert_eq!(moved.level.map(|l| l.level_id), Some(3));
        assert_eq!(moved.seed, kept.seed);
    }

    #[test]
    fn pass_9_flashes_when_lightning_is_on() {
        let (mut w, levels) = world();
        let mut a = assets();
        let mut view = WeatherView::new(Some(overlay_files()));
        prepared(&mut view, &mut w, &levels, &mut a, 1);
        view.weather_mut().start_lightning(false);
        prepared(&mut view, &mut w, &levels, &mut a, 1);
        let mut frame = view.frame().unwrap();
        let sky = frame.sky_passes().unwrap();
        assert_eq!(sky.sky.len(), 1);
        drop(frame);
        let items = view
            .items(&sky, OpenMode::NONE, a.shades.as_ref(), &a)
            .unwrap();
        assert_eq!(items.len(), 1);
        let it = items[0];
        assert_eq!((it.x, it.y), (0, 0));
        assert_eq!(it.key.pass(), 9);
        let (fw, fh) = flash_size(FrameSize::D2RUST, OpenMode::NONE);
        assert_eq!((fw, fh), (799, 553));
        let frame = a.frames.frame(it.frame).unwrap();
        assert_eq!((frame.width, frame.height), (799, 553));
    }

    #[test]
    fn pass_4_draws_the_pool_cels_unlit_in_mode_3() {
        let (mut w, levels) = world();
        let mut a = assets();
        let mut view = WeatherView::new(Some(overlay_files()));
        prepared(&mut view, &mut w, &levels, &mut a, 1);
        let (_, splashes, bubbles) = view.weather_mut().pools_mut();
        splashes.alloc(PoolRecord {
            x: 100,
            y: 200,
            kind: 2,
            frame: 1,
            countdown: 2,
        });
        bubbles.alloc(PoolRecord {
            x: 300,
            y: 100,
            kind: 2,
            frame: 0,
            countdown: 2,
        });
        let mut frame = view.frame().unwrap();
        let sky = frame.sky_passes().unwrap();
        assert_eq!(sky.pools.len(), 2);
        drop(frame);
        let items = view
            .items(&sky, OpenMode::NONE, a.shades.as_ref(), &a)
            .unwrap();
        assert_eq!(items.len(), 2);
        // Splashes first (major 0), then bubbles (major 1), keyed at pass 4.
        assert_eq!((items[0].key.pass(), items[0].key.major()), (4, 0));
        assert_eq!((items[1].key.pass(), items[1].key.major()), (4, 1));
        // The DC6 frame is bottom-up: top = Y + y_off − height + 1.
        assert_eq!((items[0].x, items[0].y), (100 - 2, 200 + 3 - 4 + 1));
        // Draw mode 3 is the additive table.
        assert!(items[0].blend.table().is_some());
        // Without the act's tables the cels cannot be made: an error.
        assert!(view.items(&sky, OpenMode::NONE, None, &a).is_err());
    }

    #[test]
    fn placements_and_the_push_out_empty_the_pools() {
        let (mut w, mut levels) = world();
        // No rain, no mud: only the placements clear the pools.
        levels[2].rain = false;
        let mut a = assets();
        let mut view = WeatherView::new(Some(overlay_files()));
        prepared(&mut view, &mut w, &levels, &mut a, 1);
        let fill = |view: &mut WeatherView| {
            let (_, splashes, bubbles) = view.weather_mut().pools_mut();
            let rec = PoolRecord {
                x: 100,
                y: 200,
                kind: 2,
                frame: 0,
                countdown: 50,
            };
            splashes.alloc(rec);
            bubbles.alloc(rec);
        };
        let live = |view: &WeatherView| {
            let wx = view.weather();
            (wx.splashes().live(), wx.bubbles().live())
        };
        fill(&mut view);
        prepared(&mut view, &mut w, &levels, &mut a, 2);
        assert_eq!(live(&view), (1, 1), "no placement: kept");
        // A flag-0 placement clears both pools too.
        w.local_places.push(0);
        prepared(&mut view, &mut w, &levels, &mut a, 3);
        assert_eq!(live(&view), (0, 0));
        // The push-out clears them every update it applies.
        fill(&mut view);
        view.set_local_push(true);
        prepared(&mut view, &mut w, &levels, &mut a, 4);
        assert_eq!(live(&view), (0, 0));
        fill(&mut view);
        view.set_local_push(false);
        prepared(&mut view, &mut w, &levels, &mut a, 5);
        assert_eq!(live(&view), (1, 1));
    }
}
