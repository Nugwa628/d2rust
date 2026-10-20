//! Automap data vectors on the user's game data and the cel files of
//! Needs the game files:
//! `D2_GAME_DIR=<install> cargo test -p d2-client --test game_automap -- --ignored --nocapture`

use d2_client::ui::automap::options::cel_paths;
use d2_client::ui::automap::picker::{CelPicker, PickKey};
use d2_client::ui::automap::SEED;
use d2_formats::dc6::Dc6;
use d2_formats::mpq::ArchiveSet;
use d2_sim::rng::Seed;

fn set() -> ArchiveSet {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    ArchiveSet::open_dir(&dir).expect("archives in D2_GAME_DIR open")
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn picker_first_record_vector() {
    let set = set();
    let data = d2_data::bin::load(&set, d2_data::bin::DEFAULT_LANGUAGE).expect("tables load");
    let anim = d2_data::fixup::read_animdata(&set).expect("AnimData.d2");
    let fixed = d2_data::fixup::apply(&data, &anim).expect("fix-ups");
    let p = CelPicker::new(&fixed.automap);
    assert_eq!(p.ranges[1], (0, 83));
    // Fresh seed {0, 666}; LevelType 1, `fl`, main 0, sub 5 → record 0,
    // roll(4) = 2 → Cel3 = 2; seed {666, 0}.
    let mut s = SEED;
    let k = PickKey {
        level_type: 1,
        orientation: 0,
        main: 0,
        sub: 5,
    };
    assert_eq!(p.pick(k, &mut s).unwrap(), 2);
    assert_eq!(s, Seed::new(666, 0));
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn cel_files_open() {
    let set = set();
    for mini in [false, true] {
        for path in cel_paths(mini, true).into_iter().flatten() {
            let bytes = set.read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let d = Dc6::parse(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert!(!d.frames.is_empty(), "{path}");
            println!("{path}: {} cels", d.frames.len());
        }
    }
}
