//! `d2-client play`: a window running the local single-player game.
//!
//! Each Bevy frame runs the bridge frame in `PreUpdate` (it pumps the
//! in-process server: drain the client's messages, run due ticks, flush
//! the replies; then it receives and dispatches them into the client world
//! model), then the world view in `Update`: the draw list built from the
//! client world model, composed by the GPU compositor's render-graph node
//! and presented in the window.
//!
//! The server's messages fill the client world model, and every rule of
//! how a unit, tile or panel looks is a world-view hook. [`add_game`] alone
//! answers them with `world_view::Unspecified` (draw nothing). Placement
//! goes through `rules::OriginalView` with the game camera, fed by
//! `world_view::ModelFeed`: the local player's position, unit positions
//! and BlankScreen from the model, and the open panel mode of the game UI
//! (0 when none is open). Without a player in the model there is no camera
//! and nothing is placed. The frame palette is the act's `pal.pl2` for the
//! model's palette act, presented by [`super::palette`] when the player's
//! archives are read; without them it is [`unspecified_palette`].
//!
//! Frames come from the frame store (`ViewAssets::frames`), UI text goes
//! through `ui::text::layout_text` (`world_view::text_sprites`), and the
//! audio core plays from the sound pool after each bridge frame
//! ([`super::sound`]; the player's archives with `D2_GAME_DIR`).
//!
//! [`run`] turns the play preview on ([`add_preview`]): the feed builds the
//! map from the client DRLG, its DT1 tiles are read from the player's
//! archives (on synthetic data every tile is skipped and logged), and the
//! inputs the model lacks get the labelled fills of `world_view::preview`.
//!
//! With the player's files ([`run`] on live data) the game UI is added
//! ([`super::ui`]: the panels in the world view's root, hotkeys from the
//! bindings, panel art from the archives, the open panel mode feeding the
//! camera) and the audio runs the game's sound layer
//! (`AudioParts::original`: `sounds.txt`, the WAV decoder, one sound tick
//! per server tick, the UI's sound requests).
//!
//! `play --offscreen` runs the same game in a hidden window that never
//! takes focus, and can write shots of the presented picture
//! (`world_view::shots`). With `--display-clock` the frame count drives
//! the clock instead of the wall clock, so every run draws the same
//! frames (`world_view::display_clock`). `play --control` drives it over
//! a line channel ([`super::control`]).

use bevy::prelude::*;
use d2_formats::palette::{Palette, Rgb};

use d2_server::host::SystemClock;

use super::palette::{self, ActPalettes};
use super::save;
use super::session::Setup;
use super::single_player::{self, GameData};
use super::sound::{self, AudioParts, GameAudio};
use super::ui;
use crate::bridge::drlg::DrlgSource;
use crate::bridge::mirror::DynLink;
use crate::bridge::predict::{PredictLink, WalkTap};
use crate::bridge::world::LevelRow;
use crate::bridge::{Bridge, BridgeError, BridgePlugin, BridgeResource};
use crate::world_view::node::NodeRuns;
use crate::world_view::object_label::ObjectLabels;
use crate::world_view::preview::Preview;
use crate::world_view::tile_assets::TileAssets;
use crate::world_view::walk::{add_preview_walk, PreviewWalk};
use crate::world_view::weather_view::WeatherView;
use crate::world_view::{
    ModelFeed, NoFeed, Unspecified, ViewAssets, WorldViewPlugin, WorldViewState,
};

/// Frames between two progress lines in the log.
const LOG_EVERY: u64 = 250;

/// The frame palette before an act palette is presented: all zeros
/// (black). With the player's archives [`super::palette`] replaces it with
/// the `pal.pl2` of the model's palette act; without them (synthetic data)
/// there is no palette file and it stays. Index 0, the only index an empty
/// frame shows, is black in every act palette.
pub fn unspecified_palette() -> Palette {
    Palette {
        colors: [Rgb { r: 0, g: 0, b: 0 }; 256],
    }
}

/// Adds the bridge (on `link`), the world view and the audio frame (every
/// hook at its placeholder, an empty file source) to `app`: the play
/// mode's wiring, shared by the window and the headless tests. Add it
/// after Bevy's render plugin when one is used (the GPU node needs the
/// render world; without one the CPU reference is presented). Inserting
/// a [`WorldViewState`] or [`GameAudio`] afterwards replaces the defaults.
pub fn add_game(app: &mut App, link: DynLink, gpu: bool) -> Result<(), BridgeError> {
    register_game(app, gpu);
    install_game(app, link)
}

/// The play mode's systems ([`add_game`]), once per app: the bridge, the
/// world view (with the GPU node when `gpu`), the progress log, the end
/// of game, the audio frame.
pub fn register_game(app: &mut App, gpu: bool) {
    if !app.is_plugin_added::<BridgePlugin>() {
        app.add_plugins((BridgePlugin, WorldViewPlugin { gpu }))
            .add_systems(
                Last,
                log_progress
                    .run_if(resource_exists::<BridgeResource>)
                    .run_if(resource_exists::<WorldViewState>),
            )
            .add_systems(Update, super::save::end_of_game);
    }
}

/// [`add_game`]'s game: the bridge on `link`, the world view's state,
/// the audio frame's placeholder (the systems are the app's,
/// [`register_game`]).
pub fn install_game(app: &mut impl Setup, link: DynLink) -> Result<(), BridgeError> {
    let mut bridge = Bridge::new(link)?;
    // The client sends its own pings, as a connected client does.
    bridge.set_ping(true);
    let w = app.world_mut();
    w.insert_resource(BridgeResource(bridge));
    w.insert_resource(WorldViewState::new(
        ViewAssets::new(unspecified_palette()),
        Box::new(Unspecified),
        Box::new(ModelFeed::<NoFeed>::default()),
    ));
    sound::add_audio(app, AudioParts::empty());
    // The position check's visibility predicate over no unit art (every
    // unit reads as not visible, the check corrects); `run` installs it
    // again once the game UI's unit art exists.
    super::visibility::add_visibility(app);
    Ok(())
}

/// The local client's C→S 0x67 ([`single_player::create_request`])
/// through the bridge's send path (the system queue),
/// before the first frame: the first pump drains it and the server
/// creates the game. The bridge answers the 0x02 that follows
/// with C→S 0x6B on its own: the join.
pub fn send_create_game(app: &mut impl Setup) -> Result<(), BridgeError> {
    send_create_game_for(app, &single_player::Character::New)
}

/// [`send_create_game`] for `character` (a save's class and name,
/// [`single_player::create_request_for`]).
pub fn send_create_game_for(
    app: &mut impl Setup,
    character: &single_player::Character,
) -> Result<(), BridgeError> {
    send_create_game_flags(app, character, None)
}

