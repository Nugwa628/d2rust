//! A level warp in the app's own synthetic game, headless (task
//! `q-warps`): the Blood Moor's cave entrance
//! is a tile unit (S→C 0x09); the client's interact sender puts C→S 0x13
//! (unit type 5) on the wire; the server runs the walk into the warp,
//! places the player in the Den of Evil and sends the new
//! level's 0x07 / 0x15 / 0x0D; the client's model follows to the new
//! level. The tile creation and the 0x13 tile case follow the game's
//! 0x13 handler, seen in known runs (tile near, walk, far): from < 5 the
//! warp is in the
//! same frame; from 9 a run, then the warp on arrival with no second
//! C→S 0x13; from 30 or 62 nothing.

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

/// The server player's level id.
fn server_level(server: &app_support::Server<StepClock>) -> Option<u32> {
    app_support::with(server, |l| {
        let (p, _) = single_player::local_player(&l.host().game)?;
        let g = &mut l.host_mut().game;
        let room = g.game.lists.unit(p)?.room()?;
        g.events.action.hooks().drlg.level_id(&g.game, room)
    })
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn clicking_the_cave_entrance_takes_the_player_to_the_den_of_evil() {
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
    // The cave entrance is a tile unit of the client's model.
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
    let entrance: UnitKey = tile(&app).expect("the Blood Moor's cave entrance reached the client");
    // The tile hover's feedback: the destination level label (the level's
    // LevelWarp key, else its LevelName, as the OpenWarp object label reads
    // them) and the map records the highlight tints. At least one record is
    // drawn: a floor entrance's hidden warp cells are skipped by the draw, so
    // the visible floor cells of the same footprint carry the highlight.
    let labels = d2_client::world_view::object_label::ObjectLabels::new(
        single_player::client_object_labels(&data),
        single_player::client_level_names(&data),
        single_player::client_level_warps(&data),
    );
    let strings =
        d2_client::app::strings::TableStrings::load(app_support::live().archives.as_ref(), "eng")
            .expect("the string tables load");
    {
        use d2_client::ui::StringLookup;
        use d2_sim::drlg::tiles::RecordKind;

        let w = app.world().resource::<BridgeResource>().0.world();
        let u = &w.units[&entrance];
        let room = w.unit_room(entrance).expect("the entrance's room");
        let d = w.drlg.as_ref().expect("the client DRLG");
        let dest = d
            .warp_dest_level(room.room, u.class)
            .expect("the entrance's destination level");
        let key = &labels.level_warps[dest as usize];
        let expected = if !key.is_empty() {
            strings
                .get(key)
                .map_or_else(|| key.encode_utf16().collect::<Vec<_>>(), <[u16]>::to_vec)
        } else {
            let key = &labels.level_names[dest as usize];
            strings
                .get(key)
                .map_or_else(|| key.encode_utf16().collect::<Vec<_>>(), <[u16]>::to_vec)
        };
        assert_eq!(
            labels.text(w, entrance, &strings),
            Some(expected),
            "the entrance's destination label"
        );

        let records = d.warp_highlight_records(
            room.room,
            u.class,
            u.position.expect("the entrance's position"),
        );
        assert!(!records.is_empty(), "the entrance's map records");
        let mut drawn = 0;
        for (id, kind, index) in records {
            let tiles = d.drlg.room(id).tiles().expect("the record's room tiles");
            let record = match kind {
                RecordKind::Floor => &tiles.floors[index],
                RecordKind::Wall => &tiles.walls[index],
                RecordKind::Shadow => &tiles.shadows[index],
            };
            if kind != RecordKind::Shadow && record.flags & 0x8 == 0 {
                drawn += 1;
            }
        }
        assert!(drawn > 0, "a highlighted map record is drawn");
    }
    // Playtest B031: the cursor on the entrance hovers it (its lvlwarp
    // select box), so a press reaches the tile.
    let picked = {
        use d2_client::rules::camera::{moving_to_client, Camera, FrameSize, OpenMode};
        let w = app.world().resource::<BridgeResource>().0.world();
        let (px, py) = w.local().and_then(|p| p.position).expect("player cell");
        let at = moving_to_client(u32::from(px) << 16, u32::from(py) << 16);
        let cam = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
        let u = &w.units[&entrance];
        let (l, t, r, b) =
            d2_client::bridge::hover::tile_rect(w, &cam, u).expect("the entrance has a hover box");
        d2_client::bridge::hover::pick(w, &cam, ((l + r) / 2, (t + b) / 2))
    };
    assert_eq!(
        picked,
        Some(entrance),
        "the cursor on the entrance hovers it"
    );
    // The click's interact sender sends 0x13 for
    // the tile; the server warps the player.
    let sent = app
        .world_mut()
        .resource_mut::<BridgeResource>()
        .0
        .interact(entrance)
        .unwrap();
    assert!(sent.is_empty());
    while server_level(&server) != Some(single_player::DEN_OF_EVIL) {
        step(&mut app);
    }
    for _ in 0..30 {
        step(&mut app);
    }
    // The client's model follows: the new level, the player placed in
    // it, the arrival tile of the Den among its units.
    assert_eq!(level(&app), Some(single_player::DEN_OF_EVIL as u16));
    let b = &app.world().resource::<BridgeResource>().0;
    assert!(b.log().rejected.is_empty(), "{:?}", b.log().rejected);
    assert!(
        b.world().units.iter().any(|(k, u)| k.unit_type == TILE
            && app_support::warp_ids(single_player::DEN_OF_EVIL, single_player::BLOOD_MOOR)
                .contains(&u.class)),
        "the Den's way back is in the client's model"
    );
}
