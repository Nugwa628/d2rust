//! `data-tool variant`: build a test-variant install from a patch stack,
//! or re-check one.
//!
//! ```text
//! data-tool variant build traces/variants/<name>/<name>.d2stack [--game DIR] [--out DIR]
//! data-tool variant check <out dir> [--game DIR]
//! ```
//!
//! `build` applies the stack to the base install's tables and compiles
//! them; the patched set is every `.bin` the game reads whose compiled
//! bytes differ from the live file (load order P → X → D). It writes
//! `<out>/` (default
//! `$D2_GAME_DIR/../variants/<name>/`, refused inside this repository's
//! work tree): every top-level file of the install hard-linked (or
//! copied), except `patch_d2.mpq`, rewritten by [`patch_archive`] (the
//! base archive's bytes kept, the patched `.bin` files appended stored,
//! new hash and block tables after them), then runs the variant check
//! and writes `variant.json`. Variant installs are game files: they
//! never go in the public repo.
//!
//! `check` re-runs the variant check of an out dir against the base
//! install (`--game` or `D2_GAME_DIR`), from its `variant.json`.
//!
//! Exit 0 ok, 1 findings or a failed check, 2 usage or I/O.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use d2_data::bin::{self, excel_path, read_excel};
use d2_data::patch::{self, apply_stack, compile_patched, has_errors};
use d2_data::schema::schema;
use d2_formats::mpq::crypto::{decrypt, encrypt, hash, HashType, BLOCK_TABLE_KEY, HASH_TABLE_KEY};
use d2_formats::mpq::{flags, Archive, ArchiveSet};
use sha2::{Digest, Sha256};

const USAGE: &str = "usage: data-tool variant build <traces/variants/<name>/<name>.d2stack> [--game DIR] [--out DIR]
       data-tool variant check <out dir> [--game DIR]";

const PATCH_D2: &str = "patch_d2.mpq";
const JSON: &str = "variant.json";

/// Runs `data-tool variant …`; returns the exit status.
pub fn main(args: &[String]) -> i32 {
    match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            2
        }
    }
}

