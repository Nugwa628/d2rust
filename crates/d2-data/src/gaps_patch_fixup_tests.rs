//! Gap tests for the patch layers, fix-ups, callbacks, runtime maps, txt format and
//! calc expressions: rules no other test covered.
//!
//! Synthetic data only (our own invented rows, keys and strings); helpers
//! private to other test modules are reimplemented here.

use d2_formats::animdata::{self, AnimData};
use d2_formats::tbl::{key_hash, StringTable, TblEntry, TblHeader};

use crate::bin::{cstr, u32_at, BinSet, BinTable};
use crate::compile::{
    code4, compile_table, name_key, special_linker, CodeLinker, Compiled, DiagKind, Linker,
    Linkers, NameLinker, SpecialItem, SpecialItems, StdCallbacks, SETS_LINKER, UNIQUES_LINKER,
};
use crate::fixup::maps::{self, EquivKind};
use crate::fixup::qsort::qsort;
use crate::fixup::records::{self, SPEED_CAP};
use crate::fixup::text::{copy_wide, fix_path, wide_text};
use crate::fixup::{self, MISSING_ITEM_NAME};
use crate::patch::{
    apply_stack, has_errors, parse_layer, parse_stack, Code, Finding, KeyKind, Layer, Origin,
    PatchData, PatchTable, Row, TableRules, Writer, VERSION,
};
use crate::schema::{schema, FieldDef, FieldType, Link};
use crate::strings::StringTables;
use crate::txt::{ErrorCode, TxtTable};

// ================================================================ helpers

fn v(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

/// A `.bin` table of `records`, each padded to `size`.
fn bin_table(name: &str, size: usize, records: Vec<Vec<u8>>) -> BinTable {
    BinTable {
        name: name.into(),
        source: "test".into(),
        count: records.len(),
        record_size: size,
        records: records
            .into_iter()
            .flat_map(|mut r| {
                r.resize(size, 0);
                r
            })
            .collect(),
    }
}

/// A record of `size` zero bytes with the given writes.
fn rec(size: usize, writes: &[(usize, &[u8])]) -> Vec<u8> {
    let mut r = vec![0; size];
    for (o, b) in writes {
        r[*o..*o + b.len()].copy_from_slice(b);
    }
    r
}

fn rec_mut(t: &mut BinTable, k: usize) -> &mut [u8] {
    let size = t.record_size;
    &mut t.records[k * size..(k + 1) * size]
}

fn get_u16(r: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([r[o], r[o + 1]])
}

fn i32_at(r: &[u8], o: usize) -> i32 {
    u32_at(r, o) as i32
}

/// UCS-2 text at `at`, up to its 0 or `units`.
fn wide(r: &[u8], at: usize, units: usize) -> String {
    (0..units)
        .map(|k| get_u16(r, at + 2 * k))
        .take_while(|&u| u != 0)
        .map(|u| char::from(u as u8))
        .collect()
}

/// A string table holding `(key, element, text)` entries.
fn string_table(keys: &[(&[u8], u16, &[u8])]) -> StringTable {
    const SLOTS: usize = 64;
    let max = keys.iter().map(|k| k.1).max().unwrap_or(0);
    let unused = TblEntry {
        used: 0,
        index: 0,
        hash: 0,
        key: Vec::new(),
        value: Vec::new(),
    };
    let mut entries = vec![unused; SLOTS];
    let mut indices = vec![0u16; usize::from(max) + 1];
    for (key, element, text) in keys {
        let mut slot = key_hash(key) as usize % SLOTS;
        while entries[slot].used != 0 {
            slot = (slot + 1) % SLOTS;
        }
        entries[slot] = TblEntry {
            used: 1,
            index: *element,
            hash: key_hash(key),
            key: key.to_vec(),
            value: text.to_vec(),
        };
        indices[usize::from(*element)] = slot as u16;
    }
    StringTable {
        header: TblHeader {
            crc: 0,
            num_elements: max + 1,
            hash_table_size: SLOTS as u32,
            version: 0,
            data_start: 0,
            max_tries: SLOTS as u32,
            file_size: 0,
        },
        indices,
        entries,
    }
}

/// `string.tbl` only, with the given entries (ids = element numbers).
fn strings(keys: &[(&[u8], u16, &[u8])]) -> StringTables {
    StringTables {
        base: Some(string_table(keys)),
        patch: None,
        expansion: None,
        layer: Default::default(),
    }
}

fn empty_txt() -> TxtTable {
    TxtTable {
        header: Vec::new(),
        records: Vec::new(),
        removed_lines: Vec::new(),
    }
}

/// A loaded set holding only `tables`.
fn bin_set(tables: Vec<BinTable>, strings: StringTables) -> BinSet {
    BinSet {
        lod: true,
        rules: Default::default(),
        strings,
        tables,
        code: Default::default(),
        code_reports: Default::default(),
        hitclass: bin_table("hitclass", 4, vec![]),
        sounds: empty_txt(),
        soundenviron: empty_txt(),
    }
}

fn no_anim() -> AnimData {
    AnimData {
        buckets: vec![Vec::new(); animdata::BUCKETS],
    }
}

fn record_size(table: &str) -> usize {
    schema().table(table).unwrap().record_size
}

fn permutations(v: &mut Vec<usize>, k: usize, out: &mut Vec<Vec<usize>>) {
    if k == v.len() {
        out.push(v.clone());
        return;
    }
    for i in k..v.len() {
        v.swap(k, i);
        permutations(v, k + 1, out);
        v.swap(k, i);
    }
}

// ======================================================= patch layers

fn patch_rules(
    name: &str,
    key: Option<&str>,
    kind: KeyKind,
    unique: bool,
    fixed: bool,
    fields: &[&str],
) -> TableRules {
    TableRules {
        name: name.into(),
        txt_name: format!("{name}.txt"),
        key_field: key.map(v),
        kind,
        unique,
        fixed,
        scope: if key.is_some() {
            "items.code".into()
        } else {
            name.into()
        },
        lists: vec![fields.iter().map(|f| v(f)).collect()],
    }
}

fn patch_table(r: TableRules, header: &[&str], rows: &[&str]) -> PatchTable {
    let rows = rows
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let cells: Vec<Vec<u8>> = line.split(';').map(v).collect();
            Row {
                origin: Origin::Base(i + 2),
                writers: vec![Writer::Base; cells.len()],
                cells,
            }
        })
        .collect();
    PatchTable::new(&r, "fixture", header.iter().map(|h| v(h)).collect(), rows).unwrap()
}

