//! Save/load round trips on the user's install (q-tool-soak): the play
//! client, headless, Save and Exit through the server's leave, the file
//! read and joined again, the live state and a second save compared with
//! the first (`d2_client::app::soak::roundtrip`). `#[ignore]`: the
//! real-data gate runs them (`tools/realdata-gate.sh`).

use d2_client::app::soak::log::Start;
use d2_client::app::soak::{soak, Finding, SoakArgs};

use crate::app_support;

fn args(start: Start, seed: u64, steps: u32, at: &[u32], rate: u32) -> SoakArgs {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "save-roundtrip-{}-{seed}-{}",
        std::process::id(),
        start.class.as_deref().unwrap_or("save")
    ));
    SoakArgs {
        start,
        seed,
        steps,
        keep_going: true,
        rate: Some(rate),
        roundtrip_at: at.to_vec(),
        save_dir: Some(dir),
        ..SoakArgs::default()
    }
}

fn new(class: &str) -> Start {
    Start {
        class: Some(class.into()),
        kit: false,
        ..Start::default()
    }
}

/// The run's findings of `kinds`, after checking it ran its round trips.
fn run(a: &SoakArgs, kinds: &[&str]) -> Vec<Finding> {
    let data = app_support::game_data();
    let out = soak(&data, a, None, |_| {});
    let setup: Vec<_> = out
        .findings
        .iter()
        .filter(|f| f.kind == "setup" || f.kind == "panic")
        .collect();
    assert!(setup.is_empty(), "the run broke: {setup:?}");
    assert_eq!(
        out.roundtrips as usize,
        a.roundtrip_at.len(),
        "round trips run: {:?}",
        out.findings
    );
    out.findings
        .into_iter()
        .filter(|f| kinds.contains(&f.kind.as_str()))
        .collect()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_new_character_of_every_class_round_trips_twice() {
    for class in [
        "amazon",
        "sorceress",
        "necromancer",
        "paladin",
        "barbarian",
        "druid",
        "assassin",
    ] {
        let f = run(&args(new(class), 1, 120, &[30, 80], 0), &["roundtrip"]);
        assert!(f.is_empty(), "{class}: {f:?}");
    }
}

/// Each act's town (the start poke `warp`: an act change),
/// then a round trip: the town act and the
/// waypoints come back.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn every_act_town_round_trips() {
    for town in [40, 75, 103, 109] {
        let mut s = new("sorceress");
        s.warp = Some(town);
        let f = run(&args(s, 1, 120, &[40], 0), &["roundtrip"]);
        assert!(f.is_empty(), "town {town}: {f:?}");
    }
}

/// After seeded random play (soak play: item moves, panels, keys,
/// waypoints), two round trips: what the reload brings back is what was
/// saved (an item saved on the cursor comes back on it,
/// q-fix-soak-cursor-reload).
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn random_play_round_trips() {
    for seed in 1..=3 {
        let mut s = new("sorceress");
        s.kit = true;
        let f = run(&args(s, seed, 400, &[150, 300], 30), &["roundtrip"]);
        assert!(f.is_empty(), "seed {seed}: {f:?}");
    }
}

/// A new character's belt potions reach the client model (the soak's checks).
/// Settled (q-fix-items-shop): the join's belt records reach the model.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_new_characters_belt_is_in_the_model() {
    let f = run(&args(new("sorceress"), 1, 120, &[], 0), &["desync"]);
    assert!(f.is_empty(), "{f:?}");
}
