//! Test vectors of the environment machines against a fake of the sound
//! layer (call log + scripted rolls).

use std::collections::{BTreeMap, VecDeque};

use super::*;
use crate::audio::calls::FLAG_NO_FADE_IN;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Request {
        id: i32,
        unit: Option<UnitKey>,
        flags: u32,
        offset: u32,
        h: Handle,
    },
    SetVolume(Handle, i32),
    Fade(Handle, i32, u32, u32),
    StopHandle(Handle),
    StopId(i32),
    StopSongs,
    Stop52(i32, i32),
    Stop72(i32, i32),
    SetPosition(Handle, i32, i32, i32),
    Roll(i32, u32),
}

#[derive(Clone, Debug)]
struct Req {
    h: Handle,
    id: i32,
    vol: i32,
}

#[derive(Default)]
struct Fake {
    rate: d2_sim::tick::TickRate,
    t: u32,
    calls: Vec<Call>,
    rolls: VecDeque<u32>,
    next: Handle,
    reqs: Vec<Req>,
    positions: BTreeMap<i32, u32>,
    blocks: BTreeMap<i32, [i32; 3]>,
    speech: bool,
    /// Ids whose request returns 0.
    refuse: Vec<i32>,
}

impl Fake {
    fn take(&mut self) -> Vec<Call> {
        std::mem::take(&mut self.calls)
    }

    /// Requests, song stops and id stops only.
    fn music(&mut self) -> Vec<Call> {
        self.take()
            .into_iter()
            .filter(|c| matches!(c, Call::Request { .. } | Call::StopSongs | Call::StopId(_)))
            .collect()
    }
}

impl SoundCalls for Fake {
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.rate
    }
    fn request(
        &mut self,
        id: i32,
        unit: Option<UnitKey>,
        delay: u32,
        flags: u32,
        offset: u32,
    ) -> Handle {
        assert_eq!(delay, 0);
        let h = if id < 1 || self.refuse.contains(&id) {
            0
        } else {
            self.next += 1;
            self.reqs.push(Req {
                h: self.next,
                id,
                vol: 255,
            });
            self.next
        };
        self.calls.push(Call::Request {
            id,
            unit,
            flags,
            offset,
            h,
        });
        h
    }
    fn set_volume(&mut self, h: Handle, v: i32) {
        if let Some(r) = self.reqs.iter_mut().find(|r| r.h == h) {
            r.vol = v;
        }
        self.calls.push(Call::SetVolume(h, v));
    }
    fn fade(&mut self, h: Handle, target: i32, delay: u32, len: u32) {
        self.calls.push(Call::Fade(h, target, delay, len));
    }
    fn detach(&mut self, _: Handle, _: UnitKey, _: bool) {
        unreachable!("not used by the environment machines");
    }
    fn stop_handle(&mut self, h: Handle) {
        self.calls.push(Call::StopHandle(h));
    }
    fn stop_id(&mut self, id: i32) {
        self.calls.push(Call::StopId(id));
    }
    fn stop_songs(&mut self) {
        self.calls.push(Call::StopSongs);
    }
    fn stop_52_71_except(&mut self, a: i32, b: i32) {
        self.calls.push(Call::Stop52(a, b));
    }
    fn stop_72_201_except(&mut self, a: i32, b: i32) {
        self.calls.push(Call::Stop72(a, b));
    }
    fn stop_speech(&mut self) {
        unreachable!("not used by the environment machines");
    }
    fn speaking(&self, _: UnitKey) -> bool {
        unreachable!("not used by the environment machines");
    }
    fn any_speech(&self) -> bool {
        self.speech
    }
    fn is_active(&self, h: Handle) -> bool {
        self.reqs.iter().any(|r| r.h == h)
    }
    fn sound_tick(&self) -> u32 {
        self.t
    }
    fn roll(&mut self, n: i32) -> u32 {
        let r = self.rolls.pop_front().unwrap_or(0);
        assert!(n > 0 && r < n as u32, "roll({n}) scripted {r}");
        self.calls.push(Call::Roll(n, r));
        r
    }
}

impl EnvCalls for Fake {
    fn id_requested(&self, id: i32) -> bool {
        self.reqs.iter().any(|r| r.id == id)
    }
    fn music_active(&self) -> bool {
        self.reqs.iter().any(|r| (4657..=4698).contains(&r.id))
    }
    fn play_position(&self, id: i32) -> Option<u32> {
        self.positions.get(&id).copied()
    }
    fn blocks(&self, id: i32) -> [i32; 3] {
        self.blocks.get(&id).copied().unwrap_or([-1, -1, -1])
    }
    fn volume(&self, h: Handle) -> Option<i32> {
        self.reqs.iter().find(|r| r.h == h).map(|r| r.vol)
    }
    fn set_position(&mut self, h: Handle, x: i32, y: i32, z: i32) {
        self.calls.push(Call::SetPosition(h, x, y, z));
    }
}

#[derive(Default)]
struct Hooks {
    quest_ok: bool,
    events: Vec<u8>,
}

impl EnvHooks for Hooks {
    fn quest_check(&self, _: u8) -> bool {
        self.quest_ok
    }
    fn player_event(&mut self, _: &mut dyn SoundCalls, e: u8) {
        self.events.push(e);
    }
}

const P: UnitKey = UnitKey::new(0, 1);
const SONG_A: i32 = 4660;
const SONG_B: i32 = 4670;
/// `music_caves` blocks (resume point test vectors).
const CAVES_BLOCKS: [i32; 3] = [1_478_063, -1, -1];
/// `music_wilderness` blocks.
const WILD_BLOCKS: [i32; 3] = [3_457_024, 6_755_328, -1];

fn song_row(song: i32) -> EnvRow {
    EnvRow {
        song,
        ..EnvRow::default()
    }
}

fn amb_row() -> EnvRow {
    EnvRow {
        song: 0,
        day_ambience: 52,
        night_ambience: 53,
        day_event: 72,
        night_event: 80,
        event_delay: 250,
        indoors: 0,
        material1: 2,
        material2: 3,
    }
}

