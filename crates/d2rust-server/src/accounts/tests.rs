//! The account rules end to end on a real database in a temp folder, at
//! the test hash cost.

use super::*;
use d2_formats::d2s::{status, Header};

fn temp(test: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("d2rust-accounts-tests")
        .join(format!("{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(dir: &Path) -> AccountsConfig {
    AccountsConfig {
        cost: HashCost::Test,
        fail_delay: Duration::ZERO,
        ..AccountsConfig::new(dir)
    }
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(f)
}

fn ip(n: u8) -> IpAddr {
    IpAddr::from([10, 0, 0, n])
}

const PW: &str = "correct horse";

fn new_char(name: &str, class: u8) -> NewChar {
    NewChar {
        name: name.into(),
        class,
        hardcore: false,
        expansion: true,
        progression: 0,
    }
}

/// A save record of `name` at `level`.
fn record(name: &str, class: u8, level: u8, status: u16) -> Vec<u8> {
    let mut h = Header {
        class,
        level,
        status,
        ..Header::default()
    };
    h.set_name(name.as_bytes()).unwrap();
    h.to_bytes().to_vec()
}

#[test]
fn register_then_login_in_any_case() {
    let dir = temp("login");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        let a = acc.register("Mogger", PW, ip(1)).await.unwrap();
        assert_eq!(a.name, "Mogger");
        let b = acc.login("mOGGER", PW, ip(1)).await.unwrap();
        assert_eq!(b, a, "the stored name, with its case");
        for (name, pw) in [
            ("Mogger", "correct horsf"),
            ("Nobody", PW),
            ("9bad", PW),
            ("Mogger", "short"),
        ] {
            let e = acc.login(name, pw, ip(2)).await.unwrap_err();
            assert_eq!(
                e.code,
                ErrorCode::WRONG_LOGIN,
                "{name}/{pw}: one answer for every failure"
            );
        }
    });
}

#[test]
fn accounts_survive_a_restart() {
    let dir = temp("restart");
    let id = {
        let acc = Accounts::open_with(config(&dir)).unwrap();
        run(acc.register("Mogger", PW, ip(1))).unwrap().id
    };
    let acc = Accounts::open_with(config(&dir)).unwrap();
    assert_eq!(run(acc.login("Mogger", PW, ip(1))).unwrap().id, id);
}

#[test]
fn register_rules() {
    let dir = temp("register-rules");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        let e = acc.register("1abc", PW, ip(1)).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::BAD_ACCOUNT_NAME);
        assert!(e.text.contains("start with a letter"), "{}", e.text);
        let e = acc.register("a", PW, ip(1)).await.unwrap_err();
        assert!(e.text.contains("2 to 15"), "{}", e.text);
        assert_eq!(
            acc.register("ab cd", PW, ip(1)).await.unwrap_err().code,
            ErrorCode::BAD_ACCOUNT_NAME
        );
        assert_eq!(
            acc.register("Mogger", "short", ip(1))
                .await
                .unwrap_err()
                .code,
            ErrorCode::BAD_PASSWORD
        );
        let long = "p".repeat(65);
        assert_eq!(
            acc.register("Mogger", &long, ip(1)).await.unwrap_err().code,
            ErrorCode::BAD_PASSWORD
        );
        acc.register("Mog-ger_1", PW, ip(1)).await.unwrap();
        let e = acc.register("MOG-GER_1", PW, ip(2)).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::ACCOUNT_NAME_TAKEN);
    });
    let mut closed = config(&dir);
    closed.registration_open = false;
    drop(acc);
    let acc = Accounts::open_with(closed).unwrap();
    let e = run(acc.register("Another", PW, ip(3))).unwrap_err();
    assert_eq!(e.code, ErrorCode::REGISTRATION_CLOSED);
    // Existing accounts still log in.
    run(acc.login("mog-ger_1", PW, ip(3))).unwrap();
}

