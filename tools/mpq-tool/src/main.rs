//! MPQ inspection tool.
//!
//! Usage (run with --release for `check`):
//!   mpq-tool info    <archive.mpq>
//!   mpq-tool list    <archive.mpq>
//!   mpq-tool extract <archive.mpq> <pattern> [out_dir]
//!   mpq-tool extract-names <archive.mpq> <names.txt> <out_dir>
//!   mpq-tool check   [game_dir]
//!   mpq-tool formats [game_dir]
//!   mpq-tool render  <name> [palette] [out.png]
//!
//! `pattern` is case-insensitive with `*` wildcards, e.g. `data\global\excel\*.txt`.
//! `extract-names` extracts the names listed one per line (for archives
//! whose `(listfile)` is missing or partial); names not in the archive are
//! skipped and counted.
//! Extracted files default to `game/extracted/<archive>/` (gitignored).
//! `check` decodes every block of every archive in `game_dir` (default
//! $D2_GAME_DIR) and prints a survey of flags and compression masks.
//! `formats` parses every named file with its `d2-formats` parser.
//! `render` draws a DC6/DCC/DT1 to `game/renders/` (gitignored) for visual checks.

mod formats;
mod render;

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use d2_formats::mpq::{flags, Archive, SectorStats};

fn game_dir(arg: Option<&String>) -> Result<PathBuf> {
    Ok(match arg {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var("D2_GAME_DIR").context("set D2_GAME_DIR")?),
    })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("info") if args.len() == 2 => info(&args[1]),
        Some("list") if args.len() == 2 => list(&args[1]),
        Some("extract") if (3..=4).contains(&args.len()) => {
            extract(&args[1], &args[2], args.get(3).map(PathBuf::from))
        }
        Some("extract-names") if args.len() == 4 => {
            extract_names(&args[1], &args[2], Path::new(&args[3]))
        }
        Some("check") if args.len() <= 2 => check(&game_dir(args.get(1))?),
        Some("formats") if args.len() <= 2 => formats::run(&game_dir(args.get(1))?),
        Some("render") if (2..=4).contains(&args.len()) => render::run(
            &game_dir(None)?,
            &args[1],
            args.get(2).map(String::as_str),
            args.get(3).map(PathBuf::from),
        ),
        _ => bail!(
            "usage: mpq-tool info|list <mpq> | extract <mpq> <pattern> [out] | check|formats [game_dir]"
        ),
    }
}

fn open(path: &str) -> Result<Archive> {
    Archive::open(path).with_context(|| format!("opening {path}"))
}

fn info(path: &str) -> Result<()> {
    let a = open(path)?;
    let h = a.header();
    println!("{h:#?}");
    println!("sector size: {}", a.sector_size());
    let used = a
        .block_table()
        .iter()
        .filter(|b| b.has(flags::EXISTS))
        .count();
    println!("blocks in use: {used} / {}", a.block_table().len());
    let mut locales = BTreeMap::new();
    for e in a
        .hash_table()
        .iter()
        .filter(|e| !e.is_empty() && !e.is_deleted())
    {
        *locales.entry(e.locale).or_insert(0u32) += 1;
    }
    println!("hash entries by locale: {locales:x?}");
    match a.listfile()? {
        Some(names) => println!("(listfile): {} names", names.len()),
        None => println!("(listfile): none"),
    }
    Ok(())
}

fn list(path: &str) -> Result<()> {
    let a = open(path)?;
    let names = a.listfile()?.context("archive has no (listfile)")?;
    for name in names {
        if let Some(i) = a.find(&name) {
            println!("{:>10}  {name}", a.block_table()[i].file_size);
        }
    }
    Ok(())
}