fn input(level: u32, env: Option<EnvRow>) -> TickInput {
    TickInput {
        level,
        env,
        day_phase: 1,
        weather_active: false,
        rain_level: 0,
        master_volume: 100,
        music_volume: 50,
        c: 0,
        player: Some(PlayerState {
            key: P,
            alive: true,
            last_voice: 0,
        }),
        level_count: 137,
    }
}

fn env() -> Environment {
    Environment::new(SongRange::LIVE)
}

fn req(id: i32, unit: Option<UnitKey>, flags: u32, offset: u32, h: Handle) -> Call {
    Call::Request {
        id,
        unit,
        flags,
        offset,
        h,
    }
}

#[test]
fn no_level_does_nothing() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.t = 500;
    let mut inp = input(0, Some(amb_row()));
    inp.weather_active = true;
    assert_eq!(e.tick(&mut s, &mut h, &inp), None);
    assert!(s.calls.is_empty());
    assert_eq!(e, env());
}

#[test]
fn env_row_out_of_range_is_no_row() {
    let rows = vec![song_row(4657); 50];
    assert_eq!(env_row(&rows, 49), Some(song_row(4657)));
    assert_eq!(env_row(&rows, 50), None);
    // No row: song 0, ambience 0, no event; the music stops all songs.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    e.tick(&mut s, &mut h, &input(1, None));
    assert_eq!(e.music.cur, 0);
    assert_eq!(e.ambience.bed, 0);
    assert_eq!(s.music(), vec![Call::StopSongs]);
    // The song range of the live rows.
    let rows = [song_row(0), song_row(4684), song_row(4657), song_row(-1)];
    assert_eq!(SongRange::from_rows(&rows), Some(SongRange::LIVE));
    assert_eq!(SongRange::from_rows(&[song_row(0)]), None);
}

#[test]
fn day_is_phase_1_to_3() {
    let day: Vec<u32> = (0..8).filter(|&p| is_day(p)).collect();
    assert_eq!(day, vec![1, 2, 3]);
}

#[test]
fn first_song_starts_at_once_without_fade() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.t = 10;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    assert_eq!(e.music.cur, SONG_A);
    assert_eq!(e.music.ts, 10);
    assert_eq!(
        s.music(),
        vec![Call::StopSongs, req(SONG_A, None, FLAG_NO_FADE_IN, 0, 1)]
    );
    // Next tick: the request exists, nothing more.
    s.t = 11;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    assert!(s.music().is_empty());
    // A `Song` outside the range is song 0 (stinger ids included).
    let (mut e, mut s) = (env(), Fake::default());
    e.tick(&mut s, &mut h, &input(1, Some(song_row(4685))));
    assert_eq!(e.music.cur, 0);
    assert_eq!(s.music(), vec![Call::StopSongs]);
}

#[test]
fn level_change_waits_75_ticks() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    e.music.set_resume(SONG_B, 1234);
    s.take();
    for t in 100..=174 {
        s.t = t;
        e.tick(&mut s, &mut h, &input(2, Some(song_row(SONG_B))));
        assert_eq!(e.music.cur, SONG_A, "T {t}");
        assert!(s.music().is_empty(), "T {t}");
    }
    assert_eq!(e.music.tl, 100);
    assert_eq!(e.music.last_level, 2);
    s.t = 175;
    e.tick(&mut s, &mut h, &input(2, Some(song_row(SONG_B))));
    assert_eq!(e.music.cur, SONG_B);
    assert_eq!(e.music.ts, 175);
    // A still active → flags 0 (fade in); A gets the stop, B its offset.
    assert_eq!(
        s.music(),
        vec![Call::StopSongs, req(SONG_B, None, 0, 1234, 2)]
    );
}

#[test]
fn music_volume_0_stops_songs_but_not_stingers() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    let mut inp = input(1, Some(song_row(SONG_A)));
    inp.music_volume = 0;
    e.tick(&mut s, &mut h, &inp);
    assert_eq!(e.music.cur, SONG_A);
    assert_eq!(s.music(), vec![Call::StopSongs]);
    let st = stinger_for_event(33, 2961).unwrap();
    inp.c = 500;
    e.music.start_stinger(&mut s, inp.c, st);
    assert_eq!(s.music(), vec![Call::StopSongs]);
    s.t = 1;
    e.tick(&mut s, &mut h, &inp);
    // Song stop, then M at tM = C + 0 (the stinger holds the music); no song request.
    assert_eq!(s.music(), vec![Call::StopSongs, req(4685, None, 0, 0, 1)]);
    // At the hold end (C 975) the line (due at C 925) is requested on P
    // and the stinger ends; not audible → no stop of M.
    inp.c = 975;
    s.t = 2;
    e.tick(&mut s, &mut h, &inp);
    assert!(!e.music.stinger.active);
    assert_eq!(
        s.music(),
        vec![Call::StopSongs, req(2961, Some(P), 0, 0, 2)]
    );
    // Master Volume 0 is not audible either.
    let (mut e, mut s) = (env(), Fake::default());
    let mut inp = input(1, Some(song_row(SONG_A)));
    inp.master_volume = 0;
    e.tick(&mut s, &mut h, &inp);
    assert_eq!(s.music(), vec![Call::StopSongs]);
}

#[test]
fn resume_points() {
    assert_eq!(resume_point(CAVES_BLOCKS, 1_000_000), 1_478_063);
    assert_eq!(resume_point(CAVES_BLOCKS, 2_000_000), 0);
    assert_eq!(resume_point(WILD_BLOCKS, 4_000_000), 6_755_328);
    assert_eq!(resume_point(WILD_BLOCKS, 0), 3_457_024);
    assert_eq!(resume_point(WILD_BLOCKS, 7_000_000), 0);
    assert_eq!(resume_point([-1, -1, -1], 5), 0);
    assert_eq!(resume_point([10, 20, 30], 25), 30);
}

#[test]
fn resume_bookkeeping_every_125_ticks() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.blocks.insert(SONG_A, CAVES_BLOCKS);
    s.t = 0;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    s.positions.insert(SONG_A, 1_000_000);
    s.t = 125;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    assert_eq!(e.music.resume(SONG_A), 0, "T must exceed Ts + 125");
    s.t = 126;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    assert_eq!(e.music.resume(SONG_A), 1_478_063);
    assert_eq!(e.music.ts, 126);
    // Not playing: nothing.
    s.positions.clear();
    s.t = 300;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    assert_eq!(e.music.ts, 126);
}