#[test]
fn three_new_accounts_per_address_per_hour() {
    let dir = temp("register-limit");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        for i in 0..3 {
            acc.register(&format!("Acc{i}"), PW, ip(1)).await.unwrap();
        }
        let e = acc.register("Acc3", PW, ip(1)).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::RATE_LIMITED);
        assert!(e.wait_s > 3000, "{}", e.wait_s);
        acc.register("Acc3", PW, ip(2)).await.unwrap();
    });
}

#[test]
fn ten_logins_per_address_per_minute() {
    let dir = temp("login-limit");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        acc.register("Mogger", PW, ip(9)).await.unwrap();
        for _ in 0..10 {
            acc.login("Mogger", PW, ip(1)).await.unwrap();
        }
        let e = acc.login("Mogger", PW, ip(1)).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::RATE_LIMITED);
        assert!((1..=6).contains(&e.wait_s), "{}", e.wait_s);
        acc.login("Mogger", PW, ip(2)).await.unwrap();
    });
}

#[test]
fn ten_failed_logins_lock_the_account() {
    let dir = temp("lock");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        acc.register("Mogger", PW, ip(1)).await.unwrap();
        acc.register("Other", PW, ip(1)).await.unwrap();
        // From many addresses, so the per-address bucket isn't what stops them.
        for i in 0..10 {
            let e = acc
                .login("mogger", "wrong password", ip(10 + i))
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::WRONG_LOGIN);
        }
        let e = acc.login("Mogger", PW, ip(50)).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::ACCOUNT_LOCKED, "even the right password");
        assert!(e.wait_s > 800, "{}", e.wait_s);
        acc.login("Other", PW, ip(50)).await.unwrap();
        // An unknown name locks the same way: a lock tells nothing about
        // which names exist.
        for i in 0..10 {
            let _ = acc.login("Nobody", "wrong password", ip(100 + i)).await;
        }
        let e = acc.login("Nobody", PW, ip(150)).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::ACCOUNT_LOCKED);
    });
}

#[test]
fn failed_logins_wait_the_fixed_delay() {
    let dir = temp("delay");
    let mut cfg = config(&dir);
    cfg.fail_delay = Duration::from_millis(300);
    let acc = Accounts::open_with(cfg).unwrap();
    run(async {
        acc.register("Mogger", PW, ip(1)).await.unwrap();
        for (name, pw) in [("Mogger", "wrong password"), ("Nobody", PW), ("x", "y")] {
            let t = std::time::Instant::now();
            acc.login(name, pw, ip(2)).await.unwrap_err();
            assert!(
                t.elapsed() >= Duration::from_millis(300),
                "{name}: {:?}",
                t.elapsed()
            );
        }
        let t = std::time::Instant::now();
        acc.login("Mogger", PW, ip(2)).await.unwrap();
        assert!(
            t.elapsed() < Duration::from_millis(300),
            "a good login doesn't wait"
        );
    });
}

#[test]
fn disabled_accounts_and_password_reset() {
    let dir = temp("admin");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(acc.register("Mogger", PW, ip(1))).unwrap();
    acc.set_disabled("MOGGER", true).unwrap();
    assert_eq!(
        run(acc.login("Mogger", PW, ip(1))).unwrap_err().code,
        ErrorCode::ACCOUNT_DISABLED
    );
    assert_eq!(
        run(acc.login("Mogger", "wrong password", ip(1)))
            .unwrap_err()
            .code,
        ErrorCode::WRONG_LOGIN,
        "only the right password learns the account is disabled"
    );
    acc.set_disabled("Mogger", false).unwrap();
    run(acc.login("Mogger", PW, ip(1))).unwrap();

    acc.set_password("mogger", "new password").unwrap();
    assert_eq!(
        run(acc.login("Mogger", PW, ip(1))).unwrap_err().code,
        ErrorCode::WRONG_LOGIN
    );
    run(acc.login("Mogger", "new password", ip(1))).unwrap();
    assert!(acc.set_password("Mogger", "short").is_err());
    assert!(acc.set_password("Nobody", "long enough").is_err());
    assert!(acc.set_disabled("Nobody", true).is_err());
}

