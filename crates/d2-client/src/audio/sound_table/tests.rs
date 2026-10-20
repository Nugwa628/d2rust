//! Test vectors of the sound table (synthetic tables, no game files) and
//! the ignored checks on a game install (`D2_GAME_DIR`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use d2_data::sounds::{SoundEnvironRow, SoundRow};
use d2_sim::rng::Seed;

use super::system::{Fade, SoundWorld};
use super::volume::*;
use super::*;
use crate::audio::calls::{SoundCalls, FLAG_EXACT};
use crate::audio::{
    AudioEngine, Cue, GainCurve, Sound, SoundBank, SoundId, StopTarget, TriggerQueue, Unlimited,
    VoiceKind,
};
use crate::bridge::world::UnitKey;

// --- fixtures -----------------------------------------------------------

fn row(name: &str) -> SoundRow {
    SoundRow {
        file_name: name.as_bytes().to_vec(),
        volume: 255,
        falloff: 1,
        blocks: [-1, -1, -1],
        ..SoundRow::default()
    }
}

/// `n` rows `s1.wav` …; row 0 is `none.wav` with everything 0.
fn rows(n: usize) -> Vec<SoundRow> {
    let mut v: Vec<SoundRow> = (0..n).map(|i| row(&format!("s{i}.wav"))).collect();
    v[0] = SoundRow {
        file_name: b"none.wav".to_vec(),
        ..SoundRow::default()
    };
    v
}

fn table(rows: Vec<SoundRow>) -> SoundTableData {
    SoundTableData::new(rows, &[])
}

/// Every id has a 1-second mono 1,000 Hz sample (25 sound ticks) except
/// the `missing` ones; `stereo` ids have a stereo file; `sizes` are file
/// sizes for the cache (others: the data plus 44 bytes).
struct Bank {
    missing: BTreeSet<u32>,
    stereo: BTreeSet<u32>,
    sizes: BTreeMap<u32, u64>,
    frames: usize,
}

impl SoundBank for Bank {
    fn file(&self, id: SoundId) -> Option<Arc<str>> {
        Some(format!("s{}.wav", id.0).into())
    }
    fn samples(&self, id: SoundId) -> Option<Arc<Sound>> {
        if self.missing.contains(&id.0) {
            return None;
        }
        let ch = if self.stereo.contains(&id.0) { 2 } else { 1 };
        Some(Arc::new(
            Sound::new(1_000, ch, vec![0; self.frames * usize::from(ch)]).unwrap(),
        ))
    }
    fn file_size(&self, id: SoundId) -> Option<u64> {
        self.sizes.get(&id.0).copied()
    }
}

fn bank() -> Box<Bank> {
    Box::new(Bank {
        missing: BTreeSet::new(),
        stereo: BTreeSet::new(),
        sizes: BTreeMap::new(),
        frames: 1_000,
    })
}

/// Mono channels of mixer mode 0 (slots 4–15).
const MONO_SLOTS: usize = 12;

fn system(rows: Vec<SoundRow>) -> SoundSystem {
    SoundSystem::new(table(rows), bank())
}

const PLAYER: UnitKey = UnitKey::new(0, 1);
const MONSTER: UnitKey = UnitKey::new(1, 7);
const MONSTER2: UnitKey = UnitKey::new(1, 8);

struct World {
    positions: BTreeMap<UnitKey, (i32, i32)>,
    client_only: BTreeMap<UnitKey, (i32, i32)>,
    blocked: BTreeSet<UnitKey>,
    indoors: bool,
    duck: bool,
    seed: Option<Seed>,
}

impl World {
    fn new() -> Self {
        World {
            positions: [(PLAYER, (1000, 1000)), (MONSTER, (1000, 1000))].into(),
            client_only: BTreeMap::new(),
            blocked: BTreeSet::new(),
            indoors: false,
            duck: false,
            seed: Some(Seed::init()),
        }
    }
}

impl SoundWorld for World {
    fn local_player(&self) -> Option<UnitKey> {
        Some(PLAYER)
    }
    fn position(&self, unit: UnitKey) -> Option<(i32, i32)> {
        self.positions.get(&unit).copied()
    }
    fn client_only_position(&self, unit: UnitKey) -> Option<(i32, i32)> {
        self.client_only.get(&unit).copied()
    }
    fn blocked(&self, unit: UnitKey) -> bool {
        self.blocked.contains(&unit)
    }
    fn indoors(&self) -> bool {
        self.indoors
    }
    fn state_duck(&self) -> bool {
        self.duck
    }
    fn client_seed(&mut self) -> Option<&mut Seed> {
        self.seed.as_mut()
    }
}

fn starts(q: &mut TriggerQueue) -> Vec<(u32, u32, i32, i32)> {
    q.take_due(u32::MAX)
        .into_iter()
        .filter_map(|(_, c)| match c {
            Cue::Start(t) => Some((t.tick, t.sound.0, t.params.vol, t.params.pan)),
            _ => None,
        })
        .collect()
}

fn cues(q: &mut TriggerQueue) -> Vec<Cue> {
    q.take_due(u32::MAX).into_iter().map(|(_, c)| c).collect()
}

fn ticks(sys: &mut SoundSystem, w: &mut World, q: &mut TriggerQueue, n: u32) {
    for _ in 0..n {
        sys.run_tick(w, q);
    }
}

// --- table ------------------------------------------------------------------

#[test]
fn group_pass_vectors() {
    let mut r = rows(8);
    for (i, g) in [3, 0, 0, 0, 2, 0, 0].into_iter().enumerate() {
        r[i + 1].group_size = g;
    }
    let t = table(r);
    let bases: Vec<i32> = (1..8).map(|i| t.base(i)).collect();
    assert_eq!(bases, [1, 1, 1, 4, 5, 5, 7]);
    let sizes: Vec<u8> = (1..8).map(|i| t.get(i).unwrap().group_size).collect();
    assert_eq!(sizes, [3, 1, 1, 1, 2, 1, 1]);
    assert_eq!((t.base(0), t.get(0).unwrap().group_size), (0, 0));

    let mut r = rows(8);
    r[1].group_size = 5;
    r[2].group_size = 5;
    let t = table(r);
    let bases: Vec<i32> = (1..8).map(|i| t.base(i)).collect();
    assert_eq!(bases, [1, 1, 2, 2, 2, 2, 7]);
}

#[test]
fn block_count() {
    let mut r = rows(5);
    r[1].blocks = [10, -1, 5];
    r[2].blocks = [10, 20, -1];
    r[3].blocks = [1, 2, 3];
    r[4].blocks = [-1, 2, 3];
    let t = table(r);
    let c: Vec<u8> = (1..5).map(|i| t.get(i).unwrap().block_count).collect();
    assert_eq!(c, [1, 2, 3, 0]);
}

#[test]
fn variant_pick_vectors() {
    let mut r = rows(110);
    r[100].group_size = 4;
    r[50].group_size = 2;
    let mut t = table(r);
    let mut seed = Seed::init();
    let v = t.pick_variant(100, &mut |n| seed.roll(n));
    assert_eq!(v, 103);
    assert_eq!(seed, Seed::new(1_791_398_751, 0));

    t.record_history(50, 50);
    let mut seed = Seed::init();
    let mut draws = Vec::new();
    let v = t.pick_variant(50, &mut |n| {
        let x = seed.roll(n);
        draws.push((n, x));
        x
    });
    assert_eq!(draws, [(3, 0), (2, 1)]);
    assert_eq!(v, 51);
    assert_eq!(seed, Seed::new(791_599_131, 747_178_749));

    // n ≤ 1: no draw.
    let mut called = false;
    assert_eq!(
        t.pick_variant(7, &mut |_| {
            called = true;
            0
        }),
        7
    );
    assert!(!called);
}

#[test]
fn variant_avoids_history_and_history_updates() {
    let mut r = rows(20);
    r[10].group_size = 4;
    let mut t = table(r);
    t.record_history(10, 11);
    t.record_history(10, 12);
    assert_eq!(t.get(10).unwrap().history, [12, 11]);
    // Scripted rolls: 1 (=11, in history), 2 (=12, in history), 3 → 13.
    let mut script = vec![1, 2, 3].into_iter();
    let v = t.pick_variant(10, &mut |_| script.next().unwrap());
    assert_eq!(v, 13);
}

#[test]
fn nested_group_variants_run_past_the_group() {
    let mut r = rows(10);
    r[1].group_size = 5;
    r[2].group_size = 5;
    let t = table(r);
    // id 2 picks 2 + roll(5): up to 6, past 1's group (1..5).
    assert_eq!(t.pick_variant(2, &mut |_| 4), 6);
}

#[test]
fn path_vectors() {
    let mut r = rows(4700);
    r[1].file_name = b"cursor\\pass.wav".to_vec();
    let t = table(r);
    assert_eq!(
        t.path(1).unwrap().unwrap(),
        "DATA\\GLOBAL\\SFX\\cursor\\pass.wav"
    );
    assert_eq!(
        t.path(2933).unwrap().unwrap(),
        "DATA\\GLOBAL\\SFX\\s2933.wav"
    );
    assert_eq!(
        t.path(2934).unwrap().unwrap(),
        "DATA\\LOCAL\\SFX\\s2934.wav"
    );
    assert_eq!(
        t.path(4656).unwrap().unwrap(),
        "DATA\\LOCAL\\SFX\\s4656.wav"
    );
    assert_eq!(
        t.path(4657).unwrap().unwrap(),
        "DATA\\GLOBAL\\MUSIC\\s4657.wav"
    );
    assert_eq!(
        t.path(4698).unwrap().unwrap(),
        "DATA\\GLOBAL\\MUSIC\\s4698.wav"
    );
    assert!(t.path(4700).is_none());
    let paths = t.paths();
    assert_eq!(
        paths.get(SoundId(1)).as_deref(),
        Some("DATA\\GLOBAL\\SFX\\cursor\\pass.wav")
    );
}

#[test]
fn path_limit() {
    let mut r = rows(3);
    r[1].file_name = vec![b'a'; 59];
    r[2].file_name = vec![b'a'; 70]; // only via a hand-built row
    let t = table(r);
    assert_eq!(t.path(1).unwrap().unwrap().len(), 16 + 59);
    assert!(matches!(
        t.path(2).unwrap(),
        Err(SoundTableError::PathTooLong { id: 2, .. })
    ));
}

#[test]
fn song_range_and_record_access() {
    let env = [4684, 0, 4657, -3, 4670].map(|song| SoundEnvironRow {
        song,
        ..SoundEnvironRow::default()
    });
    let t = SoundTableData::new(rows(3), &env);
    assert_eq!(t.song_range(), Some((4657, 4684)));
    assert!(t.is_song(4657) && t.is_song(4684) && !t.is_song(4685));
    assert!(t.get(3).is_none() && t.get(-1).is_none() && t.get(2).is_some());
    assert_eq!(table(rows(2)).song_range(), None);
}

#[test]
fn from_txt_builds_the_table() {
    let s = "Sound\tIndex\tFileName\tVolume\tGroup Size\tBlock 1\tBlock 2\tBlock 3\r\n\
             none\t0\tnone.wav\t0\t0\t-1\t-1\t-1\r\n\
             a\t1\ta.wav\t255\t2\t-1\t-1\t-1\r\n\
             b\t2\tb.wav\t200\t0\t5\t-1\t-1\r\n";
    let e = "Handle\tIndex\tSong\r\nx\t0\t2\r\n";
    let st = d2_data::txt::TxtTable::parse("sounds.txt", s.as_bytes()).unwrap();
    let et = d2_data::txt::TxtTable::parse("soundenviron.txt", e.as_bytes()).unwrap();
    let t = SoundTableData::from_txt(&st, &et).unwrap();
    assert_eq!(t.count(), 3);
    assert_eq!((t.base(2), t.get(2).unwrap().block_count), (1, 1));
    assert_eq!(t.song_range(), Some((2, 2)));
}

// --- requests ---------------------------------------------------------------

#[test]
fn request_rejects() {
    let mut r = rows(4);
    r[2].volume = 0;
    let mut sys = system(r);
    let mut w = World::new();
    assert_eq!(sys.request(&mut w, 0, None, 0, 0, 0), 0);
    assert_eq!(sys.request(&mut w, -5, None, 0, 0, 0), 0);
    assert_eq!(sys.request(&mut w, 2, None, 0, 0, 0), 0);
    assert_eq!(sys.requests().count(), 0);
    assert_eq!(sys.request(&mut w, 9, None, 0, 0, 0), 0);
    assert_eq!(sys.take_errors(), [SoundError::OutOfTable(9)]);
    sys.set_enabled(false);
    assert_eq!(sys.request(&mut w, 1, None, 0, 0, 0), 0);
    sys.set_enabled(true);
    assert_eq!(sys.request(&mut w, 1, None, 0, 0, 0), 1);
}