#[test]
fn reset_clears_music_and_stinger() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.t = 7;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    e.music.set_resume(SONG_A, 99);
    e.music
        .start_stinger(&mut s, 3, stinger_for_event(35, 2961).unwrap());
    e.music.reset();
    assert_eq!(e.music, Music::new(SongRange::LIVE));
}

#[test]
fn stinger_event_35_at_c_1000() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    let mut inp = input(1, Some(song_row(SONG_A)));
    e.tick(&mut s, &mut h, &inp);
    s.take();
    s.reqs.clear(); // the song ended: §2 r6 would request it again
    let st = stinger_for_event(35, 2961).unwrap();
    assert_eq!(st.s, 2963);
    e.music.start_stinger(&mut s, 1000, st);
    assert_eq!(
        e.music.stinger,
        Stinger {
            active: true,
            m: 4688,
            tm: 1000,
            m_pending: true,
            s: 2963,
            ts: 1250,
            s_pending: true,
            th: 1300,
        }
    );
    let mut log = Vec::new();
    for c in 1000..=1300 {
        inp.c = c;
        s.t = c;
        e.tick(&mut s, &mut h, &inp);
        for call in s.music() {
            log.push((c, call));
        }
    }
    assert_eq!(
        log,
        vec![
            // The stinger's own song stop, then M at tM.
            (1000, Call::StopSongs),
            (1000, req(4688, None, 0, 0, 2)),
            (1250, req(2963, Some(P), 0, 0, 3)),
            // Hold over: the stinger track is stopped, the song requested.
            (1300, Call::StopId(4688)),
            (1300, Call::StopSongs),
            (1300, req(SONG_A, None, 0, 0, 4)),
        ]
    );
    assert!(!e.music.stinger.active);
}

#[test]
fn stinger_stores_block_k_of_playing_song() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.blocks.insert(SONG_A, WILD_BLOCKS);
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    s.take();
    s.positions.insert(SONG_A, 5);
    s.t = 40;
    e.music
        .start_stinger(&mut s, 0, stinger_for_event(34, 2961).unwrap());
    assert_eq!(e.music.resume(SONG_A), 3_457_024);
    assert_eq!(e.music.ts, 40);
    assert_eq!(s.take(), vec![Call::StopSongs]);
    // k = 0 → 0.
    e.music
        .start_stinger(&mut s, 0, stinger_for_event(33, 2961).unwrap());
    assert_eq!(e.music.resume(SONG_A), 0);
    // Not playing: resume and Ts kept.
    e.music.set_resume(SONG_A, 77);
    s.positions.clear();
    s.t = 90;
    e.music
        .start_stinger(&mut s, 0, stinger_for_event(34, 2961).unwrap());
    assert_eq!(e.music.resume(SONG_A), 77);
    assert_eq!(e.music.ts, 40);
}

#[test]
fn stinger_line_needs_living_player() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    let mut inp = input(1, Some(song_row(SONG_A)));
    inp.player = Some(PlayerState {
        key: P,
        alive: false,
        last_voice: 0,
    });
    e.music
        .start_stinger(&mut s, 0, stinger_for_event(35, 2961).unwrap());
    s.take();
    inp.c = 250;
    e.tick(&mut s, &mut h, &inp);
    assert_eq!(s.music(), vec![req(4688, None, 0, 0, 1)]);
    assert!(!e.music.stinger.s_pending);
}

#[test]
fn stinger_rearm() {
    let mut m = Music::new(SongRange::LIVE);
    m.rearm_stinger(100, 25, true);
    assert_eq!(m.stinger, Stinger::default(), "only while active");
    let mut s = Fake::default();
    m.start_stinger(&mut s, 1000, stinger_for_event(35, 2961).unwrap());
    m.rearm_stinger(1100, 25, true);
    assert_eq!(
        (m.stinger.ts, m.stinger.s_pending, m.stinger.th),
        (1125, true, 1175)
    );
    m.rearm_stinger(1100, 25, false);
    assert_eq!(
        (m.stinger.ts, m.stinger.s_pending, m.stinger.th),
        (1125, false, 1175)
    );
}

#[test]
fn stinger_table() {
    // (event, M, dM, H, k, dS, play) as in the stinger caller table.
    let rows: [(u8, i32, u32, u32, u8, u32, bool); 11] = [
        (33, 4685, 0, 475, 0, 425, true),
        (34, 4686, 475, 800, 1, 525, true),
        (35, 4688, 0, 300, 1, 250, true),
        (37, 0, 0, 1500, 0, 0, false),
        (50, 4693, 475, 1100, 1, 525, true),
        (52, 4694, 0, 425, 1, 375, true),
        (66, 4692, 0, 425, 0, 375, true),
        (75, 4695, 0, 475, 0, 325, true),
        (80, 4698, 0, 500, 1, 475, true),
        (82, 4697, 0, 550, 1, 400, true),
        (83, 4696, 0, 525, 1, 425, true),
    ];
    for (e, m, dm, h, k, ds, play) in rows {
        let k = if k == 1 {
            ResumeBlock::Block(BlockIndex::B1)
        } else {
            ResumeBlock::Start
        };
        assert_eq!(
            stinger_for_event(e, 3273),
            Some(StingerArgs {
                m,
                dm,
                h,
                k,
                s: 3273 + i32::from(e) - 33,
                ds,
                play
            }),
            "event {e}"
        );
    }
    for e in (0..=255u8).filter(|e| !rows.iter().any(|r| r.0 == *e)) {
        assert_eq!(stinger_for_event(e, 3273), None);
    }
    let silent = |m, h| StingerArgs {
        m,
        dm: 0,
        h,
        k: ResumeBlock::Start,
        s: 0,
        ds: 0,
        play: false,
    };
    assert_eq!(STINGER_COMPELLING, silent(4687, 350));
    assert_eq!(STINGER_IZUAL, silent(4691, 250));

    // Event 37 silences music for 1,500 updates and never plays its line.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    let mut inp = input(1, Some(song_row(SONG_A)));
    e.tick(&mut s, &mut h, &inp);
    s.reqs.clear();
    s.take();
    e.music
        .start_stinger(&mut s, 0, stinger_for_event(37, 2961).unwrap());
    let mut requested = Vec::new();
    for c in 0..1500 {
        inp.c = c;
        e.tick(&mut s, &mut h, &inp);
        requested.extend(
            s.music()
                .into_iter()
                .filter(|c| matches!(c, Call::Request { .. })),
        );
    }
    assert_eq!(requested, vec![req(0, None, 0, 0, 0)]);
    inp.c = 1500;
    e.tick(&mut s, &mut h, &inp);
    // No music request left active and resume 0 → no fade-in.
    assert!(s
        .music()
        .contains(&req(SONG_A, None, FLAG_NO_FADE_IN, 0, 2)));
}

