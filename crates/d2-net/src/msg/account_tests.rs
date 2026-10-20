//! The account messages' round trips and their decode rules.

use super::*;
use crate::wire::MAX_FRAME;

fn summary() -> CharSummary {
    CharSummary {
        name: "Kashya".into(),
        class: 0,
        level: 31,
        status: 0x0020 | (5 << 8),
        difficulties: 0b011,
        look: (0..32).collect(),
        last_played: 1_792_000_000,
    }
}

#[test]
fn requests_round_trip() {
    for m in [
        AccountRequest::Register {
            account: "Mogger".into(),
            password: "correct horse".into(),
        },
        AccountRequest::Login {
            account: "mogger".into(),
            password: "correct horse".into(),
        },
        AccountRequest::Logout,
        AccountRequest::CharList,
        AccountRequest::CharCreate {
            name: "Ama-Zon".into(),
            class: 6,
            hardcore: true,
            expansion: true,
        },
        AccountRequest::CharDelete {
            name: "Ama-Zon".into(),
            password: "correct horse".into(),
        },
        AccountRequest::CharSelect {
            name: "Kashya".into(),
        },
    ] {
        let body = m.body();
        assert_eq!(body[0], m.kind());
        assert!(super::super::kind::ACCOUNT.contains(&body[0]));
        assert_eq!(AccountRequest::parse(&body), Ok(m));
    }
}

#[test]
fn events_round_trip() {
    for m in [
        AccountEvent::LoggedIn {
            account: "Mogger".into(),
            max_characters: 16,
        },
        AccountEvent::CharList(vec![]),
        AccountEvent::CharList(vec![summary(), summary()]),
        AccountEvent::CharCreated(summary()),
        AccountEvent::CharDeleted {
            name: "Kashya".into(),
        },
        AccountEvent::CharSelected(summary()),
    ] {
        let body = m.body();
        assert_eq!(body[0], m.kind());
        assert!(super::super::kind::ACCOUNT.contains(&body[0]));
        assert_eq!(AccountEvent::parse(&body), Ok(m));
    }
}

#[test]
fn only_register_and_login_work_before_login() {
    let before: Vec<bool> = [
        AccountRequest::Register {
            account: "a".into(),
            password: "b".into(),
        },
        AccountRequest::Login {
            account: "a".into(),
            password: "b".into(),
        },
        AccountRequest::Logout,
        AccountRequest::CharList,
        AccountRequest::CharSelect { name: "c".into() },
    ]
    .iter()
    .map(AccountRequest::before_login)
    .collect();
    assert_eq!(before, [true, true, false, false, false]);
}

#[test]
fn debug_hides_passwords() {
    let secret = "hunter2-secret";
    for m in [
        AccountRequest::Register {
            account: "Mogger".into(),
            password: secret.into(),
        },
        AccountRequest::Login {
            account: "Mogger".into(),
            password: secret.into(),
        },
        AccountRequest::CharDelete {
            name: "Kashya".into(),
            password: secret.into(),
        },
    ] {
        let shown = format!("{m:?}");
        assert!(!shown.contains(secret), "{shown}");
        assert!(shown.contains("<hidden>"), "{shown}");
    }
}

#[test]
fn rule_breaking_names_and_passwords_still_decode() {
    // The server answers these with the rule they break, not as malformed.
    let m = AccountRequest::Register {
        account: "x".repeat(20),
        password: "short".into(),
    };
    assert_eq!(AccountRequest::parse(&m.body()), Ok(m));
    let m = AccountRequest::Register {
        account: "Mogger".into(),
        password: "p".repeat(100),
    };
    assert_eq!(AccountRequest::parse(&m.body()), Ok(m));
}

#[test]
fn bad_fields_are_malformed() {
    // Class 7 doesn't exist.
    let mut body = AccountRequest::CharCreate {
        name: "Kashya".into(),
        class: 0,
        hardcore: false,
        expansion: true,
    }
    .body();
    body[1 + 1 + 6] = CLASSES;
    assert_eq!(AccountRequest::parse(&body), Err(NetError::Malformed));

    // Over-long account name.
    let mut w = Writer::new(kind::LOGIN);
    w.str8(&"a".repeat(MAX_ACCOUNT_WIRE + 1)).str8("password");
    assert_eq!(AccountRequest::parse(&w.finish()), Err(NetError::Malformed));

    // Trailing bytes.
    let mut body = AccountRequest::CharList.body();
    body.push(0);
    assert_eq!(AccountRequest::parse(&body), Err(NetError::Malformed));

    // A difficulty bit past Hell.
    let mut s = summary();
    s.difficulties = 0b1001;
    let body = AccountEvent::CharSelected(s).body();
    assert_eq!(AccountEvent::parse(&body), Err(NetError::Malformed));

    // An empty character name in a summary.
    let mut s = summary();
    s.name.clear();
    let body = AccountEvent::CharCreated(s).body();
    assert_eq!(AccountEvent::parse(&body), Err(NetError::Malformed));

    // A list count past the limit.
    let body = vec![kind::CHAR_LIST_ANSWER, MAX_CHARACTERS_LIMIT as u8 + 1];
    assert_eq!(AccountEvent::parse(&body), Err(NetError::Malformed));

    // Kinds of other areas.
    assert_eq!(AccountRequest::parse(&[0x40]), Err(NetError::Malformed));
    assert_eq!(
        AccountEvent::parse(&[kind::LOGIN]),
        Err(NetError::Malformed)
    );
}

#[test]
fn the_longest_character_list_fits_one_frame() {
    let longest = CharSummary {
        name: "n".repeat(MAX_CHAR_NAME_WIRE),
        look: vec![0xFF; MAX_LOOK],
        ..summary()
    };
    let m = AccountEvent::CharList(vec![longest; MAX_CHARACTERS_LIMIT]);
    let body = m.body();
    assert!(body.len() <= MAX_FRAME, "{} bytes", body.len());
    assert_eq!(AccountEvent::parse(&body), Ok(m));
}

#[test]
fn a_list_past_the_limit_is_cut() {
    let m = AccountEvent::CharList(vec![summary(); MAX_CHARACTERS_LIMIT + 3]);
    match AccountEvent::parse(&m.body()) {
        Ok(AccountEvent::CharList(v)) => assert_eq!(v.len(), MAX_CHARACTERS_LIMIT),
        other => panic!("{other:?}"),
    }
}