#[test]
fn characters_belong_to_their_account() {
    let dir = temp("chars");
    let mut cfg = config(&dir);
    cfg.max_characters = 2;
    let acc = Accounts::open_with(cfg).unwrap();
    run(async {
        let a = acc.register("Mogger", PW, ip(1)).await.unwrap().id;
        let b = acc.register("Friend", PW, ip(1)).await.unwrap().id;
        let s = acc
            .create_character(a, new_char("Kashya", 0))
            .await
            .unwrap();
        assert_eq!((s.level, s.difficulties, s.last_played), (1, 0b001, 0));
        assert_eq!(s.status, status::EXPANSION);
        acc.create_character(a, new_char("Akara", 1)).await.unwrap();
        let names: Vec<_> = acc
            .characters(a)
            .await
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["Kashya", "Akara"]);
        assert!(acc.characters(b).await.is_empty());

        let e = acc
            .create_character(a, new_char("Charsi", 2))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::CHAR_LIMIT);
        let e = acc
            .create_character(b, new_char("KASHYA", 2))
            .await
            .unwrap_err();
        assert_eq!(
            e.code,
            ErrorCode::CHAR_NAME_TAKEN,
            "names are unique on the server"
        );
        let e = acc
            .create_character(b, new_char("K4shya", 2))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::BAD_CHAR_NAME);
        assert!(e.text.contains("letters"), "{}", e.text);
        let mut classic_druid = new_char("Druid", 5);
        classic_druid.expansion = false;
        assert_eq!(
            acc.create_character(b, classic_druid)
                .await
                .unwrap_err()
                .code,
            ErrorCode::EXPANSION_CLASS
        );

        assert_eq!(acc.character(a, "kashya").await.unwrap().name, "Kashya");
        assert_eq!(
            acc.character(b, "Kashya").await.unwrap_err().code,
            ErrorCode::UNKNOWN_CHAR
        );
    });
}

#[test]
fn delete_needs_the_password_and_holds_the_name() {
    let dir = temp("delete");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        let a = acc.register("Mogger", PW, ip(1)).await.unwrap().id;
        let b = acc.register("Friend", PW, ip(1)).await.unwrap().id;
        acc.create_character(a, new_char("Kashya", 0))
            .await
            .unwrap();
        // A played character has a file.
        acc.character_store(a)
            .unwrap()
            .create("Kashya", &record("Kashya", 0, 5, status::EXPANSION))
            .unwrap();
        let file = acc.character_file(a, "kashya").unwrap();
        assert!(file.exists());

        let e = acc
            .delete_character(a, "Kashya", "wrong password")
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::WRONG_LOGIN);
        let e = acc.delete_character(b, "Kashya", PW).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::UNKNOWN_CHAR, "not this account's");
        acc.delete_character(a, "KASHYA", PW).await.unwrap();
        assert!(acc.characters(a).await.is_empty());
        assert!(!file.exists());
        assert_eq!(
            acc.character_file(a, "Kashya").unwrap_err().code,
            ErrorCode::UNKNOWN_CHAR,
            "a deleted character has no file"
        );
        let kept: Vec<_> = std::fs::read_dir(dir.join("data/deleted").join(a.0.to_string()))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(kept.len(), 1);
        assert!(
            kept[0].starts_with("Kashya-") && kept[0].ends_with(".d2rc"),
            "{kept:?}"
        );

        let e = acc
            .create_character(b, new_char("Kashya", 0))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::CHAR_NAME_TAKEN, "held for 7 days");
        assert_eq!(
            acc.delete_character(a, "Kashya", PW)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UNKNOWN_CHAR
        );
    });
}

