//! The play preview's combat links headless, wired as `d2-client play
//! --new` wires them, over the synthetic single-player game: a monster
//! (S→C 0xAC, injected: no synthetic `monstats` row has `isSpawn`, so no
//! monster spawns in a synthetic game) walks by its S→C 0x67 (the
//! preview's monster motion), and a left click on it sends the skill on
//! the unit (C→S 0x06) through the link to the server.
//!
//! Synthetic fills (no game file): the client `skills` row 0 (Attack:
//! `anim` A1 = mode 7, `range` h2h, `InTown` so the cast's town test
//! passes in the synthetic town) and a `monstats` class 0 with `isAtt`.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{
    add_client_data, add_game, add_preview, add_walk, predict_link, send_create_game_for,
};
use d2_client::app::single_player::{self};
use d2_client::bridge::click::{screen_to_world, ClickView};
use d2_client::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use d2_client::bridge::mirror::DynLink;
use d2_client::bridge::world::{MonsterClass, MonsterSetup, SkillRow, UnitKey, UnitRows, MONSTER};
use d2_client::bridge::BridgeResource;
use d2_client::controls::click::{skill_flag, ClickState, Kind};
use d2_client::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
use d2_client::world_view::tile_assets::TileAssets;
use d2_server::seams::Clock;

use crate::app_support;

struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

