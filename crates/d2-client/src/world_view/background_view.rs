//! Pass 1 of the world draw in the play preview (draw order row 1): the
//! level background of Arreat Summit (level 120) through
//! [`Backgrounds::pass1`], its cels (`data\global\ui\summit01`,
//! `cloud01`) read from the user's archives on first use and drawn at
//! their screen positions with their light and draw mode through the
//! act's blend tables ([`cel_ops`]).
//!
//! Level 74 (the Arcane Sanctuary stars) is drawn too: the star tick
//! `last` is 0 before the first call, the star seed is
//! `init_low(time_value(time + GetTickCount + shake start))`
//! ([`stars_seed_at`]); the summit's is
//! `init_low(time_value(time + 2 * GetTickCount))`
//! ([`summit_seed_at`]). Both backgrounds initialise once per process.
//! Their layouts are time-seeded in the base game too, so only a capture that
//! records can compare them.
//!
//! The star lines are one-pixel items of pass 1 (palette index = the star
//! colour, opaque: `alpha` 0xFF) plus a [`SkyDraw::Line`] each in
//! `WorldFrame::sky`, which the facts export writes as `DrawLine` rows.
//!
//! A replay of a recorded capture pins the star seed
//! ([`BackgroundView::with_stars_seed`]) and the recorder's clock
//! ([`BackgroundView::with_replay_clock`]): the recorded the base game runs every
//! call more than 40 ms after the last, so each call moves the stars once.
//!
//! `D2RUST's own, unverified`:
//! - the screen-shake start tick of the star seed is 0 until a caller
//!   sets it ([`BackgroundView::set_shake_start`]); the feed keeps shake
//!   starts as server ticks, not `GetTickCount` values;
//! - the cel light byte is the low byte of the light argument (−1 →
//!   0xFF unlit, `0xDDDDDDDD` → 0xDD; takes one byte);
//! - without the act's shade tables a mode-5 cel draws opaque and a
//!   mode-3 cel is not drawn;
//! - a file no archive holds is logged once and its cels are not drawn.

use std::collections::BTreeMap;
use std::sync::Arc;

use d2_sim::rng::Seed;

use crate::assets::path::{CanonicalPath, FileSource};
use crate::bridge::world::ClientWorld;
use crate::frames::{FramePart, FrameSet, FrameSetKey, IndexFrame};
use crate::rules::blend::cel_ops;
use crate::rules::draw_order::background::{
    BackgroundDraw, BackgroundFrame, Backgrounds, ARCANE_SANCTUARY, ARREAT_SUMMIT,
};
use crate::rules::draw_order::weather::SkyDraw;
use crate::rules::placement::draw_position;
use crate::scene::{BlendOp, DrawItem, DrawKey, ItemTag, ShadeChain};

use super::{ViewAssets, WorldFrame};

/// Draw mode 5: opaque.
const MODE_OPAQUE: u8 = 5;

/// The level backgrounds' layer of the world view: the kept state of
/// (first-call flags, clouds), the file source and the cel files
/// loaded so far.
pub struct BackgroundView {
    source: Arc<dyn FileSource>,
    state: Backgrounds,
    /// The recorded / host seed of the summit's first use.
    summit_seed: Option<Seed>,
    /// The recorded seed of the stars' first use (`None`: the host
    /// clock's, [`stars_seed_at`]).
    stars_seed: Option<Seed>,
    /// The last screen-shake start tick (0 if none).
    shake_start: u32,
    /// Replay of a recorded capture: the clock advances 41 ms per call.
    replay_clock: bool,
    calls: u32,
    files: BTreeMap<&'static str, Option<FrameSetKey>>,
}

impl std::fmt::Debug for BackgroundView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundView")
            .field("state", &self.state)
            .field("summit_seed", &self.summit_seed)
            .finish_non_exhaustive()
    }
}

/// The host clock as `(time, GetTickCount)`: the Unix seconds and the
/// milliseconds of the same clock.
pub fn host_clock() -> (u32, u32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (now.as_secs() as u32, now.as_millis() as u32)
}

/// The summit's seed, `init_low(time_value(x))` with x =
/// `time + GetTickCount + GetTickCount` (the call's `v` is the tick
/// count).
pub fn summit_seed_at(time: u32, tick: u32) -> Seed {
    Seed::init_low(d2_sim::rng::time_value(
        time.wrapping_add(tick).wrapping_add(tick),
    ))
}

