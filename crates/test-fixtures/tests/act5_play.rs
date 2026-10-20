//! Act V in the play host (task `q-a5-fields`): a `Session` created in
//! act 4 (Harrogath), its player walking east-to-west through Bloody
//! Foothills and Frigid Highlands to Arreat Plateau, the barricaded
//! outdoor levels populated and the client told of their monsters.
//! Made-up data; no `Covers:` claim.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use d2_sim::units::UnitType;
use test_fixtures::act5::{self, ARREAT_PLATEAU, BLOODY_FOOTHILLS, FRIGID_HIGHLANDS, TOWN};
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{Session, Setup};
use test_fixtures::install;
use test_fixtures::synth::Synthetic;

const ACT: u8 = 4;

/// The Act V set with `isSpawn` on every monster that has a rarity.
fn spawning_act5() -> Synthetic {
    let mut s = act5::act5();
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
            .join(format!("act5-play-{}", std::process::id()));
        let i = install::build(&dir, &spawning_act5()).unwrap_or_else(|e| panic!("{e}"));
        GameData::from_install(&i).unwrap_or_else(|e| panic!("{e}"))
    })
}

fn setup() -> Setup {
    Setup {
        creation: ActCreation::Full,
        init_seed: 644_409_375,
        town: TOWN,
        game_seed: 1234,
        class: 3,
        known_waypoints: Vec::new(),
    }
}

/// Into level `to` along the collision route (the levels' preset objects
/// stand in the way of a straight walk).
fn walk_to(fx: &mut Session, from: u32, to: u32, what: &str) {
    fx.walk_route(from, to, what);
    fx.assert_clean(what);
}

#[test]
fn the_player_walks_from_harrogath_through_the_foothills_to_arreat_plateau() {
    let mut fx = Session::new_in_act(data(), &setup(), ACT);
    assert_eq!(fx.unit_level(fx.player), Some(TOWN));
    walk_to(&mut fx, TOWN, BLOODY_FOOTHILLS, "Bloody Foothills");
    assert_eq!(fx.unit_level(fx.player), Some(BLOODY_FOOTHILLS));
    walk_to(
        &mut fx,
        BLOODY_FOOTHILLS,
        FRIGID_HIGHLANDS,
        "Frigid Highlands",
    );
    assert_eq!(fx.unit_level(fx.player), Some(FRIGID_HIGHLANDS));

    let mons = fx.sim().game.lists.units_of_type(UnitType::Monster);
    let mut in_level = Vec::new();
    for u in mons {
        if fx.unit_level(u) == Some(FRIGID_HIGHLANDS) {
            in_level.push(fx.sim().game.lists.unit(u).unwrap().guid);
        }
    }
    assert!(!in_level.is_empty(), "Frigid Highlands populated");
    let added: BTreeSet<u32> = fx
        .transcript
        .iter()
        .filter(|m| m[0] == 0xAC && m.len() >= 5)
        .map(|m| u32::from_le_bytes([m[1], m[2], m[3], m[4]]))
        .collect();
    assert!(
        in_level.iter().any(|g| added.contains(g)),
        "monsters reached the client"
    );

    // Frigid Highlands' north rows are the barricade border (walled in the
    // original): the collision route goes round it.
    walk_to(&mut fx, FRIGID_HIGHLANDS, ARREAT_PLATEAU, "Arreat Plateau");
    assert_eq!(fx.unit_level(fx.player), Some(ARREAT_PLATEAU));
}