/// Drives one tick at T = C = `t` on level `l` (no song, no ambience).
fn entry_tick(e: &mut Environment, s: &mut Fake, h: &mut Hooks, l: u32, t: u32, last_voice: u32) {
    let mut inp = input(l, Some(EnvRow::default()));
    inp.c = t;
    inp.player = Some(PlayerState {
        key: P,
        alive: true,
        last_voice,
    });
    s.t = t;
    e.tick(s, h, &inp);
}

#[test]
fn entry_line_on_first_entry() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    h.quest_ok = true;
    entry_tick(&mut e, &mut s, &mut h, 1, 1000, 0);
    // Level 3 (`find_wilderness`, q 1) entered at T 1100: line at T − Tl = 62.
    for t in 1100..=1161 {
        entry_tick(&mut e, &mut s, &mut h, 3, t, 0);
    }
    assert!(h.events.is_empty());
    entry_tick(&mut e, &mut s, &mut h, 3, 1162, 0);
    assert_eq!(h.events, vec![47]);
    assert_eq!(e.music.last_announced, 3);
    // Every level of the record is flagged: entering 5 plays nothing.
    for l in [2, 3, 4, 5, 6, 7] {
        assert!(e.entry.is_flagged(l));
    }
    for t in 1200..1300 {
        entry_tick(&mut e, &mut s, &mut h, 5, t, 0);
    }
    assert_eq!(h.events, vec![47]);
}

#[test]
fn entry_line_skipped_is_lost() {
    // Hero spoke within 62 updates: flagged, no line, never retried.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    h.quest_ok = true;
    for t in 1000..1100 {
        entry_tick(&mut e, &mut s, &mut h, 8, t, 1000);
    }
    assert!(h.events.is_empty());
    assert!(e.entry.is_flagged(8));
    entry_tick(&mut e, &mut s, &mut h, 1, 1200, 0);
    for t in 1300..1400 {
        entry_tick(&mut e, &mut s, &mut h, 8, t, 0);
    }
    assert!(h.events.is_empty());
    // C − P+0x7C = 63 passes.
    let (mut e, mut s) = (env(), Fake::default());
    for t in 1000..=1062 {
        entry_tick(&mut e, &mut s, &mut h, 8, t, 1062 - 62 - 1);
    }
    assert_eq!(h.events, vec![41]);
    // Quest check fails → nothing, flagged.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    for t in 0..100 {
        entry_tick(&mut e, &mut s, &mut h, 74, t + 1000, 0);
    }
    assert!(h.events.is_empty());
    assert!(e.entry.is_flagged(74));
    // Any speech playing → nothing.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    h.quest_ok = true;
    s.speech = true;
    for t in 1000..1100 {
        entry_tick(&mut e, &mut s, &mut h, 58, t, 0);
    }
    assert!(h.events.is_empty());
    assert!(e.entry.is_flagged(58));
    // L ≥ level count: nothing, not flagged.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    h.quest_ok = true;
    let mut inp = input(132, Some(EnvRow::default()));
    inp.level_count = 132;
    for t in 1000..1100 {
        inp.c = t;
        s.t = t;
        e.tick(&mut s, &mut h, &inp);
    }
    assert!(h.events.is_empty());
    assert!(!e.entry.is_flagged(132));
}

#[test]
fn sound_init_clears_the_flags_but_not_the_last_level() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    h.quest_ok = true;
    entry_tick(&mut e, &mut s, &mut h, 1, 1000, 0);
    for t in 1100..=1162 {
        entry_tick(&mut e, &mut s, &mut h, 3, t, 0);
    }
    assert_eq!(h.events, vec![47]);
    e.music.cur = SONG_A;
    // A new game: sound on, so the music reset runs; the flags are
    // cleared, the last level checked (3) stays.
    e.sound_init(false);
    assert!(!e.entry.is_flagged(3) && !e.entry.is_flagged(5));
    assert_eq!(e.entry.last_checked(), 3);
    assert_eq!(e.music.cur, 0);
    // The new game's first announced level equals it: no line until L
    // changes once.
    let (mut s, mut h2) = (Fake::default(), Hooks::default());
    h2.quest_ok = true;
    for t in 0..=62 {
        entry_tick(&mut e, &mut s, &mut h2, 3, t, 0);
    }
    assert!(h2.events.is_empty());
    assert!(!e.entry.is_flagged(3));
    // `-ns`: no music reset.
    e.music.cur = SONG_A;
    e.sound_init(true);
    assert_eq!(e.music.cur, SONG_A);
}

#[test]
fn entry_table() {
    let rows: [(&[u32], u8, u8); 14] = [
        (&[17], 2, 38),
        (&[34, 35, 36, 37], 6, 40),
        (&[8], 1, 41),
        (&[29, 30, 31], 6, 42),
        (&[26, 27], 3, 43),
        (&[20, 21, 22, 23, 24, 25], 5, 44),
        (&[38], 6, 46),
        (&[2, 3, 4, 5, 6, 7], 1, 47),
        (&[74], 12, 55),
        (&[58], 13, 56),
        (&[110], 31, 76),
        (&[120], 35, 78),
        (&[121], 34, 77),
        (&[132], 36, 79),
    ];
    for (rec, (levels, q, ev)) in ENTRY_RECORDS.iter().zip(rows) {
        assert_eq!((rec.levels, rec.quest, rec.event), (levels, q, ev));
        // Each level plays its record's line on first entry.
        for &l in levels {
            let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
            h.quest_ok = true;
            for t in 1000..1063 {
                entry_tick(&mut e, &mut s, &mut h, l, t, 0);
            }
            assert_eq!(h.events, vec![ev], "level {l}");
        }
    }
}

