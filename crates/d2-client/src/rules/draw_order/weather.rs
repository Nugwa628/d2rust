//! Weather: rain, snow, lightning, and the splashes and bubbles of water
//! floors.
//!
//! Only the GDI path (perspective 0) is modelled. Part of the particle
//! move is not pinned down yet: where it is unknown, moving live
//! particles returns [`WeatherError::Open`] instead of a
//! guessed value. The shape values, the snow spawn and color, the snow
//! line table and the falling-drop vector are exact.

use d2_formats::palette::Palette;
use d2_sim::rng::Seed;

use crate::rules::camera::{FrameSize, OpenMode};
use crate::rules::lighting::overrides::sine_table;
use crate::rules::shading::nearest;

/// Particle pool slots.
pub const PARTICLE_SLOTS: usize = 256;
/// Splash pool slots.
pub const SPLASH_SLOTS: usize = 512;
/// Bubble pool slots.
pub const BUBBLE_SLOTS: usize = 128;

/// Rain cycle lengths in client updates, per phase (dry, rising, steady,
/// falling): a phase lasts `CYCLE_MIN[p] + roll(CYCLE_N[p])` updates. Acts
/// III and V rain three times as long.
pub const CYCLE_MIN: [i32; 4] = [7_500, 250, 3_000, 125];
pub const CYCLE_N: [i32; 4] = [7_500, 250, 3_000, 50];
/// Snow wind goal roll `(min, n)` at client start.
pub const SNOW_GOAL_START: (i32, i32) = (42, 170);
/// Sound of a thunder strike.
pub const THUNDER_SOUND: u16 = 202;
/// Passes 4 and 9 stop this many pixels above the frame's bottom edge
/// (the control panel): they draw only `y < H − 47`.
pub const BOTTOM_MARGIN: i32 = 47;
/// Snow particle alpha by day period 0–3.
pub const SNOW_ALPHA: [u8; 4] = [200, 160, 80, 160];
/// The snow line table: 9 entries of (x0, y0,
/// x1, y1) offsets; size `s` draws `T[s]` and `T[s + 1]`.
pub const SNOW_LINES: [[i32; 4]; 9] = [
    [0, 0, 0, 1],
    [-1, -1, 0, 0],
    [0, -1, 0, 1],
    [1, 0, -1, 1],
    [1, 0, 0, 1],
    [2, -1, -1, 1],
    [1, 1, -2, -1],
    [2, 1, -2, -1],
    [2, -1, -2, 2],
];

/// A fatal or unresolved condition of the weather code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WeatherError {
    /// A fatal error of the weather code (`0x547`, `0x573`, `0x390`,
    /// `0x356`, `0x15B`, `0x12A`).
    #[error("weather fatal 0x{0:X} (render/draw-order-2.md §11)")]
    Fatal(u16),
    /// A rule the weather design leaves open.
    #[error("render/draw-order-2.md open question {question}: {what}")]
    Open { question: u8, what: &'static str },
    /// A falling drop below its ground with `v` = 0: the
    /// original's C division `(g − y) × u / v` divides by zero (crash).
    #[error("falling drop at ({x}, {y}) with v = 0 below its ground: division by zero (render/draw-order-2.md §11.7 r3)")]
    DropDivideByZero { x: i32, y: i32 },
    /// The act resources are not loaded.
    #[error("weather used before act load (render/draw-order-2.md §11.8)")]
    NotLoaded,
}

/// A fixed-size slot pool: first free slot from the
/// lowest index, highest used index (+0x110), live count
/// (+0x114), clear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pool<T> {
    slots: Vec<Option<T>>,
    highest: Option<usize>,
    live: usize,
}

impl<T: Copy> Pool<T> {
    pub fn new(capacity: usize) -> Self {
        Pool {
            slots: vec![None; capacity],
            highest: None,
            live: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Live count (+0x114).
    pub fn live(&self) -> usize {
        self.live
    }

    /// Highest index allocated since the last clear (+0x110).
    pub fn highest(&self) -> Option<usize> {
        self.highest
    }

    pub fn is_full(&self) -> bool {
        self.live == self.slots.len()
    }

    /// Takes the lowest free slot; `None` when full.
    pub fn alloc(&mut self, rec: T) -> Option<usize> {
        let i = self.slots.iter().position(Option::is_none)?;
        self.slots[i] = Some(rec);
        self.live += 1;
        self.highest = Some(self.highest.map_or(i, |h| h.max(i)));
        Some(i)
    }

    pub fn free(&mut self, i: usize) {
        if self.slots[i].take().is_some() {
            self.live -= 1;
        }
    }

    /// Frees every slot.
    pub fn clear(&mut self) {
        self.slots.iter_mut().for_each(|s| *s = None);
        self.highest = None;
        self.live = 0;
    }

    pub fn get(&self, i: usize) -> Option<&T> {
        self.slots.get(i).and_then(Option::as_ref)
    }

    /// Live records in slot order 0 … highest used.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        let end = self.highest.map_or(0, |h| h + 1);
        self.slots[..end]
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|r| (i, r)))
    }

    fn set(&mut self, i: usize, rec: T) {
        if let Some(slot) = self.slots.get_mut(i).filter(|s| s.is_some()) {
            *slot = Some(rec);
        }
    }

    fn slot_mut(&mut self, i: usize) -> Option<&mut T> {
        self.slots.get_mut(i).and_then(Option::as_mut)
    }

    fn end(&self) -> usize {
        self.highest.map_or(0, |h| h + 1)
    }
}