#[test]
fn request_fields_and_pool_limit() {
    let mut r = rows(3);
    r[1].priority = 40;
    let mut sys = system(r);
    let mut w = World::new();
    w.positions.insert(MONSTER, (1100, 1050));
    let h = sys.request(&mut w, 1, Some(MONSTER), 5, 2, 64);
    let q = sys.request_by_handle(h).unwrap();
    assert_eq!((q.id, q.volume, q.priority, q.start_tick), (1, 255, 40, 5));
    assert_eq!(
        (q.flags, q.start_offset, q.state),
        (2, 64, RequestState::Waiting)
    );
    // Unit requests have z = 640.0.
    assert_eq!(q.pos, [100.0, 100.0, 640.0]);
    assert_eq!(q.dist2, 20_000.0);
    // Without a unit, (0, 0, 320.0) and distance² 0.
    let n = sys.request(&mut w, 1, None, 0, 0, 0);
    let q = sys.request_by_handle(n).unwrap();
    assert_eq!((q.pos, q.dist2), ([0.0, 0.0, 320.0], 0.0));
    for _ in 2..REQUEST_SLOTS {
        assert_ne!(sys.request(&mut w, 1, None, 0, 0, 0), 0);
    }
    assert_eq!(sys.request(&mut w, 1, None, 0, 0, 0), 0);
    // Handles are pre-incremented.
    assert_eq!(h, 1);
}

#[test]
fn local_player_priority_wraps() {
    let mut r = rows(3);
    r[1].priority = 255;
    r[2].priority = 100;
    let mut sys = system(r);
    let mut w = World::new();
    let h = sys.request(&mut w, 1, Some(PLAYER), 0, 0, 0);
    assert_eq!(sys.request_by_handle(h).unwrap().priority, 79);
    let h = sys.request(&mut w, 2, Some(MONSTER), 0, 0, 0);
    assert_eq!(sys.request_by_handle(h).unwrap().priority, 100);
}

#[test]
fn compound_merges_into_the_group() {
    let mut r = rows(6);
    r[1].group_size = 3;
    r[1].compound = 4;
    r[2].compound = 4;
    r[1].priority = 10;
    r[4].compound = -1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, Some(PLAYER), 0, 0, 0);
    assert_eq!(sys.request_by_handle(h).unwrap().priority, 90);
    // Same group base (2's base is 1), within 4 ticks: same handle, +80 again.
    assert_eq!(sys.request(&mut w, 2, Some(PLAYER), 0, 0, 0), h);
    assert_eq!(sys.request_by_handle(h).unwrap().priority, 170);
    ticks(&mut sys, &mut w, &mut q, 4);
    assert_eq!(sys.request(&mut w, 1, None, 0, 0, 0), h);
    ticks(&mut sys, &mut w, &mut q, 1);
    // now − start = 5 > 4: a new request.
    let h2 = sys.request(&mut w, 1, None, 0, 0, 0);
    assert_ne!(h2, h);
    // Compound −1: any time.
    let h4 = sys.request(&mut w, 4, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 10);
    assert_eq!(sys.request(&mut w, 4, None, 0, 0, 0), h4);
    // A stopping request is not merged into.
    sys.stop_handle(h4);
    assert_ne!(sys.request(&mut w, 4, None, 0, 0, 0), h4);
}

#[test]
fn fade_call() {
    let mut r = rows(3);
    r[1].fade_in = 6;
    r[1].fade_out = 9;
    r[1].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    sys.set_volume(h, 100);
    sys.fade(h, 255, 2, 3);
    let f = sys.request_by_handle(h).unwrap().fade.unwrap();
    assert_eq!(
        f,
        Fade {
            start: 100,
            end: 255,
            t0: 2,
            t1: 8
        }
    );
    sys.fade(h, 0, 0, 0);
    let q = sys.request_by_handle(h).unwrap();
    assert!(q.stop);
    assert_eq!(
        q.fade.unwrap(),
        Fade {
            start: 100,
            end: 0,
            t0: 0,
            t1: 9
        }
    );
    // A fade-out cannot be turned around.
    sys.fade(h, 40, 0, 0);
    assert_eq!(sys.request_by_handle(h).unwrap().volume, 100);
    assert!(sys.take_errors().is_empty());
    // Without a running fade: len 0 sets the volume at once; with a delay
    // it is fatal (0x25C).
    let h2 = sys.request(&mut w, 2, None, 0, 0, 0);
    sys.fade(h2, 40, 0, 0);
    assert_eq!(sys.request_by_handle(h2).unwrap().volume, 40);
    sys.fade(h2, 40, 3, 0);
    assert_eq!(
        sys.take_errors(),
        [SoundError::FadeDelay {
            handle: h2,
            delay: 3
        }]
    );
    // Same target and the new end not before the running fade's end:
    // nothing (the running fade ends no later); an earlier end replaces it.
    let h3 = sys.request(&mut w, 2, None, 0, 0, 0);
    sys.fade(h3, 100, 0, 10);
    sys.fade(h3, 100, 2, 10);
    assert_eq!(sys.request_by_handle(h3).unwrap().fade.unwrap().t1, 10);
    sys.fade(h3, 100, 0, 5);
    assert_eq!(sys.request_by_handle(h3).unwrap().fade.unwrap().t1, 5);
}

// --- sound tick -------------------------------------------------------------

#[test]
fn fade_vectors() {
    let f = Fade {
        start: 0,
        end: 200,
        t0: 100,
        t1: 112,
    };
    assert_eq!(f.volume_at(99), (0, false));
    assert_eq!(f.volume_at(106), (100, false));
    assert_eq!(f.volume_at(107), (116, false));
    assert_eq!(f.volume_at(112), (200, false));
    assert_eq!(f.volume_at(113), (200, true));
}

#[test]
fn start_on_tick_with_centre_volume() {
    let mut sys = system(rows(3));
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    ticks(&mut sys, &mut w, &mut q, 2);
    sys.request(&mut w, 1, None, 3, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 3);
    assert!(starts(&mut q).is_empty());
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(starts(&mut q), [(5, 1, 255, 128)]);
    assert_eq!(sys.tick(), 6);
}

#[test]
fn list_order() {
    let mut r = rows(5);
    r[1].priority = 10;
    r[2].priority = 20;
    r[3].priority = 10;
    r[4].priority = 10;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    w.positions.insert(MONSTER, (1100, 1000));
    sys.request(&mut w, 4, Some(MONSTER), 0, 0, 0);
    sys.request(&mut w, 1, None, 0, 0, 0);
    sys.request(&mut w, 2, None, 0, 0, 0);
    sys.request(&mut w, 3, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let order: Vec<(i32, usize)> = sys.requests().map(|r| (r.id, r.slot)).collect();
    // Priority, then distance², then start tick, then higher slot first.
    assert_eq!(order, [(2, 2), (3, 3), (1, 1), (4, 0)]);
    // Starts follow the list order.
    let s: Vec<u32> = starts(&mut q).iter().map(|s| s.1).collect();
    assert_eq!(s, [2, 3, 1, 4]);
}

#[test]
fn stop_flag_stops_and_frees() {
    let mut r = rows(3);
    r[1].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    // The update that started it sends volume and pan again.
    assert!(matches!(&cues(&mut q)[..], [Cue::Start(_), Cue::Param(_)]));
    sys.stop_handle(h);
    ticks(&mut sys, &mut w, &mut q, 1);
    let c = cues(&mut q);
    assert!(matches!(&c[..], [Cue::Stop(s)] if matches!(s.target, StopTarget::Voice(_))));
    // Ended by the channel end, then freed by the end step in the same update.
    assert!(sys.request_by_handle(h).is_none());
    // A waiting request with the stop flag ends and is freed too.
    let w2 = sys.request(&mut w, 1, None, 5, 0, 0);
    sys.stop_handle(w2);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.request_by_handle(w2).is_none());
    assert!(cues(&mut q).is_empty());
}

#[test]
fn fade_to_zero_runs_before_the_stop() {
    let mut r = rows(3);
    r[1].looped = 1;
    r[1].fade_out = 4;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    sys.stop_handle(h); // playing, Fade Out 4: fade 1 → 5
    ticks(&mut sys, &mut w, &mut q, 5); // ticks 1..5: fading
    let c = cues(&mut q);
    assert!(c.iter().all(|c| !matches!(c, Cue::Stop(_))));
    let vols: Vec<i32> = c
        .iter()
        .filter_map(|c| match c {
            Cue::Param(p) => Some(p.vol),
            _ => None,
        })
        .collect();
    // 255 is the start update's second send.
    assert_eq!(vols, [255, 192, 128, 64, 0]);
    ticks(&mut sys, &mut w, &mut q, 2); // fade ends at 6, stop at 7
    assert!(cues(&mut q).iter().any(|c| matches!(c, Cue::Stop(_))));
}

#[test]
fn loop_out_of_range_waits_and_restarts() {
    let mut r = rows(3);
    r[1].looped = 1;
    r[1].falloff = 0; // max 400
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    w.positions.insert(MONSTER, (1300, 1000));
    w.positions.insert(MONSTER2, (1300, 1000));
    let h = sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(starts(&mut q).len(), 1);
    // Out of range by a direct position change (no tracking): emulate by
    // the master volume going to 0 (not audible).
    sys.set_settings(SoundSettings {
        master_volume: 0,
        ..SoundSettings::default()
    });
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(cues(&mut q).iter().any(|c| matches!(c, Cue::Stop(_))));
    ticks(&mut sys, &mut w, &mut q, 3);
    let r = sys.request_by_handle(h).unwrap();
    assert_eq!(r.state, RequestState::Waiting);
    sys.set_settings(SoundSettings::default());
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(starts(&mut q).len(), 1);
    // A one-shot out of range is dropped at once.
    let far = sys.request(&mut w, 2, Some(MONSTER), 0, 0, 0);
    let _ = sys.request_by_handle(far).unwrap();
    w.positions.insert(MONSTER, (2000, 1000));
    let far = sys.request(&mut w, 2, Some(MONSTER), 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.request_by_handle(far).is_none());
}

#[test]
fn one_shots_never_wait() {
    let mut r = rows(3);
    r[2].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    sys.set_settings(SoundSettings {
        master_volume: 0,
        ..SoundSettings::default()
    });
    let one = sys.request(&mut w, 1, None, 0, 0, 0);
    let looped = sys.request(&mut w, 2, None, 0, 0, 0);
    let later = sys.request(&mut w, 1, None, 5, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.request_by_handle(one).is_none());
    assert!(sys.request_by_handle(looped).is_some());
    assert!(sys.request_by_handle(later).is_some());
    // Volume 0 is not due: kept.
    sys.set_settings(SoundSettings::default());
    let quiet = sys.request(&mut w, 1, None, 0, 0, 0);
    sys.set_volume(quiet, 0);
    ticks(&mut sys, &mut w, &mut q, 2);
    assert!(sys.request_by_handle(quiet).is_some());
}

#[test]
fn duration_ends_a_playing_request() {
    let mut r = rows(3);
    r[1].looped = 1;
    r[1].duration = 3;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 4); // ticks 0..3: 3 − 0 = 3, not > 3
    assert_eq!(
        sys.request_by_handle(h).unwrap().state,
        RequestState::Playing
    );
    ticks(&mut sys, &mut w, &mut q, 1); // tick 4
    assert_eq!(sys.request_by_handle(h).unwrap().state, RequestState::Ended);
    ticks(&mut sys, &mut w, &mut q, 1); // Loop with Duration > 0: freed
    assert!(sys.request_by_handle(h).is_none());
}

