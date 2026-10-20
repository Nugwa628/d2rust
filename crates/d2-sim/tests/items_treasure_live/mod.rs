//! The player's table set for `game_items.rs` and `game_treasure.rs`:
//! the `.bin` set of the install in `D2_GAME_DIR`, loaded and fixed up
//! through D2RUST (`d2_data::bin::load`, `d2_data::fixup::apply`,
//! as `d2-data`'s `fixups_on_live_set`). Tests only: the sim never reads
//! files.

#![allow(dead_code)]

use std::sync::OnceLock;

use d2_data::bin;
use d2_data::fixup::{self, FixedSet};
use d2_data::tables::{decode_all, Record};
use d2_formats::mpq::ArchiveSet;

/// The fixed-up live set, loaded once per test binary.
pub fn fixed() -> &'static FixedSet {
    static SET: OnceLock<FixedSet> = OnceLock::new();
    SET.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let data = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live .bin set loads");
        let anim = fixup::read_animdata(&set).expect("animdata reads");
        fixup::apply(&data, &anim).expect("fix-ups apply")
    })
}

/// One typed table of the fixed-up set.
pub fn typed<T: Record>() -> Vec<T> {
    let t = fixed()
        .table(T::TABLE)
        .unwrap_or_else(|| panic!("table {} in the live set", T::TABLE));
    decode_all(t).unwrap_or_else(|e| panic!("{}: {e}", T::TABLE))
}

/// A code cell as text (pad spaces kept).
pub fn code(c: [u8; 4]) -> String {
    String::from_utf8_lossy(&c).into_owned()
}
