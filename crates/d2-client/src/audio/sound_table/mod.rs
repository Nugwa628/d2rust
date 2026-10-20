//! The sound table and request layer: sound id →
//! file path, groups and variants ([`table`]), volume, pan and settings
//! ([`volume`]), requests, sound ticks and the 16 channels ([`system`]).
//!
//! Hook mapping: [`SoundPaths`] is the play mode's `SoundTable` /
//! `SoundBank::file`; the variant choice runs on the
//! local player's client unit seed through [`SoundWorld`];
//! admission, stealing and the scheduler rules are
//! [`SoundSystem`], which hands the core every start, stop and volume/pan
//! send as cues (so the core runs with [`crate::audio::Unlimited`]);
//! [`DeviceGain`] is the `GainCurve` of the device volume.

pub mod sliders;
pub mod system;
pub mod table;
pub mod volume;

#[cfg(test)]
mod tests;

pub use system::{
    ChannelKind, Fade, Request, RequestState, SoundCtx, SoundError, SoundSystem, SoundWorld,
    CHANNELS, REQUEST_SLOTS,
};
pub use table::{
    is_music_path, is_speech, LoadState, SoundEntry, SoundPaths, SoundTableData, SoundTableError,
};
pub use volume::{DeviceGain, SoundSettings};