fn run(args: &[String]) -> Result<i32> {
    let (cmd, rest) = args.split_first().context(USAGE)?;
    let (target, opts) = rest.split_first().context(USAGE)?;
    let mut game = None;
    let mut out = None;
    let mut it = opts.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--game" => game = Some(PathBuf::from(it.next().context(USAGE)?)),
            "--out" if cmd == "build" => out = Some(PathBuf::from(it.next().context(USAGE)?)),
            _ => bail!(USAGE),
        }
    }
    let game = match game {
        Some(g) => g,
        None => PathBuf::from(std::env::var("D2_GAME_DIR").context("set D2_GAME_DIR or --game")?),
    };
    let game = game
        .canonicalize()
        .with_context(|| format!("game dir {}", game.display()))?;
    match cmd.as_str() {
        "build" => build(Path::new(target), &game, out),
        "check" => check(Path::new(target), &game),
        _ => bail!(USAGE),
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ------------------------------------------------------------ archive

const HEADER_MAGIC: [u8; 4] = *b"MPQ\x1A";
const USER_DATA_MAGIC: [u8; 4] = *b"MPQ\x1B";
const HASH_EMPTY: u32 = 0xFFFF_FFFF;
const HASH_DELETED: u32 = 0xFFFF_FFFE;

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// The archive start `A` (where the MPQ header sits in the file).
fn archive_start(b: &[u8]) -> Result<usize> {
    let mut at = 0usize;
    while at + 32 <= b.len() {
        if b[at..at + 4] == HEADER_MAGIC {
            return Ok(at);
        }
        if b[at..at + 4] == USER_DATA_MAGIC {
            let a = at + u32_at(b, at + 8) as usize;
            ensure!(
                a + 32 <= b.len() && b[a..a + 4] == HEADER_MAGIC,
                "bad user-data header"
            );
            return Ok(a);
        }
        at += 0x200;
    }
    bail!("not an MPQ archive")
}

/// A decrypted table of `count` 16-byte entries at `A + pos`.
fn read_table(b: &[u8], a: usize, pos: u32, count: u32, key: u32) -> Result<Vec<[u32; 4]>> {
    let start = a + pos as usize;
    let end = start + count as usize * 16;
    ensure!(
        end <= b.len(),
        "table at {start:#x} extends past end of file"
    );
    let mut bytes = b[start..end].to_vec();
    decrypt(&mut bytes, key);
    Ok(bytes
        .as_chunks::<16>()
        .0
        .iter()
        .map(|e| [u32_at(e, 0), u32_at(e, 4), u32_at(e, 8), u32_at(e, 12)])
        .collect())
}

fn table_bytes(t: &[[u32; 4]], key: u32) -> Vec<u8> {
    let mut bytes: Vec<u8> = t.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    encrypt(&mut bytes, key);
    bytes
}

/// Rewrites an MPQ archive for a variant install: every byte of
/// `base` is kept; each file of `files` is appended stored (no
/// compression, no encryption, not single-unit: one contiguous range),
/// its block entry replaced when the name has a locale-0
/// hash entry, else a new block entry appended and the name put in the
/// first free (empty or deleted) slot of its hash probe; the hash
/// and block tables follow, encrypted with their fixed keys, and the
/// header's table offsets, block count and archive size are updated. An
/// empty `files` returns `base` unchanged.
pub fn patch_archive(base: &[u8], files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    if files.is_empty() {
        return Ok(base.to_vec());
    }
    let a = archive_start(base)?;
    let h = &base[a..a + 32];
    ensure!(u32_at(h, 4) >= 0x20, "header size {:#x}", u32_at(h, 4));
    ensure!(
        u16::from_le_bytes([h[0x0C], h[0x0D]]) == 0,
        "format version not 0"
    );
    let hash_count = u32_at(h, 0x18);
    ensure!(
        hash_count.is_power_of_two(),
        "hash table count {hash_count}"
    );
    let mut hashes = read_table(base, a, u32_at(h, 0x10), hash_count, HASH_TABLE_KEY)?;
    let mut blocks = read_table(base, a, u32_at(h, 0x14), u32_at(h, 0x1C), BLOCK_TABLE_KEY)?;
    let base_blocks = blocks.len();

    let mut seen = BTreeSet::new();
    let mut out = base.to_vec();
    let rel = |len: usize| u32::try_from(len - a).context("archive larger than 4 GiB");
    for (name, data) in files {
        ensure!(seen.insert(name.to_ascii_lowercase()), "{name} named twice");
        let pos = rel(out.len())?;
        let size = u32::try_from(data.len()).context("file larger than 4 GiB")?;
        out.extend_from_slice(data);
        rel(out.len())?;
        let entry = [pos, size, size, flags::EXISTS];

        let n = name.as_bytes();
        let mask = hash_count - 1;
        let start = hash(n, HashType::TableOffset) & mask;
        let (na, nb) = (hash(n, HashType::NameA), hash(n, HashType::NameB));
        let mut found = None;
        let mut free = None;
        for i in 0..hash_count {
            let slot = ((start + i) & mask) as usize;
            let e = hashes[slot];
            if e[3] == HASH_EMPTY {
                free.get_or_insert(slot);
                break;
            }
            if e[3] == HASH_DELETED {
                free.get_or_insert(slot);
                continue;
            }
            if e[0] == na && e[1] == nb && e[2] == 0 && (e[3] as usize) < blocks.len() {
                found = Some(e[3] as usize);
                break;
            }
        }
        match found {
            Some(b) => {
                let shared = hashes
                    .iter()
                    .filter(|e| e[3] as usize == b && e[3] < HASH_DELETED)
                    .count();
                ensure!(
                    shared == 1 || b >= base_blocks,
                    "{name}: block {b} is shared by {shared} hash entries"
                );
                blocks[b] = entry;
            }
            None => {
                let slot = free.with_context(|| format!("{name}: hash table full"))?;
                hashes[slot] = [na, nb, 0, blocks.len() as u32];
                blocks.push(entry);
            }
        }
    }

    let hash_pos = rel(out.len())?;
    out.extend_from_slice(&table_bytes(&hashes, HASH_TABLE_KEY));
    let block_pos = rel(out.len())?;
    out.extend_from_slice(&table_bytes(&blocks, BLOCK_TABLE_KEY));
    let size = rel(out.len())?;
    let h = &mut out[a..a + 32];
    h[0x08..0x0C].copy_from_slice(&size.to_le_bytes());
    h[0x10..0x14].copy_from_slice(&hash_pos.to_le_bytes());
    h[0x14..0x18].copy_from_slice(&block_pos.to_le_bytes());
    h[0x1C..0x20].copy_from_slice(&(blocks.len() as u32).to_le_bytes());
    Ok(out)
}

/// The archive half of the variant check: every `(name, sha256)` of `expected` read
/// back from `variant` has that digest; every other block of `base` (by
/// block index) has the same entry and the same raw bytes in `variant`;
/// every hash slot of `base` is unchanged unless a patched name took it;
/// the extra blocks belong to patched names. Returns one line per
/// difference (empty: pass).
pub fn check_archive(
    base: &Path,
    variant: &Path,
    expected: &[(String, String)],
) -> Result<Vec<String>> {
    let (ba, va) = (Archive::open(base)?, Archive::open(variant)?);
    let read = |p: &Path| std::fs::read(p).with_context(|| format!("reading {}", p.display()));
    let (bb, vb) = (read(base)?, read(variant)?);
    let mut problems = Vec::new();
    let mut patched_base = BTreeSet::new();
    let mut patched_variant = BTreeSet::new();
    for (name, digest) in expected {
        if let Some(i) = ba.find(name) {
            patched_base.insert(i);
        }
        match va.find(name) {
            None => problems.push(format!("{name}: not in the variant archive")),
            Some(i) => {
                patched_variant.insert(i);
                match va.read(name) {
                    Ok(bytes) if sha256(&bytes) == *digest => {}
                    Ok(_) => {
                        problems.push(format!("{name}: read back differs from the compiled file"))
                    }
                    Err(e) => problems.push(format!("{name}: {e}")),
                }
            }
        }
    }
    let raw = |b: &[u8], arch: &Archive, i: usize| -> Option<Vec<u8>> {
        let e = arch.block_table()[i];
        let start = arch.header().offset as usize + e.file_pos as usize;
        b.get(start..start + e.compressed_size as usize)
            .map(<[u8]>::to_vec)
    };
    let (bt, vt) = (ba.block_table(), va.block_table());
    for (i, entry) in bt.iter().enumerate() {
        if patched_base.contains(&i) {
            continue;
        }
        if vt.get(i) != Some(entry) {
            problems.push(format!("block {i}: entry differs"));
        } else if raw(&bb, &ba, i).is_none() || raw(&bb, &ba, i) != raw(&vb, &va, i) {
            problems.push(format!("block {i}: data differs"));
        }
    }
    for i in bt.len()..vt.len() {
        if !patched_variant.contains(&i) {
            problems.push(format!("block {i}: added but no patched name uses it"));
        }
    }
    let (bh, vh) = (ba.hash_table(), va.hash_table());
    if bh.len() != vh.len() {
        problems.push("hash table size differs".into());
    } else {
        for (i, (b, v)) in bh.iter().zip(vh).enumerate() {
            if b != v && !(b.is_empty() || b.is_deleted()) {
                problems.push(format!("hash slot {i}: changed"));
            }
        }
    }
    Ok(problems)
}

// ------------------------------------------------------------ install

/// The roots of this repository's work tree that `out` must not be in:
/// the workspace this tool was built from, and any ancestor of `out` that
/// is a clone of it.
fn inside_repo(out: &Path) -> Result<Option<PathBuf>> {
    let abs = if out.is_absolute() {
        out.to_path_buf()
    } else {
        std::env::current_dir()?.join(out)
    };
    // Canonicalize the nearest existing ancestor, keep the rest.
    let mut existing = abs.as_path();
    let mut rest = Vec::new();
    while !existing.exists() {
        rest.push(existing.file_name().map(ToOwned::to_owned));
        existing = existing
            .parent()
            .context("output path has no existing ancestor")?;
    }
    let mut full = existing.canonicalize()?;
    for c in rest.into_iter().rev().flatten() {
        full.push(c);
    }
    let mut roots = Vec::new();
    if let Ok(ws) = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
    {
        roots.push(ws);
    }
    for anc in full.ancestors() {
        if anc.join(".git").exists() && anc.join("tools/data-tool/Cargo.toml").exists() {
            roots.push(anc.to_path_buf());
        }
    }
    Ok(roots.into_iter().find(|r| full.starts_with(r)))
}

/// The base install's `patch_d2.mpq` (any case).
fn find_patch_d2(dir: &Path) -> Result<PathBuf> {
    for e in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let p = e?.path();
        if p.is_file()
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(PATCH_D2))
        {
            return Ok(p);
        }
    }
    bail!("no {PATCH_D2} in {}", dir.display())
}

