//! Mutation-testing gaps of the audio core: checks the
//! existing tests left unasserted.

use std::sync::Arc;

use d2_client::audio::{
    AudioEngine, Cue, Sound, SoundBank, SoundId, Trigger, TriggerQueue, TriggerSource, UnityGain,
    Unlimited, VoiceLog, VoiceParams,
};

struct NoBank;

impl SoundBank for NoBank {
    fn file(&self, _: SoundId) -> Option<Arc<str>> {
        None
    }

    fn samples(&self, _: SoundId) -> Option<Arc<Sound>> {
        None
    }
}

fn start(tick: u32) -> Cue {
    Cue::Start(Trigger {
        tick,
        source: TriggerSource::Ui,
        sound: SoundId(0),
        params: VoiceParams {
            vol: 0,
            pan: 0,
            looped: false,
            priority: 0,
            group: 0,
        },
        cause: String::new(),
    })
}

/// `len` and `is_empty` report the pending cues, and `take_due` removes
/// what it returns.
#[test]
fn queue_counts_pending_cues() {
    let mut q = TriggerQueue::new();
    assert!(q.is_empty());
    assert_eq!(q.len(), 0);
    q.push(start(3));
    q.push(start(7));
    assert!(!q.is_empty());
    assert_eq!(q.len(), 2);
    assert_eq!(q.take_due(3).len(), 1);
    assert_eq!(q.len(), 1);
}

/// The audio clock is slaved to the presented tick, and a frame
/// without a server tick presents the same tick again (the bridge's
/// replay). Only a smaller tick goes back.
#[test]
fn presenting_the_same_tick_again_is_not_backwards() {
    let mut e = AudioEngine::new(Box::new(NoBank), Box::new(UnityGain), Box::new(Unlimited));
    e.present(5).unwrap();
    e.present(5).unwrap();
    assert!(e.present(4).is_err());
    assert!(e.take_errors().is_empty());
}

/// A decoded sound keeps its rate, channel count and samples.
#[test]
fn sound_keeps_its_parameters() {
    let s = Sound::new(22_050, 2, vec![-3, 4, 5, -6]).unwrap();
    assert_eq!(s.rate(), 22_050);
    assert_eq!(s.channels(), 2);
    assert_eq!(s.samples(), &[-3, 4, 5, -6]);
    assert_eq!(s.frames(), 2);
}

fn log_with_cause(escape: &str) -> String {
    format!(
        "{}\n{{\"tick\":1,\"kind\":\"start\",\"file\":\"a.wav\",\"vol\":0,\"pan\":0,\"looped\":false,\"error\":false,\"cause\":\"{escape}\"}}\n",
        d2_client::audio::log::header()
    )
}

/// Log format (module doc): `\u00XX` escapes only chars below U+0020,
/// so the strict parser refuses ` `, which the writer never emits.
#[test]
fn log_parser_refuses_a_space_escape() {
    let ok = VoiceLog::parse(&log_with_cause("\\u001f")).unwrap();
    assert_eq!(ok.events[0].cause, "\u{1f}");
    assert!(VoiceLog::parse(&log_with_cause("\\u0020")).is_err());
}

/// 32.32 phase accumulator, nearest sample (frame `floor(phase)`); a
/// looped voice wraps to frame 0 keeping the accumulated fraction (module
/// doc of `mixer`), so output frame `k` plays source frame
/// `floor(k × step / 2^32) mod frames`. A rate that does not divide the
/// output rate makes every wrap land between source frames.
#[test]
fn looped_voice_wraps_with_the_phase_fraction() {
    use d2_client::audio::{mix, CueId, Gains, Voice, BLOCK_FRAMES, GAIN_UNITY, OUTPUT_RATE};

    let rate = 30_000u32;
    let src: Vec<i16> = vec![100, -200, 300];
    let sound = Arc::new(Sound::new(rate, 1, src.clone()).unwrap());
    let params = VoiceParams {
        vol: 0,
        pan: 0,
        looped: true,
        priority: 0,
        group: 0,
    };
    let gains = Gains::new(GAIN_UNITY, GAIN_UNITY, GAIN_UNITY).unwrap();
    let mut voices = vec![Voice::new(CueId(0), "a.wav".into(), sound, params, gains)];
    let step = (u64::from(rate) << 32) / u64::from(OUTPUT_RATE);
    let frames = src.len() as u64;
    let mut expected = Vec::new();
    for block in 0..3u64 {
        let out = mix(&mut voices);
        for k in 0..BLOCK_FRAMES as u64 {
            let n = block * BLOCK_FRAMES as u64 + k;
            let s = src[(((n * step) >> 32) % frames) as usize];
            expected.push((s, s));
        }
        let got: Vec<(i16, i16)> = out.chunks(2).map(|c| (c[0], c[1])).collect();
        assert_eq!(
            got,
            expected[(block as usize * BLOCK_FRAMES)..],
            "block {block}"
        );
    }
    assert_eq!(voices.len(), 1);
}

/// The mixer (`mix`: "voices that end are removed"): a one-shot voice whose last
/// frame is the block's last frame is gone after that block, so a later
/// stop of it logs nothing (`AudioEngine::stop`).
#[test]
fn one_shot_ending_on_a_block_edge_is_removed_with_that_block() {
    use d2_client::audio::{mix, CueId, Gains, Voice, BLOCK_FRAMES, GAIN_UNITY, OUTPUT_RATE};

    let sound = Arc::new(Sound::new(OUTPUT_RATE, 1, vec![1; BLOCK_FRAMES]).unwrap());
    let params = VoiceParams {
        vol: 0,
        pan: 0,
        looped: false,
        priority: 0,
        group: 0,
    };
    let gains = Gains::new(GAIN_UNITY, GAIN_UNITY, GAIN_UNITY).unwrap();
    let mut voices = vec![Voice::new(CueId(0), "a.wav".into(), sound, params, gains)];
    let out = mix(&mut voices);
    assert!(out.iter().all(|&s| s == 1));
    assert!(voices.is_empty());
}
