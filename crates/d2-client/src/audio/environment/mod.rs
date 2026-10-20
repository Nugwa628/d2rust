//! Sound environment state machines: music and
//! quest stingers, level-entry lines, ambience bed, rain and
//! event cues, sample pins.
//!
//! Plain Rust, no float, no I/O: [`Environment::tick`] runs once per sound
//! tick with every input passed explicitly ([`TickInput`]) and reaches the
//! sound layer only through [`EnvCalls`] (= [`SoundCalls`] plus the few
//! request queries these machines need). Things outside the sound layer
//! (the client quest check, the player event line of the stinger rules)
//! go through [`EnvHooks`].
//!
//! Inputs owned by other parts of the client are plain parameters: the
//! day phase is the period index of the client act's environment record;
//! the weather flag and intensity are the weather state of the draw
//! order's weather rules.

use std::collections::BTreeSet;

use super::calls::{jitter, uniform, Handle, SoundCalls, FLAG_NO_FADE_IN};
use crate::bridge::world::UnitKey;

pub mod frontend;
pub mod frontend_stream;

#[cfg(test)]
mod tests;

/// The request queries the environment machines need beyond
/// [`SoundCalls`]; the sound table owns what they read.
pub trait EnvCalls: SoundCalls {
    /// A request with this id exists.
    fn id_requested(&self, id: i32) -> bool;
    /// A request with an id in 4,657–4,698 is still active
    /// (not ended; a stop flag alone does not end it).
    fn music_active(&self) -> bool;
    /// The play position of the first
    /// request of the active list whose current id is `id`, if it is
    /// playing, else `None`. Units: 4-byte units of
    /// the stream, one frame of a 16-bit stereo
    /// song.
    fn play_position(&self, id: i32) -> Option<u32>;
    /// `Block 1`, `Block 2`, `Block 3` of sound row `id`
    /// (an empty cell reads −1).
    fn blocks(&self, id: i32) -> [i32; 3];
    /// Volume (+0x1C) of the request with this handle, `None` when no
    /// such request exists.
    fn volume(&self, h: Handle) -> Option<i32>;
    /// The request's position := (x, y, z + 640.0) and its distance² from
    /// x, y. The game stores f32; every value passed here is integral.
    fn set_position(&mut self, h: Handle, x: i32, y: i32, z: i32);
}

/// Calls into client code outside the sound layer.
pub trait EnvHooks {
    /// 1 when quest
    /// `q` is open and not done for this player: byte q of the last S→C
    /// 0x5E ≠ 0 (indexed directly, so q 12 / 13 read the bytes of chains 11
    /// / 12), the first quest-log entry whose chain is q found, its slot
    /// clear in the game record (bit 13) and the player record (bits 1, 0,
    /// 14), and for q = 1 the Den of Evil shown status < 5. Only called
    /// with q ≠ 0.
    fn quest_check(&self, q: u8) -> bool;
    /// Player event `e` on the local player P (the class quest line base
    /// + e − 33, delay per the stinger rules).
    fn player_event(&mut self, s: &mut dyn SoundCalls, e: u8);
}

/// The `soundenviron.txt` values these machines read (the EAX columns are
/// not reproduced).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnvRow {
    /// `Song` (+0x00).
    pub song: i32,
    /// `Day Ambience` (+0x04).
    pub day_ambience: i32,
    /// `Night Ambience` (+0x08).
    pub night_ambience: i32,
    /// `Day Event` (+0x0C).
    pub day_event: i32,
    /// `Night Event` (+0x10).
    pub night_event: i32,
    /// `Event Delay` (+0x14).
    pub event_delay: i32,
    /// `Indoors` (+0x18, u8).
    pub indoors: u8,
    /// `Material 1` (+0x1C).
    pub material1: i32,
    /// `Material 2` (+0x20).
    pub material2: i32,
}