/// [`send_create_game_for`] with the 0x67 u32@0x27 the front end computed
/// (`front_start`, `start_flags`).
pub fn send_create_game_flags(
    app: &mut impl Setup,
    character: &single_player::Character,
    flags: Option<u32>,
) -> Result<(), BridgeError> {
    let mut request = single_player::create_request_for(character);
    if let Some(f) = flags {
        request.flags = f;
    }
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .send(&request)?;
    Ok(())
}

/// The client data of the game: the
/// client DRLG's source and the `Levels.txt` rows go to the bridge; the
/// rows also answer BlankScreen in the world view's feed.
pub fn add_client_data(app: &mut impl Setup, drlg: DrlgSource, levels: Vec<LevelRow>) {
    let world = app.world_mut();
    let mut bridge = world.resource_mut::<BridgeResource>();
    bridge.0.set_drlg_source(Some(drlg));
    bridge.0.set_levels(levels.clone());
    let mut state = world.resource_mut::<WorldViewState>();
    state.feed = Box::new(ModelFeed {
        levels: Some(levels),
        ..ModelFeed::<NoFeed>::default()
    });
}

/// Turns the play preview on (`world_view::preview`): the
/// model feed with the map from the client DRLG,
/// the DT1 tiles and act shade tables of `tiles`, and the labelled fills
/// for the inputs the model lacks; a frame that still fails is logged,
/// not fatal.
pub fn add_preview(app: &mut impl Setup, levels: Vec<LevelRow>, tiles: TileAssets) {
    add_preview_lit(app, levels, tiles, None);
}

/// [`add_preview`] with the monster / missile light columns of the tables
/// (`world_view::light_sources`).
pub fn add_preview_lit(
    app: &mut impl Setup,
    levels: Vec<LevelRow>,
    tiles: TileAssets,
    lights: Option<crate::world_view::light_sources::LightRows>,
) {
    add_preview_shifted(app, levels, tiles, lights, None);
}

/// [`add_preview_lit`] with the monster palette shifts
/// (`world_view::palshift`).
pub fn add_preview_shifted(
    app: &mut impl Setup,
    levels: Vec<LevelRow>,
    tiles: TileAssets,
    lights: Option<crate::world_view::light_sources::LightRows>,
    shifts: Option<crate::world_view::palshift::ShiftRows>,
) {
    let link = app
        .world_mut()
        .get_resource_or_insert_with(crate::audio::driver::SoundLink::default)
        .clone();
    let weather = WeatherView::new(tiles.source()).with_sound(link);
    let tiles_source = tiles.source();
    let object_offsets = tiles.source().map(|s| {
        crate::world_view::unit_assets::object_offsets(s.as_ref()).unwrap_or_else(|e| {
            eprintln!("play: objects Xoffset / Yoffset NOT read: {e}");
            Default::default()
        })
    });
    let mut preview = Preview::new(tiles);
    preview.object_offsets = std::sync::Arc::new(object_offsets.unwrap_or_default());
    preview.light.sources = lights.map(std::sync::Arc::new);
    let source = tiles_source;
    preview.light.set_shifts(shifts.map(|rows| {
        crate::world_view::palshift::PalShifts::new(std::sync::Arc::new(rows), source)
    }));
    let mut state = app.world_mut().resource_mut::<WorldViewState>();
    state.feed = Box::new(
        ModelFeed {
            levels: Some(levels),
            ..ModelFeed::<NoFeed>::default()
        }
        .with_preview(preview)
        .with_weather(weather),
    );
    state.preview = true;
}

/// `link` wrapped in the walk recorder of the play preview's own-walk
/// prediction (`bridge::predict`), and
/// the handle on what it records. Messages pass through unchanged.
pub fn predict_link(link: DynLink) -> (DynLink, WalkTap) {
    let link = PredictLink::new(link);
    let tap = link.tap();
    (Box::new(link), tap)
}

/// Turns the play preview's walk prediction on ([`PreviewWalk`] over the
/// walks `tap` records, at `speeds`), drawing the local player's
/// predicted mode through the unit art when the game UI installed it.
pub fn add_walk(
    app: &mut impl Setup,
    tap: WalkTap,
    speeds: Option<crate::bridge::predict::Speeds>,
) {
    if let Some(mut b) = app.world_mut().get_resource_mut::<BridgeResource>() {
        b.0.set_local_aim_late(true);
    }
    let mut walk = PreviewWalk::new(tap, speeds);
    walk.art = app
        .world()
        .get_resource::<ui::UnitArt>()
        .map(|a| a.0.art.clone());
    add_preview_walk(app, walk);
    crate::world_view::monster_walk::add_monster_walk(app);
    crate::world_view::skill_motion::add_skill_motion(app);
    crate::world_view::walk_room::add_preview_walk_room(app);
}

/// One log line every [`LOG_EVERY`] bridge frames.
fn log_progress(
    bridge: Res<BridgeResource>,
    state: Res<WorldViewState>,
    runs: Option<Res<NodeRuns>>,
    audio: Option<Res<GameAudio>>,
    walk: Option<Res<PreviewWalk>>,
    mut seen_rejected: Local<usize>,
) {
    if bridge.is_added() {
        *seen_rejected = 0;
    }
    // Every refused S→C message once, as it happens (a refused 0x07 or
    // unit add leaves the model short with no other trace).
    let rejected = &bridge.0.log().rejected;
    for r in rejected.iter().skip(*seen_rejected) {
        warn!("S→C 0x{:02X} refused: {}", r.id, r.error);
    }
    *seen_rejected = rejected.len();
    let w = bridge.0.world();
    if w.frames == 0 || !w.frames.is_multiple_of(LOG_EVERY) {
        return;
    }
    info!(
        "frame {}: {} server ticks, {} units in the model, unowned S→C ids {:?}; last view {:?}; node frames {}; audio {:?}",
        w.frames,
        w.server_ticks,
        w.units.len(),
        bridge.0.log().unowned,
        state.last,
        runs.map_or(0, |r| r.get()),
        audio.map(|a| a.stats.clone()),
    );
    let log = bridge.0.log();
    info!(
        "frame {}: {}; refused {}, dropped (unit not in the model) {:?}, discarded {}",
        w.frames,
        where_line(w, walk.as_deref()),
        log.rejected.len(),
        log.dropped,
        log.discarded.len(),
    );
}

