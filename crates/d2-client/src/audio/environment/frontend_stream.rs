//! The device layer under the front-end [`Jukebox`]: one music voice that
//! streams a WAV from the archives, with the jukebox picks it makes
//! recorded as [`MusicEvent`]s. Plain Rust (no Bevy, no I/O except the
//! injected [`FileSource`]); the Bevy edge pulls samples with
//! [`FrontMusic::fill`] (`app::front_host`). Out of game there is no tick
//! rule: the random source is D2RUST's own.

use std::sync::Arc;

use super::frontend::{Jukebox, JukeboxHost};
use crate::assets::path::{read_asset, CanonicalPath, FileSource};
use crate::audio::sound_table::volume::device_volume;

/// Archive folder of the playlists' paths.
pub const MUSIC_DIR: &str = "data\\global\\music\\";
/// Bytes of the first block the game writes into the stream buffer when
/// a track starts.
pub const FIRST_BLOCK_BYTES: usize = 32768;

/// One start of the music voice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicEvent {
    /// Playlist path (relative to [`MUSIC_DIR`]).
    pub track: String,
    /// The voice volume asked (110).
    pub volume: i32,
    /// `device_volume(volume)` in hundredths of a dB.
    pub device_volume: i32,
    /// The first [`FIRST_BLOCK_BYTES`] bytes of the decoded stream.
    pub first_block: Vec<u8>,
}

struct Voice {
    samples: Vec<i16>,
    pos: usize,
    channels: u16,
    gain_db: i32,
}

/// The music voice, its random source and the jukebox state.
pub struct FrontMusic {
    source: Arc<dyn FileSource>,
    jukebox: Jukebox,
    voice: Option<Voice>,
    rng: u32,
    expansion: bool,
    gain: i32,
    /// The player's Sound and Music Volume as an extra gain on the voice,
    /// hundredths of a dB (0: full; [`FrontMusic::set_levels`]).
    level_db: i32,
    pub events: Vec<MusicEvent>,
}

struct Host<'a> {
    source: &'a dyn FileSource,
    voice: &'a mut Option<Voice>,
    rng: &'a mut u32,
    gain: &'a mut i32,
    events: &'a mut Vec<MusicEvent>,
}

impl JukeboxHost for Host<'_> {
    fn rand(&mut self) -> u32 {
        // D2RUST's own source: an LCG step, the high bits.
        *self.rng = self.rng.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (*self.rng >> 16) & 0x7FFF
    }

    fn voice_playing(&self) -> bool {
        self.voice.as_ref().is_some_and(|v| v.pos < v.samples.len())
    }

    fn start_stream(&mut self, path: &str, volume: i32) -> bool {
        let _perf = d2_server::perf::span("audio");
        let Ok(full) = CanonicalPath::new(&format!("{MUSIC_DIR}{path}")) else {
            return false;
        };
        let Ok(bytes) = read_asset(self.source, full.as_str()) else {
            return false;
        };
        let Ok(wav) = d2_formats::wav::Wav::parse(&bytes) else {
            return false;
        };
        if !wav.is_playable() {
            return false;
        }
        let device = device_volume(volume);
        let first_block: Vec<u8> = wav
            .samples
            .iter()
            .take(FIRST_BLOCK_BYTES / 2)
            .flat_map(|s| s.to_le_bytes())
            .collect();
        self.events.push(MusicEvent {
            track: path.to_owned(),
            volume,
            device_volume: device,
            first_block,
        });
        *self.voice = Some(Voice {
            samples: wav.samples,
            pos: 0,
            channels: wav.channels,
            gain_db: device,
        });
        *self.gain = device;
        true
    }

    fn fade_out(&mut self, _ms: u32) {
        // Wall-clock fade: the Bevy edge ramps the gain; the model drops it.
    }

    fn stop_voice(&mut self) {
        *self.voice = None;
    }

    fn set_global_gain(&mut self, _g: i32) {}
}