/// Records every C→S message the bridge sends.
struct Recorder {
    link: DynLink,
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ServerLink for Recorder {
    fn protocol_version(&self) -> u32 {
        self.link.protocol_version()
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        self.link.tick_rate()
    }
    fn send(&mut self, queue: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.sent.lock().unwrap().push(msg.to_vec());
        self.link.send(queue, msg)
    }
    fn pump(&mut self) -> Result<Pumped, LinkError> {
        self.link.pump()
    }
    fn receive(&mut self) -> Vec<Vec<u8>> {
        self.link.receive()
    }
}

fn step(app: &mut App, ms: &AtomicU32, n: usize) {
    for _ in 0..n {
        app.update();
        ms.fetch_add(40, Ordering::SeqCst);
    }
}

fn monster_at(app: &App, guid: u32) -> (u16, u16) {
    app.world().resource::<BridgeResource>().0.world().units[&UnitKey::new(MONSTER, guid)]
        .position
        .expect("placed")
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_monster_walks_and_a_left_click_on_it_attacks() {
    let data = app_support::game_data();
    let character = single_player::new_character("sorceress", "Test").unwrap();
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start_with(
        data.clone(),
        single_player::DEFAULT_SEED,
        character.clone(),
        StepClock(ms.clone()),
    )
    .unwrap();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let link = Recorder {
        link: Box::new(link),
        sent: sent.clone(),
    };
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    let (link, tap) = predict_link(Box::new(link));
    add_game(&mut app, link, false).unwrap();
    send_create_game_for(&mut app, &character).unwrap();
    let levels = single_player::client_level_rows(&data);
    add_client_data(
        &mut app,
        single_player::client_drlg_source(&data),
        levels.clone(),
    );
    add_preview(&mut app, levels, TileAssets::default());
    {
        let mut b = app.world_mut().resource_mut::<BridgeResource>();
        b.0.set_skill_rows(vec![SkillRow {
            anim: 7,
            range: 1,
            flags: skill_flag::IN_TOWN,
            ingame: true,
            ..SkillRow::default()
        }]);
        b.0.set_unit_rows(UnitRows {
            monsters: vec![Some(MonsterClass {
                // monstats `Velocity` 16: path velocity 0x1000 at 100 %.
                setup: Some(MonsterSetup {
                    is_att: true,
                    is_sel: true,
                    velocity: 16,
                    ..MonsterSetup::default()
                }),
                ..MonsterClass::default()
            })],
            ..UnitRows::default()
        });
    }
    add_walk(&mut app, tap, None);
    step(&mut app, &ms, 10);

    let (guid, (px, py)) = {
        let w = app.world().resource::<BridgeResource>().0.world();
        let p = w.local().expect("local player");
        (p.key.guid, p.position.expect("placed"))
    };
    // S→C 0x94 (skill 0 at level 1) and 0x23 (skill 0 left), as
    // `app_play_e2e.rs`; then 0xAC: monster 77 of class 0, four sub-tiles
    // east of the player, life 128; then 0x67: walk (code 1) to six
    // more sub-tiles east at velocity percent 100 (stat 67; with the
    // class's `Velocity` 16 one sub-tile a tick,
    // as the movement prediction does).
    let mut msgs = vec![0x94, 1];
    msgs.extend_from_slice(&guid.to_le_bytes());
    msgs.extend_from_slice(&[0, 0, 1]);
    msgs.extend_from_slice(&[0x23, 0]);
    msgs.extend_from_slice(&guid.to_le_bytes());
    msgs.extend_from_slice(&[1, 0, 0]);
    msgs.extend_from_slice(&u32::MAX.to_le_bytes());
    let (mx, my) = (px + 4, py);
    msgs.push(0xAC);
    msgs.extend_from_slice(&77u32.to_le_bytes());
    msgs.extend_from_slice(&0u16.to_le_bytes());
    msgs.extend_from_slice(&mx.to_le_bytes());
    msgs.extend_from_slice(&my.to_le_bytes());
    msgs.extend_from_slice(&[128, 14, 1]);
    let mut walk = vec![0x67];
    walk.extend_from_slice(&77u32.to_le_bytes());
    walk.push(1);
    walk.extend_from_slice(&(mx + 6).to_le_bytes());
    walk.extend_from_slice(&my.to_le_bytes());
    walk.extend_from_slice(&[0, 0, 0]);
    walk.extend_from_slice(&100u16.to_le_bytes());
    walk.push(0);
    msgs.extend(walk);
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .receive_chunk(&msgs)
        .unwrap();

    // 1. The monster walks: the update pass applies its 0x67 (walk mode),
    // and each server tick moves it east on the model.
    step(&mut app, &ms, 3);
    let moved = monster_at(&app, 77);
    assert_eq!(moved.1, my);
    assert!(moved.0 > mx, "the monster walked east: {moved:?}");
    step(&mut app, &ms, 12);
    assert_eq!(monster_at(&app, 77), (mx + 6, my), "stops on its target");

    // 2. A left press on the monster's feet: C→S 0x06 [type 1][GUID 77].
    let at = monster_at(&app, 77);
    let size = FrameSize::D2RUST;
    let cam = Camera::new(
        size,
        OpenMode::NONE,
        moving_to_client(
            (u32::from(px) << 16) | 0x8000,
            (u32::from(py) << 16) | 0x8000,
        ),
        (0, 0),
    );
    let mouse = (0..size.width)
        .flat_map(|x| (0..size.play_height()).map(move |y| (x, y)))
        .find(|&(x, y)| screen_to_world(&cam, x, y) == (i32::from(at.0), i32::from(at.1)))
        .expect("the monster is on screen");
    let view = ClickView {
        size,
        open_mode: 0,
        right_panel_bottom: size.play_height(),
        skill_y_limit: size.play_height(),
        mouse,
        game_menu_open: false,
        pick: false,
        shake: (0, 0),
        prev_hover: None,
        local_mode: None,
        speeds: None,
    };
    sent.lock().unwrap().clear();
    let mut st = ClickState::default();
    app.world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .world_click(&mut st, view, Kind::LeftDown, Some(mouse), 0)
        .unwrap();
    let mut want = vec![0x06];
    want.extend_from_slice(&1u32.to_le_bytes());
    want.extend_from_slice(&77u32.to_le_bytes());
    assert_eq!(*sent.lock().unwrap(), vec![want], "the skill on the unit");
}
