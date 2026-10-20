//! The play mode's audio: the audio core ([`AudioEngine`]) reading its
//! samples through the sound pool ([`SoundPool`]: each
//! file read and decoded once, kept under the `sounds` budget), driven
//! once per Bevy frame after the bridge frame: the pool
//! starts the frame, the cue source's cues are queued, and the frame's
//! server tick is presented. The output edge
//! ([`crate::audio::output`]) mixes on the audio thread when the app has
//! an audio device.
//!
//! Plumbing only; the game's sound rules sit behind hooks. With a sound table
//! ([`AudioParts::original`]) the cues come from the sound layer
//! ([`SoundDriver`]: requests from the game's sound triggers, channels,
//! stealing and volume / pan), sound ids map to files by the sound table
//! ([`SoundTable`]), the WAV decode is [`D2Wav`] and the gain curve is
//! [`DeviceGain`]. [`AudioParts::unspecified`] keeps every hook at its
//! neutral implementation ([`NoCues`], [`NoSoundTable`], [`NoWavDecoder`],
//! [`UnityGain`], [`Unlimited`]): the app then starts no voice.

use std::sync::{Arc, Mutex};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use d2_data::bin::TableFiles;
use d2_data::txt::TxtTable;

use crate::app::config::{ConfigRes, Settings};
use crate::assets::cache::{Budgets, CacheEvent};
use crate::assets::path::{CanonicalPath, FileSource, MemorySource};
use crate::audio::driver::{DriverError, SoundDriver, SoundLink};
use crate::audio::output::{self, MixerStream};
use crate::audio::sound_table::{DeviceGain, SoundSettings, SoundSystem, SoundTableData};
use crate::audio::D2Wav;
use crate::audio::{
    AudioEngine, AudioError, CueSource, GainCurve, Sound, SoundBank, SoundId, SoundPool,
    SoundPoolError, TriggerQueue, UnityGain, Unlimited, VoicePolicy, WavDecoder,
};
use crate::bridge::BridgeResource;
use crate::world_view::present::FrameSchedule;
use crate::world_view::walk::PreviewWalk;
use crate::world_view::UiSounds;

/// Sound ids to archive paths: the `sounds.txt` mapping of
/// the sound table (id = data-line index; path prefix by id
/// range, then `FileName`); variants are picked by the sound layer before an id
/// reaches this hook. The implementation is
/// [`crate::audio::sound_table::SoundPaths`].
pub trait SoundTable: Send + Sync {
    fn file(&self, id: SoundId) -> Option<Arc<str>>;
}

/// No sound table (the hook-only play mode): no id names a file.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSoundTable;

impl SoundTable for NoSoundTable {
    fn file(&self, _: SoundId) -> Option<Arc<str>> {
        None
    }
}

/// No WAV decoder (the hook-only play mode): every decode is an error. The
/// decoder of the game's WAV files is [`D2Wav`].
#[derive(Debug, Clone, Copy, Default)]
pub struct NoWavDecoder;

impl WavDecoder for NoWavDecoder {
    fn decode(&self, _: &CanonicalPath, _: &[u8]) -> Result<Sound, String> {
        Err("no WAV decoder in this play mode (formats/wav.md §5 is D2Wav)".into())
    }
}

/// No cue source (the hook-only play mode): no event makes a sound.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCues;

impl CueSource for NoCues {
    fn drain_cues(&mut self, _: &mut TriggerQueue) {}
}

/// The engine's [`SoundBank`]: the table's file, loaded through the pool.
/// A failed load is kept for the frame's report and answers `None` (the
/// engine logs the start as failed).
pub struct PoolBank {
    table: Box<dyn SoundTable>,
    pool: Arc<Mutex<SoundPool>>,
    errors: Arc<Mutex<Vec<SoundPoolError>>>,
}

impl SoundBank for PoolBank {
    fn file(&self, id: SoundId) -> Option<Arc<str>> {
        self.table.file(id)
    }

    /// The file's byte size as read, charged by the sample cache.
    fn file_size(&self, id: SoundId) -> Option<u64> {
        let file = self.table.file(id)?;
        self.pool.lock().ok()?.file_size(&file)
    }

    fn samples(&self, id: SoundId) -> Option<Arc<Sound>> {
        let file = self.table.file(id)?;
        let loaded = self.pool.lock().ok()?.load(&file);
        match loaded {
            Ok(sound) => Some(sound),
            Err(e) => {
                if let Ok(mut errors) = self.errors.lock() {
                    errors.push(e);
                }
                None
            }
        }
    }
}

