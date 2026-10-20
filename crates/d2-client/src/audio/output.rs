//! Output edge: one rodio `Source` registered
//! through Bevy's `Decodable`. It pulls blocks from the shared
//! [`AudioEngine`] and converts i16 → f32 as `s / 32768.0` (exact in f32).
//! Device conversion and resampling after this point are outside the
//! exactness boundary.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::audio::{AddAudioSource, ChannelCount, Decodable, Sample, SampleRate, Source};
use bevy::prelude::{App, Asset};
use bevy::reflect::TypePath;

use super::{AudioEngine, BLOCK_SAMPLES, OUTPUT_RATE};

/// The mixed stream as a Bevy audio asset. Play it with
/// `AudioPlayer(assets.add(MixerStream::new(engine)))`; the game side keeps
/// its own clone of the engine handle to queue cues and present ticks.
#[derive(Asset, TypePath, Clone)]
pub struct MixerStream {
    engine: Arc<Mutex<AudioEngine>>,
}

impl MixerStream {
    pub fn new(engine: Arc<Mutex<AudioEngine>>) -> Self {
        MixerStream { engine }
    }
}

impl Decodable for MixerStream {
    type Decoder = MixerDecoder;

    fn decoder(&self) -> MixerDecoder {
        MixerDecoder {
            engine: Arc::clone(&self.engine),
            block: [0; BLOCK_SAMPLES],
            pos: BLOCK_SAMPLES,
        }
    }
}

/// Register [`MixerStream`] with Bevy's audio plugin.
pub fn register(app: &mut App) {
    app.add_audio_source::<MixerStream>();
}

/// Endless interleaved stereo stream at [`OUTPUT_RATE`].
pub struct MixerDecoder {
    engine: Arc<Mutex<AudioEngine>>,
    block: [i16; BLOCK_SAMPLES],
    pos: usize,
}

/// i16 → device sample: `s / 32768.0`, exact in f32.
pub fn to_device(s: i16) -> Sample {
    Sample::from(s) / 32768.0
}

impl Iterator for MixerDecoder {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.pos == BLOCK_SAMPLES {
            // A poisoned engine means the game side panicked: end the
            // stream rather than play stale state.
            let mut engine = self.engine.lock().ok()?;
            self.block = engine.mix_block();
            self.pos = 0;
        }
        let s = self.block[self.pos];
        self.pos += 1;
        Some(to_device(s))
    }
}

impl Source for MixerDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        const { ChannelCount::new(2).unwrap() }
    }

    fn sample_rate(&self) -> SampleRate {
        const { SampleRate::new(OUTPUT_RATE).unwrap() }
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