#[test]
fn ambience_bed_change_and_day_swap() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    let row = EnvRow {
        day_event: 0,
        night_event: 0,
        ..amb_row()
    };
    // First bed: not a swap (current 0) → group stops, request.
    e.tick(&mut s, &mut h, &input(1, Some(row)));
    assert_eq!(
        s.take(),
        vec![
            Call::Stop52(52, 0),
            Call::Stop72(0, 0),
            req(52, None, 0, 0, 1),
            Call::StopHandle(0),
            Call::StopSongs,
        ]
    );
    assert_eq!((e.ambience.bed, e.ambience.bed_handle), (52, 1));
    // Night: day/night swap → fade old to 0 over 250, new at volume 0 fading to 255.
    let mut inp = input(1, Some(row));
    inp.day_phase = 0;
    e.tick(&mut s, &mut h, &inp);
    assert_eq!(
        s.take(),
        vec![
            Call::Fade(1, 0, 0, 250),
            req(53, None, 0, 0, 2),
            Call::SetVolume(2, 0),
            Call::Fade(2, 255, 0, 250),
            Call::StopHandle(0),
            Call::StopSongs,
        ]
    );
    assert_eq!((e.ambience.bed, e.ambience.bed_handle), (53, 2));
    // Another environment while raining: stops except (a, 64) and its event.
    let other = EnvRow {
        day_ambience: 60,
        night_ambience: 61,
        day_event: 90,
        night_event: 91,
        ..row
    };
    let mut inp = input(2, Some(other));
    inp.day_phase = 0;
    inp.weather_active = true;
    s.t = 1_000_000;
    s.rolls.extend([0, 0]);
    e.tick(&mut s, &mut h, &inp);
    let calls = s.take();
    assert_eq!(
        calls[..3],
        [
            Call::Stop52(61, 64),
            Call::Stop72(91, 0),
            req(61, None, 0, 0, 3)
        ]
    );
    // Bed 0 → current and handle 0.
    let none = EnvRow {
        day_ambience: 0,
        night_ambience: 0,
        ..EnvRow::default()
    };
    e.tick(&mut s, &mut h, &input(3, Some(none)));
    assert_eq!(s.take()[..2], [Call::Stop52(0, 0), Call::Stop72(0, 0)]);
    assert_eq!((e.ambience.bed, e.ambience.bed_handle), (0, 0));
}

#[test]
fn same_bed_day_and_night_keeps_playing() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    let cave = EnvRow {
        day_ambience: 55,
        night_ambience: 55,
        ..EnvRow::default()
    };
    e.tick(&mut s, &mut h, &input(1, Some(cave)));
    s.take();
    let mut inp = input(1, Some(cave));
    inp.day_phase = 5;
    e.tick(&mut s, &mut h, &inp);
    assert_eq!(s.take(), vec![Call::StopHandle(0), Call::StopSongs]);
    assert_eq!(e.ambience.bed_handle, 1);
}

fn rain_input(level: i32) -> TickInput {
    let mut inp = input(1, Some(EnvRow::default()));
    inp.weather_active = true;
    inp.rain_level = level;
    inp
}

fn rain_calls(s: &mut Fake) -> Vec<Call> {
    s.take()
        .into_iter()
        .filter(|c| !matches!(c, Call::StopSongs))
        .collect()
}

#[test]
fn rain_starts_one_tick_late() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    // trunc(0.5 × 255.0) = 127.
    e.tick(&mut s, &mut h, &rain_input(127));
    assert_eq!(
        rain_calls(&mut s),
        vec![req(0, None, 0, 0, 0), Call::SetVolume(0, 127)]
    );
    assert_eq!((e.ambience.rain_handle, e.ambience.rain_prev), (0, 64));
    e.tick(&mut s, &mut h, &rain_input(127));
    assert_eq!(
        rain_calls(&mut s),
        vec![
            req(64, None, 0, 0, 1),
            Call::SetVolume(1, 127),
            Call::SetVolume(1, 127)
        ]
    );
    assert_eq!(e.ambience.rain_handle, 1);
    // Volume moves toward a new target by 6.
    e.tick(&mut s, &mut h, &rain_input(140));
    assert_eq!(rain_calls(&mut s), vec![Call::SetVolume(1, 133)]);
    e.tick(&mut s, &mut h, &rain_input(140));
    assert_eq!(rain_calls(&mut s), vec![Call::SetVolume(1, 139)]);
    e.tick(&mut s, &mut h, &rain_input(140));
    assert_eq!(rain_calls(&mut s), vec![Call::SetVolume(1, 140)]);
}

#[test]
fn rain_fades_out_by_6() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    e.ambience.rain_prev = 64;
    e.tick(&mut s, &mut h, &rain_input(20));
    s.take();
    let mut vols = Vec::new();
    for _ in 0..4 {
        e.tick(&mut s, &mut h, &rain_input(0));
        vols.extend(rain_calls(&mut s));
    }
    assert_eq!(
        vols,
        vec![
            Call::SetVolume(1, 14),
            Call::SetVolume(1, 8),
            Call::SetVolume(1, 2),
            Call::StopHandle(1)
        ]
    );
    assert_eq!(e.ambience.rain_handle, 0);
    // Still raining at 0: no new request while v = 0.
    e.tick(&mut s, &mut h, &rain_input(0));
    assert!(rain_calls(&mut s).is_empty());
}

#[test]
fn rain_stops_when_weather_ends() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    e.ambience.rain_prev = 64;
    e.tick(&mut s, &mut h, &rain_input(100));
    s.take();
    let mut inp = rain_input(100);
    inp.weather_active = false;
    e.tick(&mut s, &mut h, &inp);
    assert_eq!(rain_calls(&mut s), vec![Call::StopHandle(1)]);
    assert_eq!((e.ambience.rain_handle, e.ambience.rain_prev), (0, 0));
}

fn cue_row() -> EnvRow {
    EnvRow {
        day_ambience: 0,
        night_ambience: 0,
        ..amb_row()
    }
}

