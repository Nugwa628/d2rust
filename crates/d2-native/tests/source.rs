//! The native source against the archives, on the synthetic install (no
//! game files): every asset decodes to the same value from both sides and
//! the table loader builds the same tables; a later mod wins over an
//! earlier one; a broken mod or a bad manifest refuses the whole root.

use std::path::{Path, PathBuf};

use d2_data::bin;
use d2_formats::mpq::ArchiveSet;
use d2_native::kind::{grey_palette, NativeKind};
use d2_native::manifest::{write_files_tsv, Manifest};
use d2_native::source::{
    decode_original, fold, sha256_hex, NativeAsset, NativeSource, SourceError, KNOWN_KINDS,
};
use test_fixtures::install;
use test_fixtures::sprites::{dc6_file, dc6_frames, Dc6Shape};
use test_fixtures::synth::synthetic;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("d2native-source-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn dc6(seed: u32) -> Vec<u8> {
    let shape = Dc6Shape {
        directions: 1,
        frames: 2,
        width: 5,
        height: 4,
        panel: false,
    };
    dc6_file(&dc6_frames(shape, seed), 1, 2)
}

fn pal(seed: u8) -> Vec<u8> {
    (0..768u32).map(|i| (i as u8).wrapping_mul(seed)).collect()
}

/// Writes the native files of decoded asset `a` (canonical path `p`)
/// under `dir` (a layer folder), as the converter does.
fn write_native(dir: &Path, p: &str, a: &NativeAsset) {
    let view = grey_palette();
    let files = match a {
        NativeAsset::Dc6(x) => x.write(p, &view).unwrap(),
        NativeAsset::Dt1(x) => x.write(p, &view).unwrap(),
        NativeAsset::Pal(x) => x.write(p, &view).unwrap(),
        NativeAsset::Ds1(x) => {
            put(
                dir,
                &format!("{p}.toml"),
                d2_native::toml_kinds::write_ds1(p, x).unwrap().as_bytes(),
            );
            return;
        }
        NativeAsset::Tbl(x) => {
            let t = d2_native::tbl::write_tbl(p, x, true).unwrap();
            put(dir, &format!("{p}.tsv"), t.tsv.as_bytes());
            put(dir, &format!("{p}.toml"), t.toml.as_bytes());
            return;
        }
        NativeAsset::Excel(b) => {
            put(dir, p, b);
            return;
        }
        other => panic!("{p}: no writer in this test for {other:?}"),
    };
    for f in files {
        put(dir, &f.path, &f.bytes);
    }
}

fn manifest(root: &Path, format_version: Option<u32>) {
    let tsv = write_files_tsv(&[]);
    put(root, "files.tsv", tsv.as_bytes());
    let m = Manifest {
        converter_version: "0.1.0".into(),
        converter_commit: "test".into(),
        kinds: KNOWN_KINDS.iter().map(|&(k, v)| (k.into(), v)).collect(),
        language: "eng".into(),
        lod: true,
        archives: Vec::new(),
        counts: [("excel".to_string(), Default::default())].into(),
        unnamed_blocks: Default::default(),
        complete: true,
        files_sha256: sha256_hex(tsv.as_bytes()),
    };
    let mut text = m.to_toml();
    if let Some(v) = format_version {
        text = text.replace("format_version = 1", &format!("format_version = {v}"));
    }
    put(root, "manifest.toml", text.as_bytes());
}

const SHEET: &str = "data/global/ui/panel/sourcetest.dc6";
const PAL: &str = "data/global/palette/act1/pal.dat";

/// The synthetic install with a sprite sheet and a palette added, and its
/// conversion: the archives, the native root and every converted path.
struct Pair {
    archives: ArchiveSet,
    native: PathBuf,
    names: Vec<String>,
}