impl From<&d2_data::sounds::SoundEnvironRow> for EnvRow {
    fn from(r: &d2_data::sounds::SoundEnvironRow) -> Self {
        EnvRow {
            song: r.song,
            day_ambience: r.day_ambience,
            night_ambience: r.night_ambience,
            day_event: r.day_event,
            night_event: r.night_event,
            event_delay: r.event_delay,
            indoors: r.indoors,
            material1: r.material_1,
            material2: r.material_2,
        }
    }
}

/// E for a level: row `SoundEnv` of `rows`; an index outside the table is
/// no row.
pub fn env_row(rows: &[EnvRow], sound_env: u8) -> Option<EnvRow> {
    rows.get(usize::from(sound_env)).copied()
}

/// Day: phase ∈ {1, 2, 3}; any other phase is night.
pub fn is_day(phase: u32) -> bool {
    (1..=3).contains(&phase)
}

/// The song range: minimum and maximum `Song` over
/// the `soundenviron` rows with `Song > 0`. Live: 4,657–4,684.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SongRange {
    pub first: i32,
    pub last: i32,
}

impl SongRange {
    /// The live song range.
    pub const LIVE: SongRange = SongRange {
        first: 4657,
        last: 4684,
    };

    /// `None` when no row has `Song > 0`.
    pub fn from_rows(rows: &[EnvRow]) -> Option<SongRange> {
        let songs = rows.iter().map(|r| r.song).filter(|&s| s > 0);
        let first = songs.clone().min()?;
        let last = songs.max()?;
        Some(SongRange { first, last })
    }

    pub fn contains(&self, id: i32) -> bool {
        (self.first..=self.last).contains(&id)
    }

    fn len(&self) -> usize {
        usize::try_from(self.last - self.first + 1).unwrap_or(0)
    }
}

/// The local player as these machines see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerState {
    pub key: UnitKey,
    /// The player's dead check is false.
    pub alive: bool,
    /// P+0x7C: last voice, in C.
    pub last_voice: u32,
}

/// Everything one sound tick reads besides the sound layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickInput {
    /// L: the local player's level id; 0 = no player or no room.
    pub level: u32,
    /// E: [`env_row`] of `levels[L].SoundEnv`.
    pub env: Option<EnvRow>,
    /// The period index (+0x00) of the client act's environment record
    /// (the day phase): set by the client update's
    /// advance and S→C 0x53, index 2
    /// before the first 0x53. Day is 1–3.
    pub day_phase: u32,
    /// Weather active (the draw order's weather state).
    pub weather_active: bool,
    /// trunc(intensity × 255.0) of the weather intensity (f32 = target
    /// particles × 1/256), computed by the weather owner so no float
    /// reaches this module.
    pub rain_level: i32,
    /// `Master Volume` 0–100.
    pub master_volume: i32,
    /// `Music Volume` 0–100.
    pub music_volume: i32,
    /// C: client update counter.
    pub c: u32,
    /// P; `None` when there is no local player.
    pub player: Option<PlayerState>,
    /// Number of levels (`levels.txt` rows).
    pub level_count: u32,
}

impl TickInput {
    fn audible(&self) -> bool {
        self.master_volume != 0 && self.music_volume != 0
    }

    fn day(&self) -> bool {
        is_day(self.day_phase)
    }
}

/// What the sample pins ask of the sample cache on a level change: lock the walk and
/// run footstep groups of these materials (`Material 1`, `Material 2`)
/// after clearing the locks of groups 2,720–2,876. Cache only:
/// no observable effect in D2RUST.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SamplePins {
    pub materials: [i32; 2],
}

/// Which `Block` a stinger stores as the interrupted song's resume point
/// (column k).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeBlock {
    /// k = 0: resume at 0.
    Start,
    /// k = 1..3: `Block k` of the song's row.
    Block(BlockIndex),
}

/// `Block 1`..`Block 3`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockIndex {
    B1,
    B2,
    B3,
}

/// The arguments of a stinger start (M, dM, H, k, S, dS, play).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StingerArgs {
    /// Quest track (0: none).
    pub m: i32,
    /// Delay of M, in C.
    pub dm: u32,
    /// Music hold, in C.
    pub h: u32,
    pub k: ResumeBlock,
    /// Hero quest line.
    pub s: i32,
    /// Delay of S, in C.
    pub ds: u32,
    /// Whether S is played.
    pub play: bool,
}

