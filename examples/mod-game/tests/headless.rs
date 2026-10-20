//! The example mod game runs headless on the player's files: its game is
//! built under its own rules and save folder, a character joins and the
//! game ticks (`docs/plans/mods/seams.md` MS16).

use d2_client::app::single_player::{GameData, GameSource, Layers};
use d2_sim::rules::{ModRules, SkillFn};
use mod_game_example::{
    builder, rules_name, run_frames, start_and_join, ExampleRules, Options, StepClock,
    EXAMPLE_DO_SLOT, NAME,
};

#[test]
fn the_example_adds_one_skill_body_past_d2rusts_table() {
    let rules = ExampleRules::default();
    assert_eq!(rules.name(), NAME);
    let skills = rules.skills().expect("skill bodies");
    assert!(skills.has(SkillFn::Do, EXAMPLE_DO_SLOT));
    assert!(!skills.has(SkillFn::Do, 1));
    assert!(!skills.has(SkillFn::Start, EXAMPLE_DO_SLOT));
}

#[test]
fn options_choose_the_source_layers_and_saves() {
    let args = |a: &[&str]| Options::parse(a.iter().map(|s| s.to_string()));
    let o = args(&[
        "--native", "n", "--layer", "a", "--layer", "b", "--saves", "s",
    ])
    .unwrap();
    assert_eq!(
        o.source,
        GameSource::Native("n".into(), Layers::List(vec!["a".into(), "b".into()]))
    );
    assert_eq!(o.saves, std::path::PathBuf::from("s"));
    let o = args(&["--game-dir", "g"]).unwrap();
    assert_eq!(o.source, GameSource::Archives("g".into()));
    assert!(args(&["--game-dir", "g", "--layer", "a"]).is_err());
    assert!(args(&[]).is_err());
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_example_game_runs_under_its_own_rules() {
    let dir = std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR");
    let data = GameData::open(&GameSource::Archives(dir.into())).expect("the install loads");
    let saves = std::env::temp_dir().join(format!("d2rust-mod-game-{}", std::process::id()));
    let b = builder(data, saves.clone());
    assert_eq!(b.saves.folder, saves);
    let clock = StepClock::default();
    let (mut link, _) = start_and_join(b, &clock).expect("started");
    assert!(run_frames(&mut link, &clock, 10).unwrap() > 0);
    assert_eq!(rules_name(&mut link).unwrap().as_deref(), Some(NAME));
}
