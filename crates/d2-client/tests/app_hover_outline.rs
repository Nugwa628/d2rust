//! The hover highlight of an area entrance on the install's files, headless:
//! the Blood Moor's cave entrance (a floor patch under a rock arch) lights
//! its arch, a sprite silhouette, not a diamond of ground.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use d2_client::app::play::{add_client_data, add_game, add_walk, predict_link, send_create_game};
use d2_client::app::single_player::{self};
use d2_client::bridge::predict::Speeds;
use d2_client::bridge::world::{UnitKey, TILE};
use d2_client::bridge::BridgeResource;
use d2_server::seams::Clock;

use crate::app_support;
use app_support::SharedLink;

/// The host clock, advanced by the test.
struct StepClock(Arc<AtomicU32>);

impl Clock for StepClock {
    fn now_ms(&mut self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_den_entrance_highlight_is_its_wall_art_not_the_ground() {
    let data = app_support::game_data();
    let ms = Arc::new(AtomicU32::new(1000));
    let (link, _) = single_player::start(
        data.clone(),
        single_player::DEFAULT_SEED,
        StepClock(ms.clone()),
    )
    .unwrap();
    let server = Arc::new(Mutex::new(link));
    let mut app = App::new();
    app.insert_resource(d2_client::bridge::mirror::ScriptedClock(ms.clone()));
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>();
    let (link, tap) = predict_link(Box::new(SharedLink(server.clone())));
    add_game(&mut app, link, false).unwrap();
    add_walk(&mut app, tap, Some(Speeds { walk: 6, run: 9 }));
    send_create_game(&mut app).unwrap();
    add_client_data(
        &mut app,
        single_player::client_drlg_source(&data),
        single_player::client_level_rows(&data),
    );
    app_support::live_tables(&mut app);
    app.update();
    let mut steps = 0;
    let mut step = |app: &mut App| {
        ms.fetch_add(40, Ordering::SeqCst);
        app.update();
        steps += 1;
        assert!(steps < 3000, "the warp happens");
    };
    while app_support::local_player(&server).is_none() {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    let level = |app: &App| {
        app.world()
            .resource::<BridgeResource>()
            .0
            .world()
            .player_level()
    };
    assert_eq!(level(&app), Some(single_player::ACT1_TOWN as u16));
    // The Blood Moor's cave entrance is a tile unit once the player is
    // near its room (the server sends the units of the rooms near the
    // player): walk there. Which of the four `lvlwarp` ids into the Den
    // of Evil exists depends on the stamp the level placed.
    let dens = app_support::warp_ids(single_player::BLOOD_MOOR, single_player::DEN_OF_EVIL);
    app_support::walk_into(
        &mut app,
        &server,
        &ms,
        single_player::ACT1_TOWN,
        single_player::BLOOD_MOOR,
    );
    for _ in 0..30 {
        step(&mut app);
    }
    let spot = app_support::warp_preset_spot(&server, single_player::BLOOD_MOOR, &dens);
    let spot = spot.expect("the Den entrance's preset in the Blood Moor");
    // 5 sub-tiles out the tile is in the 0x13 walk band (unit distance
    // 5..=50): the server runs to it and the
    // pending 0x13 warps on the stop. (From 8 to 12 sub-tiles the run's
    // target is prepared to a cell 13 sub-tiles away and the player
    // stops short: a pathing question, not this warp's.)
    app_support::walk_toward(&mut app, &server, &ms, single_player::BLOOD_MOOR, spot, 5);
    for _ in 0..30 {
        step(&mut app);
    }
    let tile = |app: &App| {
        app.world()
            .resource::<BridgeResource>()
            .0
            .world()
            .units
            .iter()
            .find(|(k, u)| k.unit_type == TILE && dens.contains(&u.class))
            .map(|(k, _)| *k)
    };
    let entrance: UnitKey = tile(&app).expect("entrance");
    use d2_sim::drlg::tiles::{rec_flags, RecordKind};
    let w = app.world().resource::<BridgeResource>().0.world();
    let u = &w.units[&entrance];
    let room = w.unit_room(entrance).expect("the entrance's room");
    let d = w.drlg.as_ref().expect("the client DRLG");
    let pos = u.position.expect("the entrance's position");
    let records = d.warp_highlight_records(room.room, u.class, pos);
    assert!(!records.is_empty(), "the entrance has map records");

    // The cave mouth is a floor patch (hidden warp floors) under a rock arch
    // of wall records of the same DT1 file. The hover highlight follows the
    // arch, a sprite silhouette; it never brightens the ground (a filled
    // diamond of floor tiles).
    let mut warp_files = Vec::new();
    for id in d.adjacency(room.room) {
        let Some(t) = d.drlg.room(id).tiles() else {
            continue;
        };
        for f in &t.floors {
            let (cx, cy) = (i32::from(pos.0) / 5, i32::from(pos.1) / 5);
            let r = d.drlg.room(id).rect;
            let (dx, dy) = (r.x + f.x - cx, r.y + f.y - cy);
            if f.flags & rec_flags::HIDDEN != 0 && dx.abs() <= 2 && dy.abs() <= 2 {
                warp_files.push(f.tile.file);
            }
        }
    }
    assert!(
        !warp_files.is_empty(),
        "the entrance has hidden warp floors"
    );
    for (id, kind, index) in &records {
        assert_eq!(
            *kind,
            RecordKind::Wall,
            "the highlight is wall art, not ground: {records:?}"
        );
        let rec = &d.drlg.room(*id).tiles().expect("tiles").walls[*index];
        assert!(
            warp_files.contains(&rec.tile.file),
            "wall {rec:?} is not from the entrance's own DT1 file"
        );
        assert!(rec.flags & rec_flags::HIDDEN == 0, "a drawn record");
    }
}
