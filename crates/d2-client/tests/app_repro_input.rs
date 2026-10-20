//! The input steps of a `repro` scenario end to end on the user's game
//! files: a skill cast by name at a unit, a walk held with the left button
//! through a right-button cast (the BUG-009 scenario), and `wait-until`
//! steps that pause the script until a check holds, or fail the run when
//! it never does.
//!
//! Real data (`D2_GAME_DIR`), so `#[ignore]`: the synthetic install has no
//! UI art for the client. The parsing and timing of every step are unit
//! tests in `world_view::input_script`.

use std::path::PathBuf;

use d2_client::app::repro::{self, exit, ReproReport, RunOpts};
use d2_client::scenario_script::Scenario;

fn run(tag: &str, s: &Scenario) -> ReproReport {
    let dir = PathBuf::from(std::env::var_os("D2_GAME_DIR").expect("D2_GAME_DIR"));
    let opts = RunOpts {
        out: PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("repro-input-{tag}-{}", std::process::id())),
        game_dir: Some(dir.clone()),
        base_dir: dir,
        file: format!("{tag}.scn"),
        keep_going: true,
        ..RunOpts::default()
    };
    repro::run(s, &opts).unwrap_or_else(|e| panic!("{e}"))
}

/// The outcomes of kind `kind` as (text, ok, result).
fn outcomes(r: &ReproReport, kind: &str) -> Vec<(String, bool, String)> {
    r.json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["kind"] == kind)
        .map(|s| {
            (
                s["text"].as_str().unwrap_or_default().to_owned(),
                s["ok"].as_bool().unwrap_or(false),
                s["result"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// A level 30 Sorceress with Fire Ball (no hand set) and a Fallen east of
/// her in the Blood Moor, then `extra`.
fn sorceress(extra: &str) -> Scenario {
    let text = format!(
        "scenario 2\ntick_rate 60\nname cast\nseed 1234\nend 40\nchar class 1\nchar level 30\n\
         char skill 47 20\nchar area 0 2\nat 0 dev god on\nat 1 spawn 19 @x+12 @y normal\n{extra}"
    );
    Scenario::parse(&text).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR"]
fn cast_by_name_at_a_unit_selects_the_skill_and_casts() {
    let s = sorceress(
        "at 8 input cast fireball at unit 1 *\n\
         by 20 check sent(0x3C) >= 1\n\
         by 30 check player.mode == cast\n",
    );
    let r = run("cast-unit", &s);
    assert_eq!(r.exit, exit::PASS, "{:#}", r.json);
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR"]
fn a_skill_the_player_lacks_fails_the_run() {
    // Skill 36 (Fire Bolt) is not in her list.
    let r = run(
        "cast-missing",
        &sorceress("at 8 input cast 36 at unit 1 *\n"),
    );
    assert_eq!(r.exit, exit::FAIL, "{:#}", r.json);
    let input = outcomes(&r, "input");
    assert!(
        input
            .iter()
            .any(|(t, ok, _)| !ok && t.starts_with("cast 36 at unit")),
        "{input:?}"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR"]
fn wait_until_waits_for_the_cast_and_fails_when_it_never_comes() {
    let r = run(
        "wait-cast",
        &sorceress(
            "at 8 input cast fireball at unit 1 *\n\
             at 9 input wait-until player.mode == cast max 20\n",
        ),
    );
    assert_eq!(r.exit, exit::PASS, "{:#}", r.json);
    let input = outcomes(&r, "input");
    assert!(
        input.iter().any(|(t, ok, res)| *ok
            && t == "wait-until player.mode == cast max 20"
            && res == "held: \"cast\""),
        "{input:?}"
    );
    // No cast: the wait runs out and the script stops there.
    let r = run(
        "wait-never",
        &sorceress("at 9 input wait-until player.mode == cast max 5\nat 10 input click 400 300\n"),
    );
    assert_eq!(r.exit, exit::FAIL, "{:#}", r.json);
    let input = outcomes(&r, "input");
    assert!(
        input
            .iter()
            .any(|(t, ok, _)| !ok && t.contains("did not hold within 5 ticks")),
        "{input:?}"
    );
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR"]
fn bug_009_the_held_walk_resumes_after_a_cast() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/scenarios/bugs/BUG-009.scn"
    );
    let text = std::fs::read_to_string(path).expect("BUG-009.scn");
    let s = Scenario::parse(&text).expect("BUG-009.scn parses");
    let r = run("bug-009", &s);
    assert_eq!(r.exit, exit::PASS, "{:#}", r.json);
    let held: Vec<_> = outcomes(&r, "input")
        .into_iter()
        .filter(|(t, ok, _)| *ok && t.starts_with("wait-until"))
        .collect();
    assert_eq!(held.len(), 2, "{:#}", r.json);
}