fn pair(name: &str) -> Pair {
    let dir = scratch(name);
    let mut data = synthetic();
    data.files.push((SHEET.replace('/', "\\"), dc6(7)));
    data.files.push((PAL.replace('/', "\\"), pal(3)));
    let inst = install::build(&dir.join("install"), &data).unwrap();
    let native = dir.join("native");
    let base = native.join("base");

    let mut names: Vec<String> = data.files.iter().map(|(n, _)| fold(n)).collect();
    for f in ["string.tbl", "patchstring.tbl", "expansionstring.tbl"] {
        names.push(format!("data/local/lng/eng/{f}"));
    }
    for (txt, _) in data.tables.render() {
        names.push(format!("data/global/excel/{}", txt.to_ascii_lowercase()));
    }
    names.push("data/global/excel/soundenviron.txt".into());
    names.sort();
    names.dedup();
    for p in &names {
        let bytes = inst.archives.read(&p.replace('/', "\\")).unwrap();
        let a = decode_original(p, &bytes).expect("converted kind").unwrap();
        write_native(&base, p, &a);
    }
    manifest(&native, None);
    Pair {
        archives: inst.archives,
        native,
        names,
    }
}

#[test]
fn native_equals_mpq_assets_and_tables() {
    let p = pair("same");
    let src = NativeSource::open(&p.native).unwrap();
    let mut kinds = std::collections::HashSet::new();
    for name in &p.names {
        let bytes = p.archives.read(&name.replace('/', "\\")).unwrap();
        let mpq = decode_original(name, &bytes).unwrap().unwrap();
        let native = src.read_native(name).expect("native file").unwrap();
        kinds.insert(std::mem::discriminant(&mpq));
        if let (NativeAsset::Tbl(_), NativeAsset::Tbl(_)) = (&mpq, &native) {
            continue; // compared through the loaded string tables below
        }
        assert_eq!(mpq, native, "{name}");
    }
    assert!(kinds.len() >= 5, "kinds covered: {}", kinds.len());

    // Tables: the same loader, both sources.
    let a = bin::load(&p.archives, "eng").unwrap();
    let b = bin::load_from(&src.tables("eng").unwrap(), "eng").unwrap();
    assert_eq!(a.lod, b.lod);
    assert_eq!(a.tables.len(), b.tables.len());
    for (x, y) in a.tables.iter().zip(&b.tables) {
        assert_eq!((&x.name, x.records.len()), (&y.name, y.records.len()));
        assert_eq!(x.records, y.records, "table {}", x.name);
    }
    assert_eq!(a.hitclass.records, b.hitclass.records);
    for (k, c) in &a.code {
        assert_eq!(c.bytes, b.code[k].bytes, "{k:?}");
    }
    assert_eq!(a.sounds, b.sounds);
    assert_eq!(a.soundenviron, b.soundenviron);
    for base in [0u32, 10_000, 20_000] {
        for id in base..base + 64 {
            assert_eq!(
                a.strings.by_index(id),
                b.strings.by_index(id),
                "string {id}"
            );
        }
    }
}