/// Case-insensitive match with `*` matching any run of characters.
fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern
        .to_ascii_lowercase()
        .replace('/', "\\")
        .chars()
        .collect();
    let t: Vec<char> = text
        .to_ascii_lowercase()
        .replace('/', "\\")
        .chars()
        .collect();
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// Turns an archive name into a relative path, refusing anything that could
/// escape the output directory.
fn safe_relative(name: &str) -> Option<PathBuf> {
    if name.contains(':') {
        return None;
    }
    let path: PathBuf = name.split(['\\', '/']).collect();
    path.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then_some(path)
}

fn extract(path: &str, pattern: &str, out: Option<PathBuf>) -> Result<()> {
    let a = open(path)?;
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let out = out.unwrap_or_else(|| PathBuf::from("game/extracted").join(stem));
    let names = a.listfile()?.context("archive has no (listfile)")?;
    let names: Vec<String> = names
        .iter()
        .filter(|n| wildcard(pattern, n))
        .map(|n| n.to_string())
        .collect();
    let (count, _) = write_names(&a, &names, &out, true)?;
    println!("extracted {count} files to {}", out.display());
    Ok(())
}

fn extract_names(path: &str, list: &str, out: &Path) -> Result<()> {
    let a = open(path)?;
    let text = std::fs::read_to_string(list).with_context(|| format!("reading {list}"))?;
    let mut names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names.dedup_by_key(|n| n.to_ascii_lowercase());
    let (count, missing) = write_names(&a, &names, out, false)?;
    println!(
        "extracted {count} files to {} ({missing} listed names not in the archive)",
        out.display()
    );
    Ok(())
}

/// Writes each name under `out`; returns (written, unreadable). Read errors
/// are printed when `report` is set (names from the archive's own listfile).
fn write_names(a: &Archive, names: &[String], out: &Path, report: bool) -> Result<(usize, usize)> {
    let (mut count, mut missing) = (0, 0);
    for name in names {
        let Some(rel) = safe_relative(name) else {
            eprintln!("skipping unsafe name: {name}");
            continue;
        };
        let bytes = match a.read(name) {
            Ok(b) => b,
            Err(e) => {
                if report {
                    eprintln!("{name}: {e}");
                }
                missing += 1;
                continue;
            }
        };
        let dest = out.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, bytes)?;
        count += 1;
    }
    Ok((count, missing))
}

#[derive(Default)]
struct Report {
    archive: String,
    blocks: usize,
    decoded: usize,
    empty: usize,
    named: usize,
    recovered: usize,
    listfile_names: Option<usize>,
    listfile_missing: usize,
    wav_ok: usize,
    wav_bad: Vec<String>,
    errors: Vec<String>,
    flags: BTreeMap<u32, usize>,
    stats: SectorStats,
}

fn check_archive(path: &Path) -> Report {
    let mut r = Report {
        archive: path.file_name().unwrap().to_string_lossy().into_owned(),
        ..Report::default()
    };
    let a = match Archive::open(path) {
        Ok(a) => a,
        Err(e) => {
            r.errors.push(format!("open: {e}"));
            return r;
        }
    };

    // Map block index -> name using the listfile, where available.
    let mut names: BTreeMap<usize, String> = BTreeMap::new();
    match a.listfile() {
        Ok(Some(list)) => {
            r.listfile_names = Some(list.len());
            for name in list {
                match a.find(&name) {
                    Some(i) => {
                        names.entry(i).or_insert(name);
                    }
                    None => r.listfile_missing += 1,
                }
            }
        }
        Ok(None) => {}
        Err(e) => r.errors.push(format!("(listfile): {e}")),
    }

    for (i, block) in a.block_table().iter().enumerate() {
        if !block.has(flags::EXISTS) || block.has(flags::DELETE_MARKER) {
            continue;
        }
        r.blocks += 1;
        *r.flags.entry(block.flags).or_insert(0) += 1;
        if block.file_size == 0 {
            r.empty += 1;
            continue;
        }
        let name = names.get(&i);
        let key = if !block.has(flags::ENCRYPTED) {
            None
        } else if let Some(n) = name {
            r.named += 1;
            a.file_key(n, i)
        } else {
            match a.recover_key(i) {
                Ok(Some(k)) => {
                    r.recovered += 1;
                    Some(k)
                }
                Ok(None) => {
                    r.errors
                        .push(format!("block {i}: encrypted, no name, key not recovered"));
                    continue;
                }
                Err(e) => {
                    r.errors.push(format!("block {i}: {e}"));
                    continue;
                }
            }
        };
        match a.read_block_stats(i, key, &mut r.stats) {
            Ok(bytes) => {
                r.decoded += 1;
                if bytes.starts_with(b"RIFF") && bytes.len() >= 8 {
                    let riff = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                    if riff as usize + 8 == bytes.len() {
                        r.wav_ok += 1;
                    } else {
                        let label = name.cloned().unwrap_or_else(|| format!("block {i}"));
                        r.wav_bad.push(format!(
                            "{label}: RIFF size {} vs {}",
                            riff + 8,
                            bytes.len()
                        ));
                    }
                }
            }
            Err(e) => {
                let label = name.cloned().unwrap_or_else(|| format!("block {i}"));
                r.errors.push(format!("{label}: {e}"));
            }
        }
    }
    r
}

