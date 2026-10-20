//! Property tests (proptest) over the `d2-data` stages the parser
//! properties (`robust_tests.rs`, `calc/robust_tests.rs`,
//! `patch/robust_tests.rs`) do not reach: the txt → record compiler on
//! every table's field list, the whole-set compile, the `.bin` container
//! round trip into typed records, the post-load fix-ups and runtime maps
//! on arbitrary tables, and link validation. Any input returns (Ok or a
//! refusal), never a panic, a debug overflow, a hang or an unbounded
//! allocation.
//!
//! Default case counts keep `cargo test` fast; set `PROPTEST_CASES` to
//! hunt harder.

mod prop_common;

use std::collections::BTreeMap;

use proptest::prelude::*;

use d2_data::bin::{BinSet, BinTable, CodeFile};
use d2_data::compile::{compile_table, Linkers, StdCallbacks};
use d2_data::compile_set::compile_all;
use d2_data::fixup;
use d2_data::links::{validate, LinkerSizes};
use d2_data::schema::{schema, CalcBuffer, TableDef};
use d2_data::strings::StringTables;
use d2_data::tables::{decode_all, decode_by_name, Record};
use d2_data::txt::TxtTable;
use d2_formats::animdata::AnimData;

use prop_common::{bounded, config, Gen};

/// Cell texts: integers at the type boundaries, codes, names that link
/// to each other across tables, formulas, quotes, high bytes.
const CELLS: &[&[u8]] = &[
    b"",
    b"",
    b"",
    b"0",
    b"1",
    b"2",
    b"7",
    b"-1",
    b"-128",
    b"255",
    b"256",
    b"-32768",
    b"65535",
    b"65536",
    b"2147483647",
    b"-2147483648",
    b"4294967295",
    b"4294967296",
    b"99999999999999999999",
    b"+5",
    b"0x10",
    b"1.5",
    b" 3",
    b"axe",
    b"hax",
    b"weap",
    b"mele",
    b"armo",
    b"misc",
    b"gld",
    b"rin",
    b"amu",
    b"Axe",
    b"Fire Bolt",
    b"Attack",
    b"Neutral",
    b"Amazon",
    b"str",
    b"dmg%",
    b"fire",
    b"tbk",
    b"isk",
    b"act1",
    b"Act 1 Good",
    b"none",
    b"skeleton1",
    b"zombie1",
    b"NU",
    b"A1",
    b"Expansion",
    b"lvl*2",
    b"min(1,2)",
    b"ln12",
    b"par1+par2",
    b"skill('Fire Bolt'.ln12)",
    b"stat('strength'.accr)",
    b"rand(1,9)",
    b"(1",
    b"1?2:3",
    b"\"quoted\"",
    b"\x80\xFF",
    b"#",
    b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
];

/// Cells that pass the reader and the name-key checks: small integers,
/// codes and names, formulas.
const TAME_CELLS: &[&[u8]] = &[
    b"",
    b"",
    b"",
    b"",
    b"0",
    b"1",
    b"2",
    b"3",
    b"7",
    b"-1",
    b"255",
    b"axe",
    b"hax",
    b"weap",
    b"mele",
    b"armo",
    b"misc",
    b"rin",
    b"Axe",
    b"Fire Bolt",
    b"str",
    b"fire",
    b"tbk",
    b"act1",
    b"none",
    b"skeleton1",
    b"NU",
    b"A1",
    b"lvl*2",
    b"min(1,2)",
    b"ln12",
];

/// A `.txt` for `def`: its columns (some dropped, duplicated or swapped
/// for an unknown one, unless `tame`), then `rows` records of random
/// cells ([`TAME_CELLS`] when `tame`).
fn txt_for(def: &TableDef, g: &mut Gen, rows: usize) -> Vec<u8> {
    txt_in(def, g, rows, false)
}