/// The local player's place in the model for the progress log: its model
/// sub-tile, the predicted one, its room, the client's active rooms by
/// level and the model's units by type (0 player, 1 monster, 2 object…).
fn where_line(w: &crate::bridge::world::ClientWorld, walk: Option<&PreviewWalk>) -> String {
    let mut levels = std::collections::BTreeMap::<u16, usize>::new();
    for r in w.active_rooms.as_deref().unwrap_or(&[]) {
        *levels.entry(r.level).or_default() += 1;
    }
    let mut types = std::collections::BTreeMap::<u8, usize>::new();
    for k in w.units.keys() {
        *types.entry(k.unit_type).or_default() += 1;
    }
    format!(
        "local cell {:?}, predicted {:?}, local room {:?}, active rooms by level {:?}, units by type {:?}",
        w.local().map(|u| u.cell()),
        walk.and_then(|p| p.predict.cell()),
        w.local_room().map(|r| (r.level, r.x0, r.y0, r.w, r.h)),
        levels,
        types,
    )
}

/// What `d2-client play` runs.
pub struct PlayConfig {
    pub data: GameData,
    pub seed: u32,
    /// The character the join loads (`--save`).
    pub character: single_player::Character,
    /// Close after this many frames (smoke test).
    pub exit_after: Option<u32>,
    /// Where the character is saved on exit (`app::save::save_path`);
    /// `None`: not saved.
    pub save_path: Option<std::path::PathBuf>,
    /// A hardcore character (`play --new --hardcore`); a loaded save's own
    /// status bit makes it hardcore too ([`super::hardcore`]).
    pub hardcore: bool,
    /// The 0x67 flags the front end chose (`front_start`); `None`: the
    /// character's own.
    pub start_flags: Option<u32>,
    /// `--dump-draws DIR --at-tick N`: write one frame's rendering facts,
    /// then exit.
    pub dump: Option<crate::facts::export::DumpRequest>,
    /// `--input SCRIPT`: scripted
    /// pointer input in place of the window's.
    pub input: Option<Vec<crate::world_view::input_script::Step>>,
    /// `--sound-log FILE`: every
    /// sound request call written to FILE.
    pub sound_log: Option<std::path::PathBuf>,
    /// `--frame-schedule FILE`: the
    /// frame schedule and host clock a check run follows, frame by frame.
    pub frame_schedule: Option<crate::world_view::present::FrameSchedule>,
    /// `--no-sound`: no sound device; sound requests return at once.
    pub no_sound: bool,
    /// `--device-ends FILE`: when each one-shot sound ends, as a test gives it.
    pub device_ends: Option<super::sound::DeviceEnds>,
    /// `--async-collects FILE`: the
    /// preload passes that collected a finished async read.
    pub async_collects: Option<super::sound::AsyncCollects>,
    /// `--registry FILE`: the Options values a check run starts from; `None`:
    /// live play
    /// (no registry, `settings.toml` only).
    pub registry: Option<super::registry::HostRegistry>,
    /// `--audio-dump FILE [--audio-ticks N]`: the audio engine runs without
    /// a device, driven tick by tick by
    /// the dump; exit once the server tick reaches N.
    pub audio_dump: Option<(std::path::PathBuf, Option<u64>)>,
    /// `--dump-loading FILE`: every loading-screen present, one row each.
    pub loading_dump: Option<std::path::PathBuf>,
    /// `--poke` / `--poke-file`: pokes
    /// applied on the server thread between frames; empty: none.
    pub pokes: Vec<super::poke::Entry>,
    /// `--send "<f> <Name> f=v..."`: C→S messages injected on the server
    /// thread after frame f − 1.
    pub sends: Vec<super::send::SendEntry>,
    /// Dev tools asked for (no `--no-dev`, `D2RUST_DEV` not off);
    /// [`PlayConfig::dev_tools_on`] says whether they run.
    pub dev: bool,
    /// The game's difficulty (0–2), for the dev tools' slots and reports.
    pub difficulty: u8,
    /// Dev tools: `--dev-start SLOT`, a saved dev slot's folder to start
    /// from.
    pub dev_start: Option<std::path::PathBuf>,
    /// `--offscreen`: the game runs in a hidden window and may write shots
    /// of the presented picture (`world_view::shots`).
    pub offscreen: Option<crate::world_view::shots::Offscreen>,
    /// `--control stdio|pipe`: the game is driven over a line channel
    /// ([`super::control`]); `None`: no channel.
    pub control: Option<super::control::Transport>,
}

impl PlayConfig {
    /// A check run: a run any test flag turns on
    /// (frame dumps, scripted input, schedules, registries, audio dumps,
    /// sound logs, `--no-sound`, pokes, sends, `--frames`).
    pub fn is_check_run(&self) -> bool {
        self.exit_after.is_some()
            || self.dump.is_some()
            || self.input.is_some()
            || self.frame_schedule.is_some()
            || self.registry.is_some()
            || self.audio_dump.is_some()
            || self.loading_dump.is_some()
            || self.sound_log.is_some()
            || self.no_sound
            || !self.pokes.is_empty()
            || !self.sends.is_empty()
    }

    /// Whether the run uses the fixed 800 x 600 check frame instead of the
    /// player's Resolution setting. An offscreen run is meant to see what
    /// players see, so it keeps the player's frame even with check flags;
    /// `--res` still chooses the frame in every run.
    pub fn uses_check_frame(&self) -> bool {
        self.is_check_run() && self.offscreen.is_none()
    }

    /// Whether the dev tools run. They are on unless turned off, except in
    /// a check run, where they could change what the run checks. An
    /// offscreen check run keeps them, as players have them, unless it
    /// injects pokes or sends: those and the dev tools share the server's
    /// one between-frames hook, and the run's own setup wins.
    pub fn dev_tools_on(&self) -> bool {
        self.dev
            && (!self.is_check_run()
                || (self.offscreen.is_some() && self.pokes.is_empty() && self.sends.is_empty()))
    }
}

#[derive(Resource)]
struct ExitAfter(u32);

/// What the exit system saw: `App::run` moves the app's world out
/// before it returns, so the leave cannot run afterwards. [`leave_on_exit`]
/// runs while the world still exists and leaves its result here, shared
/// with `run`.
#[derive(Resource, Clone, Default)]
pub struct ExitReport(pub std::sync::Arc<std::sync::Mutex<Option<ExitOutcome>>>);

/// The result of the leave at the app's exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitOutcome {
    /// Save and Exit was asked in the game (see [`PlayEnd::to_menu`]).
    pub to_menu: bool,
    /// [`after_run`]'s result, the error as text.
    pub left: Result<bool, String>,
}