/// The patch fixture: `items`, `gear`, `recipes`.
fn patch_fixture() -> PatchData {
    let items = patch_table(
        patch_rules(
            "items",
            Some("code"),
            KeyKind::Code,
            true,
            false,
            &["name", "code", "lvl", "dam"],
        ),
        &["name", "code", "lvl", "dam", "dam", "*note"],
        &[
            "Axe;axe;1;3;0;",
            "Club;clb;1;2;0;old",
            "Axe;ax2;5;7;0;",
            "Big Club;clb;9;8;1;",
        ],
    );
    let gear = patch_table(
        patch_rules(
            "gear",
            Some("code"),
            KeyKind::Code,
            true,
            true,
            &["name", "code"],
        ),
        &["name", "code"],
        &["Cap;cap", "Belt;blt"],
    );
    let recipes = patch_table(
        patch_rules(
            "recipes",
            None,
            KeyKind::Exact,
            false,
            false,
            &["description", "enabled", "output"],
        ),
        &["description", "enabled", "output"],
        &["A;1;\"hp1,qty=3\"", "B;1;x", "C;0;y"],
    );
    PatchData::from_tables(vec![items, gear, recipes])
}

/// One layer `a.d2patch` of `stmts` after `table items`, on the fixture.
fn apply_items(stmts: &[&str]) -> (PatchData, Vec<Finding>) {
    let text = format!("d2patch 1\ntable items\n{}\n", stmts.join("\n"));
    let (layer, f): (Layer, _) = parse_layer("a.d2patch", text.as_bytes(), 1);
    assert!(f.is_empty(), "{f:?}");
    let mut data = patch_fixture();
    let f = apply_stack(&mut data, &[layer], "s.d2stack");
    (data, f)
}

// Not covered here: the policy that a format change needs a new version plus a
// migration, which no test can check; and a valid layer reorders to A05, which the
// version rules do not mention.
#[test]
fn patch_versioning_and_determinism() {
    // Line 1 names the version; this engine reads 1 (newer: P03, S02).
    assert_eq!(VERSION, 1);
    let layer_codes = |text: &str| -> Vec<Code> {
        let (_, f) = parse_layer("a.d2patch", text.as_bytes(), 1);
        f.iter().map(|f| f.code).collect()
    };
    let stack_codes = |text: &str| -> Vec<Code> {
        let (_, f) = parse_stack("s.d2stack", text.as_bytes());
        f.iter().map(|f| f.code).collect()
    };
    assert!(layer_codes("d2patch 1\ntable items\n").is_empty());
    assert_eq!(layer_codes("d2patch 2\ntable items\n"), [Code::P03]);
    assert!(stack_codes("d2stack 1\nlayer a.d2patch\n").is_empty());
    assert_eq!(stack_codes("d2stack 2\nlayer a.d2patch\n"), [Code::S02]);
    // Unknown statements are never ignored (deferred ones included).
    for s in ["frob axe", "remove axe", "delcol x", "[addcol] x int"] {
        let text = format!("d2patch 1\ntable items\n{s}\n");
        assert_eq!(layer_codes(&text), [Code::P04], "{s}");
    }
    // `addcol` is a statement of version 1: a malformed one is P05.
    assert_eq!(
        layer_codes("d2patch 1\ntable items\naddcol x\n"),
        [Code::P05]
    );

    // A valid layer: renames, appends from a template, sets, a check.
    let stmts = [
        "set #0 axe code axe -> ax9",
        "add #4 spr like ax2",
        "set spr lvl 5 -> 6",
        "set #1 clb dam@1 2 -> 3",
        "check #3 clb lvl 9",
        "add #5 hlm",
        "set hlm name [] -> Helm",
    ];
    let (reference, f) = apply_items(&stmts);
    assert!(f.is_empty(), "{f:?}");
    // Pure: the same inputs give the same cells, provenance, log and
    // report.
    assert_eq!(apply_items(&stmts), (reference.clone(), f));

    // Order independence: every permutation gives identical cells or
    // fails with A03, A08, A09, A14 or A15.
    let cells = |d: &PatchData| -> Vec<Vec<Vec<Vec<u8>>>> {
        d.tables
            .iter()
            .map(|t| t.rows.iter().map(|r| r.cells.clone()).collect())
            .collect()
    };
    let want = cells(&reference);
    let mut perm: Vec<usize> = (0..stmts.len()).collect();
    let mut all = Vec::new();
    permutations(&mut perm, 0, &mut all);
    let (mut valid, mut failed) = (0, 0);
    for p in all {
        let body: Vec<&str> = p.iter().map(|&i| stmts[i]).collect();
        let (d, f) = apply_items(&body);
        if has_errors(&f) {
            failed += 1;
            for x in f.iter().filter(|x| x.code.is_error()) {
                assert!(
                    matches!(
                        x.code,
                        Code::A03 | Code::A08 | Code::A09 | Code::A14 | Code::A15
                    ),
                    "{body:?}: {x}"
                );
            }
            continue;
        }
        valid += 1;
        assert_eq!(cells(&d), want, "{body:?}");
    }
    assert!(valid > 1 && failed > 0, "{valid} valid, {failed} failed");
}

// ============================================================ fix-ups

#[test]
fn stat_stuff_is_kept_in_1_to_8_else_6() {
    for (stuff, want) in [(0i32, 6u32), (1, 1), (5, 5), (8, 8), (9, 6), (-1, 6)] {
        let r0 = rec(0x144, &[(0x140, &stuff.to_le_bytes())]);
        let set = bin_set(
            vec![bin_table("itemstatcost", 0x144, vec![r0, vec![]])],
            StringTables::default(),
        );
        let f = fixup::apply(&set, &no_anim()).unwrap();
        assert_eq!(f.stat_stuff, want, "stuff {stuff}");
        // The record is not changed.
        let r = f.table("itemstatcost").unwrap().record(0);
        assert_eq!(i32_at(r, 0x140), stuff);
    }
}

