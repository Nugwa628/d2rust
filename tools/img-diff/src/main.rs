//! `img-diff`: compare two screenshots, or two folders of screenshots, from
//! two D2RUST builds.
//!
//! Usage:
//!   img-diff A B [--tol N] [--max-pct P] [--mask M[,M..]]... [--out DIR]
//!
//! - `A`, `B`: two PNG files, or two folders whose `.png` files are paired
//!   by name (a name in only one folder fails).
//! - `--tol N`: per-channel colour difference still counted as equal
//!   (0-255, default 0).
//! - `--max-pct P`: share of changed pixels, in percent of the compared
//!   ones, that still passes (default 0).
//! - `--mask`: regions left out: `hud`, `belt`, `edges:N`, `rect:X,Y,W,H`.
//!   Repeatable, or comma-separated (`rect:` last).
//! - `--out DIR`: where `diff.png` (or `<name>.diff.png` per folder pair)
//!   and `verdict.json` go (default `img-diff-out`). Diff images show the
//!   after shot in grey with changed pixels red and masked regions dark blue.
//!
//! The verdict JSON is also printed. Exit code: 0 pass, 1 fail, 2 bad
//! arguments or unreadable files.
//!
//! Shots are game art: keep them and the diffs in a work folder, never in
//! the repository.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use img_diff::{compare, pair_names, Mask, Options};
use serde_json::{json, Value};

struct Args {
    a: PathBuf,
    b: PathBuf,
    out: PathBuf,
    opts: Options,
}

fn parse_args(raw: &[String]) -> Result<Args> {
    let mut paths = Vec::new();
    let mut opts = Options::default();
    let mut out = PathBuf::from("img-diff-out");
    let mut it = raw.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .with_context(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--tol" => opts.tol = value("--tol")?.parse().context("--tol takes 0-255")?,
            "--max-pct" => {
                let p: f64 = value("--max-pct")?
                    .parse()
                    .context("--max-pct takes a number")?;
                if !(0.0..=100.0).contains(&p) {
                    bail!("--max-pct must be 0-100");
                }
                opts.max_pct = p;
            }
            "--mask" => opts
                .masks
                .extend(Mask::parse_list(&value("--mask")?).map_err(anyhow::Error::msg)?),
            "--out" => out = PathBuf::from(value("--out")?),
            "-h" | "--help" => bail!("usage"),
            s if s.starts_with("--") => bail!("unknown option {s}"),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    let [a, b]: [PathBuf; 2] = paths
        .try_into()
        .map_err(|_| anyhow::anyhow!("give exactly two paths: A B"))?;
    Ok(Args { a, b, out, opts })
}

fn load(path: &Path) -> Result<image::RgbaImage> {
    Ok(image::open(path)
        .with_context(|| format!("reading {}", path.display()))?
        .to_rgba8())
}

/// Compares one pair and writes its diff image. Returns the pair's JSON.
fn one_pair(a: &Path, b: &Path, diff_path: &Path, opts: &Options) -> Result<Value> {
    let (ia, ib) = (load(a)?, load(b)?);
    Ok(match compare(&ia, &ib, opts) {
        Ok((outcome, diff)) => {
            diff.save(diff_path)
                .with_context(|| format!("writing {}", diff_path.display()))?;
            let mut v = outcome.json();
            v["diff"] = json!(diff_path.display().to_string());
            v
        }
        Err(m) => json!({
            "pass": false,
            "error": format!("size {}x{} vs {}x{}", m.a.0, m.a.1, m.b.0, m.b.1),
        }),
    })
}

fn png_names(dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for e in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let name = e?.file_name().to_string_lossy().into_owned();
        if name.to_ascii_lowercase().ends_with(".png") {
            names.push(name);
        }
    }
    Ok(names)
}

fn run(args: &Args) -> Result<Value> {
    fs::create_dir_all(&args.out).with_context(|| format!("creating {}", args.out.display()))?;
    let opts = &args.opts;
    let settings = json!({
        "tol": opts.tol,
        "max_pct": opts.max_pct,
        "masks": opts.masks.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
    });
    let mut verdict = match (args.a.is_dir(), args.b.is_dir()) {
        (false, false) => one_pair(&args.a, &args.b, &args.out.join("diff.png"), opts)?,
        (true, true) => {
            let pairs = pair_names(&png_names(&args.a)?, &png_names(&args.b)?);
            if pairs.is_empty() {
                bail!("no .png files in either folder");
            }
            let mut shots = serde_json::Map::new();
            let mut pass = true;
            for (name, in_a, in_b) in pairs {
                let v = if in_a && in_b {
                    let stem = name.trim_end_matches(".png").trim_end_matches(".PNG");
                    let diff = args.out.join(format!("{stem}.diff.png"));
                    one_pair(&args.a.join(&name), &args.b.join(&name), &diff, opts)?
                } else {
                    let side = if in_a { "only in A" } else { "only in B" };
                    json!({ "pass": false, "error": side })
                };
                pass &= v["pass"] == json!(true);
                shots.insert(name, v);
            }
            json!({ "pass": pass, "shots": shots })
        }
        _ => bail!("A and B must both be files or both be folders"),
    };
    verdict["settings"] = settings;
    let path = args.out.join("verdict.json");
    fs::write(&path, serde_json::to_string_pretty(&verdict)? + "\n")
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(verdict)
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("img-diff: {e}");
            eprintln!(
                "usage: img-diff A B [--tol N] [--max-pct P] [--mask hud|belt|edges:N|rect:X,Y,W,H] [--out DIR]"
            );
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            if v["pass"] == json!(true) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("img-diff: {e:#}");
            ExitCode::from(2)
        }
    }
}
