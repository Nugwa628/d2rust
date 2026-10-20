//! `data-tool patch`: check a stack, render patched tables, diff an edited
//! table into a layer. Exit 0 no error, 1 errors, 2 usage or I/O.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use d2_data::bin::{self, read_excel};
use d2_data::patch::{
    self, apply_stack, compile_patched, diff_tables, has_errors, load_stack, Finding, Layer,
    PatchData,
};
use d2_formats::mpq::ArchiveSet;

const USAGE: &str = "usage: data-tool patch check <stack>
       data-tool patch render <stack> [--base | --upto <layer>] <table>...
       data-tool patch diff <stack> (--base | --upto <layer>) <table>=<edited.txt>... [--no-like] [-o <file>]";

/// Runs `data-tool patch …`; returns the exit status.
pub fn main(args: &[String]) -> i32 {
    match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            2
        }
    }
}

fn game_dir() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var("D2_GAME_DIR").context("set D2_GAME_DIR")?,
    ))
}

pub(crate) fn print(findings: &[Finding]) {
    for f in findings {
        println!("{f}");
    }
}

/// Parses the stack and its layers (paths relative to the stack file).
pub(crate) fn stack(path: &str) -> Result<(Vec<Layer>, Vec<Finding>)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    let dir = Path::new(path).parent().unwrap_or(Path::new("")).to_owned();
    Ok(load_stack(path, &bytes, &mut |p| {
        std::fs::read(dir.join(p)).ok()
    }))
}

pub(crate) fn base(set: &ArchiveSet) -> std::result::Result<PatchData, Vec<Finding>> {
    let mut read = |f: &str| read_excel(set, f).map_err(|e| e.to_string());
    PatchData::from_base(&patch::rules(), &mut read)
}

/// `--base` → no layers; `--upto <layer>` → the layers through it.
fn upto<'a>(layers: &'a [Layer], opt: &[String]) -> Result<&'a [Layer]> {
    match opt {
        [flag] if flag == "--base" => Ok(&[]),
        [flag, name] if flag == "--upto" => match layers.iter().position(|l| &l.path == name) {
            Some(i) => Ok(&layers[..=i]),
            None => bail!("layer {name} is not in the stack"),
        },
        _ => bail!(USAGE),
    }
}

/// Base, patched state and findings so far.
type State = (PatchData, PatchData, Vec<Finding>);

/// Reads the stack and base and applies `layers` (all, or a prefix);
/// `Err(code)` after printing findings.
fn state(stack_path: &str, select: Option<&[String]>) -> Result<std::result::Result<State, i32>> {
    let (layers, mut findings) = stack(stack_path)?;
    if has_errors(&findings) {
        patch::sort_report(&mut findings);
        print(&findings);
        return Ok(Err(1));
    }
    let layers = match select {
        Some(opt) => upto(&layers, opt)?,
        None => &layers[..],
    };
    let set = ArchiveSet::open_dir(game_dir()?).context("opening the game archives")?;
    let base = match base(&set) {
        Ok(b) => b,
        Err(f) => {
            print(&f);
            return Ok(Err(1));
        }
    };
    let mut data = base.clone();
    findings.extend(apply_stack(&mut data, layers, stack_path));
    patch::sort_report(&mut findings);
    if has_errors(&findings) {
        print(&findings);
        return Ok(Err(1));
    }
    Ok(Ok((base, data, findings)))
}

fn run(args: &[String]) -> Result<i32> {
    match args {
        [cmd, stack_path] if cmd == "check" => {
            let (base, data, mut findings) = match state(stack_path, None)? {
                Ok(s) => s,
                Err(code) => return Ok(code),
            };
            let set = ArchiveSet::open_dir(game_dir()?).context("opening the game archives")?;
            let live = bin::load(&set, bin::DEFAULT_LANGUAGE).context("loading the live set")?;
            let r = compile_patched(&base, &data, &live);
            findings.extend(r.findings);
            patch::sort_report(&mut findings);
            print(&findings);
            println!("data digest {}", data.digest());
            Ok(i32::from(has_errors(&findings)))
        }
        [cmd, stack_path, rest @ ..] if cmd == "render" => {
            let (opt, tables) = match rest.first().map(String::as_str) {
                Some("--base") => (Some(&rest[..1]), &rest[1..]),
                Some("--upto") if rest.len() >= 2 => (Some(&rest[..2]), &rest[2..]),
                _ => (None, rest),
            };
            if tables.is_empty() {
                bail!(USAGE);
            }
            let (_, data, findings) = match state(stack_path, opt)? {
                Ok(s) => s,
                Err(code) => return Ok(code),
            };
            print(&findings);
            let stem = Path::new(stack_path)
                .file_stem()
                .context("stack file name")?
                .to_string_lossy()
                .into_owned();
            let dir = game_dir()?.join("mod-render").join(stem);
            std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            for name in tables {
                let t = data
                    .table(name)
                    .with_context(|| format!("no patchable table {name}"))?;
                let path = dir.join(format!("{name}.txt"));
                std::fs::write(&path, t.render())
                    .with_context(|| format!("writing {}", path.display()))?;
                println!("wrote {}", path.display());
            }
            Ok(0)
        }
        [cmd, stack_path, rest @ ..] if cmd == "diff" => {
            let n = match rest.first().map(String::as_str) {
                Some("--base") => 1,
                Some("--upto") => 2,
                _ => bail!(USAGE),
            };
            if rest.len() < n {
                bail!(USAGE);
            }
            let (opt, rest) = rest.split_at(n);
            let mut no_like = false;
            let mut out_file = None;
            let mut edits = Vec::new();
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--no-like" => no_like = true,
                    "-o" => out_file = Some(it.next().context(USAGE)?.clone()),
                    _ => {
                        let (t, f) = a.split_once('=').context(USAGE)?;
                        let bytes = std::fs::read(f).with_context(|| format!("reading {f}"))?;
                        edits.push((t.to_owned(), bytes));
                    }
                }
            }
            if edits.is_empty() {
                bail!(USAGE);
            }
            let (_, data, _) = match state(stack_path, Some(opt))? {
                Ok(s) => s,
                Err(code) => return Ok(code),
            };
            let mut pairs = Vec::new();
            for (t, bytes) in &edits {
                let table = data
                    .table(t)
                    .with_context(|| format!("no patchable table {t}"))?;
                pairs.push((table, bytes.as_slice()));
            }
            match diff_tables(&pairs, no_like) {
                Err(f) => {
                    println!("{f}");
                    Ok(1)
                }
                Ok(layer) => {
                    match out_file {
                        Some(p) => {
                            std::fs::write(&p, &layer).with_context(|| format!("writing {p}"))?
                        }
                        None => print!("{}", String::from_utf8_lossy(&layer)),
                    }
                    Ok(0)
                }
            }
        }
        _ => bail!(USAGE),
    }
}