/// One row of the stinger caller table (player events).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StingerRow {
    pub event: u8,
    pub m: i32,
    pub dm: u32,
    pub h: u32,
    pub k: ResumeBlock,
    pub ds: u32,
    pub play: bool,
}

const fn row(event: u8, m: i32, dm: u32, h: u32, k1: bool, ds: u32, play: bool) -> StingerRow {
    StingerRow {
        event,
        m,
        dm,
        h,
        k: if k1 {
            ResumeBlock::Block(BlockIndex::B1)
        } else {
            ResumeBlock::Start
        },
        ds,
        play,
    }
}

/// The player events that start a stinger.
pub const STINGER_ROWS: [StingerRow; 11] = [
    row(33, 4685, 0, 475, false, 425, true),
    row(34, 4686, 475, 800, true, 525, true),
    row(35, 4688, 0, 300, true, 250, true),
    row(37, 0, 0, 1500, false, 0, false),
    row(50, 4693, 475, 1100, true, 525, true),
    row(52, 4694, 0, 425, true, 375, true),
    row(66, 4692, 0, 425, false, 375, true),
    row(75, 4695, 0, 475, false, 325, true),
    row(80, 4698, 0, 500, true, 475, true),
    row(82, 4697, 0, 550, true, 400, true),
    row(83, 4696, 0, 525, true, 425, true),
];

/// 4,687 `music_quest_compelling`, no speech.
pub const STINGER_COMPELLING: StingerArgs = StingerArgs {
    m: 4687,
    dm: 0,
    h: 350,
    k: ResumeBlock::Start,
    s: 0,
    ds: 0,
    play: false,
};

/// 4,691 `music_quest_izual`, no speech.
pub const STINGER_IZUAL: StingerArgs = StingerArgs {
    m: 4691,
    dm: 0,
    h: 250,
    k: ResumeBlock::Start,
    s: 0,
    ds: 0,
    play: false,
};

/// The stinger of player event `e`, with S = the class quest line
/// base + e − 33; `None` for an event without one.
pub fn stinger_for_event(e: u8, quest_base: i32) -> Option<StingerArgs> {
    STINGER_ROWS
        .iter()
        .find(|r| r.event == e)
        .map(|r| StingerArgs {
            m: r.m,
            dm: r.dm,
            h: r.h,
            k: r.k,
            s: quest_base + i32::from(e) - 33,
            ds: r.ds,
            play: r.play,
        })
}

/// One record of the level-entry table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryRecord {
    pub levels: &'static [u32],
    /// Quest q (0: no quest check).
    pub quest: u8,
    /// Player event e (the line).
    pub event: u8,
}

/// The 14 level-entry records, in table order.
pub const ENTRY_RECORDS: [EntryRecord; 14] = [
    EntryRecord {
        levels: &[17],
        quest: 2,
        event: 38,
    },
    EntryRecord {
        levels: &[34, 35, 36, 37],
        quest: 6,
        event: 40,
    },
    EntryRecord {
        levels: &[8],
        quest: 1,
        event: 41,
    },
    EntryRecord {
        levels: &[29, 30, 31],
        quest: 6,
        event: 42,
    },
    EntryRecord {
        levels: &[26, 27],
        quest: 3,
        event: 43,
    },
    EntryRecord {
        levels: &[20, 21, 22, 23, 24, 25],
        quest: 5,
        event: 44,
    },
    EntryRecord {
        levels: &[38],
        quest: 6,
        event: 46,
    },
    EntryRecord {
        levels: &[2, 3, 4, 5, 6, 7],
        quest: 1,
        event: 47,
    },
    EntryRecord {
        levels: &[74],
        quest: 12,
        event: 55,
    },
    EntryRecord {
        levels: &[58],
        quest: 13,
        event: 56,
    },
    EntryRecord {
        levels: &[110],
        quest: 31,
        event: 76,
    },
    EntryRecord {
        levels: &[120],
        quest: 35,
        event: 78,
    },
    EntryRecord {
        levels: &[121],
        quest: 34,
        event: 77,
    },
    EntryRecord {
        levels: &[132],
        quest: 36,
        event: 79,
    },
];

