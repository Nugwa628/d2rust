//! `play --audio-dump FILE` and `d2-client audio-mix`: the D2RUST side of
//! audio-diff. A [`Stepper`] drives an [`AudioEngine`] tick by tick with no
//! audio device: it presents each sound tick in turn and mixes that tick's
//! share of output frames ([`FRAMES_PER_TICK`] per 40 ms tick, in
//! [`BLOCK_FRAMES`] blocks, never more than the ticks presented so far), so
//! the output does not depend on frame pacing. Each new voice, each voice
//! log record after the starts and each tick's mixed output becomes one
//! JSON line ([`Record`]).
//!
//! `audio-mix` feeds the same stepper with a voice list from outside
//! (`audio_diff.py` writes it from a reference capture): the captured
//! samples played through D2RUST's mixer, so the mixed check compares the two
//! voice sets through one mixer.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use bevy::prelude::Resource;
use sha2::{Digest, Sha256};

use crate::audio::sound_table::volume::device_occluded;
use crate::audio::sound_table::DeviceGain;
use crate::audio::{
    AudioEngine, AudioError, Cue, ParamChange, Sound, SoundBank, SoundId, Stop, StopTarget,
    Trigger, TriggerSource, Unlimited, VoiceEvent, VoiceKind, VoiceParams, BLOCK_FRAMES,
    OUTPUT_RATE,
};

/// Dump format.
pub const DUMP_FORMAT: &str = "d2rust-audio-dump";
pub const DUMP_VERSION: u32 = 1;
/// Output frames per 40 ms tick at [`OUTPUT_RATE`].
pub const FRAMES_PER_TICK: u64 = OUTPUT_RATE as u64 * 40 / 1000;

/// One dump line.
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    /// A voice the mixer started, first seen after the block it started in.
    Start {
        tick: u32,
        file: String,
        vol: i32,
        pan: i32,
        looped: bool,
        occlusion: f32,
        loop_start: u64,
        loop_end: u64,
        channels: u16,
        frames: usize,
        sha256: String,
        dev_vol: i32,
        dev_pan: i32,
    },
    /// A voice log record other than a successful start (stop, param, a
    /// failed start).
    Log(VoiceEvent),
    /// The frames mixed for one tick.
    Mix {
        tick: u32,
        frames: u64,
        sha256: String,
    },
}

/// `trunc(−2000 × log10(full / x))` hundredths of a dB, −10,000 at or below
/// 0.0001, 0 at or above `full − 0.0001` (the device conversion; truncation
/// measured). The game takes f32 arguments, divides and takes the log on
/// the x87 stack, stores the product as f32 and the caller truncates it; D2RUST
/// does the x87 part in f64 and keeps the f32 store. For every integer `x` of
/// 255 or 127 the f64, f32-store and all-f32 paths truncate alike.
pub fn device_db(x: f64, full: f64) -> i32 {
    if x <= 0.0001 {
        -10_000
    } else if x >= full - 0.0001 {
        0
    } else {
        ((-2000.0 * (full / x).log10()) as f32).trunc() as i32
    }
}

/// The DirectSound volume of a voice.
pub fn device_volume(vol: i32, occlusion: f32) -> i32 {
    device_db(f64::from(device_occluded(vol, occlusion).max(0)), 255.0)
}

/// The DirectSound pan: `pan < 128` attenuates the right side
/// (negative), `pan > 128` the left (positive).
pub fn device_pan(pan: i32) -> i32 {
    match pan {
        p if p < 128 => device_db(f64::from(p), 127.0),
        p if p > 128 => -device_db(f64::from(255 - p), 127.0),
        _ => 0,
    }
}