/// Hard-links (else copies) `from` to `to`, replacing `to`.
fn link(from: &Path, to: &Path) -> Result<()> {
    if to.symlink_metadata().is_ok() {
        std::fs::remove_file(to).with_context(|| format!("removing {}", to.display()))?;
    }
    if std::fs::hard_link(from, to).is_err() {
        std::fs::copy(from, to).with_context(|| format!("copying {}", from.display()))?;
    }
    Ok(())
}

/// One patched (or candidate) excel file.
struct BinFile {
    /// `levels.bin`.
    file: String,
    bytes: Vec<u8>,
    /// Archive the live file comes from (`patch_d2`, `d2exp`, `d2data`).
    source: String,
}

/// The `.bin` files the game reads that the stack can change (record tables
/// with a live source, the four code buffers), compiled from `data`,
/// with whether each differs from the live file.
fn candidates(
    set: &ArchiveSet,
    compiled: &d2_data::compile_set::CompiledSet,
) -> Result<Vec<(BinFile, bool)>> {
    let mut out = Vec::new();
    let mut push = |file: String, bytes: Vec<u8>| -> Result<()> {
        let live = read_excel(set, &file).with_context(|| format!("reading {file}"))?;
        let (source, differs) = match live {
            Some((src, live)) => (src.trim_end_matches(".mpq").to_owned(), live != bytes),
            None => bail!("{file}: no live file in the install"),
        };
        out.push((
            BinFile {
                file,
                bytes,
                source,
            },
            differs,
        ));
        Ok(())
    };
    for t in &compiled.tables {
        let def = schema()
            .table(&t.name)
            .context("compiled tables come from the schema")?;
        if def.live_source.is_none() {
            continue;
        }
        let mut bytes = (t.compiled.count as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(&t.compiled.records);
        push(def.bin_name.clone(), bytes)?;
    }
    for (buffer, code) in &compiled.buffers {
        push(format!("{}.bin", buffer.name()), code.clone())?;
    }
    Ok(out)
}

/// `live` with the bytes where `patched` differs from `base` taken from
/// `patched`; `None` when the three lengths are not equal.
fn overlay(live: &[u8], base: &[u8], patched: &[u8]) -> Option<Vec<u8>> {
    if live.len() != base.len() || base.len() != patched.len() {
        return None;
    }
    Some(
        live.iter()
            .zip(base.iter().zip(patched))
            .map(|(&l, (&b, &p))| if b == p { l } else { p })
            .collect(),
    )
}

/// The excel half of the variant check: the variant install loads
/// with `d2-data`, and every patched file resolves to `patch_d2` with the
/// expected digest.
fn check_install(out: &Path, tables: &[(String, String)]) -> Result<Vec<String>> {
    let set = ArchiveSet::open_dir(out).context("opening the variant archives")?;
    let mut problems = Vec::new();
    if let Err(e) = bin::load(&set, bin::DEFAULT_LANGUAGE) {
        problems.push(format!("the variant install does not load: {e}"));
    }
    for (file, digest) in tables {
        match read_excel(&set, file) {
            Ok(Some((src, bytes))) => {
                if src != PATCH_D2 {
                    problems.push(format!("{file}: resolves to {src}, not {PATCH_D2}"));
                } else if sha256(&bytes) != *digest {
                    problems.push(format!(
                        "{file}: loaded bytes differ from the compiled file"
                    ));
                }
            }
            Ok(None) => problems.push(format!("{file}: not found in the variant install")),
            Err(e) => problems.push(format!("{file}: {e}")),
        }
    }
    Ok(problems)
}

fn report(problems: &[String]) -> i32 {
    for p in problems {
        println!("error: {p}");
    }
    i32::from(!problems.is_empty())
}

fn build(stack_path: &Path, game: &Path, out: Option<PathBuf>) -> Result<i32> {
    let name = stack_path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("stack file name")?
        .to_owned();
    ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "variant name {name:?} must match [a-z0-9-]+"
    );
    ensure!(
        stack_path.extension().and_then(|e| e.to_str()) == Some("d2stack"),
        "{} is not a .d2stack file",
        stack_path.display()
    );
    let parent = stack_path.parent().and_then(Path::file_name);
    ensure!(
        parent.and_then(|p| p.to_str()) == Some(name.as_str()),
        "the stack must be traces/variants/{name}/{name}.d2stack"
    );
    let out = match out {
        Some(o) => o,
        None => game
            .parent()
            .context("game dir has no parent")?
            .join("variants")
            .join(&name),
    };
    if let Some(root) = inside_repo(&out)? {
        bail!(
            "{} is inside the repository work tree {} (variant installs are game files)",
            out.display(),
            root.display()
        );
    }
    if out.exists() {
        let out_c = out.canonicalize()?;
        ensure!(out_c != game, "the output dir is the game dir");
        let empty = std::fs::read_dir(&out)?.next().is_none();
        ensure!(
            empty || out.join(JSON).is_file(),
            "{} is not empty and holds no {JSON}",
            out.display()
        );
    }

    // Apply and compile.
    let sp = stack_path.to_str().context("stack path is not UTF-8")?;
    let (layers, mut findings) = crate::patch::stack(sp)?;
    if has_errors(&findings) {
        patch::sort_report(&mut findings);
        crate::patch::print(&findings);
        return Ok(1);
    }
    let set = ArchiveSet::open_dir(game).context("opening the game archives")?;
    let base = match crate::patch::base(&set) {
        Ok(b) => b,
        Err(f) => {
            crate::patch::print(&f);
            return Ok(1);
        }
    };
    let mut data = base.clone();
    findings.extend(apply_stack(&mut data, &layers, sp));
    let mut compiled = None;
    let mut base_compiled = None;
    if !has_errors(&findings) {
        let live = bin::load(&set, bin::DEFAULT_LANGUAGE).context("loading the live set")?;
        let r = compile_patched(&base, &data, &live);
        findings.extend(r.findings);
        compiled = r.compiled;
        base_compiled = compile_patched(&base, &base, &live).compiled;
    }
    patch::sort_report(&mut findings);
    crate::patch::print(&findings);
    let compiled = match compiled {
        Some(c) if !has_errors(&findings) => c,
        _ => return Ok(1),
    };

    // The patched set: files the stack changes. Where the
    // base compile differs from the live file (an explained difference of
    // `data-tool tables`), the variant file is the live file with only the
    // bytes the stack changes replaced; with a different size that is
    // impossible and the build stops.
    let Some(base_compiled) = base_compiled else {
        eprintln!("error: the base tables do not compile");
        return Ok(1);
    };
    let base_files: BTreeMap<String, (Vec<u8>, bool)> = candidates(&set, &base_compiled)?
        .into_iter()
        .map(|(f, differs)| (f.file, (f.bytes, differs)))
        .collect();
    let mut patched: Vec<BinFile> = Vec::new();
    let mut inexact = Vec::new();
    for (mut f, _) in candidates(&set, &compiled)? {
        let Some((base_bytes, base_differs)) = base_files.get(&f.file) else {
            continue;
        };
        if *base_bytes == f.bytes {
            continue;
        }
        if *base_differs {
            let live = read_excel(&set, &f.file)?
                .map(|(_, b)| b)
                .unwrap_or_default();
            match overlay(&live, base_bytes, &f.bytes) {
                Some(b) => f.bytes = b,
                None => inexact.push(f.file.clone()),
            }
        }
        patched.push(f);
    }
    if !inexact.is_empty() {
        eprintln!(
            "error: {}: the base compile differs from the live file and the patch changes the size; a variant would not be exact",
            inexact.join(", ")
        );
        return Ok(1);
    }
    if patched.is_empty() {
        println!("note: the stack changes no compiled .bin; the variant equals the base");
    }
    for f in &patched {
        println!("patched {} (live from {})", f.file, f.source);
    }

    // Write the install.
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let base_p = find_patch_d2(game)?;
    for e in std::fs::read_dir(game)? {
        let p = e?.path();
        let Some(fname) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !p.is_file() || fname.eq_ignore_ascii_case(PATCH_D2) || fname == JSON {
            continue;
        }
        link(&p, &out.join(fname))?;
    }
    let base_bytes =
        std::fs::read(&base_p).with_context(|| format!("reading {}", base_p.display()))?;
    let files: Vec<(String, Vec<u8>)> = patched
        .iter()
        .map(|f| (excel_path(&f.file), f.bytes.clone()))
        .collect();
    let variant_bytes = patch_archive(&base_bytes, &files)?;
    let var_p = out.join(base_p.file_name().context("patch_d2 name")?);
    if var_p.symlink_metadata().is_ok() {
        std::fs::remove_file(&var_p)?;
    }
    std::fs::write(&var_p, &variant_bytes)
        .with_context(|| format!("writing {}", var_p.display()))?;

    // variant.json, then the variant check.
    let mut stack_digests = vec![serde_json::json!({
        "file": stack_path.file_name().and_then(|n| n.to_str()),
        "sha256": sha256(&std::fs::read(stack_path)?),
    })];
    let dir = stack_path.parent().unwrap_or(Path::new(""));
    for l in &layers {
        stack_digests.push(serde_json::json!({
            "file": l.path,
            "sha256": sha256(&std::fs::read(dir.join(&l.path))?),
        }));
    }
    let tables: Vec<serde_json::Value> = patched
        .iter()
        .map(|f| {
            serde_json::json!({
                "table": f.file.trim_end_matches(".bin"),
                "bin_sha256": sha256(&f.bytes),
                "source": f.source,
            })
        })
        .collect();
    let json = serde_json::json!({
        "format": "test-variant",
        "version": 1,
        "name": name,
        "stack_sha256": stack_digests,
        "base_patch_d2_sha256": sha256(&base_bytes),
        "variant_patch_d2_sha256": sha256(&variant_bytes),
        "tables": tables,
        "tool": format!("data-tool {}", env!("CARGO_PKG_VERSION")),
    });
    std::fs::write(out.join(JSON), serde_json::to_string_pretty(&json)? + "\n")?;

    let code = check(&out, game)?;
    if code == 0 {
        println!(
            "wrote {} ({} table(s) patched)",
            out.display(),
            patched.len()
        );
    }
    Ok(code)
}