/// A splash or bubble record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolRecord {
    /// Screen pixels (+0x04, +0x08).
    pub x: i32,
    pub y: i32,
    /// Cel kind `k` 0–3 (+0x0C): file `Rain{k+1}` / `bubble{k+1}`.
    pub kind: u8,
    /// Cel frame (+0x10).
    pub frame: u32,
    /// Countdown (+0x14).
    pub countdown: i32,
}

/// A rain drop or snow flake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Particle {
    pub x: i32,
    pub y: i32,
    /// Ground y (+0x0C).
    pub ground_y: i32,
    /// Shape values +0x10 and +0x14: rain drop length and
    /// +0x14; snow size `s` (0–7) and +0x14.
    pub shape_10: i32,
    pub shape_14: i32,
    /// Phase 0–511 (+0x18).
    pub phase: u32,
    /// Landed flag (+0x1C).
    pub landed: bool,
    /// Bounce count (+0x20).
    pub bounces: u32,
    /// Color index (+0x24) and alpha byte (+0x25).
    pub color: u8,
    pub alpha: u8,
}

/// Which pass-4 pool a record belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvPool {
    Splashes,
    Bubbles,
}

impl EnvPool {
    /// The cel file of kind `k` (in `DATA\GLOBAL\UncompOverlays\`).
    pub fn cel_file(self, kind: u8) -> String {
        let stem = match self {
            EnvPool::Splashes => "Rain",
            EnvPool::Bubbles => "bubble",
        };
        format!("DATA\\GLOBAL\\UncompOverlays\\{stem}{}", kind + 1)
    }

    /// `DrawKey` major of pass 4: 0 splashes, 1 bubbles.
    pub fn major(self) -> u32 {
        match self {
            EnvPool::Splashes => 0,
            EnvPool::Bubbles => 1,
        }
    }
}

/// The five 12-entry particle color tables, built at act load
/// by [`ColorTables::build`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorTables {
    /// Rain, day period 0 (alpha 0x7F).
    pub period0: [u8; 12],
    /// Rain, day periods 1 and 3.
    pub period13: [u8; 12],
    /// Rain, day period 2.
    pub period2: [u8; 12],
    /// Snow `S` (video modes 1–3, 6).
    pub snow: [u8; 12],
    /// Snow `S'` (other video modes).
    pub snow_alt: [u8; 12],
}

impl ColorTables {
    /// The act-load ramps: entry `i` = the palette's `nearest` colour
    /// over the current palette of, with `q` = ⌊80i / 12⌋: rain 0 (98
    /// − q, 123 − q, 98 − q); rain 1/3 (v, v + 10, v), v = 45 − ⌊40i /
    /// 12⌋; rain 2 (v, v + 5, v), v = 25 − 2i; snow (120 + q)³ and
    /// (170 + q)³.
    pub fn build(palette: &Palette) -> ColorTables {
        let n = |r: i32, g: i32, b: i32| nearest(palette, r as u32, g as u32, b as u32);
        let q = |i: i32| 80 * i / 12;
        ColorTables {
            period0: std::array::from_fn(|i| {
                let v = 98 - q(i as i32);
                n(v, v + 25, v)
            }),
            period13: std::array::from_fn(|i| {
                let v = 45 - 40 * i as i32 / 12;
                n(v, v + 10, v)
            }),
            period2: std::array::from_fn(|i| {
                let v = 25 - 2 * i as i32;
                n(v, v + 5, v)
            }),
            snow: std::array::from_fn(|i| {
                let v = 120 + q(i as i32);
                n(v, v, v)
            }),
            snow_alt: std::array::from_fn(|i| {
                let v = 170 + q(i as i32);
                n(v, v, v)
            }),
        }
    }
}

/// What act load reads from files: the eight cel files' frame
/// counts and the color tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActResources {
    /// Frame counts of `Rain1`–`Rain4`.
    pub splash_frames: [u32; 4],
    /// Frame counts of `bubble1`–`bubble4`.
    pub bubble_frames: [u32; 4],
    pub colors: ColorTables,
}

/// The local player's level as the weather reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelWeather {
    /// `levels.txt` row id.
    pub level_id: u32,
    /// Act index 0–4.
    pub act: u8,
    /// levels `Rain` and `Mud`.
    pub rain: bool,
    pub mud: bool,
}

/// The local player unit: its seed, which every weather draw uses, and
/// its level.
#[derive(Debug)]
pub struct LocalPlayer<'a> {
    pub seed: &'a mut Seed,
    pub level: LevelWeather,
}

/// Frame inputs of the weather update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateInput {
    /// The client update count: the weather advances once per update.
    pub update_count: u32,
    /// The frame's `W`, `H`.
    pub frame: FrameSize,
    /// How far the camera origin moved since the last drawn frame; every
    /// particle, splash and bubble moves back by it to stay on the ground.
    pub camera_delta: (i32, i32),
    /// The act's day period 0–3 (day, dusk, night, dawn).
    pub day_period: u8,
    /// The video mode, which picks the snow color table.
    pub video_mode: u32,
}

