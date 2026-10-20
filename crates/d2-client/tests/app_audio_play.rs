//! The classic sound layer on the play path: the app's single-player
//! game (synthetic tables) in a headless Bevy `App`, the audio frame with
//! the standard audio parts ([`AudioParts::original`], a small `sounds.txt`),
//! and sound requests that ask the model a question it cannot answer
//! yet (the line test, the client seed). Such a question is reported and
//! answered neutrally; it never fails the frame (Bevy's default error
//! handler panics, so a failed audio frame stopped play on the first unit
//! sound of the real install).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::config::{ConfigRes, Settings};
use d2_client::app::play::{add_client_data, add_game, send_create_game};
use d2_client::app::single_player::{self, DEFAULT_SEED};
use d2_client::app::sound::{AudioParts, GameAudio};
use d2_client::assets::path::MemorySource;
use d2_client::audio::driver::SoundRequest;
use d2_client::audio::sound_table::SoundTableData;
use d2_client::audio::SoundId;
use d2_client::bridge::world::UnitKey;
use d2_client::bridge::BridgeResource;
use d2_client::controls::Bindings;
use d2_client::rules::OpenMode;
use d2_client::world_view::{UiSounds, WorldViewState};
use d2_data::txt::TxtTable;
use d2_server::seams::Clock;

use crate::app_support;
use app_support::SharedLink;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The song every `soundenviron` row of [`table`] names.
const SONG: i32 = 4660;

/// Ids 0–4,700 (row 0 silent): id 2 heads a group of 2 (a variant draw on
/// the client seed). 50 `soundenviron` rows, each with song [`SONG`].
fn table() -> SoundTableData {
    let mut s =
        String::from("Sound\tIndex\tFileName\tVolume\tGroup Size\tBlock 1\tBlock 2\tBlock 3\r\n");
    for i in 0..=4_700 {
        let (v, g) = match i {
            0 => (0, 0),
            2 => (255, 2),
            _ => (255, 0),
        };
        s += &format!("s{i}\t{i}\ts{i}.wav\t{v}\t{g}\t-1\t-1\t-1\r\n");
    }
    let mut e = String::from("Handle\tIndex\tSong\r\n");
    for i in 0..50 {
        e += &format!("e{i}\t{i}\t{SONG}\r\n");
    }
    let st = TxtTable::parse("sounds.txt", s.as_bytes()).unwrap();
    let et = TxtTable::parse("soundenviron.txt", e.as_bytes()).unwrap();
    SoundTableData::from_txt(&st, &et).unwrap()
}

/// A 16-bit mono PCM WAV of `n` samples at 22,050 Hz.
fn wav(n: u32) -> Vec<u8> {
    let data = n * 2;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&22_050u32.to_le_bytes());
    b.extend_from_slice(&44_100u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    b.resize(b.len() + data as usize, 0);
    b
}

/// The table's files, each a 2-second WAV (the table's paths).
fn sounds(t: &SoundTableData) -> MemorySource {
    let paths = t.paths();
    let mut m = MemorySource::default();
    for id in 1..5_000 {
        if let Some(p) = paths.get(SoundId(id)) {
            m.insert(&p, wav(44_100));
        }
    }
    m
}

/// The play app over the synthetic game with the standard audio parts,
/// run until the local player joined; its key.
fn play_app(ms: &Arc<AtomicU32>) -> (App, UnitKey) {
    let (link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let server = Arc::new(Mutex::new(link));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    add_game(&mut app, Box::new(SharedLink(server.clone())), true).unwrap();
    app_support::live_tables(&mut app);
    app.world_mut()
        .resource_mut::<WorldViewState>()
        .feed
        .set_ui_open_mode(OpenMode::new(0).unwrap());
    app.insert_resource(GameAudio::new(AudioParts::original(
        Arc::new(sounds(&table())),
        table(),
    )));
    app.init_resource::<UiSounds>();
    send_create_game(&mut app).unwrap();
    add_client_data(
        &mut app,
        single_player::client_drlg_source(&app_support::game_data()),
        single_player::client_level_rows(&app_support::game_data()),
    );
    for _ in 0..40 {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        if let Some(p) = app
            .world()
            .resource::<BridgeResource>()
            .0
            .world()
            .local_player
        {
            return (app, p);
        }
    }
    panic!("the local player did not join");
}

fn frames(app: &mut App, ms: &Arc<AtomicU32>, n: usize) {
    for _ in 0..n {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_pending_sound_input_does_not_stop_play() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut app, p) = play_app(&ms);
    // A unit sound on the player (its line test is pending) and a grouped
    // UI sound (its variant draw needs the client seed).
    app.world_mut().resource_mut::<UiSounds>().0.extend([
        SoundRequest::UnitRequest { id: 1, unit: p },
        SoundRequest::Ui(2),
    ]);
    frames(&mut app, &ms, 3);
    let audio = app.world().resource::<GameAudio>();
    assert!(
        !audio.stats.pending.is_empty(),
        "the pending questions are reported"
    );
    let ticks = app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .server_ticks;
    assert_eq!(
        audio.stats.presented, ticks as u32,
        "a sound tick per server tick"
    );
}

/// Whether a request of `id` holds a channel after a UI request of it.
fn plays(app: &mut App, ms: &Arc<AtomicU32>, id: i32) -> bool {
    app.world_mut()
        .resource_mut::<UiSounds>()
        .0
        .push(SoundRequest::Ui(id));
    frames(app, ms, 1);
    let audio = app.world().resource::<GameAudio>();
    let d = audio.driver.as_ref().unwrap().lock().unwrap();
    (0..64).any(|c| d.system().channel_request(c).is_some_and(|r| r.id == id))
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_options_master_volume_reaches_the_sound_layer() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut app, _) = play_app(&ms);
    let mut settings = Settings::default();
    app.insert_resource(ConfigRes {
        dir: std::env::temp_dir(),
        settings,
        bindings: Bindings::empty(),
        sound_device: true,
    });
    assert!(plays(&mut app, &ms, 1), "master 100: the UI sound plays");
    // The options menu sets Master Volume 0: nothing is heard.
    settings.master_volume = 0;
    app.world_mut().resource_mut::<ConfigRes>().settings = settings;
    frames(&mut app, &ms, 1);
    assert!(!plays(&mut app, &ms, 3), "master 0 silences");
    let audio = app.world().resource::<GameAudio>();
    let d = audio.driver.as_ref().unwrap().lock().unwrap();
    assert_eq!(d.system().settings().master_volume, 0);
    assert_eq!(d.system().settings().music_volume, 50);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_town_levels_song_starts_on_the_play_path() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (mut app, _) = play_app(&ms);
    frames(&mut app, &ms, 2);
    let level = app
        .world()
        .resource::<BridgeResource>()
        .0
        .world()
        .player_level();
    assert_eq!(level, Some(single_player::ACT1_TOWN as u16));
    let audio = app.world().resource::<GameAudio>();
    let d = audio.driver.as_ref().unwrap().lock().unwrap();
    assert_eq!(d.environment().unwrap().music.cur, SONG);
    assert!(d.system().requests().any(|r| r.id == SONG));
}
