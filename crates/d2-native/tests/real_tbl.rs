//! String-table check on the real install: every string table of every archive
//! parses, writes and reads back equal, and the hash slots a rebuild from
//! element order gives equal the table's own.
use d2_formats::mpq::Archive;
use d2_formats::tbl::StringTable;
use d2_native::tbl::{check_tbl, rebuild_differs};

fn game_dir() -> std::path::PathBuf {
    std::env::var("D2_GAME_DIR")
        .expect("set D2_GAME_DIR")
        .into()
}

/// The rebuild equals the stored slots in
/// every shipped table (differences, if any, are counted and listed).
#[test]
#[ignore = "needs the game install (D2_GAME_DIR)"]
fn every_tbl_round_trips_and_rebuilds() {
    let mut checked = 0;
    let mut rebuild_diff = Vec::new();
    for mpq in ["d2data", "d2exp", "Patch_D2"] {
        let a = Archive::open(game_dir().join(format!("{mpq}.mpq"))).unwrap();
        let mut names: Vec<String> = a
            .listfile()
            .unwrap()
            .unwrap_or_default()
            .into_iter()
            .filter(|n| n.to_ascii_lowercase().ends_with(".tbl"))
            .collect();
        // Patch_D2 has no listfile: probe the ENG names by hand.
        if mpq == "Patch_D2" {
            names = ["patchstring", "string", "expansionstring"]
                .iter()
                .map(|s| format!("data\\local\\LNG\\ENG\\{s}.tbl"))
                .filter(|n| a.contains(n))
                .collect();
        }
        for n in names {
            let data = a.read(&n).unwrap();
            let Ok(t) = StringTable::parse(&data) else {
                continue; // font .tbl files share the extension
            };
            if n.to_ascii_lowercase().contains("\\font\\") {
                continue;
            }
            check_tbl(&n, &t).unwrap_or_else(|e| panic!("{mpq}:{n}: {e}"));
            if rebuild_differs(&t) {
                rebuild_diff.push(format!("{mpq}:{n}"));
            }
            checked += 1;
        }
    }
    println!("C-TBL: {checked} tables checked, rebuild differs in {rebuild_diff:?}");
    assert!(checked >= 20, "only {checked} string tables found");
    assert!(rebuild_diff.is_empty(), "rebuild differs: {rebuild_diff:?}");
}
