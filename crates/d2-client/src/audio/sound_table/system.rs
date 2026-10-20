//! The sound requests and the 16 channels, driven one sound tick at a
//! time. Every channel start, stop and volume/pan send becomes a cue on
//! the audio core's [`TriggerQueue`] stamped with the sound tick, so the
//! voice log records them; the core's voice policy must then admit every
//! start ([`crate::audio::Unlimited`]), since channel limits and stealing
//! are decided here.
//!
//! A request's volume runs through the player's settings and the ducks
//! (`volume::chain`), then a mod game's per-category gains when it has any
//! ([`SoundSystem::set_mods`], `crate::mods::AudioGains`), then distance
//! falloff and the sound's own record volume.
//!
//! Unit positions, the listener, occlusion and the client RNG seed come
//! through the narrow [`SoundWorld`] trait, never from Bevy.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use crate::audio::calls::{Handle, SoundCalls, FLAG_EXACT, FLAG_NO_FADE_IN};
use crate::audio::{
    sound_bytes, Cue, CueId, DeviceChange, ParamChange, Sound, SoundBank, SoundId, Stop,
    StopTarget, Trigger, TriggerQueue, TriggerSource, VoiceParams,
};
use crate::bridge::world::UnitKey;
use crate::mods::{ClientMods, SoundCategory};
use d2_sim::rng::Seed;

use super::table::{is_music_path, is_speech, LoadState, SoundTableData, SoundTableError};
use super::volume::{
    chain, falloff, ftol, linear_falloff, mode0_gain_pan, record_volume, ChainInputs, SoundSettings,
};

/// Request pool size.
pub const REQUEST_SLOTS: usize = 200;
/// Channel slots.
pub const CHANNELS: usize = 16;
/// Solo duck floor.
pub const SOLO_DUCK_MIN: i32 = 70;
/// Fade-in length after a resume.
pub const RESUME_FADE: u32 = 3;
/// Async loads pending at most during a preload.
pub const MAX_PENDING_LOADS: u32 = 15;
/// Preload period in sound ticks.
pub const PRELOAD_PERIOD: u32 = 25;
/// Highest id the preload pass walks.
pub const PRELOAD_LAST_ID: i32 = 2_933;
/// Ticks after a failed eviction during which the preload starts nothing.
pub const EVICTION_HOLD: u32 = 250;
/// Ticks a recently used sample is kept by an eviction with `recent`.
pub const RECENT_USE: u32 = 750;
/// Cache limit on any machine above 500 MB.
pub const CACHE_LIMIT: u32 = 5 * 1024 * 1024;

/// Cache limit for a machine with `physical` bytes of memory: physical
/// memory / 100, clamped to 3 MiB–5 MiB
/// (5,242,880 bytes on any machine above 500 MB).
pub fn cache_limit(physical: u64) -> u32 {
    (physical / 100).clamp(3 * 1024 * 1024, u64::from(CACHE_LIMIT)) as u32
}
/// Duration of one sound tick in ms: one client tick, used only by
/// D2RUST's channel-end model ([`SoundSystem`] upkeep).
pub const TICK_MS: u64 = crate::rules::camera::CLIENT_TICK_MS as u64;
/// Distance clamp per axis for distance².
const AXIS_CLAMP: f32 = 2000.0;
/// Group base of `event_thunder_*`.
const THUNDER_BASE: i32 = 202;
/// `object_river`, the object whose sound comes from the river point.
const RIVER: i32 = 2599;
/// z of a unit request and the offset of a set
/// position.
const UNIT_Z: f32 = 640.0;
/// z of a request without a unit.
const NO_UNIT_Z: f32 = 320.0;
/// Occlusion step per tick (f32 0.05 widened).
const OCCLUSION_STEP: f32 = 0.05;

/// What the sound layer needs from the game world.
pub trait SoundWorld {
    /// The local player unit.
    fn local_player(&self) -> Option<UnitKey>;
    /// A unit's client position: its client **pixel point**: the dynamic
    /// path's position for unit types 0, 1, 3; the static path's for types
    /// 2, 4, 5.
    fn position(&self, unit: UnitKey) -> Option<(i32, i32)>;
    /// The position of the client-only unit with this key: a client-only
    /// object can share its GUID with a server object, and the sound
    /// layer's [`UnitKey`] cannot tell them apart. `object_river` (2599)
    /// is asked of the client-only units first.
    fn client_only_position(&self, _unit: UnitKey) -> Option<(i32, i32)> {
        None
    }
    /// The unit is blocked from the player (the line test with mask 2).
    fn blocked(&self, unit: UnitKey) -> bool;
    /// `Indoors` of the current sound environment.
    fn indoors(&self) -> bool;
    /// The state-duck condition (single player with the ESC menu or the
    /// options panel open).
    fn state_duck(&self) -> bool;
    /// The local player's client unit seed (the variant draws use it).
    fn client_seed(&mut self) -> Option<&mut Seed>;
}

/// Request state (+0x30).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestState {
    Waiting,
    Playing,
    Ended,
}

/// A volume fade (+0x45 … +0x55).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fade {
    pub start: i32,
    pub end: i32,
    pub t0: u32,
    pub t1: u32,
}

impl Fade {
    /// The volume at `now` and whether the fade has ended
    /// (signed 32-bit, truncating).
    pub fn volume_at(&self, now: u32) -> (i32, bool) {
        if now < self.t0 {
            (self.start, false)
        } else if now > self.t1 {
            (self.end, true)
        } else {
            let num = self
                .end
                .wrapping_sub(self.start)
                .wrapping_mul(now.wrapping_sub(self.t0) as i32);
            let v = self
                .start
                .wrapping_add(num / self.t1.wrapping_sub(self.t0) as i32);
            (v, false)
        }
    }
}

/// One request.
#[derive(Clone, Debug)]
pub struct Request {
    /// Pool slot (its address order).
    pub slot: usize,
    /// +0x04, the current id: the requested id, overwritten by the variant
    /// at every start attempt.
    pub id: i32,
    /// +0x08.
    pub handle: Handle,
    /// +0x0C … +0x14.
    pub pos: [f32; 3],
    /// +0x18.
    pub dist2: f32,
    /// +0x1C.
    pub volume: i32,
    /// +0x20.
    pub occlusion: f32,
    /// The stream start offset, in 4-byte units (the stream starts at `offset × 4`
    /// bytes).
    pub start_offset: u32,
    /// +0x28, newest first. A compound request can hold several
    /// units, or one unit twice.
    pub units: Vec<UnitKey>,
    /// +0x2C.
    pub channel: Option<usize>,
    pub state: RequestState,
    /// +0x34.
    pub start_tick: u32,
    /// +0x38.
    pub flags: u32,
    /// +0x3C.
    pub priority: u8,
    /// +0x3D.
    pub stop: bool,
    /// +0x41, in 4-byte units; 0 is "none".
    pub resume_offset: u32,
    pub fade: Option<Fade>,
}

/// Channel kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    /// Kind 0: plain mono buffer (slots 4–15 in mixer mode 0).
    Plain,
    /// Kind 1: stereo buffer (slots 0–3 in every mode).
    Stereo,
    /// Kind 2: slots 4–15 in mixer mode 1 (not reproduced).
    Positional,
    /// Kind 6: slots 4–15 in mixer mode 2 (not reproduced).
    Eax,
}

/// The slot kinds of mixer mode 0: 4 stereo and 12
/// mono channels. D2RUST reproduces mode 0 only.
pub const MODE0_LAYOUT: [ChannelKind; CHANNELS] = {
    let mut l = [ChannelKind::Plain; CHANNELS];
    let mut i = 0;
    while i < 4 {
        l[i] = ChannelKind::Stereo;
        i += 1;
    }
    l
};