fn check(out: &Path, game: &Path) -> Result<i32> {
    let text = std::fs::read_to_string(out.join(JSON))
        .with_context(|| format!("reading {}", out.join(JSON).display()))?;
    let v: serde_json::Value = serde_json::from_str(&text).context("parsing variant.json")?;
    ensure!(
        v["format"] == "test-variant" && v["version"] == 1,
        "variant.json: not a version-1 test-variant file"
    );
    let str_of = |v: &serde_json::Value, k: &str| -> Result<String> {
        v[k].as_str()
            .map(str::to_owned)
            .with_context(|| format!("variant.json: no {k}"))
    };
    let mut tables = Vec::new();
    for t in v["tables"].as_array().context("variant.json: no tables")? {
        tables.push((
            format!("{}.bin", str_of(t, "table")?),
            str_of(t, "bin_sha256")?,
        ));
    }
    let base_p = find_patch_d2(game)?;
    let var_p = find_patch_d2(out)?;
    let mut problems = Vec::new();
    if sha256(&std::fs::read(&base_p)?) != str_of(&v, "base_patch_d2_sha256")? {
        problems.push(format!(
            "{}: not the base this variant was built from",
            base_p.display()
        ));
    }
    if sha256(&std::fs::read(&var_p)?) != str_of(&v, "variant_patch_d2_sha256")? {
        problems.push(format!("{}: changed since the build", var_p.display()));
    }
    let names: Vec<(String, String)> = tables
        .iter()
        .map(|(f, d)| (excel_path(f), d.clone()))
        .collect();
    problems.extend(check_archive(&base_p, &var_p, &names)?);
    problems.extend(check_install(out, &tables)?);
    let code = report(&problems);
    if code == 0 {
        println!("check ok: {} table(s)", tables.len());
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::mpq::writer::{FileOptions, MpqWriter};

    #[test]
    fn overlay_keeps_live_bytes_the_patch_does_not_change() {
        let live = [1, 9, 3, 4];
        let base = [1, 2, 3, 4];
        assert_eq!(overlay(&live, &base, &[1, 2, 7, 4]), Some(vec![1, 9, 7, 4]));
        assert_eq!(overlay(&live, &base, &base), Some(live.to_vec()));
        assert_eq!(overlay(&live, &base, &[1, 2, 3]), None);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("variant-unit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn data(n: usize, seed: u8) -> Vec<u8> {
        (0..n)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    /// A base archive with compressed, encrypted and stored files, no
    /// listfile (like `patch_d2.mpq`).
    fn base_writer() -> MpqWriter {
        let mut w = MpqWriter::new().hash_table_count(16);
        w.add_file("data\\global\\excel\\monstats.bin", data(9000, 1));
        w.add(
            "data\\global\\excel\\levels.bin",
            data(5000, 2),
            FileOptions::stored(),
        );
        w.add(
            "data\\local\\x.wav",
            data(3000, 3),
            FileOptions::encrypted_fix_key(),
        );
        w.add_deleted("data\\gone.txt");
        w
    }

    fn files(list: &[(&str, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
        list.iter()
            .map(|(n, d)| ((*n).to_owned(), d.clone()))
            .collect()
    }

    fn digests(f: &[(String, Vec<u8>)]) -> Vec<(String, String)> {
        f.iter().map(|(n, d)| (n.clone(), sha256(d))).collect()
    }

    fn write_pair(tag: &str, base: &[u8], var: &[u8]) -> (PathBuf, PathBuf) {
        let d = tmp(tag);
        let (b, v) = (d.join("base.mpq"), d.join("var.mpq"));
        std::fs::write(&b, base).unwrap();
        std::fs::write(&v, var).unwrap();
        (b, v)
    }

    #[test]
    fn replace_and_add_read_back() {
        let base = base_writer().to_bytes().unwrap();
        let f = files(&[
            ("data\\global\\excel\\monstats.bin", data(12345, 9)),
            ("data\\global\\excel\\skills.bin", data(4097, 7)),
        ]);
        let var = patch_archive(&base, &f).unwrap();
        // Every base byte except the header is kept.
        assert_eq!(var[0x20..base.len()], base[0x20..]);
        let (b, v) = write_pair("replace", &base, &var);
        assert_eq!(
            check_archive(&b, &v, &digests(&f)).unwrap(),
            Vec::<String>::new()
        );
        let va = Archive::open(&v).unwrap();
        let ba = Archive::open(&b).unwrap();
        assert_eq!(
            va.read("data\\global\\excel\\monstats.bin").unwrap(),
            f[0].1
        );
        assert_eq!(va.read("data\\global\\excel\\skills.bin").unwrap(), f[1].1);
        assert_eq!(va.block_table().len(), ba.block_table().len() + 1);
        assert_eq!(va.header().archive_size as usize, var.len());
        for n in ["data\\global\\excel\\levels.bin", "data\\local\\x.wav"] {
            assert_eq!(va.read(n).unwrap(), ba.read(n).unwrap(), "{n}");
        }
    }

    #[test]
    fn new_name_takes_first_free_probe_slot() {
        let count = 16u32;
        let new = "data\\global\\excel\\monprop.bin";
        let start = hash(new.as_bytes(), HashType::TableOffset) & (count - 1);
        // Find base names whose first probe slot is the new name's.
        let mut w = MpqWriter::new().hash_table_count(count);
        let mut taken = Vec::new();
        for i in 0..10_000 {
            let n = format!("data\\f{i}.bin");
            if hash(n.as_bytes(), HashType::TableOffset) & (count - 1) == start {
                taken.push(n);
                if taken.len() == 2 {
                    break;
                }
            }
        }
        for (k, n) in taken.iter().enumerate() {
            w.add_file(n, data(700, k as u8));
        }
        let base = w.to_bytes().unwrap();
        let f = files(&[(new, data(100, 5))]);
        let var = patch_archive(&base, &f).unwrap();
        let (b, v) = write_pair("probe", &base, &var);
        assert!(check_archive(&b, &v, &digests(&f)).unwrap().is_empty());
        let va = Archive::open(&v).unwrap();
        assert_eq!(va.read(new).unwrap(), f[0].1);
        // The two colliding names hold start and start+1; the new one is next.
        let slot = (start as usize + 2) & (count as usize - 1);
        assert_eq!(va.hash_table()[slot].block_index, 2);
        for n in &taken {
            assert!(va.read(n).is_ok());
        }
    }

    #[test]
    fn empty_patch_list_is_identity() {
        let base = base_writer().to_bytes().unwrap();
        let var = patch_archive(&base, &[]).unwrap();
        assert_eq!(var, base);
        let (b, v) = write_pair("empty", &base, &var);
        assert!(check_archive(&b, &v, &[]).unwrap().is_empty());
    }

    #[test]
    fn perturbed_append_is_reported_for_that_file_only() {
        let base = base_writer().to_bytes().unwrap();
        let f = files(&[
            ("data\\global\\excel\\monstats.bin", data(600, 9)),
            ("data\\global\\excel\\skills.bin", data(300, 7)),
        ]);
        let mut var = patch_archive(&base, &f).unwrap();
        // skills.bin's data starts right after monstats.bin's.
        var[base.len() + 600 + 10] ^= 0x40;
        let (b, v) = write_pair("perturb", &base, &var);
        let p = check_archive(&b, &v, &digests(&f)).unwrap();
        assert_eq!(
            p,
            vec![
                "data\\global\\excel\\skills.bin: read back differs from the compiled file"
                    .to_owned()
            ]
        );
        // Flipping a byte of an untouched block (1, levels.bin) is
        // reported by its index.
        let pos = Archive::open(&b).unwrap().block_table()[1].file_pos as usize;
        let mut var2 = patch_archive(&base, &f).unwrap();
        var2[pos + 3] ^= 1;
        let (b, v) = write_pair("perturb2", &base, &var2);
        assert_eq!(
            check_archive(&b, &v, &digests(&f)).unwrap(),
            vec!["block 1: data differs".to_owned()]
        );
    }

    #[test]
    fn full_hash_table_and_duplicates_are_errors() {
        let mut w = MpqWriter::new().hash_table_count(2);
        w.add_file("a", data(10, 1));
        w.add_file("b", data(10, 2));
        let base = w.to_bytes().unwrap();
        assert!(patch_archive(&base, &files(&[("c", vec![1])])).is_err());
        let base = base_writer().to_bytes().unwrap();
        assert!(patch_archive(&base, &files(&[("x", vec![1]), ("X", vec![2])])).is_err());
    }

    #[test]
    fn repo_tree_is_refused() {
        let ws = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(inside_repo(&ws.join("target/variants/x"))
            .unwrap()
            .is_some());
        assert!(inside_repo(&std::env::temp_dir().join("v"))
            .unwrap()
            .is_none());
    }
}
