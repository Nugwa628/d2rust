//! The play mode's original UI and sound layer, headless: the app's own
//! wiring (`app::play::add_game` + `app::ui::add_original_ui` +
//! `AudioParts::original`) over the synthetic single-player game, with
//! synthetic panel DC6 files in a memory source (the art is the user's;
//! the files here are fixtures).
//!
//! - a hotkey action queued as the input edge does becomes
//!   `SetUIState(1, toggle, 0)` in the next drawn frame: the inventory
//!   opens, the root draws its art, the right border and the control
//!   panel, the panel files are read once into the frame store, and the
//!   UI's open mode is the feed's;
//! - the sound layer runs one sound tick per server tick and the core
//!   presents the sound tick.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use d2_client::app::play::add_game;
use d2_client::app::single_player::{self, DEFAULT_SEED};
use d2_client::app::sound::{AudioParts, GameAudio};
use d2_client::app::ui::{add_original_ui, UiParts};
use d2_client::assets::path::MemorySource;
use d2_client::audio::sound_table::SoundTableData;
use d2_client::bridge::BridgeResource;
use d2_client::controls::Action;
use d2_client::ui::{ActionId, UiEvent};
use d2_client::world_view::{UiSounds, WorldViewState, WorldViewUi};
use d2_server::seams::Clock;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A DC6 of one direction with `frames` frames of 2 × 2 literal pixels.
fn dc6(frames: u32) -> Vec<u8> {
    let rows = [2u8, 1, 2, 0x80, 2, 3, 4, 0x80];
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&frames.to_le_bytes());
    let mut at = d.len() + 4 * frames as usize;
    let mut body = Vec::new();
    for _ in 0..frames {
        d.extend_from_slice(&(at as u32).to_le_bytes());
        for v in [0u32, 2, 2, 0, 0, 0, 0, rows.len() as u32] {
            body.extend_from_slice(&v.to_le_bytes());
        }
        body.extend_from_slice(&rows);
        body.extend_from_slice(&[0xEE; 3]);
        at += 32 + rows.len() + 3;
    }
    d.extend(body);
    d
}

/// The files the inventory, the 800 × 600 border and control panel name
/// (classic install: `InvChar`).
fn panel_files() -> MemorySource {
    let mut s = MemorySource::default();
    for (name, frames) in [
        ("panel\\invchar", 8),
        ("panel\\buysellbtn", 12),
        ("panel\\800borderframe", 10),
        ("panel\\800ctrlpnl7", 6),
        ("panel\\goldcoinbtn", 2),
        // The cursor cels, drawn every frame.
        ("cursor\\gaunt", 1),
        ("cursor\\grasp", 8),
        ("cursor\\ohand", 8),
        ("cursor\\orotate", 8),
        ("cursor\\ppress", 8),
        ("cursor\\protate", 8),
        ("cursor\\buysell", 10),
    ] {
        s.insert(&format!("data\\global\\ui\\{name}.dc6"), dc6(frames));
    }
    s
}