fn txt_in(def: &TableDef, g: &mut Gen, rows: usize, tame: bool) -> Vec<u8> {
    let mut header: Vec<Vec<u8>> = Vec::new();
    for f in &def.fields {
        if header.contains(&f.column) {
            continue;
        }
        match if tame { 15 } else { g.below(16) } {
            0 => {}
            1 => {
                header.push(f.column.clone());
                header.push(f.column.clone());
            }
            2 => header.push(b"zz unknown".to_vec()),
            _ => header.push(f.column.clone()),
        }
    }
    if header.is_empty() {
        header.push(b"name".to_vec());
    }
    let mut out = header.join(&b'\t');
    out.extend_from_slice(b"\r\n");
    for _ in 0..rows {
        let alphabet = if tame { TAME_CELLS } else { CELLS };
        let cells: Vec<&[u8]> = (0..header.len()).map(|_| *g.pick(alphabet)).collect();
        out.extend_from_slice(&cells.join(&b'\t'));
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// The `.bin` container of `count` records.
fn bin_bytes(count: usize, records: &[u8]) -> Vec<u8> {
    let mut b = (count as u32).to_le_bytes().to_vec();
    b.extend_from_slice(records);
    b
}

/// Compiles one called table's random text with every linker it names
/// present (empty), then round-trips the records through the `.bin`
/// container and the typed decode.
fn compile_one(table: usize, seed: u64, rows: usize) {
    bounded(move || {
        let defs: Vec<&TableDef> = schema().called().collect();
        let def = defs[table % defs.len()];
        let mut g = Gen::new(seed);
        let text = txt_for(def, &mut g, rows);
        let Ok(txt) = TxtTable::parse(&def.txt_name, &text) else {
            return;
        };
        let mut linkers = Linkers::default();
        for f in &def.fields {
            if let (Some(kind), Some(name)) = (f.field_type.linker_kind(), f.link.linker()) {
                let _ = linkers.ensure(name, kind);
            }
        }
        let strings = StringTables::default();
        let mut callbacks = StdCallbacks::new(&strings);
        let Ok(c) = compile_table(
            &def.txt_name,
            &txt,
            &def.fields,
            def.record_size,
            &mut linkers,
            &mut callbacks,
        ) else {
            return;
        };
        assert_eq!(c.count, txt.records.len());
        assert_eq!(c.records.len(), c.count * def.record_size);
        round_trip(&def.name, def.record_size, c.count, &c.records);
    });
}

/// txt → bin → typed (the `.bin` container): the container
/// keeps the records byte for byte, and every typed table decodes the
/// same records from either side.
fn round_trip(name: &str, record_size: usize, count: usize, records: &[u8]) {
    let bin = bin_bytes(count, records);
    let t = BinTable::parse(name, "prop", "prop.bin", &bin, record_size).expect("container");
    assert_eq!(t.count, count);
    assert_eq!(t.records, records);
    for i in 0..count {
        assert_eq!(
            t.record(i),
            &records[i * record_size..(i + 1) * record_size]
        );
    }
    if let Some(r) = decode_by_name(&t) {
        assert_eq!(r.expect("schema size"), count);
    }
    match name {
        "weapons" => typed_equal::<d2_data::tables::Weapons>(&t, records),
        "skills" => typed_equal::<d2_data::tables::Skills>(&t, records),
        "uniqueitems" => typed_equal::<d2_data::tables::Uniqueitems>(&t, records),
        "monstats" => typed_equal::<d2_data::tables::Monstats>(&t, records),
        "itemstatcost" => typed_equal::<d2_data::tables::Itemstatcost>(&t, records),
        "levels" => typed_equal::<d2_data::tables::Levels>(&t, records),
        _ => {}
    }
}

fn typed_equal<T: Record + PartialEq + std::fmt::Debug>(t: &BinTable, records: &[u8]) {
    let from_bin = decode_all::<T>(t).expect("table");
    let from_txt: Vec<T> = records.chunks_exact(T::SIZE).map(T::decode).collect();
    assert_eq!(from_bin, from_txt);
}

/// The whole called sequence on random texts (linkers carried across
/// tables, the hand-built linkers), then the fix-ups on the result.
fn compile_set(seed: u64, rows: usize) {
    bounded(move || {
        let mut g = Gen::new(seed);
        let tame = !g.one_in(4);
        let strings = StringTables::default();
        let mut read = |file: &str| {
            let def = schema()
                .called()
                .find(|d| d.txt_name == file)
                .expect("asked for a called table");
            let n = 1 + g.below(rows);
            Ok(Some(("prop.mpq".to_owned(), txt_in(def, &mut g, n, tame))))
        };
        let Ok(set) = compile_all(&mut read, &strings) else {
            return;
        };
        let mut tables = Vec::new();
        for ct in &set.tables {
            let def = schema().table(&ct.name).expect("schema table");
            let c = &ct.compiled;
            round_trip(&ct.name, def.record_size, c.count, &c.records);
            if def.is_runtime() {
                tables.push(BinTable {
                    name: ct.name.clone(),
                    source: "prop.mpq".into(),
                    count: c.count,
                    record_size: c.record_size,
                    records: c.records.clone(),
                });
            }
        }
        let code = set
            .buffers
            .iter()
            .map(|(&buffer, bytes)| {
                (
                    buffer,
                    CodeFile {
                        buffer,
                        source: "prop.mpq".into(),
                        bytes: bytes.clone(),
                    },
                )
            })
            .collect();
        let mut set = bin_set(tables, code, true);
        if !g.one_in(3) {
            clear_refusing(&mut set);
        }
        fixups(set);
    });
}

fn empty_txt() -> TxtTable {
    TxtTable {
        header: vec![b"Sound".to_vec()],
        records: Vec::new(),
        removed_lines: Vec::new(),
    }
}

fn bin_set(tables: Vec<BinTable>, code: BTreeMap<CalcBuffer, CodeFile>, lod: bool) -> BinSet {
    BinSet {
        lod,
        rules: Default::default(),
        strings: StringTables::default(),
        tables,
        code,
        code_reports: BTreeMap::new(),
        hitclass: BinTable {
            name: "hitclass".into(),
            source: "prop".into(),
            count: 0,
            record_size: 4,
            records: Vec::new(),
        },
        sounds: empty_txt(),
        soundenviron: empty_txt(),
    }
}

/// An `AnimData.d2` with every bucket empty.
fn empty_animdata() -> AnimData {
    AnimData::parse(&[0u8; 256 * 4]).expect("256 empty buckets")
}

/// Fix-ups and runtime maps, then the typed decode and link validation of
/// the fixed tables.
fn fixups(set: BinSet) {
    let anim = empty_animdata();
    let fixed = fixup::apply(&set, &anim);
    let tables = match &fixed {
        Ok(f) => {
            for t in &f.tables {
                if let Some(r) = decode_by_name(t) {
                    assert_eq!(r.expect("schema size"), t.count);
                }
            }
            for i in 0..f.itemtypes_equiv.n.min(70) {
                for j in 0..f.itemtypes_equiv.n.min(70) {
                    let _ = f.itemtypes_equiv.get(i, j);
                }
            }
            f.tables.clone()
        }
        Err(_) => set.tables.clone(),
    };
    let sizes = LinkerSizes::from_tables(&tables);
    let report = validate(&tables, &sizes);
    let _ = report.is_clean();
}

/// Tables whose fix-ups refuse most random rows (links into other tables,
/// row order, act numbers).
const REFUSING: &[&str] = &[
    "gamble",
    "gems",
    "monseq",
    "monpreset",
    "automap",
    "monstats",
    "charstats",
    "setitems",
    "uniqueitems",
];

/// Empties the [`REFUSING`] tables.
fn clear_refusing(set: &mut BinSet) {
    for t in &mut set.tables {
        if REFUSING.contains(&t.name.as_str()) {
            t.count = 0;
            t.records.clear();
        }
    }
}

/// Every runtime table with 0–`max` random records, some tables left out
/// (`omit` in 1/n) and the code buffers filled with random bytes.
fn random_set(seed: u64, max: usize, omit: usize) -> BinSet {
    let mut g = Gen::new(seed);
    let nasty = g.one_in(4);
    let mut tables = Vec::new();
    for def in schema().runtime() {
        if omit > 0 && g.one_in(omit) {
            continue;
        }
        let count = g.below(max + 1);
        let records: Vec<u8> = (0..count)
            .flat_map(|_| g.record_in(def.record_size, nasty))
            .collect();
        tables.push(BinTable {
            name: def.name.clone(),
            source: "prop".into(),
            count,
            record_size: def.record_size,
            records,
        });
    }
    let code = CalcBuffer::ALL
        .iter()
        .map(|&buffer| {
            let n = g.below(64);
            (
                buffer,
                CodeFile {
                    buffer,
                    source: "prop".into(),
                    bytes: (0..n).map(|_| g.next() as u8).collect(),
                },
            )
        })
        .collect();
    let lod = g.one_in(2);
    bin_set(tables, code, lod)
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn compile_every_field_list(table in any::<usize>(), seed in any::<u64>(), rows in 1usize..6) {
        compile_one(table, seed, rows);
    }

    #[test]
    fn decode_every_table(seed in any::<u64>()) {
        bounded(move || {
            let mut g = Gen::new(seed);
            for def in schema().runtime() {
                let count = g.below(4);
                let records: Vec<u8> = (0..count).flat_map(|_| g.record(def.record_size)).collect();
                round_trip(&def.name, def.record_size, count, &records);
            }
        });
    }
}

/// A random table of runtime table `name` with 0–`max` records.
fn random_table(g: &mut Gen, name: &str, max: usize, nasty: bool) -> BinTable {
    let size = schema().table(name).expect("schema table").record_size;
    let count = g.below(max + 1);
    BinTable {
        name: name.into(),
        source: "prop".into(),
        count,
        record_size: size,
        records: (0..count).flat_map(|_| g.record_in(size, nasty)).collect(),
    }
}

/// An `AnimData.d2` with up to 3 records: `HTH` (the name an all-zero
/// monstats row composes) and random names.
fn random_animdata(g: &mut Gen) -> AnimData {
    use d2_formats::animdata::{hash, BUCKETS, RECORD_SIZE};
    let mut buckets: Vec<Vec<Vec<u8>>> = vec![Vec::new(); BUCKETS];
    for k in 0..g.below(4) {
        let mut name = [0u8; 8];
        if k == 0 {
            name[..3].copy_from_slice(b"HTH");
        } else {
            for b in name.iter_mut().take(g.below(8)) {
                *b = b'A' + g.below(26) as u8;
            }
        }
        let len = name.iter().position(|&b| b == 0).unwrap_or(8);
        let mut r = vec![0u8; RECORD_SIZE];
        r[..8].copy_from_slice(&name);
        r[8..12].copy_from_slice(&g.interesting_u32().to_le_bytes());
        r[12..16].copy_from_slice(&g.interesting_u32().to_le_bytes());
        for e in r[16..].iter_mut() {
            if g.one_in(40) {
                *e = g.next() as u8;
            }
        }
        buckets[hash(&name[..len])].push(r);
    }
    let mut bytes = Vec::new();
    for b in &buckets {
        bytes.extend((b.len() as u32).to_le_bytes());
        for r in b {
            bytes.extend(r);
        }
    }
    AnimData::parse(&bytes).expect("built AnimData")
}

/// Each fix-up and runtime-map builder on its own random tables, so the
/// later ones run even when an earlier one would refuse the set.
fn each_fixup(seed: u64, max: usize) {
    use d2_data::fixup::maps;
    use d2_data::fixup::records;
    bounded(move || {
        let mut g = Gen::new(seed);
        let nasty = g.one_in(3);
        let strings = StringTables::default();
        let t = |g: &mut Gen, n: &str| random_table(g, n, max, nasty);

        for (n, kind) in [
            ("itemtypes", maps::EquivKind::ItemTypes),
            ("montype", maps::EquivKind::MonType),
        ] {
            if let Ok(m) = maps::equiv_matrix(&t(&mut g, n), kind) {
                assert_eq!(m.bits.len(), m.n * m.words);
            }
        }
        let mut isc = t(&mut g, "itemstatcost");
        records::stat_ops(&mut isc);
        let _ = maps::desc_list(&isc);
        let _ = maps::states(&t(&mut g, "states"));
        let sl = maps::skill_lists(&t(&mut g, "skills"));
        for c in 0..sl.counts.len() {
            let _ = sl.lists.get(c * sl.max);
        }
        let items: Vec<BinTable> = ["weapons", "armor", "misc"]
            .iter()
            .map(|n| t(&mut g, n))
            .collect();
        let refs: Vec<&BinTable> = items.iter().collect();
        let _ = maps::version0_items(&refs);
        let codes = d2_data::bin::item_code_map(&items);
        let _ = maps::gamble(&mut t(&mut g, "gamble"), &codes, &refs);
        let _ = maps::monseq(&t(&mut g, "monseq"));
        let _ = maps::monpreset(&t(&mut g, "monpreset"));
        let _ = maps::hireling_first(&t(&mut g, "hireling"));
        let _ = maps::portals(&t(&mut g, "leveldefs"));
        let _ = maps::lvlsub_types(&t(&mut g, "lvlsub"));
        let _ = maps::automap(&t(&mut g, "automap"));

        let _ = records::charstats(&mut t(&mut g, "charstats"), &strings);
        let _ = records::attach_set_items(&mut t(&mut g, "setitems"), &mut t(&mut g, "sets"));
        let mut gem_items = items.clone();
        let _ = records::gems(&mut t(&mut g, "gems"), &mut gem_items, &strings);
        let mut monstats = t(&mut g, "monstats");
        let anim = random_animdata(&mut g);
        if records::monstats_chains(&mut monstats).is_ok() {
            let _ = records::monstats_speeds(
                &mut monstats,
                &t(&mut g, "monstats2"),
                &t(&mut g, "monmode"),
                &anim,
            );
        }
        records::link_monequip(&mut t(&mut g, "monequip"), &mut monstats, &codes);
        records::clamp_monumod(&mut t(&mut g, "monumod"));
        let _ = records::levels(&mut t(&mut g, "levels"), &strings);
        for n in ["lvltypes", "lvlprest", "lvlsub"] {
            let _ = records::tile_paths(&mut t(&mut g, n), g.one_in(2));
        }
        let _ = records::objects(&mut t(&mut g, "objects"), &strings);
    });
}

/// The patch base: every patchable table's text from [`txt_in`] (tame).
fn patch_base(g: &mut Gen) -> Option<d2_data::patch::PatchData> {
    use d2_data::patch::{rules, PatchData};
    let mut read = |file: &str| {
        let def = schema()
            .called()
            .find(|d| d.txt_name == file)
            .expect("called txt");
        let n = 1 + g.below(3);
        Ok(Some(("prop.mpq".to_owned(), txt_in(def, g, n, true))))
    };
    PatchData::from_base(&rules(), &mut read).ok()
}

/// A layer of statements drawn from `data`: real tables, keys, columns
/// and current values (so many statements apply), mixed with wrong ones.
fn random_layer(g: &mut Gen, data: &d2_data::patch::PatchData) -> Vec<u8> {
    use d2_data::patch::canonical_token;
    let mut out = b"d2patch 1\n".to_vec();
    // Careful layers: only `set` / `check` on index selectors, so most
    // stacks apply without an error and reach the diff round trip.
    let careful = g.one_in(2);
    let mut written = std::collections::BTreeSet::new();
    for _ in 0..g.below(12) {
        let t = g.pick(&data.tables);
        out.extend_from_slice(format!("table {}\n", t.name()).as_bytes());
        for _ in 0..g.below(6) {
            let n = t.rows.len();
            if n == 0 || t.header.is_empty() {
                break;
            }
            let r = if careful {
                g.below(n)
            } else {
                g.below(n + 1).min(n - 1)
            };
            let c = g.below(t.header.len());
            if careful && (c == t.key_col || !written.insert((t.name().to_owned(), r, c))) {
                continue;
            }
            let sel = if careful || g.one_in(3) {
                let i = if careful { r } else { r + g.below(2) };
                let mut sel = format!("#{i} ").into_bytes();
                sel.extend(canonical_token(t.key(r)));
                sel
            } else {
                canonical_token(t.key(r))
            };
            let col = t.canonical(c);
            let old = canonical_token(&t.rows[r].cells[c]);
            let mut new = canonical_token(g.pick(TAME_CELLS));
            if careful && new == old {
                new.push(b'x');
            }
            let mut line: Vec<u8> = Vec::new();
            match if careful { g.below(3) } else { g.below(5) } {
                0 | 1 => {
                    for part in [&b"set"[..], &sel, &col, &old, b"->", &new] {
                        line.extend_from_slice(part);
                        line.push(b' ');
                    }
                }
                2 => {
                    for part in [&b"check"[..], &sel, &col, &old] {
                        line.extend_from_slice(part);
                        line.push(b' ');
                    }
                }
                3 => {
                    let key = canonical_token(g.pick(TAME_CELLS));
                    let pin = if g.one_in(2) {
                        t.pin(r)
                    } else {
                        "sha:0".into()
                    };
                    line.extend_from_slice(format!("add #{} ", n + g.below(2)).as_bytes());
                    line.extend_from_slice(&key);
                    line.extend_from_slice(b" like ");
                    line.extend_from_slice(&sel);
                    line.push(b' ');
                    line.extend_from_slice(pin.as_bytes());
                }
                _ => line.extend_from_slice(g.pick(CELLS)),
            }
            while line.last() == Some(&b' ') {
                line.pop();
            }
            out.extend_from_slice(&line);
            out.push(b'\n');
        }
    }
    out
}

/// Base from text, 1–3 layers applied, the diff of the result, and the
/// patched compile against a random live set.
fn patch_pipeline(seed: u64) {
    use d2_data::patch::{apply_stack, compile_patched, diff_tables, parse_layer};
    bounded(move || {
        let mut g = Gen::new(seed);
        let Some(base) = patch_base(&mut g) else {
            return;
        };
        // Layer k is written against the state after layers 0..k.
        let mut layers = Vec::new();
        for k in 0..1 + g.below(3) {
            let mut current = base.clone();
            let _ = apply_stack(&mut current, &layers, "prop.d2stack");
            let text = random_layer(&mut g, &current);
            layers.push(parse_layer(&format!("l{k}.d2patch"), &text, k + 1).0);
        }
        let mut patched = base.clone();
        let applied = apply_stack(&mut patched, &layers, "prop.d2stack");
        let edits: Vec<(&d2_data::patch::PatchTable, Vec<u8>)> = base
            .tables
            .iter()
            .zip(&patched.tables)
            .filter(|(b, p)| b.rows != p.rows)
            .map(|(b, p)| (b, p.render()))
            .collect();
        let refs: Vec<(&d2_data::patch::PatchTable, &[u8])> =
            edits.iter().map(|(b, r)| (*b, r.as_slice())).collect();
        let diff = diff_tables(&refs, g.one_in(2));
        // The diff is a layer; applied to the base it gives the same cells
        // (round trip). Only for a stack that applied
        // cleanly: after an A error the state keeps that layer's passing
        // statements and can hold a key twice in a scope that spans
        // tables (`items.code`), which the per-table D06 does not reject.
        if let (Ok(diff), false) = (diff, d2_data::patch::has_errors(&applied)) {
            let (layer, f) = parse_layer("diff.d2patch", &diff, 1);
            assert!(f.is_empty(), "{f:?}");
            let mut again = base.clone();
            let f = apply_stack(&mut again, &[layer], "diff.d2stack");
            assert!(!d2_data::patch::has_errors(&f), "{f:?}");
            for (a, p) in again.tables.iter().zip(&patched.tables) {
                assert_eq!(a.render(), p.render(), "{}", a.name());
            }
        }
        if g.one_in(2) {
            let live = random_set(g.next(), 2, 0);
            let _ = compile_patched(&base, &patched, &live);
        }
    });
}

proptest! {
    #![proptest_config(config(24))]

    #[test]
    fn patch_layers_on_real_rules(seed in any::<u64>()) {
        patch_pipeline(seed);
    }
}

proptest! {
    #![proptest_config(config(48))]

    #[test]
    fn each_fixup_on_random_tables(seed in any::<u64>(), max in 0usize..10) {
        each_fixup(seed, max);
    }

    #[test]
    fn qsort_sorts_and_permutes(keys in prop::collection::vec(any::<u8>(), 0..300)) {
        use d2_data::fixup::qsort::qsort;
        let mut v: Vec<(u8, usize)> = keys.iter().copied().zip(0..).collect();
        qsort(&mut v, |a, b| a.0.cmp(&b.0));
        prop_assert!(v.windows(2).all(|w| w[0].0 <= w[1].0));
        let mut ids: Vec<usize> = v.iter().map(|e| e.1).collect();
        ids.sort_unstable();
        prop_assert_eq!(ids, (0..keys.len()).collect::<Vec<_>>());
    }

    #[test]
    fn qsort_inconsistent_comparator(n in 0usize..300, seed in any::<u64>()) {
        // A comparator that is not an order (callers never pass one) still
        // returns, with the elements permuted.
        use d2_data::fixup::qsort::qsort;
        let mut v: Vec<usize> = (0..n).collect();
        let mut g = Gen::new(seed);
        bounded(move || {
            qsort(&mut v, |_, _| *g.pick(&[
                std::cmp::Ordering::Less,
                std::cmp::Ordering::Equal,
                std::cmp::Ordering::Greater,
            ]));
            v.sort_unstable();
            assert_eq!(v, (0..n).collect::<Vec<_>>());
        });
    }

    #[test]
    fn compile_whole_set(seed in any::<u64>(), rows in 1usize..5) {
        compile_set(seed, rows);
    }

    #[test]
    fn fixups_on_random_tables(seed in any::<u64>(), max in 0usize..8) {
        bounded(move || {
            let mut set = random_set(seed, max, 0);
            // Tables that refuse random rows are emptied in 2 of 3 sets, so
            // the fix-ups after them run.
            if !Gen::new(!seed).one_in(3) {
                clear_refusing(&mut set);
            }
            fixups(set)
        });
    }

    #[test]
    fn fixups_with_tables_missing(seed in any::<u64>(), max in 0usize..5, omit in 2usize..12) {
        bounded(move || fixups(random_set(seed, max, omit)));
    }
}

/// The bases the properties start from are accepted: an all-empty
/// compile and fix-up of an empty set run to the end.
#[test]
fn empty_inputs_pass() {
    let set = random_set(0, 0, 0);
    assert_eq!(set.tables.len(), schema().runtime().count());
    let fixed = fixup::apply(&set, &empty_animdata());
    assert!(fixed.is_ok(), "{:?}", fixed.err());
    let def = schema().table("weapons").unwrap();
    let mut g = Gen::new(1);
    let text = txt_for(def, &mut g, 1);
    let r = TxtTable::parse("weapons.txt", &text);
    assert!(r.is_ok(), "{:?}", r.err());
}

/// A table of `name` whose records are zeros with `writes` applied.
fn table_of(name: &str, records: &[&[(usize, &[u8])]]) -> BinTable {
    let size = schema().table(name).expect("schema table").record_size;
    let mut bytes = Vec::new();
    for writes in records {
        let mut r = vec![0u8; size];
        for (o, b) in *writes {
            r[*o..*o + b.len()].copy_from_slice(b);
        }
        bytes.extend(r);
    }
    BinTable {
        name: name.into(),
        source: "prop".into(),
        count: records.len(),
        record_size: size,
        records: bytes,
    }
}

#[test]
fn regress_equiv_walk_cycle() {
    // itemtypes row 1 `equiv1` = 1: the walk pops 1, pushes 1, forever
    // (the stack never passes 124). Hung `fixup::apply`; now a load error.
    // montype rows 1 → 2 → 1 is the two-row form (row 3 outside the
    // cycle is the column it never finds).
    use d2_data::fixup::maps::{equiv_matrix, EquivKind};
    let t = table_of("itemtypes", &[&[], &[(4, &1i16.to_le_bytes())], &[]]);
    assert!(bounded(
        move || equiv_matrix(&t, EquivKind::ItemTypes).is_err()
    ));
    let t = table_of(
        "montype",
        &[
            &[],
            &[(2, &2i16.to_le_bytes())],
            &[(2, &1i16.to_le_bytes())],
            &[],
        ],
    );
    assert!(bounded(
        move || equiv_matrix(&t, EquivKind::MonType).is_err()
    ));
}