/// The parts of the play mode's audio; each hook defaults to its neutral
/// implementation.
pub struct AudioParts {
    pub source: Arc<dyn FileSource>,
    pub decoder: Box<dyn WavDecoder>,
    pub table: Box<dyn SoundTable>,
    pub cues: Box<dyn CueSource + Send + Sync>,
    pub gain: Box<dyn GainCurve + Send>,
    pub policy: Box<dyn VoicePolicy + Send>,
    /// The `sounds` pool budget.
    pub budget: u64,
    /// The sound table: with it the cues come from the
    /// game's sound layer ([`SoundDriver`]) instead of `cues`.
    pub sounds: Option<SoundTableData>,
}

impl AudioParts {
    /// Every hook at its neutral implementation over `source` (the user's archives,
    /// or an empty source without game files).
    pub fn unspecified(source: Arc<dyn FileSource>) -> Self {
        AudioParts {
            source,
            decoder: Box::new(NoWavDecoder),
            table: Box::new(NoSoundTable),
            cues: Box::new(NoCues),
            gain: Box::new(UnityGain),
            policy: Box::new(Unlimited),
            budget: Budgets::default().sounds,
            sounds: None,
        }
    }

    /// The game's audio over `source`: the
    /// sound table's paths, the WAV decoder ([`D2Wav`]), the sound
    /// layer as the cue source (channels and stealing are its own, so the
    /// core admits every start, [`Unlimited`]) and the device gain curve
    /// ([`DeviceGain`]).
    pub fn original(source: Arc<dyn FileSource>, table: SoundTableData) -> Self {
        AudioParts {
            source,
            decoder: Box::new(D2Wav),
            table: Box::new(table.paths()),
            cues: Box::new(NoCues),
            gain: Box::new(DeviceGain),
            policy: Box::new(Unlimited),
            budget: Budgets::default().sounds,
            sounds: Some(table),
        }
    }

    /// [`AudioParts::unspecified`] over an empty source.
    pub fn empty() -> Self {
        Self::unspecified(Arc::new(MemorySource::default()))
    }
}

/// What the last audio frame did, for logs and tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioStats {
    /// Bridge frame of the last pool frame.
    pub frame: u64,
    /// Tick presented last (the sound tick when the sound layer runs,
    /// else the server tick).
    pub presented: u32,
    /// Files decoded so far.
    pub decodes: u64,
    /// Sound loads that failed, and engine errors, so far.
    pub load_errors: usize,
    pub engine_errors: usize,
    /// The sound layer's pending model questions seen so far, each once,
    /// in first-seen order (`SoundDriver::take_pending`).
    pub pending: Vec<&'static str>,
}

/// The play mode's audio state.
#[derive(Resource)]
pub struct GameAudio {
    /// Shared with the output edge, which mixes from it.
    pub engine: Arc<Mutex<AudioEngine>>,
    pub pool: Arc<Mutex<SoundPool>>,
    cues: Box<dyn CueSource + Send + Sync>,
    /// The game's sound layer, when the parts carry a sound table
    /// (shared with the thunder step through [`SoundLink`]).
    pub driver: Option<Arc<Mutex<SoundDriver>>>,
    errors: Arc<Mutex<Vec<SoundPoolError>>>,
    pub stats: AudioStats,
}

impl GameAudio {
    pub fn new(parts: AudioParts) -> Self {
        let pool = Arc::new(Mutex::new(SoundPool::new(
            parts.source,
            parts.decoder,
            parts.budget,
        )));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let bank = PoolBank {
            table: parts.table,
            pool: Arc::clone(&pool),
            errors: Arc::clone(&errors),
        };
        let driver = parts.sounds.map(|table| {
            let paths = table.paths();
            let bank = PoolBank {
                table: Box::new(paths),
                pool: Arc::clone(&pool),
                errors: Arc::clone(&errors),
            };
            Arc::new(Mutex::new(SoundDriver::new(SoundSystem::new(
                table,
                Box::new(bank),
            ))))
        });
        GameAudio {
            engine: Arc::new(Mutex::new(AudioEngine::new(
                Box::new(bank),
                parts.gain,
                parts.policy,
            ))),
            pool,
            cues: parts.cues,
            driver,
            errors,
            stats: AudioStats::default(),
        }
    }
}