/// Frame inputs of pass 9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pass9Input {
    pub frame: FrameSize,
    pub mode: OpenMode,
    /// Frames counted in the last 1,000 ms.
    pub frame_rate: u32,
    /// The low-quality setting is on.
    pub low_quality: bool,
    /// Without a [`ThunderSound`] hook: whether the request of sound 202
    /// returns a handle; the two position rolls happen only
    /// then.
    pub thunder_sound_starts: bool,
}

/// The sound layer as the thunder step of pass 9 calls it (thunder plays
/// with the lightning draws).
pub trait ThunderSound: std::fmt::Debug {
    /// Requests sound `id` after `delay`: the request's handle, 0 when none
    /// was made.
    fn request(&mut self, id: u16, delay: i32) -> u32;
    /// Places sound `h` at (x, y).
    fn set_position(&mut self, h: u32, x: i32, y: i32);
}

/// A pass-4 draw: cel `frame` of the kind's file at (`x`, `y`),
/// `D2GFX_DrawCelContext`, light −1, draw mode 3, palette 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolDraw {
    pub pool: EnvPool,
    /// Slot index: the `DrawKey` minor.
    pub slot: usize,
    pub kind: u8,
    pub frame: u32,
    pub x: i32,
    pub y: i32,
}

impl PoolDraw {
    /// Light −1: unlit.
    pub const LIGHT: i32 = -1;
    /// Draw mode 3.
    pub const DRAW_MODE: u8 = 3;
    pub const PALETTE: u8 = 0;
}

/// A pass-9 draw, in build order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkyDraw {
    /// `D2GFX_DrawRectangle`, color 255, mode 5.
    Flash { x0: i32, y0: i32, x1: i32, y1: i32 },
    /// `D2GFX_DrawLine`.
    Line {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: u8,
        alpha: u8,
    },
}

impl SkyDraw {
    pub const FLASH_COLOR: u8 = 255;
    pub const FLASH_DRAW_MODE: u8 = 5;
}

/// Sound 202 of a thunder strike: emitted, not played here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thunder {
    pub sound: u16,
    /// The request's delay `roll_range(25, 50)` (a delay, not a volume).
    pub delay: i32,
    /// (x, y) of the sound's position when the request returned a handle: y =
    /// `roll_range(−200, 400)` is drawn first, then x.
    pub position: Option<(i32, i32)>,
}

/// The output of pass 9.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pass9 {
    pub draws: Vec<SkyDraw>,
    pub thunder: Option<Thunder>,
}

/// The floor pass's per-frame context (what the water floors of a frame
/// spawn splashes and bubbles from).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FloorContext {
    /// The splash threshold `k` (+0x10) of this frame.
    pub k: i32,
    /// Update count of the last splash frame (+0x14) and bubble frame
    /// (+0x18); 0 at program start, never reset.
    pub last_s: u32,
    pub last_b: u32,
    /// This frame's flags; cleared at the start of each frame.
    pub splash: bool,
    pub bubble: bool,
}

impl FloorContext {
    /// Start of a frame's floor pass: `k` := [`Weather::splash_threshold`];
    /// when `c` > `last_s` + 3, `last_s` := `c` (whatever `k` is) and the
    /// splash flag when `k` ≠ 0; the bubble flag likewise with `Mud`.
    pub fn begin_frame(&mut self, update_count: u32, weather: &Weather, mud: bool) {
        let c = update_count;
        self.splash = false;
        self.bubble = false;
        self.k = weather.splash_threshold();
        if c > self.last_s.wrapping_add(3) {
            self.last_s = c;
            self.splash = self.k != 0;
        }
        if mud && c > self.last_b.wrapping_add(25) {
            self.bubble = true;
            self.last_b = c;
        }
    }
}

/// `L`, `R` of passes 4 and 9 for frame width `W` and open mode:
/// the world beside an open side panel (`FrameSize::side_panel_span`,
/// `W / 2` at 4:3).
pub fn span(frame: FrameSize, mode: OpenMode) -> (i32, i32) {
    let w = frame.width;
    match mode.get() {
        1 => (0, w - frame.side_panel_span()),
        2 => (frame.side_panel_span(), w),
        _ => (0, w),
    }
}

/// The weather state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Weather {
    particles: Pool<Particle>,
    splashes: Pool<PoolRecord>,
    bubbles: Pool<PoolRecord>,
    /// The level's rain and mud flags at the previous update: losing one
    /// empties its pools.
    pub rain_flag: bool,
    pub mud_flag: bool,
    /// Particles are snow flakes instead of rain drops.
    pub snow_mode: bool,
    /// Snow lock: the cycle stays in the steady phase (the summit levels).
    pub snow_lock: bool,
    /// Size bump: snow flakes draw one size larger.
    pub size_bump: bool,
    /// Rain cycle phase 0–3, the phase's length and its remaining count.
    pub phase: u32,
    pub length: u32,
    pub countdown: u32,
    /// The cycle's phase lengths ([`CYCLE_MIN`], [`CYCLE_N`] as act load
    /// set them).
    pub cycle_min: [i32; 4],
    pub cycle_n: [i32; 4],
    /// Peak and target particle counts.
    pub peak: u32,
    pub target: u32,
    /// Intensity in 1/256ths (the target the cycle last reached, or 0 when
    /// it is dry).
    pub intensity_256: u32,
    /// Wind direction (an index of the 512-step sine table), the goal it
    /// drifts toward, and the countdown to the next goal.
    pub wind: i32,
    pub wind_goal: i32,
    pub wind_countdown: i32,
    /// Snow wind goal roll `(min, n)`.
    pub snow_goal_min: i32,
    pub snow_goal_n: i32,
    /// Lightning: on, countdown to the next flash, flash phase, the
    /// frame trigger pass 9 counts down, and whether the strike thunders.
    pub lightning_on: bool,
    pub lightning_countdown: i32,
    pub lightning_phase: u32,
    pub lightning_trigger: i32,
    pub thunder: bool,
    /// The update count the weather last advanced at.
    pub mark: u32,
    /// The particle-move counter `F`, which phases the snow sway; counts
    /// for the client's lifetime, never reset.
    pub move_counter: u32,
    /// Whether the one-time wind and lightning roll of the first act load
    /// has run; never cleared.
    wind_ready: bool,
    resources: Option<ActResources>,
}

