//! The binary: `--check` reads the config, opens the folders and the log;
//! bad configs and missing game data fail with a message.

mod support;

use std::process::Command;

use d2rust_server::log::FILE_NAME;

fn server() -> Command {
    Command::new(env!("CARGO_BIN_EXE_d2rust-server"))
}

fn check(cfg: &std::path::Path) -> std::process::Output {
    server()
        .arg("--config")
        .arg(cfg)
        .arg("--check")
        .output()
        .unwrap()
}

#[test]
fn check_opens_the_folders_and_logs() {
    let dir = support::temp_dir("bin-check");
    std::fs::create_dir_all(dir.join("d2")).unwrap();
    let cfg = dir.join("d2rust-server.cfg");
    std::fs::write(&cfg, "listen = 127.0.0.1:7000\ngame_data = d2\n").unwrap();
    let out = check(&cfg);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("127.0.0.1:7000"), "{stdout}");
    // The accounts' data folder (database and characters) is opened.
    assert!(dir.join("data").is_dir(), "{stdout}");
    let log = std::fs::read_to_string(dir.join("logs").join(FILE_NAME)).unwrap();
    assert!(log.contains("listen 127.0.0.1:7000"), "{log}");
}

#[test]
fn missing_game_data_fails() {
    let dir = support::temp_dir("bin-nodata");
    let cfg = dir.join("d2rust-server.cfg");
    std::fs::write(&cfg, "game_data = missing\n").unwrap();
    let out = check(&cfg);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("doesn't exist"));
}

#[test]
fn a_bad_config_names_its_line() {
    let dir = support::temp_dir("bin-bad");
    let cfg = dir.join("d2rust-server.cfg");
    std::fs::write(&cfg, "game_data = d2\nmax_players = 99\n").unwrap();
    let out = check(&cfg);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("line 2"));
}

#[test]
fn bad_arguments_print_the_usage() {
    let out = server().arg("--nope").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
    // --set-progression needs a character and a number.
    let out = server()
        .args(["--set-progression", "Nobody"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
}

/// `--set-progression` is an offline admin command: it opens the accounts
/// and names the character it can't find.
#[test]
fn set_progression_names_the_character() {
    let dir = support::temp_dir("bin-prog");
    let cfg = dir.join("d2rust-server.cfg");
    std::fs::write(&cfg, "game_data = d2\n").unwrap();
    let out = server()
        .arg("--config")
        .arg(&cfg)
        .args(["--set-progression", "Nobody", "5"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("no character named"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The server runs: it prints its ready line, takes a client's hello,
/// writes `status.json` and stops cleanly on SIGTERM.
#[cfg(unix)]
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn the_server_runs_answers_and_stops_on_sigterm() {
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    let dir = support::temp_dir("bin-run");
    let game = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let cfg = dir.join("d2rust-server.cfg");
    std::fs::write(
        &cfg,
        format!("listen = 127.0.0.1:0\ngame_data = {game}\nargon2 = test\nserver_name = Test\n"),
    )
    .unwrap();
    let mut child = server()
        .arg("--config")
        .arg(&cfg)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let ready = lines.next().expect("a ready line").unwrap();
    let field = |k: &str| {
        ready
            .split(' ')
            .find_map(|f| f.strip_prefix(&format!("{k}=")))
            .unwrap_or_else(|| panic!("{k} in {ready}"))
            .to_owned()
    };
    assert!(ready.starts_with("ready "), "{ready}");
    let caps = field("caps");
    assert!(caps.split(',').any(|c| c == "accounts"), "{ready}");
    let addr: std::net::SocketAddr = field("listen").parse().unwrap();
    let cert = d2_quic::parse_fingerprint(&field("cert_sha256")).unwrap();
    let hello = d2_net::wire::ClientHello {
        protocol: field("protocol").parse().unwrap(),
        game: field("game"),
        build: field("build"),
        tick_rate: field("tick").parse().unwrap(),
        resume: None,
    };
    let (_client, mut control) = d2_quic::QuicClient::connect(
        addr,
        d2_quic::ServerTrust::Pinned(cert),
        d2_net::wire::Timing::default(),
    )
    .unwrap();
    control.send(hello.body()).unwrap();
    let answer = control
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .expect("a welcome");
    let welcome = d2_net::wire::parse_session_answer(&answer).unwrap();
    assert_eq!(welcome.server_name, "Test");
    assert!(welcome.registration_open);
    // The health file comes within its interval.
    let status = dir.join("logs").join("status.json");
    let until = Instant::now() + Duration::from_secs(20);
    while !status.exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    let text = std::fs::read_to_string(&status).expect("status.json");
    assert!(text.contains("\"sessions\": 1"), "{text}");
    assert!(dir.join("data").join("server.lock").exists());
    // SIGTERM: a clean stop.
    let pid = child.id().to_string();
    assert!(std::process::Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .unwrap()
        .success());
    let until = Instant::now() + Duration::from_secs(60);
    let code = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(Instant::now() < until, "the server did not stop");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(code.success(), "{code:?}");
    let log = std::fs::read_to_string(dir.join("logs").join(FILE_NAME)).unwrap();
    assert!(log.contains("stopping (SIGTERM)"), "{log}");
    assert!(log.contains("stopped"), "{log}");
    assert!(
        !dir.join("data").join("server.lock").exists(),
        "the lock is gone"
    );
}

#[test]
fn version_prints_the_game_version() {
    let out = server().arg("--version").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    let v = text
        .trim()
        .strip_prefix("d2rust-server ")
        .expect("the name");
    assert_eq!(v.split('.').count(), 4, "{text}");
}