/// One busy channel.
#[derive(Clone, Debug)]
struct Channel {
    request: usize,
    cue: CueId,
    start_tick: u32,
    /// First frame played (the stream start offset; 0 otherwise).
    offset_frames: u64,
    frames: u64,
    /// `frames` without the sample's zero tail, the length a device-end
    /// row matches.
    tail_frames: u64,
    rate: u64,
    block_align: u64,
    looped: bool,
    stereo: bool,
    stream: bool,
    /// Loop start in bytes (`Block 1` × 2), when the block count is 1.
    loop_start: Option<u32>,
    vol: i32,
    pan: i32,
    /// (v after the request's volume step, occlusion, position) last sent; none
    /// right after the start, so the update of the start tick sends again.
    last_sent: Option<(i32, f32, [f32; 3])>,
    /// Occlusion last handed to the device.
    device_occlusion: f32,
}

/// A fatal condition of the sound layer, kept as an error instead of aborting.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SoundError {
    #[error(transparent)]
    Table(#[from] SoundTableError),
    /// `len = 0` with `delay ≠ 0` (fatal `0x25C`).
    #[error("fade of handle {handle} with len 0 and delay {delay}")]
    FadeDelay { handle: Handle, delay: u32 },
    /// A request for an id outside the table (there is no record to read).
    #[error("sound id {0} outside the table")]
    OutOfTable(i32),
    /// Unloading a sample a channel plays
    /// (fatal `0x259`).
    #[error("sound {0}: unload while a channel plays it")]
    UnloadPlaying(i32),
    /// A variant draw with no local player (there is no seed to draw from).
    #[error("variant draw roll({0}) without a local player")]
    NoLocalPlayer(i32),
}

/// The sample cache state.
#[derive(Clone, Debug)]
pub struct Cache {
    /// The cache limit ([`cache_limit`]).
    pub limit: u32,
    /// Bytes charged by loaded and pending samples.
    pub total: u32,
    /// Ids, least recently used first.
    pub lru: Vec<i32>,
    /// Pending async loads (not decremented for an
    /// abandoned read).
    pub pending: u32,
    /// The tick of the last failed eviction.
    pub failed_eviction: u32,
    /// The tick of the last preload pass.
    pub last_pass: u32,
    /// Async reads in flight: id → (tick started, sample).
    reads: BTreeMap<i32, (u32, Arc<Sound>)>,
}

impl Default for Cache {
    fn default() -> Self {
        Cache {
            limit: CACHE_LIMIT,
            total: 0,
            lru: Vec::new(),
            pending: 0,
            // The sound init sets the last failed eviction to 0,
            // so the T ≠ 0 passes
            // before T = 250 start no async preload.
            failed_eviction: 0,
            last_pass: 0,
            reads: BTreeMap::new(),
        }
    }
}

/// The sound table, the request pool and the channels.
pub struct SoundSystem {
    rate: d2_sim::tick::TickRate,
    table: SoundTableData,
    bank: Box<dyn SoundBank + Send>,
    settings: SoundSettings,
    /// The request handle counter.
    enabled: bool,
    slots: Vec<Option<Request>>,
    /// Active list: slot indices.
    active: Vec<usize>,
    next_handle: u32,
    channels: [Option<Channel>; CHANNELS],
    /// Each unit's request list, newest
    /// first.
    unit_lists: BTreeMap<UnitKey, Vec<Handle>>,
    /// The sound table is loaded (sound init ran).
    tick: u32,
    last_update: Option<u32>,
    solo_duck: i32,
    state_duck: i32,
    cache: Cache,
    errors: Vec<SoundError>,
    /// `play --sound-log`: every
    /// call of [`SoundSystem::request`] at its entry, when on.
    request_log: Option<Vec<RequestCall>>,
    /// Recorded device ends: (start tick, frames, end tick).
    device_ends: Vec<(u32, u64, u32)>,
    /// A mod game's client hooks: its per-category gains
    /// ([`Self::set_mods`]); empty in D2RUST's own game.
    mods: ClientMods,
    /// Recorded async collects: per sound id, the
    /// preload-pass ticks that collected a finished read, oldest first.
    async_collects: Option<BTreeMap<i32, VecDeque<u32>>>,
}

/// One call of the request entry, its arguments as given
/// and the sound tick at the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestCall {
    pub sound_tick: u32,
    pub id: i32,
    pub unit: Option<UnitKey>,
    pub delay: u32,
    pub flags: u32,
    pub offset: u32,
}

impl SoundSystem {
    pub fn set_tick_rate(&mut self, rate: d2_sim::tick::TickRate) {
        self.rate = rate;
    }

    fn period(&self, base_ticks: u32) -> u32 {
        if self.rate.is_base() {
            base_ticks
        } else {
            self.rate.ticks_for_ms(base_ticks.saturating_mul(40))
        }
    }

    pub fn new(table: SoundTableData, bank: Box<dyn SoundBank + Send>) -> Self {
        SoundSystem {
            rate: d2_sim::tick::TickRate::BASE,
            table,
            bank,
            settings: SoundSettings::default(),
            enabled: true,
            slots: vec![None; REQUEST_SLOTS],
            active: Vec::new(),
            next_handle: 0,
            channels: Default::default(),
            unit_lists: BTreeMap::new(),
            tick: 0,
            last_update: None,
            solo_duck: 100,
            state_duck: 100,
            cache: Cache::default(),
            errors: Vec::new(),
            request_log: None,
            device_ends: Vec::new(),
            async_collects: None,
            mods: ClientMods::default(),
        }
    }

    /// Starts (or stops) logging every request call; a running log is kept.
    /// A check's given ends of one-shots (a host
    /// input like the frame schedule): a channel started at the tick with
    /// that many frames ends in the first upkeep at or after the end tick,
    /// instead of by the elapsed-time model.
    pub fn set_device_ends(&mut self, ends: Vec<(u32, u64, u32)>) {
        self.device_ends = ends;
    }

    /// A check's given async collects (a host
    /// input like the device ends): a pending read of a sound id is
    /// collected by the first preload pass at or after that id's next
    /// given collect tick, instead of by the read-latency model;
    /// an id with no given collect left stays pending.
    pub fn set_async_collects(&mut self, collects: Vec<(u32, i32)>) {
        let mut m: BTreeMap<i32, VecDeque<u32>> = BTreeMap::new();
        for (t, id) in collects {
            m.entry(id).or_default().push_back(t);
        }
        self.async_collects = Some(m);
    }

    pub fn log_requests(&mut self, on: bool) {
        match (on, self.request_log.is_some()) {
            (true, false) => self.request_log = Some(Vec::new()),
            (false, _) => self.request_log = None,
            _ => {}
        }
    }