#[test]
fn stat_op_base_slots_are_reset() {
    // Compiled bytes 0x11 everywhere except the op (0: nothing else runs).
    let records = (0..3)
        .map(|_| {
            let mut r = vec![0x11u8; 0x144];
            r[0x54] = 0;
            r
        })
        .collect();
    let mut t = bin_table("itemstatcost", 0x144, records);
    records::stat_ops(&mut t);
    for k in 0..3 {
        let r = t.record(k);
        assert!(r[0x5E..0xDE].iter().all(|&b| b == 0xFF), "record {k}");
        assert!(r[..0x54].iter().all(|&b| b == 0x11));
        assert!(r[0x55..0x5E].iter().all(|&b| b == 0x11));
        assert!(r[0xDE..].iter().all(|&b| b == 0x11));
    }
}

#[test]
fn stat_op_source_may_be_its_own_target() {
    // The pass runs on a set that passed the count check (the loader's record-count check):
    // 511 records pass it, 512 fail before any fix-up is applied.
    let strings = StringTables::default();
    let many = |n: usize| bin_table("itemstatcost", 0x144, vec![vec![]; n]);
    assert_eq!(many(1).record_size, 0x144);
    assert!(crate::bin::post_load_check(&many(511), &[], &strings, true).is_ok());
    assert!(crate::bin::post_load_check(&many(512), &[], &strings, true).is_err());
    // Stat 1: op 2, param 3, no base (0xFFFF), op stat 1 (itself).
    let s1 = rec(
        0x144,
        &[
            (0x54, &[2, 3]),
            (0x56, &0xFFFFu16.to_le_bytes()),
            (0x58, &1u16.to_le_bytes()),
            (0x5A, &0xFFFFu16.to_le_bytes()),
            (0x5C, &0xFFFFu16.to_le_bytes()),
        ],
    );
    let mut t = bin_table("itemstatcost", 0x144, vec![vec![], s1]);
    records::stat_ops(&mut t);
    let r = t.record(1);
    // The entry in its own table: base as stored, source 1, op, param.
    assert_eq!(
        (get_u16(r, 0xDE), get_u16(r, 0xE0), r[0xE2], r[0xE3]),
        (0xFFFF, 1, 2, 3)
    );
    assert_eq!((r[0x51], r[0x52], r[0x53]), (1, 1, 0));
    // +0x13E–0x13F stay 0.
    for k in 0..2 {
        assert_eq!(&t.record(k)[0x13E..0x140], [0, 0]);
    }
}

#[test]
fn levels_pass_runs_per_record_after_count_check() {
    let strings = StringTables::default();
    let mut recs: Vec<Vec<u8>> = (0..2).map(|_| rec(0x220, &[])).collect();
    recs[1][0x36..0x38].copy_from_slice(&3i16.to_le_bytes());
    recs[1][0x38..0x3A].copy_from_slice(&(-1i16).to_le_bytes());
    let mut t = bin_table("levels", 0x220, recs);
    assert_eq!(t.record_size, 0x220);
    assert!(crate::bin::post_load_check(&t, &[], &strings, true).is_ok());
    records::levels(&mut t, &strings).unwrap();
    // Every record is processed: the counts are per record.
    assert_eq!(t.record(0)[0x33], 25);
    assert_eq!(t.record(1)[0x33], 1);
    let big = bin_table("levels", 0x220, vec![vec![]; 1024]);
    assert!(crate::bin::post_load_check(&big, &[], &strings, true).is_err());
}

#[test]
fn charstats_class_name_is_wide() {
    let s = strings(&[(b"ama", 3, b"Amazon")]);
    let row = |class: &[u8]| {
        let mut r = rec(0xC4, &[(0x20, class)]);
        r[..0x20].fill(0x55);
        r
    };
    let mut t = bin_table("charstats", 0xC4, vec![row(b"ama"), row(b"zzz"), row(b"")]);
    records::charstats(&mut t, &s).unwrap();
    // Hit: the text, the rest of the 16 units zero.
    let r = t.record(0);
    assert_eq!(wide(r, 0, 16), "Amazon");
    assert!(r[12..0x20].iter().all(|&b| b == 0));
    // Miss: the miss text, cut to 16 units with no terminator.
    let r = t.record(1);
    let want: Vec<u8> = b"zzz -not xlated ".iter().flat_map(|&b| [b, 0]).collect();
    assert_eq!(&r[..0x20], want);
    // Empty key: all 32 bytes zero.
    assert!(t.record(2)[..0x20].iter().all(|&b| b == 0));
    // The key itself is untouched.
    assert_eq!(cstr(t.record(0), 0x20..0x30), b"ama");
}

#[test]
fn other_string_id_rows() {
    let s = strings(&[
        (b"Keen", 7, b"Keen"),
        (b"Hawk", 9, b"Hawk"),
        (b"Crude", 11, b"Crude"),
        (b"El", 13, b"El Rune"),
        (b"Ann", 15, b"Ann"),
        (b"Cole", 17, b"Cole"),
    ]);
    // (table, [(key offset, id offset)]).
    type Pair = (usize, usize);
    let cases: [(&str, &[Pair]); 9] = [
        ("magicprefix", &[(0x00, 0x20)]),
        ("magicsuffix", &[(0x00, 0x20)]),
        ("automagic", &[(0x00, 0x20)]),
        ("rareprefix", &[(0x26, 0x0C)]),
        ("raresuffix", &[(0x26, 0x0C)]),
        ("qualityitems", &[(0x2C, 0x6C), (0x4C, 0x6E)]),
        ("lowqualityitems", &[(0x00, 0x20)]),
        ("runes", &[(0x00, 0x82)]),
        ("hireling", &[(0xD3, 0x114), (0xF3, 0x116)]),
    ];
    let keys: [(&[u8], u16); 6] = [
        (b"Keen", 7),
        (b"Hawk", 9),
        (b"Crude", 11),
        (b"El", 13),
        (b"Ann", 15),
        (b"Cole", 17),
    ];
    for (name, pairs) in cases {
        let size = record_size(name);
        // Row 0: hits; row 1: misses; row 2: empty keys. Ids prefilled.
        let mut rows = vec![vec![0u8; size]; 3];
        for (k, &(key_at, id_at)) in pairs.iter().enumerate() {
            let (key, _) = keys[k];
            rows[0][key_at..key_at + key.len()].copy_from_slice(key);
            rows[1][key_at..key_at + 4].copy_from_slice(b"Nope");
            for r in &mut rows {
                r[id_at..id_at + 2].copy_from_slice(&0x7777u16.to_le_bytes());
            }
        }
        let set = bin_set(vec![bin_table(name, size, rows)], s.clone());
        let f = fixup::apply(&set, &no_anim()).unwrap();
        let t = f.table(name).unwrap();
        for (k, &(_, id_at)) in pairs.iter().enumerate() {
            assert_eq!(get_u16(t.record(0), id_at), keys[k].1, "{name} hit");
            assert_eq!(get_u16(t.record(1), id_at), 0, "{name} miss");
            assert_eq!(get_u16(t.record(2), id_at), 0, "{name} empty");
        }
    }
    // Unlike these, unique and set item names miss to 5,383.
    assert_eq!(MISSING_ITEM_NAME, 5383);
}