#[test]
fn instance_rules() {
    let mut r = rows(6);
    r[1].group_size = 2;
    r[1].stop_inst = 0;
    r[1].defer_inst = 1;
    r[1].looped = 1;
    r[3].stop_inst = 1;
    r[3].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    // Defer Inst with a unit list: the second gets the stop flag.
    let a = sys.request(&mut w, 1, Some(MONSTER), 0, FLAG_EXACT, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let b = sys.request(&mut w, 1, Some(MONSTER), 0, FLAG_EXACT, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.request_by_handle(b).unwrap().stop);
    assert_eq!(
        sys.request_by_handle(a).unwrap().state,
        RequestState::Playing
    );
    // Without a unit list (position (0, 0, 320.0), never (0, 0,
    // 0)) a Defer Inst request starts alongside the found one.
    let x = sys.request(&mut w, 1, None, 0, FLAG_EXACT, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let xr = sys.request_by_handle(x).unwrap();
    assert_eq!((xr.state, xr.stop), (RequestState::Playing, false));
    assert!(!sys.request_by_handle(a).unwrap().stop);
    // Stop Inst: the old one gets the stop flag, the new one starts.
    let c = sys.request(&mut w, 3, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let d = sys.request(&mut w, 3, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.request_by_handle(c).unwrap().stop);
    assert_eq!(
        sys.request_by_handle(d).unwrap().state,
        RequestState::Playing
    );
    assert!(!sys.request_by_handle(d).unwrap().stop);
}

#[test]
fn speech_instance_rule_per_unit() {
    let mut r = rows(2940);
    r[2934].looped = 1;
    r[2935].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    w.positions.insert(MONSTER2, (1000, 1000));
    let a = sys.request(&mut w, 2934, Some(MONSTER), 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.speaking(MONSTER) && !sys.speaking(MONSTER2) && sys.any_speech());
    let b = sys.request(&mut w, 2935, Some(MONSTER), 0, 0, 0);
    let c = sys.request(&mut w, 2935, Some(MONSTER2), 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.request_by_handle(b).unwrap().stop);
    assert_eq!(
        sys.request_by_handle(c).unwrap().state,
        RequestState::Playing
    );
    assert_eq!(
        sys.request_by_handle(a).unwrap().state,
        RequestState::Playing
    );
}

#[test]
fn fade_in_and_no_fade_in_flag() {
    let mut r = rows(3);
    r[1].fade_in = 4;
    r[1].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(starts(&mut q), [(0, 1, 0, 128)]);
    assert_eq!(
        sys.request_by_handle(h).unwrap().fade,
        Some(Fade {
            start: 0,
            end: 255,
            t0: 0,
            t1: 4
        })
    );
    let h2 = sys.request(&mut w, 1, None, 0, 2, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.request_by_handle(h2).unwrap().fade.is_none());
}

#[test]
fn tracking_and_occlusion() {
    let mut r = rows(210);
    r[1].tracking = 1;
    r[1].looped = 1;
    r[2].falloff = 4;
    r[202].group_size = 3;
    let mut sys = system(r);
    assert!(sys.settings().game_loaded);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    w.blocked.insert(MONSTER);
    let h = sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    assert_eq!(sys.request_by_handle(h).unwrap().occlusion, 0.5);
    let h2 = sys.request(&mut w, 2, Some(MONSTER), 0, 0, 0);
    assert_eq!(sys.request_by_handle(h2).unwrap().occlusion, 0.0);
    w.indoors = true;
    let h3 = sys.request(&mut w, 203, Some(MONSTER), 0, 0, 0);
    assert_eq!(sys.request_by_handle(h3).unwrap().occlusion, 0.5);
    // Tracking: the unit moves; occlusion falls by 0.05 per tick.
    w.blocked.clear();
    w.positions.insert(MONSTER, (1010, 1000));
    ticks(&mut sys, &mut w, &mut q, 1);
    let r = sys.request_by_handle(h).unwrap();
    assert_eq!(r.pos, [10.0, 0.0, 640.0]);
    assert_eq!(r.occlusion, 0.45);
    // Not game-loaded: no tracking.
    sys.set_settings(SoundSettings {
        game_loaded: false,
        ..SoundSettings::default()
    });
    w.positions.insert(MONSTER, (1020, 1000));
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.request_by_handle(h).unwrap().pos, [10.0, 0.0, 640.0]);
}

#[test]
fn ducks() {
    let mut r = rows(70);
    r[1].solo = 1;
    r[1].looped = 1;
    r[2].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let solo = sys.request(&mut w, 1, None, 0, 0, 0);
    let other = sys.request(&mut w, 2, None, 0, 0, 0);
    let exempt = sys.request(&mut w, 60, None, 0, 0, 0);
    let _ = (other, exempt);
    ticks(&mut sys, &mut w, &mut q, 20);
    assert_eq!(sys.solo_duck(), 70);
    sys.fade(solo, 0, 0, 50);
    ticks(&mut sys, &mut w, &mut q, 3);
    assert_eq!(sys.solo_duck(), 76);
    w.duck = true;
    ticks(&mut sys, &mut w, &mut q, 25);
    assert_eq!(sys.state_duck(), 0);
}

// --- channels ---------------------------------------------------------------

#[test]
fn sixteen_channels_and_stealing() {
    let mut r = rows(30);
    for row in r.iter_mut().skip(1) {
        row.looped = 1;
        row.priority = 10;
    }
    r[21].priority = 20;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    // Mode 0: mono requests take slots 4–15 only.
    let low: Vec<_> = (1..=MONO_SLOTS as i32)
        .map(|id| sys.request(&mut w, id, None, 0, 0, 0))
        .collect();
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(starts(&mut q).len(), MONO_SLOTS);
    assert!((0..4).all(|c| sys.channel_request(c).is_none()));
    // Equal priority, higher slot: steals the least important (slot 0).
    let eq = sys.request(&mut w, 20, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let c = cues(&mut q);
    assert!(matches!(c[0], Cue::Stop(_)));
    assert!(matches!(&c[1], Cue::Start(t) if t.sound.0 == 20));
    assert_eq!(
        sys.request_by_handle(eq).unwrap().state,
        RequestState::Playing
    );
    let victim = sys.request_by_handle(low[0]).unwrap();
    // Stolen (ended), then (looping) waiting again by the end step, which is
    // all an ended request does in that update. A non-stream
    // voice saves no position (resume offset 0 = none).
    assert_eq!(victim.state, RequestState::Waiting);
    assert_eq!((victim.resume_offset, victim.start_offset), (0, 0));
    assert!(victim.fade.is_none());
    // Higher priority steals too.
    let hi = sys.request(&mut w, 21, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(
        sys.request_by_handle(hi).unwrap().state,
        RequestState::Playing
    );
    // `hi` took low[1]'s channel (the least important busy one now). When
    // it stops, the freed channel goes to the first waiting request in list
    // order: low[1] (higher slot) before low[0].
    sys.stop_handle(hi);
    ticks(&mut sys, &mut w, &mut q, 1);
    let v = sys.request_by_handle(low[1]).unwrap();
    assert_eq!(v.state, RequestState::Playing);
    let v = sys.request_by_handle(low[0]).unwrap();
    assert_eq!(v.state, RequestState::Waiting);
}

#[test]
fn mode0_has_four_stereo_channels() {
    let mut r = rows(10);
    for row in r.iter_mut().skip(1) {
        row.looped = 1;
        row.priority = 10;
    }
    let mut b = bank();
    b.stereo.extend(1..=5);
    let mut sys = SoundSystem::new(table(r), b);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    // Five stereo requests at once, equal priority: four take slots 0–3;
    // the fifth (lowest slot, so not more important) cannot use 4–15.
    let hs: Vec<_> = (1..=5)
        .map(|id| sys.request(&mut w, id, None, 0, 0, 0))
        .collect();
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(starts(&mut q).len(), 4);
    assert_eq!(
        sys.request_by_handle(hs[0]).unwrap().state,
        RequestState::Waiting
    );
    assert!((0..4).all(|c| sys.channel_request(c).is_some()));
    assert!((4..CHANNELS).all(|c| sys.channel_request(c).is_none()));
}

#[test]
fn stereo_comes_from_the_file_of_a_loaded_sample() {
    // `Stereo` 1 in the row but a mono file: the format check
    // overwrites it at the load, so the voice is mono (slot 4).
    let mut r = rows(3);
    r[1].stereo = 1;
    r[1].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.table().get(1).unwrap().row.stereo, 0);
    assert!(sys.channel_request(4).is_some());
    // A `Stream` row keeps its cell (no load).
    let mut r = rows(3);
    r[1].stereo = 1;
    r[1].stream = 1;
    r[1].looped = 1;
    let mut sys = system(r);
    sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.table().get(1).unwrap().row.stereo, 1);
    assert!(sys.channel_request(0).is_some());
}

#[test]
fn lower_priority_does_not_steal() {
    let mut r = rows(30);
    for row in r.iter_mut().skip(1) {
        row.looped = 1;
        row.priority = 50;
    }
    r[25].priority = 10;
    r[25].looped = 0;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    for id in 1..=MONO_SLOTS as i32 {
        sys.request(&mut w, id, None, 0, 0, 0);
    }
    ticks(&mut sys, &mut w, &mut q, 1);
    let lo = sys.request(&mut w, 25, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    // One-shot without a channel: dropped.
    assert!(sys.request_by_handle(lo).is_none());
}

#[test]
fn duplicate_suppression() {
    let mut r = rows(3);
    r[1].priority = 10;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let b = sys.request(&mut w, 1, None, 0, 0, 0); // start tick 1: 1 tick later
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.request_by_handle(b).is_none());
    ticks(&mut sys, &mut w, &mut q, 1);
    let c = sys.request(&mut w, 1, None, 0, 0, 0); // 3 ticks later
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(
        sys.request_by_handle(c).unwrap().state,
        RequestState::Playing
    );
    assert_eq!(starts(&mut q).len(), 2);
}

#[test]
fn missing_file_never_retried() {
    let mut r = rows(3);
    r[1].looped = 1;
    let mut b = bank();
    b.missing.insert(1);
    let mut sys = SoundSystem::new(table(r), b);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 3);
    assert!(starts(&mut q).is_empty());
    assert!(sys.table().get(1).unwrap().failed);
    assert_eq!(
        sys.request_by_handle(h).unwrap().state,
        RequestState::Waiting
    );
}

#[test]
fn async_only_waits_for_the_next_preload_pass() {
    let mut r = rows(3);
    r[1].async_only = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    // T 0: the start attempt starts the async read and fails; the one-shot
    // is kept while its sample loads.
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(starts(&mut q).is_empty());
    assert_eq!(sys.table().get(1).unwrap().load, LoadState::Pending);
    assert_eq!(sys.cache().pending, 1);
    assert!(sys.request_by_handle(h).is_some());
    // Collected only by the preload pass at T 25, which runs before the
    // update of T 25.
    ticks(&mut sys, &mut w, &mut q, 24);
    assert!(starts(&mut q).is_empty());
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(starts(&mut q), [(25, 1, 255, 128)]);
    assert_eq!(sys.cache().pending, 0);
}

#[test]
fn preload_and_locks() {
    let mut r = rows(8);
    r[1].group_size = 3;
    r[5].cache = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    sys.lock(2, 1).unwrap();
    assert_eq!(
        (1..5)
            .map(|i| sys.table().get(i).unwrap().locks)
            .collect::<Vec<_>>(),
        [1, 1, 1, 0]
    );
    ticks(&mut sys, &mut w, &mut q, 2);
    let loaded: Vec<i32> = (1..8)
        .filter(|&i| sys.table().get(i).unwrap().load == LoadState::Loaded)
        .collect();
    assert_eq!(loaded, [1, 2, 3, 5]);
    sys.lock(1, -1).unwrap();
    assert!(matches!(
        sys.lock(1, -1),
        Err(SoundError::Table(SoundTableError::Unlock { .. }))
    ));
}

// --- volume and pan ---------------------------------------------------------

#[test]
fn falloff_table() {
    assert_eq!(falloff(0), (60, 400));
    assert_eq!(falloff(1), (60, 700));
    assert_eq!(falloff(2), (200, 1000));
    assert_eq!(falloff(3), (400, 1500));
    assert_eq!(falloff(4), (2000, 2000));
    assert_eq!(falloff(9), (60, 700));
    assert_eq!(falloff(-1), (60, 700));
}

#[test]
fn volume_chain_vectors() {
    let s = SoundSettings {
        master_volume: 50,
        ..SoundSettings::default()
    };
    let c = ChainInputs {
        music_vol: false,
        state_duck_applies: true,
        solo: false,
        state_duck: 100,
        solo_duck: 100,
    };
    let v = chain(255, &s, &c);
    assert_eq!(v, 127);
    assert_eq!(record_volume(linear_falloff(v, 30.0, 0.0, 1), 210), 104);
    let f = linear_falloff(v, 380.0, 0.0, 1);
    assert_eq!(f, 63);
    assert_eq!(record_volume(f, 210), 51);
    // Music Vol and ducks.
    let c2 = ChainInputs {
        music_vol: true,
        state_duck: 50,
        solo_duck: 70,
        ..c
    };
    // 255 → music 50: 127 → master 50: 63 → state 50: 31 → solo 70: 21.
    assert_eq!(chain(255, &s, &c2), 21);
    let c3 = ChainInputs {
        state_duck_applies: false,
        solo: true,
        ..c2
    };
    assert_eq!(chain(255, &s, &c3), 63);
}

#[test]
fn positional_bias_modes_1_2() {
    assert_eq!(positional_bias(200, 50, true, true), 200);
    // b = 100: c = 33 → (255 − 168) × 200 / 255 = 68.
    assert_eq!(positional_bias(200, 100, true, false), 68);
    // b = 0: c = −33 → (−168 + 255) × 200 / 255 = 68.
    assert_eq!(positional_bias(200, 0, false, true), 68);
    assert_eq!(positional_bias(200, 0, true, false), 200);
}

#[test]
fn mode0_gain_pan_vectors() {
    assert_eq!(mode0_gain_pan([320.0, 0.0, 0.0]), (255, 229));
    assert_eq!(mode0_gain_pan([-320.0, 0.0, 0.0]), (255, 26));
    assert_eq!(mode0_gain_pan([1280.0, 0.0, 0.0]), (127, 255));
    assert_eq!(mode0_gain_pan([0.0, 0.0, 0.0]), (255, 128));
}

#[test]
fn positional_start_and_unchanged_sends_nothing() {
    let mut r = rows(3);
    r[1].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    w.positions.insert(MONSTER, (1160, 1000)); // x = 160: X = 0.5
    sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    // d = 160 > 60: (700 − 160) × 255 / 640 = 215; position (160, 0,
    // 640): X = 0.5, Z = 2 → gain 247, 215 × 247 / 255 = 208; pan 178.
    assert_eq!(starts(&mut q), [(0, 1, 208, 178)]);
    ticks(&mut sys, &mut w, &mut q, 3);
    assert!(cues(&mut q).is_empty());
    sys.set_settings(SoundSettings {
        master_volume: 50,
        ..SoundSettings::default()
    });
    ticks(&mut sys, &mut w, &mut q, 1);
    let c = cues(&mut q);
    // 127 → (700 − 160) × 127 / 640 = 107 → × 247 / 255 = 103.
    assert!(matches!(&c[..], [Cue::Param(p)] if p.vol == 103 && p.pan == 178));
}

#[test]
fn stereo_voices_get_no_pan_or_gain() {
    let mut b = bank();
    b.stereo.insert(1);
    let mut sys = SoundSystem::new(table(rows(3)), b);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    w.positions.insert(MONSTER, (1040, 1000));
    sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(starts(&mut q), [(0, 1, 255, 128)]);
}

#[test]
fn falloff_4_division_by_zero() {
    assert_eq!(ftol(f32::INFINITY), i32::MIN);
    assert_eq!(ftol(f32::NAN), i32::MIN);
    assert_eq!(ftol(-3.9), -3);
    // d = 2,100 > min = max = 2,000: (2000 − d) × v / 0.
    assert_eq!(linear_falloff(100, 2100.0, 0.0, 4), i32::MIN);
    assert_eq!(linear_falloff(100, 1999.0, 0.0, 4), 100);
}

#[test]
fn device_curves() {
    assert_eq!(device_volume(255), 0);
    assert_eq!(device_volume(0), -10_000);
    // −2000 × log10(2) = −602.06.
    assert_eq!(device_db(127.5, 255.0), -602);
    assert_eq!(device_pan(128), (0, 0));
    assert_eq!(device_pan(0), (0, -10_000));
    assert_eq!(device_pan(255), (-10_000, 0));
    let g = DeviceGain;
    let full = g.gains(255, 128).unwrap();
    assert_eq!((full.vol, full.pan_l, full.pan_r), (256, 256, 256));
    let left = g.gains(0, 0).unwrap();
    assert_eq!((left.vol, left.pan_l, left.pan_r), (0, 256, 0));
    let right = g.gains(255, 255).unwrap();
    assert_eq!((right.pan_l, right.pan_r), (0, 256));
    assert!(g.gains(256, 128).is_err());
}

// --- settings ---------------------------------------------------------------

#[test]
fn settings_store() {
    let d = SoundSettings::default();
    assert_eq!(
        (
            d.mixer_mode,
            d.master_volume,
            d.music_volume,
            d.positional_bias
        ),
        (0, 100, 50, 50)
    );
    assert_eq!((d.npc_speech, d.options_music), (2, 1));
    let store: BTreeMap<&str, i64> = [
        ("Sound Mixer", 3),
        ("Master Volume", 80),
        ("Music Volume", 101),
        ("Positional Bias", 0),
        ("NPC Speech", 1),
        ("Options Music", 0),
    ]
    .into();
    let s = SoundSettings::from_store(|k| store.get(k).copied());
    assert_eq!(
        (
            s.mixer_mode,
            s.master_volume,
            s.music_volume,
            s.positional_bias
        ),
        (0, 80, 50, 0)
    );
    assert_eq!((s.npc_speech, s.options_music), (1, 0));
}

#[test]
fn music_volume_zero_silences_songs_only() {
    let env = [SoundEnvironRow {
        song: 2,
        ..SoundEnvironRow::default()
    }];
    let mut r = rows(4);
    r[2].looped = 1;
    let mut sys = SoundSystem::new(SoundTableData::new(r, &env), bank());
    sys.set_settings(SoundSettings {
        music_volume: 0,
        ..SoundSettings::default()
    });
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    sys.request(&mut w, 2, None, 0, 0, 0);
    sys.request(&mut w, 3, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let s: Vec<u32> = starts(&mut q).iter().map(|s| s.1).collect();
    assert_eq!(s, [3]);
}

// --- SoundCalls, engine ------------------------------------------------------

#[test]
fn sound_calls_surface() {
    let mut r = rows(5000);
    r[4657].looped = 1;
    r[2950].looped = 1;
    r[60].looped = 1;
    r[61].looped = 1;
    let env = [SoundEnvironRow {
        song: 4657,
        ..SoundEnvironRow::default()
    }];
    let mut sys = SoundSystem::new(SoundTableData::new(r, &env), bank());
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let song;
    let speech;
    let a;
    let b;
    {
        let mut ctx = sys.with(&mut w);
        let calls: &mut dyn SoundCalls = &mut ctx;
        song = calls.request(4657, None, 0, 0, 0);
        speech = calls.request(2950, Some(MONSTER), 0, 0, 0);
        a = calls.request(60, None, 0, 0, 0);
        b = calls.request(61, None, 0, 0, 0);
        assert!(calls.is_active(song));
        assert_eq!(calls.roll(4), 3); // seed {1, 666}: 1,791,398,751 mod 4
        ctx.run_tick(&mut q);
        let calls: &mut dyn SoundCalls = &mut ctx;
        assert_eq!(calls.sound_tick(), 1);
        assert!(calls.speaking(MONSTER) && calls.any_speech());
        calls.stop_52_71_except(61, 0);
        calls.stop_songs();
        calls.stop_speech();
    }
    assert!(sys.request_by_handle(a).unwrap().stop);
    assert!(!sys.request_by_handle(b).unwrap().stop);
    assert!(sys.request_by_handle(song).unwrap().stop);
    assert!(sys.request_by_handle(speech).unwrap().stop);
    // detach: a looping request without other units stops.
    let mut ctx = sys.with(&mut w);
    let h = ctx.request(61, Some(MONSTER2), 0, 0, 0);
    ctx.detach(h, MONSTER2, false);
    assert!(ctx.sys.request_by_handle(h).unwrap().stop);
    ctx.stop_id(61);
    assert!(ctx.sys.request_by_handle(b).unwrap().stop);
}

#[test]
fn engine_voice_log() {
    let mut r = rows(3);
    r[1].looped = 1;
    let mut sys = system(r);
    let paths = sys.table().paths();
    struct PathBank(SoundPaths);
    impl SoundBank for PathBank {
        fn file(&self, id: SoundId) -> Option<Arc<str>> {
            self.0.get(id)
        }
        fn samples(&self, _: SoundId) -> Option<Arc<Sound>> {
            Some(Arc::new(Sound::new(1_000, 1, vec![0; 1_000]).unwrap()))
        }
    }
    let mut engine = AudioEngine::new(
        Box::new(PathBank(paths)),
        Box::new(DeviceGain),
        Box::new(Unlimited),
    );
    let mut w = World::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    sys.run_tick(&mut w, engine.queue_mut());
    sys.stop_handle(h);
    sys.run_tick(&mut w, engine.queue_mut());
    engine.present(1).unwrap();
    engine.mix_block();
    let log: Vec<(u32, VoiceKind, &str, i32, i32)> = engine
        .log()
        .events
        .iter()
        .map(|e| (e.tick, e.kind, e.file.as_str(), e.vol, e.pan))
        .collect();
    assert_eq!(
        log,
        [
            (0, VoiceKind::Start, "DATA\\GLOBAL\\SFX\\s1.wav", 255, 128),
            // Sent a second time by the start's update.
            (0, VoiceKind::Param, "DATA\\GLOBAL\\SFX\\s1.wav", 255, 128),
            (1, VoiceKind::Stop, "DATA\\GLOBAL\\SFX\\s1.wav", 255, 128),
        ]
    );
    assert!(engine.take_errors().is_empty());
}

#[test]
fn env_and_trigger_surfaces() {
    use crate::audio::environment::EnvCalls;
    use crate::audio::triggers::TriggerSound;
    let mut r = rows(4700);
    r[4657].looped = 1;
    r[4657].stream = 1;
    r[4657].blocks = [10, 20, -1];
    r[10].group_size = 4;
    r[10].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let mut ctx = sys.with(&mut w);
    assert!(!ctx.music_active() && !ctx.id_requested(4657));
    let m = ctx.request(4657, None, 0, 0, 0);
    let u = ctx.request(10, Some(MONSTER), 0, FLAG_EXACT, 0);
    assert!(ctx.music_active() && ctx.id_requested(4657));
    assert_eq!(ctx.blocks(4657), [10, 20, -1]);
    assert_eq!(ctx.blocks(9_999), [-1, -1, -1]);
    assert_eq!(ctx.volume(m), Some(255));
    assert_eq!(ctx.volume(999), None);
    assert_eq!(ctx.play_position(4657), None);
    ctx.set_position(m, 3, 4, 5);
    assert!(ctx.sound_on());
    assert_eq!((ctx.group_base(12), ctx.group_base(10)), (10, 10));
    assert!(ctx.looping(10) && !ctx.looping(11));
    assert_eq!(ctx.unit_requests(MONSTER), [(u, 10)]);
    assert_eq!((ctx.unit_count(u), ctx.unit_count(m)), (1, 0));
    // Variant: seed {1, 666}, size 4 → 10 + 3.
    assert_eq!(ctx.variant(10), 13);
    ctx.run_tick(&mut q);
    ctx.run_tick(&mut q);
    // Two sound ticks at 1,000 Hz mono: 80 frames = 160 bytes = 40 units
    // of 4 bytes.
    assert_eq!(ctx.play_position(4657), Some(40));
    // z + 640, distance² from x, y.
    let r = ctx.sys.request_by_handle(m).unwrap();
    assert_eq!((r.pos, r.dist2), ([3.0, 4.0, 645.0], 25.0));
}

// --- game files ---------------------------------------------------------------

mod game {
    use super::*;
    use d2_data::bin::read_excel;
    use d2_data::txt::TxtTable;
    use d2_formats::mpq::ArchiveSet;

    fn load() -> (ArchiveSet, SoundTableData) {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let txt = |f: &str| {
            let (_, b) = read_excel(&set, f).unwrap().unwrap();
            TxtTable::parse(f, &b).unwrap()
        };
        let t = SoundTableData::from_txt(&txt("sounds.txt"), &txt("soundenviron.txt")).unwrap();
        (set, t)
    }

    fn found(set: &ArchiveSet, path: &str) -> Option<String> {
        set.read_with_source(path).unwrap().map(|(s, _)| s)
    }

    /// `D2_GAME_DIR=... cargo test -p d2-client --lib sound_table::tests::game -- --ignored`
    #[test]
    #[ignore = "needs D2_GAME_DIR"]
    fn live_sound_table() {
        let (set, t) = load();
        assert_eq!(t.count(), 4_699);
        assert_eq!(t.song_range(), Some((4_657, 4_684)));
        let e = t.get(1).unwrap();
        assert_eq!((e.row.volume, e.row.priority), (255, 100));
        let p = t.path(1).unwrap().unwrap();
        assert_eq!(p, "DATA\\GLOBAL\\SFX\\cursor\\pass.wav");
        assert_eq!(found(&set, &p).as_deref(), Some("d2sfx.mpq"));
        assert_eq!(t.get(202).unwrap().row.group_size, 3);
        assert_eq!((t.base(202), t.base(203), t.base(204)), (202, 202, 202));
        let e = t.get(309).unwrap();
        assert_eq!((e.row.group_size, e.row.compound), (5, 4));
        assert_eq!(t.get(314).unwrap().row.group_size, 3);
        let p = t.path(2934).unwrap().unwrap();
        assert_eq!(p, "DATA\\LOCAL\\SFX\\common\\amazon\\ama_cantcarry.wav");
        assert_eq!(found(&set, &p).as_deref(), Some("d2speech.mpq"));
        let p = t.path(4657).unwrap().unwrap();
        assert_eq!(p, "DATA\\GLOBAL\\MUSIC\\act1\\caves.wav");
        assert_eq!(found(&set, &p).as_deref(), Some("d2music.mpq"));
        let e = t.get(4657).unwrap();
        assert_eq!((e.row.looped, e.row.stream, e.block_count), (1, 1, 1));
        assert_eq!(t.get(4679).unwrap().block_count, 2);
        let p = t.path(4698).unwrap().unwrap();
        assert_eq!(found(&set, &p).as_deref(), Some("d2xmusic.mpq"));
        assert_eq!(t.get(1595).unwrap().row.volume, 0);
        assert_eq!(found(&set, &t.path(1595).unwrap().unwrap()), None);
        assert_eq!(found(&set, &t.path(4640).unwrap().unwrap()), None);
        // Whole table.
        let (mut resolve, mut none, mut missing) = (0, 0, 0);
        for id in 0..t.count() as i32 {
            let p = t.path(id).unwrap().unwrap();
            if found(&set, &p).is_some() {
                resolve += 1;
            } else if t
                .get(id)
                .unwrap()
                .row
                .file_name
                .eq_ignore_ascii_case(b"none.wav")
            {
                none += 1;
            } else {
                missing += 1;
            }
        }
        assert_eq!((resolve, none, missing), (4_508, 157, 34));
        let openers = (1..t.count() as i32)
            .filter(|&i| t.get(i).unwrap().row.group_size != 0)
            .count();
        let nested = (1..t.count() as i32)
            .filter(|&i| t.get(i).unwrap().row.group_size != 0 && t.base(i) != i)
            .count();
        assert_eq!((openers, nested), (698, 7));
    }
}

#[test]
fn runtime_fields_sample_load_state_and_last_use() {
    // +0x86 load state 0 → 1 (pending) → 2 (loaded) with the sample handle
    // (+0x6C) held once loaded; +0x7C follows the sound tick of every
    // update in which the playing request used the sample.
    let mut r = rows(3);
    r[1].looped = 1;
    r[1].async_only = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let e = sys.table().get(1).unwrap();
    assert_eq!(
        (e.load, e.sample.is_some(), e.last_use),
        (LoadState::None, false, 0)
    );
    sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let e = sys.table().get(1).unwrap();
    assert_eq!((e.load, e.sample.is_some()), (LoadState::Pending, false));
    // The load start stamps the use.
    assert_eq!((e.last_use, e.size), (0, 2_044));
    ticks(&mut sys, &mut w, &mut q, 25);
    let e = sys.table().get(1).unwrap();
    assert_eq!((e.load, e.sample.is_some()), (LoadState::Loaded, true));
    assert_eq!(starts(&mut q).len(), 1);
    for _ in 0..4 {
        ticks(&mut sys, &mut w, &mut q, 1);
        assert_eq!(sys.table().get(1).unwrap().last_use, sys.tick() - 1);
    }
    // Row 2 was never requested: untouched.
    let e = sys.table().get(2).unwrap();
    assert_eq!((e.load, e.last_use), (LoadState::None, 0));
}

#[test]
fn mixer_modes_1_and_2_play_as_mode_0() {
    // D2RUST reproduces mixer mode 0 only: a positioned start sends the same
    // volume and pan whatever `Sound Mixer` holds.
    let run = |mode: u8| {
        let mut sys = system(rows(3));
        sys.set_settings(SoundSettings {
            mixer_mode: mode,
            ..SoundSettings::default()
        });
        let mut w = World::new();
        let mut q = TriggerQueue::new();
        w.positions.insert(MONSTER, (1100, 1000));
        sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
        ticks(&mut sys, &mut w, &mut q, 2);
        starts(&mut q)
    };
    let mode0 = run(0);
    assert_eq!(mode0.len(), 1);
    assert_eq!(run(1), mode0);
    assert_eq!(run(2), mode0);
}

// --- trigger conventions on the real system ---------------------------------

/// `n` rows, every row playable and `Fade Out` 10.
fn faded_rows(n: usize) -> Vec<SoundRow> {
    let mut r = rows(n);
    for x in r.iter_mut().skip(1) {
        x.fade_out = 10;
    }
    r
}

#[test]
fn volume_set_is_applied_by_the_next_update() {
    let mut sys = system(rows(3));
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 2);
    assert_eq!(starts(&mut q).len(), 1);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(cues(&mut q).is_empty(), "nothing changed: nothing sent");
    sys.set_volume(h, 100);
    assert_eq!(sys.request_by_handle(h).unwrap().volume, 100);
    assert!(cues(&mut q).is_empty(), "not sent by the call itself");
    ticks(&mut sys, &mut w, &mut q, 1);
    let sent: Vec<i32> = cues(&mut q)
        .into_iter()
        .filter_map(|c| match c {
            Cue::Param(p) => Some(p.vol),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert!(sent[0] < 255);
    // No request with that handle: nothing.
    sys.set_volume(999, 7);
    assert!(sys.request_by_handle(999).is_none());
}

#[test]
fn detach_stops_only_the_last_unit_of_a_loop_or_by_force() {
    let mut r = rows(4);
    r[2].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    w.positions.insert(MONSTER2, (1000, 1000));
    // A one-shot on MONSTER: detaching another unit does nothing; the last
    // unit leaves and the one-shot keeps playing.
    let h1 = sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    sys.detach(h1, MONSTER2, false);
    assert_eq!(sys.request_by_handle(h1).unwrap().units, [MONSTER]);
    sys.detach(h1, MONSTER, false);
    let q = sys.request_by_handle(h1).unwrap();
    assert!(q.units.is_empty() && !q.stop);
    // A loop: stopped when its last unit leaves.
    let h2 = sys.request(&mut w, 2, Some(MONSTER), 0, 0, 0);
    sys.detach(h2, MONSTER, false);
    assert!(sys.request_by_handle(h2).unwrap().stop);
    // Force stops a one-shot too.
    let h3 = sys.request(&mut w, 3, Some(MONSTER), 0, 0, 0);
    sys.detach(h3, MONSTER, true);
    assert!(sys.request_by_handle(h3).unwrap().stop);
}

#[test]
fn group_stops_select_by_handle_id_range_and_speech() {
    let mut sys = system(faded_rows(3000));
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    // The second id-5 request waits (delay 100) so both exist: a second
    // start of the same id in the same tick is suppressed and the
    // one-shot removed.
    let ids = [5, 5, 52, 60, 71, 72, 150, 2934, 2999];
    let hs: Vec<crate::audio::calls::Handle> = ids
        .iter()
        .enumerate()
        .map(|(i, &id)| sys.request(&mut w, id, None, if i == 1 { 100 } else { 0 }, 0, 0))
        .collect();
    ticks(&mut sys, &mut w, &mut q, 2);
    let stopped = |sys: &SoundSystem| -> Vec<bool> {
        hs.iter()
            .map(|&h| sys.request_by_handle(h).unwrap().stop)
            .collect()
    };
    // Every request of id 5; playing with Fade Out: a fade to 0 over 10.
    sys.stop_id(5);
    assert_eq!(stopped(&sys)[..2], [true, true]);
    let f = sys.request_by_handle(hs[0]).unwrap().fade.unwrap();
    assert_eq!((f.end, f.t1 - f.t0), (0, 10));
    // 52–71 except the group bases 60 and 61.
    sys.stop_range_except(52, 71, 60, 61);
    assert_eq!(stopped(&sys)[2..5], [true, false, true]);
    // 72–201 except 150.
    sys.stop_range_except(72, 201, 150, 0);
    assert_eq!(stopped(&sys)[5..7], [true, false]);
    // Speech: 2,934–4,656.
    sys.stop_speech();
    assert_eq!(stopped(&sys)[7..], [true, true]);
    // One handle.
    sys.stop_handle(hs[3]);
    assert!(stopped(&sys)[3]);
}

#[test]
fn speaking_needs_a_playing_speech_request_on_the_unit() {
    let mut sys = system(rows(3000));
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    assert!(!sys.any_speech() && !sys.speaking(MONSTER));
    sys.request(&mut w, 2950, Some(MONSTER), 0, 0, 0);
    // Waiting: any_speech (not ended), not speaking (not playing).
    assert!(sys.any_speech());
    assert!(!sys.speaking(MONSTER));
    ticks(&mut sys, &mut w, &mut q, 2);
    assert!(sys.speaking(MONSTER));
    assert!(!sys.speaking(PLAYER), "other unit");
    // A non-speech id on PLAYER does not count.
    sys.request(&mut w, 100, Some(PLAYER), 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 2);
    assert!(!sys.speaking(PLAYER));
}

#[test]
fn draw_helpers_on_the_client_seed() {
    use crate::audio::calls::{jitter, uniform};
    let mut sys = system(rows(3));
    let mut w = World::new();
    w.seed = Some(Seed::new(12345, 666));
    let mut want = Seed::new(12345, 666);
    let mut ctx = SoundCtx {
        sys: &mut sys,
        world: &mut w,
    };
    let r = ctx.roll(7);
    assert_eq!(r, want.roll(7));
    assert_eq!(uniform(&mut ctx, 450, 750), 450 + want.roll(301) as i32);
    assert_eq!(jitter(&mut ctx, 100), want.roll(201) as i32 - 100);
    assert_eq!(w.seed, Some(want));
}

// --- 2026-10-07 corrections: requests, variants, streams, cache ------------

#[test]
fn compound_merge_attaches_its_unit() {
    let mut r = rows(4);
    r[1].compound = -1;
    r[1].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    w.positions.insert(MONSTER2, (1000, 1000));
    let h = sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    assert_eq!(sys.request(&mut w, 1, Some(MONSTER2), 0, 0, 0), h);
    // One request, unit list newest first; the handle in both units' lists.
    assert_eq!(sys.request_by_handle(h).unwrap().units, [MONSTER2, MONSTER]);
    assert_eq!(sys.unit_requests(MONSTER), [(h, 1)]);
    assert_eq!(sys.unit_requests(MONSTER2), [(h, 1)]);
    // The same unit again: held twice, no duplicate check.
    sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    assert_eq!(sys.unit_requests(MONSTER), [(h, 1), (h, 1)]);
    // With several units the fade call does nothing.
    sys.fade(h, 0, 0, 5);
    assert!(!sys.request_by_handle(h).unwrap().stop);
}

#[test]
fn compound_window_is_unsigned() {
    let mut r = rows(3);
    r[1].compound = 4;
    let mut sys = system(r);
    let mut w = World::new();
    // Start tick in the future (delay 5): now − start wraps, no merge.
    let h = sys.request(&mut w, 1, None, 5, 0, 0);
    assert_ne!(sys.request(&mut w, 1, None, 0, 0, 0), h);
}

#[test]
fn unit_request_list_order_and_lifetime() {
    let mut r = rows(4);
    r[3].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let a = sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    let b = sys.request(&mut w, 3, Some(MONSTER), 0, 0, 0);
    assert_eq!(sys.unit_requests(MONSTER), [(b, 3), (a, 1)]);
    // Ended but not yet freed requests stay listed.
    sys.stop_handle(b);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.unit_requests(MONSTER), [(a, 1)]);
    // A detach drops the unit's node and one unit; a one-shot keeps
    // playing.
    let c = sys.request(&mut w, 2, Some(MONSTER2), 0, 0, 0);
    sys.detach(c, MONSTER2, false);
    assert!(sys.unit_requests(MONSTER2).is_empty());
    let cr = sys.request_by_handle(c).unwrap();
    assert!(cr.units.is_empty() && !cr.stop);
}

#[test]
fn the_variant_overwrites_the_request_id() {
    let mut r = rows(110);
    r[100].group_size = 4;
    // Every row of the group loops (the variant's record is read).
    for x in &mut r[100..104] {
        x.looped = 1;
    }
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 100, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    // Seed {1, 666}: roll(4) = 3 → 103; history on the requested id's record.
    assert_eq!(sys.request_by_handle(h).unwrap().id, 103);
    assert_eq!(sys.table().get(100).unwrap().history, [103, 0]);
    // A loop restart picks relative to 103 (size 1): no draw, stays 103.
    sys.set_settings(SoundSettings {
        master_volume: 0,
        ..SoundSettings::default()
    });
    ticks(&mut sys, &mut w, &mut q, 2);
    sys.set_settings(SoundSettings::default());
    let seed = w.seed;
    ticks(&mut sys, &mut w, &mut q, 1);
    let r = sys.request_by_handle(h).unwrap();
    assert_eq!((r.id, r.state), (103, RequestState::Playing));
    assert_eq!(w.seed, seed);
}

#[test]
fn a_failed_start_after_fade_in_leaves_volume_0() {
    let mut r = rows(3);
    r[1].looped = 1;
    r[1].fade_in = 4;
    let mut b = bank();
    b.missing.insert(1);
    let mut sys = SoundSystem::new(table(r), b);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 3);
    let r = sys.request_by_handle(h).unwrap();
    assert_eq!(
        (r.state, r.volume, r.fade),
        (RequestState::Waiting, 0, None)
    );
    assert!(starts(&mut q).is_empty());
}

#[test]
fn stream_offsets_resume_and_loop_start() {
    use crate::audio::DeviceChange;
    let mut r = rows(4);
    r[1].stream = 1;
    r[1].looped = 1;
    r[2].looped = 1;
    r[2].blocks = [100, -1, -1];
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    // Start offset 0xFFFFFFFF: × 4 wraps to 0xFFFFFFFC, mod the 2,000 data
    // bytes = 1,292, frame 646.
    let h = sys.request(&mut w, 1, None, 0, 0, 0xFFFF_FFFF);
    // Block count 1: loop start `Block 1` × 2 bytes = frame 100 (mono).
    sys.request(&mut w, 2, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let dev: Vec<DeviceChange> = cues(&mut q)
        .into_iter()
        .filter_map(|c| match c {
            Cue::Device(d) => Some(d),
            _ => None,
        })
        .collect();
    let mut got: Vec<(Option<u64>, Option<u64>)> =
        dev.iter().map(|d| (d.start_frame, d.loop_start)).collect();
    got.sort();
    assert_eq!(got, [(Some(0), Some(100)), (Some(646), None)]);
    let c = (0..CHANNELS)
        .find(|&c| sys.channel_request(c).is_some_and(|r| r.id == 2))
        .unwrap();
    assert_eq!(sys.channel_loop_start(c), Some(200));
    // Stopped at T 1 (inaudible): the stream saves its position in 4-byte
    // units: (646 + 40) frames × 2 bytes / 4 = 343.
    sys.set_settings(SoundSettings {
        master_volume: 0,
        ..SoundSettings::default()
    });
    ticks(&mut sys, &mut w, &mut q, 2);
    assert_eq!(sys.request_by_handle(h).unwrap().resume_offset, 343);
    sys.set_settings(SoundSettings::default());
    ticks(&mut sys, &mut w, &mut q, 1);
    // Resumed from there with a 3-tick fade-in.
    let r = sys.request_by_handle(h).unwrap();
    assert_eq!(
        (r.state, r.start_offset, r.resume_offset),
        (RequestState::Playing, 343, 0)
    );
    assert_eq!(r.fade.map(|f| f.t1 - f.t0), Some(3));
}

#[test]
fn a_stream_that_fails_to_open_retries() {
    let mut r = rows(3);
    r[1].stream = 1;
    r[1].looped = 1;
    let mut b = bank();
    b.missing.insert(1);
    let mut sys = SoundSystem::new(table(r), b);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 2);
    // No file-failed flag: the loop waits and is tried again.
    assert!(!sys.table().get(1).unwrap().failed);
    assert_eq!(
        sys.request_by_handle(h).unwrap().state,
        RequestState::Waiting
    );
    assert!((0..CHANNELS).all(|c| sys.channel_request(c).is_none()));
}

#[test]
fn river_projection() {
    use super::system::river_point;
    // P (0, 0), U (10, 5): the foot of P on the slope −1/2 line through U.
    assert_eq!(river_point((0, 0), (10, 5)), (4, 8));
    assert_eq!(river_point((100, 50), (100, 50)), (100, 50));
    let mut r = rows(2600);
    r[2599].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    w.positions.insert(PLAYER, (0, 0));
    w.positions.insert(MONSTER, (10, 5));
    let h = sys.request(&mut w, 2599, Some(MONSTER), 0, 0, 0);
    // x = 4 − 0, y = 2 × (8 − 0).
    assert_eq!(sys.request_by_handle(h).unwrap().pos, [4.0, 16.0, 640.0]);
}

#[test]
fn river_reads_the_client_only_unit_first() {
    // A set-C river object shares its GUID with a server object;
    // 2599 asks set C first, any other sound the unit table.
    let mut r = rows(2600);
    r[2599].looped = 1;
    r[1].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    w.positions.insert(PLAYER, (0, 0));
    w.positions.insert(MONSTER, (500, 500));
    w.client_only.insert(MONSTER, (10, 5));
    let h = sys.request(&mut w, 2599, Some(MONSTER), 0, 0, 0);
    assert_eq!(sys.request_by_handle(h).unwrap().pos, [4.0, 16.0, 640.0]);
    let h = sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    assert_eq!(
        sys.request_by_handle(h).unwrap().pos,
        [500.0, 1000.0, 640.0]
    );
}

#[test]
fn tracking_takes_the_strictly_nearest_unit() {
    let mut r = rows(3);
    r[1].tracking = 1;
    r[1].looped = 1;
    r[1].compound = -1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    w.positions.insert(MONSTER, (1100, 1000));
    w.positions.insert(MONSTER2, (900, 1000));
    let h = sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    sys.request(&mut w, 1, Some(MONSTER2), 0, 0, 0);
    // A tie: the earlier unit in the list (the newest, MONSTER2) wins.
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.request_by_handle(h).unwrap().pos, [-100.0, 0.0, 640.0]);
    w.positions.insert(MONSTER, (1050, 1000));
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.request_by_handle(h).unwrap().pos, [50.0, 0.0, 640.0]);
}