#[test]
fn seat_saved_and_the_character_file() {
    let dir = temp("seat");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        let a = acc.register("Mogger", PW, ip(1)).await.unwrap().id;
        acc.create_character(a, new_char("Kashya", 6))
            .await
            .unwrap();
        let seat = acc.seat(a, "KASHYA").await.unwrap();
        assert_eq!(
            seat.new,
            Some(NewChar {
                name: "Kashya".into(),
                class: 6,
                hardcore: false,
                expansion: true,
                progression: 0
            })
        );
        assert_eq!(
            seat.file,
            dir.join("data/characters")
                .join(a.0.to_string())
                .join("Kashya.d2rc")
        );
        assert_eq!(acc.character_file(a, "kAsHyA").unwrap(), seat.file);
        assert_eq!(
            acc.character_file(a, "Nobody").unwrap_err().code,
            ErrorCode::UNKNOWN_CHAR
        );
        assert_eq!(
            acc.character_file(a, "..\\Kashya").unwrap_err().code,
            ErrorCode::UNKNOWN_CHAR,
            "an unchecked name is never a path"
        );

        // The game saves it, then reports the summary.
        let rec = record("Kashya", 6, 30, status::EXPANSION | (5 << 8));
        acc.character_store(a)
            .unwrap()
            .create("Kashya", &rec)
            .unwrap();
        let mut s = summary::of_record(&rec, 1_800_000_000).unwrap();
        acc.saved(a, s.clone()).await;
        let seat = acc.seat(a, "Kashya").await.unwrap();
        assert_eq!(seat.new, None);
        assert_eq!(seat.summary, s);
        assert_eq!(seat.summary.difficulties, 0b011);
        // Another account's save of the name changes nothing.
        let b = acc.register("Friend", PW, ip(1)).await.unwrap().id;
        s.level = 99;
        acc.saved(b, s).await;
        assert_eq!(acc.character(a, "Kashya").await.unwrap().level, 30);
    });
}

#[test]
fn reindex_rebuilds_rows_from_files() {
    let dir = temp("reindex");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    let a = run(async {
        let a = acc.register("Mogger", PW, ip(1)).await.unwrap().id;
        acc.create_character(a, new_char("Kashya", 0))
            .await
            .unwrap();
        acc.create_character(a, new_char("Unplayed", 1))
            .await
            .unwrap();
        a
    });
    let store = acc.character_store(a).unwrap();
    store
        .create(
            "Kashya",
            &record("Kashya", 0, 44, status::EXPANSION | (10 << 8)),
        )
        .unwrap();
    // A file the database never heard of comes back as a row.
    store
        .create("Lost", &record("Lost", 4, 12, status::EXPANSION))
        .unwrap();
    // A folder of no account is reported.
    CharacterStore::open(dir.join("data/characters/999"))
        .unwrap()
        .create("Ghost", &record("Ghost", 0, 1, 0))
        .unwrap();
    let r = acc.reindex().unwrap();
    assert_eq!(r.rows, 2, "{r:?}");
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert!(r.skipped[0].contains("no account 999"), "{r:?}");
    run(async {
        let k = acc.character(a, "Kashya").await.unwrap();
        assert_eq!((k.level, k.difficulties), (44, 0b111));
        assert!(k.last_played > 0);
        assert_eq!(acc.character(a, "lost").await.unwrap().class, 4);
        assert_eq!(
            acc.character(a, "Unplayed").await.unwrap().level,
            1,
            "no file: the row stays"
        );
    });
}