impl Default for Weather {
    fn default() -> Self {
        Self::new()
    }
}

impl Weather {
    /// The state at client start: empty pools, thunder on, the start snow
    /// goal, every other value 0. Nothing resets the cycle later: phase,
    /// length, countdown and the floor context carry over from game to
    /// game.
    pub fn new() -> Self {
        Weather {
            particles: Pool::new(PARTICLE_SLOTS),
            splashes: Pool::new(SPLASH_SLOTS),
            bubbles: Pool::new(BUBBLE_SLOTS),
            rain_flag: false,
            mud_flag: false,
            snow_mode: false,
            snow_lock: false,
            size_bump: false,
            phase: 0,
            length: 0,
            countdown: 0,
            cycle_min: [0; 4],
            cycle_n: [0; 4],
            peak: 0,
            target: 0,
            intensity_256: 0,
            wind: 0,
            wind_goal: 0,
            wind_countdown: 0,
            snow_goal_min: SNOW_GOAL_START.0,
            snow_goal_n: SNOW_GOAL_START.1,
            lightning_on: false,
            lightning_countdown: 0,
            lightning_phase: 0,
            lightning_trigger: 0,
            thunder: true,
            mark: 0,
            move_counter: 0,
            wind_ready: false,
            resources: None,
        }
    }

    pub fn particles(&self) -> &Pool<Particle> {
        &self.particles
    }

    pub fn splashes(&self) -> &Pool<PoolRecord> {
        &self.splashes
    }

    pub fn bubbles(&self) -> &Pool<PoolRecord> {
        &self.bubbles
    }

    /// A placement of the local player (a waypoint, a portal, a new game)
    /// empties the splash and bubble pools: they belong to the floors the
    /// player left.
    pub fn placement_clear(&mut self) {
        self.splashes.clear();
        self.bubbles.clear();
    }

    /// Mutable pools, for restoring a captured state.
    pub fn pools_mut(
        &mut self,
    ) -> (
        &mut Pool<Particle>,
        &mut Pool<PoolRecord>,
        &mut Pool<PoolRecord>,
    ) {
        (&mut self.particles, &mut self.splashes, &mut self.bubbles)
    }

    pub fn resources(&self) -> Option<&ActResources> {
        self.resources.as_ref()
    }

    /// The splash threshold `k` = trunc(intensity × 1000), out of 1000
    /// per drawn water floor. Intensity is `intensity_256 / 256`, exact in
    /// a float for every target the cycle produces (≤ 256), so this is the
    /// integer ⌊intensity_256 × 1000 / 256⌋.
    pub fn splash_threshold(&self) -> i32 {
        (u64::from(self.intensity_256) * 1_000 / 256) as i32
    }

    // ---------------------------------------------------------------- act and level changes

    /// Act load: the level-entry re-check with `arg` 1, the cycle
    /// tables (`Min[2]` × 3 in acts III and V), the color tables and cel
    /// frame counts, intensity := 0.
    pub fn act_load(
        &mut self,
        player: LocalPlayer<'_>,
        resources: ActResources,
    ) -> Result<(), WeatherError> {
        let level = player.level;
        self.level_entry(1, player.seed, &level)?;
        if !self.wind_ready {
            self.reset_wind_lightning(player.seed);
            self.wind_ready = true;
        }
        self.cycle_min = CYCLE_MIN;
        self.cycle_n = CYCLE_N;
        if level.act == 2 || level.act == 4 {
            self.cycle_min[2] *= 3;
        }
        self.resources = Some(resources);
        self.intensity_256 = 0;
        Ok(())
    }

    /// The snow and lightning re-check of a level entry
    /// (`arg` ≠ 0 at act load) and of rain phase 1 (`arg` = 0).
    pub fn level_entry(
        &mut self,
        arg: u32,
        seed: &mut Seed,
        level: &LevelWeather,
    ) -> Result<(), WeatherError> {
        if level.act != 4 {
            self.snow_off()?;
            self.reset_wind_lightning(seed);
            return Ok(());
        }
        self.reset_wind_lightning(seed);
        let mut t = 0;
        let mut lock = false;
        match level.level_id {
            109..=112 => {
                t = 25;
                self.size_bump = false;
            }
            117 => self.size_bump = true,
            120 | 121 => {
                self.size_bump = true;
                lock = true;
            }
            _ => {}
        }
        if seed.step() % 100 < t && arg == 0 {
            return self.snow_off();
        }
        self.snow_lock = lock;
        if lock {
            self.phase_entry(2, seed, level)?;
        }
        // Lightning off.
        self.lightning_on = false;
        if !self.snow_mode || arg != 0 {
            self.clear_particles()?;
        }
        self.snow_mode = true;
        Ok(())
    }