    /// The request calls since the last take, in call order.
    pub fn take_request_log(&mut self) -> Vec<RequestCall> {
        self.request_log
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    pub fn table(&self) -> &SoundTableData {
        &self.table
    }

    pub fn table_mut(&mut self) -> &mut SoundTableData {
        &mut self.table
    }

    pub fn settings(&self) -> &SoundSettings {
        &self.settings
    }

    pub fn set_settings(&mut self, s: SoundSettings) {
        self.settings = s;
    }

    /// A mod game's client hooks; their audio gains apply from the next
    /// volume computed.
    pub fn set_mods(&mut self, mods: ClientMods) {
        self.mods = mods;
    }

    /// The category of sound `id` for a mod game's gains: music (songs and
    /// sounds that follow the music volume), speech, or effects.
    pub fn category(&self, id: i32) -> SoundCategory {
        let music = self.table.get(id).is_some_and(|e| e.row.music_vol != 0);
        if music || self.table.is_song(id) || is_music_path(id) {
            SoundCategory::Music
        } else if is_speech(id) {
            SoundCategory::Speech
        } else {
            SoundCategory::Effects
        }
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    pub fn cache_mut(&mut self) -> &mut Cache {
        &mut self.cache
    }

    pub fn tick(&self) -> u32 {
        self.tick
    }

    pub fn solo_duck(&self) -> i32 {
        self.solo_duck
    }

    pub fn state_duck(&self) -> i32 {
        self.state_duck
    }

    pub fn take_errors(&mut self) -> Vec<SoundError> {
        std::mem::take(&mut self.errors)
    }

    /// The active requests in list order.
    pub fn requests(&self) -> impl Iterator<Item = &Request> {
        self.active.iter().filter_map(|&s| self.slots[s].as_ref())
    }

    pub fn request_by_handle(&self, h: Handle) -> Option<&Request> {
        self.requests().find(|r| r.handle == h && h != 0)
    }

    /// The request playing on channel `c`.
    pub fn channel_request(&self, c: usize) -> Option<&Request> {
        let ch = self.channels.get(c)?.as_ref()?;
        self.slots[ch.request].as_ref()
    }

    /// Volume and pan last sent on channel `c`.
    pub fn channel_output(&self, c: usize) -> Option<(i32, i32)> {
        let ch = self.channels.get(c)?.as_ref()?;
        Some((ch.vol, ch.pan))
    }

    /// Loop start in bytes of channel `c` (`Block 1` × 2 when the
    /// block count is 1). The mixer gets it as a frame through the start's
    /// [`DeviceChange`].
    pub fn channel_loop_start(&self, c: usize) -> Option<u32> {
        self.channels.get(c)?.as_ref()?.loop_start
    }

    /// Wraps this system with a world as the [`SoundCalls`] surface.
    pub fn with<'a>(&'a mut self, world: &'a mut dyn SoundWorld) -> SoundCtx<'a> {
        SoundCtx { sys: self, world }
    }

    fn handle_slot(&self, h: Handle) -> Option<usize> {
        if h == 0 {
            return None;
        }
        self.active
            .iter()
            .copied()
            .find(|&s| self.slots[s].as_ref().is_some_and(|r| r.handle == h))
    }

    fn req(&self, s: usize) -> &Request {
        self.slots[s].as_ref().expect("active slot holds a request")
    }

    fn req_mut(&mut self, s: usize) -> &mut Request {
        self.slots[s].as_mut().expect("active slot holds a request")
    }

    // ------------------------------------------------------------ requests

    /// Position and distance² of `unit` relative to the local player,
    /// for a request whose current
    /// id is `id` (2599 `object_river` uses the projected point).
    fn unit_position(world: &dyn SoundWorld, unit: UnitKey, id: i32) -> Option<([f32; 3], f32)> {
        let (a, b) = world.position(world.local_player()?)?;
        let (mut c, mut e) = match id {
            RIVER => world
                .client_only_position(unit)
                .or_else(|| world.position(unit))?,
            _ => world.position(unit)?,
        };
        if id == RIVER {
            (c, e) = river_point((a, b), (c, e));
        }
        let x = c.wrapping_sub(a) as f32;
        let y = 2.0 * (e.wrapping_sub(b) as f32);
        Some(([x, y, UNIT_Z], dist2(x, y)))
    }

    /// A unit's occlusion value.
    fn unit_occlusion(&self, world: &dyn SoundWorld, id: i32, unit: UnitKey) -> f32 {
        let Some(e) = self.table.get(id) else {
            return 0.0;
        };
        if e.row.falloff == 4 {
            0.0
        } else if e.group_base == THUNDER_BASE {
            if world.indoors() {
                0.5
            } else {
                0.0
            }
        } else if world.blocked(unit) {
            0.5
        } else {
            0.0
        }
    }

    /// Pushes `h` at the head of `unit`'s request list.
    fn push_unit_handle(&mut self, unit: UnitKey, h: Handle) {
        self.unit_lists.entry(unit).or_default().insert(0, h);
    }

    /// Removes the first node holding `h` from `unit`'s list.
    fn drop_unit_handle(&mut self, unit: UnitKey, h: Handle) {
        if let Some(list) = self.unit_lists.get_mut(&unit) {
            if let Some(i) = list.iter().position(|&x| x == h) {
                list.remove(i);
            }
            if list.is_empty() {
                self.unit_lists.remove(&unit);
            }
        }
    }

    /// A new request.
    pub fn request(
        &mut self,
        world: &mut dyn SoundWorld,
        id: i32,
        unit: Option<UnitKey>,
        delay: u32,
        flags: u32,
        offset: u32,
    ) -> Handle {
        if let Some(log) = self.request_log.as_mut() {
            log.push(RequestCall {
                sound_tick: self.tick,
                id,
                unit,
                delay,
                flags,
                offset,
            });
        }
        let delay = self.period(delay);
        if !self.enabled || id < 1 {
            return 0;
        }
        let Some(e) = self.table.get(id) else {
            // There is no record to read here.
            self.errors.push(SoundError::OutOfTable(id));
            return 0;
        };
        if e.row.volume == 0 {
            return 0;
        }
        let (compound, base, priority) = (e.row.compound, e.group_base, e.row.priority);
        let now = self.tick;
        let local = unit.is_some() && unit == world.local_player();
        // The window test is unsigned; the merged
        // call still attaches its unit at the list heads.
        if compound != 0 {
            let hit = self.active.iter().copied().find(|&s| {
                let r = self.req(s);
                self.table.base(r.id) == base
                    && !r.stop
                    && (compound < 0
                        || now.wrapping_sub(r.start_tick) <= self.period(compound as u32))
            });
            if let Some(s) = hit {
                let r = self.req_mut(s);
                if local {
                    r.priority = r.priority.wrapping_add(80);
                }
                let h = r.handle;
                if let Some(u) = unit {
                    r.units.insert(0, u);
                    self.push_unit_handle(u, h);
                }
                return h;
            }
        }
        // First free slot.
        let Some(slot) = self.slots.iter().position(Option::is_none) else {
            return 0;
        };
        self.next_handle = self.next_handle.wrapping_add(1);
        let (pos, dist2, occlusion) = match unit {
            Some(u) => {
                let (pos, d2) =
                    Self::unit_position(world, u, id).unwrap_or(([0.0, 0.0, UNIT_Z], 0.0));
                (pos, d2, self.unit_occlusion(world, id, u))
            }
            None => ([0.0, 0.0, NO_UNIT_Z], 0.0, 0.0),
        };
        let mut priority = priority;
        // Byte wrap.
        if local {
            priority = priority.wrapping_add(80);
        }
        let h = self.next_handle;
        self.slots[slot] = Some(Request {
            slot,
            id,
            handle: h,
            pos,
            dist2,
            volume: 255,
            occlusion,
            start_offset: offset,
            units: unit.into_iter().collect(),
            channel: None,
            state: RequestState::Waiting,
            start_tick: now.wrapping_add(delay),
            flags,
            priority,
            stop: false,
            resume_offset: 0,
            fade: None,
        });
        // At the head of the active list.
        self.active.insert(0, slot);
        if let Some(u) = unit {
            self.push_unit_handle(u, h);
        }
        h
    }

    /// Sets the volume of request `h` to `v`.
    pub fn set_volume(&mut self, h: Handle, v: i32) {
        if let Some(s) = self.handle_slot(h) {
            self.req_mut(s).volume = v;
        }
    }

    /// Fades request `h` toward `target` after `delay` over `len`, in its exact order.
    pub fn fade(&mut self, h: Handle, target: i32, delay: u32, len: u32) {
        let Some(s) = self.handle_slot(h) else { return };
        let now = self.tick;
        let r = self.req(s);
        if r.units.len() > 1 {
            return;
        }
        let Some(e) = self.table.get(r.id) else {
            return;
        };
        let len = match target {
            255 => len.max(u32::from(e.row.fade_in)),
            0 => len.max(u32::from(e.row.fade_out)),
            _ => len,
        };
        let (delay, len) = (self.period(delay), self.period(len));
        if let Some(f) = r.fade {
            let end = now.wrapping_add(delay).wrapping_add(len);
            if f.end == target && end >= f.t1 {
                return;
            }
            if f.end == 0 && target != 0 {
                return;
            }
        }
        let r = self.req_mut(s);
        if target == 0 {
            r.stop = true;
        }
        if len == 0 {
            if delay != 0 {
                self.errors.push(SoundError::FadeDelay { handle: h, delay });
                return;
            }
            r.volume = target;
        } else {
            let t0 = now.wrapping_add(delay);
            r.fade = Some(Fade {
                start: r.volume,
                end: target,
                t0,
                t1: t0.wrapping_add(len),
            });
        }
    }

    /// Detaches request `h` from `unit` (`force` ends it too): the first
    /// node holding `h` leaves the unit's list
    /// whether or not the request exists; then one `unit` leaves the
    /// request's unit list.
    pub fn detach(&mut self, h: Handle, unit: UnitKey, force: bool) {
        self.drop_unit_handle(unit, h);
        let Some(s) = self.handle_slot(h) else { return };
        let r = self.req_mut(s);
        if let Some(i) = r.units.iter().position(|&u| u == unit) {
            r.units.remove(i);
        }
        if !r.units.is_empty() {
            return;
        }
        let fading_out = r.fade.is_some_and(|f| f.end == 0);
        let id = r.id;
        let looped = self.table.get(id).is_some_and(|e| e.row.looped != 0);
        if force || (looped && !fading_out) {
            self.req_mut(s).stop = true;
            self.fade(h, 0, 0, 0);
        }
    }

    /// Group stop of one request.
    fn group_stop(&mut self, s: usize) {
        let now = self.tick;
        let r = self.req(s);
        let fade_out = self
            .table
            .get(r.id)
            .map_or(0, |e| u32::from(e.row.fade_out));
        let fade_out = self.period(fade_out);
        let r = self.req_mut(s);
        r.stop = true;
        if r.state == RequestState::Playing && fade_out > 0 {
            r.fade = Some(Fade {
                start: r.volume,
                end: 0,
                t0: now,
                t1: now.wrapping_add(fade_out),
            });
        }
    }

    fn stop_where(&mut self, pred: impl Fn(&SoundTableData, &Request) -> bool) {
        let hits: Vec<usize> = self
            .active
            .iter()
            .copied()
            .filter(|&s| pred(&self.table, self.req(s)))
            .collect();
        for s in hits {
            self.group_stop(s);
        }
    }

    pub fn stop_handle(&mut self, h: Handle) {
        if let Some(s) = self.handle_slot(h) {
            self.group_stop(s);
        }
    }

    pub fn stop_id(&mut self, id: i32) {
        self.stop_where(|_, r| r.id == id);
    }

    pub fn stop_songs(&mut self) {
        self.stop_where(|t, r| t.is_song(r.id));
    }

    pub fn stop_range_except(&mut self, lo: i32, hi: i32, a: i32, b: i32) {
        self.stop_where(|t, r| (lo..=hi).contains(&r.id) && t.base(r.id) != a && t.base(r.id) != b);
    }

    pub fn stop_speech(&mut self) {
        self.stop_where(|_, r| is_speech(r.id));
    }

    /// `speaking(U)`.
    pub fn speaking(&self, unit: UnitKey) -> bool {
        self.requests()
            .any(|r| r.state == RequestState::Playing && is_speech(r.id) && r.units.contains(&unit))
    }

    /// `any_speech`.
    pub fn any_speech(&self) -> bool {
        self.requests()
            .any(|r| r.state != RequestState::Ended && is_speech(r.id))
    }

    pub fn is_active(&self, h: Handle) -> bool {
        self.request_by_handle(h)
            .is_some_and(|r| r.state != RequestState::Ended)
    }

    /// `roll(n)` on the client seed. A draw with `n ≥ 1` needs a local
    /// player (`n < 1` returns 0 first), and no game path draws without
    /// one. D2RUST treats such a draw as an internal error (debug assert)
    /// and returns 0 without a step in release, which is a fallback, not
    /// a game rule.
    pub fn roll(world: &mut dyn SoundWorld, n: i32) -> u32 {
        if n < 1 {
            return 0;
        }
        match world.client_seed() {
            Some(s) => s.roll(n),
            None => {
                debug_assert!(
                    world.local_player().is_some(),
                    "{}",
                    SoundError::NoLocalPlayer(n)
                );
                0
            }
        }
    }

    // ------------------------------------------------------------ the sample cache

    /// Takes a lock on `id`'s group.
    pub fn lock(&mut self, id: i32, delta: i32) -> Result<(), SoundError> {
        Ok(self.table.lock(id, delta)?)
    }

    /// The use stamp: last use :=
    /// T and the id moves to the tail of the use list.
    fn stamp(&mut self, id: i32, now: u32) {
        if let Some(e) = self.table.get_mut(id) {
            e.last_use = now;
        }
        self.cache.lru.retain(|&x| x != id);
        self.cache.lru.push(id);
    }

    /// The format check of a non-stream load:
    /// `Stereo` := (file channels = 2).
    fn loaded(&mut self, id: i32, sample: Arc<Sound>) {
        if let Some(e) = self.table.get_mut(id) {
            e.row.stereo = u8::from(sample.channels() == 2);
            e.sample = Some(sample);
            e.load = LoadState::Loaded;
        }
    }

    /// Collect a finished async read.
    fn collect(&mut self, id: i32) {
        if let Some((_, sample)) = self.cache.reads.remove(&id) {
            self.loaded(id, sample);
            self.cache.pending = self.cache.pending.saturating_sub(1);
        }
    }

    /// Loads sample `id` (`sync`: wait for it; `recent`: the eviction's recent window).
    fn load(&mut self, id: i32, sync: bool, recent: bool, now: u32) {
        let Some(e) = self.table.get(id) else { return };
        if e.failed || e.load == LoadState::Loaded {
            return;
        }
        if e.load == LoadState::Pending {
            if sync {
                self.collect(id);
            }
            return;
        }
        let Some(sample) = self.bank.samples(SoundId(id as u32)) else {
            self.table.get_mut(id).expect("in table").failed = true;
            return;
        };
        let size = self
            .bank
            .file_size(SoundId(id as u32))
            .unwrap_or(sound_bytes(&sample) + 44);
        let size = u32::try_from(size).unwrap_or(u32::MAX);
        let after = self.cache.total.wrapping_add(size);
        if after > self.cache.limit && !self.evict(after - self.cache.limit, sync, recent, now) {
            self.cache.failed_eviction = now;
            return;
        }
        self.cache.total = self.cache.total.wrapping_add(size);
        self.table.get_mut(id).expect("in table").size = size;
        self.stamp(id, now);
        if sync {
            self.loaded(id, sample);
        } else {
            self.table.get_mut(id).expect("in table").load = LoadState::Pending;
            self.cache.reads.insert(id, (now, sample));
            self.cache.pending = self.cache.pending.wrapping_add(1);
        }
    }

    /// Among the channels
    /// whose request's current id is `id`, the one with the smallest start
    /// tick.
    fn playing_channel_of(&self, id: i32) -> Option<usize> {
        let mut best: Option<(usize, u32)> = None;
        for (c, ch) in self.channels.iter().enumerate() {
            let Some(ch) = ch else { continue };
            let r = self.req(ch.request);
            if r.id == id && best.is_none_or(|(_, t)| r.start_tick < t) {
                best = Some((c, r.start_tick));
            }
        }
        best.map(|(c, _)| c)
    }

    /// Eviction (`need` bytes; `second`, `recent`: the second pass and the
    /// recent window). True when enough was freed.
    fn evict(&mut self, need: u32, second: bool, recent: bool, now: u32) -> bool {
        let target = self.cache.total.wrapping_sub(need);
        while self.cache.total > target {
            let mut any = false;
            for walk in 0..if second { 2 } else { 1 } {
                let pick = self.cache.lru.iter().copied().find(|&id| {
                    let Some(e) = self.table.get(id) else {
                        return false;
                    };
                    let kept = (walk == 0 && (e.locks > 0 || e.row.cache != 0))
                        || (recent && now.wrapping_sub(e.last_use) < self.period(RECENT_USE))
                        || self.playing_channel_of(id).is_some();
                    !kept
                });
                if let Some(id) = pick {
                    self.unload(id);
                    any = true;
                }
            }
            if !any {
                return false;
            }
        }
        true
    }

    /// Unload sample `id`. An abandoned
    /// async read keeps its pending count (preserved bug, reproduced).
    fn unload(&mut self, id: i32) {
        let Some(e) = self.table.get(id) else { return };
        if e.load == LoadState::None {
            return;
        }
        if self.playing_channel_of(id).is_some() {
            self.errors.push(SoundError::UnloadPlaying(id));
            return;
        }
        let size = e.size;
        self.cache.total = self.cache.total.wrapping_sub(size);
        self.cache.lru.retain(|&x| x != id);
        self.cache.reads.remove(&id);
        let e = self.table.get_mut(id).expect("in table");
        e.last_use = 0;
        e.sample = None;
        e.load = LoadState::None;
    }

    /// The preload pass at tick T: at T = 0 and every 25 ticks after,
    /// over ids 1–2,933; at T =
    /// 0 the loads are synchronous.
    fn preload(&mut self, now: u32) {
        if now != 0 && now.wrapping_sub(self.cache.last_pass) < self.period(PRELOAD_PERIOD) {
            return;
        }
        self.cache.last_pass = now;
        let last = PRELOAD_LAST_ID.min(self.table.count() as i32 - 1);
        for id in 1..=last {
            let e = &self.table.entries()[id as usize];
            if e.load == LoadState::None && (e.locks > 0 || e.row.cache != 0) {
                if now == 0 {
                    self.load(id, true, true, now);
                } else if self.cache.pending < MAX_PENDING_LOADS
                    && now.wrapping_sub(self.cache.failed_eviction) >= self.period(EVICTION_HOLD)
                {
                    self.load(id, false, true, now);
                }
            }
            // A read's finish is host timing: a check replays the given
            // collect ticks; otherwise a read is finished by the first
            // pass after the tick it started (D2RUST has no read latency),
            // so one started in this pass is not collected until the next.
            let Some(&(started, _)) = self.cache.reads.get(&id) else {
                continue;
            };
            let finished = match &mut self.async_collects {
                // the given collect tick for this read
                Some(m) => match m.get_mut(&id) {
                    Some(q)
                        if q.front()
                            .is_some_and(|&t| now.wrapping_sub(t) < u32::MAX / 2) =>
                    {
                        q.pop_front();
                        true
                    }
                    _ => false,
                },
                None => started != now,
            };
            if finished {
                self.collect(id);
            }
        }
    }

    // ------------------------------------------------------------ the request update

    /// One sound tick: preload, the request update at
    /// the current tick, channel upkeep, then tick + 1.
    pub fn run_tick(&mut self, world: &mut dyn SoundWorld, queue: &mut TriggerQueue) {
        let now = self.tick;
        self.preload(now);
        self.update(world, queue, now);
        self.upkeep(now);
        self.tick = now.wrapping_add(1);
    }

    /// The priority order: true when `a` is more important than `b`.
    fn more_important(a: &Request, b: &Request) -> bool {
        if a.priority != b.priority {
            return a.priority > b.priority;
        }
        if a.dist2 != b.dist2 {
            return a.dist2 < b.dist2;
        }
        if a.start_tick != b.start_tick {
            return a.start_tick > b.start_tick;
        }
        a.slot > b.slot
    }

    /// Bubble sort of the active list, most important first.
    fn sort_active(&mut self) {
        let n = self.active.len();
        for i in 0..n {
            let mut swapped = false;
            for j in 0..n - 1 - i {
                let (a, b) = (self.active[j], self.active[j + 1]);
                if Self::more_important(self.req(b), self.req(a)) {
                    self.active.swap(j, j + 1);
                    swapped = true;
                }
            }
            if !swapped {
                break;
            }
        }
    }

    fn audible(&self, id: i32) -> bool {
        self.settings.master_volume > 0
            && !(self.table.is_song(id) && self.settings.music_volume == 0)
    }

    fn max2(&self, id: i32) -> f32 {
        let (_, max) = falloff(self.table.get(id).map_or(0, |e| e.row.falloff));
        (max as f32) * (max as f32)
    }

    /// Frees a request: for each unit of its list, the first
    /// node holding its handle leaves that unit's list.
    fn free(&mut self, s: usize) {
        if let Some(r) = self.slots[s].take() {
            for u in r.units {
                self.drop_unit_handle(u, r.handle);
            }
        }
        self.active.retain(|&a| a != s);
    }

    /// The request update at `now`.
    fn update(&mut self, world: &mut dyn SoundWorld, queue: &mut TriggerQueue, now: u32) {
        self.sort_active();
        // Priority order first.
        for s in self.active.clone() {
            let r = self.req(s);
            if r.stop && r.fade.is_none_or(|f| f.end != 0) {
                match r.state {
                    RequestState::Waiting => self.req_mut(s).state = RequestState::Ended,
                    RequestState::Playing => self.stop_channel(s, queue, now, "stop"),
                    RequestState::Ended => {}
                }
            }
        }
        let mut removal = Vec::new();
        for s in self.active.clone() {
            if self.slots[s].is_none() {
                continue;
            }
            self.update_one(world, queue, now, s, &mut removal);
        }
        for s in removal {
            if self.slots[s].as_ref().is_some_and(|r| r.channel.is_none()) {
                self.free(s);
            }
        }
        // The state duck.
        if self.last_update != Some(now) {
            let solo = self.requests().any(|r| {
                self.table.get(r.id).is_some_and(|e| e.row.solo != 0)
                    && !r.fade.is_some_and(|f| f.end == 0)
            });
            self.solo_duck = if solo {
                (self.solo_duck - self.rate.share(2, now as i32)).max(SOLO_DUCK_MIN)
            } else {
                (self.solo_duck + self.rate.share(2, now as i32)).min(100)
            };
            self.state_duck = if world.state_duck() {
                (self.state_duck - self.rate.share(5, now as i32)).max(0)
            } else {
                (self.state_duck + self.rate.share(5, now as i32)).min(100)
            };
        }
        self.last_update = Some(now);
    }

    /// The per-request update, in list order: an ended request only does
    /// the end step; any other runs tracking, the fade, the start test and
    /// the start, then the volume step if it is playing at that point (one
    /// just started included). The start and the volume step read the
    /// record of the current id, which the start test may have changed to
    /// the variant.
    fn update_one(
        &mut self,
        world: &mut dyn SoundWorld,
        queue: &mut TriggerQueue,
        now: u32,
        s: usize,
        removal: &mut Vec<usize>,
    ) {
        let id = self.req(s).id;
        let Some(e) = self.table.get(id) else { return };
        let row = e.row.clone();
        // The end step. A restarted loop waits for the next update.
        if self.req(s).state == RequestState::Ended {
            let r = self.req_mut(s);
            if row.looped != 0 && row.duration == 0 && !r.stop {
                r.state = RequestState::Waiting;
                if let Some(f) = r.fade.take() {
                    r.volume = f.end;
                }
            } else {
                self.free(s);
            }
            return;
        }
        // Tracking: the position of the nearest unit.
        if self.settings.game_loaded && row.tracking != 0 && !self.req(s).units.is_empty() {
            self.track(world, s, now);
        }
        // The fade.
        let r = self.req_mut(s);
        if let Some(f) = r.fade {
            let (v, ended) = f.volume_at(now);
            r.volume = v;
            if ended {
                r.fade = None;
            }
        }
        let due = now >= r.start_tick && r.volume != 0;
        // The start test.
        if self.req(s).state == RequestState::Waiting
            && due
            && self.audible(id)
            && self.req(s).dist2 <= self.max2(id)
            && self.instance_rules(s, &row)
        {
            self.start_with_fade_in(world, queue, now, s, &row);
        }
        // The start, with the current id's record.
        let id = self.req(s).id;
        let Some(e) = self.table.get(id) else { return };
        let row = e.row.clone();
        let r = self.req(s);
        if due && r.channel.is_none() && row.looped == 0 {
            let loading = row.async_only != 0 && e.load == LoadState::Pending;
            if !loading {
                removal.push(s);
            }
        }
        // The volume step (the `Duration` test is unsigned).
        if r.state == RequestState::Playing {
            let over = row.duration > 0
                && now.wrapping_sub(r.start_tick) > self.period(u32::from(row.duration));
            if over || !self.audible(id) || r.dist2 > self.max2(id) {
                self.stop_channel(s, queue, now, "end");
            } else {
                self.send(queue, now, s);
                if self
                    .table
                    .get(id)
                    .is_some_and(|e| e.load == LoadState::Loaded)
                {
                    self.stamp(id, now);
                }
            }
        }
    }

    /// Tracking: position of the nearest unit (unclamped squared length,
    /// strictly nearer replaces, so the earlier unit in the list wins a
    /// tie); occlusion toward the units' mean by at most 0.05.
    fn track(&mut self, world: &mut dyn SoundWorld, s: usize, now: u32) {
        let r = self.req(s);
        let id = r.id;
        let mut nearest: Option<([f32; 3], f32, f32)> = None;
        let mut sum = 0.0f32;
        let units = r.units.clone();
        for &u in &units {
            if let Some((p, d2)) = Self::unit_position(world, u, id) {
                let len2 = p[0] * p[0] + p[1] * p[1];
                if nearest.is_none_or(|(_, _, n)| len2 < n) {
                    nearest = Some((p, d2, len2));
                }
            }
            sum += self.unit_occlusion(world, id, u);
        }
        let target = sum / units.len() as f32;
        let advance_occlusion = self.rate.share(1, now as i32) != 0;
        let r = self.req_mut(s);
        if let Some((p, d2, _)) = nearest {
            r.pos = p;
            r.dist2 = d2;
        }
        if advance_occlusion {
            r.occlusion = occlusion_step(r.occlusion, target);
        }
    }

    /// The start test (group and speech rules). Returns whether the request may start.
    fn instance_rules(&mut self, s: usize, row: &d2_data::sounds::SoundRow) -> bool {
        let r = self.req(s);
        let id = r.id;
        let unit = (r.units.len() == 1).then(|| r.units[0]);
        let found = if is_speech(id) && unit.is_some() {
            // Any playing speech request of that unit.
            self.active.iter().copied().find(|&o| {
                let q = self.req(o);
                o != s
                    && q.state == RequestState::Playing
                    && is_speech(q.id)
                    && q.units.contains(&unit.expect("one unit"))
            })
        } else if row.defer_inst != 0 || row.stop_inst != 0 {
            // Same group base, smallest start tick.
            let base = self.table.base(id);
            let mut best: Option<usize> = None;
            for &o in &self.active {
                let q = self.req(o);
                if o == s
                    || q.state != RequestState::Playing
                    || self.table.base(q.id) != base
                    || unit.is_some_and(|u| !q.units.contains(&u))
                {
                    continue;
                }
                if best.is_none_or(|b| q.start_tick < self.req(b).start_tick) {
                    best = Some(o);
                }
            }
            best
        } else {
            None
        };
        let Some(f) = found else { return true };
        let r = self.req(s);
        if row.defer_inst == 0 || !r.units.is_empty() || r.pos == [0.0; 3] {
            if row.stop_inst == 0 {
                self.req_mut(s).stop = true;
                return false;
            }
            self.req_mut(f).stop = true;
        }
        true
    }

    /// The start with its fade-in: a stored resume offset (≠ 0) becomes
    /// the start offset with a 3-tick fade-in, else `Fade In` without a
    /// running fade or flag bit 1 gives a `Fade In`-tick one. A fade-in
    /// writes volume 0 before the start; only a successful start sets the
    /// fade back to the saved volume (a failed one leaves volume 0; kept
    /// on purpose).
    fn start_with_fade_in(
        &mut self,
        world: &mut dyn SoundWorld,
        queue: &mut TriggerQueue,
        now: u32,
        s: usize,
        row: &d2_data::sounds::SoundRow,
    ) {
        let rate = self.rate;
        let r = self.req_mut(s);
        let len = if r.resume_offset != 0 {
            r.start_offset = std::mem::take(&mut r.resume_offset);
            Some(rate.ticks_for_ms(RESUME_FADE * 40))
        } else if row.fade_in > 0 && r.fade.is_none() && r.flags & FLAG_NO_FADE_IN == 0 {
            Some(rate.ticks_for_ms(u32::from(row.fade_in) * 40))
        } else {
            None
        };
        let saved = r.volume;
        if len.is_some() {
            r.volume = 0;
        }
        if self.start_channel(world, queue, now, s) {
            let r = self.req_mut(s);
            r.state = RequestState::Playing;
            if let Some(len) = len {
                r.fade = Some(Fade {
                    start: 0,
                    end: saved,
                    t0: now,
                    t1: now.wrapping_add(len),
                });
            }
        }
    }

    // ------------------------------------------------------------ channels

    /// Starts a channel for request `s`. The variant pick
    /// overwrites the request's id before the duplicate test; the history
    /// goes on the record of the id before the pick; everything after the
    /// pick reads the variant's record.
    fn start_channel(
        &mut self,
        world: &mut dyn SoundWorld,
        queue: &mut TriggerQueue,
        now: u32,
        s: usize,
    ) -> bool {
        let r = self.req(s);
        let (id, flags, start_tick, priority) = (r.id, r.flags, r.start_tick, r.priority);
        // A free channel.
        let variant = if flags & FLAG_EXACT != 0 {
            id
        } else {
            self.table.pick_variant(id, &mut |n| Self::roll(world, n))
        };
        self.req_mut(s).id = variant;
        let dup = self.playing_channel_of(variant).is_some_and(|c| {
            let q = self.req(self.channels[c].as_ref().expect("busy").request);
            !q.stop
                && start_tick.wrapping_sub(q.start_tick) <= self.period(1)
                && q.priority >= priority
        });
        if dup {
            return false;
        }
        // The sample.
        self.table.record_history(id, variant);
        let Some(e) = self.table.get(variant) else {
            self.errors.push(SoundError::OutOfTable(variant));
            return false;
        };
        let stream = e.row.stream != 0;
        if !stream {
            let sync = e.row.async_only == 0;
            self.load(variant, sync, false, now);
            let e = self.table.get(variant).expect("in table");
            if e.load != LoadState::Loaded {
                return false;
            }
        }
        let e = self.table.get(variant).expect("in table");
        let vrow = e.row.clone();
        let block_count = e.block_count;
        // The kind is the variant's `Stereo` (after the format
        // check of a load); slots match by equal kind only.
        let stereo = vrow.stereo != 0;
        let want = if stereo {
            ChannelKind::Stereo
        } else {
            ChannelKind::Plain
        };
        let fits = |c: usize| MODE0_LAYOUT[c] == want;
        let mut slot = (0..CHANNELS).find(|&c| fits(c) && self.channels[c].is_none());
        if slot.is_none() {
            let mut victim: Option<usize> = None;
            for c in (0..CHANNELS).filter(|&c| fits(c)) {
                let Some(ch) = &self.channels[c] else {
                    continue;
                };
                if victim.is_none_or(|v| {
                    let vr = self.req(self.channels[v].as_ref().expect("busy").request);
                    Self::more_important(vr, self.req(ch.request))
                }) {
                    victim = Some(c);
                }
            }
            if let Some(c) = victim {
                let vs = self.channels[c].as_ref().expect("busy").request;
                let v = self.req(vs);
                if priority > v.priority || (priority == v.priority && s > v.slot) {
                    self.stop_channel(vs, queue, now, "steal");
                    slot = Some(c);
                }
            }
        }
        let Some(c) = slot else { return false };
        // The sample is attached (cached) or
        // the stream opens; a stream that fails to open fails the start
        // with the slot left free and sets no file-failed flag.
        let sample = if stream {
            match self.bank.samples(SoundId(variant as u32)) {
                Some(x) => x,
                None => return false,
            }
        } else {
            match self.table.get(variant).and_then(|e| e.sample.clone()) {
                Some(x) => x,
                None => return false,
            }
        };
        // Loop, stream offset and start.
        let looped = vrow.looped != 0;
        let block_align = u64::from(sample.channels()) * 2;
        let data = sample.frames() as u64 * block_align;
        let r = self.req(s);
        let offset_frames = if stream {
            (u64::from(r.start_offset.wrapping_mul(4)) % data) / block_align
        } else {
            0
        };
        let loop_start =
            (!stream && block_count == 1).then(|| (vrow.blocks[0] as u32).wrapping_mul(2));
        // A looping stream wraps at its data rounded up to the stream block.
        let loop_end = (stream && looped).then(|| data.div_ceil(0x4000) * 0x4000 / block_align);
        let (_, vol, pan) = self.compute(s, stereo);
        let r = self.req(s);
        let cause = format!("sound {id}->{variant} h{}", r.handle);
        let trigger = Trigger {
            tick: now,
            source: TriggerSource::Ui,
            sound: SoundId(variant as u32),
            params: VoiceParams {
                vol,
                pan,
                looped,
                priority: i32::from(r.priority),
                group: self.table.base(variant),
            },
            cause,
        };
        let occlusion = r.occlusion;
        let cue = queue.push(Cue::Start(trigger));
        if offset_frames != 0 || loop_start.is_some() || loop_end.is_some() || occlusion != 0.0 {
            queue.push(Cue::Device(DeviceChange {
                tick: now,
                target: cue,
                start_frame: Some(offset_frames),
                loop_start: loop_start.map(|b| u64::from(b) / block_align),
                loop_end,
                occlusion: occlusion.to_bits(),
            }));
        }
        self.channels[c] = Some(Channel {
            request: s,
            cue,
            start_tick: now,
            offset_frames,
            frames: sample.frames() as u64,
            tail_frames: sample.frames_without_zero_tail() as u64,
            rate: u64::from(sample.rate()),
            block_align,
            looped,
            stereo,
            stream,
            loop_start,
            vol,
            pan,
            last_sent: None,
            device_occlusion: occlusion,
        });
        self.req_mut(s).channel = Some(c);
        true
    }

    /// Frames played on channel `c` by tick `now` (D2RUST's model: elapsed
    /// sound ticks × 40 ms at the file rate).
    ///
    /// A buffer's end is host timing (a voice service thread sees it on its
    /// own clock, and the saved play position comes from the device).
    /// D2RUST derives both from the tick (ended at the first upkeep with
    /// elapsed ticks × 40 ms ≥ the sample's duration), its own model of a
    /// host-timed event; a check replays the given ends instead
    /// (`device-ends.tsv`).
    fn played_frames(&self, ch: &Channel, now: u32) -> u64 {
        u64::from(now.wrapping_sub(ch.start_tick)) * ch.rate / u64::from(self.rate.hz())
    }

    /// The play position saved as a resume offset:
    /// for a stream, the bytes of `data` played (capped at the data size;
    /// a looping stream wraps) in 4-byte units; 0 for every other voice.
    fn resume_position(&self, c: usize, now: u32) -> u32 {
        let ch = self.channels[c].as_ref().expect("busy");
        if !ch.stream {
            return 0;
        }
        let mut f = ch.offset_frames + self.played_frames(ch, now);
        if ch.looped && ch.frames > 0 {
            f %= ch.frames;
        } else {
            f = f.min(ch.frames);
        }
        ((f * ch.block_align) >> 2) as u32
    }

    /// The stop: the play position becomes the
    /// resume offset, the voice stops, the request ends and the slot is
    /// freed. A steal does the same.
    fn stop_channel(&mut self, s: usize, queue: &mut TriggerQueue, now: u32, why: &str) {
        let Some(c) = self.req(s).channel else {
            self.req_mut(s).state = RequestState::Ended;
            return;
        };
        let resume = self.resume_position(c, now);
        let r = self.req_mut(s);
        r.state = RequestState::Ended;
        r.channel = None;
        r.resume_offset = resume;
        let handle = r.handle;
        if let Some(ch) = self.channels[c].take() {
            queue.push(Cue::Stop(Stop {
                tick: now,
                target: StopTarget::Voice(ch.cue),
                cause: format!("{why} h{handle}"),
            }));
        }
    }

    /// Channel upkeep: a non-`Loop` channel whose
    /// sample has played to its end frees itself and its request ends,
    /// with the stream's play position saved (no stop cue: the voice ends
    /// by itself in the mixer).
    fn upkeep(&mut self, now: u32) {
        for c in 0..CHANNELS {
            let Some(ch) = &self.channels[c] else {
                continue;
            };
            if ch.looped {
                continue;
            }
            let recorded = self
                .device_ends
                .iter()
                .find(|(t, f, _)| t.abs_diff(ch.start_tick) <= 1 && f.abs_diff(ch.tail_frames) <= 4)
                .map(|&(_, _, e)| e);
            let over = match recorded {
                Some(end) => now.wrapping_sub(end) < u32::MAX / 2,
                None => ch.offset_frames + self.played_frames(ch, now) >= ch.frames,
            };
            if !over {
                continue;
            }
            let s = ch.request;
            let resume = self.resume_position(c, now);
            self.channels[c] = None;
            if let Some(r) = self.slots[s].as_mut() {
                r.channel = None;
                r.state = RequestState::Ended;
                r.resume_offset = resume;
            }
        }
    }

    // ------------------------------------------------------------ volume and pan

    /// The volume chain of request `s` with its current id's record: (the
    /// volume after the settings, ducks and mod gains, the volume sent,
    /// pan). Mixer mode 0 only (positional bias,
    /// [`super::volume::positional_bias`], is not used).
    fn compute(&self, s: usize, stereo: bool) -> (i32, i32, i32) {
        let r = self.req(s);
        let Some(e) = self.table.get(r.id) else {
            return (0, 0, 128);
        };
        let exempt = (1..=15).contains(&r.id) || (52..=71).contains(&r.id) || is_music_path(r.id);
        let mut pre = chain(
            r.volume,
            &self.settings,
            &ChainInputs {
                music_vol: e.row.music_vol != 0,
                state_duck_applies: !exempt,
                solo: e.row.solo != 0,
                state_duck: self.state_duck,
                solo_duck: self.solo_duck,
            },
        );
        if self.mods.has_audio_gains() {
            pre = self.mods.sound_gain(self.category(r.id), r.id, pre);
        }
        let mut v = linear_falloff(pre, r.pos[0], r.pos[1], e.row.falloff);
        v = record_volume(v, e.row.volume);
        let mut pan = 128;
        if !stereo {
            let (gain, p) = mode0_gain_pan(r.pos);
            v = gain.wrapping_mul(v) / 255;
            pan = p;
        }
        (pre, v, pan)
    }

    /// The send for a playing request: send unless unchanged; the
    /// occlusion goes to the device with it.
    fn send(&mut self, queue: &mut TriggerQueue, now: u32, s: usize) {
        let Some(c) = self.req(s).channel else { return };
        let stereo = self.channels[c].as_ref().is_some_and(|ch| ch.stereo);
        let (pre, vol, pan) = self.compute(s, stereo);
        let r = self.req(s);
        let key = (pre, r.occlusion, r.pos);
        let (handle, occlusion) = (r.handle, r.occlusion);
        let Some(ch) = self.channels[c].as_mut() else {
            return;
        };
        if ch.last_sent == Some(key) {
            return;
        }
        ch.last_sent = Some(key);
        ch.vol = vol;
        if !stereo {
            ch.pan = pan;
        }
        if ch.device_occlusion != occlusion {
            ch.device_occlusion = occlusion;
            queue.push(Cue::Device(DeviceChange {
                tick: now,
                target: ch.cue,
                start_frame: None,
                loop_start: None,
                loop_end: None,
                occlusion: occlusion.to_bits(),
            }));
        }
        queue.push(Cue::Param(ParamChange {
            tick: now,
            target: ch.cue,
            vol,
            pan: ch.pan,
            cause: format!("update h{handle}"),
        }));
    }
}

/// The occlusion step arithmetic: `occ` moves toward
/// the target `t` by at most `s` = f32 0.05, each operation rounded to f32.
pub fn occlusion_step(occ: f32, t: f32) -> f32 {
    if occ < t {
        (occ + OCCLUSION_STEP).min(t)
    } else {
        (occ - OCCLUSION_STEP).max(t)
    }
}

/// Distance² for ordering and range: each axis clamped to ±2,000, z not in
/// it.
fn dist2(x: f32, y: f32) -> f32 {
    let cx = x.clamp(-AXIS_CLAMP, AXIS_CLAMP);
    let cy = y.clamp(-AXIS_CLAMP, AXIS_CLAMP);
    cx * cx + cy * cy
}

/// The river point of 2599 `object_river`: with P = (a,
/// b) and U = (c, e), t = f32((b − 2a − c/2 − e) / −2.5); U' = (trunc(t),
/// trunc(f32(b − 2·(a − t)))): the foot of P on the slope −1/2 line
/// through U.
pub fn river_point(p: (i32, i32), u: (i32, i32)) -> (i32, i32) {
    let (a, b) = (f64::from(p.0), f64::from(p.1));
    let (c, e) = (f64::from(u.0), f64::from(u.1));
    let t = ((b + -2.0 * a - c * 0.5 - e) / -2.5) as f32;
    let y = (b - 2.0 * (a - f64::from(t))) as f32;
    (ftol(t), ftol(y))
}

/// [`SoundSystem`] with a world: the [`SoundCalls`] surface the trigger
/// and environment rules use.
pub struct SoundCtx<'a> {
    pub sys: &'a mut SoundSystem,
    pub world: &'a mut dyn SoundWorld,
}

impl SoundCtx<'_> {
    /// One sound tick.
    pub fn run_tick(&mut self, queue: &mut TriggerQueue) {
        self.sys.run_tick(self.world, queue);
    }
}

