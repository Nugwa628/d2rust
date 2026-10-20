//! Act III in the play host (task `q-a3-fields`): a `Session` created in
//! act 2 (Kurast Docks), its player walking north through Spider Forest,
//! Great Marsh and Flayer Jungle on through the Kurast chain to Travincal, Spider Forest populated
//! and the client told of the monsters. Made-up data; no `Covers:` claim.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use d2_sim::units::UnitType;
use test_fixtures::act3::{
    self, FLAYER_JUNGLE, GREAT_MARSH, KURAST_BAZAAR, KURAST_CAUSEWAY, LOWER_KURAST, SPIDER_FOREST,
    TOWN, TRAVINCAL, UPPER_KURAST,
};
use test_fixtures::game::{ActCreation, GameData};
use test_fixtures::host::{border_goals, Session, Setup};
use test_fixtures::install;
use test_fixtures::synth::Synthetic;

const ACT: u8 = 2;

/// The Act III set with `isSpawn` on every monster that has a rarity.
fn spawning_act3() -> Synthetic {
    let mut s = act3::act3();
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
            .join(format!("act3-play-{}", std::process::id()));
        let i = install::build(&dir, &spawning_act3()).unwrap_or_else(|e| panic!("{e}"));
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

fn walk_to(fx: &mut Session, from: u32, to: u32, what: &str) {
    let a = fx.level_rect(from);
    let b = fx.level_rect(to);
    let p = fx.pos();
    let goals = border_goals(a, b, p);
    fx.walk(&goals, |f| f.unit_level(f.player) == Some(to), what);
    fx.assert_clean(what);
}

#[test]
fn the_player_walks_from_the_docks_through_the_jungle_and_kurast_to_travincal() {
    let mut fx = Session::new_in_act(data(), &setup(), ACT);
    assert_eq!(fx.unit_level(fx.player), Some(TOWN));
    walk_to(&mut fx, TOWN, SPIDER_FOREST, "Spider Forest");
    assert_eq!(fx.unit_level(fx.player), Some(SPIDER_FOREST));

    let mons = fx.sim().game.lists.units_of_type(UnitType::Monster);
    let mut in_forest = Vec::new();
    for u in mons {
        if fx.unit_level(u) == Some(SPIDER_FOREST) {
            in_forest.push(fx.sim().game.lists.unit(u).unwrap().guid);
        }
    }
    assert!(!in_forest.is_empty(), "Spider Forest populated");
    let added: BTreeSet<u32> = fx
        .transcript
        .iter()
        .filter(|m| m[0] == 0xAC && m.len() >= 5)
        .map(|m| u32::from_le_bytes([m[1], m[2], m[3], m[4]]))
        .collect();
    assert!(
        in_forest.iter().any(|g| added.contains(g)),
        "monsters reached the client"
    );

    // Onward along the chain.
    let chain = [
        SPIDER_FOREST,
        GREAT_MARSH,
        FLAYER_JUNGLE,
        LOWER_KURAST,
        KURAST_BAZAAR,
        UPPER_KURAST,
        KURAST_CAUSEWAY,
        TRAVINCAL,
    ];
    for w in chain.windows(2) {
        let (a, b) = (fx.level_rect(w[0]), fx.level_rect(w[1]));
        let p = fx.pos();
        let goals = border_goals(a, b, p);
        assert!(!goals.is_empty(), "levels {} and {} meet", w[0], w[1]);
        let to = w[1];
        // The jungles meet along a stretch of their edge only: first walk
        // along the level to the goal's row (or column), then across.
        let g = goals[0];
        // (6 tiles inside, off the level's blocked border cells).
        let inside = |edge: i32, toward: i32| (edge - 6 * toward) * 5;
        let stage = if a.x + a.w == b.x {
            (inside(a.x + a.w, 1), g.1)
        } else if b.x + b.w == a.x {
            (inside(a.x, -1), g.1)
        } else {
            // Keep the column the player is in when it is in the overlap.
            let (lo, hi) = goals
                .iter()
                .fold((i32::MAX, i32::MIN), |(l, h), q| (l.min(q.0), h.max(q.0)));
            let x = p.0.clamp(lo, hi);
            if a.y + a.h == b.y {
                (x, inside(a.y + a.h, 1))
            } else {
                (x, inside(a.y, -1))
            }
        };
        // A blocked cell on the straight line gives way to the next offset.
        let stages: Vec<(i32, i32)> = [0, 10, -10, 20, -20, 30, -30, 40, -40]
            .iter()
            .map(|&o| (stage.0 + o, stage.1))
            .collect();
        fx.walk(
            &stages,
            |f| f.unit_level(f.player) != Some(w[0]) || (f.pos().1 - stage.1).abs() <= 15,
            "along the edge",
        );
        fx.walk(&goals, |f| f.unit_level(f.player) == Some(to), "next level");
        fx.assert_clean("next level");
    }
    assert_eq!(fx.unit_level(fx.player), Some(TRAVINCAL));
}