// ---------------------------------------------------------------------------
// Level-entry lines
// ---------------------------------------------------------------------------

/// The level-entry line state: the last level checked and the per-level
/// flags. Sound init at every game start clears the flags
/// ([`EntryLines::game_start`]); the last level checked is never reset,
/// so it carries the previous game's last level into the next game
/// (reproduced).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryLines {
    last_checked: u32,
    flagged: BTreeSet<u32>,
}

impl EntryLines {
    pub fn is_flagged(&self, level: u32) -> bool {
        self.flagged.contains(&level)
    }

    /// The last level checked.
    pub fn last_checked(&self) -> u32 {
        self.last_checked
    }

    /// From sound init: the level flags := 0; the
    /// last level checked keeps its value.
    pub fn game_start(&mut self) {
        self.flagged.clear();
    }

    /// The level-entry check of a newly announced level.
    fn check(
        &mut self,
        s: &mut dyn EnvCalls,
        hooks: &mut dyn EnvHooks,
        inp: &TickInput,
        level: u32,
    ) {
        if level >= inp.level_count || level == self.last_checked {
            return;
        }
        self.last_checked = level;
        if self.flagged.contains(&level) {
            return;
        }
        let Some(rec) = ENTRY_RECORDS.iter().find(|r| r.levels.contains(&level)) else {
            return;
        };
        self.flagged.extend(rec.levels.iter().copied());
        let Some(p) = inp.player else {
            return;
        };
        let quest_ok = rec.quest == 0 || hooks.quest_check(rec.quest);
        // C − P+0x7C, unsigned like every C difference here.
        if quest_ok
            && inp.c.wrapping_sub(p.last_voice) > s.tick_rate().ticks_for_ms(62 * 40)
            && !s.any_speech()
        {
            hooks.player_event(s, rec.event);
        }
    }
}

// ---------------------------------------------------------------------------
// Music and stingers
// ---------------------------------------------------------------------------

/// The stinger state; times in C.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stinger {
    pub active: bool,
    pub m: i32,
    pub tm: u32,
    pub m_pending: bool,
    pub s: i32,
    pub ts: u32,
    pub s_pending: bool,
    pub th: u32,
}

/// The music state: `cur`, last level, Tl, Ts, last announced, the
/// resume table, and the stinger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Music {
    rate: d2_sim::tick::TickRate,
    range: SongRange,
    pub cur: i32,
    pub last_level: u32,
    pub tl: u32,
    pub ts: u32,
    pub last_announced: u32,
    /// One value per song id of the range (`Block` values, may be −1).
    resume: Vec<i32>,
    pub stinger: Stinger,
}

impl Music {
    pub fn new(range: SongRange) -> Self {
        Music {
            rate: d2_sim::tick::TickRate::BASE,
            range,
            cur: 0,
            last_level: 0,
            tl: 0,
            ts: 0,
            last_announced: 0,
            resume: vec![0; range.len()],
            stinger: Stinger::default(),
        }
    }

    /// Every music and stinger variable and the resume
    /// table := 0. Run by [`Environment::sound_init`], which applies the
    /// `-ns` condition.
    pub fn reset(&mut self) {
        *self = Music::new(self.range);
    }

    /// resume[id]; 0 outside the song range.
    pub fn resume(&self, id: i32) -> i32 {
        self.slot(id).map_or(0, |i| self.resume[i])
    }

    pub fn set_resume(&mut self, id: i32, v: i32) {
        if let Some(i) = self.slot(id) {
            self.resume[i] = v;
        }
    }

    fn slot(&self, id: i32) -> Option<usize> {
        if self.range.contains(id) {
            usize::try_from(id - self.range.first).ok()
        } else {
            None
        }
    }