impl SoundCalls for SoundCtx<'_> {
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.sys.rate
    }

    fn request(
        &mut self,
        id: i32,
        unit: Option<UnitKey>,
        delay: u32,
        flags: u32,
        offset: u32,
    ) -> Handle {
        self.sys.request(self.world, id, unit, delay, flags, offset)
    }

    fn set_volume(&mut self, h: Handle, v: i32) {
        self.sys.set_volume(h, v);
    }

    fn fade(&mut self, h: Handle, target: i32, delay: u32, len: u32) {
        self.sys.fade(h, target, delay, len);
    }

    fn detach(&mut self, h: Handle, unit: UnitKey, force: bool) {
        self.sys.detach(h, unit, force);
    }

    fn stop_handle(&mut self, h: Handle) {
        self.sys.stop_handle(h);
    }

    fn stop_id(&mut self, id: i32) {
        self.sys.stop_id(id);
    }

    fn stop_songs(&mut self) {
        self.sys.stop_songs();
    }

    fn stop_52_71_except(&mut self, a: i32, b: i32) {
        self.sys.stop_range_except(52, 71, a, b);
    }

    fn stop_72_201_except(&mut self, a: i32, b: i32) {
        self.sys.stop_range_except(72, 201, a, b);
    }

    fn stop_speech(&mut self) {
        self.sys.stop_speech();
    }

    fn speaking(&self, unit: UnitKey) -> bool {
        self.sys.speaking(unit)
    }

    fn any_speech(&self) -> bool {
        self.sys.any_speech()
    }

    fn is_active(&self, h: Handle) -> bool {
        self.sys.is_active(h)
    }

    fn sound_tick(&self) -> u32 {
        self.sys.tick()
    }

    fn roll(&mut self, n: i32) -> u32 {
        SoundSystem::roll(self.world, n)
    }
}

