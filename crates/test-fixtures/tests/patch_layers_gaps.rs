//! Patch layers over the synthetic install (no game files): the patched
//! compile against the synthetic live set. Everything is generated at test
//! time into `CARGO_TARGET_TMPDIR`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use d2_data::bin::{read_excel, u32_at};
use d2_data::patch::{
    apply_stack, compile_patched, has_errors, parse_layer, rules, PatchData, PatchedCompile,
};
use test_fixtures::install::{self, Install};
use test_fixtures::synth;

fn dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()))
}

fn built() -> &'static Install {
    static I: OnceLock<Install> = OnceLock::new();
    I.get_or_init(|| {
        install::build(&dir("patch-gaps"), &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"))
    })
}

/// The patchable tables read from the install's `.txt` files.
fn base() -> PatchData {
    let i = built();
    let mut read = |f: &str| read_excel(&i.archives, f).map_err(|e| e.to_string());
    PatchData::from_base(&rules(), &mut read).unwrap_or_else(|f| panic!("{f:?}"))
}

/// Applies one layer of `body` (after `d2patch 1`) and compiles.
fn patch(base: &PatchData, body: &str) -> (PatchData, PatchedCompile) {
    let text = format!("d2patch 1\n{body}\n");
    let (layer, f) = parse_layer("t.d2patch", text.as_bytes(), 1);
    assert!(f.is_empty(), "{f:?}");
    let mut data = base.clone();
    let f = apply_stack(&mut data, &[layer], "t.d2stack");
    assert!(!has_errors(&f), "{f:?}");
    let r = compile_patched(base, &data, &built().loaded);
    (data, r)
}

/// Every file under `dir`, with its bytes, sorted by path.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_owned()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                todo.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// Appending a `weapons` row shifts the combined item index of every
/// `armor` and `misc` row by one; a link into `items.code` is a key in the
/// cells and resolves to the new index (`gems` `code` and `runes` `rune1`
/// name the misc item `pt1`), while the armor and misc records themselves
/// are unchanged.
#[test]
fn appending_weapons_shifts_armor_and_misc_indices() {
    let base = base();
    let w = base.table("weapons").unwrap();
    let n = w.rows.len();
    let body = format!(
        "table weapons\nadd #{n} sb9 like sb1 {}\nset #{n} sb9 namestr sb1 -> lb1",
        w.pin(0)
    );
    let (data, r) = patch(&base, &body);
    assert!(!has_errors(&r.findings), "{:?}", r.findings);
    let new = r.compiled.as_ref().unwrap();
    let old = &built().compiled;
    let count = |c: &d2_data::compile_set::CompiledSet, t: &str| c.table(t).unwrap().compiled.count;
    assert_eq!(count(new, "weapons"), count(old, "weapons") + 1);
    // pt1 is misc row 0: combined index weapons + armor + 0.
    let misc = data.table("misc").unwrap();
    assert_eq!(misc.key(0), b"pt1");
    let pt1_old = (count(old, "weapons") + count(old, "armor")) as u32;
    for (table, offset) in [("gems", 40), ("runes", 152)] {
        let rec = |c: &d2_data::compile_set::CompiledSet| {
            u32_at(c.table(table).unwrap().compiled.record(0), offset)
        };
        assert_eq!(rec(old), pt1_old, "{table} base");
        assert_eq!(rec(new), pt1_old + 1, "{table} patched");
        // The cell text is untouched: the link is resolved by key.
        assert_eq!(
            data.table(table).unwrap().rows[0].cells,
            base.table(table).unwrap().rows[0].cells
        );
    }
    for t in ["armor", "misc"] {
        assert_eq!(
            new.table(t).unwrap().compiled.records,
            old.table(t).unwrap().compiled.records,
            "{t}"
        );
    }
    // The live set built from the patched compile carries the same shift.
    let live = r.live.as_ref().unwrap();
    assert_eq!(
        u32_at(live.table("gems").unwrap().record(0), 40),
        pt1_old + 1
    );
}

/// Layers patch `.txt` cells only, and the mod set compiles every table
/// from text: after a one-cell layer on `misc`, every runtime table of the
/// patched live set (touched or not) holds the text compile's records and
/// names the `.txt` archive as its source, never the live `.bin` file; the
/// loaded `.bin` set and the install's files are left as they were.
#[test]
fn layers_patch_text_and_every_table_compiles_from_text() {
    let i = built();
    let before = snapshot(&i.dir);
    let loaded = i.loaded.clone();
    let base = base();
    let (data, r) = patch(&base, "table misc\nset pt1 cost 5 -> 6");
    assert!(!has_errors(&r.findings), "{:?}", r.findings);
    // Only the one cell changed, in the text tables.
    let changed: Vec<(&str, usize)> = data
        .tables
        .iter()
        .zip(&base.tables)
        .flat_map(|(p, b)| {
            p.rows
                .iter()
                .zip(&b.rows)
                .enumerate()
                .filter(|(_, (pr, br))| pr.cells != br.cells)
                .map(move |(i, _)| (p.name(), i))
        })
        .collect();
    assert_eq!(changed, [("misc", 0)]);

    let compiled = r.compiled.as_ref().unwrap();
    let live = r.live.as_ref().unwrap();
    assert_eq!(live.tables.len(), loaded.tables.len());
    for t in &live.tables {
        let ct = compiled.table(&t.name).unwrap();
        assert_eq!(t.records, ct.compiled.records, "{}", t.name);
        assert_eq!(t.source, ct.txt_source, "{}", t.name);
        assert_eq!(t.source, "d2data.mpq", "{}", t.name);
        let shipped = loaded.table(&t.name).unwrap();
        assert_eq!(shipped.source, "patch_d2.mpq", "{}", t.name);
        if t.name != "misc" {
            assert_eq!(t.records, shipped.records, "{}", t.name);
        }
    }
    assert_ne!(
        live.table("misc").unwrap().records,
        loaded.table("misc").unwrap().records
    );
    // The loaded `.bin` set and the files on disk are not modified.
    assert_eq!(i.loaded.tables, loaded.tables);
    assert_eq!(snapshot(&i.dir), before);
}
