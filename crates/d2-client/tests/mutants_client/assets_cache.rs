//! Mutation-testing gaps of `assets::cache`.

use std::time::Duration;

use d2_client::assets::cache::{Clock, WallClock};

/// A synchronous load is logged with its duration, measured by the
/// wall clock: time that passed shows in its reading.
#[test]
fn wall_clock_measures_elapsed_time() {
    let mut clock = WallClock::default();
    std::thread::sleep(Duration::from_millis(3));
    assert!(clock.now_micros() >= 3_000);
}

/// Pools: name, budget, current frame, residency and size are what
/// was set and inserted.
#[test]
fn pool_accessors() {
    use d2_client::assets::cache::Pool;

    let mut pool: Pool<u32, ()> = Pool::new("frames", 100);
    assert_eq!(pool.name(), "frames");
    assert_eq!(pool.budget(), 100);
    assert!(pool.is_empty());
    assert_eq!(pool.len(), 0);
    assert!(!pool.contains(&1));
    pool.begin_frame(7).unwrap();
    assert_eq!(pool.frame(), 7);
    pool.insert(1, (), 10).unwrap();
    pool.insert(2, (), 10).unwrap();
    assert!(!pool.is_empty());
    assert_eq!(pool.len(), 2);
    assert!(pool.contains(&1));
    assert!(!pool.contains(&3));
}

/// Canonical form: `\` → `/` and ASCII lower case, so two spellings
/// of one file give one path.
#[test]
fn fold_gives_the_canonical_spelling() {
    use d2_client::assets::path::fold;
    use d2_client::assets::CanonicalPath;

    assert_eq!(fold("DATA\\Global\\X.DC6"), "data/global/x.dc6");
    let a = CanonicalPath::new("DATA\\Global\\X.DC6").unwrap();
    assert_eq!(a.as_str(), "data/global/x.dc6");
    assert_eq!(a, CanonicalPath::new("data\\global\\x.dc6").unwrap());
}

/// Paths through the user's archive set: a file the set holds reads as its
/// bytes (`pal.dat` is 768 bytes); a missing one
/// is `None`, never a fallback.
/// `D2_GAME_DIR=... cargo test -p d2-client --test mutants_assets_cache -- --ignored`
#[test]
#[ignore = "needs D2_GAME_DIR"]
fn archive_set_reads_files_it_holds() {
    use d2_client::assets::FileSource;
    use d2_formats::mpq::ArchiveSet;

    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR");
    let set = ArchiveSet::open_dir(&dir).unwrap();
    let pal = set.read_file("data\\global\\palette\\act1\\pal.dat");
    assert_eq!(pal.map(|r| r.map(|b| b.len())), Some(Ok(768)));
    assert!(set.read_file("data\\global\\no\\such\\file.dat").is_none());
}