#[test]
fn eviction_vectors() {
    let run = |async_only: bool| {
        let mut r = rows(9);
        r[7].cache = 1;
        r[3].async_only = u8::from(async_only);
        let mut b = bank();
        b.sizes.extend([(3, 30), (5, 20), (7, 25)]);
        let mut sys = SoundSystem::new(table(r), b);
        let mut w = World::new();
        let mut q = TriggerQueue::new();
        // T 0 preloads 7 (Cache) synchronously; then the stage: limit 100,
        // total 90, LRU [5 (20, unlocked), 7 (25, Cache)].
        ticks(&mut sys, &mut w, &mut q, 1);
        let e = sys.table_mut().get_mut(5).unwrap();
        e.load = LoadState::Loaded;
        e.size = 20;
        e.sample = Some(Arc::new(Sound::new(1_000, 1, vec![0; 10]).unwrap()));
        let c = sys.cache_mut();
        c.limit = 100;
        c.total = 90;
        c.lru = vec![5, 7];
        sys.request(&mut w, 3, None, 0, 0, 0);
        ticks(&mut sys, &mut w, &mut q, 1);
        let loaded: Vec<bool> = [5, 7]
            .iter()
            .map(|&i| sys.table().get(i).unwrap().load == LoadState::Loaded)
            .collect();
        (sys.cache().total, loaded, sys.cache().lru.clone())
    };
    // Sync: need 20, target 70; walk 0 unloads 5 (70), walk 1 unloads 7
    // (45); the load: 75.
    assert_eq!(run(false), (75, vec![false, false], vec![3]));
    // Async: walk 0 only: 5 (70); the read starts: 100.
    assert_eq!(run(true), (100, vec![false, true], vec![7, 3]));
}