#[test]
fn backup_holds_the_database_and_the_files() {
    let dir = temp("backup");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    let a = run(async {
        let a = acc.register("Mogger", PW, ip(1)).await.unwrap().id;
        acc.create_character(a, new_char("Kashya", 0))
            .await
            .unwrap();
        a
    });
    let rec = record("Kashya", 0, 9, status::EXPANSION);
    acc.character_store(a)
        .unwrap()
        .create("Kashya", &rec)
        .unwrap();
    // An unfinished save is not copied.
    std::fs::write(
        acc.character_file(a, "Kashya")
            .unwrap()
            .with_extension("d2rc.tmp"),
        b"half",
    )
    .unwrap();

    let dest = dir.join("backups").join("2026-10-19");
    std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
    assert_eq!(acc.backup_into(&dest).unwrap(), 1);
    assert!(dest.join(crate::backup::COMPLETE).exists());
    let copied = std::fs::read(
        dest.join("characters")
            .join(a.0.to_string())
            .join("Kashya.d2rc"),
    )
    .unwrap();
    assert_eq!(characters::decode("Kashya", &copied).unwrap(), rec);
    assert!(!dest
        .join("characters")
        .join(a.0.to_string())
        .join("Kashya.d2rc.tmp")
        .exists());
    // The copied database opens and holds the account.
    let restored = Accounts::open_with(AccountsConfig {
        data: dest.clone(),
        ..config(&dir)
    })
    .unwrap();
    assert_eq!(run(restored.login("Mogger", PW, ip(1))).unwrap().id, a);
    // A backup is never written over, nor into the data folder.
    assert!(acc.backup_into(&dest).is_err());
    assert!(acc.backup_into(&dir.join("data").join("b")).is_err());
}

#[test]
fn logs_never_hold_passwords() {
    let dir = temp("logs");
    let log = Arc::new(ServerLog::open(&dir.join("logs")).unwrap());
    let acc = Accounts::open_with(config(&dir))
        .unwrap()
        .with_log(log.clone());
    let secret = "hunter2-secret-pw";
    run(async {
        let a = acc.register("Mogger", secret, ip(1)).await.unwrap().id;
        acc.login("Mogger", secret, ip(1)).await.unwrap();
        acc.login("Mogger", "another-secret", ip(1))
            .await
            .unwrap_err();
        acc.create_character(a, new_char("Kashya", 0))
            .await
            .unwrap();
        acc.delete_character(a, "Kashya", secret).await.unwrap();
    });
    acc.set_password("Mogger", "admin-secret-pw").unwrap();
    let text = std::fs::read_to_string(log.path()).unwrap();
    assert!(text.contains("registered `Mogger`"), "{text}");
    for s in [secret, "another-secret", "admin-secret-pw", "$argon2"] {
        assert!(!text.contains(s), "the log holds {s}: {text}");
    }
}