/// monstats rows (BaseId, NextInClass).
fn chain_table(rows: &[(i16, i16)]) -> BinTable {
    bin_table(
        "monstats",
        0x1A8,
        rows.iter()
            .map(|(b, n)| rec(0x1A8, &[(0x02, &b.to_le_bytes()), (0x04, &n.to_le_bytes())]))
            .collect(),
    )
}

fn anim(records: &[(&[u8], u32)]) -> AnimData {
    let mut a = no_anim();
    for (name, speed) in records {
        let mut r = animdata::DEFAULT_RECORD;
        r.name[..name.len()].copy_from_slice(name);
        r.speed = *speed;
        a.buckets[animdata::hash(name)].push(r);
    }
    a
}

fn monmode() -> BinTable {
    let mut t = bin_table("monmode", 0x34, vec![vec![]; 16]);
    rec_mut(&mut t, 2)[0x20..0x24].copy_from_slice(b"WL  ");
    rec_mut(&mut t, 15)[0x20..0x24].copy_from_slice(b"RN  ");
    t
}

/// monstats rows of class `aa` after pass B: (BaseId, Velocity, Run,
/// compiled +0x36); walk speed `walk` for `AAWLHTH`.
fn speeds(rows: &[(i16, i16, i16, i16)], walk: u32) -> BinTable {
    let mut t = bin_table(
        "monstats",
        0x1A8,
        rows.iter()
            .map(|(b, vel, run, w)| {
                rec(
                    0x1A8,
                    &[
                        (0x02, &b.to_le_bytes()),
                        (0x04, &(-1i16).to_le_bytes()),
                        (0x10, b"aa  "),
                        (0x18, &(-1i16).to_le_bytes()),
                        (0x32, &vel.to_le_bytes()),
                        (0x34, &run.to_le_bytes()),
                        (0x36, &w.to_le_bytes()),
                    ],
                )
            })
            .collect(),
    );
    let m2 = bin_table("monstats2", 0x134, vec![]);
    records::monstats_speeds(&mut t, &m2, &monmode(), &anim(&[(b"AAWLHTH", walk)])).unwrap();
    t
}

#[test]
fn fixup_edge_cases() {
    let s = strings(&[(b"x", 7, b"Ex"), (b"y", 8, b"Why")]);

    // gems +0x2C looks up the item index bytes: 120 → `x`, 121 → `y`.
    // Loop 2 resets items 0 … gem count − 1, whatever they are.
    let item = || rec(424, &[(0xF0, &7i32.to_le_bytes())]);
    let mut items = vec![
        bin_table("weapons", 424, (0..130).map(|_| item()).collect()),
        bin_table("armor", 424, vec![]),
        bin_table("misc", 424, vec![]),
    ];
    let gem = |j: i32| rec(0xC0, &[(0x28, &j.to_le_bytes())]);
    let mut gems = bin_table("gems", 0xC0, vec![gem(120), gem(121)]);
    records::gems(&mut gems, &mut items, &s).unwrap();
    assert_eq!(get_u16(gems.record(0), 0x2C), 7);
    assert_eq!(get_u16(gems.record(1), 0x2C), 8);
    let off = |k: usize| i32_at(items[0].record(k), 0xF0);
    assert_eq!([off(0), off(1), off(2)], [-1, -1, 7]);
    assert_eq!([off(120), off(121)], [0, 1]);

    // A 7th item of a set is skipped silently; its +0x2E = 0 looks like
    // slot 0.
    let mut sets = bin_table(
        "sets",
        0x128,
        vec![rec(0x128, &[(0x04, &100u16.to_le_bytes())])],
    );
    let mut setitems = bin_table("setitems", 0x1B8, vec![vec![]; 7]);
    records::attach_set_items(&mut setitems, &mut sets).unwrap();
    assert_eq!(u32_at(sets.record(0), 0x0C), 6);
    let r = setitems.record(6);
    assert_eq!((get_u16(r, 0x2E), get_u16(r, 0x22)), (0, 0));
    assert_eq!(get_u16(setitems.record(0), 0x2E), 0);
    assert_eq!(get_u16(setitems.record(0), 0x22), 100);

    // monstats: the run base for a row < 410 reads +0x36 of a later BaseId
    // row as compiled (0).
    let t = speeds(&[(1, 0, 0, 0), (1, 0, 0, 0)], 100);
    let walk = |t: &BinTable, r: usize| get_u16(t.record(r), 0x36);
    let run = |t: &BinTable, r: usize| get_u16(t.record(r), 0x38);
    assert_eq!((walk(&t, 0), run(&t, 0)), (100, 0));
    assert_eq!((walk(&t, 1), run(&t, 1)), (100, 50));
    // No lower clamp: negative products become 32,767.
    let t = speeds(&[(0, 2, 0, 0), (0, -1, 0, 0)], 100);
    assert_eq!(walk(&t, 1), SPEED_CAP as u16);
    let t = speeds(&[(0, 0, 2, 0), (0, 0, -2, 0)], 100);
    assert_eq!(run(&t, 1), SPEED_CAP as u16);

    // The miss text drops its last character and is cut to the field.
    let text = wide_text(&StringTables::default(), "t", b"abc").unwrap();
    let mut r = vec![0u8; 80];
    copy_wide(&mut r, 0, &text, 40);
    assert_eq!(wide(&r, 0, 40), "abc -not xlated call ken ");
    let mut lv = rec(0x220, &[(0x11D, b"To The Pandemonium Run 1")]);
    // levels counts stop at the first negative entry.
    for (k, x) in [5i16, -1, 7].iter().enumerate() {
        lv[0x36 + 2 * k..0x38 + 2 * k].copy_from_slice(&x.to_le_bytes());
    }
    let mut levels = bin_table("levels", 0x220, vec![lv]);
    records::levels(&mut levels, &StringTables::default()).unwrap();
    let r = levels.record(0);
    assert_eq!(
        wide(r, 0x1BE, 40),
        "To The Pandemonium Run 1 -not xlated ca"
    );
    assert_eq!(r[0x33], 1);

    // Out of range: a load error instead.
    assert!(records::monstats_chains(&mut chain_table(&[(-1, -1)])).is_err());
    assert!(records::monstats_chains(&mut chain_table(&[(1, -1)])).is_err());
    assert!(records::monstats_chains(&mut chain_table(&[(0, 1)])).is_err());
    assert!(records::monstats_chains(&mut chain_table(&[(0, -1)])).is_ok());
    let mut bad = bin_table("gems", 0xC0, vec![gem(130)]);
    assert!(records::gems(&mut bad, &mut items, &s).is_err());
    let path = |n: usize| {
        let mut r = vec![0u8; 60];
        r[..n].fill(b'x');
        fix_path(&mut r, 0, "lvlsub")
    };
    assert!(path(41).is_ok());
    assert!(path(42).is_err());
}