/// Errors of an audio frame (Bevy's error handler reports them).
#[derive(Debug, thiserror::Error)]
pub enum AudioFrameError {
    #[error("audio state poisoned (the audio thread panicked)")]
    Poisoned,
    #[error("server tick {0} does not fit the audio tick")]
    Tick(u64),
    #[error(transparent)]
    Cache(#[from] crate::assets::cache::CacheError),
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Driver(#[from] DriverError),
}

/// Adds the audio frame (after the bridge frame) and a [`GameAudio`] with
/// `parts`. A later `insert_resource(GameAudio::new(…))` replaces it.
pub fn add_audio(app: &mut impl super::session::Setup, parts: AudioParts) {
    app.register("audio", register_audio);
    let w = app.world_mut();
    w.insert_resource(GameAudio::new(parts));
    w.init_resource::<SoundLink>();
}

pub fn register_audio(app: &mut App) {
    app.add_systems(
        PostUpdate,
        audio_frame
            .run_if(resource_exists::<BridgeResource>)
            .run_if(resource_exists::<GameAudio>),
    );
}

/// Plays the mixed stream on the app's audio device. Needs Bevy's audio
/// plugin (the window's `DefaultPlugins`).
pub fn add_output(app: &mut App) {
    super::session::Setup::register(app, "audio-output", |app| {
        output::register(app);
        app.add_systems(
            PostUpdate,
            start_output.run_if(resource_exists_and_changed::<GameAudio>),
        );
    });
}

/// The game's output stream on its [`GameAudio`] (a game's start installs
/// a new one: the old stream goes, `session::end_game`).
fn start_output(
    mut commands: Commands,
    audio: Res<GameAudio>,
    old: Query<Entity, With<GameOutput>>,
    mut streams: ResMut<Assets<MixerStream>>,
) {
    if !audio.is_added() {
        return;
    }
    for e in &old {
        commands.entity(e).despawn();
    }
    let stream = streams.add(MixerStream::new(Arc::clone(&audio.engine)));
    commands.spawn((AudioPlayer(stream), GameOutput, super::session::GameEntity));
}

/// The entity playing the game's mixed stream.
#[derive(Component)]
struct GameOutput;

/// `play --sound-log FILE`: every
/// sound request call, one row each, tagged with the server tick of the
/// audio frame that made it.
#[derive(Resource)]
pub struct SoundLog {
    out: std::io::BufWriter<std::fs::File>,
}

/// The log's header lines (format version 1).
pub const SOUND_LOG_HEADER: &str =
    "# sound-log v1; d2-client play --sound-log\ntick\tsound_tick\tid\tunit_type\tguid\tdelay\tflags\toffset\n";

impl SoundLog {
    pub fn create(path: &std::path::Path) -> std::io::Result<Self> {
        use std::io::Write;
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        out.write_all(SOUND_LOG_HEADER.as_bytes())?;
        out.flush()?;
        Ok(SoundLog { out })
    }

    /// One row per call; `-` for no unit.
    pub fn write(
        &mut self,
        tick: u64,
        calls: &[crate::audio::sound_table::system::RequestCall],
    ) -> std::io::Result<()> {
        use std::io::Write;
        for c in calls {
            let (t, g) = c.unit.map_or(("-".to_owned(), "-".to_owned()), |u| {
                (u.unit_type.to_string(), u.guid.to_string())
            });
            writeln!(
                self.out,
                "{tick}\t{}\t{}\t{t}\t{g}\t{}\t{}\t{}",
                c.sound_tick, c.id, c.delay, c.flags, c.offset
            )?;
        }
        self.out.flush()
    }
}

/// `play --device-ends FILE`: the recorded ends of one-shots, handed to the
/// sound system at the first audio frame.
#[derive(bevy::prelude::Resource)]
pub struct DeviceEnds(pub Option<Vec<(u32, u64, u32)>>);

impl DeviceEnds {
    /// `# device-ends 1` text: `tick\tframes\tend` rows.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut rows = Vec::new();
        let mut version = false;
        for (n, line) in text.lines().enumerate() {
            if line.starts_with("# device-ends 1") {
                version = true;
                continue;
            }
            if line.starts_with('#') || line.starts_with("tick") || line.trim().is_empty() {
                continue;
            }
            let c: Vec<&str> = line.split('\t').collect();
            let num = |i: usize| c.get(i).and_then(|v| v.parse::<u64>().ok());
            match (num(0), num(1), num(2)) {
                (Some(t), Some(f), Some(e)) => rows.push((t as u32, f, e as u32)),
                _ => return Err(format!("device ends line {}: tick, frames, end", n + 1)),
            }
        }
        if !version {
            return Err("device ends: no `# device-ends 1` line".into());
        }
        Ok(Self(Some(rows)))
    }
}

/// `play --async-collects FILE`: the recorded preload passes that collected a
/// finished async read, handed to the sound system at the first audio frame.
#[derive(bevy::prelude::Resource)]
pub struct AsyncCollects(pub Option<Vec<(u32, i32)>>);

impl AsyncCollects {
    /// `# async-collects 1` text: `tick\tid` rows.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut rows = Vec::new();
        let mut version = false;
        for (n, line) in text.lines().enumerate() {
            if line.starts_with("# async-collects 1") {
                version = true;
                continue;
            }
            if line.starts_with('#') || line.starts_with("tick") || line.trim().is_empty() {
                continue;
            }
            let c: Vec<&str> = line.split('\t').collect();
            match (
                c.first().and_then(|v| v.parse::<u32>().ok()),
                c.get(1).and_then(|v| v.parse::<i32>().ok()),
            ) {
                (Some(t), Some(id)) => rows.push((t, id)),
                _ => return Err(format!("async collects line {}: tick, id", n + 1)),
            }
        }
        if !version {
            return Err("async collects: no `# async-collects 1` line".into());
        }
        Ok(Self(Some(rows)))
    }
}

