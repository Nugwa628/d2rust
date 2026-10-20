//! Read-only checks of the effective table set; findings are Unverified.
mod mods;
mod rules;
use anyhow::{bail, Context, Result};
use d2_data::{
    bin::{TableFiles, DEFAULT_LANGUAGE},
    schema::schema,
    txt::TxtTable,
};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

fn check(
    source: &dyn TableFiles,
    exists: impl Fn(&str) -> bool,
    animation: impl Fn(&str) -> bool,
) -> Result<()> {
    let start = Instant::now();
    let mut tables = BTreeMap::new();
    for def in &schema().tables {
        if def.txt_name.is_empty() || tables.contains_key(&def.txt_name) {
            continue;
        }
        if let Some((_, bytes)) = source.read_excel(&def.txt_name)? {
            tables.insert(
                def.txt_name.clone(),
                TxtTable::parse(&def.txt_name, &bytes)?,
            );
        }
    }
    for required in [
        "skills.txt",
        "missiles.txt",
        "monstats.txt",
        "monstats2.txt",
        "weapons.txt",
        "armor.txt",
        "misc.txt",
        "sounds.txt",
        "levels.txt",
        "lvlprest.txt",
    ] {
        if !tables.contains_key(required) {
            bail!("effective source is missing required table {required}");
        }
    }
    let rules::Report { findings, info } = rules::report(&tables, exists, animation);
    for line in &info {
        println!("info: {line}");
    }
    for finding in &findings {
        println!("Unverified: {finding}");
    }
    println!(
        "data-check: {} tables, {} findings, {} info, {:.3} ms (source preparation excluded)",
        tables.len(),
        findings.len(),
        info.len(),
        start.elapsed().as_secs_f64() * 1000.0
    );
    if !findings.is_empty() {
        bail!("{} Unverified candidates", findings.len());
    }
    Ok(())
}
fn run() -> Result<()> {
    let mut root = None;
    let mut selected = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--mod" {
            selected = Some(args.next().context("--mod needs NAME")?);
        } else if root.is_none() && !arg.starts_with('-') {
            root = Some(PathBuf::from(arg));
        } else {
            bail!("usage: data-check [GAME_OR_NATIVE_ROOT] [--mod NAME]");
        }
    }
    let root = root
        .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from))
        .context("set D2_GAME_DIR or pass a root")?;

    if let Some(name) = selected {
        let source =
            d2_native::source::NativeSource::open_with(&root, Some(&mods::order(&root, &name)?))?;
        let tables = source.tables(DEFAULT_LANGUAGE)?;
        let anim = match source.read_native("data/global/animdata.d2") {
            Some(Ok(d2_native::source::NativeAsset::AnimData(a))) => a,
            _ => bail!("native source has no readable AnimData"),
        };
        check(
            &tables,
            |p| source.contains(&d2_native::source::fold(p)),
            |n| anim.info(n.as_bytes()).is_ok_and(|a| a.found),
        )
    } else {
        let source = d2_formats::mpq::ArchiveSet::open_dir(&root)?;
        let anim =
            d2_formats::animdata::AnimData::parse(&source.read(d2_formats::animdata::PATH)?)?;
        check(
            &source,
            |p| source.archives().iter().any(|a| a.contains(p)),
            |n| anim.info(n.as_bytes()).is_ok_and(|a| a.found),
        )
    }
}
fn main() -> Result<()> {
    let start = Instant::now();
    let result = run();
    println!(
        "data-check total: {:.3} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    result
}