fn cue_calls(s: &mut Fake) -> Vec<Call> {
    s.take()
        .into_iter()
        .filter(|c| !matches!(c, Call::StopSongs | Call::StopHandle(0)))
        .collect()
}

#[test]
fn cue_gap_on_event_change() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.t = 1000;
    // D = 250: gap = 250 + roll(167) − 83; last = T − roll(gap).
    s.rolls.extend([100, 50]);
    e.tick(&mut s, &mut h, &input(1, Some(cue_row())));
    assert_eq!(
        cue_calls(&mut s),
        vec![Call::Roll(167, 100), Call::Roll(267, 50)]
    );
    assert_eq!(e.ambience.gap, 267);
    assert_eq!(e.ambience.last_cue, 950);
    assert_eq!(e.ambience.event_id, 72);
    // Night event differs → new gap.
    let mut inp = input(1, Some(cue_row()));
    inp.day_phase = 4;
    s.rolls.extend([0, 0]);
    e.tick(&mut s, &mut h, &inp);
    assert_eq!(
        cue_calls(&mut s),
        vec![Call::Roll(167, 0), Call::Roll(167, 0)]
    );
    assert_eq!((e.ambience.event_id, e.ambience.gap), (80, 167));
    // ev = 0 → nothing, state kept.
    let quiet = EnvRow {
        day_event: 0,
        ..cue_row()
    };
    s.t = 1_000_000;
    e.tick(&mut s, &mut h, &input(1, Some(quiet)));
    assert!(cue_calls(&mut s).is_empty());
    assert_eq!(e.ambience.event_id, 80);
}

#[test]
fn cue_fires_left_or_right() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.t = 1000;
    s.rolls.extend([83, 0]); // gap 250, last 1000
    e.tick(&mut s, &mut h, &input(1, Some(cue_row())));
    s.take();
    s.t = 1249;
    e.tick(&mut s, &mut h, &input(1, Some(cue_row())));
    assert!(cue_calls(&mut s).is_empty());
    // Due, but the request fails: no draws, retried next tick.
    s.t = 1250;
    s.refuse.push(72);
    e.tick(&mut s, &mut h, &input(1, Some(cue_row())));
    assert_eq!(cue_calls(&mut s), vec![req(72, None, 0, 0, 0)]);
    s.refuse.clear();
    s.t = 1251;
    // roll(2) = 0 → left; uniform(450, 750) = 450 + 300; jitter(100) = 5 − 100.
    s.rolls.extend([0, 300, 5, 166]);
    e.tick(&mut s, &mut h, &input(1, Some(cue_row())));
    assert_eq!(
        cue_calls(&mut s),
        vec![
            req(72, None, 0, 0, 1),
            Call::Roll(2, 0),
            Call::Roll(301, 300),
            Call::Roll(201, 5),
            Call::SetPosition(1, -750, -95, 0),
            Call::Roll(167, 166),
        ]
    );
    assert_eq!((e.ambience.last_cue, e.ambience.gap), (1251, 333));
    // Right side.
    s.t = 1251 + 333;
    s.rolls.extend([1, 0, 200, 0]);
    e.tick(&mut s, &mut h, &input(1, Some(cue_row())));
    assert!(cue_calls(&mut s).contains(&Call::SetPosition(2, 450, 100, 0)));
}

#[test]
fn ambience_runs_before_music() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    let row = EnvRow {
        song: SONG_A,
        ..amb_row()
    };
    e.tick(&mut s, &mut h, &input(1, Some(row)));
    let calls = s.take();
    let bed = calls.iter().position(|c| *c == req(52, None, 0, 0, 1));
    let song = calls
        .iter()
        .position(|c| *c == req(SONG_A, None, FLAG_NO_FADE_IN, 0, 2));
    assert!(bed.is_some() && song.is_some() && bed < song, "{calls:?}");
}

#[test]
fn sample_pins_on_level_change() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    let row = cue_row();
    s.rolls.extend([0, 0]);
    assert_eq!(
        e.tick(&mut s, &mut h, &input(1, Some(row))),
        Some(SamplePins { materials: [2, 3] })
    );
    assert_eq!(e.tick(&mut s, &mut h, &input(1, Some(row))), None);
    assert_eq!(
        e.tick(&mut s, &mut h, &input(2, None)),
        Some(SamplePins { materials: [0, 0] })
    );
}

#[test]
fn state_variables_start_at_zero() {
    // The music state (cur, last level, Tl, Ts, last announced, one
    // resume value per song id of the range), the stinger state and the
    // ambience, rain and event-cue state all start at 0.
    let e = Environment::new(SongRange::LIVE);
    let m = &e.music;
    assert_eq!(
        (m.cur, m.last_level, m.tl, m.ts, m.last_announced),
        (0, 0, 0, 0, 0)
    );
    for id in 4657..=4684 {
        assert_eq!(m.resume(id), 0, "song {id}");
    }
    assert_eq!(m.stinger, Stinger::default());
    assert!(!m.stinger.active && !m.stinger.m_pending && !m.stinger.s_pending);
    let a = e.ambience;
    assert_eq!((a.bed, a.bed_handle), (0, 0));
    assert_eq!((a.rain_handle, a.rain_prev), (0, 0));
    assert_eq!((a.event_id, a.gap, a.last_cue), (0, 0, 0));
}