/// On the frame carrying `AppExit` (window close, Save
/// and Exit, `--frames`) the leave through the server runs once, with the
/// world still in the app ([`after_run`]).
pub fn leave_on_exit(world: &mut bevy::ecs::world::World) {
    let exiting = world
        .get_resource::<bevy::ecs::message::Messages<AppExit>>()
        .is_some_and(|m| !m.is_empty());
    let Some(report) = world.get_resource::<ExitReport>().cloned() else {
        return;
    };
    if !exiting || report.0.lock().map_or(true, |r| r.is_some()) {
        return;
    }
    let to_menu = world
        .get_resource::<BridgeResource>()
        .is_some_and(|b| b.0.world().exit_requested);
    let left = after_run(world).map_err(|e| e.to_string());
    let mut slot = report.0.lock();
    if let Ok(r) = slot.as_mut() {
        **r = Some(ExitOutcome { to_menu, left });
    }
}

/// What `run` does once `app.run()` returned: first the leave through
/// the server ([`leave_game`]: every way out of a game saves), then the
/// automap teardown. Nothing here needs a resource to exist:
/// a missing [`BridgeResource`] leaves nothing (`Ok(false)`), a missing
/// [`WorldViewState`] has no automap to tear down. Returns
/// [`leave_game`]'s result.
pub fn after_run(world: &mut bevy::ecs::world::World) -> Result<bool, BridgeError> {
    let left = match world.get_resource_mut::<BridgeResource>() {
        Some(mut b) => leave_game(&mut b.0),
        None => Ok(false),
    };
    if let Some(a) = world
        .get_resource_mut::<WorldViewState>()
        .and_then(|s| s.into_inner().automap.as_mut())
    {
        if let Err(e) = a.teardown() {
            eprintln!("play: the automap was NOT saved: {e}");
        }
    }
    left
}

/// Leaves the game through the server after the app stopped
/// (Save and Exit's path): a client still in game sends
/// C→S 0x69 ([`Bridge::save_and_exit`]) and runs bridge frames until the
/// server's 0x05 takes it out of the game (at most 100 frames: the drain
/// of the next server frame answers it; a remote server's answer comes on
/// the network's time, so those frames are 20 ms apart, 2 s in all).
/// Returns whether the client is out of the game. Closing the window leaves the same way, so a close
/// in a game saves the character and then ends the program, the same as
/// Save and Exit followed by quitting.
pub fn leave_game<L: crate::bridge::link::ServerLink>(
    bridge: &mut Bridge<L>,
) -> Result<bool, BridgeError> {
    if bridge.world().in_game && !bridge.world().exit_requested {
        bridge.save_and_exit()?;
    }
    let remote = bridge.link().is_remote();
    for _ in 0..100 {
        if !bridge.world().in_game {
            return Ok(true);
        }
        bridge.frame()?;
        if remote {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    Ok(!bridge.world().in_game)
}

fn exit_after(limit: Res<ExitAfter>, mut seen: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *seen += 1;
    if *seen >= limit.0 {
        info!("play: exiting after {} frames", *seen);
        exit.write(AppExit::Success);
    }
}

/// Wall-clock seconds for the client's day-night light: a
/// client-only host input. Under a fixed display clock
/// (`world_view::display_clock`) it follows that clock, so runs repeat.
fn wall_seconds() -> i32 {
    crate::world_view::display_clock::wall_seconds().unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i32)
    })
}

/// What [`add_live_client`] wires besides the link: the character and
/// how the game was asked for, and the server's published state.
pub struct LiveClient<'a> {
    /// The user's files (`GameData::Live`).
    pub data: &'a std::sync::Arc<single_player::LiveData>,
    /// The character the join loads (its 0x67, `send_create_game_flags`).
    pub request: &'a single_player::Character,
    /// The 0x67 flags the front end chose; `None`: the character's own.
    pub start_flags: Option<u32>,
    /// The store prices the server host publishes (`Started::prices`).
    pub prices: crate::ui::original::ShopPrices,
    /// The walk speeds of the character (`single_player::walk_speeds`).
    pub speeds: Option<crate::bridge::predict::Speeds>,
    pub hardcore: bool,
    /// Where the automap is saved (next to the character save).
    pub automap_files: Option<super::automap::SaveFiles>,
    /// The GPU compositor's node (the window); `false` headless.
    pub gpu: bool,
}

/// The play game's client on the user's files, on `link`: the bridge
/// with every client table, the world view with the preview over the
/// archives' tiles, palettes, lights, the items, missiles,
/// automap, game UI, strings, HUD tables, shop prices, hireling
/// stats, the game's sound layer, the walk prediction, visibility,
/// loading overlay, death and hardcore screens. `run` calls it after
/// the window plugins; the headless tests call it on `MinimalPlugins`,
/// so a test runs the same client `d2-client play` runs.
pub fn add_live_client(app: &mut App, link: DynLink, c: LiveClient) -> anyhow::Result<()> {
    register_live_client(app, c.gpu);
    install_live_client(app, link, c)
}

/// The systems of [`add_live_client`], once per app (`app::session`): a
/// game started in the running app ([`install_live_client`] on its
/// `World`) finds them there.
pub fn register_live_client(app: &mut App, gpu: bool) {
    register_game(app, gpu);
    #[cfg(feature = "online")]
    app.register("online-watch", super::online::add_watch);
    #[cfg(feature = "online")]
    app.register("online-prices", super::online_prices::add_prices);
    app.register("audio", sound::register_audio);
    app.register("act-palettes", palette::register_act_palettes);
    app.register("text-colors", ui::register_text_colors);
    app.register("unit-art", ui::register_unit_art);
    app.register(
        "preview-walk",
        crate::world_view::walk::register_preview_walk,
    );
    app.register(
        "monster-walk",
        crate::world_view::monster_walk::register_monster_walk,
    );
    app.register(
        "skill-motion",
        crate::world_view::skill_motion::register_skill_motion,
    );
    app.register(
        "preview-walk-room",
        crate::world_view::walk_room::register_preview_walk_room,
    );
    app.register("loading", super::loading_overlay::register_loading);
    app.register("death", super::death::register_death);
    app.register("hardcore", super::hardcore::register_hardcore);
}