/// Lowercase hex sha256 of i16 samples as little-endian bytes.
pub fn samples_sha256(samples: &[i16]) -> String {
    let mut h = Sha256::new();
    for s in samples {
        h.update(s.to_le_bytes());
    }
    hex(&h.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Presents ticks one by one and mixes each tick's frames.
#[derive(Default)]
pub struct Stepper {
    first: Option<u32>,
    last: Option<u32>,
    mixed: u64,
    log_at: usize,
    /// Keep each started sound's samples once (by digest) for
    /// [`take_pcm`](Self::take_pcm).
    pub keep_pcm: bool,
    pcm_seen: std::collections::BTreeSet<String>,
    pcm: Vec<(String, Vec<i16>)>,
}

impl Stepper {
    /// Ticks `last + 1..= t` (only `t` the first time); returns their records.
    pub fn advance(&mut self, engine: &mut AudioEngine, t: u32) -> Result<Vec<Record>, AudioError> {
        let from = match self.last {
            Some(l) if t <= l => return Ok(Vec::new()),
            Some(l) => l + 1,
            None => t,
        };
        let first = *self.first.get_or_insert(from);
        engine.record_starts(true);
        let mut out = Vec::new();
        for tick in from..=t {
            engine.present(tick)?;
            let budget = (u64::from(tick - first) + 1) * FRAMES_PER_TICK;
            let mut h = Sha256::new();
            let mut frames = 0;
            while self.mixed + BLOCK_FRAMES as u64 <= budget {
                let block = engine.mix_block();
                for s in block {
                    h.update(s.to_le_bytes());
                }
                self.mixed += BLOCK_FRAMES as u64;
                frames += BLOCK_FRAMES as u64;
                self.collect(engine, &mut out);
            }
            if frames > 0 {
                out.push(Record::Mix {
                    tick,
                    frames,
                    sha256: hex(&h.finalize()),
                });
            }
            self.last = Some(tick);
        }
        Ok(out)
    }

    /// Samples of the sounds first started since the last call (digest,
    /// samples), when [`keep_pcm`](Self::keep_pcm) is set.
    pub fn take_pcm(&mut self) -> Vec<(String, Vec<i16>)> {
        std::mem::take(&mut self.pcm)
    }

    /// The voices started in the last block (their device state read from
    /// the live voice when it still plays), then the log records since the
    /// last call that are not successful starts.
    fn collect(&mut self, engine: &mut AudioEngine, out: &mut Vec<Record>) {
        for (tick, started) in engine.take_started() {
            let live = engine.voices().iter().find(|v| v.id() == started.id());
            let v = live.unwrap_or(&started);
            let p = v.params();
            let s = v.sound();
            let sha256 = samples_sha256(s.samples());
            if self.keep_pcm && self.pcm_seen.insert(sha256.clone()) {
                self.pcm.push((sha256.clone(), s.samples().to_vec()));
            }
            out.push(Record::Start {
                tick,
                file: v.file().to_owned(),
                vol: p.vol,
                pan: p.pan,
                looped: p.looped,
                occlusion: v.occlusion(),
                loop_start: v.loop_start(),
                loop_end: v.loop_end(),
                channels: s.channels(),
                frames: s.frames(),
                sha256,
                dev_vol: device_volume(p.vol, v.occlusion()),
                dev_pan: device_pan(p.pan),
            });
        }
        let events = &engine.log().events;
        for e in &events[self.log_at..] {
            if !(e.kind == VoiceKind::Start && !e.error) {
                out.push(Record::Log(e.clone()));
            }
        }
        self.log_at = events.len();
    }
}

/// One record as its JSON line (keys in a fixed order).
pub fn record_line(r: &Record, server_tick: Option<u64>) -> String {
    use serde_json::json;
    let v = match r {
        Record::Start {
            tick,
            file,
            vol,
            pan,
            looped,
            occlusion,
            loop_start,
            loop_end,
            channels,
            frames,
            sha256,
            dev_vol,
            dev_pan,
        } => json!({"k": "start", "tick": tick, "server_tick": server_tick, "file": file,
                    "vol": vol, "pan": pan, "looped": looped, "occ": occlusion,
                    "loop_start": loop_start, "loop_end": loop_end, "channels": channels, "frames": frames,
                    "sha256": sha256, "dev_vol": dev_vol, "dev_pan": dev_pan}),
        Record::Log(e) => {
            let kind = match e.kind {
                VoiceKind::Start => "start-failed",
                VoiceKind::Stop => "stop",
                VoiceKind::Param => "param",
            };
            json!({"k": kind, "tick": e.tick, "server_tick": server_tick, "file": e.file,
                   "vol": e.vol, "pan": e.pan, "looped": e.looped, "cause": e.cause})
        }
        Record::Mix {
            tick,
            frames,
            sha256,
        } => json!({"k": "mix", "tick": tick, "server_tick": server_tick, "frames": frames,
                    "sha256": sha256}),
    };
    v.to_string()
}

/// The header line.
pub fn header_line() -> String {
    serde_json::json!({"format": DUMP_FORMAT, "version": DUMP_VERSION, "rate": OUTPUT_RATE,
                       "block": BLOCK_FRAMES, "frames_per_tick": FRAMES_PER_TICK})
    .to_string()
}

/// `play --audio-dump FILE [--audio-ticks N]`.
#[derive(Resource)]
pub struct AudioDump {
    out: std::io::BufWriter<std::fs::File>,
    /// `FILE.pcm/`: each started sound's samples, `<sha256>.pcm` (the
    /// user's game data: a work folder only, never the repository).
    pcm_dir: std::path::PathBuf,
    pub stepper: Stepper,
    /// Exit once the server tick reaches this.
    pub until: Option<u64>,
}

impl AudioDump {
    pub fn create(path: &Path, until: Option<u64>) -> std::io::Result<Self> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(out, "{}", header_line())?;
        out.flush()?;
        let mut pcm_dir = path.as_os_str().to_owned();
        pcm_dir.push(".pcm");
        let pcm_dir = std::path::PathBuf::from(pcm_dir);
        std::fs::create_dir_all(&pcm_dir)?;
        Ok(AudioDump {
            out,
            pcm_dir,
            stepper: Stepper {
                keep_pcm: true,
                ..Stepper::default()
            },
            until,
        })
    }

    pub fn write(&mut self, records: &[Record], server_tick: u64) -> std::io::Result<()> {
        for r in records {
            writeln!(self.out, "{}", record_line(r, Some(server_tick)))?;
        }
        for (sha, samples) in self.stepper.take_pcm() {
            let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
            std::fs::write(self.pcm_dir.join(format!("{sha}.pcm")), bytes)?;
        }
        self.out.flush()
    }
}

// --- audio-mix ---------------------------------------------------------

/// One voice of a list (`audio-voices-1`, written by audio_diff.py): start
/// tick, label, channels, the samples (i16 LE file), device-side integer volume
/// and pan, later (tick, vol, pan) changes, stop tick.
#[derive(Clone, Debug, PartialEq)]
pub struct ListVoice {
    pub tick: u32,
    pub label: String,
    pub channels: u16,
    pub pcm: std::path::PathBuf,
    pub vol: i32,
    pub pan: i32,
    pub params: Vec<(u32, i32, i32)>,
    pub stop: Option<u32>,
}

struct ListBank(Vec<(Arc<str>, Arc<Sound>)>);

impl SoundBank for ListBank {
    fn file(&self, id: SoundId) -> Option<Arc<str>> {
        self.0.get(id.0 as usize).map(|(f, _)| Arc::clone(f))
    }

    fn samples(&self, id: SoundId) -> Option<Arc<Sound>> {
        self.0.get(id.0 as usize).map(|(_, s)| Arc::clone(s))
    }
}

/// Parses an `audio-voices-1` file (header line then one voice per line).
pub fn parse_voice_list(text: &str, base: &Path) -> Result<Vec<ListVoice>, String> {
    let mut lines = text.lines();
    let head: serde_json::Value = serde_json::from_str(lines.next().ok_or("empty voice list")?)
        .map_err(|e| format!("line 1: {e}"))?;
    if head["format"] != "audio-voices" || head["version"] != 1 {
        return Err("line 1: not an audio-voices version 1 header".into());
    }
    let mut out = Vec::new();
    for (i, line) in lines.enumerate() {
        let n = i + 2;
        let v: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("line {n}: {e}"))?;
        let int = |k: &str| {
            v[k].as_i64()
                .ok_or_else(|| format!("line {n}: {k} is not an integer"))
        };
        let params = v["params"]
            .as_array()
            .ok_or_else(|| format!("line {n}: params is not a list"))?
            .iter()
            .map(|p| {
                let a = p.as_array().filter(|a| a.len() == 3);
                let g = |j: usize| a.and_then(|a| a[j].as_i64());
                match (g(0), g(1), g(2)) {
                    (Some(t), Some(vv), Some(pp)) => Ok((t as u32, vv as i32, pp as i32)),
                    _ => Err(format!("line {n}: a params entry is not [tick, vol, pan]")),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        out.push(ListVoice {
            tick: int("tick")? as u32,
            label: v["label"]
                .as_str()
                .ok_or_else(|| format!("line {n}: label"))?
                .to_owned(),
            channels: int("channels")? as u16,
            pcm: base.join(v["pcm"].as_str().ok_or_else(|| format!("line {n}: pcm"))?),
            vol: int("vol")? as i32,
            pan: int("pan")? as i32,
            params,
            stop: v["stop"].as_i64().map(|t| t as u32),
        });
    }
    Ok(out)
}

/// The mixed output of a voice list through D2RUST's mixer with the device
/// gain: the `mix` records for ticks `first..= last`.
pub fn mix_list(voices: &[ListVoice], last: u32) -> Result<Vec<Record>, String> {
    let mut sounds = Vec::new();
    for v in voices {
        let bytes = std::fs::read(&v.pcm).map_err(|e| format!("{}: {e}", v.pcm.display()))?;
        let samples: Vec<i16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect();
        let sound =
            Sound::new(22_050, v.channels, samples).map_err(|e| format!("{}: {e}", v.label))?;
        sounds.push((Arc::<str>::from(v.label.as_str()), Arc::new(sound)));
    }
    let mut engine = AudioEngine::new(
        Box::new(ListBank(sounds)),
        Box::new(DeviceGain),
        Box::new(Unlimited),
    );
    let mut ids = Vec::new();
    for (i, v) in voices.iter().enumerate() {
        let params = VoiceParams {
            vol: v.vol,
            pan: v.pan,
            looped: false,
            priority: 0,
            group: 0,
        };
        ids.push(engine.queue_mut().push(Cue::Start(Trigger {
            tick: v.tick,
            source: TriggerSource::Sim,
            sound: SoundId(i as u32),
            params,
            cause: v.label.clone(),
        })));
    }
    for (v, id) in voices.iter().zip(&ids) {
        for &(tick, vol, pan) in &v.params {
            engine.queue_mut().push(Cue::Param(ParamChange {
                tick,
                target: *id,
                vol,
                pan,
                cause: String::new(),
            }));
        }
        if let Some(tick) = v.stop {
            engine.queue_mut().push(Cue::Stop(Stop {
                tick,
                target: StopTarget::Voice(*id),
                cause: String::new(),
            }));
        }
    }
    let first = voices.iter().map(|v| v.tick).min().unwrap_or(0);
    let mut st = Stepper::default();
    let recs = st
        .advance(&mut engine, first.min(last))
        .and_then(|mut r| {
            r.extend(st.advance(&mut engine, last)?);
            Ok(r)
        })
        .map_err(|e| e.to_string())?;
    Ok(recs
        .into_iter()
        .filter(|r| matches!(r, Record::Mix { .. }))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Bank(BTreeMap<u32, (Arc<str>, Arc<Sound>)>);

    impl SoundBank for Bank {
        fn file(&self, id: SoundId) -> Option<Arc<str>> {
            self.0.get(&id.0).map(|(f, _)| Arc::clone(f))
        }
        fn samples(&self, id: SoundId) -> Option<Arc<Sound>> {
            self.0.get(&id.0).map(|(_, s)| Arc::clone(s))
        }
    }

    fn engine() -> AudioEngine {
        let mut m = BTreeMap::new();
        m.insert(
            1,
            (
                Arc::<str>::from("a.wav"),
                Arc::new(Sound::new(22_050, 1, vec![1000; 4000]).unwrap()),
            ),
        );
        AudioEngine::new(Box::new(Bank(m)), Box::new(DeviceGain), Box::new(Unlimited))
    }

    fn start(tick: u32) -> Cue {
        Cue::Start(Trigger {
            tick,
            source: TriggerSource::Sim,
            sound: SoundId(1),
            params: VoiceParams {
                vol: 255,
                pan: 128,
                looped: false,
                priority: 0,
                group: 0,
            },
            cause: "t".into(),
        })
    }

    #[test]
    fn frames_per_tick_is_40_ms() {
        assert_eq!(FRAMES_PER_TICK, 1764);
    }

    #[test]
    fn device_curves_match_the_measured_values() {
        // Values a reference capture sent (SetVolume / SetPan).
        assert_eq!(device_db(255.0, 255.0), 0);
        assert_eq!(device_db(0.0, 255.0), -10_000);
        let vols: Vec<i32> = (0..=255).map(|v| device_db(f64::from(v), 255.0)).collect();
        for measured in [-128, -885, -415, -1187, -754, -2460, -2168, -78] {
            assert!(vols.contains(&measured), "{measured}");
        }
        assert_eq!(device_pan(128), 0);
        assert_eq!(device_pan(0), -10_000);
        assert_eq!(device_pan(255), 10_000);
        let pans: Vec<i32> = (0..=255).map(device_pan).collect();
        for measured in [1377, -595, -492, -348, -252] {
            assert!(pans.contains(&measured), "{measured}");
        }
        assert_eq!(device_volume(255, 0.5), device_db(127.0, 255.0));
    }

    #[test]
    fn stepper_mixes_each_ticks_share_and_reports_the_start() {
        let mut e = engine();
        e.queue_mut().push(start(5));
        let mut st = Stepper::default();
        let r = st.advance(&mut e, 3).unwrap();
        // tick 3: 1764 frames → 3 blocks (1536)
        assert_eq!(
            r.iter()
                .filter_map(|r| match r {
                    Record::Mix { tick, frames, .. } => Some((*tick, *frames)),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec![(3, 1536)]
        );
        let r = st.advance(&mut e, 6).unwrap();
        let starts: Vec<_> = r
            .iter()
            .filter_map(|r| match r {
                Record::Start {
                    tick,
                    file,
                    frames,
                    dev_vol,
                    ..
                } => Some((*tick, file.clone(), *frames, *dev_vol)),
                _ => None,
            })
            .collect();
        assert_eq!(starts, vec![(5, "a.wav".to_owned(), 4000, 0)]);
        assert!(st.take_pcm().is_empty(), "keep_pcm is off");
        let mixes: Vec<_> = r
            .iter()
            .filter_map(|r| match r {
                Record::Mix { tick, frames, .. } => Some((*tick, *frames)),
                _ => None,
            })
            .collect();
        // budget 4 × 1764 = 7056 → total 13 blocks (6656): ticks 4, 5, 6
        assert_eq!(mixes, vec![(4, 1536), (5, 2048), (6, 1536)]);
        assert_eq!(st.advance(&mut e, 6).unwrap(), Vec::new());
        let mut e = engine();
        e.queue_mut().push(start(1));
        e.queue_mut().push(start(2));
        let mut st = Stepper {
            keep_pcm: true,
            ..Stepper::default()
        };
        st.advance(&mut e, 4).unwrap();
        let pcm = st.take_pcm();
        assert_eq!(pcm.len(), 1, "one entry per distinct sound");
        assert_eq!(pcm[0].0, samples_sha256(&[1000; 4000]));
    }

    #[test]
    fn same_voices_same_mix_through_the_list_path() {
        let dir = std::env::temp_dir().join(format!("audio-dump-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pcm: Vec<u8> = [1000i16; 4000]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        std::fs::write(dir.join("v.pcm"), pcm).unwrap();
        let list = format!(
            "{}\n{}\n",
            r#"{"format":"audio-voices","version":1}"#,
            r#"{"tick":5,"label":"a.wav","channels":1,"pcm":"v.pcm","vol":255,"pan":128,"params":[],"stop":null}"#
        );
        let voices = parse_voice_list(&list, &dir).unwrap();
        assert_eq!(voices[0].tick, 5);
        let a = mix_list(&voices, 8).unwrap();
        // the same voice through the engine path
        let mut e = engine();
        e.queue_mut().push(start(5));
        let mut st = Stepper::default();
        let b: Vec<_> = st
            .advance(&mut e, 5)
            .unwrap()
            .into_iter()
            .chain(st.advance(&mut e, 8).unwrap())
            .filter(|r| matches!(r, Record::Mix { .. }))
            .collect();
        assert_eq!(a, b);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(parse_voice_list("{}\n", &dir).is_err());
    }

    #[test]
    fn lines_have_the_fixed_keys() {
        let l = record_line(
            &Record::Mix {
                tick: 2,
                frames: 512,
                sha256: "ab".into(),
            },
            Some(9),
        );
        let v: serde_json::Value = serde_json::from_str(&l).unwrap();
        assert_eq!(v["k"], "mix");
        assert_eq!(v["server_tick"], 9);
        let h: serde_json::Value = serde_json::from_str(&header_line()).unwrap();
        assert_eq!(h["format"], DUMP_FORMAT);
        assert_eq!(h["frames_per_tick"], 1764);
    }
}