    /// Starts a stinger (M, dM, H, k, S, dS, play). T is
    /// `s.sound_tick`, `c` is C.
    pub fn start_stinger(&mut self, s: &mut dyn EnvCalls, c: u32, a: StingerArgs) {
        self.rate = s.tick_rate();
        if self.cur != 0 && s.play_position(self.cur).is_some() {
            let v = match a.k {
                ResumeBlock::Start => 0,
                ResumeBlock::Block(b) => s.blocks(self.cur)[b as usize],
            };
            self.set_resume(self.cur, v);
            self.ts = s.sound_tick();
        }
        s.stop_songs();
        self.stinger = Stinger {
            active: true,
            m: a.m,
            tm: c.wrapping_add(self.rate.ticks_for_ms(a.dm.saturating_mul(40))),
            m_pending: true,
            s: a.s,
            ts: c.wrapping_add(self.rate.ticks_for_ms(a.ds.saturating_mul(40))),
            s_pending: a.play,
            th: c.wrapping_add(self.rate.ticks_for_ms(a.h.saturating_mul(40))),
        };
    }

    /// Re-arms the stinger by `dt` (0x2C event 92 with (25, 1)).
    pub fn rearm_stinger(&mut self, c: u32, dt: u32, play: bool) {
        let dt = self.rate.ticks_for_ms(dt.saturating_mul(40));
        let hold = self.rate.ticks_for_ms(50 * 40);
        let st = &mut self.stinger;
        if !st.active {
            return;
        }
        if play {
            st.ts = c.wrapping_add(dt);
            st.s_pending = true;
            st.th = c.wrapping_add(dt).wrapping_add(hold);
        } else {
            st.th = st.ts.wrapping_add(hold);
            st.s_pending = false;
        }
    }

    /// The stinger tick; true while the stinger holds the rest of the music tick.
    fn stinger_tick(&mut self, s: &mut dyn EnvCalls, inp: &TickInput, audible: bool) -> bool {
        let st = &mut self.stinger;
        if st.m_pending && inp.c >= st.tm {
            s.request(st.m, None, 0, 0, 0);
            st.m_pending = false;
        }
        if st.s_pending && inp.c >= st.ts {
            if let Some(p) = inp.player.filter(|p| p.alive) {
                s.request(st.s, Some(p.key), 0, 0, 0);
            }
            st.s_pending = false;
        }
        if inp.c < st.th {
            return true;
        }
        st.active = false;
        if audible {
            s.stop_id(st.m);
        }
        false
    }

    /// The music tick, for a tick with L ≠ 0.
    fn tick(
        &mut self,
        s: &mut dyn EnvCalls,
        hooks: &mut dyn EnvHooks,
        inp: &TickInput,
        entry: &mut EntryLines,
    ) {
        self.rate = s.tick_rate();
        let t = s.sound_tick();
        let l = inp.level;
        // The level's song, if it is in the song range.
        let song = inp
            .env
            .map(|e| e.song)
            .filter(|&id| self.range.contains(id))
            .unwrap_or(0);
        let audible = inp.audible();
        // A level change restarts Tl.
        if l != self.last_level {
            self.tl = t;
            self.last_level = l;
        }
        // Switch song after 75 ticks on the level, when not audible, or from none.
        if self.cur != song
            && (t.wrapping_sub(self.tl) >= self.rate.ticks_for_ms(75 * 40)
                || !audible
                || self.cur == 0)
        {
            self.cur = song;
            self.ts = t;
        }
        // Silence the songs when not audible or there is no song.
        if !audible || self.cur == 0 {
            s.stop_songs();
        }
        // A running stinger holds the rest.
        if self.stinger.active && self.stinger_tick(s, inp, audible) {
            return;
        }
        // Request the song from its resume point when it is not requested.
        if self.cur != 0 && audible && !s.id_requested(self.cur) {
            let o = self.resume(self.cur);
            let flags = if !s.music_active() && o == 0 {
                FLAG_NO_FADE_IN
            } else {
                0
            };
            s.stop_songs();
            // The stored value is passed through as the offset (a −1
            // `Block` cell from the stinger start included).
            s.request(self.cur, None, 0, flags, o as u32);
        }
        // The level-entry line, 62 ticks after the level change.
        if l != self.last_announced && t.wrapping_sub(self.tl) >= self.rate.ticks_for_ms(62 * 40) {
            entry.check(s, hooks, inp, l);
            self.last_announced = l;
        }
        // Every 125 ticks, store the song's next block as its resume point.
        if self.cur != 0 && t > self.ts.wrapping_add(self.rate.ticks_for_ms(125 * 40)) {
            if let Some(p) = s.play_position(self.cur) {
                let r = resume_point(s.blocks(self.cur), p);
                self.set_resume(self.cur, r);
                self.ts = t;
            }
        }
    }
}

