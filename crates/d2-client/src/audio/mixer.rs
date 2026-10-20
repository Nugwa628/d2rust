//! Integer mixer: fixed output rate, 32.32 phase
//! accumulator, nearest-sample resampling (the frame at `floor(phase)`, no
//! interpolation), i32 gain and sum, saturation to i16. Ours:
//! deterministic and golden-tested, not compared to the game.

use std::sync::Arc;

use super::{AudioError, CueId, VoiceParams};

/// Output rate `R` in Hz (ours).
pub const OUTPUT_RATE: u32 = 44_100;
/// Frames per mixer block (ours).
pub const BLOCK_FRAMES: usize = 512;
/// Interleaved stereo samples per block.
pub const BLOCK_SAMPLES: usize = BLOCK_FRAMES * 2;
/// Gain components are Q8: 256 is unity.
pub const GAIN_UNITY: i32 = 256;
/// `out = (s × vol × pan) >> GAIN_SHIFT` with both gains Q8.
const GAIN_SHIFT: u32 = 16;

/// Decoded samples of one file: interleaved i16 frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sound {
    rate: u32,
    channels: u16,
    samples: Vec<i16>,
}

impl Sound {
    /// Strict: rate > 0, mono or stereo, whole frames, at least one frame.
    pub fn new(rate: u32, channels: u16, samples: Vec<i16>) -> Result<Self, AudioError> {
        if rate == 0 {
            return Err(AudioError::ZeroRate);
        }
        if !(1..=2).contains(&channels) {
            return Err(AudioError::Channels(channels));
        }
        if !samples.len().is_multiple_of(channels as usize) {
            return Err(AudioError::PartialFrame {
                samples: samples.len(),
                channels,
            });
        }
        if samples.is_empty() {
            return Err(AudioError::Empty);
        }
        Ok(Sound {
            rate,
            channels,
            samples,
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }

    /// Frames up to the last nonzero sample, a partial frame dropped: the
    /// length a capture's device-end row gives a voice ("the sample's
    /// frames without the zero tail").
    pub fn frames_without_zero_tail(&self) -> usize {
        let used = self
            .samples
            .iter()
            .rposition(|&x| x != 0)
            .map_or(0, |i| i + 1);
        used / self.channels as usize
    }

    /// (left, right) of frame `i`; mono plays the same sample on both.
    fn frame(&self, i: usize) -> (i32, i32) {
        if self.channels == 1 {
            let s = i32::from(self.samples[i]);
            (s, s)
        } else {
            (
                i32::from(self.samples[2 * i]),
                i32::from(self.samples[2 * i + 1]),
            )
        }
    }
}

/// Q8 gains of one voice: volume and per-side pan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gains {
    pub vol: i32,
    pub pan_l: i32,
    pub pan_r: i32,
}

impl Gains {
    /// Strict: every component in `0..=GAIN_UNITY`, which also keeps
    /// `s × vol × pan` inside i32.
    pub fn new(vol: i32, pan_l: i32, pan_r: i32) -> Result<Self, AudioError> {
        for g in [vol, pan_l, pan_r] {
            if !(0..=GAIN_UNITY).contains(&g) {
                return Err(AudioError::GainRange(g));
            }
        }
        Ok(Gains { vol, pan_l, pan_r })
    }
}

/// Volume and pan curves: integer `vol`/`pan` to Q8 gains.
///
/// The game's curves: volume amplitude `v / 255`, pan as one side scaled
/// by `pan / 127` or `(255 − pan) / 127`, the device gain `G` and the
/// occlusion applied to the volume before the amplitude, `G` = 255 at
/// every send of a game sound
/// tick ([`crate::audio::sound_table::DeviceGain`]).
pub trait GainCurve {
    fn gains(&self, vol: i32, pan: i32) -> Result<Gains, AudioError>;

