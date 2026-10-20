//! `data-tool patch` against a synthetic install (no game files): the
//! binary is run with `D2_GAME_DIR` pointing at archives generated at test
//! time into `CARGO_TARGET_TMPDIR`. Covers the patch commands and the
//! features the patch language deliberately leaves out.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

use d2_data::bin::read_excel;
use d2_data::patch::{
    apply_stack, compile_patched, diff_tables, has_errors, parse_layer, rules, Code, PatchData,
};
use test_fixtures::install::{self, Install};
use test_fixtures::synth;

fn tmp(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()))
}

fn built() -> &'static Install {
    static I: OnceLock<Install> = OnceLock::new();
    I.get_or_init(|| {
        install::build(&tmp("patch-cli-game"), &synth::synthetic())
            .unwrap_or_else(|e| panic!("{e}"))
    })
}

fn base() -> PatchData {
    let i = built();
    let mut read = |f: &str| read_excel(&i.archives, f).map_err(|e| e.to_string());
    PatchData::from_base(&rules(), &mut read).unwrap_or_else(|f| panic!("{f:?}"))
}

/// Applies layer texts (whole files) to the base.
fn applied(texts: &[&str]) -> PatchData {
    let layers: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(k, t)| parse_layer(&format!("l{k}.d2patch"), t.as_bytes(), k + 1).0)
        .collect();
    let mut data = base();
    let f = apply_stack(&mut data, &layers, "s.d2stack");
    assert!(!has_errors(&f), "{f:?}");
    data
}

/// Runs `data-tool patch <args>` with `D2_GAME_DIR` = `game` (or unset).
fn run(game: Option<&Path>, args: &[&str]) -> (i32, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_data-tool"));
    cmd.arg("patch").args(args).env_remove("D2_GAME_DIR");
    if let Some(g) = game {
        cmd.env("D2_GAME_DIR", g);
    }
    let Output { status, stdout, .. } = cmd.output().expect("data-tool runs");
    (
        status.code().expect("exit code"),
        String::from_utf8_lossy(&stdout).into_owned(),
    )
}