/// The driver's inputs from the app for this pass, read by both of its
/// parts (the client update and the sound tick).
fn driver_inputs(
    d: &mut SoundDriver,
    config: Option<&ConfigRes>,
    schedule: Option<&FrameSchedule>,
    walk: Option<&PreviewWalk>,
    bridge: &BridgeResource,
    tick: u32,
    log: bool,
) {
    let world = bridge.0.world();
    // The options the menu wrote. They are applied
    // before the audio frame, so the voices this frame starts already use
    // them.
    if let Some(c) = config {
        let s = sound_settings(&c.settings, *d.system().settings());
        d.set_settings(s);
    }
    // A check's input script under a recorded frame schedule reaches the
    // model one pass late on a drawn tick (`present::script_tick`): the
    // skill-start hold is that delay already.
    d.set_click_late(schedule.is_some_and(|s| s.started && s.drawn(u64::from(tick))));
    // The listener is where the player is drawn.
    d.set_local_prediction(walk.and_then(PreviewWalk::local_at));
    let local_mode = walk.and_then(|w| w.sound_mode(world));
    d.set_local_mode(local_mode);
    // The listener of the units the unit pass runs after the local
    // player.
    d.set_listener_push(crate::world_view::walk::local_push_out(
        world,
        &bridge.0.inputs().objclient.rows,
        walk.and_then(PreviewWalk::local_at),
        local_mode,
    ));
    if log {
        d.log_requests(true);
    }
}

/// The sound layer as the world view's pass reaches it: the UI's sound
/// requests and the driver, for the client update's sounds.
#[derive(SystemParam)]
pub struct FrameSounds<'w> {
    /// The pass' sound requests ([`UiSounds`]).
    pub ui: Option<ResMut<'w, UiSounds>>,
    audio: Option<Res<'w, GameAudio>>,
    link: Option<Res<'w, SoundLink>>,
    config: Option<Res<'w, ConfigRes>>,
    log: Option<Res<'w, SoundLog>>,
}

impl FrameSounds<'_> {
    /// The pass' client update sounds (`SoundDriver::client_update`): the
    /// requests so far and the unit pass, whose rolls draw on the client seed
    /// before the pass' weather update (client update, then the in-game draw).
    /// The audio frame then runs the sound ticks. No-op without a sound layer.
    pub fn client_update(
        &mut self,
        bridge: &BridgeResource,
        walk: Option<&PreviewWalk>,
        schedule: Option<&FrameSchedule>,
    ) -> std::result::Result<(), BevyError> {
        let Some(driver) = self.audio.as_deref().and_then(|a| a.driver.as_ref()) else {
            return Ok(());
        };
        if let Some(l) = self.link.as_deref() {
            l.set(Some(driver));
        }
        let world = bridge.0.world();
        let tick = u32::try_from(world.server_ticks)
            .map_err(|_| AudioFrameError::Tick(world.server_ticks))?;
        let mut d = driver.lock().map_err(|_| AudioFrameError::Poisoned)?;
        // The tick's client update ran on an earlier pass whose sound tick
        // waits for the tick's draw ([`audio_frame`]): its requests since
        // are the sound tick's late ones.
        if d.update_pending() {
            return Ok(());
        }
        let requests = self.ui.as_deref_mut().map(|s| std::mem::take(&mut s.0));
        driver_inputs(
            &mut d,
            self.config.as_deref(),
            schedule,
            walk,
            bridge,
            tick,
            self.log.is_some(),
        );
        d.client_update(
            world,
            &bridge.0.inputs().tables.levels,
            requests.as_deref().unwrap_or_default(),
        )
        .map_err(AudioFrameError::from)?;
        Ok(())
    }
}