impl SoundSystem {
    /// A request of this id exists.
    pub fn id_requested(&self, id: i32) -> bool {
        self.requests().any(|r| r.id == id)
    }

    /// A request with an id in 4,657–4,698 is not ended.
    pub fn music_active(&self) -> bool {
        self.requests()
            .any(|r| is_music_path(r.id) && r.state != RequestState::Ended)
    }

    /// The play position of the request with id `id`:
    /// the first request of the active
    /// list whose **current** id is `id`, if it is playing; the position
    /// is the device's, in 4-byte units (a frame of a 16-bit
    /// stereo song; 0 for a non-stream voice). A request without a channel
    /// has none (never asked).
    pub fn play_position(&self, id: i32) -> Option<u32> {
        let r = self.requests().find(|r| r.id == id)?;
        if r.state != RequestState::Playing {
            return None;
        }
        Some(self.resume_position(r.channel?, self.tick))
    }

    /// Sets request `h`'s position: position := (x, y, z + 640) and
    /// distance² recomputed from x, y; no unit is written.
    pub fn set_position(&mut self, h: Handle, x: i32, y: i32, z: i32) {
        if let Some(s) = self.handle_slot(h) {
            let (fx, fy) = (x as f32, y as f32);
            let r = self.req_mut(s);
            r.pos = [fx, fy, (f64::from(z) + f64::from(UNIT_Z)) as f32];
            r.dist2 = dist2(fx, fy);
        }
    }