/// A stack directory holding `<stem>.d2stack` and its layer files.
fn stack(tag: &str, stem: &str, layers: &[(&str, &str)]) -> PathBuf {
    let dir = tmp(&format!("patch-cli-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = String::from("d2stack 1\n");
    for (name, text) in layers {
        s.push_str(&format!("layer {name}\n"));
        std::fs::write(dir.join(name), text).unwrap();
    }
    let path = dir.join(format!("{stem}.d2stack"));
    std::fs::write(&path, s).unwrap();
    path
}

const L1: &str = "d2patch 1\ntable misc\nset pt1 cost 5 -> 6\n";
const L2: &str = "d2patch 1\ntable misc\nset pt1 cost 6 -> 7\n";

/// `check`, `render` and `diff`: what each does, where `render` writes
/// (only under `D2_GAME_DIR`), and the exit codes (0 no error, 1 errors,
/// 2 usage or I/O); game files come only from `D2_GAME_DIR`.
#[test]
fn patch_tool_commands_and_exit_codes() {
    let game = &built().dir;
    let s = stack("ok", "mymod", &[("a.d2patch", L1), ("b.d2patch", L2)]);
    let sp = s.to_str().unwrap();

    // check: parse, apply, compile; the report and the data digest.
    let (code, out) = run(Some(game), &["check", sp]);
    assert_eq!(code, 0, "{out}");
    let digest = applied(&[L1, L2]).digest();
    assert!(out.ends_with(&format!("data digest {digest}\n")), "{out}");
    // An A error (and an S error) exit 1 with the finding printed.
    let bad = stack("bad", "bad", &[("a.d2patch", L2)]);
    let (code, out) = run(Some(game), &["check", bad.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("error[A06] a.d2patch:3:"), "{out}");
    let bad_stack = tmp("patch-cli-badstack.d2stack");
    std::fs::write(&bad_stack, "d2stack 2\n").unwrap();
    let (code, out) = run(Some(game), &["check", bad_stack.to_str().unwrap()]);
    assert_eq!((code, out.starts_with("error[S02]")), (1, true), "{out}");
    // A C error (a new compiler diagnostic) also exits 1.
    let c02 = stack(
        "c02",
        "c02",
        &[("a.d2patch", "d2patch 1\ntable misc\nset pt1 cost 5 -> x\n")],
    );
    let (code, out) = run(Some(game), &["check", c02.to_str().unwrap()]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("error[C02]"), "{out}");

    // render: `render(T)` to <game>/mod-render/<stack stem>/<table>.txt.
    let target = game.join("mod-render").join("mymod").join("misc.txt");
    let _ = std::fs::remove_file(&target);
    let rendered = |args: &[&str], want: &PatchData| {
        let (code, out) = run(Some(game), args);
        assert_eq!(code, 0, "{args:?}: {out}");
        assert_eq!(
            std::fs::read(&target).unwrap(),
            want.table("misc").unwrap().render(),
            "{args:?}"
        );
    };
    rendered(&["render", sp, "misc"], &applied(&[L1, L2]));
    rendered(&["render", sp, "--base", "misc"], &base());
    rendered(
        &["render", sp, "--upto", "a.d2patch", "misc"],
        &applied(&[L1]),
    );
    // Nothing is written next to the stack.
    let mut beside: Vec<String> = std::fs::read_dir(s.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    beside.sort();
    assert_eq!(beside, ["a.d2patch", "b.d2patch", "mymod.d2stack"]);

    // diff: an edited table from a state → its canonical layer.
    let edited_state = applied(&[L1, L2]);
    let edited = tmp("patch-cli-misc-edited.txt");
    std::fs::write(&edited, edited_state.table("misc").unwrap().render()).unwrap();
    let out_file = tmp("patch-cli-diff.d2patch");
    let arg = format!("misc={}", edited.display());
    for (from, layers) in [
        (vec!["--base"], vec![]),
        (vec!["--upto", "a.d2patch"], vec![L1]),
    ] {
        let state = applied(&layers);
        let want = diff_tables(
            &[(
                state.table("misc").unwrap(),
                &edited_state.table("misc").unwrap().render()[..],
            )],
            false,
        )
        .unwrap();
        let mut args = vec!["diff", sp];
        args.extend(&from);
        args.extend([arg.as_str(), "-o", out_file.to_str().unwrap()]);
        let (code, out) = run(Some(game), &args);
        assert_eq!(code, 0, "{out}");
        assert_eq!(std::fs::read(&out_file).unwrap(), want, "{from:?}");
        // Without -o the layer goes to stdout; --no-like is accepted.
        let mut args = vec!["diff", sp];
        args.extend(&from);
        args.extend([arg.as_str(), "--no-like"]);
        let (code, out) = run(Some(game), &args);
        assert_eq!((code, out.as_bytes()), (0, &want[..]));
    }
    assert_eq!(
        std::fs::read_to_string(&out_file).unwrap(),
        "d2patch 1\n\ntable misc\nset pt1 cost 6 -> 7\n"
    );

    // Usage and I/O errors exit 2; without D2_GAME_DIR nothing is read.
    for args in [
        vec![],
        vec!["check"],
        vec!["render", sp],
        vec!["diff", sp, "misc=x"],
        vec!["check", "/nonexistent/x.d2stack"],
    ] {
        assert_eq!(run(Some(game), &args).0, 2, "{args:?}");
    }
    assert_eq!(run(None, &["check", sp]).0, 2);
}

/// What is deferred is absent, never silently accepted: `remove` and
/// `delcol` are unknown statements (P04), while `addcol` is a statement of
/// version 1 (well-formed it parses clean, malformed it is P05); string
/// tables and `soundenviron.txt` are not patchable tables (A01), and a
/// string key no table has is a new compiler diagnostic (C02 `StrMiss`); a
/// patched `sounds` table does not reach the runtime sound table; `fmt` and
/// `blame` are not commands, and `check` writes no lock file.
#[test]
fn deferred_features_are_rejected() {
    let codes = |stmt: &str| {
        let text = format!("d2patch 1\ntable misc\n{stmt}\n");
        let (_, f) = parse_layer("a.d2patch", text.as_bytes(), 1);
        f.iter()
            .map(|f| (f.code, f.line, f.col))
            .collect::<Vec<_>>()
    };
    for stmt in ["remove #0 pt1", "delcol cost"] {
        assert_eq!(codes(stmt), [(Code::P04, 3, 1)], "{stmt}");
    }
    assert_eq!(codes("addcol x int"), []);
    assert_eq!(codes("addcol x"), [(Code::P05, 3, 9)]);
    for table in ["soundenviron", "string", "patchstring", "expansionstring"] {
        let text = format!("d2patch 1\ntable {table}\n");
        let (layer, f) = parse_layer("a.d2patch", text.as_bytes(), 1);
        assert!(f.is_empty(), "{f:?}");
        let f = apply_stack(&mut base(), &[layer], "s.d2stack");
        assert_eq!(f[0].code, Code::A01, "{table}");
    }

    let i = built();
    let base = base();
    let patch = |body: &str| {
        let (layer, f) = parse_layer("a.d2patch", body.as_bytes(), 1);
        assert!(f.is_empty(), "{f:?}");
        let mut data = base.clone();
        let f = apply_stack(&mut data, &[layer], "s.d2stack");
        assert!(!has_errors(&f), "{f:?}");
        compile_patched(&base, &data, &i.loaded)
    };
    let r = patch("d2patch 1\ntable weapons\nset sb1 namestr sb1 -> zz_no_such_string\n");
    let c: Vec<_> = r
        .findings
        .iter()
        .map(|f| (f.code, f.table.as_deref(), f.detail.starts_with("StrMiss")))
        .collect();
    assert_eq!(c, [(Code::C02, Some("weapons"), true)], "{:?}", r.findings);

    let sounds = base.table("sounds").unwrap();
    let n = sounds.rows.len();
    let r = patch(&format!("d2patch 1\ntable sounds\nadd #{n} zz_new_sound\n"));
    assert!(!has_errors(&r.findings), "{:?}", r.findings);
    let compiled = r.compiled.as_ref().unwrap().table("sounds").unwrap();
    assert_eq!(compiled.compiled.count, n + 1);
    let live = r.live.as_ref().unwrap();
    assert_eq!(live.sounds, i.loaded.sounds);
    assert_eq!(live.soundenviron, i.loaded.soundenviron);

    let s = stack("deferred", "d", &[("a.d2patch", L1)]);
    let sp = s.to_str().unwrap();
    for cmd in ["fmt", "blame"] {
        assert_eq!(run(Some(&i.dir), &[cmd, sp]).0, 2, "{cmd}");
    }
    assert_eq!(run(Some(&i.dir), &["check", sp]).0, 0);
    let mut beside: Vec<String> = std::fs::read_dir(s.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    beside.sort();
    assert_eq!(beside, ["a.d2patch", "d.d2stack"]);
}

/// A string a layer adds binds in `strkey` cells: the `namestr` that names
/// a missing key is a C02 alone, and clean once the layer adds the key;
/// the patched live set reads the added text.
#[test]
fn added_strings_bind_in_strkey_cells() {
    let i = built();
    let base = base();
    let set = "table weapons\nset sb1 namestr sb1 -> zz_mod_name\n";
    let missing = applied(&[&format!("d2patch 1\n{set}")]);
    let r = compile_patched(&base, &missing, &i.loaded);
    let c: Vec<_> = r.findings.iter().map(|f| f.code).collect();
    assert_eq!(c, [Code::C02], "{:?}", r.findings);

    let added = applied(&[&format!("d2patch 1\nstradd zz_mod_name [Mod Blade]\n{set}")]);
    let r = compile_patched(&base, &added, &i.loaded);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
    let live = r.live.as_ref().unwrap();
    let id = d2_data::strings::MOD_BASE;
    assert_eq!(live.strings.id(b"zz_mod_name"), id);
    assert_eq!(
        live.strings.by_index(id).unwrap(),
        "Mod Blade".encode_utf16().collect::<Vec<_>>()
    );
    // The weapon's record now carries the new id where the old one was.
    let rec = |set: &d2_data::compile_set::CompiledSet| {
        set.table("weapons").unwrap().compiled.records.clone()
    };
    let unpatched = compile_patched(&base, &base, &i.loaded).compiled.unwrap();
    let (before, after) = (rec(&unpatched), rec(r.compiled.as_ref().unwrap()));
    let old = i.loaded.strings.strkey(b"sb1").to_le_bytes();
    let new = (id as u16).to_le_bytes();
    let diff: Vec<_> = (0..before.len())
        .filter(|&k| before[k] != after[k])
        .collect();
    assert!(!diff.is_empty());
    let at = diff[0] - usize::from(old[0] == new[0]);
    assert_eq!(&after[at..at + 2], &new);
    assert_eq!(&before[at..at + 2], &old);

    // Adding a key that exists is refused.
    let twice = applied(&["d2patch 1\nstradd zz_k a\nstradd zz_k b\n"]);
    let r = compile_patched(&base, &twice, &i.loaded);
    let c: Vec<_> = r.findings.iter().map(|f| (f.code, f.line)).collect();
    assert_eq!(c, [(Code::A18, 3)]);
    assert!(r.live.is_none());
}
