//! The character folder: name rules, the versioned envelope, and a store
//! that keeps characters across a restart.

mod support;

use d2rust_server::characters::{
    check_name, decode, encode, CharacterError, CharacterStore, EXTENSION, FORMAT_VERSION,
};

#[test]
fn names_follow_the_game_rules() {
    for ok in ["Ab", "Amazon", "Sor-ceress", "Big_Bad", "Abcdefghijklmno"] {
        assert_eq!(check_name(ok), Ok(()), "{ok}");
    }
    for bad in [
        "A",
        "Abcdefghijklmnop",
        "-Lead",
        "Trail_",
        "Two-Of_Them",
        "Digit1",
        "Space Name",
        "Ünicode",
        "../up",
    ] {
        assert!(
            matches!(check_name(bad), Err(CharacterError::BadName { .. })),
            "{bad}"
        );
    }
}

#[test]
fn the_envelope_round_trips_and_carries_its_version() {
    let record = b"a character record".to_vec();
    let file = encode(&record);
    assert_eq!(&file[..4], b"D2RC");
    assert_eq!(u16::from_le_bytes([file[4], file[5]]), FORMAT_VERSION);
    assert_eq!(decode("Hero", &file), Ok(record));
    assert_eq!(decode("Hero", &encode(&[])), Ok(Vec::new()));
}

#[test]
fn a_damaged_file_is_refused() {
    let file = encode(b"record");
    let damaged = |f: &[u8]| matches!(decode("Hero", f), Err(CharacterError::Damaged { .. }));
    assert!(damaged(b""));
    assert!(damaged(&file[..file.len() - 1]));
    let mut flipped = file.clone();
    flipped[13] ^= 1;
    assert!(damaged(&flipped));
    let mut magic = file.clone();
    magic[0] = b'X';
    assert!(damaged(&magic));
}

/// A file from a newer server: a valid envelope with a higher version.
#[test]
fn a_newer_format_is_refused() {
    let mut file = encode(b"record");
    let n = file.len();
    file[4..6].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
    file.truncate(n - 4);
    let sum = file.iter().fold(0x811C_9DC5u32, |h, &b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    file.extend_from_slice(&sum.to_le_bytes());
    assert_eq!(
        decode("Hero", &file),
        Err(CharacterError::TooNew {
            name: "Hero".into(),
            version: FORMAT_VERSION + 1
        })
    );
}

#[test]
fn the_store_creates_loads_and_saves() {
    let dir = support::temp_dir("store");
    let store = CharacterStore::open(dir.join("characters")).unwrap();
    assert!(store.list().unwrap().is_empty());
    store.create("Zed", b"zed v1").unwrap();
    store.create("amy", b"amy v1").unwrap();
    assert_eq!(store.list().unwrap(), ["amy", "Zed"]);
    assert_eq!(store.load("zed").unwrap(), b"zed v1");
    store.save("ZED", b"zed v2").unwrap();
    assert_eq!(store.load("Zed").unwrap(), b"zed v2");
    // The save went through a temporary file that is gone now.
    let files: Vec<_> = std::fs::read_dir(store.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files.iter().all(|f| f.ends_with(EXTENSION)), "{files:?}");
}

#[test]
fn names_are_unique_in_any_case() {
    let dir = support::temp_dir("unique");
    let store = CharacterStore::open(&dir).unwrap();
    store.create("Hero", b"one").unwrap();
    assert_eq!(
        store.create("HERO", b"two"),
        Err(CharacterError::Exists("Hero".into()))
    );
    assert_eq!(
        store.save("Nobody", b"x"),
        Err(CharacterError::NotFound("Nobody".into()))
    );
    assert_eq!(
        store.load("Nobody"),
        Err(CharacterError::NotFound("Nobody".into()))
    );
    assert!(matches!(
        store.create("../evil", b"x"),
        Err(CharacterError::BadName { .. })
    ));
}

#[test]
fn characters_survive_a_restart() {
    let dir = support::temp_dir("restart");
    {
        let store = CharacterStore::open(&dir).unwrap();
        store.create("Keeper", b"kept").unwrap();
    }
    let again = CharacterStore::open(&dir).unwrap();
    assert_eq!(again.list().unwrap(), ["Keeper"]);
    assert_eq!(again.load("Keeper").unwrap(), b"kept");
}

#[test]
fn other_files_in_the_folder_are_ignored() {
    let dir = support::temp_dir("others");
    std::fs::write(dir.join("notes.txt"), b"x").unwrap();
    std::fs::write(dir.join("Bad Name.d2rc"), b"x").unwrap();
    let store = CharacterStore::open(&dir).unwrap();
    assert!(store.list().unwrap().is_empty());
}