/// [`add_live_client`]'s game on `app` (an app built with
/// [`register_live_client`], or its `World` while it runs).
pub fn install_live_client(
    app: &mut impl Setup,
    link: DynLink,
    c: LiveClient,
) -> anyhow::Result<()> {
    let d = c.data;
    let data = GameData::Live(d.clone());
    let archives = d.archives.clone();
    let drlg_source = single_player::client_drlg_source(&data);
    let level_rows = single_player::client_level_rows(&data);
    let waypoint_map = single_player::client_waypoint_map(&data);
    let level_names = single_player::client_level_names(&data);
    let level_warps = single_player::client_level_warps(&data);
    let object_rows = single_player::client_object_rows(&data);
    let object_labels = single_player::client_object_labels(&data);
    let automap_source = super::automap::live_source(&d.tables).map_err(anyhow::Error::msg)?;
    let hire_rows = d2_sim::world::hirelings::HirelingRows::from_table(
        d.tables.table("hireling").map_err(anyhow::Error::msg)?,
    )
    .map_err(|e| anyhow::anyhow!("hireling: {e}"))?;
    let (link, tap) = predict_link(link);
    install_game(app, link)?;
    send_create_game_flags(app, c.request, c.start_flags)?;
    add_client_data(app, drlg_source, level_rows.clone());
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .set_object_rows(object_rows);
    if let Some(mut state) = app.world_mut().get_resource_mut::<WorldViewState>() {
        state.object_labels = ObjectLabels::new(object_labels, level_names.clone(), level_warps);
        state.monster_labels = crate::world_view::monster_label::MonsterLabels {
            rows: single_player::client_monster_labels(&data),
        };
    }
    let skills = single_player::client_skill_rows(archives.as_ref())?;
    let skill_tables = single_player::client_skill_tables(archives.as_ref())?;
    let class_skills = single_player::client_class_skills(archives.as_ref())?;
    let class_vitals = single_player::client_class_vitals(archives.as_ref())?;
    let class_velocities = single_player::client_class_velocities(archives.as_ref())?;
    let stat_ops = single_player::client_stat_ops(archives.as_ref())?;
    {
        let mut bridge = app.world_mut().resource_mut::<BridgeResource>();
        bridge.0.set_skill_rows(skills);
        bridge.0.set_class_skills(class_skills);
        bridge.0.set_class_velocities(class_velocities);
        bridge.0.set_class_vitals(class_vitals);
        bridge.0.set_stat_ops(stat_ops);
        bridge.0.set_skill_tables(std::sync::Arc::new(skill_tables));
        bridge
            .0
            .set_expansion_installed(d2_data::bin::TableFiles::lod(archives.as_ref()));
    }
    let mut units = single_player::client_unit_rows(archives.as_ref())?;
    single_player::client_monster_anims(archives.as_ref(), &mut units)?;
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .set_unit_rows(units);
    let player_anims = single_player::client_player_anims(&data)?;
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .set_player_anims(std::sync::Arc::new(player_anims));
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .set_wall_seconds(wall_seconds);
    // The preview draws the light map at quality 2 (high,
    // `preview_light`): the model's missile lights follow it.
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .set_high_light_quality(true);
    let palettes = ActPalettes::live(archives.as_ref()).map_err(anyhow::Error::msg)?;
    let tiles = TileAssets::new(Some(archives.source()), Some(palettes.pl2.clone()));
    let lights = crate::world_view::light_sources::load(archives.as_ref())
        .map_err(|e| {
            warn!("light rows (D2RUST's own, unverified): {e}; level ambients from the environment")
        })
        .ok();
    let shifts = crate::world_view::palshift::ShiftRows::live(archives.as_ref())
        .map_err(|e| warn!("palette shift rows: {e}; no monster shifted"))
        .ok();
    add_preview_shifted(app, level_rows, tiles, lights, shifts);
    // Pass 1's level background (`background_view`); the summit's star
    // seed is drawn from the clock at first use.
    if let Some(mut state) = app.world_mut().get_resource_mut::<WorldViewState>() {
        let mut view = crate::world_view::background_view::BackgroundView::new(
            archives.source(),
            Some(crate::world_view::background_view::summit_seed_now()),
        );
        // `D2RUST_STARS_SEED` pins the star seed, so a test sees the same
        // sky every run.
        if let Some(v) = crate::launch::env("D2RUST_STARS_SEED").and_then(|v| v.parse::<u32>().ok())
        {
            view = view
                .with_stars_seed(d2_sim::rng::Seed::init_low(v))
                .with_replay_clock();
        }
        state.background_view = Some(view);
    }
    match crate::world_view::unit_facts::load(archives.as_ref()) {
        Ok(t) => {
            if let Some(mut state) = app.world_mut().get_resource_mut::<WorldViewState>() {
                state.feed.set_unit_fact_tables(t);
            }
        }
        Err(e) => warn!("unit facts tables: {e}; draw order uses the preview fills only"),
    }
    let mut item_parts = super::items::item_parts(archives.as_ref()).map_err(anyhow::Error::msg)?;
    match d
        .tables
        .item_tables()
        .map_err(|e| e.to_string())
        .and_then(|t| super::items::item_tips(archives.as_ref(), t))
    {
        Ok(t) => {
            app.world_mut()
                .resource_mut::<BridgeResource>()
                .0
                .set_item_tables(std::sync::Arc::new(super::items::TableDecoder(t.tables())));
            item_parts.tips = Some(t);
        }
        Err(e) => warn!("item tips (D2RUST's own, unverified): {e}; no tool tips"),
    }
    // The equip-box click's tables (`ui::panels::inv_items` `equip`).
    match d2_sim::items::inventory::InvTables::from_fixed(&d.tables.fixed) {
        Ok(t) => item_parts.inv_tables = Some(std::sync::Arc::new(t)),
        Err(e) => warn!("inventory tables: {e}; equipment boxes take no clicks"),
    }
    super::items::add_items(app, archives.source(), item_parts);
    let effects = super::missile_art::effect_rows(archives.as_ref()).map_err(anyhow::Error::msg)?;
    super::missile_art::add_missiles(app, archives.source(), effects);
    palette::add_act_palettes(app, palettes);
    super::automap::add_automap(
        app,
        automap_source,
        c.automap_files,
        archives.source(),
        true,
    );
    let parts = ui::UiParts::live(archives.clone()).map_err(anyhow::Error::msg)?;
    ui::add_original_ui(app, parts)?;
    let strings = super::strings::TableStrings::load(archives.as_ref(), super::strings::LANG)
        .map_err(anyhow::Error::msg)?;
    super::strings::install_strings(app, strings);
    super::hud::install_hud_tables(app, archives.as_ref()).map_err(anyhow::Error::msg)?;
    super::hud::install_char_tables(app, archives.as_ref()).map_err(anyhow::Error::msg)?;
    super::hud::install_skill_tree_tables(app, archives.as_ref()).map_err(anyhow::Error::msg)?;
    ui::set_waypoint_map(app, waypoint_map);
    ui::set_level_names(app, level_names);
    ui::set_shop_prices(app, c.prices);
    super::hire_stats::install_hire_stats(app, hire_rows, true, c.request.difficulty());
    let table = sound::sound_table_live(archives.as_ref()).map_err(anyhow::Error::msg)?;
    let audio = GameAudio::new(AudioParts::original(archives.source(), table));
    // The unit sounds' tables (`audio/unit_feed.rs`): `monsounds`, the
    // animation of each unit's mode.
    let looks = std::sync::Arc::new(
        crate::world_view::unit_assets::UnitLooks::live(archives.as_ref())
            .map_err(anyhow::Error::msg)?,
    );
    let unit_rows = crate::audio::unit_feed::UnitSoundRows::live(
        archives.as_ref(),
        looks,
        std::sync::Arc::new(d.tables.anim.clone()),
    )
    .map_err(anyhow::Error::msg)?;
    if let Some(driver) = &audio.driver {
        let mut driver = driver
            .lock()
            .map_err(|_| anyhow::anyhow!("audio state poisoned"))?;
        driver.set_unit_rows(std::sync::Arc::new(unit_rows));
        // A mod game's per-category gains, when its binary inserted hooks.
        if let Some(mods) = app.world().get_resource::<crate::mods::ClientMods>() {
            driver.set_mods(mods.clone());
        }
        match crate::world_view::unit_facts::load(archives.as_ref()) {
            Ok(t) => driver.set_sight_tables(std::sync::Arc::new(t)),
            Err(e) => warn!("sound occlusion tables: {e}; occlusion stays 0"),
        }
    }
    app.world_mut().insert_resource(audio);
    add_walk(app, tap, c.speeds);
    super::visibility::add_visibility(app);
    super::loading_overlay::add_loading(app, Some(archives.source()));
    super::death::add_death(app);
    super::hardcore::add_hardcore(app, c.hardcore);
    Ok(())
}

