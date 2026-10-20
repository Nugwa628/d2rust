//! `mpq-tool formats`: parse every known file in every archive with the
//! matching `d2-formats` parser. Phase 1 exit check.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use d2_formats::cof::Cof;
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::font::FontTable;
use d2_formats::mpq::names::known_names;
use d2_formats::mpq::Archive;
use d2_formats::palette::{Palette, Pl2};
use d2_formats::tbl::StringTable;

/// Files known to be unusable leftovers, with the spec that documents each.
/// They're still parsed: a failure is expected, a success means this list
/// is stale.
const KNOWN_UNUSED: &[(&str, &str)] = &[
    (
        r"data\global\chars\am\cof\amblxbow.cof",
        "specs/formats/cof.md",
    ),
    (
        r"data\global\tiles\act1\barracks\barracks.dt1",
        "specs/formats/dt1.md",
    ),
    (
        r"data\global\tiles\act1\barracks\gargtrap.dt1",
        "specs/formats/dt1.md",
    ),
    (
        r"data\global\tiles\act1\catacomb\catacombs.dt1",
        "specs/formats/dt1.md",
    ),
    (
        r"data\global\tiles\act1\cathedrl\cathedrl.dt1",
        "specs/formats/dt1.md",
    ),
    (
        r"data\global\tiles\act1\court\court.dt1",
        "specs/formats/dt1.md",
    ),
    (
        r"data\global\tiles\act1\outdoors\outdoor1.dt1",
        "specs/formats/dt1.md",
    ),
];

fn known_unused(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    KNOWN_UNUSED
        .iter()
        .find(|(n, _)| *n == lower)
        .map(|&(_, spec)| spec)
}

/// Kinds with a parser; anything else is only read from the archive.
const PARSED_KINDS: &[&str] = &[
    "pal.dat", "pl2", "dc6", "tbl", "font.tbl", "cof", "dt1", "ds1", "dcc",
];

#[derive(Default)]
struct Tally {
    total: usize,
    ok: usize,
    skipped: usize,
    errors: Vec<String>,
    notes: BTreeMap<String, usize>,
}

impl Tally {
    fn note(&mut self, what: impl Into<String>) {
        *self.notes.entry(what.into()).or_insert(0) += 1;
    }
}

/// Parses one file; returns notes to record, or an error message.
fn parse(kind: &str, name: &str, bytes: &[u8], tally: &mut Tally) -> Result<(), String> {
    match kind {
        "pal.dat" => Palette::parse(bytes).map(|_| ()).map_err(|e| e.to_string()),
        "cof" => {
            let c = Cof::parse(bytes).map_err(|e| e.to_string())?;
            tally.note(format!("version = {}", c.version));
            if !c.event_padding.is_empty() {
                tally.note(format!(
                    "event padding {} bytes (L{} F{} D{})",
                    c.event_padding.len(),
                    c.layers_count,
                    c.frames,
                    c.directions
                ));
            }
            Ok(())
        }
        "dt1" => {
            let d = Dt1::parse(bytes).map_err(|e| e.to_string())?;
            tally.note(format!("minor version = {}", d.minor_version));
            for t in &d.tiles {
                for b in &t.blocks {
                    tally.note(format!("block format {:#06x}", b.format));
                }
            }
            Ok(())
        }
        "ds1" => {
            let d = Ds1::parse(bytes).map_err(|e| e.to_string())?;
            tally.note(format!("version = {}", d.version));
            if !d.trailing.is_empty() {
                let head: Vec<String> = d
                    .trailing
                    .iter()
                    .take(8)
                    .map(|b| format!("{b:02x}"))
                    .collect();
                tally.note(format!(
                    "trailing {} bytes (v{}, tag_type {}, starts {})",
                    d.trailing.len(),
                    d.version,
                    d.tag_type,
                    head.join(" ")
                ));
            }
            if d.groups_truncated {
                tally.note(format!("truncated groups: {name}"));
            }
            for f in &d.files {
                let f = String::from_utf8_lossy(f).to_ascii_lowercase();
                if KNOWN_UNUSED.iter().any(|(n, _)| {
                    n.ends_with(".dt1") && f.ends_with(n.rsplit('\\').next().unwrap())
                }) {
                    tally.note(format!("references a known-unused DT1: {name} -> {f}"));
                }
            }
            Ok(())
        }
        "dcc" => {
            let d = Dcc::parse(bytes).map_err(|e| e.to_string())?;
            tally.note(format!("version = {}", d.version));
            for dir in &d.directions {
                if dir.pcd_leftover_bits >= 8 {
                    tally.note(format!("PCD leftover >= 8 bits: {name}"));
                }
                for f in &dir.frames {
                    if f.bottom_up {
                        tally.note(format!("bottom-up frame: {name}"));
                    }
                }
            }
            Ok(())
        }
        "font.tbl" => {
            let f = FontTable::parse(bytes).map_err(|e| e.to_string())?;
            tally.note(format!("{} glyphs", f.glyphs.len()));
            Ok(())
        }
        "pl2" => {
            let p = Pl2::parse(bytes).map_err(|e| e.to_string())?;
            tally.note(format!("text colors = {}", p.text_colors.len()));
            Ok(())
        }
        "dc6" => {
            let d = Dc6::parse(bytes).map_err(|e| e.to_string())?;
            for f in &d.frames {
                tally.note(format!("flip = {}", f.flip));
            }
            if d.frames.iter().any(|f| f.flip != 0) {
                tally.note(format!("has flipped frames: {name}"));
            }
            tally.note(format!("termination = {:02x?}", d.header.termination));
            Ok(())
        }
        "tbl" => {
            let t = StringTable::parse(bytes).map_err(|e| e.to_string())?;
            if t.header.file_size as usize != bytes.len() {
                tally.note("file_size field differs from length");
            }
            tally.note(format!("version = {}", t.header.version));
            // Every element's key must look itself up, unless an earlier
            // slot in the probe sequence holds the same key (duplicates).
            let mut unresolved = 0;
            for i in 0..t.indices.len() {
                let Some(e) = t.element(i) else { continue };
                if !e.key.is_ascii() {
                    tally.note("non-ASCII key");
                }
                match t.find_slot(&e.key) {
                    Some(s) if t.entries[s].key == e.key => {}
                    _ => unresolved += 1,
                }
            }
            if unresolved > 0 {
                return Err(format!("{name}: {unresolved} keys do not resolve"));
            }
            tally.note(format!("{name}: {} elements", t.indices.len()));
            Ok(())
        }
        _ => Ok(()),
    }
}

