//! The synthetic equivalent of the real 73/73: a synthetic install whose
//! `monstats` has record 707 and whose live `monstats.bin` holds a
//! different `NameStr` there. Convert compiles the native excel set,
//! writes `_bin-overrides.toml`, and `verify --deep` reports every table.

use std::fs;
use std::path::{Path, PathBuf};

use d2_convert::fsutil::join;
use d2_convert::{convert, kinds, verify_full, Options, VerifyOptions};
use d2_data::schema::schema;
use d2_formats::mpq::writer::{FileOptions, Method, MpqWriter, Pkware};
use d2_native::tables::OVERRIDES_PATH;
use test_fixtures::install::{bin_files, compile_and_pack, string_path, write_text_install};
use test_fixtures::synth::synthetic;

fn scratch(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

const LIVE_NAME_STR: u16 = 5382;

/// A full synthetic install (text + live `.bin`) with 708 monstats
/// records; the live record 707 `NameStr` is `live_name_str` when given,
/// and `flip` flips one byte of another monstats cell.
fn install(dir: &Path, live_name_str: Option<u16>, flip: bool) {
    let mut data = synthetic();
    let f = data.tables.files.get_mut("monstats.txt").unwrap();
    let id = f.columns.iter().position(|c| c == "Id").unwrap();
    let mut row = f.rows[0].clone();
    while f.rows.len() < 708 {
        row[id] = format!("pad{}", f.rows.len());
        f.rows.push(row.clone());
    }
    write_text_install(dir, &data).unwrap();
    let compiled = compile_and_pack(dir, &data).unwrap();
    let def = schema().table("monstats").unwrap();
    let off = def.field("NameStr").unwrap().footprint().start;
    // Rewrite patch_d2.mpq with the doctored live monstats.bin.
    let strings: Vec<(&[u8], &[u8])> = data
        .strings
        .patch
        .iter()
        .map(|(k, v)| (k.as_bytes(), v.as_bytes()))
        .collect();
    let mut p = MpqWriter::new().with_listfile();
    p.add(
        &string_path("patchstring.tbl"),
        test_fixtures::tbl::write(&strings),
        FileOptions::default(),
    );
    p.add(
        r"data\global\excel\soundenviron.txt",
        data.soundenviron.clone(),
        FileOptions {
            method: Method::Implode(Pkware::default()),
            ..FileOptions::default()
        },
    );
    for (name, mut bytes) in bin_files(&compiled) {
        if name == "monstats.bin" {
            let rec = 4 + 707 * def.record_size;
            if let Some(v) = live_name_str {
                bytes[rec + off..rec + off + 2].copy_from_slice(&v.to_le_bytes());
            }
            if flip {
                bytes[4 + 3 * def.record_size + 9] ^= 1;
            }
        }
        p.add(
            &format!(r"data\global\excel\{name}"),
            bytes,
            FileOptions::default(),
        );
    }
    p.write(dir.join("patch_d2.mpq")).unwrap();
    for name in d2_convert::convert::REQUIRED_ARCHIVES {
        let path = dir.join(name);
        if !path.exists() {
            MpqWriter::new().write(path).unwrap();
        }
    }
}

fn run(dir: &Path) -> (d2_convert::RunSummary, PathBuf) {
    let out = dir.join("native");
    let mut o = Options::new(dir.join("game"), &out);
    o.converter_commit = "test".into();
    (convert(&o, &kinds::builtin()).unwrap(), out)
}

#[test]
fn convert_checks_every_table_and_writes_the_707_override() {
    let dir = scratch("ctable");
    install(&dir.join("game"), Some(LIVE_NAME_STR), false);
    let (s, out) = run(&dir);
    assert_eq!(s.failures, Vec::new());
    let t = s.tables.as_ref().expect("step 2 ran");
    let total = schema().runtime().count();
    assert_eq!(
        (t.identical.len(), t.checked(), t.overrides),
        (total, total, 1)
    );
    assert!(s.manifest.as_ref().unwrap().complete);

    // The overrides file is a native file with its own row (no source).
    let text = fs::read_to_string(join(&out.join("base"), OVERRIDES_PATH)).unwrap();
    assert!(
        text.contains("record = 707") && text.contains("0x0615"),
        "{text}"
    );
    let tsv = fs::read_to_string(out.join("files.tsv")).unwrap();
    let row = tsv.lines().find(|l| l.starts_with(OVERRIDES_PATH)).unwrap();
    let cols: Vec<&str> = row.split('\t').collect();
    assert_eq!((cols[1], cols[2], cols[3]), ("excel", "-", ""));
    assert!(cols[5] == "ok" && cols[4].contains(OVERRIDES_PATH), "{row}");
    let report = fs::read_to_string(out.join("report.txt")).unwrap();
    assert!(
        report.contains(&format!("C-TABLE step 2: {total}/{total} tables identical")),
        "{report}"
    );
    assert!(report.contains("monstats: identical"), "{report}");

    // verify --deep reports every table too.
    let vo = VerifyOptions {
        out: out.clone(),
        deep_install: Some(dir.join("game")),
        progress: false,
    };
    let v = verify_full(&vo, &kinds::builtin());
    assert_eq!(v.problems, Vec::<String>::new());
    assert_eq!(v.tables.unwrap().identical.len(), total);

    // Resume: a rerun rederives the row and leaves files.tsv identical.
    let (s2, _) = run(&dir);
    assert_eq!(s2.failures, Vec::new());
    assert_eq!(fs::read_to_string(out.join("files.tsv")).unwrap(), tsv);

    // Tampering with the overrides file is caught by name.
    let p = join(&out.join("base"), OVERRIDES_PATH);
    let orig = fs::read_to_string(&p).unwrap();
    fs::write(&p, orig.replace("0x0615", "0x0616")).unwrap();
    let v = verify_full(&vo, &kinds::builtin());
    assert!(
        v.problems.iter().any(|x| x.contains(OVERRIDES_PATH)),
        "{:?}",
        v.problems
    );
    fs::write(&p, &orig).unwrap();
    assert!(verify_full(&vo, &kinds::builtin()).problems.is_empty());
    // Deleting the native text of a table is caught by the table check.
    let weapons = join(&out.join("base"), "data/global/excel/weapons.txt");
    let w = fs::read(&weapons).unwrap();
    fs::write(&weapons, &w[..w.len() - 3]).unwrap();
    let v = verify_full(&vo, &kinds::builtin());
    assert!(!v.problems.is_empty());
}

#[test]
fn an_install_without_the_difference_needs_no_override() {
    let dir = scratch("ctable-none");
    install(&dir.join("game"), None, false);
    let (s, out) = run(&dir);
    assert_eq!(s.failures, Vec::new());
    assert_eq!(s.tables.unwrap().overrides, 0);
    let text = fs::read_to_string(join(&out.join("base"), OVERRIDES_PATH)).unwrap();
    assert!(!text.contains("[[override]]"), "{text}");
}

#[test]
fn any_other_difference_fails_c_table() {
    let dir = scratch("ctable-bad");
    install(&dir.join("game"), Some(LIVE_NAME_STR), true);
    let (s, out) = run(&dir);
    assert_eq!(s.exit_code(), 1);
    assert!(
        s.failures
            .iter()
            .any(|(_, c, d)| c == "C-TABLE" && d.contains("monstats") && d.contains("record 3")),
        "{:?}",
        s.failures
    );
    let t = s.tables.unwrap();
    assert_eq!(t.failed.len(), 1);
    let m = s.manifest.unwrap();
    assert!(!m.complete);
    let tsv = fs::read_to_string(out.join("files.tsv")).unwrap();
    assert!(tsv.contains("failed:C-TABLE"), "{tsv}");
}
