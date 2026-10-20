//! Coverage bookkeeping cannot change deterministic server snapshots.
use d2_client::app::{
    autoplay_host::{Command, Host, HostArgs},
    coverage,
};
use d2_client::bridge::state::StateSource;
use std::path::Path;

#[test]
fn counters_on_and_off_preserve_every_state_1_tick() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("coverage-install-{}", std::process::id()));
    let mut data = test_fixtures::play::playable();
    for name in [
        "invchar6",
        "800borderframe",
        "goldcoinbtn",
        "buysellbtn",
        "inv_armor",
        "inv_belt",
        "inv_boots",
        "inv_helm_glove",
        "inv_ring_amulet",
        "inv_weapons",
    ] {
        data.stored.push((
            format!(r"data\global\ui\panel\{name}.dc6"),
            test_fixtures::play::stand_in_dc6(),
        ));
    }
    test_fixtures::cache::copy_install(Path::new(env!("CARGO_TARGET_TMPDIR")), &dir, &data)
        .unwrap();
    let args = HostArgs {
        game_dir: Some(dir.clone()),
        new: Some(("amazon".into(), "Cover".into())),
        seed: Some(7),
        ..Default::default()
    };
    let run = |disabled: bool| {
        let mut host = Host::start(&args).unwrap();
        host.server()
            .lock()
            .unwrap()
            .with(|l| {
                assert!(!l.host().game.game.coverage.enabled);
                assert!(l.host().game.game.coverage.rows.is_empty());
                assert!(!l.host().coverage.enabled);
                assert!(l.host().coverage.rows.is_empty());
            })
            .unwrap();
        assert!(
            !host
                .app_mut()
                .world()
                .non_send::<d2_client::world_view::WorldViewUi>()
                .root
                .coverage
                .enabled
        );
        host.server()
            .lock()
            .unwrap()
            .with(move |l| {
                l.host_mut().game.game.coverage.rows.clear();
                l.host_mut().game.game.coverage.enabled = !disabled;
                l.host_mut().coverage.rows.clear();
                l.host_mut().coverage.enabled = !disabled;
            })
            .unwrap();
        {
            let mut ui = host
                .app_mut()
                .world_mut()
                .non_send_mut::<d2_client::world_view::WorldViewUi>();
            ui.root.coverage.rows.clear();
            ui.root.coverage.enabled = !disabled;
        }
        let mut states = Vec::new();
        for tick in 0..30 {
            if matches!(tick, 2 | 12) {
                host.run(Command::Key(0x49)).unwrap();
            }
            host.step(1);
            states.push(
                host.server()
                    .lock()
                    .unwrap()
                    .state_snapshot()
                    .unwrap()
                    .to_json_line(),
            );
        }
        if disabled {
            host.server()
                .lock()
                .unwrap()
                .with(|l| {
                    assert!(l.host().game.game.coverage.rows.is_empty());
                    assert!(l.host().coverage.rows.is_empty());
                })
                .unwrap();
            assert!(host
                .app_mut()
                .world()
                .non_send::<d2_client::world_view::WorldViewUi>()
                .root
                .coverage
                .rows
                .is_empty());
        }
        let report = coverage::snapshot(&mut host).unwrap();
        (states, report)
    };
    let (on, on_counts) = run(false);
    let (off, off_counts) = run(true);
    assert_eq!(on, off, "all state-1 bytes must match");
    assert!(on_counts
        .counters
        .get("panels_opened")
        .is_some_and(|r| r.values().sum::<u64>() > 0));
    assert!(off_counts
        .counters
        .get("panels_opened")
        .is_none_or(|r| r.values().sum::<u64>() == 0));
    assert!(!on_counts.tables["skills_cast"].is_empty());
}
