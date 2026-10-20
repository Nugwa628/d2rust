//! The whole tool against a real server process: the stand-in server
//! (`mp-test-synth`) on loopback. A passing scenario passes, and planted
//! failures are reported with the rule they break.

use std::path::{Path, PathBuf};

use d2_net::wire::{parse_attach_answer, parse_session_answer, Attach, ClientHello};
use d2_quic::{QuicClient, ServerTrust};
use mp_test::bot::BOT_TIMING;
use mp_test::report::Verdict;
use mp_test::runner::{self, Probe};
use mp_test::scenario::parse;
use mp_test::server::{ServerProc, Target};

fn target() -> Target {
    Target::Synth {
        bin: PathBuf::from(env!("CARGO_BIN_EXE_mp-test-synth")),
    }
}

fn work(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("mp-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn run(name: &str, text: &str) -> mp_test::report::ScenarioReport {
    let s = parse(Path::new(&format!("{name}.mps")), text).unwrap();
    runner::validate(&s).unwrap();
    runner::run(&s, &target(), work(name), 1)
}

#[test]
fn the_stand_in_announces_what_it_serves() {
    match runner::probe(&target(), work("probe")) {
        Probe::Serving(Some(caps)) => {
            assert!(
                caps.contains("connect") && caps.contains("game"),
                "{caps:?}"
            );
            assert!(!caps.contains("persist"), "the stand-in keeps nothing");
        }
        p => panic!("{p:?}"),
    }
}

#[test]
fn two_bots_see_each_other_and_a_leave() {
    let r = run(
        "pass",
        "rule SEE players see each other\n\
         bots 2\n\
         all connect\n\
         all enter g1 expect ok\n\
         all see expect players=2 within 5s\n\
         b1 leave\n\
         b0 see expect players=1 b1=no within 5s\n",
    );
    assert_eq!(r.verdict, Verdict::Pass, "{:#?}", r.failures);
    assert_eq!(r.steps_run, 5);
}

/// A planted wrong expectation fails, naming its rule, the line and both
/// values.
#[test]
fn a_planted_wrong_expectation_names_its_rule() {
    let r = run(
        "planted",
        "rule SEE players see each other\n\
         rule COUNT a two-player game shows three players\n\
         bots 2\n\
         all connect\n\
         all enter g1\n\
         b0 see expect players=3 within 1s rule COUNT\n",
    );
    assert_eq!(r.verdict, Verdict::Fail);
    let f = &r.failures[0];
    assert_eq!(f.rule.id, "COUNT");
    assert_eq!(f.rule.text, "a two-player game shows three players");
    assert_eq!(f.line, 6);
    assert_eq!(f.bot, Some(0));
    assert_eq!(f.expected, "players=3");
    assert_eq!(f.actual, "ok players=2");
}

/// A version mismatch is refused with its reason over the real handshake.
#[test]
fn another_build_is_refused_with_its_reason() {
    let r = run(
        "build",
        "rule BUILD another build is refused\n\
         bots 2\n\
         b0 connect expect ok\n\
         b1 connect build=other expect refused=Build\n",
    );
    assert_eq!(r.verdict, Verdict::Pass, "{:#?}", r.failures);
    let r = run(
        "build-planted",
        "rule BUILD another build is let in\n\
         b0 connect build=other expect ok\n",
    );
    assert_eq!(r.verdict, Verdict::Fail);
    assert!(
        r.failures[0].actual.starts_with("refused=Build"),
        "{:?}",
        r.failures
    );
}

/// A connection the scenario didn't end, ending, breaks `core.conn-kept`;
/// a server killed by the scenario doesn't break `core.server-alive`.
#[test]
fn an_unasked_close_breaks_core_conn_kept() {
    let r = run(
        "unasked",
        "rule X a bot stays\n\
         b0 connect\n\
         server kill\n\
         b0 stats expect closed within 10s\n",
    );
    assert_eq!(
        r.verdict,
        Verdict::Pass,
        "asked for by the kill: {:#?}",
        r.failures
    );
    let r = run(
        "unasked-cut",
        "rule X a bot stays\n\
         bots 2\n\
         all connect\n\
         net b1 cut=on\n\
         net b1 cut=off\n\
         b0 stats expect connected=yes\n\
         wait 5s\n",
    );
    // The cut was asked for: b1's close is the scenario's own doing.
    assert!(
        r.failures.iter().all(|f| f.rule.id != "core.conn-kept"),
        "{:#?}",
        r.failures
    );
}

/// A lost player comes back with its token to its own slot.
#[test]
fn a_lost_player_resumes_its_slot() {
    let r = run(
        "resume",
        "rule RESUME a player that comes back gets its slot\n\
         bots 2\n\
         server grace=20000\n\
         all connect\n\
         all enter g1\n\
         net b1 cut=on\n\
         b1 stats expect closed within 10s\n\
         net b1 cut=off\n\
         b1 reconnect expect ok same=yes\n\
         b0 see expect players=2 b1=yes within 5s\n",
    );
    assert_eq!(r.verdict, Verdict::Pass, "{:#?}", r.failures);
}

/// The stand-in speaks wire version 4 (contracts 3.1, 3.4): a session
/// hello gets a session welcome, the game stream starts with an attach,
/// and a hello of the previous wire version is refused as `WireVersion`.
#[test]
fn the_stand_in_speaks_wire_v4() {
    let mut server = ServerProc::new(target(), work("wire"), Vec::new()).unwrap();
    let ready = match server.start() {
        Ok(r) => r.clone(),
        Err(e) => panic!("{e}"),
    };
    let hello = ClientHello {
        protocol: d2_proto::PROTOCOL_VERSION,
        game: "d2rust".into(),
        build: "0.0.0".into(),
        tick_rate: d2_sim::tick::TickRate::BASE.hz(),
        resume: None,
    };
    let (client, mut control) =
        QuicClient::connect(ready.listen, ServerTrust::Any, BOT_TIMING).unwrap();
    control.send(hello.body()).unwrap();
    let body = control
        .recv_timeout(BOT_TIMING.timeout)
        .unwrap()
        .expect("a session welcome");
    let welcome = parse_session_answer(&body).unwrap();
    assert_eq!(
        welcome.max_players,
        d2_server::adapters::session_flow::DEFAULT_MAX_PLAYERS as u32
    );
    let mut game = client.open_stream().unwrap();
    game.send(Attach::Ticket([0; 16]).body()).unwrap();
    let body = game
        .recv_timeout(BOT_TIMING.timeout)
        .unwrap()
        .expect("a game welcome");
    let game = parse_attach_answer(&body).unwrap();
    assert_eq!(game.client, 0);
    assert_eq!(game.tick_rate, hello.tick_rate);

    let (client, mut control) =
        QuicClient::connect(ready.listen, ServerTrust::Any, BOT_TIMING).unwrap();
    let mut previous = hello.body();
    previous[5..7].copy_from_slice(&3u16.to_le_bytes());
    control.send(previous).unwrap();
    let body = control
        .recv_timeout(BOT_TIMING.timeout)
        .unwrap()
        .expect("a refusal");
    let refused = parse_session_answer(&body).unwrap_err();
    assert_eq!(refused.code.name(), "WireVersion");
    drop(client);
    server.stop();
}