/// Opens the window and runs the game until it is closed.
/// How a game run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayEnd {
    pub exit: AppExit,
    /// Save and Exit asked in the game: the front end opens at the main
    /// menu. False when the window was closed: the program ends.
    pub to_menu: bool,
    /// Dev tools: a `load` ended the game: the slot folder to
    /// start again from (its character is already the save file).
    pub relaunch: Option<std::path::PathBuf>,
}

/// At game start the character's
/// `<save>/<name>.key` is read; when it is missing or invalid it is written
/// with the defaults (the install's `default.key` when valid, else the
/// compiled table). The in-game bindings stay in `controls.toml`, and
/// `default.key` is never written into the player's install.
fn write_key_file(save: &std::path::Path) {
    use crate::controls::original::{char_file_bytes, load_at_start};
    let (Some(dir), Some(name)) = (save.parent(), save.file_stem()) else {
        return;
    };
    let path = dir.join(format!("{}.key", name.to_string_lossy()));
    let char_file = std::fs::read(&path).ok();
    let install = std::env::var_os("D2_GAME_DIR")
        .and_then(|d| std::fs::read(std::path::Path::new(&d).join("default.key")).ok());
    let load = load_at_start(char_file.as_deref(), install.as_deref(), None);
    if load.write_files {
        // A first start has no save folder yet.
        let written = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&path, char_file_bytes(&load.table)));
        if let Err(e) = written {
            eprintln!("warning: {}: {e}", path.display());
        }
    }
}

/// The config folder, the settings and the UI registry values of a game
/// ([`run`] needs the settings before the window opens).
pub struct PlaySettings {
    pub cfg_dir: std::path::PathBuf,
    pub settings: super::config::Settings,
    ui_registry: Option<super::registry::UiRegistry>,
}

/// The config folder is next to the saves; a bad settings.toml stops
/// here (no silent default).
pub fn play_settings(config: &PlayConfig) -> anyhow::Result<PlaySettings> {
    let cfg_dir = super::config::config_dir(
        &config
            .save_path
            .as_deref()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(super::save::default_save_dir),
    );
    let mut settings = super::config::load_settings(&cfg_dir)?;
    // A check run: the registry's Options values over the defaults; a row
    // the registry lacks keeps the game's default, not what an earlier run
    // left in `settings.toml`. The window mode and size stay the user's.
    let ui_registry = match &config.registry {
        Some(r) => {
            settings = super::config::Settings {
                window_mode: settings.window_mode,
                window_size: settings.window_size,
                alt_enter_return: settings.alt_enter_return,
                ..Default::default()
            };
            r.apply_settings(&mut settings)?;
            Some(r.ui_values()?)
        }
        None => None,
    };
    Ok(PlaySettings {
        cfg_dir,
        settings,
        ui_registry,
    })
}

/// The systems of a play game, once per app: the live client
/// ([`register_live_client`]), the settings' application, the dev
/// overlay, the leave at the app's exit and, with `output`, the sound
/// device's stream.
pub fn register_play(app: &mut App, output: bool) {
    // A fatal system error (a panicking server/compute task) ends the
    // process with the crash log instead of leaving it running.
    app.insert_resource(bevy::ecs::error::FallbackErrorHandler(
        crate::launch::exit_on_system_error,
    ));
    register_live_client(app, true);
    // A muted process (`crate::mute`) has no audio plugin to play on.
    if output && !crate::mute::muted() {
        sound::add_output(app);
    }
    app.register("dev-overlay", super::dev::overlay::register_overlay);
    app.register("play", |app| {
        // Before the audio frame: a menu change applies to the sounds the same
        // frame requests.
        app.add_systems(
            PostUpdate,
            super::config::apply_settings.before(super::sound::audio_frame),
        )
        .add_systems(Last, leave_on_exit);
    });
}

/// What a game installed that its end reads: the save, the dev tools'
/// `load` request.
pub struct PlayHandles {
    pub saver: Option<save::SaveHandle>,
    pub dev_relaunch: super::dev::overlay::DevRelaunch,
}

/// The server's host clock in a play run: the wall clock, or the display
/// clock the app shares with the bridge.
#[derive(Debug)]
pub enum ServerClock {
    Wall(SystemClock),
    Shared(super::state_dump::StepClock),
}

impl d2_server::seams::Clock for ServerClock {
    fn now_ms(&mut self) -> u32 {
        match self {
            Self::Wall(c) => d2_server::seams::Clock::now_ms(c),
            Self::Shared(c) => d2_server::seams::Clock::now_ms(c),
        }
    }
}

