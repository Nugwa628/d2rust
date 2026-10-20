//! Act II in the play host (task `q-act-worlds`): a `Session` created in
//! act 1 (Lut Gholein), its player walking out to the Rocky Waste and the
//! Dry Hills, with the desert levels populated and the client told of the
//! monsters. Made-up data; no `Covers:` claim.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use d2_sim::units::UnitType;
use test_fixtures::act2::{self, DRY_HILLS, ROCKY_WASTE, TOWN};
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{Session, Setup};
use test_fixtures::install;
use test_fixtures::synth::Synthetic;

const ACT: u8 = 1;

/// The Act II set with `isSpawn` on every monster that has a rarity.
fn spawning_act2() -> Synthetic {
    let mut s = act2::act2();
    let f = s.tables.file("monstats.txt");
    let col = |name: &str| {
        f.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
            .unwrap()
    };
    let (rarity, id) = (col("Rarity"), col("Id"));
    let rows: Vec<usize> = f
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| !r[rarity].is_empty() && !r[id].is_empty())
        .map(|(i, _)| i)
        .collect();
    assert!(!rows.is_empty(), "the set has spawnable monsters");
    for i in rows {
        s.tables.set("monstats", i, "isSpawn", "1");
    }
    s
}

fn data() -> &'static GameData {
    static D: OnceLock<GameData> = OnceLock::new();
    D.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("act2-play-{}", std::process::id()));
        let i = install::build(&dir, &spawning_act2()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

/// The game seed; `ACT2_SEED` overrides it to try other seeds.
fn seed() -> u32 {
    std::env::var("ACT2_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

fn setup() -> Setup {
    Setup {
        creation: ActCreation::Full,
        init_seed: 644_409_375,
        town: TOWN,
        game_seed: seed(),
        class: 3,
        known_waypoints: Vec::new(),
    }
}

/// Walk along a searched route over the active rooms' collision (the
/// route is searched again as the rooms ahead become active), not to
/// fixed goal points.
fn walk_to(fx: &mut Session, from: u32, to: u32, what: &str) {
    fx.walk_route(from, to, what);
    fx.assert_clean(what);
}

#[test]
fn the_player_walks_from_lut_gholein_into_the_desert_with_monsters() {
    let mut fx = Session::new_in_act(data(), &setup(), ACT);
    assert_eq!(fx.unit_level(fx.player), Some(TOWN));
    walk_to(&mut fx, TOWN, ROCKY_WASTE, "Rocky Waste");
    assert_eq!(fx.unit_level(fx.player), Some(ROCKY_WASTE));

    // The rooms spawn as they become active: a seed may leave the first
    // room empty, so the walker goes on toward the Dry Hills until the
    // Waste holds a monster (or the walk cannot go further).
    let monsters_in_waste = |fx: &mut Session| {
        let mons = fx.sim().game.lists.units_of_type(UnitType::Monster);
        let mut guids = Vec::new();
        for u in mons {
            if fx.unit_level(u) == Some(ROCKY_WASTE) {
                guids.push(fx.sim().game.lists.unit(u).unwrap().guid);
            }
        }
        guids
    };
    fx.walk_route_until(ROCKY_WASTE, DRY_HILLS, "Waste monsters", |f| {
        !monsters_in_waste(f).is_empty()
    });
    let in_waste = monsters_in_waste(&mut fx);
    assert!(!in_waste.is_empty(), "the Rocky Waste populated");
    let added: BTreeSet<u32> = fx
        .transcript
        .iter()
        .filter(|m| m[0] == 0xAC && m.len() >= 5)
        .map(|m| u32::from_le_bytes([m[1], m[2], m[3], m[4]]))
        .collect();
    assert!(
        in_waste.iter().any(|g| added.contains(g)),
        "monsters reached the client"
    );

    // Onward, to the Dry Hills (the chain's next level).
    walk_to(&mut fx, ROCKY_WASTE, DRY_HILLS, "Dry Hills");
    assert_eq!(fx.unit_level(fx.player), Some(DRY_HILLS));
}