impl FrontMusic {
    /// `options_music`: the `Options Music` setting; on, the jukebox is
    /// started as a front-end screen opens.
    pub fn new(
        source: Arc<dyn FileSource>,
        expansion: bool,
        options_music: bool,
        seed: u32,
    ) -> Self {
        let mut m = Self {
            source,
            jukebox: Jukebox::default(),
            voice: None,
            rng: seed,
            expansion,
            gain: 0,
            level_db: 0,
            events: Vec::new(),
        };
        if options_music {
            m.jukebox.start();
        }
        m
    }

    fn host(&mut self) -> (Host<'_>, &mut Jukebox) {
        (
            Host {
                source: self.source.as_ref(),
                voice: &mut self.voice,
                rng: &mut self.rng,
                gain: &mut self.gain,
                events: &mut self.events,
            },
            &mut self.jukebox,
        )
    }

    /// The service pass: pick a track when wanted and the voice is idle.
    pub fn service(&mut self) -> Option<&'static str> {
        let expansion = self.expansion;
        let (mut host, jb) = self.host();
        jb.service(&mut host, expansion)
    }

    /// Leaving the front end.
    pub fn leave(&mut self) {
        let (mut host, jb) = self.host();
        jb.leave_front_end(&mut host);
    }

    /// Whether a track is playing.
    pub fn playing(&self) -> bool {
        self.voice.as_ref().is_some_and(|v| v.pos < v.samples.len())
    }

    /// Follows the player's sound settings: `master` (Sound) and `music`
    /// (Music Volume), each 0–100, scale the menu music's voice volume as
    /// they scale music in game, and the result is heard at once (the
    /// main menu's Options screen changes them live). Both at 100 play the
    /// tracks at their own volume; either at 0 silences them. The
    /// recorded [`MusicEvent`]s keep the voice's own volume.
    pub fn set_levels(&mut self, master: i32, music: i32) {
        let own = super::frontend::FRONT_END_VOLUME;
        let v = own * music.clamp(0, 100) / 100 * master.clamp(0, 100) / 100;
        self.level_db = device_volume(v) - device_volume(own);
    }

    /// The extra gain [`FrontMusic::set_levels`] set, hundredths of a dB.
    pub fn level_db(&self) -> i32 {
        self.level_db
    }

    /// Linear amplitude of the voice from its device volume and the
    /// player's levels (hundredths of a dB); silent at the device floor.
    pub fn amplitude(&self) -> f32 {
        let db = self.voice.as_ref().map_or(self.gain, |v| v.gain_db) + self.level_db;
        if db <= -10_000 {
            return 0.0;
        }
        10f32.powf(db as f32 / 2000.0)
    }

    /// Next stereo frame (L, R) of the voice at its own rate, or silence.
    pub fn next_frame(&mut self) -> (i16, i16) {
        let Some(v) = self.voice.as_mut() else {
            return (0, 0);
        };
        if v.pos >= v.samples.len() {
            return (0, 0);
        }
        let l = v.samples[v.pos];
        let r = if v.channels == 2 {
            v.samples[v.pos + 1]
        } else {
            l
        };
        v.pos += usize::from(v.channels);
        (l, r)
    }

    /// Sample rate of the playing voice (22,050 for every shipped track).
    pub fn rate(&self) -> u32 {
        22_050
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::path::MemorySource;

    fn wav(frames: usize) -> Vec<u8> {
        let data: Vec<u8> = (0..frames * 2)
            .flat_map(|i| (i as i16).to_le_bytes())
            .collect();
        let mut b = b"RIFF".to_vec();
        b.extend((36 + data.len() as u32).to_le_bytes());
        b.extend(*b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        for v in [1u16, 2] {
            b.extend(v.to_le_bytes());
        }
        b.extend(22_050u32.to_le_bytes());
        b.extend(88_200u32.to_le_bytes());
        b.extend(4u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(*b"data");
        b.extend((data.len() as u32).to_le_bytes());
        b.extend(data);
        b
    }

    fn source() -> Arc<dyn FileSource> {
        let mut m = MemorySource::default();
        for p in super::super::frontend::LIST_A
            .iter()
            .chain(&super::super::frontend::LIST_B)
        {
            m.insert(&format!("{MUSIC_DIR}{p}"), wav(20_000));
        }
        Arc::new(m)
    }

    #[test]
    fn stream_load_records_audio_only_with_perf_and_preserves_playback() {
        use crate::app::perf_summary::{Frame, Report, Span};

        let mut baseline = FrontMusic::new(source(), true, true, 1);
        assert!(!d2_server::perf::enabled());
        assert_eq!(baseline.service(), Some("introedit.wav"));
        assert!(d2_server::perf::take_spans().is_empty());

        d2_server::perf::enable();
        let mut measured = FrontMusic::new(source(), true, true, 1);
        assert_eq!(measured.service(), Some("introedit.wav"));
        let spans = d2_server::perf::take_spans();
        assert_eq!(spans.len(), 1);
        let audio = spans[0];
        assert_eq!(audio.kind, "audio");

        // The report's existing JSON and hitch attribution accept the recorded span.
        let report = Report {
            version: 2,
            frames: vec![Frame {
                start_us: audio.start_us,
                interval: audio.dur_us.max(1),
                ..Default::default()
            }],
            spans: vec![Span {
                kind: audio.kind.to_owned(),
                start_us: audio.start_us,
                dur_us: audio.dur_us,
            }],
            ..Default::default()
        };
        let json: serde_json::Value = serde_json::from_str(&report.to_json(0)).unwrap();
        assert_eq!(json["spans"][0][0], "audio");
        assert_eq!(json["summary"]["hitches"][0]["causes"][0]["kind"], "audio");
        assert_eq!(json["summary"]["hitches"][0]["causes"][0]["count"], 1);

        assert_eq!(measured.events, baseline.events);
        assert_eq!(measured.rate(), baseline.rate());
        assert_eq!(measured.amplitude(), baseline.amplitude());
        for i in 0..=20_000 {
            assert_eq!(measured.playing(), baseline.playing(), "frame {i}");
            assert_eq!(measured.next_frame(), baseline.next_frame(), "frame {i}");
        }
        assert!(!measured.playing());
        assert_eq!(measured.service(), baseline.service());
        assert_eq!(measured.events, baseline.events);
    }

    #[test]
    fn first_pick_is_entry_zero_of_the_list_at_volume_110() {
        for (exp, first) in [(true, "introedit.wav"), (false, "common\\options.wav")] {
            let mut m = FrontMusic::new(source(), exp, true, 1);
            assert_eq!(m.service(), Some(first));
            let e = &m.events[0];
            assert_eq!((e.volume, e.device_volume), (110, -730));
            assert_eq!(e.first_block.len(), FIRST_BLOCK_BYTES);
            assert_eq!(&e.first_block[..4], &[0, 0, 1, 0]);
            assert!(m.playing());
            // not picked again while the voice plays
            assert_eq!(m.service(), None);
        }
    }

    #[test]
    fn options_music_off_plays_nothing() {
        let mut m = FrontMusic::new(source(), true, false, 1);
        assert_eq!(m.service(), None);
        assert!(m.events.is_empty() && !m.playing());
    }

    #[test]
    fn leaving_stops_the_voice() {
        let mut m = FrontMusic::new(source(), true, true, 1);
        m.service();
        m.leave();
        assert!(!m.playing());
        assert_eq!(m.next_frame(), (0, 0));
    }

    #[test]
    fn the_players_volumes_scale_the_music_at_once() {
        let mut m = FrontMusic::new(source(), true, true, 1);
        m.service();
        let full = m.amplitude();
        assert!(full > 0.0);
        // Sound and Music at 100: the track's own volume.
        m.set_levels(100, 100);
        assert_eq!((m.level_db(), m.amplitude()), (0, full));
        // Lower Music Volume, then lower Sound too: quieter each time.
        m.set_levels(100, 50);
        let half = m.amplitude();
        assert!(half < full);
        m.set_levels(50, 50);
        assert!(m.amplitude() < half);
        // Either at 0: silent.
        m.set_levels(0, 100);
        assert_eq!(m.amplitude(), 0.0);
        m.set_levels(100, 0);
        assert_eq!(m.amplitude(), 0.0);
        // The recorded start keeps the track's own volume.
        assert_eq!(m.events[0].volume, super::super::frontend::FRONT_END_VOLUME);
    }
}
