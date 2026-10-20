//! The scenario format: what parses, what is refused with its line, and
//! that every shipped scenario parses and uses known verbs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use mp_test::runner;
use mp_test::scenario::{parse, parse_duration, Op, Pred, Who};

fn p(text: &str) -> Result<mp_test::scenario::Scenario, String> {
    parse(Path::new("t.mps"), text).map_err(|e| e.to_string())
}

#[test]
fn a_full_scenario_parses() {
    let s = p("# comment\n\
         name demo\n\
         about Shows things.\n\
         rule A  first rule text\n\
         rule B  second rule\n\
         bots 3\n\
         server players=2 grace=500\n\
         net latency=20\n\
         seed 9\n\
         all connect\n\
         b1-2 see expect players=2 b0=yes x>=1 y<=9 z!=0 within 2s rule A   # trailing\n\
         net b1 cut=on\n\
         wait 250ms\n\
         server restart\n\
         b0 connect build=x expect refused=Build rule B\n")
    .unwrap();
    assert_eq!(s.name, "demo");
    assert_eq!(s.bots, 3);
    assert_eq!(s.rules.len(), 2);
    assert_eq!(s.rules[0].text, "first rule text");
    assert_eq!(
        s.server,
        vec![
            ("players".into(), "2".into()),
            ("grace".into(), "500".into())
        ]
    );
    assert_eq!(s.net, vec![("latency".into(), "20".into())]);
    assert_eq!(s.seed, 9);
    assert_eq!(s.steps.len(), 6);
    assert_eq!(s.steps[0].who, Who::Bots(vec![0, 1, 2]));
    let see = &s.steps[1];
    assert_eq!(see.who, Who::Bots(vec![1, 2]));
    assert_eq!(see.verb, "see");
    assert_eq!(see.within, Some(Duration::from_secs(2)));
    assert_eq!(see.rule.as_deref(), Some("A"));
    assert_eq!(
        see.expect[0],
        Pred::Fact {
            key: "players".into(),
            op: Op::Eq,
            value: "2".into()
        }
    );
    assert!(matches!(&see.expect[2], Pred::Fact { op: Op::Ge, .. }));
    assert!(matches!(&see.expect[3], Pred::Fact { op: Op::Le, .. }));
    assert!(matches!(&see.expect[4], Pred::Fact { op: Op::Ne, .. }));
    assert_eq!(s.steps[2].who, Who::Net(vec![1]));
    assert_eq!(s.steps[2].args, vec!["cut=on"]);
    assert_eq!(s.steps[3].who, Who::Wait);
    assert_eq!(s.steps[3].verb, "250ms");
    assert_eq!(s.steps[4].who, Who::Server);
    assert_eq!(s.steps[5].expect, vec![Pred::Refused(Some("Build".into()))]);
    assert_eq!(s.steps[5].args, vec!["build=x"]);
}

#[test]
fn bots_get_their_own_addresses_unless_the_scenario_shares_one() {
    let own = p("rule A x\nbots 2\nall connect\n").unwrap();
    assert!(!own.shared_address, "one address per bot by default");
    let per_bot = p("rule A x\naddresses per-bot\nbots 2\nall connect\n").unwrap();
    assert!(!per_bot.shared_address);
    let shared = p("rule A x\naddresses shared\nbots 2\nall connect\n").unwrap();
    assert!(shared.shared_address);
    let bad = p("rule A x\naddresses both\nbots 2\nall connect\n");
    assert!(bad.is_err(), "addresses takes per-bot or shared");
}

#[test]
fn errors_name_their_line() {
    let cases = [
        ("bots 2\nall connect\n", "states at least one rule"),
        (
            "rule A x\nrule A y\nb0 connect\n",
            "t.mps:2: rule A is stated twice",
        ),
        ("rule A x\nbots 65\n", "t.mps:2: bots takes"),
        ("rule A x\nbots 2\nb2 connect\n", "t.mps:3: `b2`"),
        (
            "rule A x\nb0 connect rule Z\n",
            "t.mps:2: rule Z is not stated",
        ),
        (
            "rule A x\nrule B y\nb0 see expect players=1\n",
            "t.mps:3: a step with expect names its rule",
        ),
        ("rule A x\nserver explode\n", "t.mps:2: server takes"),
        ("rule A x\nserver admin\n", "t.mps:2: server takes"),
        ("rule A x\nwait soon\n", "t.mps:2: wait takes a duration"),
        ("rule A x\nb0 see expect x>=many\n", "not a number"),
        ("rule A x\nfrobnicate\n", "is not a header"),
        ("rule A x\n", "has steps"),
    ];
    for (text, want) in cases {
        let e = p(text).unwrap_err();
        assert!(e.contains(want), "{text:?}: {e}");
    }
}

#[test]
fn a_setup_step_with_plain_ok_needs_no_rule() {
    let s = p("rule A x\nrule B y\nb0 connect expect ok\n").unwrap();
    assert!(s.rule_of(&s.steps[0]).is_none(), "setup: core.step");
}

#[test]
fn a_single_rule_is_every_expecting_step_s_rule() {
    let s = p("rule ONLY the rule\nb0 connect\nb0 see expect players=1\n").unwrap();
    assert!(s.rule_of(&s.steps[0]).is_none(), "no expectation: setup");
    assert_eq!(s.rule_of(&s.steps[1]).map(|r| r.id.as_str()), Some("ONLY"));
}

#[test]
fn durations() {
    assert_eq!(parse_duration("250ms"), Some(Duration::from_millis(250)));
    assert_eq!(parse_duration("3s"), Some(Duration::from_secs(3)));
    assert_eq!(parse_duration("1.5s"), Some(Duration::from_millis(1500)));
    assert_eq!(parse_duration("3"), None);
    assert_eq!(parse_duration("-1s"), None);
}

fn shipped() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/mp-test/scenarios");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|f| f.extension().is_some_and(|x| x == "mps"))
        .collect();
    v.sort();
    v
}

/// Every scenario in `scenarios/` parses and uses known verbs, and the
/// owner's listed scenario families are all there.
#[test]
fn every_shipped_scenario_parses() {
    let files = shipped();
    let mut names = Vec::new();
    for f in &files {
        let s = mp_test::scenario::load(f).unwrap_or_else(|e| panic!("{e}"));
        runner::validate(&s).unwrap_or_else(|e| panic!("{e}"));
        names.push(s.name);
    }
    for want in [
        "accounts-register-login",
        "accounts-rate-limit",
        "characters",
        "difficulty-rule",
        "lobby-passwords",
        "lobby-game-list",
        "join-leave",
        "move-together",
        "move-under-lag",
        "accounts-session-takeover",
        "fight-same-monster",
        "party",
        "trade",
        "items-drop-pickup",
        "waypoints-acts",
        "reconnect",
        "server-restart-persistence",
        "version-mismatch",
        "mod-mismatch",
        "net-conditions",
        "sixty-four",
    ] {
        assert!(names.iter().any(|n| n == want), "missing scenario {want}");
    }
}

#[test]
fn an_unknown_verb_is_refused() {
    let s = p("rule A x\nb0 teleport\n").unwrap();
    let e = runner::validate(&s).unwrap_err();
    assert!(e.contains("t.mps:2: unknown verb teleport"), "{e}");
}
