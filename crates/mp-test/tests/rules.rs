//! The judging side: expectations against outcomes, pending decisions,
//! the ready line, and the report naming each broken rule.

use std::collections::BTreeSet;
use std::path::Path;

use mp_test::bot::{Outcome, Status};
use mp_test::report::{Failure, Report, RuleRef, ScenarioReport, Verdict};
use mp_test::runner::{check, pending_reason, Probe};
use mp_test::scenario::{parse, Pred};
use mp_test::server::parse_ready;

fn preds(s: &str) -> Vec<Pred> {
    let sc = parse(
        Path::new("t.mps"),
        &format!("rule A x\nb0 see expect {s}\n"),
    )
    .unwrap();
    sc.steps[0].expect.clone()
}

#[test]
fn expectations_hold_and_break() {
    let o = Outcome::ok().fact("players", 2).fact("b1", "yes");
    assert!(check(&preds("players=2 b1=yes"), &o).is_ok());
    assert!(check(&preds("players>=2 players<=2 b1!=no"), &o).is_ok());
    let e = check(&preds("players=3"), &o).unwrap_err();
    assert_eq!(e, "ok players=2", "the actual value is shown");
    let e = check(&preds("nothing=1"), &o).unwrap_err();
    assert!(e.contains("no fact nothing"), "{e}");
    // Facts alone also need the step to have worked.
    let closed = Outcome {
        status: Status::Closed("timed out".into()),
        ..Outcome::ok().fact("players", 2)
    };
    assert!(check(&preds("players=2"), &closed).is_err());
    assert!(check(&preds("closed"), &closed).is_ok());
    assert!(check(&[], &closed).is_err(), "no expectation: must be ok");
    assert!(check(&[], &Outcome::ok()).is_ok());
}

#[test]
fn refusal_codes_are_matched() {
    let r = Outcome {
        status: Status::Refused {
            code: "build".into(),
            text: "game build x, server y".into(),
        },
        ..Outcome::ok()
    };
    assert!(check(&preds("refused"), &r).is_ok());
    assert!(check(&preds("refused=build"), &r).is_ok());
    assert!(check(&preds("refused=protocol"), &r).is_err());
    assert!(check(&preds("ok"), &r).is_err());
    assert!(check(&preds("error"), &Outcome::error("x")).is_ok());
}

fn scenario(text: &str) -> mp_test::scenario::Scenario {
    parse(Path::new("t.mps"), text).unwrap()
}

#[test]
fn unbuilt_verbs_make_a_scenario_pending() {
    let s = scenario("rule A x\nb0 connect\nb0 party-invite b1\nb0 party expect members=2\n");
    let why = pending_reason(&s, &Probe::Serving(None)).unwrap();
    assert!(
        why.contains("no driver yet for: party-invite, party"),
        "{why}"
    );
}

#[test]
fn a_server_that_cant_run_games_makes_it_pending() {
    let s = scenario("rule A x\nb0 connect\n");
    let why = pending_reason(
        &s,
        &Probe::NotServing("running games is not built yet (use --check)".into()),
    )
    .unwrap();
    assert!(why.contains("can't run games yet"), "{why}");
    assert_eq!(pending_reason(&s, &Probe::Serving(None)), None);
}

#[test]
fn capabilities_the_server_lacks_make_it_pending() {
    let caps: BTreeSet<String> = ["connect", "game"].iter().map(|s| s.to_string()).collect();
    let s = scenario("rule A x\nneeds persist\nb0 connect\nserver restart\n");
    let why = pending_reason(&s, &Probe::Serving(Some(caps.clone()))).unwrap();
    assert!(why.contains("doesn't serve: persist"), "{why}");
    // A server that names nothing still has to name the server-only ones.
    let why = pending_reason(&s, &Probe::Serving(None)).unwrap();
    assert!(why.contains("doesn't serve: persist"), "{why}");
    let s = scenario(
        "rule A x\nb0 connect\nserver admin --set-progression X 5\nserver dev bot0 acts\nserver progression X 5\n",
    );
    let why = pending_reason(&s, &Probe::Serving(None)).unwrap();
    assert!(why.contains("doesn't serve: admin, dev"), "{why}");
    let s = scenario("rule A x\nb0 connect\nb0 enter g\n");
    assert_eq!(pending_reason(&s, &Probe::Serving(Some(caps))), None);
    let s = scenario("rule A x\nb0 connect game=other expect refused=WrongGame\n");
    let why = pending_reason(&s, &Probe::Serving(None)).unwrap();
    assert!(why.contains("can't drive: mods"), "{why}");
    let s = scenario("rule A x\npending waits for the lobby\nb0 connect\n");
    assert_eq!(
        pending_reason(&s, &Probe::Serving(None)).as_deref(),
        Some("waits for the lobby")
    );
}