    /// The units with a non-empty request list (U +0x78).
    pub fn attached_units(&self) -> impl Iterator<Item = UnitKey> + '_ {
        self.unit_lists
            .iter()
            .filter(|(_, l)| !l.is_empty())
            .map(|(&k, _)| k)
    }

    /// `unit`'s request list,
    /// newest first, as (handle, current id): waiting, playing and
    /// ended-not-yet-freed requests; a handle whose request was freed
    /// reads id 0.
    pub fn unit_requests(&self, unit: UnitKey) -> Vec<(Handle, i32)> {
        self.unit_lists.get(&unit).map_or_else(Vec::new, |l| {
            l.iter()
                .map(|&h| (h, self.request_by_handle(h).map_or(0, |r| r.id)))
                .collect()
        })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

impl crate::audio::environment::EnvCalls for SoundCtx<'_> {
    fn id_requested(&self, id: i32) -> bool {
        self.sys.id_requested(id)
    }

    fn music_active(&self) -> bool {
        self.sys.music_active()
    }

    fn play_position(&self, id: i32) -> Option<u32> {
        self.sys.play_position(id)
    }

    fn blocks(&self, id: i32) -> [i32; 3] {
        self.sys.table.get(id).map_or([-1; 3], |e| e.row.blocks)
    }

    fn volume(&self, h: Handle) -> Option<i32> {
        self.sys.request_by_handle(h).map(|r| r.volume)
    }

    fn set_position(&mut self, h: Handle, x: i32, y: i32, z: i32) {
        self.sys.set_position(h, x, y, z);
    }
}