#[test]
fn time_tests_are_unsigned_and_exact() {
    // T − Tl is an unsigned difference across the u32 wrap.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.t = 0xFFFF_FFF0;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    assert_eq!(e.music.cur, SONG_A);
    s.t = 0xFFFF_FFF1;
    e.tick(&mut s, &mut h, &input(2, Some(song_row(SONG_B))));
    assert_eq!(e.music.tl, 0xFFFF_FFF1);
    // T − Tl = 74 (T = 0x39): still song A; 75 (T = 0x3A): switch.
    s.t = 0xFFFF_FFF1u32.wrapping_add(74);
    e.tick(&mut s, &mut h, &input(2, Some(song_row(SONG_B))));
    assert_eq!(e.music.cur, SONG_A);
    s.t = 0xFFFF_FFF1u32.wrapping_add(75);
    e.tick(&mut s, &mut h, &input(2, Some(song_row(SONG_B))));
    assert_eq!(e.music.cur, SONG_B);

    // `T > Ts + 125` is an absolute compare of the wrapped sum, so
    // just after Ts = 0xFFFFFFF0 (sum wraps to 109) it is already true.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.blocks.insert(SONG_A, CAVES_BLOCKS);
    s.t = 0xFFFF_FFF0;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    s.positions.insert(SONG_A, 1_000_000);
    s.t = 0xFFFF_FFF5;
    e.tick(&mut s, &mut h, &input(1, Some(song_row(SONG_A))));
    assert_eq!(e.music.resume(SONG_A), 1_478_063);
    assert_eq!(e.music.ts, 0xFFFF_FFF5);

    // The stinger hold `C < tH` is absolute: tH = C + H wrapping below C ends the
    // hold at once, and `C ≥ tM` is absolute too.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    let c = 0xFFFF_FF00u32;
    e.music
        .start_stinger(&mut s, c, stinger_for_event(35, 2961).unwrap());
    assert_eq!(e.music.stinger.th, c.wrapping_add(300));
    assert!(e.music.stinger.th < c);
    let mut inp = input(1, Some(song_row(0)));
    inp.c = c;
    s.t = 1;
    e.tick(&mut s, &mut h, &inp);
    assert!(!e.music.stinger.active);

    // The level-entry gap `C − P+0x7C > 62` is an unsigned difference.
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    h.quest_ok = true;
    entry_tick(&mut e, &mut s, &mut h, 1, 5, 0);
    for t in 100..=200 {
        entry_tick(&mut e, &mut s, &mut h, 8, t, 0xFFFF_FFF0);
    }
    // C = 100 → diff 116 > 62 from the first eligible tick.
    assert_eq!(h.events, vec![41]);

    // The event cue gap `T − last ≥ gap` is unsigned: last just before the wrap is
    // not "in the future".
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    s.t = 0x10;
    s.rolls.extend([0, 0]);
    e.tick(&mut s, &mut h, &input(1, Some(cue_row())));
    e.ambience.last_cue = 0xFFFF_FFF0;
    e.ambience.gap = 250;
    s.take();
    // diff = T − last = 100 + 16 = 116 < 250: no cue.
    s.t = 100;
    e.tick(&mut s, &mut h, &input(1, Some(cue_row())));
    assert!(cue_calls(&mut s).is_empty());
    // diff = 250 at T = 234: due.
    s.t = 234;
    s.rolls.extend([1, 0, 0, 0]);
    e.tick(&mut s, &mut h, &input(1, Some(cue_row())));
    assert!(cue_calls(&mut s)
        .iter()
        .any(|c| matches!(c, Call::Request { id: 72, .. })));
    // A signed play position below 0 matches no block.
    assert_eq!(resume_point(WILD_BLOCKS, 0x8000_0000), 0);
}

#[test]
fn rain_edge_cases_exact() {
    let (mut e, mut s, mut h) = (env(), Fake::default(), Hooks::default());
    // Weather active with intensity 0 from the first tick: previous = 64.
    e.tick(&mut s, &mut h, &rain_input(0));
    assert_eq!((e.ambience.rain_handle, e.ambience.rain_prev), (0, 64));
    s.take();
    // First tick with v ≠ 0 requests 64 at once (no one-tick delay).
    e.tick(&mut s, &mut h, &rain_input(100));
    assert_eq!(rain_calls(&mut s)[0], req(64, None, 0, 0, 1));
    // The request disappears (stale handle): w reads 0, min(6, v) is
    // written to the missing request, the handle is kept.
    s.reqs.clear();
    e.tick(&mut s, &mut h, &rain_input(100));
    assert_eq!(rain_calls(&mut s), vec![Call::SetVolume(1, 6)]);
    assert_eq!(e.ambience.rain_handle, 1);
    e.tick(&mut s, &mut h, &rain_input(100));
    assert_eq!(e.ambience.rain_handle, 1);
    assert!(rain_calls(&mut s)
        .iter()
        .all(|c| matches!(c, Call::SetVolume(1, 6))));
    // v = 0 clears the stale handle at once.
    e.tick(&mut s, &mut h, &rain_input(0));
    assert_eq!(rain_calls(&mut s), vec![Call::StopHandle(1)]);
    assert_eq!(e.ambience.rain_handle, 0);
}

// --- front-end music --------------------------------------------------------

use super::frontend::*;

#[derive(Default)]
struct Dev {
    rands: VecDeque<u32>,
    playing: bool,
    /// Paths whose start fails.
    bad: Vec<&'static str>,
    started: Vec<(String, i32)>,
    fades: Vec<u32>,
    stops: u32,
    gain: Option<i32>,
}

impl JukeboxHost for Dev {
    fn rand(&mut self) -> u32 {
        self.rands.pop_front().expect("scripted rand")
    }
    fn voice_playing(&self) -> bool {
        self.playing
    }
    fn start_stream(&mut self, path: &str, volume: i32) -> bool {
        if self.bad.contains(&path) {
            return false;
        }
        self.started.push((path.to_string(), volume));
        self.playing = true;
        true
    }
    fn fade_out(&mut self, ms: u32) {
        self.fades.push(ms);
    }
    fn stop_voice(&mut self) {
        self.stops += 1;
        self.playing = false;
    }
    fn set_global_gain(&mut self, g: i32) {
        self.gain = Some(g);
    }
}

#[test]
fn front_end_playlists_and_latch() {
    assert_eq!(LIST_A[0], "common\\options.wav");
    assert_eq!(LIST_A[7], "act3\\kurastsewer.wav");
    assert_eq!(LIST_B[0], "introedit.wav");
    assert_eq!(LIST_B[7], "act4\\diablo.wav");
    assert_eq!(LIST_A.len(), 8);
    assert_eq!(LIST_B.len(), 8);
    // The list is latched on first use.
    let mut j = Jukebox::default();
    assert_eq!(j.playlist(true)[0], "introedit.wav");
    assert_eq!(j.playlist(false)[0], "introedit.wav");
    let mut j = Jukebox::default();
    assert_eq!(j.playlist(false)[0], "common\\options.wav");
    assert_eq!(j.playlist(true)[0], "common\\options.wav");
}

