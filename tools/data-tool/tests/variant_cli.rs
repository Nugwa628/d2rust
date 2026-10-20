//! `data-tool variant` against a synthetic install (no game files): the
//! base install is generated into `CARGO_TARGET_TMPDIR`; variant installs
//! go to the system temp dir, since the tool refuses any output inside
//! the repository work tree (`target/` included).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use d2_data::bin::{self, excel_path, read_excel};
use d2_formats::mpq::{Archive, ArchiveSet};
use test_fixtures::install::{self, Install};
use test_fixtures::synth;

fn built() -> &'static Install {
    static I: OnceLock<Install> = OnceLock::new();
    I.get_or_init(|| {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("variant-cli-game-{}", std::process::id()));
        install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"))
    })
}

fn out_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("variant-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// `<root>/<name>/<name>.d2stack` with the given layers.
fn stack(name: &str, layers: &[(&str, &str)]) -> PathBuf {
    let root = out_dir(&format!("stack-{name}"));
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = String::from("d2stack 1\n");
    for (file, text) in layers {
        s.push_str(&format!("layer {file}\n"));
        std::fs::write(dir.join(file), text).unwrap();
    }
    let path = dir.join(format!("{name}.d2stack"));
    std::fs::write(&path, s).unwrap();
    path
}

fn run(args: &[&str]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_data-tool"))
        .arg("variant")
        .args(args)
        .env_remove("D2_GAME_DIR")
        .output()
        .expect("data-tool runs");
    (
        o.status.code().expect("exit code"),
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
    )
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

const MONSTATS: &str = "d2patch 1\ntable monstats\nset beast1 MaxGrp 3 -> 4\n";

#[test]
fn build_one_set_on_monstats_then_check() {
    let game = &built().dir;
    let st = stack("one-set", &[("a.d2patch", MONSTATS)]);
    let out = out_dir("one-set");
    let (code, text) = run(&["build", s(&st), "--game", s(game), "--out", s(&out)]);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("patched monstats.bin (live from patch_d2)"),
        "{text}"
    );
    assert!(text.contains("check ok: 1 table(s)"), "{text}");

    // The variant loads; monstats.bin differs from the base, everything
    // else of the excel set is the base's.
    let base = ArchiveSet::open_dir(game).unwrap();
    let var = ArchiveSet::open_dir(&out).unwrap();
    let loaded = bin::load(&var, bin::DEFAULT_LANGUAGE).unwrap();
    let (src, mon) = read_excel(&var, "monstats.bin").unwrap().unwrap();
    assert_eq!(src, "patch_d2.mpq");
    assert_ne!(mon, read_excel(&base, "monstats.bin").unwrap().unwrap().1);
    for t in &built().loaded.tables {
        if t.name != "monstats" {
            assert_eq!(
                loaded.table(&t.name).unwrap().records,
                t.records,
                "{}",
                t.name
            );
        }
    }
    // Other top-level files are present (hard links or copies).
    for e in std::fs::read_dir(game).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() {
            assert!(
                out.join(p.file_name().unwrap()).is_file(),
                "{}",
                p.display()
            );
        }
    }
    let json = std::fs::read_to_string(out.join("variant.json")).unwrap();
    assert!(json.contains("\"format\": \"test-variant\""), "{json}");
    assert!(json.contains("\"table\": \"monstats\""), "{json}");

    // `check` passes, and fails (exit 1) once the variant archive changes.
    let (code, text) = run(&["check", s(&out), "--game", s(game)]);
    assert_eq!(code, 0, "{text}");
    let p = out.join("patch_d2.mpq");
    let mut bytes = std::fs::read(&p).unwrap();
    let a = Archive::open(&p).unwrap();
    let i = a.find(&excel_path("monstats.bin")).unwrap();
    let pos = a.block_table()[i].file_pos as usize;
    bytes[pos + 7] ^= 0x10;
    std::fs::write(&p, bytes).unwrap();
    let (code, text) = run(&["check", s(&out), "--game", s(game)]);
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("data\\global\\excel\\monstats.bin: read back differs"),
        "{text}"
    );
    // Rebuilding over an earlier variant works.
    let (code, text) = run(&["build", s(&st), "--game", s(game), "--out", s(&out)]);
    assert_eq!(code, 0, "{text}");
}

#[test]
fn empty_stack_builds_the_base() {
    let game = &built().dir;
    let st = stack("empty", &[]);
    let out = out_dir("empty");
    let (code, text) = run(&["build", s(&st), "--game", s(game), "--out", s(&out)]);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("note: the stack changes no compiled .bin"),
        "{text}"
    );
    assert_eq!(
        std::fs::read(out.join("patch_d2.mpq")).unwrap(),
        std::fs::read(game.join("patch_d2.mpq")).unwrap()
    );
}

/// Usage and refusals exit 2; patch errors exit 1.
#[test]
fn refusals_and_errors() {
    let game = &built().dir;
    let st = stack("one-set", &[("a.d2patch", MONSTATS)]);
    // Inside the repository work tree. Not CARGO_TARGET_TMPDIR: CI sets
    // CARGO_TARGET_DIR outside the checkout (warm runner volumes).
    let inside = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("variant-inside");
    let (code, text) = run(&["build", s(&st), "--game", s(game), "--out", s(&inside)]);
    assert_eq!(code, 2, "{text}");
    assert!(text.contains("inside the repository work tree"), "{text}");
    assert!(!inside.exists());
    // A bad name / layout.
    let bad = stack("Bad_Name", &[]);
    let (code, _) = run(&["build", s(&bad), "--game", s(game)]);
    assert_eq!(code, 2);
    let (code, _) = run(&["frob"]);
    assert_eq!(code, 2);
    // A06: the old value is wrong.
    let wrong = stack(
        "wrong",
        &[(
            "a.d2patch",
            "d2patch 1\ntable monstats\nset beast1 MaxGrp 9 -> 4\n",
        )],
    );
    let out = out_dir("wrong");
    let (code, text) = run(&["build", s(&wrong), "--game", s(game), "--out", s(&out)]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("error[A06]"), "{text}");
    assert!(!out.exists());
}