    /// The gains of a voice whose occlusion is `occlusion`. A
    /// curve without occlusion ignores it.
    fn gains_occluded(&self, vol: i32, pan: i32, occlusion: f32) -> Result<Gains, AudioError> {
        let _ = occlusion;
        self.gains(vol, pan)
    }
}

/// Neutral curve: unity on both sides for every `vol` and `pan`, used by
/// the hook-only play mode; the voice log still records the integer `vol`
/// and `pan` the trigger carried.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnityGain;

impl GainCurve for UnityGain {
    fn gains(&self, _vol: i32, _pan: i32) -> Result<Gains, AudioError> {
        Ok(Gains {
            vol: GAIN_UNITY,
            pan_l: GAIN_UNITY,
            pan_r: GAIN_UNITY,
        })
    }
}

/// One playing voice.
#[derive(Clone, Debug)]
pub struct Voice {
    id: CueId,
    file: Arc<str>,
    sound: Arc<Sound>,
    params: VoiceParams,
    gains: Gains,
    /// Read position in source frames, 32.32 fixed point.
    phase: u64,
    /// Phase advance per output frame: `(rate << 32) / R`, truncated.
    step: u64,
    /// Frame a looped voice wraps to.
    loop_start: u64,
    /// Frame a looped voice wraps at; silence plays from the last sample
    /// frame up to it (a looping stream's block padding). 0: the
    /// sample's frame count.
    loop_end: u64,
    /// Device occlusion.
    occlusion: f32,
}

impl Voice {
    pub fn new(
        id: CueId,
        file: Arc<str>,
        sound: Arc<Sound>,
        params: VoiceParams,
        gains: Gains,
    ) -> Self {
        let step = (u64::from(sound.rate) << 32) / u64::from(OUTPUT_RATE);
        Voice {
            id,
            file,
            sound,
            params,
            gains,
            phase: 0,
            step,
            loop_start: 0,
            loop_end: 0,
            occlusion: 0.0,
        }
    }

    /// Device state: a start
    /// frame moves the read position (taken modulo the frame count), a
    /// loop start past the end loops to frame 0, a loop end before the
    /// last frame is ignored.
    pub(super) fn set_device(
        &mut self,
        start: Option<u64>,
        loop_start: Option<u64>,
        loop_end: Option<u64>,
        occ: f32,
    ) {
        let frames = self.sound.frames() as u64;
        if let Some(f) = start {
            self.phase = (f % frames) << 32;
        }
        if let Some(l) = loop_start {
            self.loop_start = if l < frames { l } else { 0 };
        }
        if let Some(e) = loop_end {
            self.loop_end = if e > frames { e } else { 0 };
        }
        self.occlusion = occ;
    }

    pub fn occlusion(&self) -> f32 {
        self.occlusion
    }

    pub fn loop_start(&self) -> u64 {
        self.loop_start
    }

    /// Frame a looped voice wraps at (the frame count unless padded).
    pub fn loop_end(&self) -> u64 {
        if self.loop_end == 0 {
            self.sound.frames() as u64
        } else {
            self.loop_end
        }
    }

    pub(super) fn set_gains(&mut self, gains: Gains) {
        self.gains = gains;
    }

    pub fn id(&self) -> CueId {
        self.id
    }

    pub fn file(&self) -> &str {
        &self.file
    }

    pub fn sound(&self) -> &Sound {
        &self.sound
    }

    pub fn params(&self) -> &VoiceParams {
        &self.params
    }

    pub fn gains(&self) -> Gains {
        self.gains
    }

    pub(super) fn set_params(&mut self, vol: i32, pan: i32, gains: Gains) {
        self.params.vol = vol;
        self.params.pan = pan;
        self.gains = gains;
    }

    /// Add this voice into `acc`; returns false once a one-shot voice has
    /// played its last frame. A looped voice wraps to its loop start
    /// (frame 0 unless set) at its loop end,
    /// silent past the last frame.
    fn render(&mut self, acc: &mut [i32; BLOCK_SAMPLES]) -> bool {
        let frames = self.sound.frames() as u64;
        let end = self.loop_end();
        let g = self.gains;
        for out in acc.as_chunks_mut::<2>().0 {
            let mut i = self.phase >> 32;
            if i >= frames {
                if !self.params.looped {
                    return false;
                }
                if i >= end {
                    let start = self.loop_start << 32;
                    let span = (end << 32) - start;
                    self.phase = start + (self.phase - start) % span;
                    i = self.phase >> 32;
                }
                if i >= frames {
                    self.phase += self.step;
                    continue;
                }
            }
            let (l, r) = self.sound.frame(i as usize);
            // |s| ≤ 2^15 and both gains ≤ 2^8: the product fits in i32.
            out[0] = out[0].saturating_add((l * g.vol * g.pan_l) >> GAIN_SHIFT);
            out[1] = out[1].saturating_add((r * g.vol * g.pan_r) >> GAIN_SHIFT);
            self.phase += self.step;
        }
        self.looped_or_more_frames(frames)
    }

    fn looped_or_more_frames(&self, frames: u64) -> bool {
        self.params.looped || (self.phase >> 32) < frames
    }
}

/// Mix one block of interleaved stereo i16. Voices that
/// end are removed; the others keep their order.
pub fn mix(voices: &mut Vec<Voice>) -> [i16; BLOCK_SAMPLES] {
    let mut acc = [0i32; BLOCK_SAMPLES];
    voices.retain_mut(|v| v.render(&mut acc));
    let mut out = [0i16; BLOCK_SAMPLES];
    for (o, a) in out.iter_mut().zip(acc) {
        *o = a.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
    }
    out
}