#[test]
fn failed_eviction_holds_the_preload() {
    let mut r = rows(9);
    r[3].async_only = 1;
    r[7].cache = 1;
    let mut b = bank();
    b.sizes.insert(3, 30);
    let mut sys = SoundSystem::new(table(r), b);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    ticks(&mut sys, &mut w, &mut q, 400);
    // Nothing evictable (7 is `Cache`, async: walk 0 only).
    let c = sys.cache_mut();
    c.limit = c.total + 10;
    sys.request(&mut w, 3, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.cache().failed_eviction, 400);
    assert_eq!(sys.table().get(3).unwrap().load, LoadState::None);
    // A locked row is not preloaded before T 650.
    sys.cache_mut().limit = u32::MAX;
    sys.lock(5, 1).unwrap();
    while sys.tick() < 650 {
        ticks(&mut sys, &mut w, &mut q, 1);
        assert_eq!(sys.table().get(5).unwrap().load, LoadState::None);
    }
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.table().get(5).unwrap().load, LoadState::Pending);
}

#[test]
fn an_abandoned_read_keeps_its_pending_count() {
    let mut r = rows(9);
    r[3].async_only = 1;
    let mut b = bank();
    b.sizes.extend([(3, 30), (4, 60)]);
    let mut sys = SoundSystem::new(table(r), b);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    ticks(&mut sys, &mut w, &mut q, 1);
    sys.cache_mut().limit = 80;
    sys.request(&mut w, 3, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.cache().pending, 1);
    // A sync load of 60 evicts the pending 3 (walk 0).
    sys.request(&mut w, 4, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.table().get(3).unwrap().load, LoadState::None);
    assert_eq!(sys.cache().pending, 1);
}

