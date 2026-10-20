//! The local player's position on the user's install (q-fix-walk-desync):
//! seeded random play in town and outdoors, the client's own position
//! (the walk prediction) against the server player's. Each run once gave
//! `desync:player-position` (a pick-up or interact sent from afar, a hit
//! or death while walking, a walk through a colliding object). `#[ignore]`:
//! the real-data gate runs them (`tools/realdata-gate.sh`).

use d2_client::app::soak::log::Start;
use d2_client::app::soak::{soak, SoakArgs};

use crate::app_support;

fn start(class: &str, warp: Option<u32>) -> Start {
    Start {
        class: Some(class.into()),
        warp,
        kit: true,
        ..Start::default()
    }
}

/// The run's position desyncs, after checking it did not break.
fn position_desyncs(start: Start, seed: u64) -> Vec<String> {
    let data = app_support::game_data();
    let a = SoakArgs {
        start,
        seed,
        steps: 1500,
        keep_going: true,
        save_dir: Some(
            std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("soak-walk-{}-{seed}", std::process::id())),
        ),
        ..SoakArgs::default()
    };
    let out = soak(&data, &a, None, |_| {});
    let broke: Vec<_> = out.findings.iter().filter(|f| f.kind == "setup").collect();
    assert!(broke.is_empty(), "the run broke: {broke:?}");
    out.findings
        .into_iter()
        .filter(|f| f.sig == "desync:player-position")
        .map(|f| {
            format!(
                "seed {seed} step {} frame {}: {}",
                f.step, f.frame, f.detail
            )
        })
        .collect()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_client_position_follows_the_server_in_town_and_outdoors() {
    let mut found = Vec::new();
    for (class, warp, seed) in [("sorceress", None, 3), ("barbarian", Some(3), 2)] {
        found.extend(position_desyncs(start(class, warp), seed));
    }
    assert!(found.is_empty(), "{found:#?}");
}