/// True if every byte is printable ASCII, tab, CR or LF.
fn is_text(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .all(|&b| matches!(b, b'\t' | b'\r' | b'\n' | 0x20..=0x7E))
}

fn kind_of(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(r"\pal.dat") && lower.contains(r"\palette\") {
        return "pal.dat".into();
    }
    match lower.rsplit_once('.') {
        Some((_, ext)) if !ext.contains('\\') => ext.to_owned(),
        _ => "(none)".into(),
    }
}

pub fn run(dir: &Path) -> Result<()> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mpq")))
        .collect();
    paths.sort();
    let archives: Vec<Archive> = paths.iter().map(Archive::open).collect::<Result<_, _>>()?;

    let names = known_names(&archives)?;

    // (kind) -> tally; parse every copy of every name in every archive.
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut unnamed: BTreeMap<String, usize> = BTreeMap::new();
    for a in &archives {
        let archive_name = a
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_lowercase();
        let mut named_blocks = BTreeSet::new();
        for name in names.values() {
            let Some(index) = a.find(name) else { continue };
            named_blocks.insert(index);
            let read = a.read(name);
            let mut kind = kind_of(name);
            if let Ok(bytes) = &read {
                if kind == "tbl" && FontTable::is_font_table(bytes) {
                    kind = "font.tbl".into();
                } else if kind == "tbl" && is_text(bytes) {
                    // Plain-text mapping files.
                    kind = "text.tbl".into();
                }
            }
            let tally = tallies.entry(kind.clone()).or_default();
            tally.total += 1;
            let result = match read {
                Ok(_) if !PARSED_KINDS.contains(&kind.as_str()) => {
                    tally.skipped += 1;
                    continue;
                }
                Ok(bytes) => parse(&kind, name, &bytes, tally),
                Err(e) => Err(e.to_string()),
            };
            match (result, known_unused(name)) {
                (Ok(()), None) => tally.ok += 1,
                (Err(e), None) => tally.errors.push(format!("{archive_name}: {name}: {e}")),
                (Err(_), Some(spec)) => tally.note(format!("known unused ({spec}): {name}")),
                (Ok(()), Some(_)) => tally.errors.push(format!(
                    "{archive_name}: {name}: listed as known unused but parses (stale list)"
                )),
            }
        }
        let in_use = a
            .block_table()
            .iter()
            .filter(|b| b.has(d2_formats::mpq::flags::EXISTS))
            .count();
        unnamed.insert(archive_name, in_use - named_blocks.len());
    }

    let mut failures = 0;
    println!(
        "{:<10} {:>7} {:>7} {:>7} {:>8}",
        "kind", "files", "parsed", "errors", "skipped"
    );
    for (kind, t) in &tallies {
        println!(
            "{kind:<10} {:>7} {:>7} {:>7} {:>8}",
            t.total,
            t.ok,
            t.errors.len(),
            t.skipped
        );
        for (note, n) in &t.notes {
            println!("             {note}  (x{n})");
        }
        for e in t.errors.iter().take(8) {
            println!("             error: {e}");
        }
        failures += t.errors.len();
    }
    println!("blocks without a known name:");
    for (archive, n) in &unnamed {
        println!("  {archive}: {n}");
    }
    if failures > 0 {
        bail!("{failures} files failed to parse");
    }
    Ok(())
}