// ====================================================== runtime maps

/// Equivalence rows (e1, e2, e3); e3 only for montype.
fn equiv_table(name: &str, links: &[(i16, i16, i16)]) -> BinTable {
    let (size, o) = if name == "itemtypes" {
        (0xE4, [4, 6, 8])
    } else {
        (0x0C, [2, 4, 6])
    };
    bin_table(
        name,
        size,
        links
            .iter()
            .map(|(a, b, c)| {
                let mut r = rec(size, &[(o[0], &a.to_le_bytes()), (o[1], &b.to_le_bytes())]);
                if name == "montype" {
                    r[o[2]..o[2] + 2].copy_from_slice(&c.to_le_bytes());
                }
                r
            })
            .collect(),
    )
}

#[test]
fn runtime_map_edge_cases() {
    // Unstable sort order for equal keys.
    let mut v: Vec<(u8, usize)> = (0..8).map(|i| (0, i)).collect();
    qsort(&mut v, |a, b| a.0.cmp(&b.0));
    let order: Vec<usize> = v.into_iter().map(|(_, i)| i).collect();
    assert_eq!(order, [1, 2, 3, 4, 5, 6, 7, 0]);

    // itemtypes column 0 is set in every row; montype column 0 never.
    let rows = [(0, 0, 0), (0, 0, 0), (1, 0, 0)];
    let it = maps::equiv_matrix(&equiv_table("itemtypes", &rows), EquivKind::ItemTypes).unwrap();
    let mt = maps::equiv_matrix(&equiv_table("montype", &rows), EquivKind::MonType).unwrap();
    for i in 0..3 {
        assert!(it.get(i, 0) && !mt.get(i, 0), "row {i}");
    }

    // The walk gives up on a link ≥ n even if another branch would match:
    // row 3 links 1 (e1) and 7 (e2); 7 is popped first.
    let t = equiv_table("itemtypes", &[(0, 0, 0), (0, 0, 0), (0, 0, 0), (1, 7, 0)]);
    let m = maps::equiv_matrix(&t, EquivKind::ItemTypes).unwrap();
    assert!(!m.get(3, 1));
    let t = equiv_table("itemtypes", &[(0, 0, 0), (0, 0, 0), (0, 0, 0), (1, 0, 0)]);
    assert!(maps::equiv_matrix(&t, EquivKind::ItemTypes)
        .unwrap()
        .get(3, 1));
    // ... and on a deep stack: row k ≥ 2 links 1 (a dead end) and k + 1,
    // so the stack grows by one per step.
    let n = 200i16;
    let rows: Vec<(i16, i16, i16)> = (0..n)
        .map(|k| match k {
            0 | 1 => (0, 0, 0),
            k if k == n - 1 => (1, 0, 0),
            k => (1, k + 1, 0),
        })
        .collect();
    let m = maps::equiv_matrix(&equiv_table("itemtypes", &rows), EquivKind::ItemTypes).unwrap();
    assert!(m.get(2, 100));
    // Popping 127 leaves 125 entries: it still matches itself, then stops.
    assert!(m.get(2, 127));
    assert!(!m.get(2, 128));
    assert!(!m.get(2, 199));

    // Out-of-order monpreset / lvlsub / monseq rows are counted, not
    // rejected.
    let t = bin_table(
        "monpreset",
        4,
        [1u8, 3, 2].iter().map(|&a| vec![a]).collect(),
    );
    let a = maps::monpreset(&t).unwrap();
    assert_eq!(&a.first[..3], [Some(0), Some(1), Some(1)]);
    assert_eq!(&a.count[..3], [1, 0, 2]);
    let t = bin_table(
        "lvlsub",
        0x15C,
        [0i32, 2, 1, 2]
            .iter()
            .map(|x| rec(0x15C, &[(0, &x.to_le_bytes())]))
            .collect(),
    );
    assert_eq!(maps::lvlsub_types(&t).unwrap(), [0, 2, 3]);
    let seq = |s: &[i16]| {
        bin_table(
            "monseq",
            6,
            s.iter().map(|x| rec(6, &[(0, &x.to_le_bytes())])).collect(),
        )
    };
    let e = maps::monseq(&seq(&[1, 0, 1])).unwrap();
    let got: Vec<(Option<u32>, u32)> = e.iter().map(|x| (x.first, x.count)).collect();
    assert_eq!(got, [(Some(1), 1), (Some(0), 2)]);

    // Out of range: a load error.
    for (name, kind, rows) in [
        (
            "itemtypes",
            EquivKind::ItemTypes,
            vec![(0, 0, 0), (1, -2, 0), (0, 0, 0)],
        ),
        (
            "montype",
            EquivKind::MonType,
            vec![(0, 0, 0), (1, -2, 0), (0, 0, 0)],
        ),
        (
            "montype",
            EquivKind::MonType,
            vec![(0, 0, 0), (0, 0, 0), (1, 1, -3)],
        ),
    ] {
        assert!(maps::equiv_matrix(&equiv_table(name, &rows), kind).is_err());
    }
    // e2 is not pushed when e1 ≤ 0: no error.
    let t = equiv_table("itemtypes", &[(0, 0, 0), (0, -2, 0), (0, 0, 0)]);
    assert!(maps::equiv_matrix(&t, EquivKind::ItemTypes).is_ok());
    assert!(maps::monseq(&seq(&[-1, 0])).is_err());
    assert!(maps::monseq(&seq(&[0, 3, 1])).is_err());
    assert!(maps::monseq(&seq(&[0, -1])).is_err());
    let acts = |a: u8| bin_table("monpreset", 4, vec![vec![a]]);
    assert!(maps::monpreset(&acts(5)).is_ok());
    assert!(maps::monpreset(&acts(6)).is_err());
    let hire = bin_table(
        "hireling",
        0x118,
        vec![rec(0x118, &[(4, &(-1i32).to_le_bytes())])],
    );
    assert!(maps::hireling_first(&hire).is_err());
    let t = bin_table(
        "lvlsub",
        0x15C,
        vec![rec(0x15C, &[(0, &(-1i32).to_le_bytes())])],
    );
    assert!(maps::lvlsub_types(&t).is_err());
}