#[test]
fn front_end_start_clears_flags_once() {
    let mut j = Jukebox::default();
    j.played[3] = true;
    j.start();
    assert!(j.wanted && j.first_track);
    assert_eq!(j.played, [false; 8]);
    // Already wanted: nothing is cleared.
    j.played[2] = true;
    j.first_track = false;
    j.start();
    assert!(j.played[2] && !j.first_track && j.wanted);
}

#[test]
fn front_end_pick_order() {
    let (mut j, mut d) = (Jukebox::default(), Dev::default());
    j.start();
    // Entry 0 first, volume 110.
    assert_eq!(j.service(&mut d, false), Some("common\\options.wav"));
    assert_eq!(d.started, vec![("common\\options.wav".to_string(), 110)]);
    // The voice is playing: no pick.
    assert_eq!(j.service(&mut d, false), None);
    d.playing = false;
    // rand 8 → i = 0 (played) → forward wrap to 1.
    d.rands.push_back(8);
    assert_eq!(j.service(&mut d, false), Some("act1\\caves.wav"));
    d.playing = false;
    // i = 7 unplayed.
    d.rands.push_back(7);
    assert_eq!(j.service(&mut d, false), Some("act3\\kurastsewer.wav"));
    d.playing = false;
    // i = 7 played → wraps past 0, 1 to 2.
    d.rands.push_back(7);
    assert_eq!(j.service(&mut d, false), Some("act1\\monastery.wav"));
    // Use up the list: all flags clear once, then play on.
    for _ in 0..4 {
        d.playing = false;
        d.rands.push_back(0);
        assert!(j.service(&mut d, false).is_some());
    }
    assert!(j.played.iter().all(|&p| p));
    d.playing = false;
    d.rands.push_back(5);
    assert_eq!(j.service(&mut d, false), Some("act2\\tombs.wav"));
    assert_eq!(j.played.iter().filter(|&&p| p).count(), 1);
    // Not wanted: no pick even with an idle voice.
    j.wanted = false;
    d.playing = false;
    assert_eq!(j.service(&mut d, false), None);
}

#[test]
fn front_end_failed_start_picks_again_and_exhaustion_stops() {
    let (mut j, mut d) = (Jukebox::default(), Dev::default());
    j.start();
    d.bad = vec!["common\\options.wav", "act1\\caves.wav"];
    // Entry 0 fails (marked played); rand 0 → forward to 1, which fails;
    // rand 0 → forward to 2.
    d.rands.extend([0, 0]);
    // (Entry 0 is the first-track pick, then two random ones.)
    assert_eq!(j.pick(&mut d, false), Some("act1\\monastery.wav"));
    assert!(j.played[0] && j.played[2]);
    // Every entry unplayed-or-failing: all start attempts fail → the list
    // is used up, cleared once, used up again → wanted := 0.
    let (mut j, mut d) = (Jukebox::default(), Dev::default());
    j.start();
    d.bad = LIST_A.to_vec();
    d.rands.extend([0; 32]);
    assert_eq!(j.pick(&mut d, false), None);
    assert!(!j.wanted);
}

#[test]
fn front_end_toggle() {
    let (mut j, mut d) = (Jukebox::default(), Dev::default());
    // Off → start, Options Music := 1.
    assert!(j.toggle(&mut d, false));
    assert!(j.wanted && j.first_track);
    j.service(&mut d, false);
    assert!(d.playing);
    // On → stop: wanted := 0, a playing voice fades over 200 ms.
    assert!(!j.toggle(&mut d, true));
    assert!(!j.wanted);
    assert_eq!(d.fades, vec![200]);
}

#[test]
fn front_end_leave_fades_stops_and_resets_gain() {
    let (mut j, mut d) = (Jukebox::default(), Dev::default());
    j.start();
    j.leave_front_end(&mut d);
    assert!(d.fades.is_empty() && d.gain.is_none(), "voice not playing");
    j.service(&mut d, false);
    j.leave_front_end(&mut d);
    assert_eq!(d.fades, vec![180]);
    assert_eq!((d.stops, d.gain), (1, Some(255)));
    assert!(!j.wanted);
}

#[test]
fn stingers_rain_and_event_cues_keep_real_time_at_25_and_60_hz() {
    for hz in [25, 60] {
        let mut s = Fake {
            rate: d2_sim::tick::TickRate::new(hz).unwrap(),
            ..Fake::default()
        };
        let mut music = Music::new(SongRange::LIVE);
        music.start_stinger(
            &mut s,
            7,
            StingerArgs {
                m: 4685,
                dm: 25,
                h: 50,
                k: ResumeBlock::Start,
                s: 2934,
                ds: 75,
                play: true,
            },
        );
        assert_eq!(music.stinger.tm, 7 + hz);
        assert_eq!(music.stinger.th, 7 + 2 * hz);
        assert_eq!(music.stinger.ts, 7 + 3 * hz);
        music.rearm_stinger(11, 25, true);
        assert_eq!(music.stinger.ts, 11 + hz);
        assert_eq!(music.stinger.th, 11 + 3 * hz);
        let mut ambience = Ambience {
            rain_prev: SCENE_RAIN,
            ..Ambience::default()
        };
        // A new rain request starts at its target volume; exercise the ramp
        // with an existing request below the new target instead.
        ambience.rain_handle = s.request(SCENE_RAIN, None, 0, 0, 0);
        s.set_volume(ambience.rain_handle, 0);
        s.rolls = [8, 0].into();
        let mut inp = input(
            1,
            Some(EnvRow {
                day_event: 10,
                event_delay: 25,
                ..EnvRow::default()
            }),
        );
        inp.weather_active = true;
        inp.rain_level = 255;
        for tick in 0..hz {
            s.t = tick;
            ambience.rain(&mut s, &inp);
            ambience.cues(&mut s, &inp);
        }
        assert_eq!(s.volume(ambience.rain_handle), Some(RAIN_STEP * 25));
        assert!(!s
            .calls
            .iter()
            .any(|c| matches!(c, Call::Request { id: 10, .. })));
        s.t = hz;
        ambience.cues(&mut s, &inp);
        assert!(s
            .calls
            .iter()
            .any(|c| matches!(c, Call::Request { id: 10, .. })));
    }
}