#[test]
fn use_order_moves_to_the_tail() {
    let mut r = rows(10);
    for i in [5, 7, 9] {
        r[i].looped = 1;
    }
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let mut hs = Vec::new();
    for id in [5, 9, 7] {
        hs.push(sys.request(&mut w, id, None, 0, 0, 0));
        ticks(&mut sys, &mut w, &mut q, 1);
        sys.stop_handle(*hs.last().unwrap());
        ticks(&mut sys, &mut w, &mut q, 1);
    }
    assert_eq!(sys.cache().lru, [5, 9, 7]);
    let h = sys.request(&mut w, 9, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.cache().lru, [5, 7, 9]);
    let _ = h;
}

#[test]
fn device_gain_applies_occlusion() {
    let g = DeviceGain;
    // v 200, G 255, occlusion 0.5: amplitude 100 / 255.
    assert_eq!(device_occluded(200, 0.5), 100);
    let a = g.gains_occluded(200, 128, 0.5).unwrap();
    let b = g.gains(100, 128).unwrap();
    assert_eq!(a, b);
    // A negative send is silent, not an error.
    assert_eq!(g.gains(-8_421_504, 128).unwrap().vol, 0);
}

#[test]
fn play_position_reads_the_current_id_of_the_first_request() {
    let mut r = rows(110);
    r[100].group_size = 4;
    for x in &mut r[100..104] {
        x.looped = 1;
        x.stream = 1;
    }
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 100, None, 0, 0, 0);
    // Not started yet: no channel, nothing to read.
    assert_eq!(sys.play_position(100), None);
    ticks(&mut sys, &mut w, &mut q, 3);
    // Seed {1, 666}: the variant is 103; the lookup is by the current id.
    assert_eq!(sys.request_by_handle(h).unwrap().id, 103);
    assert_eq!(sys.play_position(100), None);
    assert!(sys.play_position(103).is_some());
}

#[test]
fn duplicate_test_uses_the_smallest_start_tick() {
    let mut r = rows(3);
    r[1].priority = 10;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let a = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 5);
    let b = sys.request(&mut w, 1, None, 0, 0, 0); // start tick 5
    ticks(&mut sys, &mut w, &mut q, 1);
    ticks(&mut sys, &mut w, &mut q, 0);
    let c = sys.request(&mut w, 1, None, 0, 0, 0); // start tick 6
    ticks(&mut sys, &mut w, &mut q, 1);
    // c is compared with the playing request of the smallest start tick
    // (a, 6 ticks away), not with b (1 tick away).
    for h in [a, b, c] {
        assert_eq!(
            sys.request_by_handle(h).unwrap().state,
            RequestState::Playing
        );
    }
    assert_eq!(starts(&mut q).len(), 3);
}

#[test]
fn slider_mapping_and_enabled_tests() {
    use super::sliders::*;
    // p = ⌊(v + 1) / 5⌋ for every stored value, exactly.
    for v in 0..=100 {
        assert_eq!(position_of(v), ((v + 1) / 5) as u32, "v {v}");
    }
    // v = 5 × p; a stored 37 shows at p = 7 and stays until it moves.
    assert_eq!(position_of(37), 7);
    assert_eq!((0..SLIDER_N).map(value_of).next_back(), Some(100));
    assert_eq!(value_of(7), 35);
    for p in 0..SLIDER_N {
        assert_eq!(value_of(p), 5 * p as i32);
    }
    // n = 21; enabled tests.
    assert_eq!(SLIDER_N, 21);
    assert!(AudioSlider::Sound.enabled(true, 0));
    assert!(AudioSlider::Music.enabled(true, 0));
    assert!(!AudioSlider::Sound.enabled(false, 1));
    assert!(!AudioSlider::Bias.enabled(true, 0));
    assert!(AudioSlider::Bias.enabled(true, 1) && AudioSlider::Bias.enabled(true, 2));
    assert!(!AudioSlider::Bias.enabled(false, 2));
}

