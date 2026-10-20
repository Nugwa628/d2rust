//! Compares D2RUST treasure classes against a local memory dump
//! (`tools/trace-recorder/dump_tables.py`: `map-tc_records.bin`,
//! `map-tc_entries.bin`, `map-tc_chest.bin`, `manifest.json` `tc_base`).
//! Every TC record and entry that D2RUST builds from the live `.bin` set is
//! serialised to the runtime layout and compared byte for byte with the
//! dump. The dump is local and not committed:
//!
//! `D2_GAME_DIR=<install> D2_TABLES_DUMP=traces/raw/<time>-tables cargo test
//!  -p d2-sim --test game_treasure_dump -- --ignored`

mod items_treasure_live;

use std::path::PathBuf;

use d2_data::tables::{Armor, Itemtypes, Misc, Setitems, Treasureclassex, Uniqueitems, Weapons};
use d2_sim::treasure::{item_list, TcEntry, TcSources, TreasureClass, TreasureClasses};
use items_treasure_live::{fixed, typed};

const REC: usize = 0x2C;
const ENT: usize = 0x1C;
/// Record bytes that hold the entries pointer (a process address).
const REC_PTR: std::ops::Range<usize> = 0x28..0x2C;

#[derive(Clone)]
struct Dump {
    records: Vec<u8>,
    entries: Vec<u8>,
    chest: Vec<u8>,
    base: u32,
}