#[test]
fn a_login_already_verifying_obeys_a_lock_set_meanwhile() {
    let dir = temp("lock-race");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        acc.register("Mogger", PW, ip(1)).await.unwrap();
        for i in 0..9 {
            let e = acc
                .login("Mogger", "wrong password", ip(10 + i))
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::WRONG_LOGIN);
        }
        // Hold both hash permits, so both logins pass the lock check and
        // queue for a permit. The wrong one queues first and is the 10th
        // failure; the right one gets its permit only after that, so its
        // verify finishes after the lock.
        let first = acc.inner.hashing.acquire().await.unwrap();
        let second = acc.inner.hashing.acquire().await.unwrap();
        let wrong = tokio::spawn({
            let acc = acc.clone();
            async move { acc.login("Mogger", "wrong password", ip(60)).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let good = tokio::spawn({
            let acc = acc.clone();
            async move { acc.login("Mogger", PW, ip(61)).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        // One permit at a time: verifying both at once would let the right
        // one finish first under load.
        drop(first);
        let e = wrong.await.unwrap().unwrap_err();
        assert_eq!(e.code, ErrorCode::WRONG_LOGIN);
        drop(second);
        let e = good.await.unwrap().unwrap_err();
        assert_eq!(
            e.code,
            ErrorCode::ACCOUNT_LOCKED,
            "a queued guess can't log in after the name locked"
        );
    });
}

#[test]
fn a_delete_already_verifying_obeys_a_lock_set_meanwhile() {
    let dir = temp("delete-lock-race");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        let a = acc.register("Mogger", PW, ip(1)).await.unwrap().id;
        acc.create_character(a, new_char("Kashya", 0))
            .await
            .unwrap();
        for _ in 0..9 {
            let e = acc
                .delete_character(a, "Kashya", "wrong password")
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::WRONG_LOGIN);
        }
        let first = acc.inner.hashing.acquire().await.unwrap();
        let second = acc.inner.hashing.acquire().await.unwrap();
        let wrong = tokio::spawn({
            let acc = acc.clone();
            async move { acc.delete_character(a, "Kashya", "wrong password").await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        let good = tokio::spawn({
            let acc = acc.clone();
            async move { acc.delete_character(a, "Kashya", PW).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        // One permit at a time: verifying both at once would let the right
        // one finish first under load.
        drop(first);
        let e = wrong.await.unwrap().unwrap_err();
        assert_eq!(e.code, ErrorCode::WRONG_LOGIN);
        drop(second);
        let e = good.await.unwrap().unwrap_err();
        assert_eq!(e.code, ErrorCode::ACCOUNT_LOCKED);
        assert!(acc.character(a, "Kashya").await.is_ok(), "nothing deleted");
    });
}

#[test]
fn concurrent_registers_cannot_exceed_the_address_limit() {
    let dir = temp("register-race");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        let permits = acc.inner.hashing.acquire_many(2).await.unwrap();
        let mut tasks = Vec::new();
        for i in 0..4 {
            let acc = acc.clone();
            tasks.push(tokio::spawn(async move {
                acc.register(&format!("Acc{i}"), PW, ip(1)).await
            }));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(permits);
        let mut made = 0;
        let mut limited = 0;
        for t in tasks {
            match t.await.unwrap() {
                Ok(_) => made += 1,
                Err(e) if e.code == ErrorCode::RATE_LIMITED => limited += 1,
                Err(e) => panic!("unexpected {e:?}"),
            }
        }
        assert_eq!((made, limited), (3, 1), "the 3 per hour hold together");
    });
}

#[test]
fn the_limits_test_mode_shortens_the_windows_and_the_delay() {
    let dir = temp("limits-test");
    let mut cfg = config(&dir);
    cfg.limits = LimitMode::Test;
    // A delay that is not the test one, to prove the mode overrides it.
    cfg.fail_delay = Duration::from_millis(400);
    let acc = Accounts::open_with(cfg).unwrap();
    run(async {
        let a = acc.register("Mogger", PW, ip(2)).await.unwrap();
        assert_eq!(a.name, "Mogger");
        for i in 0..3 {
            acc.register(&format!("Acc{i}"), PW, ip(1)).await.unwrap();
        }
        let e = acc.register("Acc3", PW, ip(1)).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::RATE_LIMITED);
        assert!((1..=60).contains(&e.wait_s), "{}", e.wait_s);
        let t = std::time::Instant::now();
        let e = acc
            .login("Mogger", "wrong password", ip(2))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::WRONG_LOGIN);
        assert!(
            t.elapsed() >= Duration::from_millis(5),
            "the 10 ms test delay ran: {:?}",
            t.elapsed()
        );
        assert!(
            t.elapsed() < Duration::from_millis(200),
            "not the normal delay: {:?}",
            t.elapsed()
        );
    });
}

#[test]
fn the_test_modes_warn_in_the_log() {
    let dir = temp("warnings");
    let log = Arc::new(ServerLog::open(&dir.join("logs")).unwrap());
    let mut cfg = config(&dir);
    cfg.limits = LimitMode::Test;
    let acc = Accounts::open_with(cfg).unwrap().with_log(log.clone());
    let text = std::fs::read_to_string(log.path()).unwrap();
    assert!(text.contains("argon2 = test"), "{text}");
    assert!(text.contains("limits = test"), "{text}");
    drop(acc);
}

#[test]
fn set_progression_writes_the_row_and_the_file() {
    let dir = temp("progression");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        let a = acc.register("Mogger", PW, ip(1)).await.unwrap().id;
        acc.create_character(a, new_char("Kashya", 0))
            .await
            .unwrap();
        acc.create_character(a, new_char("Unplayed", 1))
            .await
            .unwrap();
        let rec = record("Kashya", 0, 42, status::EXPANSION);
        acc.character_store(a)
            .unwrap()
            .create("Kashya", &rec)
            .unwrap();

        acc.set_progression("kashya", 5).unwrap();
        let k = acc.character(a, "Kashya").await.unwrap();
        assert_eq!(k.difficulties, 0b011);
        assert_eq!(status::progression(k.status), 5);
        let file = acc.character_file(a, "Kashya").unwrap();
        let stored = characters::decode("Kashya", &std::fs::read(file).unwrap()).unwrap();
        assert_eq!(d2_formats::d2s::header_status(&stored), Some(k.status));

        acc.set_progression("Unplayed", 10).unwrap();
        let u = acc.character(a, "Unplayed").await.unwrap();
        assert_eq!(u.difficulties, 0b111);

        assert_eq!(
            acc.set_progression("Nobody", 1).unwrap_err(),
            "no character named `Nobody`"
        );
        assert!(acc.set_progression("Kashya", 32).is_err());
        acc.delete_character(a, "Kashya", PW).await.unwrap();
        assert!(acc.set_progression("Kashya", 1).is_err());
    });
}

#[test]
fn a_progression_set_before_the_first_game_seeds_the_record() {
    let dir = temp("progression-seed");
    let acc = Accounts::open_with(config(&dir)).unwrap();
    run(async {
        let a = acc.register("Mogger", PW, ip(1)).await.unwrap().id;
        acc.create_character(a, new_char("Kashya", 0))
            .await
            .unwrap();
        acc.set_progression("Kashya", 5).unwrap();
        // The seat carries it into the game, which writes it into the new
        // record; the server's seed still covers a record written without it.
        let seat = acc.seat(a, "Kashya").await.unwrap();
        assert_eq!(seat.new.as_ref().map(|n| n.progression), Some(5));
        assert_eq!(status::progression(seat.summary.status), 5);
        // The game makes the record on the first join and saves it without
        // progression; the server seeds the set one into it.
        let rec = record("Kashya", 0, 1, status::EXPANSION);
        acc.character_store(a)
            .unwrap()
            .create("Kashya", &rec)
            .unwrap();
        let s = summary::of_record(&rec, 1_800_000_000).unwrap();
        acc.saved(a, s).await;
        let k = acc.character(a, "Kashya").await.unwrap();
        assert_eq!(k.difficulties, 0b011);
        assert_eq!(status::progression(k.status), 5);
        let file = acc.character_file(a, "Kashya").unwrap();
        let stored = characters::decode("Kashya", &std::fs::read(file).unwrap()).unwrap();
        assert_eq!(d2_formats::d2s::header_status(&stored), Some(k.status));
    });
}

#[test]
fn config_keys() {
    let base = Path::new("/srv/d2");
    let mut c = AccountsConfig::new(base);
    assert_eq!(c.data, base.join("data"));
    assert_eq!(
        (c.cost, c.limits, c.max_characters, c.registration_open),
        (HashCost::Normal, LimitMode::Normal, 16, true)
    );
    c.set("data", "store", base).unwrap();
    c.set("argon2", "test", base).unwrap();
    c.set("limits", "test", base).unwrap();
    c.set("max_characters", "32", base).unwrap();
    c.set("registration", "closed", base).unwrap();
    assert_eq!(c.data, base.join("store"));
    assert_eq!(
        (c.cost, c.limits, c.max_characters, c.registration_open),
        (HashCost::Test, LimitMode::Test, 32, false)
    );
    assert!(c.set("max_characters", "33", base).is_err());
    assert!(c.set("max_characters", "0", base).is_err());
    assert!(c.set("argon2", "fast", base).is_err());
    assert!(c.set("limits", "fast", base).is_err());
    assert!(c.set("registration", "maybe", base).is_err());
}
