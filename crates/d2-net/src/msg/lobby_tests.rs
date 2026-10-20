//! The lobby messages' round trips and their decode rules.

use super::*;

fn game() -> GameInfo {
    GameInfo {
        game_id: 7,
        name: "baal run".into(),
        difficulty: 2,
        players: 3,
        max_players: 8,
        locked: true,
        hardcore: false,
        expansion: true,
        joinable: ErrorCode::DIFFICULTY_LOCKED.0,
    }
}

#[test]
fn requests_round_trip() {
    for m in [
        LobbyRequest::ListGames,
        LobbyRequest::CreateGame {
            name: "cows".into(),
            password: String::new(),
            difficulty: 1,
            max_players: 0,
        },
        LobbyRequest::JoinGame {
            game_id: 9,
            password: "moo".into(),
        },
        LobbyRequest::LeaveLobby,
        LobbyRequest::Chat {
            text: "hello there".into(),
        },
    ] {
        let body = m.body();
        assert_eq!(body[0], m.kind());
        assert!(super::super::kind::LOBBY.contains(&body[0]));
        assert_eq!(LobbyRequest::parse(&body).unwrap(), m);
    }
}

#[test]
fn events_round_trip() {
    for m in [
        LobbyEvent::GameListBegin { count: 2 },
        LobbyEvent::GameEntry(game()),
        LobbyEvent::GameListEnd,
        LobbyEvent::GameGone { game_id: 7 },
        LobbyEvent::JoinOk {
            game_id: 7,
            ticket: [3; 16],
            difficulty: 2,
        },
        LobbyEvent::ChatLine {
            from: "Werwer".into(),
            text: "anyone for cows?".into(),
            at: 1_800_000_000,
        },
    ] {
        let body = m.body();
        assert_eq!(body[0], m.kind());
        assert_eq!(LobbyEvent::parse(&body).unwrap(), m);
    }
}

/// The exact layout of `CreateGame` (contracts 3.6): name, password,
/// difficulty, max players.
#[test]
fn create_game_layout() {
    let body = LobbyRequest::CreateGame {
        name: "ab".into(),
        password: "x".into(),
        difficulty: 2,
        max_players: 4,
    }
    .body();
    assert_eq!(body, [0x45, 2, b'a', b'b', 1, b'x', 2, 4]);
}

/// The list's entry never carries a password, only whether there is one.
#[test]
fn game_entry_layout_has_no_password() {
    let body = LobbyEvent::GameEntry(game()).body();
    let mut want = vec![0x42, 7, 0, 0, 0, 8];
    want.extend_from_slice(b"baal run");
    want.extend_from_slice(&[2, 3, 8, 1, 0, 1, 66, 0]);
    assert_eq!(body, want);
}

#[test]
fn decode_refuses_bad_fields() {
    // Over-long name and password.
    let long = LobbyRequest::CreateGame {
        name: "a".repeat(16),
        password: String::new(),
        difficulty: 0,
        max_players: 0,
    };
    assert!(LobbyRequest::parse(&long.body()).is_err());
    let long_pw = LobbyRequest::JoinGame {
        game_id: 1,
        password: "p".repeat(16),
    };
    assert!(LobbyRequest::parse(&long_pw.body()).is_err());
    // Empty and over-long chat.
    assert!(LobbyRequest::parse(&[kind::CHAT, 0]).is_err());
    let mut chat = vec![kind::CHAT, 201];
    chat.extend(std::iter::repeat_n(b'a', 201));
    assert!(LobbyRequest::parse(&chat).is_err());
    // Control characters.
    assert!(LobbyRequest::parse(&[kind::CHAT, 2, b'a', 7]).is_err());
    // Trailing bytes, unknown kinds, short bodies.
    assert!(LobbyRequest::parse(&[kind::LIST_GAMES, 0]).is_err());
    assert!(LobbyRequest::parse(&[0x5F]).is_err());
    assert!(LobbyRequest::parse(&[kind::JOIN_GAME, 1, 0]).is_err());
    assert!(LobbyEvent::parse(&[kind::GAME_GONE, 1]).is_err());
    // A difficulty past Hell.
    let mut bad = LobbyEvent::JoinOk {
        game_id: 1,
        ticket: [0; 16],
        difficulty: 0,
    }
    .body();
    *bad.last_mut().unwrap() = 3;
    assert!(LobbyEvent::parse(&bad).is_err());
}

#[test]
fn refusal_names_the_code() {
    assert_eq!(game().refusal(), Some(ErrorCode::DIFFICULTY_LOCKED));
    assert!(!game().can_join());
    let open = GameInfo {
        joinable: 0,
        ..game()
    };
    assert!(open.can_join());
    assert_eq!(open.refusal(), None);
}

/// A password never reaches a log: `Debug` says only whether one is set.
#[test]
fn debug_hides_passwords() {
    let create = LobbyRequest::CreateGame {
        name: "cows".into(),
        password: "hunter2".into(),
        difficulty: 1,
        max_players: 0,
    };
    let text = format!("{create:?}");
    assert!(!text.contains("hunter2"), "{text}");
    assert!(text.contains("<redacted>"), "{text}");
    assert!(text.contains("cows"), "{text}");

    let join = LobbyRequest::JoinGame {
        game_id: 9,
        password: "swordfish".into(),
    };
    let text = format!("{join:?}");
    assert!(!text.contains("swordfish"), "{text}");
    assert!(text.contains("<redacted>"), "{text}");

    // An empty password stays visibly empty.
    let open = LobbyRequest::CreateGame {
        name: "cows".into(),
        password: String::new(),
        difficulty: 1,
        max_players: 0,
    };
    assert!(format!("{open:?}").contains("password: \"\""), "{open:?}");
}

/// A join ticket never reaches a log.
#[test]
fn debug_hides_tickets() {
    let ok = LobbyEvent::JoinOk {
        game_id: 7,
        ticket: [0xAB; 16],
        difficulty: 0,
    };
    let text = format!("{ok:?}");
    assert!(!text.contains("171"), "{text}");
    assert!(!text.contains("ab"), "{text}");
    assert!(text.contains("<redacted>"), "{text}");
    assert!(text.contains("game_id: 7"), "{text}");

    // The rest of the message is not secret.
    let chat = LobbyEvent::ChatLine {
        from: "Ann".into(),
        text: "hi".into(),
        at: 5,
    };
    let text = format!("{chat:?}");
    assert!(text.contains("hi") && text.contains("Ann"), "{text}");
}
