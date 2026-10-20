//! The light quality `q` and the measured draw rate that drives it.
//!
//! Lighting scales itself to the machine: the client counts how many
//! frames it draws, and the light quality follows that rate.
//! - `q` = 2: units' lights cast shadows.
//! - `q` = 1: every light is plain.
//! - `q` = 0: remote players', monsters' and missiles' lights are also cut
//!   back.
//!
//! A hysteresis keeps the quality from flickering between levels.
//!
//! Wall-clock time is always an input (milliseconds from a free-running
//! `u32` clock that wraps): nothing here reads the OS clock, so replays
//! can feed the quality they need.

/// The lighting option flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightOptions {
    /// Low-quality lighting: the quality never reaches 2 (and is always 0
    /// without missile lights). Off by default.
    pub low_quality: bool,
    /// Missiles carry lights (see [`super::sources`]). On by default.
    pub missile_lights: bool,
}

impl Default for LightOptions {
    /// Full lighting: low quality off, missile lights on.
    fn default() -> Self {
        LightOptions {
            low_quality: false,
            missile_lights: true,
        }
    }
}

/// The options menu's "lighting quality" item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QualityOption {
    Low,
    Medium,
    High,
}

impl LightOptions {
    /// The flags each menu choice sets: low turns low quality on and
    /// missile lights off; medium turns both off; high turns low quality
    /// off and missile lights on.
    pub fn from_menu(option: QualityOption) -> Self {
        let (low_quality, missile_lights) = match option {
            QualityOption::Low => (true, false),
            QualityOption::Medium => (false, false),
            QualityOption::High => (false, true),
        };
        LightOptions {
            low_quality,
            missile_lights,
        }
    }
}

/// The hysteresis: a new quality is taken only when more than this many
/// ms passed since the last change.
pub const HYSTERESIS_MS: u32 = 2_000;

/// The light quality state: `q` and the time of its last change, both 0
/// when the client starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Quality {
    /// `q` ∈ {0, 1, 2}.
    pub q: u8,
    /// Wall-clock ms of the last change.
    pub last_ms: u32,
}

impl Quality {
    /// The quality candidate for draw rate `d`. `Err(q)` is the 13–15
    /// early return with the current `q` (> 0): no timer update follows.
    fn candidate(&self, options: LightOptions, d: i32) -> Result<u8, u8> {
        if options.low_quality && !options.missile_lights {
            return Ok(0);
        }
        let start = if options.low_quality && options.missile_lights {
            1
        } else {
            2
        };
        match d {
            i32::MIN..=9 => Ok(0),
            10..=12 => Ok(1),
            13..=15 => {
                if self.q > 0 {
                    Err(self.q)
                } else {
                    Ok(1)
                }
            }
            _ => Ok(start),
        }
    }

    /// One quality step at wall-clock `now_ms` with the measured draw rate
    /// `d`; returns the new `q`. The quality changes only when the
    /// candidate differs and the hysteresis has passed; the elapsed time
    /// is `now − last` with wrapping, so a clock wrap does not stall it.
    pub fn update(&mut self, options: LightOptions, d: i32, now_ms: u32) -> u8 {
        let candidate = match self.candidate(options, d) {
            Ok(c) => c,
            Err(q) => return q,
        };
        if candidate != self.q && now_ms.wrapping_sub(self.last_ms) > HYSTERESIS_MS {
            self.q = candidate;
            self.last_ms = now_ms;
        }
        self.q
    }
}

/// The draw-rate window: 3,000 ms.
pub const RATE_WINDOW_MS: u32 = 3_000;
/// The draw rate assumed when a game starts, before any window closed.
pub const START_RATE: i32 = 25;

/// The measured draw rate `D` (frames per second) and its counting window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrawRateMeter {
    /// `D`.
    pub d: i32,
    /// Draws with a player room counted in the current window.
    pub count: u32,
    /// The window start (client loop wall-clock time, ms).
    pub window_start_ms: u32,
}

impl DrawRateMeter {
    /// `D` = [`START_RATE`], nothing counted, the window starting at
    /// `window_start_ms` (the caller's clock at game start).
    pub fn new(window_start_ms: u32) -> Self {
        DrawRateMeter {
            d: START_RATE,
            count: 0,
            window_start_ms,
        }
    }

    /// Counts one in-game draw that ran with a player room.
    pub fn count_draw(&mut self) {
        self.count = self.count.wrapping_add(1);
    }

    /// One client loop pass at loop time `now_ms`: once more than 3,000 ms
    /// passed since the window start, `D` becomes the window's count / 3
    /// and a new window starts. Returns `D`.
    pub fn pass(&mut self, now_ms: u32) -> i32 {
        if now_ms.wrapping_sub(self.window_start_ms) > RATE_WINDOW_MS {
            self.d = (self.count / 3) as i32;
            self.window_start_ms = now_ms;
            self.count = 0;
        }
        self.d
    }
}