#[test]
fn later_mod_wins_and_bad_mods_are_refused() {
    let p = pair("mods");
    let root = &p.native;
    let ok_mod = |name: &str, requires: &str| {
        put(
            root,
            &format!("mods/{name}/mod.toml"),
            format!(
                "name = \"{name}\"\nversion = \"1\"\nnative_format_version = 1\nrequires = [{requires}]\n"
            )
            .as_bytes(),
        );
    };
    let sheet = |seed| match decode_original(SHEET, &dc6(seed)).unwrap().unwrap() {
        NativeAsset::Dc6(d) => d,
        _ => unreachable!(),
    };
    ok_mod("a", "");
    ok_mod("b", "\"a\"");
    write_native(
        &root.join("mods/a/files"),
        SHEET,
        &NativeAsset::Dc6(sheet(11)),
    );
    write_native(
        &root.join("mods/b/files"),
        SHEET,
        &NativeAsset::Dc6(sheet(12)),
    );
    let read = || NativeSource::open(root).map(|s| s.read_native(SHEET).unwrap().unwrap());

    // No order file: base.
    assert_eq!(read().unwrap(), NativeAsset::Dc6(sheet(7)));
    put(root, "mods/order.toml", b"order = [\"a\", \"b\"]\n");
    assert_eq!(read().unwrap(), NativeAsset::Dc6(sheet(12)));
    put(root, "mods/order.toml", b"order = [\"a\"]\n");
    assert_eq!(read().unwrap(), NativeAsset::Dc6(sheet(11)));
    // A caller's own layer list wins over the order file: base alone, or
    // its own mods.
    let read_with = |order: &[String]| {
        NativeSource::open_with(root, Some(order)).map(|s| s.read_native(SHEET).unwrap().unwrap())
    };
    assert_eq!(read_with(&[]).unwrap(), NativeAsset::Dc6(sheet(7)));
    let ab = ["a".to_string(), "b".to_string()];
    assert_eq!(read_with(&ab).unwrap(), NativeAsset::Dc6(sheet(12)));
    // `b` requires `a` earlier in the order.
    put(root, "mods/order.toml", b"order = [\"b\", \"a\"]\n");
    let e = read().unwrap_err().to_string();
    assert!(e.contains("requires `a`"), "{e}");

    // A sheet without its sidecar is refused, naming it.
    put(root, "mods/order.toml", b"order = [\"a\"]\n");
    std::fs::remove_file(root.join(format!("mods/a/files/{SHEET}.toml"))).unwrap();
    let e = read().unwrap_err().to_string();
    assert!(e.contains(&format!("{SHEET}.toml")), "{e}");
    write_native(
        &root.join("mods/a/files"),
        SHEET,
        &NativeAsset::Dc6(sheet(11)),
    );

    // A mod `.txt` is refused.
    put(root, "mods/a/files/data/global/excel/weapons.txt", b"x");
    let e = read().unwrap_err().to_string();
    assert!(e.contains("data/global/excel/weapons.txt"), "{e}");
}

#[test]
fn bad_manifest_is_refused() {
    let p = pair("manifest");
    manifest(&p.native, Some(2));
    let e = NativeSource::open(&p.native).unwrap_err();
    assert!(matches!(e, SourceError::Root { .. }));
    let msg = e.to_string();
    assert!(msg.contains("format_version 2"), "{msg}");
    assert!(msg.contains("d2-convert"), "{msg}");

    // An edited files.tsv no longer matches the manifest's hash.
    manifest(&p.native, None);
    put(&p.native, "files.tsv", b"tampered\n");
    let msg = NativeSource::open(&p.native).unwrap_err().to_string();
    assert!(msg.contains("files_sha256"), "{msg}");

    // No manifest at all.
    std::fs::remove_file(p.native.join("manifest.toml")).unwrap();
    let msg = NativeSource::open(&p.native).unwrap_err().to_string();
    assert!(msg.contains("manifest.toml"), "{msg}");
}

/// A mod's string edits reach the loaded string tables: an added key reads
/// at the first mod id, every table string reads as before, and a refused
/// edit refuses the root.
#[test]
fn mod_string_edits_load() {
    let p = pair("strings");
    let root = &p.native;
    put(
        root,
        "mods/s/mod.toml",
        b"name = \"s\"\nversion = \"1\"\nnative_format_version = 1\nrequires = []\n",
    );
    put(root, "mods/order.toml", b"order = [\"s\"]\n");
    put(
        root,
        "mods/s/files/patches/a.d2patch",
        b"d2patch 1\nstradd zz_mod_key [Mod text]\n",
    );
    let base = bin::load(&p.archives, "eng").unwrap();
    let src = NativeSource::open(root).unwrap();
    let t = bin::load_from(&src.tables("eng").unwrap(), "eng").unwrap();
    let first = d2_data::strings::MOD_BASE;
    assert_eq!(t.strings.id(b"zz_mod_key"), first);
    assert_eq!(
        t.strings.by_index(first).unwrap(),
        "Mod text".encode_utf16().collect::<Vec<_>>()
    );
    for base_id in [0u32, 10_000, 20_000] {
        for id in base_id..base_id + 64 {
            assert_eq!(t.strings.by_index(id), base.strings.by_index(id), "{id}");
        }
    }

    put(
        root,
        "mods/s/files/patches/b.d2patch",
        b"d2patch 1\nstrset zz_no_such_key x\n",
    );
    let src = NativeSource::open(root).unwrap();
    let e = src.tables("eng").unwrap_err().to_string();
    assert!(e.contains("A19"), "{e}");
}
