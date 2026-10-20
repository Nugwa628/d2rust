//! Coverage tests for the data loader's ordering rules: each table links only to
//! tables loaded in earlier steps, the fix-up pass runs in load order, combined
//! index spaces (items, treasure classes) are built in a fixed order, and the
//! hireling check is the only install-dependent processing. Synthetic data only.

use std::collections::{BTreeMap, BTreeSet};

use crate::schema::{schema, Link};

/// `S.NN` step as a sortable pair.
fn step(s: &str) -> (u32, u32) {
    match s.split_once('.') {
        Some((a, b)) => (a.parse().unwrap(), b.parse().unwrap()),
        None => (s.parse().unwrap(), 0),
    }
}

/// Table-name links of each called table (`<table>.<column>` linkers,
/// the `_lookup` compile-only entries named by their table).
fn deps() -> BTreeMap<String, BTreeSet<String>> {
    let mut out = BTreeMap::new();
    for t in schema().called() {
        let mut d = BTreeSet::new();
        for f in &t.fields {
            if let Link::Linker(l) = &f.link {
                if l.starts_with('@') {
                    continue;
                }
                let n = l.split('.').next().unwrap();
                let own_group =
                    n == "items" && matches!(t.name.as_str(), "weapons" | "armor" | "misc");
                if n != t.name && !own_group {
                    d.insert(n.to_owned());
                }
            }
        }
        out.insert(t.name.clone(), d);
    }
    out
}

#[test]
fn dependency_table_points_to_earlier_steps() {
    let steps: BTreeMap<String, (u32, u32)> = schema()
        .called()
        .map(|t| (t.name.clone(), step(t.load_step.as_deref().unwrap())))
        .collect();
    // The combined item array is complete after `misc` (step 17).
    let first_step_of = |n: &str| -> (u32, u32) {
        if n == "items" {
            steps["misc"]
        } else {
            steps
                .get(n)
                .copied()
                .unwrap_or_else(|| panic!("no table {n}"))
        }
    };
    let deps = deps();
    for (table, ds) in &deps {
        for d in ds {
            let dep_step = first_step_of(d);
            assert!(
                dep_step < steps[table],
                "{table} {:?} links to {d} {dep_step:?}",
                steps[table]
            );
        }
    }
    // Rows of the dependency table (compile-only links included).
    let want: &[(&str, &[&str])] = &[
        ("properties", &["itemstatcost"]),
        ("charstats", &["bodylocs", "skills"]),
        ("books", &["skills"]),
        ("qualityitems", &["properties"]),
        ("raresuffix", &["itemtypes"]),
        ("rareprefix", &["itemtypes"]),
        ("cubemain", &["playerclass", "properties"]),
        ("npc", &["monstats"]),
        ("levels", &["monstats"]),
        ("monprop", &["properties"]),
        ("hireling", &["hiredesc", "skills"]),
        ("monumod", &["montype"]),
        ("gems", &["items", "properties"]),
        ("runes", &["items", "itemtypes", "properties"]),
        ("uniqueitems", &["colors", "properties", "sounds"]),
        ("itemstatcost", &["events"]),
        ("monseq", &["monmode_lookup"]),
        ("monequip", &["bodylocs", "items", "monstats"]),
        ("itemtypes", &["bodylocs", "playerclass", "storepage"]),
    ];
    for (t, w) in want {
        let got: Vec<&str> = deps[*t].iter().map(String::as_str).collect();
        assert_eq!(&got, w, "{t}");
    }
    // Tables with no links.
    for t in [
        "arena",
        "gamble",
        "inventory",
        "belts",
        "objects",
        "plrtype",
    ] {
        assert!(deps[t].is_empty(), "{t}");
    }
}