    /// Snow off (without the wind and lightning reset); a level change also
    /// clears lightning.
    fn snow_off(&mut self) -> Result<(), WeatherError> {
        self.snow_lock = false;
        if self.snow_mode {
            self.clear_particles()?;
        }
        self.snow_mode = false;
        self.lightning_on = false;
        Ok(())
    }

    fn clear_particles(&mut self) -> Result<(), WeatherError> {
        self.particles.clear();
        if self.particles.live() != 0 {
            return Err(WeatherError::Fatal(0x15B));
        }
        Ok(())
    }

    /// The wind and lightning reset: three raw steps.
    fn reset_wind_lightning(&mut self, seed: &mut Seed) {
        self.wind_countdown = 125 + (seed.step() % 375) as i32;
        self.lightning_countdown = 500 + (seed.step() % 1_500) as i32;
        let w = 92 + (seed.step() % 71) as i32;
        self.wind = w;
        self.wind_goal = w;
    }

    /// The per-level presets of rain phase 2.
    fn level_presets(&mut self, seed: &mut Seed, level: &LevelWeather) {
        match level.level_id {
            109..=112 => {
                (self.snow_goal_min, self.snow_goal_n) = (42, 170);
                self.peak = seed.roll_range(32, 56) as u32;
            }
            117 => {
                (self.snow_goal_min, self.snow_goal_n) = (170, 56);
                self.peak = seed.roll_range(40, 112) as u32;
            }
            120 | 121 => {
                (self.snow_goal_min, self.snow_goal_n) = (28, 28);
                self.wind = 28;
                self.wind_goal = 28;
                self.peak = 256;
            }
            _ => {
                self.lightning_on = false;
                if !self.snow_mode {
                    seed.step();
                }
            }
        }
        self.target = self.peak;
    }

    /// The rain phase entries for phase `p`. p > 3: fatal 0x12A; snow
    /// lock set and p ≠ 2: nothing (the stored phase stays); else phase :=
    /// p and the entry.
    fn phase_entry(
        &mut self,
        p: u32,
        seed: &mut Seed,
        level: &LevelWeather,
    ) -> Result<(), WeatherError> {
        if p > 3 {
            return Err(WeatherError::Fatal(0x12A));
        }
        if self.snow_lock && p != 2 {
            return Ok(());
        }
        self.phase = p;
        match p {
            0 => self.target = 0,
            1 => {
                self.level_entry(0, seed, level)?;
                self.peak = seed.roll_range(32, 224) as u32;
            }
            2 => self.level_presets(seed, level),
            _ => self.lightning_on = false,
        }
        Ok(())
    }

    // ---------------------------------------------------------------- the weather update

    /// The weather update, once per frame before
    /// `StartDraw`; advances at most once per client update.
    pub fn update(
        &mut self,
        player: Option<LocalPlayer<'_>>,
        input: &UpdateInput,
    ) -> Result<(), WeatherError> {
        let (r, m) = player
            .as_ref()
            .map_or((false, false), |p| (p.level.rain, p.level.mud));
        if !r && self.rain_flag {
            if self.particles.live() != 0 {
                self.particles.clear();
            }
            if self.splashes.live() != 0 {
                self.splashes.clear();
            }
        }
        if !m && self.mud_flag && self.bubbles.live() != 0 {
            self.bubbles.clear();
        }
        self.rain_flag = r;
        self.mud_flag = m;

        let c = input.update_count;
        let Some(player) = player else {
            return Err(WeatherError::Fatal(0x547));
        };
        if c <= self.mark {
            return Ok(());
        }
        self.mark = c;
        let (seed, level) = (player.seed, player.level);
        if !r {
            // Rain off: intensity := 0.0, the target is kept.
            self.intensity_256 = 0;
        } else {
            // The move runs only when particles are live, so `F`
            // counts those calls; an update that empties the pool during
            // the call still counted. Rain updates with no live particle
            // do not move and leave `F` at 0; it counts from the first
            // update with one.
            if self.particles.live() != 0 {
                self.move_particles(seed, input)?;
            }
            let frames = self.resources.ok_or(WeatherError::NotLoaded)?.splash_frames;
            update_pool(&mut self.splashes, &frames, input.camera_delta);
            self.rain_cycle(seed, &level)?;
            self.top_up(seed, input)?;
        }
        if m {
            let frames = self.resources.ok_or(WeatherError::NotLoaded)?.bubble_frames;
            update_pool(&mut self.bubbles, &frames, input.camera_delta);
        }
        Ok(())
    }