/// Starts the game of `config` (the server on its thread, the save share,
/// the dev tools) and installs it on `app` (an app [`register_play`]
/// registered, or its `World` while it runs).
pub fn install_play(
    app: &mut impl Setup,
    config: PlayConfig,
    ps: PlaySettings,
) -> anyhow::Result<PlayHandles> {
    let GameData::Live(live) = config.data.clone();
    let request = config.character.clone();
    let dev_on = config.dev_tools_on();
    // The automap files sit next to the character save.
    let automap_files = config.save_path.as_deref().and_then(|p| {
        Some(super::automap::SaveFiles {
            dir: p.parent()?.to_path_buf(),
            sub: None,
            name: p.file_stem()?.to_string_lossy().into_owned(),
        })
    });
    if let Some(p) = config.save_path.as_deref() {
        write_key_file(p);
    }
    let speeds = single_player::walk_speeds(&config.data, &config.character)?;
    let mut save_base = save::base_save(&config.character);
    let hardcore =
        config.hardcore || save_base.header.status & d2_formats::d2s::status::HARDCORE != 0;
    if hardcore {
        save_base.header.status |= d2_formats::d2s::status::HARDCORE;
    }
    let save_tables: std::sync::Arc<dyn d2_formats::d2s::SaveTables + Send + Sync> =
        std::sync::Arc::new(live.save.clone());
    // The server's saves rebuild the appearance bytes from these tables.
    let appearance = Some(std::sync::Arc::new(
        save::appearance_tables(&live.tables.fixed).map_err(anyhow::Error::msg)?,
    ));
    // A display clock (`world_view::display_clock`) is the bridge's
    // scripted clock; the server shares it, so its ticks follow the frames.
    let clock = match app
        .world()
        .get_resource::<crate::bridge::mirror::ScriptedClock>()
    {
        Some(shared) => ServerClock::Shared(super::state_dump::StepClock(shared.0.clone())),
        None => ServerClock::Wall(SystemClock::default()),
    };
    let (mut link, started) =
        single_player::start_with(config.data, config.seed, config.character, clock)?;
    if hardcore {
        link.with(|l| l.host_mut().game.events.action.hooks().x.hardcore = true)?;
    }
    // Dev tools: the server end
    // on the server thread, else the check run's pokes and sends.
    let dev_chan = if dev_on {
        let strings =
            super::strings::TableStrings::load(live.archives.as_ref(), super::strings::LANG)
                .map_err(anyhow::Error::msg)?;
        let catalog = super::dev::catalog(&live, &strings);
        let start = match &config.dev_start {
            Some(dir) => Some(
                super::dev::slots::read_slot(dir)
                    .map_err(anyhow::Error::msg)?
                    .start,
            ),
            None => None,
        };
        let chan = super::dev::install_server(&mut link, catalog.clone(), start)?;
        Some((chan, catalog))
    } else {
        super::send::install(&mut link, config.pokes, config.sends)?;
        None
    };
    // Before the app exists, so not through Bevy's log.
    println!("single player: seed {}", config.seed);
    let bindings = super::config::load_controls(&super::config::controls_dir(
        crate::ui::front_end::screens::controls::config_path().as_deref(),
        &ps.cfg_dir,
    ))?;
    app.world_mut().insert_resource(super::config::ConfigRes {
        dir: ps.cfg_dir,
        settings: ps.settings,
        bindings,
        sound_device: !config.no_sound,
    });
    // A control channel reads the game through the server thread the
    // save shares, or shares it itself.
    type Shared = Option<super::control::Server>;
    let (link, saver, server): (DynLink, Option<save::SaveHandle>, Shared) = match config.save_path
    {
        Some(path) => {
            let (link, handle) = save::share(link, save_base, save_tables, appearance, path)?;
            let server = link.server();
            (Box::new(link), Some(handle), Some(server))
        }
        None if config.control.is_some() => {
            let (link, server) = super::control::shared(link);
            (link, None, Some(server))
        }
        // Shared for the server path read alone (`save::server_paths_on`).
        None => (Box::new(save::SharedLink::new(link)), None, None),
    };
    let (link, control) = match (config.control, server) {
        (Some(t), Some(server)) => {
            let (link, c) = super::control::open(link, server, t)?;
            (link, Some(c))
        }
        _ => (link, None),
    };
    if let Some(h) = &saver {
        app.world_mut().insert_resource(h.clone());
    }
    install_live_client(
        app,
        link,
        LiveClient {
            data: &live,
            request: &request,
            start_flags: config.start_flags,
            prices: started.prices,
            speeds,
            hardcore,
            automap_files,
            gpu: true,
        },
    )?;
    let dev_relaunch = super::dev::overlay::DevRelaunch::default();
    if let Some((chan, catalog)) = dev_chan {
        app.world_mut()
            .insert_resource(super::dev::info::DevContext {
                seed: config.seed,
                difficulty: config.difficulty,
                save_path: saver.as_ref().map(|h| h.path().to_path_buf()),
                dir: saver
                    .as_ref()
                    .and_then(|h| h.path().parent().map(std::path::Path::to_path_buf))
                    .unwrap_or_else(super::save::default_save_dir),
                catalog: std::sync::Arc::new(catalog.clone()),
            });
        super::dev::overlay::add_overlay(app, chan, catalog);
        app.world_mut().insert_resource(dev_relaunch.clone());
    }
    let w = app.world_mut();
    if let Some((path, until)) = &config.audio_dump {
        // No device: the dump mixes.
        w.insert_resource(super::audio_dump::AudioDump::create(path, *until)?);
    }
    if let Some(steps) = config.input {
        if let Some(why) = crate::world_view::input_script::refuse_wait_until(&steps, "play") {
            anyhow::bail!(why);
        }
        // `skill` / `cast` steps name skills by the player's `skills.txt`.
        if crate::world_view::input_script::uses_skills(&steps) {
            match crate::world_view::script_skills::SkillNames::load(live.archives.as_ref()) {
                Ok(Some(names)) => {
                    w.insert_resource(names);
                }
                Ok(None) => warn!("play --input: no skills.txt; skills resolve by id only"),
                Err(e) => {
                    warn!("play --input: skill names not loaded ({e}); skills resolve by id only")
                }
            }
        }
        w.insert_resource(crate::world_view::input_script::InputScript::new(steps));
    }
    if let Some(c) = control {
        super::control::install(w, c);
    }
    if let Some(path) = &config.sound_log {
        w.insert_resource(super::sound::SoundLog::create(path)?);
    }
    // A run paced by a frame schedule holds the bridge until each
    // server tick has drawn, with or without a
    // draw dump: the client seed's weather and cursor steps then fall on
    // the same passes every run, not on the host's wall clock.
    let draw_paced =
        config.dump.is_some() || (config.frame_schedule.is_some() && config.audio_dump.is_some());
    if let Some(request) = config.dump {
        w.insert_resource(crate::world_view::present::DrawDump::new(request));
    }
    if draw_paced {
        w.insert_resource(crate::bridge::mirror::DrawnTick::default());
    }
    if let Some(path) = config.loading_dump {
        w.insert_resource(super::loading_overlay::LoadingDump(path, 0));
    }
    if let Some(s) = config.frame_schedule {
        w.insert_resource(s);
    }
    if let Some(e) = config.device_ends {
        w.insert_resource(e);
    }
    if let Some(c) = config.async_collects {
        w.insert_resource(c);
    }
    // `--no-sound`: no sound device, so no sound init and every request
    // returns at once: no sound tick and no
    // variant draw, but the trigger rules' own rolls still step the
    // client seed (the NPC greeting is one).
    if let Some(r) = ps.ui_registry {
        ui::set_registry(app, r);
    }
    if config.no_sound {
        if let Some(d) = app
            .world()
            .get_resource::<GameAudio>()
            .and_then(|a| a.driver.as_ref())
        {
            // The driver stays, silent: the trigger rules' direct `roll`
            // draws still step the client seed,
            // and with a request log the log still records the
            // requests the game makes.
            d.lock().unwrap_or_else(|e| e.into_inner()).set_silent();
        }
    }
    let report = ExitReport::default();
    app.world_mut().insert_resource(report);
    Ok(PlayHandles {
        saver,
        dev_relaunch,
    })
}

