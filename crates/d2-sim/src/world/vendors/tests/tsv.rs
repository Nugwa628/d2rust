// Spec: specs/world/vendors.md §1, §4; vendors.tsv
//! `vendors.tsv` is an expectation only: the per-NPC constants of the code
//! (column switch, gamble lists, +0x24 flags, hire lists) are checked
//! against it.
use super::*;
use crate::world::{tsv_num, tsv_rows, TsvError};

const TSV: &str = include_str!("../../../../../../tables/world/vendors.tsv");
const HEADER: &[&str] = &[
    "npc",
    "name",
    "act",
    "trader",
    "force_vendor",
    "cache",
    "refresh_flag",
    "gamble_flag",
    "trade_action",
    "gamble_action",
    "heals",
    "identifies",
    "hire_list",
    "resurrects",
];

/// One line per disagreement between the TSV and the code.
fn check(text: &str) -> Result<Vec<String>, TsvError> {
    let mut out = Vec::new();
    let mut classes = Vec::new();
    for (line, c) in tsv_rows("vendors.tsv", text, HEADER)? {
        let num = |col: usize| tsv_num("vendors.tsv", line, HEADER[col], c[col]);
        let class = num(0)? as u16;
        classes.push(class);
        let flag = |col: usize| -> Result<bool, TsvError> { Ok(num(col)? != 0) };
        let cache = match c[5] {
            "-" => None,
            _ => Some(num(5)? as usize),
        };
        let mut bad = |what: &str, tsv: String, code: String| {
            if tsv != code {
                out.push(format!("{} ({class}) {what}: tsv {tsv}, code {code}", c[1]));
            }
        };
        bad(
            "cache",
            format!("{cache:?}"),
            format!("{:?}", column_of(class)),
        );
        bad(
            "trader",
            format!("{}", flag(3)?),
            format!("{}", column_of(class).is_some()),
        );
        bad(
            "gamble_flag",
            format!("{}", flag(7)?),
            format!("{}", GAMBLERS.contains(&class)),
        );
        bad(
            "refresh_flag",
            format!("{}", flag(6)?),
            format!("{}", FLAGGED.contains(&class)),
        );
        bad(
            "hire_list",
            format!("{}", flag(12)?),
            format!("{}", HIRE_CLASSES.contains(&class)),
        );
    }
    for c in GAMBLERS
        .iter()
        .chain(&FLAGGED)
        .chain(&HIRE_CLASSES)
        .chain(&REPAIRERS)
        .chain(&NO_STORE_REFRESH)
    {
        if !classes.contains(c) {
            out.push(format!("class {c} not in the table"));
        }
    }
    Ok(out)
}

// Covers: specs/world/vendors.md §1 r3, §1 r4, §1 r5, §4 r2
#[test]
fn tsv_matches_code() {
    let errors = check(TSV).unwrap();
    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn tsv_check_catches_perturbations() {
    let perturb = |from: &str, to: &str| {
        assert!(TSV.contains(from), "{from}");
        check(&TSV.replacen(from, to, 1)).unwrap()
    };
    // Nihlathak's cache column 16 (the global build) is not the copy.
    let e = perturb("514\tnihlathak\t4\t1\t0\t15", "514\tnihlathak\t4\t1\t0\t16");
    assert_eq!(e.len(), 1, "{e:?}");
    assert!(e[0].starts_with("nihlathak (514) cache"));
    // Charsi with a gamble list.
    let e = perturb(
        "154\tcharsi\t0\t1\t0\t2\t1\t0",
        "154\tcharsi\t0\t1\t0\t2\t1\t1",
    );
    assert_eq!(e, vec!["charsi (154) gamble_flag: tsv true, code false"]);
    // A hire-list flag removed.
    let e = perturb(
        "252\tasheara\t2\t1\t0\t10\t1\t0\t1\t0\t0\t0\t1",
        "252\tasheara\t2\t1\t0\t10\t1\t0\t1\t0\t0\t0\t0",
    );
    assert_eq!(e, vec!["asheara (252) hire_list: tsv false, code true"]);
    // A row removed.
    let e = perturb("511\tlarzuk", "999\tlarzuk");
    assert!(e.iter().any(|l| l == "class 511 not in the table"), "{e:?}");
    // A bad cell.
    let bad = TSV.replacen("\t15\t", "\tx\t", 1);
    assert!(matches!(check(&bad), Err(TsvError::Value { .. })));
}