#[test]
fn a_lobby_server_takes_no_direct_enter() {
    let caps: BTreeSet<String> = ["connect", "game", "accounts", "characters", "lobby"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let s = scenario("rule A x\nb0 connect\nb0 enter g\n");
    let why = pending_reason(&s, &Probe::Serving(Some(caps.clone()))).unwrap();
    assert!(why.contains("through its lobby"), "{why}");
    let s = scenario("rule A x\nb0 connect\nb0 create-game g expect ok\n");
    assert_eq!(pending_reason(&s, &Probe::Serving(Some(caps))), None);
}

#[test]
fn difficulties_read_as_scenarios_say() {
    use mp_test::bot::{difficulty_name, difficulty_number};
    assert_eq!(difficulty_number("normal"), Some(0));
    assert_eq!(difficulty_number("nightmare"), Some(1));
    assert_eq!(difficulty_number("hell"), Some(2));
    assert_eq!(difficulty_number("3"), None);
    for d in 0..3 {
        assert_eq!(difficulty_number(difficulty_name(d)), Some(d));
    }
}

#[test]
fn the_ready_line() {
    let fp = "ab".repeat(32);
    let line = format!("ready listen=127.0.0.1:5000 cert=/x/c.der cert_sha256={fp} game=d2rust caps=connect,game build=0.1 protocol=2 tick=60");
    let r = parse_ready(&line).unwrap().unwrap();
    assert_eq!(r.listen.port(), 5000);
    assert_eq!(r.cert.unwrap().to_string_lossy(), "/x/c.der");
    assert_eq!(r.caps.unwrap().len(), 2);
    assert_eq!(r.game.as_deref(), Some("d2rust"));
    assert_eq!(r.build.as_deref(), Some("0.1"));
    assert_eq!(r.protocol, Some(2));
    assert_eq!(r.tick, Some(60));
    assert_eq!(r.cert_sha256, Some([0xAB; 32]));
    let bare = parse_ready("ready listen=[::1]:7").unwrap().unwrap();
    assert_eq!((bare.cert, bare.caps), (None, None));
    assert!(parse_ready("ready cert=x").unwrap().is_err());
    assert!(parse_ready("listening on 5000").is_none());
}

#[test]
fn the_report_names_each_broken_rule() {
    let failure = Failure {
        rule: RuleRef {
            id: "LOGIN".into(),
            text: "an account logs in with its password and only with its password".into(),
        },
        line: 12,
        step: "b1 login alice wrong expect refused=bad-password rule LOGIN".into(),
        bot: Some(1),
        expected: "refused=bad-password".into(),
        actual: "ok".into(),
    };
    let sc = |name: &str, verdict, failures: Vec<Failure>| ScenarioReport {
        name: name.into(),
        file: format!("scenarios/{name}.mps"),
        about: String::new(),
        verdict,
        pending: (verdict == Verdict::Pending)
            .then(|| "mp-test has no driver yet for: register".into()),
        needs: vec!["accounts".into()],
        bots: 2,
        steps_run: 4,
        ms: 1500,
        failures,
        datagrams: (100, 5),
        server_tail: None,
    };
    let report = Report {
        tool: "mp-test 0".into(),
        server: "synth".into(),
        server_bin: "mp-test-synth".into(),
        server_caps: Some(vec!["connect".into(), "game".into()]),
        seed: 1,
        passed: 1,
        failed: 1,
        pending: 1,
        scenarios: vec![
            sc("ok-one", Verdict::Pass, vec![]),
            sc("bad-one", Verdict::Fail, vec![failure]),
            sc("later-one", Verdict::Pending, vec![]),
        ],
    };
    let md = report.markdown();
    assert!(md.contains("1 passed, 1 failed, 1 pending"), "{md}");
    assert!(
        md.contains("**LOGIN**: an account logs in with its password and only with its password"),
        "{md}"
    );
    assert!(md.contains("line 12 b1"), "{md}");
    assert!(md.contains("expected: refused=bad-password"), "{md}");
    assert!(md.contains("| later-one | pending |"), "{md}");
    let json = report.to_json();
    let f = &json["scenarios"][1]["failures"][0];
    assert_eq!(f["rule"]["id"], "LOGIN");
    assert_eq!(f["line"], 12);
    assert_eq!(json["scenarios"][2]["verdict"], "pending");
    assert_eq!(json["format"], 1);
}