/// One audio frame: pool frame, the sound layer's ticks (when it runs:
/// the pass' client update first, unless the world view ran it before its
/// weather step, [`FrameSounds::client_update`]), cues, present the tick
/// (the sound tick with the sound layer, else the frame's server tick).
#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn audio_frame(
    bridge: Res<BridgeResource>,
    mut audio: ResMut<GameAudio>,
    ui_sounds: Option<ResMut<UiSounds>>,
    walk: Option<Res<PreviewWalk>>,
    config: Option<Res<ConfigRes>>,
    link: Option<Res<SoundLink>>,
    mut log: Option<ResMut<SoundLog>>,
    mut dump: Option<ResMut<super::audio_dump::AudioDump>>,
    schedule: Option<Res<FrameSchedule>>,
    mut device_ends: Option<ResMut<DeviceEnds>>,
    mut async_collects: Option<ResMut<AsyncCollects>>,
    mut exit: MessageWriter<AppExit>,
    drawn: Option<Res<crate::bridge::mirror::DrawnTick>>,
) -> std::result::Result<(), BevyError> {
    // When the bridge waits for the draws (`DrawnTick`), a pass that has not
    // drawn its tick yet (the GPU still busy) runs no sound tick: the pass that
    // draws it runs the weather update, and the sound tick follows the
    // in-game draw (the rain rule reads that update).
    if drawn.is_some_and(|d| bridge.0.world().server_ticks > d.0) {
        return Ok(());
    }
    // A game started in the running app (`app::session`) is installed
    // after this pass' bridge frame: its first audio frame follows its
    // first bridge frame.
    if bridge.0.world().frames == 0 {
        return Ok(());
    }
    let audio = &mut *audio;
    // The rain rule reads this pass' weather: the in-game draw (weather
    // update) runs before the sound tick in a client loop pass.
    let weather = link.as_deref().map(|l| {
        l.set(audio.driver.as_ref());
        l.weather()
    });
    let world = bridge.0.world();
    let tick =
        u32::try_from(world.server_ticks).map_err(|_| AudioFrameError::Tick(world.server_ticks))?;
    let decodes = {
        let mut pool = audio.pool.lock().map_err(|_| AudioFrameError::Poisoned)?;
        pool.begin_frame(world.frames)
            .map_err(AudioFrameError::from)?;
        for e in pool.drain_events() {
            if let CacheEvent::Evicted { key, .. } = &e {
                debug!("sound pool evicted {key}");
            } else {
                warn!("sound pool: {e:?}");
            }
        }
        pool.decodes()
    };
    let requests = ui_sounds.map(|mut s| std::mem::take(&mut s.0));
    let mut driver = match &audio.driver {
        Some(d) => Some(d.lock().map_err(|_| AudioFrameError::Poisoned)?),
        None => None,
    };
    if let Some(d) = driver.as_deref_mut() {
        driver_inputs(
            d,
            config.as_deref(),
            schedule.as_deref(),
            walk.as_deref(),
            &bridge,
            tick,
            log.is_some(),
        );
        if let Some(w) = weather {
            d.set_weather(w);
        }
        if let Some(ends) = device_ends.as_deref_mut().and_then(|e| e.0.take()) {
            d.set_device_ends(ends);
        }
        // A paused pass (the Esc menu or options
        // open) runs one sound tick and no client update.
        d.set_paused(bridge.0.is_paused());
        if let Some(c) = async_collects.as_deref_mut().and_then(|c| c.0.take()) {
            d.set_async_collects(c);
        }
        d.frame(
            world,
            &bridge.0.inputs().tables.levels,
            requests.as_deref().unwrap_or_default(),
        )
        .map_err(AudioFrameError::from)?;
        if let Some(l) = log.as_deref_mut() {
            if let Err(e) = l.write(world.server_ticks, &d.take_request_log()) {
                warn!("sound log: {e}");
            }
        }
        // A model question the sound layer could not answer is not an
        // error: answered neutrally, logged
        // once per question.
        for q in d.take_pending() {
            if !audio.stats.pending.contains(&q) {
                warn!("sound layer pending: {q}");
                audio.stats.pending.push(q);
            }
        }
        for s in d.take_skipped() {
            debug!("sound layer skipped: {s}");
        }
        for e in d.take_errors() {
            warn!("sound layer: {e}");
        }
    }
    let (presented, engine_errors) = {
        let mut engine = audio.engine.lock().map_err(|_| AudioFrameError::Poisoned)?;
        let presented = match driver.as_deref_mut() {
            Some(d) => {
                engine.pump(d);
                // The last sound tick that ran: the driver's counter is
                // the next T. None ran
                // yet: the first server tick has no sound tick.
                match d.tick().checked_sub(1) {
                    Some(t) => t,
                    None => return Ok(()),
                }
            }
            None => {
                engine.pump(audio.cues.as_mut());
                tick
            }
        };
        match dump.as_deref_mut() {
            // The dump presents each tick
            // and mixes its frames (no device).
            Some(d) => {
                let records = d
                    .stepper
                    .advance(&mut engine, presented)
                    .map_err(AudioFrameError::from)?;
                if let Err(e) = d.write(&records, world.server_ticks) {
                    warn!("audio dump: {e}");
                }
                if d.until.is_some_and(|n| world.server_ticks >= n) {
                    info!(
                        "play: audio dump reached server tick {}",
                        world.server_ticks
                    );
                    exit.write(AppExit::Success);
                }
            }
            None => engine.present(presented).map_err(AudioFrameError::from)?,
        }
        (presented, engine.take_errors())
    };
    drop(driver);
    for e in &engine_errors {
        warn!("audio: {e}");
    }
    let load_errors =
        std::mem::take(&mut *audio.errors.lock().map_err(|_| AudioFrameError::Poisoned)?);
    for e in &load_errors {
        warn!("sound load: {e}");
    }
    let s = &mut audio.stats;
    s.frame = world.frames;
    s.presented = presented;
    s.decodes = decodes;
    s.load_errors += load_errors.len();
    s.engine_errors += engine_errors.len();
    Ok(())
}