    /// The particle move, once per weather update
    /// while particles are live. Landed particles with no bounce left are
    /// freed and replaced by one spawn on the player seed while the live
    /// count is under the target.
    fn move_particles(&mut self, seed: &mut Seed, input: &UpdateInput) -> Result<(), WeatherError> {
        let (wc, ws) = drop_direction(self.wind);
        // Step 1: the speed factor, float32.
        let f: f32 = if self.snow_mode {
            (wc.abs() as f32).max(0.25)
        } else {
            let intensity = (self.intensity_256 as f32) * (1.0 / 256.0);
            // The two doubles hold the float32 values of 0.15 and 0.85.
            (f64::from(intensity) * f64::from(0.15f32) + f64::from(0.85f32)) as f32
        };
        let (dx, dy) = input.camera_delta;
        let width = input.frame.width;
        let sine = sine_table();
        let mut i = 0;
        // Step 2: slots 0 … highest used index, in order.
        while i < self.particles.end() {
            let Some(mut p) = self.particles.get(i).copied() else {
                i += 1;
                continue;
            };
            let a = (f64::from(p.shape_14) * f64::from(f)) as i32;
            let mut ux = (wc * f64::from(a)) as i32;
            let uy = (ws * f64::from(a)) as i32;
            p.y += uy - dy;
            if p.y > p.ground_y {
                p.landed = true;
            }
            self.particles.set(i, p);
            if p.landed {
                if p.bounces == 0 {
                    self.particles.free(i);
                    if (self.particles.live() as u64) < u64::from(self.target) {
                        self.spawn_particle(seed, input)?;
                    }
                } else {
                    ux = 0;
                    p.shape_14 = 0;
                    p.bounces -= 1;
                    self.particles.set(i, p);
                }
            }
            // Step 2.4 on whatever now lives in the slot (the spawn can
            // reuse the slot just freed; it keeps the freed `ux`).
            if let Some(mut q) = self.particles.get(i).copied() {
                if self.snow_mode && !q.landed {
                    let arg = (self.move_counter.wrapping_mul(512) / 25).wrapping_add(q.phase);
                    ux += (2.0 * f64::from(sine[(arg & 511) as usize])) as i32;
                }
                q.x += ux - dx;
                if q.x >= width {
                    q.x -= width;
                }
                if q.x < 0 {
                    q.x += width;
                }
                self.particles.set(i, q);
            }
            i += 1;
        }
        // Step 3.
        self.move_counter = self.move_counter.wrapping_add(1);
        Ok(())
    }

    /// The rain cycle. The cycle's unlock switch is never set, so a locked
    /// cycle off phase 2 always becomes phase 2.
    fn rain_cycle(&mut self, seed: &mut Seed, level: &LevelWeather) -> Result<(), WeatherError> {
        if self.snow_lock && self.phase != 2 {
            self.phase = 2;
            self.level_presets(seed, level);
            return Ok(());
        }
        // The ramp uses this call's p: a locked cycle keeps the stored
        // phase 2 while the drawn p still picks the ramp.
        let mut p = self.phase;
        if self.countdown == 0 {
            p = (self.phase + 1) % 4;
            let d = seed.roll_range(self.cycle_min[p as usize], self.cycle_n[p as usize]);
            self.length = d as u32;
            self.countdown = d as u32;
            self.phase_entry(p, seed, level)?;
        }
        self.countdown = self.countdown.wrapping_sub(1);
        let d = self.length;
        match p {
            1 => self.target = self.peak.wrapping_mul(d.wrapping_sub(self.countdown)) / d,
            3 => self.target = self.peak.wrapping_mul(self.countdown) / d,
            _ => {}
        }
        self.intensity_256 = self.target;
        Ok(())
    }

    /// Top-up, wind and lightning timer.
    fn top_up(&mut self, seed: &mut Seed, input: &UpdateInput) -> Result<(), WeatherError> {
        while (self.particles.live() as u64) < u64::from(self.target) {
            if !self.spawn_particle(seed, input)? {
                break;
            }
        }
        let s = if self.snow_mode { 1 } else { 2 };
        if self.wind < self.wind_goal {
            self.wind = (self.wind + s).min(self.wind_goal);
        } else if self.wind > self.wind_goal {
            self.wind = (self.wind - s).max(self.wind_goal);
        }
        self.wind_countdown = self.wind_countdown.wrapping_sub(1);
        if self.wind_countdown == 0 {
            if self.snow_mode {
                self.wind_countdown = 250 + (seed.step() % 875) as i32;
                self.wind_goal = seed.roll_range(self.snow_goal_min, self.snow_goal_n);
            } else {
                self.wind_countdown = 125 + (seed.step() % 375) as i32;
                self.wind_goal = 92 + (seed.step() % 71) as i32;
            }
        }
        if self.lightning_on {
            self.lightning_countdown = self.lightning_countdown.wrapping_sub(1);
            if self.lightning_countdown == 0 {
                self.lightning_trigger = 1;
            }
        }
        Ok(())
    }