#[test]
fn slider_inputs() {
    use super::sliders::*;
    assert_eq!((left(0), left(5), right(19), right(20)), (0, 4, 20, 20));
    // W = 800: h = 400, x0 = 267; stops 13.25 pixels apart.
    let (w, x0) = (800, 267);
    assert_eq!(drag_position(x0 - 1, w, false), 0);
    assert_eq!(drag_position(x0 + 266, w, false), 20);
    assert_eq!(drag_position(x0 + 265, w, false), 20);
    for k in 0..=20u32 {
        let x = x0 + (13.25 * f64::from(k)) as i32;
        assert_eq!(drag_position(x, w, false), k, "stop {k}");
    }
    assert_eq!(drag_position(x0 + 6, w, false), 0);
    assert_eq!(drag_position(x0 + 7, w, false), 1);
    assert_eq!(drag_position(x0 + 13, w, false), 1);
    // The other layout: x0 = h − 48.
    assert_eq!(drag_position(400 - 48, w, true), 0);
    assert_eq!(drag_position(400 - 49, w, true), 0);
    assert_eq!(drag_position(400 - 48 + 265, w, true), 20);
    // A drag starts only inside the window (exclusive bounds).
    assert!(!drag_starts(400 - 144, w, false) && drag_starts(400 - 143, w, false));
    assert!(drag_starts(400 + 144, w, false) && !drag_starts(400 + 145, w, false));
    assert!(!drag_starts(400 - 59, w, true) && drag_starts(400 - 58, w, true));
    assert!(drag_starts(400 + 229, w, true) && !drag_starts(400 + 230, w, true));
}

#[test]
fn slider_sounds() {
    use super::sliders::*;
    assert_eq!(
        after_move(5, 6),
        Outcome {
            p: 6,
            apply: true,
            sound: Some(1)
        }
    );
    // At the end stop the right arrow changes nothing: no apply, no sound.
    assert_eq!(
        after_move(20, right(20)),
        Outcome {
            p: 20,
            apply: false,
            sound: None
        }
    );
    assert_eq!(ARROW_UP_DOWN_SOUND, 1);
    let c = activate(EntryKind::Choice, 1, 2);
    assert_eq!((c.p, c.apply, c.sound), (0, true, Some(1)));
    assert_eq!(activate(EntryKind::Choice, 0, 2).p, 1);
    let a = activate(EntryKind::Action, 0, 1);
    assert_eq!((a.apply, a.sound), (true, Some(2)));
    let s = activate(EntryKind::Slider, 3, 21);
    assert_eq!((s.p, s.apply, s.sound), (3, false, None));
}

#[test]
fn slider_setting_is_heard_from_the_next_sound_tick() {
    use super::sliders::*;
    // D2RUST: 21 stops of 5 over the integer 0–100 setting; the setter
    // result feeds the volume chain, which reads the setting every update.
    let stops: Vec<i32> = (0..SLIDER_N).map(value_of).collect();
    assert_eq!(stops.len(), 21);
    assert!(stops.windows(2).all(|w| w[1] - w[0] == 5));
    let mut sys = system(rows(3));
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let loud = starts(&mut q)[0].2;
    sys.set_settings(SoundSettings {
        master_volume: value_of(10),
        ..SoundSettings::default()
    });
    assert_eq!(sys.settings().master_volume, 50);
    ticks(&mut sys, &mut w, &mut q, 3);
    sys.request(&mut w, 2, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    let quiet = starts(&mut q)[0].2;
    assert!(quiet < loud, "{quiet} < {loud}");
}

#[test]
fn cache_limit_is_memory_over_100_clamped() {
    use super::system::cache_limit;
    const MIB: u64 = 1024 * 1024;
    assert_eq!(cache_limit(256 * MIB), 3 * 1024 * 1024); // 2.7 MiB → 3 MiB
    assert_eq!(cache_limit(400 * MIB), 4_194_304);
    assert_eq!(cache_limit(500 * MIB), 5_242_880);
    assert_eq!(cache_limit(16 * 1024 * MIB), 5_242_880);
    assert_eq!(super::system::Cache::default().limit, 5_242_880);
}

struct NoPlayer;

impl SoundWorld for NoPlayer {
    fn local_player(&self) -> Option<UnitKey> {
        None
    }
    fn position(&self, _: UnitKey) -> Option<(i32, i32)> {
        None
    }
    fn blocked(&self, _: UnitKey) -> bool {
        false
    }
    fn indoors(&self) -> bool {
        false
    }
    fn state_duck(&self) -> bool {
        false
    }
    fn client_seed(&mut self) -> Option<&mut Seed> {
        None
    }
}

#[test]
fn a_draw_without_a_local_player() {
    let mut w = NoPlayer;
    // n < 1 returns 0 before touching the seed.
    assert_eq!(SoundSystem::roll(&mut w, 0), 0);
    assert_eq!(SoundSystem::roll(&mut w, -3), 0);
    // n ≥ 1: an internal error (debug assert); returning 0 with no step
    // is only the release fallback.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        SoundSystem::roll(&mut NoPlayer, 4)
    }));
    if cfg!(debug_assertions) {
        assert!(r.is_err());
    } else {
        assert_eq!(r.unwrap(), 0);
    }
}

#[test]
fn fade_exact_order() {
    let mut r = rows(3);
    r[1].fade_in = 6;
    r[1].fade_out = 9;
    r[2].fade_in = 6;
    let mut sys = system(r);
    let mut w = World::new();
    // Unknown handle: nothing, no error.
    sys.fade(999, 0, 0, 5);
    assert!(sys.take_errors().is_empty());
    // A running fade to 255 (len 10, ends at 10); a later fade to 255 with
    // len 0 is raised unconditionally to `Fade In` = 6: it ends at 6, no
    // later than the running one, so it replaces it.
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    sys.set_volume(h, 50);
    sys.fade(h, 255, 0, 10);
    assert_eq!(sys.request_by_handle(h).unwrap().fade.unwrap().t1, 10);
    sys.fade(h, 255, 0, 0);
    let f = sys.request_by_handle(h).unwrap().fade.unwrap();
    assert_eq!((f.start, f.end, f.t0, f.t1), (50, 255, 0, 6));
    // Target 0 raises the length to `Fade Out` and sets the stop flag; a
    // running fade to 0 is not turned around by a later fade to 255.
    sys.fade(h, 0, 0, 2);
    let q = sys.request_by_handle(h).unwrap();
    assert!(q.stop);
    assert_eq!(q.fade.unwrap().t1, 9);
    sys.fade(h, 255, 0, 30);
    assert_eq!(sys.request_by_handle(h).unwrap().fade.unwrap().end, 0);
    // len 0, delay 0: volume := target (target 0 had set stop first).
    let h2 = sys.request(&mut w, 2, None, 0, 0, 0);
    sys.fade(h2, 77, 0, 0);
    assert_eq!(sys.request_by_handle(h2).unwrap().volume, 77);
    // More than one unit: nothing at all.
    let mut r = rows(3);
    r[1].group_size = 1;
    r[1].compound = -1;
    let mut sys = system(r);
    let a = sys.request(&mut w, 1, Some(PLAYER), 0, 0, 0);
    assert_eq!(sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0), a);
    assert_eq!(sys.request_by_handle(a).unwrap().units.len(), 2);
    sys.fade(a, 0, 0, 5);
    let q = sys.request_by_handle(a).unwrap();
    assert!(!q.stop && q.fade.is_none());
}

#[test]
fn set_position_clamps_distance_and_writes_no_unit() {
    let mut w = World::new();
    let mut sys = system(rows(3));
    let h = sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    sys.with(&mut w).set_position_for_test(h);
    let q = sys.request_by_handle(h).unwrap();
    // x, y clamped to ±2,000 for distance², z + 640.0.
    assert_eq!(q.pos, [5000.0, -3000.0, 640.0]);
    assert_eq!(q.dist2, 8_000_000.0);
    assert_eq!(q.units, [MONSTER]);
    // No request with the handle: nothing.
    sys.set_position(999, 1, 2, 3);
}

impl SoundCtx<'_> {
    fn set_position_for_test(&mut self, h: crate::audio::calls::Handle) {
        use crate::audio::environment::EnvCalls;
        self.set_position(h, 5000, -3000, 0);
    }
}

#[test]
fn reading_order_ended_then_tracking_then_start_then_playing() {
    // An ended loop only does the end step: it waits, and starts one update later.
    let mut r = rows(4);
    r[1].looped = 1;
    r[2].looped = 1;
    r[2].duration = 3;
    r[3].looped = 1;
    r[3].tracking = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    starts(&mut q);
    sys.set_settings(SoundSettings {
        master_volume: 0,
        ..SoundSettings::default()
    });
    ticks(&mut sys, &mut w, &mut q, 1); // r5 stops it: ended
    cues(&mut q);
    sys.set_settings(SoundSettings::default());
    ticks(&mut sys, &mut w, &mut q, 1); // r1: ended → waiting, no start
    assert!(starts(&mut q).is_empty());
    assert_eq!(
        sys.request_by_handle(h).unwrap().state,
        RequestState::Waiting
    );
    ticks(&mut sys, &mut w, &mut q, 1); // r3 starts it
    assert_eq!(starts(&mut q).len(), 1);

    // The volume step also reads a request the start began in the same pass: a loop
    // with `Duration` 3 kept waiting 10 ticks starts and, with now − start
    // tick > 3, is stopped in that same update.
    let mut sys = system({
        let mut r = rows(4);
        r[2].looped = 1;
        r[2].duration = 3;
        r
    });
    sys.set_settings(SoundSettings {
        master_volume: 0,
        ..SoundSettings::default()
    });
    let d = sys.request(&mut w, 2, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 10);
    sys.set_settings(SoundSettings::default());
    cues(&mut q);
    ticks(&mut sys, &mut w, &mut q, 1);
    let got = cues(&mut q);
    assert!(got.iter().any(|c| matches!(c, Cue::Start(_))), "{got:?}");
    assert!(got.iter().any(|c| matches!(c, Cue::Stop(_))), "{got:?}");
    assert_eq!(sys.request_by_handle(d).unwrap().state, RequestState::Ended);

    // Tracking runs before the start: the start uses the position tracked in the
    // same update.
    let mut sys = system({
        let mut r = rows(4);
        r[3].looped = 1;
        r[3].tracking = 1;
        r
    });
    let mut w = World::new();
    w.positions.insert(MONSTER, (1160, 1000)); // X = 0.5 at creation
    let t = sys.request(&mut w, 3, Some(MONSTER), 0, 0, 0);
    w.positions.insert(MONSTER, (840, 1000)); // moves before the first update
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(sys.request_by_handle(t).unwrap().pos[0], -160.0);
    let (_, _, _, pan) = starts(&mut q)[0];
    assert!(pan < 128, "pan {pan}");
}

#[test]
fn resume_offset_zero_takes_the_fade_in_path() {
    let mut r = rows(3);
    r[1].looped = 1;
    r[1].fade_in = 6;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 1, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    sys.set_settings(SoundSettings {
        master_volume: 0,
        ..SoundSettings::default()
    });
    ticks(&mut sys, &mut w, &mut q, 2);
    // A non-stream voice saves 0: no resume offset.
    assert_eq!(sys.request_by_handle(h).unwrap().resume_offset, 0);
    sys.set_settings(SoundSettings::default());
    ticks(&mut sys, &mut w, &mut q, 2);
    let r = sys.request_by_handle(h).unwrap();
    assert_eq!(r.state, RequestState::Playing);
    // `Fade In` ticks (6), not the 3-tick resume fade.
    assert_eq!(r.fade.map(|f| f.t1 - f.t0), Some(6));
}