// ========================================================= callbacks

fn code_u32(t: &[u8]) -> u32 {
    u32::from_le_bytes(code4(t))
}

/// A code linker holding `codes` at the given indices, filler elsewhere.
fn code_linker(n: u32, codes: &[(&[u8], u32)]) -> Linker {
    let mut l = CodeLinker::default();
    for i in 0..n {
        let code = codes
            .iter()
            .find(|(_, at)| *at == i)
            .map(|(c, _)| code_u32(c))
            .unwrap_or(0x8000_0000 + i);
        l.add(code);
    }
    Linker::Code(l)
}

fn name_linker(n: u32, names: &[(&[u8], u32)]) -> Linker {
    let mut l = NameLinker::default();
    for i in 0..n {
        let key = names
            .iter()
            .find(|(_, at)| *at == i)
            .map(|(k, _)| name_key(k).unwrap())
            .unwrap_or_else(|| format!("\x01filler{i}").into_bytes());
        l.add_always(&key);
    }
    Linker::Name(l)
}

/// Items `hax` 0, `axe` 1, `rin` 2, `lrg` 3; item types `"    "` 0, `axe`
/// 5; unique 0 `Ring of Ice` (`rin`, lvl 39), unique 1 `Lost Thing` (base
/// `zzz`, not an item); set item 0 `Iron Ward` (`lrg`, lvl 13).
fn cube_linkers() -> (Linkers, SpecialItems) {
    let mut l = Linkers::default();
    l.insert(
        "items.code",
        code_linker(10, &[(b"hax", 0), (b"axe", 1), (b"rin", 2), (b"lrg", 3)]),
    );
    l.insert("itemtypes.code", code_linker(10, &[(b"", 0), (b"axe", 5)]));
    l.insert(
        UNIQUES_LINKER,
        name_linker(2, &[(b"Ring of Ice", 0), (b"Lost Thing", 1)]),
    );
    l.insert(SETS_LINKER, name_linker(1, &[(b"Iron Ward", 0)]));
    let sp = SpecialItems {
        uniques: vec![
            SpecialItem {
                code: code_u32(b"rin"),
                lvl: 39,
            },
            SpecialItem {
                code: code_u32(b"zzz"),
                lvl: 5,
            },
        ],
        sets: vec![SpecialItem {
            code: code_u32(b"lrg"),
            lvl: 13,
        }],
    };
    (l, sp)
}

fn field(table: &str, column: &str) -> FieldDef {
    schema()
        .table(table)
        .unwrap()
        .field(column)
        .unwrap_or_else(|| panic!("{table} {column}"))
        .clone()
}

/// Compiles `text` (header line and rows, CR LF) with `fields`.
fn compile_with(
    table: &str,
    text: &str,
    fields: &[FieldDef],
    linkers: &mut Linkers,
    special: &SpecialItems,
) -> Result<Compiled, crate::txt::TxtError> {
    let strings = StringTables::default();
    let mut cb = StdCallbacks::new(&strings);
    cb.special = special.clone();
    let txt = TxtTable::parse("t.txt", text.as_bytes()).unwrap();
    compile_table("t.txt", &txt, fields, record_size(table), linkers, &mut cb)
}