/// The stars' seed, `init_low(time_value(time +
/// GetTickCount + shake_start))`.
pub fn stars_seed_at(time: u32, tick: u32, shake_start: u32) -> Seed {
    Seed::init_low(d2_sim::rng::time_value(
        time.wrapping_add(tick).wrapping_add(shake_start),
    ))
}

/// The summit's first-use seed from the host clock.
pub fn summit_seed_now() -> Seed {
    let (t, tick) = host_clock();
    summit_seed_at(t, tick)
}

impl BackgroundView {
    /// A layer over `source`; `summit_seed` is the seed of level 120's
    /// first use (`None`: the background is not drawn, needs it).
    pub fn new(source: Arc<dyn FileSource>, summit_seed: Option<Seed>) -> Self {
        BackgroundView {
            source,
            state: Backgrounds::default(),
            summit_seed,
            stars_seed: None,
            shake_start: 0,
            replay_clock: false,
            calls: 0,
            files: BTreeMap::new(),
        }
    }

    /// Fixes the stars' first-use seed (a recorded one); by default the
    /// host clock's at first use.
    pub fn with_stars_seed(mut self, seed: Seed) -> Self {
        self.stars_seed = Some(seed);
        self
    }

    /// Replays a recorded capture's clock: every call is 41 ms after the
    /// last, so the stars move once per call.
    pub fn with_replay_clock(mut self) -> Self {
        self.replay_clock = true;
        self
    }

    /// The last screen-shake start tick, read by the stars' first-use
    /// seed (0 when none started).
    pub fn set_shake_start(&mut self, tick: u32) {
        self.shake_start = tick;
    }

    /// The kept state.
    pub fn state(&self) -> &Backgrounds {
        &self.state
    }

    fn file_key(
        &mut self,
        file: &'static str,
        assets: &mut ViewAssets,
        log: &mut Vec<String>,
    ) -> Option<FrameSetKey> {
        if let Some(k) = self.files.get(file) {
            return k.clone();
        }
        let loaded = load(self.source.as_ref(), file, assets);
        if let Err(e) = &loaded {
            log.push(format!("level background (D2RUST's own, unverified): {e}"));
        }
        let key = loaded.ok();
        self.files.insert(file, key.clone());
        key
    }

    /// Pass 1 of this frame: when the local
    /// player's level is 120 and the frame has a camera, the background draws
    /// added to `frame` (re-sorted by key) at their screen positions.
    /// `player_x` is the local player's client x (the frame anchor's).
    /// Open mode 3 draws no world where the side panels cover the frame
    /// (`FrameSize::draws_world`). Never fails the frame; returns log lines.
    pub fn add_to_frame(
        &mut self,
        world: &ClientWorld,
        open_mode: u8,
        player_x: i32,
        assets: &mut ViewAssets,
        frame: &mut WorldFrame,
    ) -> Vec<String> {
        let mut log = Vec::new();
        let level = world.player_level().map_or(0, u32::from);
        let mode = crate::rules::camera::OpenMode::new(open_mode).unwrap_or_default();
        let world_drawn = crate::rules::camera::FrameSize::play().draws_world(mode);
        if !matches!(level, ARREAT_SUMMIT | ARCANE_SANCTUARY) || !world_drawn {
            return log;
        }
        let Some(cam) = frame.camera else {
            return log;
        };
        let (time, mut tick) = host_clock();
        if self.replay_clock {
            self.calls += 1;
            tick = self
                .calls
                .wrapping_mul(super::background_view::REPLAY_STEP_MS);
        }
        let f = BackgroundFrame {
            level,
            w: cam.size.width,
            h: cam.size.height,
            resolution_mode: cam.size.resolution_mode(),
            // Read by level 74 only.
            now: tick,
            exiting: world.exit_requested,
            player_x,
            stars_seed: Some(
                self.stars_seed
                    .unwrap_or_else(|| stars_seed_at(time, tick, self.shake_start)),
            ),
            summit_seed: self.summit_seed,
            // The star tick is 0 before the first call.
            stars_last: Some(0),
            palette: Some(&assets.palette),
        };
        let items = match self.state.pass1(&f) {
            Ok(items) => items,
            Err(e) => {
                log.push(format!("level background: {e}"));
                return log;
            }
        };
        let mut added = false;
        for it in items {
            let (file, index, x, y, light, draw_mode) = match it.draw {
                BackgroundDraw::Cel {
                    file,
                    frame,
                    x,
                    y,
                    light,
                    draw_mode,
                } => (file, frame, x, y, light, draw_mode),
                BackgroundDraw::Line {
                    x0,
                    y0,
                    x1,
                    y1,
                    color,
                    alpha,
                } => {
                    match star_pixel_item(assets, &it.key, x0, y0, color) {
                        Ok(item) => {
                            frame.items.push(item);
                            frame.sky.push(SkyDraw::Line {
                                x0,
                                y0,
                                x1,
                                y1,
                                color,
                                alpha,
                            });
                            added = true;
                        }
                        Err(e) => log.push(format!("level background stars: {e}")),
                    }
                    continue;
                }
            };
            let Some(set) = self.file_key(file, assets, &mut log) else {
                continue;
            };
            let index = index as usize;
            let (Ok(id), Ok(image)) = (assets.id(&set, index), assets.frame(&set, index)) else {
                continue;
            };
            let (shade, blend) = match &assets.shades {
                Some(t) => cel_ops(t, draw_mode, None, light as u8),
                None if draw_mode == MODE_OPAQUE => (ShadeChain::EMPTY, BlendOp::Opaque),
                None => continue,
            };
            let (px, py) = draw_position(image, x, y);
            let Ok(key) = DrawKey::new(it.key.pass, it.key.major, it.key.minor, 0) else {
                continue;
            };
            let mut item = DrawItem::new(id, px, py);
            item.key = key;
            item.shade = shade;
            item.blend = blend;
            item.tag = ItemTag::None;
            frame.items.push(item);
            added = true;
        }
        if added {
            crate::scene::order(&mut frame.items);
        }
        log
    }
}