fn sound_table() -> SoundTableData {
    let s = "Sound\tIndex\tFileName\tVolume\tGroup Size\tBlock 1\tBlock 2\tBlock 3\r\n\
             none\t0\tnone.wav\t0\t0\t-1\t-1\t-1\r\n";
    let e = "Handle\tIndex\tSong\r\nx\t0\t0\r\n";
    let st = d2_data::txt::TxtTable::parse("sounds.txt", s.as_bytes()).unwrap();
    let et = d2_data::txt::TxtTable::parse("soundenviron.txt", e.as_bytes()).unwrap();
    SoundTableData::from_txt(&st, &et).unwrap()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn hotkey_opens_the_inventory_in_the_apps_frame() {
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start(
        app_support::game_data(),
        DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    add_game(&mut app, Box::new(link), true).unwrap();
    add_original_ui(
        &mut app,
        UiParts {
            source: Arc::new(panel_files()),
            inv_areas: None,
            expansion_installed: false,
            fonts: None,
            resist_penalties: None,
        },
    )
    .unwrap();
    app.insert_resource(GameAudio::new(AudioParts::original(
        Arc::new(MemorySource::default()),
        sound_table(),
    )));

    // Frame 1: no tick, nothing drawn.
    app.update();
    assert_eq!(app.world().resource::<WorldViewState>().last, None);

    // The input edge's action, then a tick: the frame routes it.
    let id = ActionId(Action::ToggleInventory.index() as u16);
    app.world_mut()
        .non_send_mut::<WorldViewUi>()
        .queue
        .0
        .push(UiEvent::Action(id));
    ms.fetch_add(40, Ordering::SeqCst);
    app.update();
    let ui = app.world().non_send::<WorldViewUi>();
    let original = ui.original.as_ref().unwrap();
    assert!(original.is_open(1));
    assert_eq!(original.open_mode().get(), 1);
    let state = app.world().resource::<WorldViewState>();
    let last = state.last.unwrap();
    // Inventory art 4 + gold button 1 + close 1, right border 5, control
    // panel 6 (no local player: no HUD overlay), the cursor cel last
    // (`protate`, 8 frames loaded).
    assert_eq!((last.items, last.ui_unhandled), (18, 1));
    assert_eq!(state.assets.frames.len(), 8 + 12 + 10 + 6 + 2 + 8);
    let world = app.world().resource::<BridgeResource>().0.world();
    assert_eq!(state.feed.open_mode(world).unwrap().get(), 1);

    // Toggle again: closed, mode 0; only the control panel and the cursor
    // are drawn.
    app.world_mut()
        .non_send_mut::<WorldViewUi>()
        .queue
        .0
        .push(UiEvent::Action(id));
    ms.fetch_add(40, Ordering::SeqCst);
    app.update();
    let state = app.world().resource::<WorldViewState>();
    assert_eq!(state.last.unwrap().items, 7);
    let world = app.world().resource::<BridgeResource>().0.world();
    assert_eq!(state.feed.open_mode(world).unwrap().get(), 0);

    // The sound layer ran one sound tick per server tick; the core
    // presented the sound tick; the UI made no sound.
    let audio = app.world().resource::<GameAudio>();
    let ticks = world.server_ticks as u32;
    assert_eq!(audio.driver.as_ref().unwrap().lock().unwrap().tick(), ticks);
    assert_eq!(audio.stats.presented, ticks);
    assert!(app.world().resource::<UiSounds>().0.is_empty());
}

/// The live install: every file the wired panels draw (each class, both
/// sides open, expansion game) loads into the frame store, the
/// `inventory.bin` `inv` rectangles are the known test vectors, and the sound
/// table compiles.
/// `D2_GAME_DIR=<install> cargo test -p d2-client --test app_original_ui -- --ignored`
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn wired_panels_load_from_the_install() {
    use d2_client::app::sound::sound_table_live;
    use d2_client::bridge::world::{ClientUnit, ClientWorld, UnitKey, PLAYER};
    use d2_client::ui::layout::Screen;
    use d2_client::ui::original::{InvArea, OriginalUi, UiConfig};
    use d2_client::ui::{NoPanelRules, NoStrings, UiCtx, UiDraw, UiRoot};
    use d2_client::world_view::panel_art::PanelArtLoader;
    use d2_client::world_view::ViewAssets;
    use d2_formats::mpq::ArchiveSet;

    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let archives = Arc::new(ArchiveSet::open_dir(dir).expect("archives open"));
    let files = Arc::new(d2_client::assets::game_files::GameFiles::archives(
        archives.clone(),
    ));
    let parts = UiParts::live(files).unwrap();
    let areas = parts.inv_areas.clone().unwrap();
    assert_eq!(areas.len(), 32);
    let area = |left, right, top, bottom| InvArea {
        left,
        right,
        top,
        bottom,
    };
    assert_eq!(areas[0], area(320, 640, 0, 441));
    assert_eq!(areas[16], area(400, 720, 60, 501));
    sound_table_live(archives.as_ref()).unwrap();

    let mut assets = ViewAssets::new(d2_client::app::play::unspecified_palette());
    for class in 0..7 {
        let config = UiConfig {
            screen: Screen::R800,
            expansion_installed: parts.expansion_installed,
        };
        let mut ui = OriginalUi::new(config, Some(areas.clone())).unwrap();
        let mut root = UiRoot::new(Box::new(NoPanelRules));
        ui.install(&mut root).unwrap();
        let mut w = ClientWorld::default();
        let key = UnitKey::new(PLAYER, 1);
        let mut u = ClientUnit::new(key);
        u.class = class;
        u.mode = 1;
        w.units.insert(key, u);
        w.local_player = Some(key);
        w.expansion = 1;
        let loader = PanelArtLoader::new(archives.clone(), ui.files());
        for state in [1, 4] {
            ui.before_event(UiEvent::CursorLeft, &w);
            ui.set_ui(2, 0, false).unwrap();
            ui.set_ui(state, 0, false).unwrap();
            ui.after_event(
                &mut root,
                UiEvent::CursorLeft,
                d2_client::ui::Routed::Unhandled,
            )
            .unwrap();
            let ctx = UiCtx {
                tick: 0,
                world: &w,
                strings: &NoStrings,
            };
            let mut draws: Vec<UiDraw> = Vec::new();
            root.draw(&ctx, &mut draws);
            assert!(draws.len() > 10, "class {class}, ui {state}");
            loader
                .ensure(&draws, &mut assets)
                .unwrap_or_else(|e| panic!("class {class}, ui {state}: {e}"));
        }
    }
}