#[test]
fn occlusion_steps_follow_the_f32_table() {
    use super::system::occlusion_step;
    // (occ, toward 0.5, toward 0) bit patterns of the spec table.
    let t: [(u32, Option<u32>, Option<u32>); 23] = [
        (0x00000000, Some(0x3d4ccccd), None),
        (0x32000000, Some(0x3d4ccccf), Some(0x00000000)),
        (0x3d4cccc3, Some(0x3dccccc8), Some(0x00000000)),
        (0x3d4ccccd, Some(0x3dcccccd), Some(0x00000000)),
        (0x3d4ccccf, Some(0x3dccccce), Some(0x32000000)),
        (0x3dccccc8, Some(0x3e199997), Some(0x3d4cccc3)),
        (0x3dcccccd, Some(0x3e19999a), Some(0x3d4ccccd)),
        (0x3dccccce, Some(0x3e19999a), Some(0x3d4ccccf)),
        (0x3e199997, Some(0x3e4cccca), Some(0x3dccccc8)),
        (0x3e19999a, Some(0x3e4ccccd), Some(0x3dccccce)),
        (0x3e4cccca, Some(0x3e7ffffd), Some(0x3e199997)),
        (0x3e4ccccd, Some(0x3e800000), Some(0x3e19999a)),
        (0x3e7ffffd, Some(0x3e999998), Some(0x3e4cccca)),
        (0x3e800000, Some(0x3e99999a), Some(0x3e4ccccd)),
        (0x3e999998, Some(0x3eb33332), Some(0x3e7ffffd)),
        (0x3e99999a, Some(0x3eb33334), Some(0x3e800000)),
        (0x3eb33332, Some(0x3ecccccc), Some(0x3e999998)),
        (0x3eb33334, Some(0x3eccccce), Some(0x3e99999a)),
        (0x3ecccccc, Some(0x3ee66666), Some(0x3eb33332)),
        (0x3eccccce, Some(0x3ee66668), Some(0x3eb33334)),
        (0x3ee66666, Some(0x3f000000), Some(0x3ecccccc)),
        (0x3ee66668, Some(0x3f000000), Some(0x3eccccce)),
        (0x3f000000, None, Some(0x3ee66666)),
    ];
    for (occ, up, down) in t {
        let o = f32::from_bits(occ);
        if let Some(u) = up {
            assert_eq!(occlusion_step(o, 0.5).to_bits(), u, "up from {occ:08x}");
        }
        if let Some(d) = down {
            assert_eq!(occlusion_step(o, 0.0).to_bits(), d, "down from {occ:08x}");
        }
    }
    // The mean of several units' values is the target (added in f32).
    assert_eq!(
        occlusion_step(0.0, (0.5f32 + 0.0) / 2.0).to_bits(),
        0x3d4ccccd
    );

    // Through the request: a thunder request (group base 202) walks the
    // table toward 0.5 when indoors, and the occlusion reaches the output
    // only as the voice's occlusion value (a device cue).
    let mut r = rows(210);
    r[203].tracking = 1;
    r[202].group_size = 3;
    r[203].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let h = sys.request(&mut w, 203, Some(MONSTER), 0, FLAG_EXACT, 0);
    w.indoors = true;
    let mut got = Vec::new();
    for _ in 0..10 {
        ticks(&mut sys, &mut w, &mut q, 1);
        got.push(sys.request_by_handle(h).unwrap().occlusion.to_bits());
    }
    assert_eq!(
        got,
        [
            0x3d4ccccd, 0x3dcccccd, 0x3e19999a, 0x3e4ccccd, 0x3e800000, 0x3e99999a, 0x3eb33334,
            0x3eccccce, 0x3ee66668, 0x3f000000
        ]
    );
    let dev: Vec<u32> = cues(&mut q)
        .into_iter()
        .filter_map(|c| match c {
            Cue::Device(d) => Some(d.occlusion),
            _ => None,
        })
        .collect();
    assert_eq!(dev.last(), Some(&0x3f000000));
}

#[test]
fn natural_end_frees_one_shots_but_never_loops() {
    let mut r = rows(4);
    r[2].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let one = sys.request(&mut w, 1, None, 0, 0, 0);
    let looped = sys.request(&mut w, 2, None, 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(starts(&mut q).len(), 2);
    // 1,000 frames at 1,000 Hz = 1 s = 25 ticks of 40 ms: the first upkeep
    // with elapsed × 40 ms ≥ duration ends it; the next update frees it.
    ticks(&mut sys, &mut w, &mut q, 24);
    assert_eq!(
        sys.request_by_handle(one).unwrap().state,
        RequestState::Playing
    );
    ticks(&mut sys, &mut w, &mut q, 1);
    assert_eq!(
        sys.request_by_handle(one).unwrap().state,
        RequestState::Ended
    );
    ticks(&mut sys, &mut w, &mut q, 1);
    assert!(sys.request_by_handle(one).is_none());
    // The loop plays on, however long.
    ticks(&mut sys, &mut w, &mut q, 200);
    assert_eq!(
        sys.request_by_handle(looped).unwrap().state,
        RequestState::Playing
    );
}

#[test]
fn stream_rows_use_no_cache() {
    use crate::audio::DeviceChange;
    let mut r = rows(3);
    r[1].stream = 1;
    r[1].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    sys.request(&mut w, 1, None, 0, 0, 40);
    ticks(&mut sys, &mut w, &mut q, 1);
    // No cache entry, no load state change, no cached bytes.
    assert_eq!(sys.cache().total, 0);
    assert_ne!(sys.table().get(1).unwrap().load, LoadState::Loaded);
    // The start offset (× 4 bytes, 2-byte frames: frame 80) and the loop
    // flag go to the stream player.
    let starts: Vec<Option<u64>> = cues(&mut q)
        .into_iter()
        .filter_map(|c| match c {
            Cue::Device(DeviceChange { start_frame, .. }) => Some(start_frame),
            _ => None,
        })
        .collect();
    assert_eq!(starts, [Some(80)]);
    assert!((0..CHANNELS).any(|c| sys.channel_request(c).is_some_and(|r| r.id == 1)));
}

#[test]
fn exact_float_steps_worked_values() {
    // Unit sounds have z = 640; a no-unit request (0, 0, 320).
    assert_eq!(mode0_gain_pan([320.0, 0.0, 640.0]), (228, 229));
    assert_eq!(mode0_gain_pan([1280.0, 0.0, 640.0]), (114, 255));
    assert_eq!(mode0_gain_pan([-640.0, 200.0, 640.0]), (176, 0));
    assert_eq!(mode0_gain_pan([0.0, 0.0, 640.0]), (255, 128));
    assert_eq!(mode0_gain_pan([0.0, 0.0, 320.0]), (255, 128));
    // d = f32(√ f32(x² + y²)); v = trunc(f32(max − d) × v / f32(max −
    // min)): falloff 1 is (60, 700); d = 380: (700 − 380) × 255 / 640 = 127.
    assert_eq!(linear_falloff(255, 380.0, 0.0, 1), 127);
    assert_eq!(linear_falloff(255, 30.0, 0.0, 1), 255);
    // r > 100 → 100; r < 2 → no attenuation.
    assert_eq!(mode0_gain_pan([1.0e9, 0.0, 0.0]).1, 255);
    assert_eq!(mode0_gain_pan([100.0, 0.0, 100.0]).0, 255);
}

#[test]
fn global_gain_is_255_at_every_game_send() {
    // G = 255 at every volume send of a game sound tick: the device
    // conversion is the identity before occlusion.
    assert_eq!(DEVICE_GAIN, 255);
    for v in 0..=255 {
        assert_eq!(device_occluded(v, 0.0), v);
    }
}

#[test]
fn unit_free_detaches_loops_and_leaves_one_shots() {
    use crate::audio::triggers::detach_all;
    let mut r = rows(4);
    r[1].looped = 1;
    let mut sys = system(r);
    let mut w = World::new();
    let mut q = TriggerQueue::new();
    let lp = sys.request(&mut w, 1, Some(MONSTER), 0, 0, 0);
    let one = sys.request(&mut w, 2, Some(MONSTER), 0, 0, 0);
    ticks(&mut sys, &mut w, &mut q, 1);
    detach_all(&mut sys.with(&mut w), MONSTER, false);
    // The loop lost its last unit and stops; the one-shot keeps playing at
    // its last position; the unit's list is empty.
    assert!(sys.request_by_handle(lp).unwrap().stop);
    assert!(!sys.request_by_handle(one).unwrap().stop);
    assert!(sys.unit_requests(MONSTER).is_empty());
    // A second pass (the per-type frees) finds nothing to do.
    detach_all(&mut sys.with(&mut w), MONSTER, true);
    assert!(!sys.request_by_handle(one).unwrap().stop);
}

#[test]
fn variant_pick_ends_when_the_roll_is_stuck_at_zero() {
    let mut r = rows(110);
    r[100].group_size = 4;
    let mut t = table(r);
    t.record_history(100, 100);
    t.record_history(100, 100);
    // A roll that always gives 0 (no client seed): the pick must end.
    assert_eq!(t.pick_variant(100, &mut |_| 0), 100);
}

#[test]
fn device_occlusion_product_is_exact() {
    // (1 - 0.05f) × 200 truncates to 189, not the 190 an f32 product gives.
    assert_eq!(device_occluded(200, 0.05), 189);
    assert_eq!(device_occluded(255, 0.0), 255);
}

/// The request log keeps every call at the entry, rejected ones included,
/// with its arguments as given; off, it records nothing.
#[test]
fn request_log_keeps_every_call() {
    let mut sys = system(rows(4));
    let mut w = World::new();
    sys.request(&mut w, 1, None, 0, 0, 0);
    assert!(sys.take_request_log().is_empty());
    sys.log_requests(true);
    sys.request(&mut w, 0, None, 0, 0, 0);
    sys.request(&mut w, 1, Some(MONSTER), 3, FLAG_EXACT, 5);
    let log = sys.take_request_log();
    assert_eq!(
        log.iter()
            .map(|c| (c.id, c.unit, c.delay, c.flags, c.offset))
            .collect::<Vec<_>>(),
        vec![(0, None, 0, 0, 0), (1, Some(MONSTER), 3, FLAG_EXACT, 5)]
    );
    assert!(sys.take_request_log().is_empty());
}

// --- mod gains (seams MS14c) --------------------------------------------------

/// A mod game's mix: effects at half, music silent, speech unchanged.
struct HalfEffects;

impl crate::mods::AudioGains for HalfEffects {
    fn percent(&self, category: crate::mods::SoundCategory, _id: i32) -> i32 {
        use crate::mods::SoundCategory::*;
        match category {
            Effects => 50,
            Music => 0,
            Speech => 100,
        }
    }
}

// A mod game's gains scale a sound by its category; without them the
// volume is D2RUST's.
#[test]
fn mod_audio_gains_scale_by_category() {
    let start = |mods: Option<crate::mods::ClientMods>| {
        let mut sys = system(rows(3));
        if let Some(m) = mods {
            sys.set_mods(m);
        }
        let mut w = World::new();
        let mut q = TriggerQueue::new();
        sys.request(&mut w, 1, None, 0, 0, 0);
        ticks(&mut sys, &mut w, &mut q, 1);
        starts(&mut q)
    };
    assert_eq!(start(None), [(0, 1, 255, 128)]);
    assert_eq!(
        start(Some(crate::mods::ClientMods::new())),
        [(0, 1, 255, 128)]
    );
    let half = crate::mods::ClientMods::new().with_audio_gains(HalfEffects);
    assert_eq!(start(Some(half)), [(0, 1, 127, 128)]);
}

// The categories: songs, the music path and sounds that follow the music
// volume are music; the speech ids are speech; the rest are effects.
#[test]
fn sound_categories() {
    use crate::mods::SoundCategory::*;
    let mut r = rows(5000);
    r[3].music_vol = 1;
    let env = [SoundEnvironRow {
        song: 2,
        ..SoundEnvironRow::default()
    }];
    let sys = SoundSystem::new(SoundTableData::new(r, &env), bank());
    assert_eq!(sys.category(1), Effects);
    assert_eq!(sys.category(2), Music);
    assert_eq!(sys.category(3), Music);
    assert_eq!(sys.category(4657), Music);
    assert_eq!(sys.category(0xB76), Speech);
}

#[test]
fn sample_duration_and_request_delay_follow_the_game_rate() {
    for hz in [25, 60] {
        let mut sys = system(rows(3));
        sys.set_tick_rate(d2_sim::tick::TickRate::new(hz).unwrap());
        let mut world = World::new();
        let mut queue = TriggerQueue::new();
        let handle = sys.request(&mut world, 1, None, 25, 0, 0);
        assert_eq!(sys.request_by_handle(handle).unwrap().start_tick, hz);
        ticks(&mut sys, &mut world, &mut queue, hz + 1);
        assert_eq!(
            sys.request_by_handle(handle).unwrap().state,
            RequestState::Playing
        );
        ticks(&mut sys, &mut world, &mut queue, hz - 1);
        assert_eq!(
            sys.request_by_handle(handle).unwrap().state,
            RequestState::Playing
        );
        ticks(&mut sys, &mut world, &mut queue, 1);
        assert_eq!(
            sys.request_by_handle(handle).unwrap().state,
            RequestState::Ended
        );
    }
}

#[test]
fn preload_maintains_its_one_second_period_at_both_rates() {
    for hz in [25, 60] {
        let mut rows = rows(3);
        rows[1].async_only = 1;
        let mut sys = system(rows);
        sys.set_tick_rate(d2_sim::tick::TickRate::new(hz).unwrap());
        let mut world = World::new();
        let mut queue = TriggerQueue::new();
        sys.request(&mut world, 1, None, 0, 0, 0);
        ticks(&mut sys, &mut world, &mut queue, hz);
        assert!(starts(&mut queue).is_empty());
        ticks(&mut sys, &mut world, &mut queue, 1);
        assert_eq!(starts(&mut queue), [(hz, 1, 255, 128)]);
    }
}