/// The replay clock's step per call, in ms (> 40, `STAR_TICK_MS`).
pub const REPLAY_STEP_MS: u32 = 41;

/// The frame-set path of the star pixels: frame `i` is one opaque pixel
/// of palette index `i` ([`super::weather_view::is_sky_call_path`]).
pub const STAR_PIXEL_PATH: &str = "d2rust/background/star-pixel";

fn star_pixel_item(
    assets: &mut ViewAssets,
    key: &crate::rules::draw_order::OrderKey,
    x: i32,
    y: i32,
    color: u8,
) -> Result<DrawItem, String> {
    let set = FrameSetKey::new(STAR_PIXEL_PATH, FramePart::Tile(0)).map_err(|e| e.to_string())?;
    if !assets.frames.contains(&set) {
        let frames = (0..=255u8)
            .map(|i| IndexFrame::new(1, 1, 0, 0, vec![i]).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        assets
            .frames
            .insert(set.clone(), FrameSet { frames })
            .map_err(|e| e.to_string())?;
    }
    let id = assets
        .id(&set, usize::from(color))
        .map_err(|e| e.to_string())?;
    let mut item = DrawItem::new(id, x, y);
    item.key = DrawKey::new(key.pass, key.major, key.minor.min(DrawKey::MINOR_MAX), 0)
        .map_err(|e| e.to_string())?;
    Ok(item)
}

/// Reads `file` (a background cel file, no extension) as a DC6 into the frame
/// store, direction 0.
fn load(
    source: &dyn FileSource,
    file: &str,
    assets: &mut ViewAssets,
) -> Result<FrameSetKey, String> {
    let path = format!("{file}.dc6");
    let canon = CanonicalPath::new(&path).map_err(|e| format!("{path}: {e}"))?;
    let key = FrameSetKey::new(canon.as_str(), FramePart::Dir(0)).map_err(|e| e.to_string())?;
    if assets.frames.contains(&key) {
        return Ok(key);
    }
    let dc6 = crate::assets::path::read_dc6(source, &path)
        .ok_or_else(|| format!("{path}: in no archive"))?
        .map_err(|e| format!("{path}: {e}"))?;
    let set = FrameSet::from_dc6(&dc6, 0).map_err(|e| format!("{path}: {e}"))?;
    assets
        .frames
        .insert(key.clone(), set)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

#[cfg(test)]
#[path = "background_view_tests.rs"]
mod tests;
