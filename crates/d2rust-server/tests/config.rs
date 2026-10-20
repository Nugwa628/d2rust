//! The config file: defaults, every key, relative paths, and the errors.

use std::path::Path;
use std::time::Duration;

use d2rust_server::config::{ConfigError, ServerConfig, MAX_PLAYERS_LIMIT};

fn parse(text: &str) -> Result<ServerConfig, ConfigError> {
    ServerConfig::parse(text, Path::new("srv"))
}

#[test]
fn only_game_data_is_required() {
    let c = parse("game_data = d2\n").unwrap();
    assert_eq!(c.listen, "0.0.0.0:6112".parse().unwrap());
    assert_eq!(c.max_players, 8);
    assert_eq!(c.game_data, Path::new("srv").join("d2"));
    assert_eq!(c.characters, Path::new("srv").join("characters"));
    assert_eq!(c.logs, Path::new("srv").join("logs"));
    assert_eq!(c.grace, Duration::from_secs(30));
    assert_eq!(c.frame_pause, Duration::from_millis(1));
    assert_eq!(parse("# nothing\n\n"), Err(ConfigError::NoGameData));
}

#[test]
fn every_key_is_read() {
    let c = parse(
        "# a server\n\
         listen = 127.0.0.1:7000\n\
         max_players = 64\n\
         game_data = d2\n\
         characters = chars\n\
         logs = out/logs\n\
         grace_s = 5\n\
         frame_ms = 0\n",
    )
    .unwrap();
    assert_eq!(c.listen, "127.0.0.1:7000".parse().unwrap());
    assert_eq!(c.max_players, MAX_PLAYERS_LIMIT);
    assert_eq!(c.characters, Path::new("srv").join("chars"));
    assert_eq!(c.logs, Path::new("srv").join("out/logs"));
    assert_eq!(c.grace, Duration::from_secs(5));
    assert_eq!(c.frame_pause, Duration::ZERO);
}

#[test]
fn an_absolute_path_stays_absolute() {
    let abs = std::env::temp_dir().join("d2-install");
    let c = parse(&format!("game_data = {}\n", abs.display())).unwrap();
    assert_eq!(c.game_data, abs);
}

#[test]
fn errors_name_their_line() {
    assert_eq!(
        parse("game_data = d2\nnonsense\n"),
        Err(ConfigError::Syntax { line: 2 })
    );
    assert_eq!(parse("game_data =\n"), Err(ConfigError::Syntax { line: 1 }));
    assert_eq!(
        parse("game_data = d2\nport = 1\n"),
        Err(ConfigError::UnknownKey {
            line: 2,
            key: "port".into()
        })
    );
    assert_eq!(
        parse("game_data = d2\ngame_data = d3\n"),
        Err(ConfigError::Repeated {
            line: 2,
            key: "game_data".into()
        })
    );
    for bad in [
        "max_players = 0",
        "max_players = 65",
        "max_players = eight",
        "listen = nowhere",
        "grace_s = -1",
        "frame_ms = 1001",
    ] {
        let e = parse(&format!("game_data = d2\n{bad}\n")).unwrap_err();
        assert!(
            matches!(e, ConfigError::Value { line: 2, .. }),
            "{bad}: {e}"
        );
    }
}

#[test]
fn a_missing_file_is_a_read_error() {
    let e = ServerConfig::load(Path::new("no/such/d2rust-server.cfg")).unwrap_err();
    assert!(matches!(e, ConfigError::Read { .. }), "{e}");
}