impl crate::audio::triggers::TriggerSound for SoundCtx<'_> {
    fn sound_on(&self) -> bool {
        self.sys.enabled
    }

    fn group_base(&self, id: i32) -> i32 {
        self.sys.table.base(id)
    }

    fn group_size(&self, base: i32) -> i32 {
        self.sys
            .table
            .get(base)
            .map_or(0, |e| i32::from(e.group_size))
    }

    fn looping(&self, id: i32) -> bool {
        self.sys.table.get(id).is_some_and(|e| e.row.looped != 0)
    }

    fn unit_requests(&self, unit: UnitKey) -> Vec<(Handle, i32)> {
        self.sys.unit_requests(unit)
    }

    fn unit_count(&self, h: Handle) -> usize {
        self.sys.request_by_handle(h).map_or(0, |r| r.units.len())
    }

    fn variant(&mut self, id: i32) -> i32 {
        // No sound table without the sound init (`--no-sound`): the pick
        // returns `id` with no draw.
        if !self.sys.enabled {
            return id;
        }
        let world = &mut *self.world;
        self.sys
            .table
            .pick_variant(id, &mut |n| SoundSystem::roll(world, n))
    }

    fn request_volume(&self, h: Handle) -> Option<i32> {
        self.sys.request_by_handle(h).map(|r| r.volume)
    }
}
