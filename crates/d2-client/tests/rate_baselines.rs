//! Deterministic simulation and CPU frame baselines for base play at 60 Hz.
//! All art, tables and saves in this test are synthetic.
use bevy::prelude::*;
use d2_client::app::{
    autoplay_host::{Command, Host, HostArgs},
    save, single_player,
};
use d2_client::bridge::BridgeResource;
use d2_client::world_view::{compose_cpu, feed::build_frame, present::WorldViewState};
use d2_sim::tick::TickRate;
use serde_json::json;

fn hash(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    format!("{h:016x}")
}

#[test]
fn base_game_sixty_hz_baselines_and_save_clock_independence() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("rate-baselines-{}", std::process::id()));
    test_fixtures::install::build(&dir, &test_fixtures::play::playable()).unwrap();
    let mut h = Host::start(&HostArgs {
        game_dir: Some(dir),
        new: Some(("amazon".into(), "RateProbe".into())),
        seed: Some(1234),
        ..HostArgs::default()
    })
    .unwrap();
    let mut rows = Vec::new();
    for ticks in [0, 30, 30, 60] {
        h.step(ticks);
        let state = h
            .server()
            .lock()
            .unwrap()
            .with(|l| {
                let sim = &mut l.host_mut().game;
                assert_eq!(sim.game.tick_rate, TickRate::GAME);
                d2_sim::debug::state::snapshot_world(&sim.game, &sim.events).to_json_line()
            })
            .unwrap();
        let rgba = h
            .app_mut()
            .world_mut()
            .resource_scope(|w, mut v: Mut<WorldViewState>| {
                let b = w.resource::<BridgeResource>();
                assert_eq!(b.0.world().tick_rate, TickRate::GAME);
                let WorldViewState {
                    assets,
                    rules,
                    feed,
                    last_ui,
                    ..
                } = &mut *v;
                let f = build_frame(b.0.world(), last_ui, &**rules, &mut **feed, assets).unwrap();
                compose_cpu(&f, assets).unwrap()
            });
        assert!(rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] != 0 || p[1] != 0 || p[2] != 0));
        rows.push(json!({"frame": h.server_frame().unwrap(), "state": hash(state.as_bytes()), "rgba": hash(&rgba)}));
        if ticks == 0 {
            h.run(Command::Press(
                d2_client::ui::PointerButton::Left,
                d2_client::ui::Point { x: 410, y: 310 },
            ))
            .unwrap();
        }
    }
    let got = json!({"tick_rate": 60, "rows": rows});
    let baseline =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/baselines/base-60hz.json");
    if std::env::var_os("D2_UPDATE_RATE_BASELINES").is_some() {
        std::fs::write(
            &baseline,
            format!("{}\n", serde_json::to_string_pretty(&got).unwrap()),
        )
        .unwrap();
    }
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(baseline).unwrap()).unwrap();
    assert_eq!(got, expected);

    // No tick, timer phase or game rate enters the character save. The only
    // persisted times are creation/save timestamps in Unix seconds.
    h.server()
        .lock()
        .unwrap()
        .with(|l| {
            let sim = &mut l.host_mut().game;
            let base = save::base_save(&single_player::Character::New);
            let before = save::apply_live(&base, &save::read_live(sim).unwrap(), 1234);
            let frame = sim.game.frame;
            sim.game.frame = i32::MAX;
            sim.game.tick_rate = TickRate::BASE;
            let after = save::apply_live(&base, &save::read_live(sim).unwrap(), 1234);
            sim.game.frame = frame;
            sim.game.tick_rate = TickRate::GAME;
            assert_eq!(before, after);
            assert_eq!(after.header.save_time, 1234);
        })
        .unwrap();
}