#[test]
fn fixup_steps_match_load_order() {
    // No fix-up row is still pending.
    assert!(crate::fixup::PENDING.is_empty());
    // The load step of each fix-up table (first step of a range).
    let rows: &[(&str, &str)] = &[
        ("itemtypes", "2"),
        ("montype", "3"),
        ("itemstatcost", "6"),
        ("missiles", "8"),
        ("states", "9"),
        ("skills", "10"),
        ("charstats", "12"),
        ("weapons", "15"),
        ("armor", "16"),
        ("misc", "17"),
        ("magicsuffix", "18"),
        ("magicprefix", "19"),
        ("automagic", "20"),
        ("raresuffix", "21"),
        ("rareprefix", "22"),
        ("uniqueitems", "23"),
        ("sets", "24"),
        ("setitems", "25"),
        ("gems", "26"),
        ("qualityitems", "28"),
        ("lowqualityitems", "29"),
        ("runes", "30"),
        ("gamble", "32"),
        ("monseq", "50"),
        ("monstats", "51"),
        ("monumod", "52"),
        ("superuniques", "53"),
        ("monpreset", "54"),
        ("hireling", "55"),
        ("monequip", "57"),
        ("levels", "58"),
        ("leveldefs", "59"),
        ("lvltypes", "60"),
        ("lvlprest", "61"),
        ("lvlsub", "64"),
        ("automap", "65"),
        ("objects", "66"),
    ];
    for (table, s) in rows {
        let def = schema().table(table).unwrap_or_else(|| panic!("{table}"));
        assert_eq!(def.load_step.as_deref(), Some(*s), "{table}");
    }
    // The pass runs in load order: the steps strictly increase.
    let order: Vec<(u32, u32)> = rows.iter().map(|(_, s)| step(s)).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn combined_index_spaces() {
    use crate::bin::item_code_map;
    // Items: weapons, armor, misc in that order (record `j` of the
    // combined array is the `j`-th code added).
    let mk = |name: &str, codes: &[&[u8; 4]]| {
        let records = codes
            .iter()
            .map(|c| {
                let mut r = vec![0u8; 0x84];
                r[0x80..0x84].copy_from_slice(&c[..]);
                r
            })
            .collect();
        crate::bin::BinTable {
            name: name.into(),
            source: "test".into(),
            count: codes.len(),
            record_size: 0x84,
            records: records_flat(records),
        }
    };
    // Tables given out of order are still combined weapons, armor, misc.
    let tables = [
        mk("misc", &[b"mis1"]),
        mk("armor", &[b"arm1", b"arm2"]),
        mk("weapons", &[b"wep1", b"wep2", b"wep3"]),
    ];
    let map = item_code_map(&tables);
    assert_eq!(map.len(), 6);
    for (i, c) in [b"wep1", b"wep2", b"wep3", b"arm1", b"arm2", b"mis1"]
        .iter()
        .enumerate()
    {
        assert_eq!(
            map.find(u32::from_le_bytes(**c)),
            Some(i as u32),
            "{}",
            String::from_utf8_lossy(&c[..])
        );
    }
    // Suffixes come before prefixes in both affix arrays; the other
    // arrays are in the listed part order.
    for parts in [
        &["magicsuffix", "magicprefix", "automagic"][..],
        &["raresuffix", "rareprefix"],
        &["plrtype", "plrmode"],
        &["objtype", "objmode"],
        &["weapons", "armor", "misc"],
    ] {
        let steps: Vec<(u32, u32)> = parts
            .iter()
            .map(|p| step(schema().table(p).unwrap().load_step.as_deref().unwrap()))
            .collect();
        assert!(steps.windows(2).all(|w| w[0] < w[1]), "{parts:?}");
    }
}

fn records_flat(records: Vec<Vec<u8>>) -> Vec<u8> {
    records.into_iter().flatten().collect()
}

#[test]
fn install_dependent_processing_is_the_hireling_check_only() {
    use crate::bin::{post_load_check, BinTable};
    use crate::strings::StringTables;
    let size = schema().table("hireling").unwrap().record_size;
    let mut r = vec![0u8; size];
    r[0x04..0x08].copy_from_slice(&300u32.to_le_bytes());
    let t = BinTable {
        name: "hireling".into(),
        source: "test".into(),
        count: 1,
        record_size: size,
        records: r,
    };
    let s = StringTables::default();
    // d2exp present (the D2RUST path, policy 5): the check runs and fails.
    assert!(post_load_check(&t, &[], &s, true).is_err());
    // d2exp absent: no hireling check.
    assert!(post_load_check(&t, &[], &s, false).is_ok());
    // Every table is a loader call whatever the install: the schema has no
    // classic-only or expansion-only flag, and none is skipped.
    assert!(schema().called().count() >= 73);
}