/// The next block boundary after play position `p`, or 0; −1 cells
/// never match.
pub fn resume_point(blocks: [i32; 3], p: u32) -> i32 {
    let p = i64::from(p);
    let [b1, b2, b3] = blocks.map(i64::from);
    if 0 <= p && p < b1 {
        blocks[0]
    } else if b1 <= p && p < b2 {
        blocks[1]
    } else if b2 <= p && p < b3 {
        blocks[2]
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Ambience, rain, event cues, sample pins
// ---------------------------------------------------------------------------

/// `scene_rain`.
pub const SCENE_RAIN: i32 = 64;
/// Bed cross-fade length, in T.
pub const BED_FADE: u32 = 250;
/// Rain volume step per tick.
pub const RAIN_STEP: i32 = 6;
/// The z argument of the cue's position call (`position(h, x, y, 0)`);
/// the position call adds 640.0, so the cue sits at z = 640.0.
pub const CUE_Z: i32 = 0;

/// The ambience state: bed and its handle, rain handle and previous id,
/// event id, gap, last cue, pinned level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ambience {
    pub bed: i32,
    pub bed_handle: Handle,
    pub rain_handle: Handle,
    pub rain_prev: i32,
    pub event_id: i32,
    pub gap: i32,
    pub last_cue: u32,
    pub pinned_level: u32,
}

impl Ambience {
    /// Bed, rain, cues for a tick with L ≠ 0. Returns the sample pins on a level change.
    fn tick(&mut self, s: &mut dyn EnvCalls, inp: &TickInput) -> Option<SamplePins> {
        self.bed(s, inp);
        self.rain(s, inp);
        self.cues(s, inp);
        // EAX room settings, mixer mode 2 only: not reproduced.
        self.pins(inp)
    }

    /// The ambience bed.
    fn bed(&mut self, s: &mut dyn EnvCalls, inp: &TickInput) {
        let day = inp.day();
        let a = inp.env.map_or(0, |e| {
            if day {
                e.day_ambience
            } else {
                e.night_ambience
            }
        });
        if a == self.bed {
            return;
        }
        let swap = inp.env.is_some_and(|e| {
            let (d, n) = (e.day_ambience, e.night_ambience);
            (a, self.bed) == (d, n) || (a, self.bed) == (n, d)
        });
        if swap {
            s.fade(self.bed_handle, 0, 0, BED_FADE);
        } else {
            // "when raining": the weather flag of this tick.
            let rain = if inp.weather_active { SCENE_RAIN } else { 0 };
            s.stop_52_71_except(a, rain);
            // The second exception of the event-range stop is always 0.
            s.stop_72_201_except(event_of(inp), 0);
        }
        if a == 0 {
            self.bed = 0;
            self.bed_handle = 0;
        } else {
            self.bed_handle = s.request(a, None, 0, 0, 0);
            if swap {
                s.set_volume(self.bed_handle, 0);
                s.fade(self.bed_handle, 255, 0, BED_FADE);
            }
            self.bed = a;
        }
    }

    /// The rain sound.
    fn rain(&mut self, s: &mut dyn EnvCalls, inp: &TickInput) {
        let step = s.tick_rate().share(RAIN_STEP, s.sound_tick() as i32);
        let r = if inp.weather_active { SCENE_RAIN } else { 0 };
        let v = if inp.weather_active {
            inp.rain_level
        } else {
            0
        };
        if r == 0 {
            s.stop_handle(self.rain_handle);
            self.rain_handle = 0;
            self.rain_prev = 0;
            return;
        }
        if v != 0 && self.rain_handle == 0 {
            // The previous tick's id: the first raining tick requests 0.
            self.rain_handle = s.request(self.rain_prev, None, 0, 0, 0);
            s.set_volume(self.rain_handle, v);
        }
        if self.rain_handle != 0 {
            // The volume read gives 0 for a handle whose request is
            // gone; with v > 0 the step then writes min(6, v) to the
            // missing request (nothing happens) and the stale handle is
            // kept until v reaches 0 or the weather ends.
            let w = s.volume(self.rain_handle).unwrap_or(0);
            let w = if w < v {
                (w + step).min(v)
            } else if w > v {
                (w - step).max(v)
            } else {
                w
            };
            if w == 0 {
                s.stop_handle(self.rain_handle);
                self.rain_handle = 0;
            } else {
                s.set_volume(self.rain_handle, w);
            }
        }
        self.rain_prev = r;
    }

    /// The event cues.
    fn cues(&mut self, s: &mut dyn EnvCalls, inp: &TickInput) {
        let ev = event_of(inp);
        if ev == 0 {
            return;
        }
        let d = inp.env.map_or(0, |e| e.event_delay);
        let t = s.sound_tick();
        if ev != self.event_id {
            self.gap = d + jitter(s, d / 3);
            self.last_cue = t.wrapping_sub(
                s.tick_rate()
                    .ticks_for_ms(s.roll(self.gap).saturating_mul(40)),
            );
            self.event_id = ev;
        }
        if t.wrapping_sub(self.last_cue)
            >= s.tick_rate()
                .ticks_for_ms((self.gap as u32).saturating_mul(40))
        {
            let h = s.request(ev, None, 0, 0, 0);
            if h != 0 {
                let sign = if s.roll(2) != 0 { 1 } else { -1 };
                let x = sign * uniform(s, 450, 750);
                let y = jitter(s, 100);
                s.set_position(h, x, y, CUE_Z);
                self.last_cue = t;
                self.gap = d + jitter(s, d / 3);
            }
        }
    }

    /// The sample pins: on a level change, the footstep groups to pin.
    fn pins(&mut self, inp: &TickInput) -> Option<SamplePins> {
        if inp.level == self.pinned_level {
            return None;
        }
        self.pinned_level = inp.level;
        let e = inp.env.unwrap_or_default();
        Some(SamplePins {
            materials: [e.material1, e.material2],
        })
    }
}

/// E's `Day Event` by day, else `Night Event`; 0 without E.
fn event_of(inp: &TickInput) -> i32 {
    let day = inp.day();
    inp.env
        .map_or(0, |e| if day { e.day_event } else { e.night_event })
}

// ---------------------------------------------------------------------------
// The whole machine
// ---------------------------------------------------------------------------

/// The environment machines of one client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    pub ambience: Ambience,
    pub music: Music,
    pub entry: EntryLines,
}

impl Environment {
    pub fn new(range: SongRange) -> Self {
        Environment {
            ambience: Ambience::default(),
            music: Music::new(range),
            entry: EntryLines::default(),
        }
    }

    /// Sound init at a game start: the music reset runs unless the
    /// start-up configuration's `-ns` switch is set; the level-entry flags
    /// are cleared.
    pub fn sound_init(&mut self, no_sound_switch: bool) {
        if !no_sound_switch {
            self.music.reset();
        }
        self.entry.game_start();
    }

    /// One sound tick (before the request update): ambience, then music.
    /// L = 0 → nothing. Returns the sample pins when the level changed.
    pub fn tick(
        &mut self,
        s: &mut dyn EnvCalls,
        hooks: &mut dyn EnvHooks,
        inp: &TickInput,
    ) -> Option<SamplePins> {
        if inp.level == 0 {
            return None;
        }
        let pins = self.ambience.tick(s, inp);
        self.music.tick(s, hooks, inp, &mut self.entry);
        pins
    }
}
