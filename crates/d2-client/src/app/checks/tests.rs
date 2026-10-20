//! Planted broken states for the shared server rules, using made-up data.
use super::*;
use crate::app::autoplay_host::{Host, HostArgs};

fn inspect(tag: &str, f: impl FnOnce(&mut Sim) + Send + 'static) {
    let dir = std::env::temp_dir().join(format!("d2-checks-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    test_fixtures::install::build(&dir, &test_fixtures::play::playable()).unwrap();
    {
        let mut host = Host::start(&HostArgs {
            new: Some(("amazon".into(), "Checks".into())),
            seed: Some(7),
            game_dir: Some(dir.clone()),
            ..Default::default()
        })
        .unwrap();
        host.step(8);
        host.server()
            .lock()
            .unwrap()
            .with(move |link| {
                let sim = &mut link.host_mut().game;
                let baseline = server_check(sim).0;
                assert!(baseline.is_empty(), "baseline {baseline:?}");
                f(sim);
            })
            .unwrap();
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hud_uses_the_installs_stat_replication_contract() {
    inspect("hud-replication", |sim| {
        let view = always::server_view(sim);
        assert!(view.hud.contains_key("life"));
        assert!(view.hud.contains_key("gold"));
        for name in ["life_max", "mana_max", "stamina_max", "level"] {
            assert!(!view.hud.contains_key(name), "unpublished {name}");
        }
    });
}

#[test]
fn guid_counter_corruption_trips_the_shared_rule() {
    inspect("guid", |sim| {
        let (id, _) = single_player::local_player(sim).unwrap();
        sim.game.lists.unit_mut(id).unwrap().guid = 100;
        sim.events.action.sys.units.get_mut(id).unwrap().guid = 100;
        sim.game.lists.guids.set(UnitType::Player, 0);
        assert!(server_check(sim).0.iter().any(|v| v.kind == "guid"));
    });
}

#[test]
fn list_record_mismatch_trips_the_shared_rule() {
    inspect("list", |sim| {
        let (id, _) = single_player::local_player(sim).unwrap();
        sim.events.action.sys.units.get_mut(id).unwrap().guid += 100;
        assert!(server_check(sim).0.iter().any(|v| v.kind == "list"));
    });
}

#[test]
fn negative_vitals_trip_the_shared_rule() {
    inspect("vitals", |sim| {
        let (id, _) = single_player::local_player(sim).unwrap();
        let sys = &mut sim.events.action.sys;
        let list = sys.stats.unit_list(id).unwrap();
        sys.stats.set(
            &mut d2_sim::stats::lists::NoHost,
            list,
            stat::HITPOINTS,
            -256,
            0,
            Some(id),
        );
        assert!(server_check(sim).0.iter().any(|v| v.kind == "vitals"));
    });
}

#[test]
fn room_path_mismatch_trips_the_shared_rule() {
    inspect("room", |sim| {
        let (id, _) = single_player::local_player(sim).unwrap();
        let path = sim
            .events
            .action
            .sys
            .hooks
            .paths
            .as_mut()
            .unwrap()
            .dynamic_mut(id)
            .unwrap();
        path.room = Some(d2_sim::units::RoomId(u32::MAX));
        assert!(server_check(sim).0.iter().any(|v| v.kind == "room"));
    });
}

#[test]
fn a_wall_under_the_player_is_detected_from_server_terrain() {
    inspect("walkable", |sim| {
        use d2_sim::path::collision::CollisionRooms;
        let (id, _) = single_player::local_player(sim).unwrap();
        let sys = &mut sim.events.action.sys;
        let path = sys.hooks.paths.as_ref().unwrap().record(id).unwrap();
        let room = path.room().unwrap();
        let (x, y) = path.position();
        *sys.hooks
            .drlg
            .grid_mut(room)
            .unwrap()
            .get_mut(x, y)
            .unwrap() |= 1;
        assert!(always::server_view(sim)
            .blocked
            .iter()
            .any(|(ty, _, _, _, mask)| *ty == 0 && mask & 1 != 0));
    });
}

#[test]
fn an_item_in_two_grids_trips_the_shared_rule() {
    inspect("item", |sim| {
        let (id, _) = single_player::local_player(sim).unwrap();
        let inv = sim.world.inventory.as_mut().unwrap();
        let inventory = inv.state.inventories.get_mut(&id).unwrap();
        // A made-up reference keeps this independent of starting gear.
        let item = UnitId(u32::MAX);
        inventory.grid_or_create(10, 2, 2).unwrap().items.push(item);
        inventory.grid_or_create(11, 2, 2).unwrap().items.push(item);
        assert!(server_check(sim)
            .0
            .iter()
            .any(|v| v.sig == "item:two-places-in-inventory"));
    });
}
