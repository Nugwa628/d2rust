//! The frame's pass 4 and pass 9 outputs as the view consumes them: the
//! pool cels ([`PoolDraw`]) and the sky draws ([`SkyDraw`]: lightning
//! flash, particle lines) of one frame, computed from the weather state
//! the feed lends ([`super::source::WeatherFrame`]).

use crate::rules::camera::{FrameSize, OpenMode};

use super::weather::{LevelWeather, PoolDraw, SkyDraw, Thunder};

/// What the passes need besides the weather state: the frame, the open
/// mode, `shiftX`, the local player's level and the
/// pass-9 inputs the model does not hold (`D2RUST's own, unverified` in the
/// preview).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkyFrame {
    pub frame: FrameSize,
    pub mode: OpenMode,
    pub shift_x: i32,
    pub level: LevelWeather,
    /// Frames counted in the last second.
    pub frame_rate: u32,
    /// The low-quality setting.
    pub low_quality: bool,
}

/// The draws of passes 4 and 9 for one frame, in pass order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkyPasses {
    /// Pass 4: splash cels, then bubble cels.
    pub pools: Vec<PoolDraw>,
    /// Pass 9: the flash, or the particle lines.
    pub sky: Vec<SkyDraw>,
    /// The thunder strike the pass started: emitted, not played here (the
    /// sound layer plays it).
    pub thunder: Option<Thunder>,
}

impl SkyPasses {
    pub fn is_empty(&self) -> bool {
        self.pools.is_empty() && self.sky.is_empty()
    }
}