/// The sound layer's settings from the app's (kept as the game's
/// integers): mixer mode 0 (the only one D2RUST has), `Master
/// Volume`, `Music Volume`, `Positional Bias`, `NPC Speech`; `Options Music`
/// and the game-loaded flag are not app settings and keep `prev`'s.
pub fn sound_settings(s: &Settings, prev: SoundSettings) -> SoundSettings {
    SoundSettings {
        mixer_mode: 0,
        master_volume: i32::from(s.master_volume),
        music_volume: i32::from(s.music_volume),
        positional_bias: i32::from(s.positional_bias),
        npc_speech: i32::from(s.npc_speech),
        ..prev
    }
}

/// The user's `sounds.txt` and `soundenviron.txt`, read from the archives and
/// compiled; a missing or bad table is an error, never a fallback.
pub fn sound_table_live(archives: &dyn TableFiles) -> Result<SoundTableData, String> {
    let txt = |f: &str| -> Result<TxtTable, String> {
        let (_, b) = archives
            .read_excel(f)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{f}: in no archive"))?;
        TxtTable::parse(f, &b).map_err(|e| e.to_string())
    };
    SoundTableData::from_txt(&txt("sounds.txt")?, &txt("soundenviron.txt")?)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod device_ends_tests {
    use super::DeviceEnds;

    #[test]
    fn device_ends_parse() {
        let e = DeviceEnds::parse("# device-ends 1\ntick\tframes\tend\n64\t32896\t70\n").unwrap();
        assert_eq!(e.0, Some(vec![(64, 32896, 70)]));
        assert!(DeviceEnds::parse("64\t1\t2\n").is_err());
        assert!(DeviceEnds::parse("# device-ends 1\n64\tx\t2\n").is_err());
    }

    #[test]
    fn async_collects_parse() {
        use super::AsyncCollects;
        let c = AsyncCollects::parse("# async-collects 1\ntick\tid\n100\t1834\n").unwrap();
        assert_eq!(c.0, Some(vec![(100, 1834)]));
        assert_eq!(
            AsyncCollects::parse("# async-collects 1\n").unwrap().0,
            Some(vec![])
        );
        assert!(AsyncCollects::parse("100\t1\n").is_err());
        assert!(AsyncCollects::parse("# async-collects 1\n100\tx\n").is_err());
    }
}