#[allow(clippy::disallowed_methods)]
fn load_dump() -> Option<Dump> {
    let Some(dir) = std::env::var_os("D2_TABLES_DUMP") else {
        eprintln!("SKIPPED: D2_TABLES_DUMP (a traces/raw/<time>-tables directory) is not set");
        return None;
    };
    let dir = PathBuf::from(dir);
    let read = |n: &str| {
        std::fs::read(dir.join(n)).unwrap_or_else(|e| panic!("{}: {e}", dir.join(n).display()))
    };
    let manifest = String::from_utf8(read("manifest.json")).expect("manifest utf8");
    // `"tc_base": "0x47c1fec"` (our own tool's output).
    let at = manifest.find("\"tc_base\"").expect("manifest has tc_base");
    let rest = &manifest[at..];
    let q = rest.find("\"0x").expect("tc_base value");
    let hex: String = rest[q + 3..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    Some(Dump {
        records: read("map-tc_records.bin"),
        entries: read("map-tc_entries.bin"),
        chest: read("map-tc_chest.bin"),
        base: u32::from_str_radix(&hex, 16).expect("tc_base hex"),
    })
}

fn live_tcs() -> TreasureClasses {
    let items = item_list(&typed::<Weapons>(), &typed::<Armor>(), &typed::<Misc>());
    let itemtypes = typed::<Itemtypes>();
    let equiv = fixed().itemtypes_equiv.clone();
    TreasureClasses::build(&TcSources {
        treasureclassex: &typed::<Treasureclassex>(),
        itemtypes: &itemtypes,
        items: &items,
        equiv: &equiv,
        uniqueitems: &typed::<Uniqueitems>(),
        setitems: &typed::<Setitems>(),
    })
    .expect("the live TCs build")
}

/// The D2RUST record bytes (§1.1); the pointer field is left zero.
fn record_bytes(tc: &TreasureClass) -> [u8; REC] {
    let mut b = [0u8; REC];
    b[0..2].copy_from_slice(&tc.group.to_le_bytes());
    b[2..4].copy_from_slice(&tc.level.to_le_bytes());
    b[4..8].copy_from_slice(&(tc.entries.len() as i32).to_le_bytes());
    b[8..12].copy_from_slice(&tc.total_classic.to_le_bytes());
    b[12..16].copy_from_slice(&tc.total_expansion.to_le_bytes());
    b[16..20].copy_from_slice(&tc.picks.to_le_bytes());
    b[20..24].copy_from_slice(&tc.nodrop.to_le_bytes());
    for (i, m) in tc.mods.iter().enumerate() {
        b[0x1A + 2 * i..0x1C + 2 * i].copy_from_slice(&m.to_le_bytes());
    }
    b
}

fn entry_bytes(e: &TcEntry) -> [u8; ENT] {
    let mut b = [0u8; ENT];
    b[0..4].copy_from_slice(&e.start_classic.to_le_bytes());
    b[4..8].copy_from_slice(&e.start_expansion.to_le_bytes());
    b[8..10].copy_from_slice(&e.id.to_le_bytes());
    b[10..12].copy_from_slice(&e.row.to_le_bytes());
    b[12] = e.flags;
    for (i, m) in e.mods.iter().enumerate() {
        b[0x0E + 2 * i..0x10 + 2 * i].copy_from_slice(&m.to_le_bytes());
    }
    b
}

/// Every difference between D2RUST and the dump, in TC order; empty when
/// identical. The record pointer bytes are skipped (a process address).
fn compare(tcs: &TreasureClasses, d: &Dump) -> Vec<String> {
    let mut out = Vec::new();
    if d.records.len() != tcs.tcs.len() * REC {
        out.push(format!(
            "TC count: D2RUST {}, dump {}",
            tcs.tcs.len(),
            d.records.len() / REC
        ));
        return out;
    }
    let mut off = 0usize;
    for (i, tc) in tcs.tcs.iter().enumerate() {
        let mine = record_bytes(tc);
        let theirs = &d.records[i * REC..(i + 1) * REC];
        for k in (0..REC).filter(|k| !REC_PTR.contains(k)) {
            if mine[k] != theirs[k] {
                out.push(format!(
                    "TC {i} {:?} record byte {k:#x}: D2RUST {:#04x}, dump {:#04x}",
                    String::from_utf8_lossy(&tc.name),
                    mine[k],
                    theirs[k]
                ));
            }
        }
        let dump_count = i32::from_le_bytes(theirs[4..8].try_into().unwrap()).max(0) as usize;
        for (j, e) in tc.entries.iter().enumerate() {
            let mine = entry_bytes(e);
            let at = off + j * ENT;
            let Some(theirs) = d.entries.get(at..at + ENT) else {
                out.push(format!("TC {i} entry {j}: dump entries end early"));
                continue;
            };
            for k in 0..ENT {
                if mine[k] != theirs[k] {
                    out.push(format!(
                        "TC {i} {:?} entry {j} byte {k:#x}: D2RUST {:#04x}, dump {:#04x}",
                        String::from_utf8_lossy(&tc.name),
                        mine[k],
                        theirs[k]
                    ));
                }
            }
        }
        off += dump_count * ENT;
    }
    if off != d.entries.len() {
        out.push(format!(
            "entry bytes: dump records claim {off}, file {}",
            d.entries.len()
        ));
    }
    // §1.6: the chest pointers are record addresses (`tc_base` + index * 0x2C).
    for (i, c) in tcs.chest.iter().enumerate() {
        let p = u32::from_le_bytes(d.chest[i * 4..i * 4 + 4].try_into().unwrap());
        let dump = (p != 0).then(|| ((p - d.base) as usize / REC) as u16);
        if *c != dump {
            out.push(format!("chest {i}: D2RUST {c:?}, dump {dump:?}"));
        }
    }
    out
}

#[test]
#[ignore = "needs D2_GAME_DIR (game tables) and D2_TABLES_DUMP (local memory dump)"]
fn live_tcs_equal_memory_dump() {
    let Some(d) = load_dump() else { return };
    let tcs = live_tcs();
    assert_eq!(tcs.tcs.len(), 1013, "spec §Open questions 4: 1,013 TCs");
    let diffs = compare(&tcs, &d);
    assert!(
        diffs.is_empty(),
        "{} differences, first 20:\n{}",
        diffs.len(),
        diffs
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// M08: one flipped byte of the loaded dump is reported as exactly that
/// TC (a record byte, an entry byte, a chest pointer).
#[test]
#[ignore = "needs D2_GAME_DIR (game tables) and D2_TABLES_DUMP (local memory dump)"]
fn memory_dump_perturbation_is_reported() {
    let Some(d) = load_dump() else { return };
    let tcs = live_tcs();
    // Record byte: TC 430 picks.
    let mut p = d.clone();
    p.records[430 * REC + 0x10] ^= 1;
    let diffs = compare(&tcs, &p);
    assert_eq!(diffs.len(), 1, "{diffs:?}");
    assert!(
        diffs[0].starts_with("TC 430 ") && diffs[0].contains("record byte 0x10"),
        "{diffs:?}"
    );
    // Entry byte: the first entry of TC 430 (id low byte); its offset is
    // the sum of the counts before it.
    let before: usize = (0..430)
        .map(|i| {
            i32::from_le_bytes(d.records[i * REC + 4..i * REC + 8].try_into().unwrap()).max(0)
                as usize
        })
        .sum();
    let mut p = d.clone();
    p.entries[before * ENT + 8] ^= 1;
    let diffs = compare(&tcs, &p);
    assert_eq!(diffs.len(), 1, "{diffs:?}");
    assert!(
        diffs[0].starts_with("TC 430 ") && diffs[0].contains("entry 0 byte 0x8"),
        "{diffs:?}"
    );
    // Chest pointer: Act 1 Chest A one record up.
    let mut p = d.clone();
    let v = u32::from_le_bytes(p.chest[0..4].try_into().unwrap()) + REC as u32;
    p.chest[0..4].copy_from_slice(&v.to_le_bytes());
    let diffs = compare(&tcs, &p);
    assert_eq!(diffs.len(), 1, "{diffs:?}");
    assert!(diffs[0].starts_with("chest 0:"), "{diffs:?}");
}