/// How [`run`]'s game ended, from what it left behind: the leave's
/// report and the dev tools' `load`. Logs the leave.
pub fn play_end(world: &World, h: &PlayHandles, exit: AppExit) -> anyhow::Result<PlayEnd> {
    let outcome = world
        .get_resource::<ExitReport>()
        .and_then(|r| r.0.lock().ok().and_then(|mut r| r.take()));
    finish_play(outcome, h, exit)
}

fn finish_play(
    outcome: Option<ExitOutcome>,
    h: &PlayHandles,
    exit: AppExit,
) -> anyhow::Result<PlayEnd> {
    let to_menu = outcome.as_ref().is_some_and(|o| o.to_menu);
    let left = match outcome {
        Some(o) => o.left.map_err(|e| anyhow::anyhow!(e)),
        None => Ok(false),
    };
    match (&h.saver, left) {
        (Some(h), Ok(true)) => println!(
            "play: left the game; the server's leave wrote {}",
            h.path().display()
        ),
        (None, Ok(_)) => {}
        (_, Ok(false)) => eprintln!("play: the server never answered the leave"),
        (_, Err(e)) => eprintln!("play: the leave failed: {e}"),
    }
    // Dev tools: after a `load`'s leave and save, the slot's character
    // becomes the save file.
    let relaunch = h.dev_relaunch.0.lock().ok().and_then(|mut r| r.take());
    if let (Some(dir), Some(s)) = (&relaunch, &h.saver) {
        std::fs::copy(dir.join("char.d2s"), s.path())
            .map_err(|e| anyhow::anyhow!("load: copying the slot's character: {e}"))?;
    }
    Ok(PlayEnd {
        exit,
        to_menu,
        relaunch,
    })
}

/// Plays the game in a window until it closes: D2RUST's own client, with
/// no mod hooks.
pub fn run(config: PlayConfig) -> anyhow::Result<PlayEnd> {
    run_inner(config, crate::mods::ClientMods::default())
}

/// [`run`] with a mod game's client hooks (`crate::mods`, seams MS14). An
/// empty registry is not inserted. Both load the baked animation frames of
/// their bake folder (`bake-anims` at install); neither generates frames.
pub fn run_with_mods(config: PlayConfig, mods: crate::mods::ClientMods) -> anyhow::Result<PlayEnd> {
    run_inner(config, mods)
}

fn run_inner(config: PlayConfig, mods: crate::mods::ClientMods) -> anyhow::Result<PlayEnd> {
    let dev_log = config.dev_tools_on();
    let output = config.audio_dump.is_none();
    let frames = config.exit_after;
    let offscreen = config.offscreen.clone();
    let ps = play_settings(&config)?;
    // The frame: `play --res`, else the settings' Resolution row. A check
    // run keeps 800 x 600 unless `--res` says otherwise.
    if !config.uses_check_frame() {
        super::config::set_play_frame(&ps.settings)?;
    }
    // The window follows the player's settings. An offscreen run's window
    // is a plain window of `--size` instead (a maximized or fullscreen
    // window would be shown by the system), hidden, never focused and
    // without a border (a title bar would make the system shrink a
    // screen-sized window), with its own copy of the settings so the
    // player's file is untouched.
    let window_settings = match &offscreen {
        Some(o) => super::config::Settings {
            window_mode: super::config::WindowMode::Windowed,
            window_size: super::config::WindowSize::Fixed(o.size.0, o.size.1),
            ..ps.settings
        },
        None => ps.settings,
    };
    let mut window = super::config::window_for(&window_settings);
    if offscreen.is_some() {
        window.visible = false;
        window.focused = false;
        window.decorations = false;
    }
    let mut app = App::new();
    let plugins = crate::mute::plugins(DefaultPlugins).set(WindowPlugin {
        primary_window: Some(window),
        // The pointer starts hidden: the game's cursor becomes the window's
        // hardware cursor once the first frame is drawn
        // (`world_view::hw_cursor`), or is drawn in the frame.
        primary_cursor_options: Some(super::session::hidden_cursor()),
        ..default()
    });
    // game.log, and with the dev tools the log ring the bug report reads.
    app.add_plugins(plugins.set(crate::launch::log_plugin(dev_log)));
    super::perf::add(&mut app);
    register_play(&mut app, output);
    super::control::register(&mut app);
    super::config::add_window_choice(&mut app, &window_settings, ps.cfg_dir.clone());
    if let Some((o, plan)) = offscreen
        .as_ref()
        .and_then(|o| o.shots.as_ref().map(|p| (o, p)))
    {
        crate::world_view::shots::add(&mut app, plan, o.size)
            .map_err(|e| anyhow::anyhow!("--shot-dir {}: {e}", plan.dir.display()))?;
    }
    // Before `install_play`, which gives the server the same clock.
    if let Some(clock) = offscreen.as_ref().and_then(|o| o.display_clock.clone()) {
        crate::world_view::display_clock::add(&mut app, clock);
    }
    if !mods.is_empty() {
        app.insert_resource(mods);
    }
    app.insert_resource(ui::LoadBakedAnimations);
    let handles = install_play(&mut app, config, ps)?;
    if let Some(frames) = frames {
        app.insert_resource(ExitAfter(frames))
            .add_systems(Update, exit_after);
    }
    // `App::run` moved the world out: the leave ran in `leave_on_exit`
    // and left its report in the shared slot.
    let report = app.world().resource::<ExitReport>().clone();
    let exit = app.run();
    let outcome = report.0.lock().ok().and_then(|mut r| r.take());
    finish_play(outcome, &handles, exit)
}

#[cfg(test)]
mod fresh_folder_tests {
    #[test]
    fn the_key_file_of_a_new_character_makes_the_save_folder() {
        let root = std::env::temp_dir().join(format!("d2rust-play-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let save = root.join("saves").join("Hero.d2s");
        super::write_key_file(&save);
        assert!(root.join("saves").join("Hero.key").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