fn check(dir: &Path) -> Result<()> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mpq")))
        .collect();
    paths.sort();
    if paths.is_empty() {
        bail!("no .mpq files in {}", dir.display());
    }

    let reports: Vec<Report> = std::thread::scope(|s| {
        let handles: Vec<_> = paths
            .iter()
            .map(|p| s.spawn(move || check_archive(p)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("worker panicked"))
            .collect()
    });

    let mut failures = 0;
    for r in &reports {
        println!("== {}", r.archive);
        match r.listfile_names {
            Some(n) => println!(
                "  (listfile): {n} names, {} not found in archive",
                r.listfile_missing
            ),
            None => println!("  (listfile): none"),
        }
        println!(
            "  blocks: {} in use, {} decoded, {} empty, {} errors",
            r.blocks,
            r.decoded,
            r.empty,
            r.errors.len()
        );
        println!(
            "  encrypted: {} keyed by name, {} keys recovered",
            r.named, r.recovered
        );
        let flags: Vec<String> = r
            .flags
            .iter()
            .map(|(f, n)| format!("{f:#010x}x{n}"))
            .collect();
        println!("  block flags: {}", flags.join(" "));
        let masks: Vec<String> = r
            .stats
            .masks
            .iter()
            .enumerate()
            .filter(|(_, &n)| n > 0)
            .map(|(m, n)| format!("{m:#04x}x{n}"))
            .collect();
        println!(
            "  sector masks (0x00 = stored): {}; imploded-flag sectors: {}",
            masks.join(" "),
            r.stats.imploded
        );
        println!("  wav: {} valid, {} bad", r.wav_ok, r.wav_bad.len());
        for w in r.wav_bad.iter().take(5) {
            println!("    bad wav: {w}");
        }
        for e in r.errors.iter().take(10) {
            println!("    error: {e}");
        }
        if r.errors.len() > 10 {
            println!("    ... {} more errors", r.errors.len() - 10);
        }
        failures += r.errors.len() + r.wav_bad.len();
    }
    if failures > 0 {
        bail!("{failures} problems found");
    }
    println!("all blocks decoded");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matching() {
        assert!(wildcard(
            r"data\global\excel\*.txt",
            r"DATA\GLOBAL\EXCEL\Armor.txt"
        ));
        assert!(wildcard("*.dc6", "data/global/ui/panel/invchar6.DC6"));
        assert!(!wildcard("*.dc6", "foo.dcc"));
        assert!(wildcard("*", "anything"));
    }

    #[test]
    fn unsafe_names_rejected() {
        assert!(safe_relative(r"data\global\x.txt").is_some());
        assert!(safe_relative(r"..\evil.txt").is_none());
        assert!(safe_relative(r"C:\evil.txt").is_none());
    }
}