/// cubemain with `columns` (field names, left to right) and one row.
fn cube(columns: &[&str], row: &str) -> Compiled {
    let (mut l, sp) = cube_linkers();
    let fields: Vec<FieldDef> = columns.iter().map(|c| field("cubemain", c)).collect();
    let text = format!("{}\r\n{row}\r\n", columns.join("\t"));
    compile_with("cubemain", &text, &fields, &mut l, &sp).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

/// Input slot 0 bytes and diagnostics.
fn cube_in(text: &str) -> (Vec<u8>, Vec<DiagKind>) {
    let c = cube(&["input 1"], text);
    let kinds = c.diagnostics.iter().map(|d| d.kind).collect();
    (c.record(0)[20..28].to_vec(), kinds)
}

/// Output slot 0 bytes (first 24) and diagnostics.
fn cube_out(text: &str) -> (Vec<u8>, Vec<DiagKind>) {
    let c = cube(&["output"], text);
    let kinds = c.diagnostics.iter().map(|d| d.kind).collect();
    (c.record(0)[76..100].to_vec(), kinds)
}

#[test]
fn special_item_lookups() {
    // @uniques: the `index` text at offset 2, add-always in record order;
    // a duplicate name finds its first record. Unique `code` u32 at 40 and
    // `lvl` u16 at 52; set item `item` at 40, `lvl` at 48.
    let unique = |name: &[u8], code: &[u8], lvl: u16| {
        rec(
            0x14C,
            &[(2, name), (40, &code4(code)), (52, &lvl.to_le_bytes())],
        )
    };
    let records = [
        unique(b"Ring of Ice", b"rin", 39),
        unique(b"Other", b"hax", 2),
        unique(b"ring of ice", b"lrg", 77),
    ];
    let (ul, uitems) = special_linker(records.iter().map(Vec::as_slice), 40, 52).unwrap();
    assert_eq!(ul.len(), 3);
    assert_eq!(ul.find(b"ring of ice"), Some(0));
    assert_eq!(ul.find(b"other"), Some(1));
    assert_eq!(
        uitems,
        [
            SpecialItem {
                code: code_u32(b"rin"),
                lvl: 39
            },
            SpecialItem {
                code: code_u32(b"hax"),
                lvl: 2
            },
            SpecialItem {
                code: code_u32(b"lrg"),
                lvl: 77
            },
        ]
    );
    let set = rec(
        0x1B8,
        &[
            (2, b"Iron Ward"),
            (40, &code4(b"lrg")),
            (48, &13u16.to_le_bytes()),
        ],
    );
    let (sl, sitems) = special_linker([set.as_slice()], 40, 48).unwrap();
    assert_eq!(sl.find(b"iron ward"), Some(0));
    assert_eq!(
        sitems,
        [SpecialItem {
            code: code_u32(b"lrg"),
            lvl: 13
        }]
    );

    // Through the cube callbacks: unique and set numbers, base items.
    let linkers = |uniques: Option<&NameLinker>| {
        let mut l = Linkers::default();
        l.insert(
            "items.code",
            code_linker(10, &[(b"hax", 0), (b"rin", 2), (b"lrg", 3)]),
        );
        l.insert("itemtypes.code", code_linker(4, &[]));
        if let Some(u) = uniques {
            l.insert(UNIQUES_LINKER, Linker::Name(u.clone()));
        }
        l.insert(SETS_LINKER, Linker::Name(sl.clone()));
        l
    };
    let sp = SpecialItems {
        uniques: uitems,
        sets: sitems,
    };
    let f = [field("cubemain", "output")];
    let c = compile_with(
        "cubemain",
        "output\r\nRing of Ice\r\nIron Ward\r\n",
        &f,
        &mut linkers(Some(&ul)),
        &sp,
    )
    .unwrap();
    assert_eq!(
        c.record(0)[76..88],
        hex("08 00 02 00 01 00 07 00 fc 00 00 27")
    );
    assert_eq!(
        c.record(1)[76..88],
        hex("08 00 03 00 01 00 05 00 fc 00 00 0d")
    );

    // `@uniques` absent: that lookup is skipped (a set name still hits).
    let c = compile_with(
        "cubemain",
        "input 1\r\nRing of Ice\r\nIron Ward\r\n",
        &[field("cubemain", "input 1")],
        &mut linkers(None),
        &sp,
    )
    .unwrap();
    assert_eq!(c.record(0)[20..28], [0; 8]);
    assert_eq!(c.record(1)[20..28], hex("41 00 03 00 01 00 05 00"));
    assert_eq!(c.diagnostics.len(), 1);
    assert_eq!(c.diagnostics[0].kind, DiagKind::CbMiss);

    // Any other linker missing: E13.
    let mut no_items = Linkers::default();
    no_items.insert("itemtypes.code", code_linker(4, &[]));
    let e = compile_with(
        "cubemain",
        "input 1\r\nrin\r\n",
        &[field("cubemain", "input 1")],
        &mut no_items,
        &sp,
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::E13);

    // In `.bin` mode the loaders build them the same way.
    let mut uniques = bin_table("uniqueitems", 0x14C, records.to_vec());
    for r in 0..3 {
        rec_mut(&mut uniques, r)[0x22..0x24].fill(0);
    }
    let set = bin_set(vec![uniques], StringTables::default());
    let fixed = fixup::apply(&set, &no_anim()).unwrap();
    assert_eq!(fixed.uniques.len(), 3);
    assert_eq!(fixed.uniques.find(b"ring of ice"), Some(0));
    assert_eq!(fixed.uniques.find(b"other"), Some(1));
}

#[test]
fn cubemain_param_is_a_plain_u32() {
    let param = field("cubemain", "param");
    assert_eq!(param.field_type, FieldType::Dword);
    assert_eq!(param.link, Link::Table("cubemain.param".into()));
    // The same field without the routine in its link slot.
    let plain = FieldDef::new("param", FieldType::Dword, 0, param.offset, Link::None);
    let (_, sp) = cube_linkers();
    for cell in ["17", "-5", "strength", "", "12abc"] {
        let text = format!("param\r\n{cell}\r\n");
        // An itemstatcost link that the dead routine would have used.
        let mut l = Linkers::default();
        l.insert("itemstatcost.stat", name_linker(5, &[(b"strength", 3)]));
        let strings = StringTables::default();
        let mut cb = StdCallbacks::new(&strings);
        cb.special = sp.clone();
        let txt = TxtTable::parse("t.txt", text.as_bytes()).unwrap();
        let a = compile_table(
            "t.txt",
            &txt,
            std::slice::from_ref(&param),
            328,
            &mut l,
            &mut cb,
        )
        .unwrap();
        // The routine never runs: not counted as a callback call.
        assert!(cb.unspecified.is_empty(), "{cell}");
        let b = compile_with(
            "cubemain",
            &text,
            std::slice::from_ref(&plain),
            &mut Linkers::default(),
            &sp,
        )
        .unwrap();
        assert_eq!(a.records, b.records, "{cell}");
    }
    let c = cube(&["param"], "-5");
    assert_eq!(u32_at(c.record(0), param.offset as usize), (-5i32) as u32);
}

/// monstats with `columns` (left to right) and one row; returns (mode,
/// sequence) of slot 0 and the skill field.
fn skill_mode(columns: &[&str], row: &str) -> (u8, u16, u16) {
    let mut l = Linkers::default();
    l.insert("skills.skill", name_linker(3, &[(b"fire", 0)]));
    let modes = [
        "DT", "NU", "WL", "GH", "A1", "A2", "BL", "SC", "S1", "S2", "S3", "S4", "DD", "KB", "xx",
        "RN",
    ];
    let mut mm = CodeLinker::default();
    for m in modes {
        mm.add(code_u32(m.as_bytes()));
    }
    l.insert("monmode_lookup.code", Linker::Code(mm));
    l.insert("monseq.sequence", name_linker(3, &[(b"", 0)]));
    let fields = [field("monstats", "Skill1"), field("monstats", "Sk1mode")];
    let text = format!("{}\r\n{row}\r\n", columns.join("\t"));
    let c = compile_with("monstats", &text, &fields, &mut l, &SpecialItems::default()).unwrap();
    let r = c.record(0);
    (r[384], get_u16(r, 392), get_u16(r, 368))
}

/// monstats2 `HDv` / `S8v` with `columns` and one row: (HDv count, total).
fn composit_total(columns: &[&str], row: &str) -> (u8, u8) {
    let mut l = Linkers::default();
    l.insert(
        "compcode.code",
        code_linker(4, &[(b"nil", 0), (b"lit", 1), (b"med", 2)]),
    );
    let fields = [field("monstats2", "HDv"), field("monstats2", "S8v")];
    let text = format!("{}\r\n{row}\r\n", columns.join("\t"));
    let c = compile_with(
        "monstats2",
        &text,
        &fields,
        &mut l,
        &SpecialItems::default(),
    )
    .unwrap();
    (c.record(0)[21], c.record(0)[37])
}

#[test]
fn callback_edge_cases() {
    use DiagKind::{CbMiss, CbStop};
    let z = |n: usize| vec!["00"; n].join(" ");

    // Set outputs skip their modifiers; an input set name reads them.
    assert_eq!(
        cube_out("\"Iron Ward,qty=2\""),
        (
            hex(&format!("08 00 03 00 01 00 05 00 fc 00 00 0d {}", z(12))),
            vec![CbStop]
        )
    );
    assert_eq!(
        cube_in("\"Iron Ward,mag\"").0,
        hex("41 00 03 00 01 00 04 00")
    );

    // Unique and set outputs write `ilvl` (+11); the `ilvl` column to the
    // right overwrites it (an empty cell writes 0); without the column the
    // item's level stays.
    let ilvl = |columns: &[&str], row: &str| cube(columns, row).record(0)[87];
    assert_eq!(ilvl(&["output", "ilvl"], "Ring of Ice\t"), 0);
    assert_eq!(ilvl(&["output", "ilvl"], "Ring of Ice\t5"), 5);
    assert_eq!(ilvl(&["output"], "Ring of Ice"), 39);

    // Non-`qty` words with `=v` stop the input parse at `v`.
    assert_eq!(
        cube_in("\"hax,sock=2,eth\""),
        (hex("09 00 00 00 00 00 00 00"), vec![CbStop])
    );
    // In outputs only `qty`, `pre`, `suf`, `sock` take values.
    assert_eq!(
        cube_out("\"rin,sock=3,eth\"").0,
        hex(&format!("06 00 02 00 00 00 00 03 fc {}", z(15)))
    );
    assert_eq!(
        cube_out("\"rin,eth=1,sock=3\""),
        (
            hex(&format!("04 00 02 00 00 00 00 00 fc {}", z(15))),
            vec![CbStop]
        )
    );

    // An unknown word stops the parse; later valid words are lost.
    assert_eq!(
        cube_out("\"usetype,foo,mag\""),
        (hex(&format!("{} ff {}", z(8), z(15))), vec![CbStop])
    );

    // An empty first token is code "    ", owned by itemtypes row 0.
    assert_eq!(cube_in("\",qty=2\"").0, hex("02 00 00 00 00 00 00 02"));
    assert_eq!(cube_out("\"\"").0, hex(&format!("{} fd {}", z(8), z(15))));
    // Input text that is empty after unquote writes nothing: the bytes a
    // field to the left wrote stay.
    let pad = FieldDef::new("pad", FieldType::Dword, 0, 20, Link::None);
    let (mut l, sp) = cube_linkers();
    let c = compile_with(
        "cubemain",
        "pad\tinput 1\r\n1431655765\t\"\"\r\n1431655765\t\r\n",
        &[pad, field("cubemain", "input 1")],
        &mut l,
        &sp,
    )
    .unwrap();
    for r in 0..2 {
        assert_eq!(u32_at(c.record(r), 20), 0x5555_5555, "row {r}");
        assert_eq!(c.record(r)[24..28], [0; 4]);
    }

    // Text after a second quote is dropped.
    assert_eq!(
        cube_in("rin\",mag\""),
        (hex("01 00 02 00 00 00 00 00"), vec![])
    );

    // Inputs test item types first, outputs item codes first.
    assert_eq!(cube_in("axe").0, hex("02 00 05 00 00 00 00 00"));
    assert_eq!(
        cube_out("axe").0,
        hex(&format!("00 00 01 00 {} fc {}", z(4), z(15)))
    );

    // A unique whose base code is not an item gets item 0 (as `hax`).
    assert_eq!(cube_in("Lost Thing").0, hex("41 00 00 00 02 00 07 00"));
    assert_eq!(
        cube_out("Lost Thing").0[..12],
        hex("08 00 00 00 02 00 07 00 fc 00 00 05")
    );
    assert!(cube_out("Lost Thing").1.is_empty());

    // `pre` beyond 3 overwrites the first suffix slot.
    assert_eq!(
        cube_out("\"rin,pre=1,pre=2,pre=3,pre=4\"").0,
        hex("00 00 02 00 00 00 00 00 fc 00 00 00 01 00 02 00 03 00 04 00 00 00 00 00")
    );

    // Skill-mode bytes depend on the skill column being compiled first:
    // the skill misses (−1) → no mode; right of the mode column or missing,
    // the callback reads 0 and treats it as a skill.
    assert_eq!(
        skill_mode(&["Skill1", "Sk1mode"], "nosuch\tA1"),
        (0, 0xFFFF, 0xFFFF)
    );
    assert_eq!(
        skill_mode(&["Skill1", "Sk1mode"], "fire\tA1"),
        (4, 0xFFFF, 0)
    );
    assert_eq!(
        skill_mode(&["Sk1mode", "Skill1"], "A1\tnosuch"),
        (4, 0xFFFF, 0xFFFF)
    );
    assert_eq!(skill_mode(&["Sk1mode"], "A1").0, 4);

    // The composit total is written only by the `S8v` call, from the
    // counts present then; a missing `S8v` column leaves it 0.
    assert_eq!(composit_total(&["HDv", "S8v"], "\"lit,med\"\t"), (2, 1));
    assert_eq!(composit_total(&["S8v", "HDv"], "\t\"lit,med\""), (2, 0));
    assert_eq!(composit_total(&["HDv"], "\"lit,med\""), (2, 0));

    // A miss writes nothing and is reported.
    assert_eq!(cube_in("qqq"), (vec![0; 8], vec![CbMiss]));
}