    /// One particle spawn; `false` when the pool
    /// is full.
    fn spawn_particle(
        &mut self,
        seed: &mut Seed,
        input: &UpdateInput,
    ) -> Result<bool, WeatherError> {
        let colors = self.resources.ok_or(WeatherError::NotLoaded)?.colors;
        if self.particles.is_full() {
            return Ok(false);
        }
        let (w, h) = (input.frame.width, input.frame.height);
        let x = seed.roll_range(0, w);
        let g = seed.roll_range(40, h - 87);
        let y = seed.roll_range(-20, g + 20);
        let phase = seed.step() & 511;
        // t = (g − 40) / (H − 87) in [0, 1); the float forms of this
        // ratio equal these floors.
        let (num, den) = (g - 40, h - 87);
        let floor = |k: i32| (k * num).div_euclid(den);
        let rec = if self.snow_mode {
            seed.step();
            let size = num * 7 / den + i32::from(self.size_bump);
            if !(0..=7).contains(&size) {
                return Err(WeatherError::Fatal(0x390));
            }
            let (color, alpha) = snow_color(&colors, size, input.day_period, input.video_mode)?;
            Particle {
                x,
                y,
                ground_y: g,
                shape_10: size,
                shape_14: 8 + floor(28),
                phase,
                landed: false,
                bounces: 1,
                color,
                alpha,
            }
        } else {
            let i = (seed.step() % 12) as usize;
            let (color, alpha) = match input.day_period {
                0 => (colors.period0[i], 0x7F),
                2 => (colors.period2[i], 0xFF),
                _ => (colors.period13[i], 0xFF),
            };
            Particle {
                x,
                y,
                ground_y: g,
                shape_10: 4 + floor(8),
                shape_14: 15 + floor(15),
                phase,
                landed: false,
                bounces: 3,
                color,
                alpha,
            }
        };
        self.particles.alloc(rec);
        Ok(true)
    }

    // ---------------------------------------------------------------- splashes and bubbles

    /// After every drawn floor whose DT1 material has bit 0x2 (water), at
    /// its screen position: one `roll_range(0, 1000)`,
    /// then the gated splash and bubble spawns.
    pub fn water_floor(&mut self, ctx: &FloorContext, x: i32, y: i32, seed: &mut Seed) {
        let r = seed.roll_range(0, 1_000);
        if ctx.splash && r < ctx.k {
            self.spawn_splash(x, y, seed);
        }
        if ctx.bubble && r < 100 {
            self.spawn_bubble(x, y, seed);
        }
    }

    /// Spawns a splash at (x, y); nothing when the pool is full.
    pub fn spawn_splash(&mut self, x: i32, y: i32, seed: &mut Seed) -> Option<usize> {
        if self.splashes.is_full() {
            return None;
        }
        let (dx, a) = spawn_offset(seed);
        let kind = 2 + (seed.step() & 1) as u8;
        self.splashes.alloc(PoolRecord {
            x: x + dx,
            y: y + a,
            kind,
            frame: 0,
            countdown: 2,
        })
    }

    /// Spawns a bubble at (x, y); nothing when the pool is full.
    pub fn spawn_bubble(&mut self, x: i32, y: i32, seed: &mut Seed) -> Option<usize> {
        if self.bubbles.is_full() {
            return None;
        }
        let (dx, a) = spawn_offset(seed);
        self.bubbles.alloc(PoolRecord {
            x: x + dx,
            y: y + a - 30,
            kind: 2,
            frame: 0,
            countdown: 2,
        })
    }

    // ---------------------------------------------------------------- pass 4

    /// Pass 4: splashes, then bubbles, each in slot order; `shift_x` is
    /// the camera's x shift.
    pub fn pass4(&self, frame: FrameSize, mode: OpenMode, shift_x: i32) -> Vec<PoolDraw> {
        let (l, r) = span(frame, mode);
        let bottom = frame.height - BOTTOM_MARGIN;
        let mut out = Vec::new();
        for (pool, recs) in [
            (EnvPool::Splashes, &self.splashes),
            (EnvPool::Bubbles, &self.bubbles),
        ] {
            if recs.live() == 0 {
                continue;
            }
            for (slot, rec) in recs.iter() {
                let x = rec.x + shift_x;
                if x >= l && x < r && rec.y >= 0 && rec.y < bottom {
                    out.push(PoolDraw {
                        pool,
                        slot,
                        kind: rec.kind,
                        frame: rec.frame,
                        x,
                        y: rec.y,
                    });
                }
            }
        }
        out
    }

    // ---------------------------------------------------------------- pass 9

    /// Starts lightning: lightning on, thunder flag := `thunder`, frame
    /// trigger 1. What calls it is not modelled yet.
    pub fn start_lightning(&mut self, thunder: bool) {
        self.lightning_on = true;
        self.thunder = thunder;
        self.lightning_trigger = 1;
    }

    /// Pass 9: the lightning flash, else the particles.
    pub fn pass9(
        &mut self,
        player: Option<LocalPlayer<'_>>,
        input: &Pass9Input,
    ) -> Result<Pass9, WeatherError> {
        self.pass9_with(player, input, None)
    }

    /// [`Weather::pass9`] with the sound layer called at the thunder step:
    /// its handle decides the position rolls
    /// (`input.thunder_sound_starts` is then not read).
    pub fn pass9_with(
        &mut self,
        player: Option<LocalPlayer<'_>>,
        input: &Pass9Input,
        mut sound: Option<&mut dyn ThunderSound>,
    ) -> Result<Pass9, WeatherError> {
        let player = player.ok_or(WeatherError::Fatal(0x573))?;
        let seed = player.seed;
        let (l, r) = span(input.frame, input.mode);
        let bottom = input.frame.height - BOTTOM_MARGIN;
        let mut out = Pass9::default();
        if self.lightning_on && self.lightning_trigger != 0 {
            self.lightning_trigger -= 1;
            if self.lightning_trigger == 0 {
                if self.lightning_phase == 0 {
                    self.lightning_countdown = 3;
                    self.lightning_phase = 1;
                } else {
                    self.lightning_countdown = seed.roll_range(500, 1_500);
                    self.lightning_phase = 0;
                    if self.thunder {
                        // Thunder: the delay, the request, then y and x
                        // when it returned a handle.
                        let delay = seed.roll_range(25, 50);
                        let h = match sound.as_deref_mut() {
                            Some(s) => s.request(THUNDER_SOUND, delay),
                            None => u32::from(input.thunder_sound_starts),
                        };
                        let position = (h != 0).then(|| {
                            let y = seed.roll_range(-200, 400);
                            let x = seed.roll_range(-200, 400);
                            (x, y)
                        });
                        if let (Some(s), Some((x, y))) = (sound, position) {
                            s.set_position(h, x, y);
                        }
                        out.thunder = Some(Thunder {
                            sound: THUNDER_SOUND,
                            delay,
                            position,
                        });
                    } else {
                        self.thunder = true;
                    }
                }
            }
            if input.frame_rate > 9 {
                out.draws.push(SkyDraw::Flash {
                    x0: l,
                    y0: 0,
                    x1: r,
                    y1: bottom,
                });
                return Ok(out);
            }
        }
        if !player.level.rain || self.particles.live() == 0 || input.low_quality {
            return Ok(out);
        }
        let in_range = |x: i32, y: i32| x >= l && x < r && y >= 0 && y < bottom;
        let line = |x0: i32, y0: i32, x1: i32, y1: i32, p: &Particle| SkyDraw::Line {
            x0,
            y0,
            x1,
            y1,
            color: p.color,
            alpha: p.alpha,
        };
        let (u_dir, v_dir) = drop_direction(self.wind);
        for (_, p) in self.particles.iter() {
            if self.snow_mode {
                let s = p.shape_10;
                if (0..=7).contains(&s) && in_range(p.x, p.y) {
                    for t in [SNOW_LINES[s as usize], SNOW_LINES[s as usize + 1]] {
                        out.draws
                            .push(line(p.x + t[0], p.y + t[1], p.x + t[2], p.y + t[3], p));
                    }
                }
            } else if p.landed {
                if in_range(p.x, p.y) {
                    out.draws.push(line(p.x, p.y, p.x, p.y, p));
                }
            } else {
                let len = f64::from(p.shape_10);
                let mut u = (u_dir * len) as i32;
                let mut v = (v_dir * len) as i32;
                let rest = p.ground_y - p.y;
                if v > rest {
                    if v == 0 {
                        return Err(WeatherError::DropDivideByZero { x: p.x, y: p.y });
                    }
                    u = rest.wrapping_mul(u) / v;
                    v = rest;
                }
                let (x1, y1) = (p.x + u, p.y + v);
                if in_range(p.x, p.y) || in_range(x1, y1) {
                    out.draws.push(line(p.x, p.y, x1, y1, p));
                }
            }
        }
        Ok(out)
    }
}

/// The falling-drop direction of wind `w`: the cosine and sine of the
/// 512-step sine table (`(Wt[(w + 128) & 511], Wt[w & 511])`), widened
/// exactly to `f64`.
fn drop_direction(w: i32) -> (f64, f64) {
    let t = sine_table();
    (
        f64::from(t[((w + 128) & 511) as usize]),
        f64::from(t[(w & 511) as usize]),
    )
}

/// The snow particle's color and alpha (no draws)
/// for size `s`, day period `p` and video mode.
fn snow_color(
    colors: &ColorTables,
    s: i32,
    p: u8,
    video_mode: u32,
) -> Result<(u8, u8), WeatherError> {
    let alpha = *SNOW_ALPHA
        .get(usize::from(p))
        .ok_or(WeatherError::Fatal(0x356))?;
    let i = (12 * s / 8) as usize;
    let color = if matches!(video_mode, 1..=3 | 6) {
        match p {
            0 => colors.snow[i],
            2 => colors.snow[i / 4],
            _ => colors.snow[i / 2],
        }
    } else {
        colors.snow_alt[i]
    };
    Ok((color, alpha))
}

/// The spawn offset of a splash or bubble: `a` := `roll_range(0,
/// 80)`, `b` := 80 − 2·|a − 40|, dx := `roll_range(−b, 2b)` (no draw when
/// `b` = 0). Returns (dx, a).
fn spawn_offset(seed: &mut Seed) -> (i32, i32) {
    let a = seed.roll_range(0, 80);
    let b = 80 - 2 * (a - 40).abs();
    (seed.roll_range(-b, 2 * b), a)
}

/// Splash / bubble update.
fn update_pool(pool: &mut Pool<PoolRecord>, frames: &[u32; 4], delta: (i32, i32)) {
    for i in 0..pool.end() {
        let Some(rec) = pool.slot_mut(i) else {
            continue;
        };
        rec.x -= delta.0;
        rec.y -= delta.1;
        rec.countdown -= 1;
        if rec.countdown < 1 {
            rec.frame += 1;
            rec.countdown = 2;
            if rec.frame >= frames[usize::from(rec.kind & 3)] {
                pool.free(i);
            }
        }
    }
}

#[cfg(test)]
#[path = "weather_tests.rs"]
mod tests;
