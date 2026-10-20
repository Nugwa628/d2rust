//! Data-table tool.
//!
//! Usage (run with --release):
//!   data-tool tables [game_dir]
//!   data-tool cov-records [game_dir]
//!   data-tool gen-tables
//!   data-tool gen-proto
//!   data-tool links [game_dir]
//!   data-tool dump-compare <dump_dir> [game_dir]
//!   data-tool excel-dir <out_dir> [game_dir]
//!   data-tool patch (check | render | diff) ...
//!   data-tool variant build <traces/variants/<name>/<name>.d2stack> [--game DIR] [--out DIR]
//!   data-tool variant check <out_dir> [--game DIR]
//!
//! `tables` loads and validates every live `.bin` (73 record tables, 4 code
//! buffers, `hitclass`), compiles every table's highest-priority `.txt` in
//! load order and compares them byte for byte. Exit status 1 when a
//! runtime table or code buffer differs in a way no known load step
//! explains.
//!
//! `gen-tables` regenerates `crates/d2-data/src/tables/generated.rs` (the
//! typed record structs) from the embedded schema.
//!
//! `gen-proto` regenerates `crates/d2-proto/src/generated.rs` (message
//! descriptors and typed messages) from `tables/sim/client-messages.tsv`
//! and `server-messages.tsv`.
//!
//! `links` checks every lookup field of the live set against its linker's
//! key count and prints each broken link with
//! table, row and column. Exit status 1 when a link is broken or a linker
//! size is unknown.
//!
//! `dump-compare` compares a memory dump of the tables the game loaded
//! (`tools/trace-recorder/dump_tables.py`, `traces/raw/<time>-tables/`)
//! with the live `.bin` set after `d2_data::fixup`. Exit status 1 when a
//! byte differs outside a pointer
//! field and outside the rows `fixup::PENDING` lists.
//!
//! `excel-dir` writes the live excel set (every `data\\global\\excel\\`
//! file, each from the highest-priority archive that has it) into
//! `<out_dir>` under lowercase names: the folder the `#[ignore]`
//! game-file tests read as `$D2_GAME_DIR/extracted/patch_d2/data/global/excel/`.
//! Names: the archives' `(listfile)` excel entries, the `.bin` twin of each
//! `.txt`, every schema table's `.txt` / `.bin` and the code buffers.
//!
//! `patch` checks a mod stack, renders patched tables or diffs an edited
//! table into a layer.
//!
//! `variant build` applies a test-variant stack (`traces/variants/`) to
//! the install's tables, compiles them and writes a variant install (the
//! install's files hard-linked, `patch_d2.mpq` rewritten with the patched
//! `.bin` files appended) to `$D2_GAME_DIR/../variants/<name>/` or
//! `--out`, never inside the repository; `variant check` re-checks one
//! (details in `src/variant.rs`).

mod patch;
mod variant;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use d2_data::crosscheck::{self, CrossCheck, Role, TableReport};
use d2_data::fixup::{self, FixedSet};
use d2_data::{bin, links as xref};
use d2_formats::mpq::ArchiveSet;

fn game_dir(arg: Option<&String>) -> Result<PathBuf> {
    Ok(match arg {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var("D2_GAME_DIR").context("set D2_GAME_DIR")?),
    })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("tables") if args.len() <= 2 => tables(&game_dir(args.get(1))?),
        Some("cov-records") if args.len() <= 2 => cov_records(&game_dir(args.get(1))?),
        Some("gen-tables") if args.len() == 1 => gen_tables(),
        Some("gen-proto") if args.len() == 1 => gen_proto(),
        Some("links") if args.len() <= 2 => links(&game_dir(args.get(1))?),
        Some("dump-compare") if (2..=3).contains(&args.len()) => {
            dump_compare(Path::new(&args[1]), &game_dir(args.get(2))?)
        }
        Some("excel-dir") if (2..=3).contains(&args.len()) => {
            excel_dir(Path::new(&args[1]), &game_dir(args.get(2))?)
        }
        Some("patch") => std::process::exit(patch::main(&args[1..])),
        Some("variant") => std::process::exit(variant::main(&args[1..])),
        _ => bail!(
            "usage: data-tool tables|links [game_dir] | data-tool gen-tables | data-tool gen-proto | \
             data-tool dump-compare <dump_dir> [game_dir] | data-tool excel-dir <out_dir> [game_dir] | \
             data-tool patch ... | data-tool variant (build | check) ..."
        ),
    }
}

fn excel_dir(out: &Path, game: &Path) -> Result<()> {
    let set = ArchiveSet::open_dir(game).with_context(|| format!("opening {}", game.display()))?;
    let mut names = std::collections::BTreeSet::new();
    for a in set.archives() {
        for n in a.listfile()?.unwrap_or_default() {
            let n = n.to_ascii_lowercase();
            if let Some(f) = n.strip_prefix(bin::EXCEL_DIR) {
                if !f.is_empty() && !f.contains('\\') {
                    names.insert(f.to_owned());
                }
            }
        }
    }
    for t in &d2_data::schema::schema().tables {
        for f in [&t.txt_name, &t.bin_name] {
            if !f.is_empty() {
                names.insert(f.to_ascii_lowercase());
            }
        }
    }
    for b in d2_data::schema::CalcBuffer::ALL {
        names.insert(format!("{}.bin", b.name()));
    }
    let twins: Vec<String> = names
        .iter()
        .filter_map(|n| n.strip_suffix(".txt").map(|s| format!("{s}.bin")))
        .collect();
    names.extend(twins);
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let mut per_archive: BTreeMap<String, usize> = BTreeMap::new();
    for n in &names {
        if let Some((src, bytes)) = set.read_with_source(&bin::excel_path(n))? {
            std::fs::write(out.join(n), bytes)?;
            *per_archive.entry(src).or_default() += 1;
        }
    }
    let written: usize = per_archive.values().sum();
    println!(
        "excel-dir: {written} of {} candidate names written to {} ({per_archive:?})",
        names.len(),
        out.display()
    );
    Ok(())
}

/// `cov-records`: one line per runtime or by-product table: `<table>\t<records>\t<n differing>` and
/// one `diff\t<table>\t<record>\t<field labels, '|'-joined; '~' = explained>` line per
/// differing record (compiled text vs shipped `.bin`; `tools/coord/cov_tables.py`).
fn cov_records(dir: &Path) -> Result<()> {
    let set = ArchiveSet::open_dir(dir).with_context(|| format!("opening {}", dir.display()))?;
    let report = crosscheck::run(&set).context("cross-check")?;
    for t in report.tables.iter().filter(|t| t.role != Role::ClientOnly) {
        println!(
            "table\t{}\t{}\t{}\t{}",
            t.name,
            t.records_txt,
            t.records_bin.map_or(-1, |n| n as i64),
            t.record_diffs.len()
        );
        for (r, f) in &t.record_diffs {
            println!("diff\t{}\t{}\t{}", t.name, r, f.join("|"));
        }
    }
    Ok(())
}

fn gen_tables() -> Result<()> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/d2-data")
        .join(d2_data::codegen::GENERATED_PATH);
    let code = d2_data::codegen::generate(d2_data::schema::schema());
    std::fs::write(&path, &code).with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {} ({} bytes)", path.display(), code.len());
    Ok(())
}

fn gen_proto() -> Result<()> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/d2-proto")
        .join(d2_proto::codegen::GENERATED_PATH);
    let code = d2_proto::codegen::generate(d2_proto::tsv::CLIENT_TSV, d2_proto::tsv::SERVER_TSV)?;
    std::fs::write(&path, &code).with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {} ({} bytes)", path.display(), code.len());
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn short(archive: &str) -> &str {
    archive.strip_suffix(".mpq").unwrap_or(archive)
}

fn status(t: &TableReport) -> String {
    if let Some(note) = &t.note {
        return format!("not compared: {note}");
    }
    if !t.counts_equal() {
        return format!(
            "MISMATCH: {} records from txt, {} in bin",
            t.records_txt,
            t.records_bin.unwrap_or(0)
        );
    }
    if t.identical() {
        return "identical".into();
    }
    let explained: usize = t.explained.values().map(|(bytes, _)| bytes).sum();
    if t.matches() {
        return format!("explained ({explained} bytes)");
    }
    let bytes: usize = t.mismatches.values().map(|d| d.bytes).sum();
    format!("MISMATCH: {} fields, {bytes} bytes", t.mismatches.len())
}

fn print_details(t: &TableReport) {
    for (reason, (bytes, records)) in &t.explained {
        println!("      explained: {bytes} bytes in {records} records: {reason}");
    }
    for (field, d) in &t.mismatches {
        println!(
            "      field {field}: {} records, {} bytes",
            d.records, d.bytes
        );
        for e in &d.examples {
            println!(
                "        record {} +{}: bin [{}] txt [{}]",
                e.record,
                e.offset,
                hex(&e.shipped),
                hex(&e.compiled)
            );
        }
    }
}

fn print_group(report: &CrossCheck, role: Role, title: &str) {
    let rows: Vec<&TableReport> = report.tables.iter().filter(|t| t.role == role).collect();
    if rows.is_empty() {
        return;
    }
    println!("\n{title}");
    println!(
        "  {:<18} {:<9} {:<9} {:>7}  result",
        "table", "txt", "bin", "records"
    );
    for t in rows {
        let bin = t.bin_source.as_deref().map(short).unwrap_or("-");
        let moved = match (&t.bin_source, &t.expected_source) {
            (Some(a), Some(b)) if a != b => format!(" (tables.tsv: {})", short(b)),
            _ => String::new(),
        };
        println!(
            "  {:<18} {:<9} {:<9} {:>7}  {}{moved}",
            t.name,
            short(&t.txt_source),
            bin,
            t.records_txt,
            status(t)
        );
        print_details(t);
    }
}

fn tables(dir: &std::path::Path) -> Result<()> {
    let set = ArchiveSet::open_dir(dir).with_context(|| format!("opening {}", dir.display()))?;
    let report = crosscheck::run(&set).context("cross-check")?;
    println!(
        "live .bin set: 73 record tables, 4 code buffers and hitclass loaded and validated \
         (loading.md §4.2, §8, §10.8)"
    );

    print_group(&report, Role::Runtime, "runtime tables (loading.md §6)");
    print_group(
        &report,
        Role::ClientOnly,
        "client-only table (loading.md §3.5)",
    );
    print_group(
        &report,
        Role::ByProduct,
        "compile-only by-products (loading.md §7.2)",
    );

    println!("\ncode buffers (loading.md §4.3)");
    for b in &report.buffers {
        let r = &report.code_reports[&b.buffer];
        let result = match b.first_difference {
            None => "identical".to_string(),
            Some(at) => format!("MISMATCH: first difference at byte {at}"),
        };
        println!(
            "  {:<14} {:<9} bin {:>5} bytes, txt {:>5} bytes  {result}; {} expressions, {} unreferenced, {} shared, {} PAREN, {} unknown CALL",
            b.buffer.name(),
            short(&b.source),
            b.shipped,
            b.compiled,
            r.starts.len(),
            r.unreferenced,
            r.shared,
            r.paren,
            r.unknown_call
        );
    }

    let runtime: Vec<&TableReport> = report
        .tables
        .iter()
        .filter(|t| t.role == Role::Runtime)
        .collect();
    let identical = runtime.iter().filter(|t| t.identical()).count();
    let explained = runtime
        .iter()
        .filter(|t| t.matches() && !t.identical())
        .count();
    let bad = runtime.len() - identical - explained;
    let buffers_ok = report.buffers.iter().filter(|b| b.identical()).count();
    println!(
        "\nsummary: {} runtime tables: {identical} identical, {explained} explained, {bad} mismatched; \
         code buffers: {buffers_ok}/{} identical",
        runtime.len(),
        report.buffers.len()
    );

    let diags: Vec<String> = report
        .diagnostics
        .iter()
        .map(|(k, n)| format!("{k:?} {n}"))
        .collect();
    println!(
        "compiler diagnostics (all compiled lists): {}",
        diags.join(", ")
    );
    let calc: Vec<String> = report
        .calc_diagnostics
        .iter()
        .map(|((b, k), n)| format!("{} {k:?} {n}", b.name()))
        .collect();
    println!("calc diagnostics: {}", calc.join(", "));
    let cbs: Vec<String> = report
        .unspecified_callbacks
        .iter()
        .map(|(c, n)| format!("cb({c}) {n}"))
        .collect();
    if !cbs.is_empty() {
        println!(
            "unknown table callbacks (wrote nothing): {}",
            cbs.join(", ")
        );
    }

    if bad > 0 || buffers_ok != report.buffers.len() {
        std::process::exit(1);
    }
    Ok(())
}

fn links(dir: &std::path::Path) -> Result<()> {
    let set = ArchiveSet::open_dir(dir).with_context(|| format!("opening {}", dir.display()))?;
    let data = bin::load(&set, bin::DEFAULT_LANGUAGE).context("loading the live .bin set")?;
    let lookups = xref::load_lookups(&set).context("loading lookup by-products")?;
    let (sizes, report) = xref::validate_set(&data, &lookups);
    println!("linker sizes (field-types.md §6.7)");
    for (linker, n) in sizes.iter() {
        println!("  {linker:<24} {n:>6}");
    }
    for f in &report.unchecked {
        println!(
            "UNCHECKED: {} column `{}`: size of {} unknown",
            f.table, f.column, f.linker
        );
    }
    for b in &report.broken {
        println!("BROKEN: {b}");
    }
    println!(
        "\nsummary: {} valid, {} misses (-1), {} broken, {} unchecked fields",
        report.valid,
        report.misses,
        report.broken.len(),
        report.unchecked.len()
    );
    if !report.is_clean() || !report.unchecked.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

// --- dump-compare ------------------------------------------------------------

/// Record fields that hold in-memory pointers in the game (set at load time;
/// they change from run to run), as (table, offset, width, what). They are
/// excluded from the comparison.
const POINTER_FIELDS: &[(&str, usize, usize, &str)] = &[(
    "sets",
    0x110,
    24,
    "6 pointers to the attached setitems records",
)];

/// Where the `fixup::PENDING` rows write: (pending row, table, record
/// byte range). Bytes the game changes outside these ranges are reported
/// as differences. Empty: every load fix-up row is implemented.
type PendingTarget = (&'static str, &'static str, (usize, usize));
const PENDING_TARGETS: &[PendingTarget] = &[];

struct DumpEntry {
    kind: String,
    address: String,
    count: usize,
    size: usize,
}

fn read_manifest(dir: &Path) -> Result<BTreeMap<String, DumpEntry>> {
    let path = dir.join("manifest.tsv");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut out = BTreeMap::new();
    for line in text.lines().skip(1) {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() != 5 {
            bail!("{}: bad line `{line}`", path.display());
        }
        out.insert(
            c[1].to_owned(),
            DumpEntry {
                kind: c[0].to_owned(),
                address: c[2].to_owned(),
                count: c[3].parse()?,
                size: c[4].parse()?,
            },
        );
    }
    Ok(out)
}

/// The `fields.tsv` field(s) whose footprint holds byte `offset`.
fn field_label(table: &str, offset: usize) -> String {
    let names: Vec<String> = d2_data::schema::schema()
        .runtime()
        .find(|t| t.name == table)
        .map(|d| {
            d.fields
                .iter()
                .filter(|f| f.footprint().contains(&offset))
                .map(|f| f.name())
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        "(no field)".into()
    } else {
        names.join("|")
    }
}

fn pointer_at(table: &str, offset: usize) -> bool {
    POINTER_FIELDS
        .iter()
        .any(|(t, o, w, _)| *t == table && (*o..*o + *w).contains(&offset))
}

fn pending_at(table: &str, offset: usize) -> bool {
    PENDING_TARGETS
        .iter()
        .any(|(_, t, (a, b))| *t == table && (*a..*b).contains(&offset))
}

#[derive(Default)]
struct Group {
    records: usize,
    bytes: usize,
    /// Record offsets of the differing bytes.
    span: Option<(usize, usize)>,
    last_record: Option<usize>,
    examples: Vec<(usize, usize, Vec<u8>, Vec<u8>)>,
}

/// Bytes where D2RUST and the dump differ, by (pending, field label).
#[derive(Default)]
struct TableDiff {
    groups: BTreeMap<(bool, String), Group>,
    /// Bytes the game's fix-ups changed from the shipped `.bin`.
    game_changed: usize,
    /// Bytes `d2_data::fixup` changed.
    ours_changed: usize,
    /// Pointer-field bytes that differ from the shipped `.bin` (ignored).
    pointer_bytes: usize,
}

fn diff_table(name: &str, size: usize, ours: &[u8], game: &[u8], shipped: &[u8]) -> TableDiff {
    let mut d = TableDiff::default();
    let records = ours
        .chunks_exact(size)
        .zip(game.chunks_exact(size))
        .zip(shipped.chunks_exact(size));
    for (r, ((o, g), s)) in records.enumerate() {
        let mut k = 0;
        while k < size {
            if pointer_at(name, k) {
                d.pointer_bytes += usize::from(g[k] != s[k]);
                k += 1;
                continue;
            }
            d.game_changed += usize::from(g[k] != s[k]);
            d.ours_changed += usize::from(o[k] != s[k]);
            if o[k] == g[k] {
                k += 1;
                continue;
            }
            let label = field_label(name, k);
            let start = k;
            k += 1;
            while k < size && o[k] != g[k] && !pointer_at(name, k) && field_label(name, k) == label
            {
                d.game_changed += usize::from(g[k] != s[k]);
                d.ours_changed += usize::from(o[k] != s[k]);
                k += 1;
            }
            let grp = d
                .groups
                .entry((pending_at(name, start), label))
                .or_default();
            grp.bytes += k - start;
            let (lo, hi) = grp.span.unwrap_or((start, k));
            grp.span = Some((lo.min(start), hi.max(k)));
            if grp.last_record != Some(r) {
                grp.records += 1;
                grp.last_record = Some(r);
            }
            if grp.examples.len() < 3 {
                grp.examples
                    .push((r, start, o[start..k].to_vec(), g[start..k].to_vec()));
            }
        }
    }
    d
}

fn u16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn u32s(v: impl IntoIterator<Item = u32>) -> Vec<u8> {
    v.into_iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// A pointer word in D2RUST form: a record index, or `u32::MAX` for none.
fn ptr(v: Option<u32>) -> u32 {
    v.unwrap_or(u32::MAX)
}

/// The D2RUST counterpart of dumped map `name`, with the
/// table its pointer words point into and which u32 words are pointers.
type PointerWords = (&'static str, fn(usize) -> bool);
fn our_map(name: &str, f: &FixedSet) -> Option<(Vec<u8>, Option<PointerWords>)> {
    let bytes = match name {
        "superunique_hc" => u16s(
            &f.superunique_hc
                .iter()
                .map(|v| v.unwrap_or(0xFFFF))
                .collect::<Vec<_>>(),
        ),
        "stat_stuff" => u32s([f.stat_stuff, f.stat_mask]),
        "itemtypes_equiv" => u32s(f.itemtypes_equiv.bits.iter().copied()),
        "montype_equiv" => u32s(f.montype_equiv.bits.iter().copied()),
        "isc_desc_list" => u16s(&f.stat_desc_list),
        "states_bitsets" => u32s(f.states.bitsets.iter().copied()),
        "states_pgsv" => u16s(&f.states.pgsv),
        "states_curse" => u16s(&f.states.curse),
        "states_disguise" => u16s(&f.states.disguise),
        "states_active" => u16s(&f.states.active),
        "states_itemtype" => u16s(&f.states.itemtype),
        "skills_class_counts" => u32s(f.skill_lists.counts),
        "skills_class_lists" => u16s(&f.skill_lists.lists),
        "skills_desc_list" => u16s(&f.skill_lists.passives),
        "items_f6_list" => u16s(&f.version0_items),
        "gamble_index" => u32s(f.gamble.index.iter().flatten().copied()),
        "gamble_levels" => u32s(f.gamble.thresholds),
        "automap_runtime" => f.automap.records.concat(),
        "automap_level_index" => f
            .automap
            .ranges
            .iter()
            .flat_map(|&(a, b)| [a, b])
            .flat_map(i32::to_le_bytes)
            .collect(),
        "hireling_first" => f
            .hireling_first
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect(),
        "leveldefs_portals" => u32s(f.portals.iter().copied()),
        "lvlsub_type_first" => u32s(f.lvlsub_types.iter().copied()),
        "monseq_index" => {
            let words = f
                .monseq
                .iter()
                .flat_map(|e| [ptr(e.first), e.count, e.count2]);
            return Some((u32s(words), Some(("monseq", |w| w % 3 == 0))));
        }
        "monpreset_acts" => {
            let words = f
                .monpreset
                .first
                .iter()
                .map(|&v| ptr(v))
                .chain(f.monpreset.count);
            return Some((u32s(words), Some(("monpreset", |w| w < 5))));
        }
        _ => return None,
    };
    Some((bytes, None))
}

/// Dumped pointer words → record indices of `table` (null → `u32::MAX`;
/// a value outside the table stays as it is and so differs).
fn pointers_to_indices(
    data: &mut [u8],
    words: PointerWords,
    manifest: &BTreeMap<String, DumpEntry>,
) -> Result<()> {
    let (table, is_ptr) = words;
    let e = manifest
        .get(table)
        .with_context(|| format!("{table} not in the dump manifest"))?;
    let base = u32::from_str_radix(e.address.trim_start_matches("0x"), 16)
        .with_context(|| format!("{table} address {}", e.address))?;
    let (size, end) = (e.size as u32, base + (e.count * e.size) as u32);
    for (w, chunk) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        if !is_ptr(w) {
            continue;
        }
        let v = u32::from_le_bytes(*chunk);
        let index = if v == 0 {
            u32::MAX
        } else if (base..end).contains(&v) && (v - base) % size == 0 {
            (v - base) / size
        } else {
            v
        };
        *chunk = index.to_le_bytes();
    }
    Ok(())
}

/// Maps D2RUST builds, compared with the dumped ones; the rest are listed.
fn compare_maps(
    dir: &Path,
    fixed: &FixedSet,
    manifest: &BTreeMap<String, DumpEntry>,
) -> Result<bool> {
    let mut ok = true;
    println!("\nruntime maps (map-<name>.bin; pointer words compared as record indices)");
    for (name, e) in manifest.iter().filter(|(_, e)| e.kind == "map") {
        let mut data = std::fs::read(dir.join(format!("map-{name}.bin"))).unwrap_or_default();
        let result = match our_map(name, fixed) {
            None => "no D2RUST counterpart".to_string(),
            Some((o, pointers)) => {
                if let Some(p) = pointers {
                    pointers_to_indices(&mut data, p, manifest)?;
                }
                if data == o {
                    "identical".into()
                } else {
                    ok = false;
                    let first = o.iter().zip(&data).position(|(a, b)| a != b);
                    match first {
                        Some(at) => {
                            let w = &o[at..o.len().min(at + 8)];
                            let g = &data[at..data.len().min(at + 8)];
                            format!(
                                "DIFFERS at byte {at}: D2RUST [{}] game [{}] ({} vs {} bytes)",
                                hex(w),
                                hex(g),
                                o.len(),
                                data.len()
                            )
                        }
                        None => {
                            format!("DIFFERS in length: D2RUST {}, game {}", o.len(), data.len())
                        }
                    }
                }
            }
        };
        println!(
            "  {name:<22} {:>10} {:>6} x {:<2} {result}",
            e.address, e.count, e.size
        );
    }
    Ok(ok)
}

fn dump_compare(dir: &Path, game: &Path) -> Result<()> {
    let manifest = read_manifest(dir)?;
    let set = ArchiveSet::open_dir(game).with_context(|| format!("opening {}", game.display()))?;
    let data = d2_data::bin::load(&set, d2_data::bin::DEFAULT_LANGUAGE).context("loading .bin")?;
    let anim = fixup::read_animdata(&set).context("AnimData.d2")?;
    let fixed = fixup::apply(&data, &anim).context("fix-ups")?;
    println!(
        "dump {} vs live .bin + d2_data::fixup (loading.md §7.4, open question 15)",
        dir.display()
    );
    let ptrs: Vec<String> = POINTER_FIELDS
        .iter()
        .map(|(t, o, w, _)| format!("{t}+{o:#X}({w})"))
        .collect();
    println!("pointer fields excluded: {}", ptrs.join(", "));
    println!(
        "\n  {:<18} {:>6} {:>7} {:>7} {:>6}  result",
        "table", "count", "game d", "D2RUST d", "ptr d"
    );
    let (mut identical, mut pending_only, mut bad) = (0, 0, 0);
    for (t, shipped) in fixed.tables.iter().zip(&data.tables) {
        let Some(e) = manifest.get(&t.name) else {
            println!("  {:<18} not in the dump", t.name);
            bad += 1;
            continue;
        };
        if e.kind == "freed" {
            println!(
                "  {:<18} {:>6}  records freed after conversion (maps only)",
                t.name, e.count
            );
            continue;
        }
        if e.count != t.count || e.size != t.record_size {
            println!(
                "  {:<18} MISMATCH: dump {} x {}, D2RUST {} x {}",
                t.name, e.count, e.size, t.count, t.record_size
            );
            bad += 1;
            continue;
        }
        let game_bytes = std::fs::read(dir.join(format!("{}.bin", t.name)))?;
        if game_bytes.len() != t.records.len() {
            bail!(
                "{}.bin: {} bytes, expected {}",
                t.name,
                game_bytes.len(),
                t.records.len()
            );
        }
        let d = diff_table(
            &t.name,
            t.record_size,
            &t.records,
            &game_bytes,
            &shipped.records,
        );
        let sum = |pending: bool| -> usize {
            d.groups
                .iter()
                .filter(|(k, _)| k.0 == pending)
                .map(|(_, g)| g.bytes)
                .sum()
        };
        let (pend, unexp) = (sum(true), sum(false));
        let result = if d.groups.is_empty() {
            identical += 1;
            "identical".to_string()
        } else if unexp == 0 {
            pending_only += 1;
            format!("{pend} bytes differ, all in PENDING rows")
        } else {
            bad += 1;
            format!("DIFFERS: {unexp} bytes ({pend} more in PENDING rows)")
        };
        println!(
            "  {:<18} {:>6} {:>7} {:>7} {:>6}  {result}",
            t.name, t.count, d.game_changed, d.ours_changed, d.pointer_bytes
        );
        for ((pending, label), g) in &d.groups {
            let tag = if *pending { " [pending]" } else { "" };
            let (lo, hi) = g.span.unwrap_or_default();
            println!(
                "      {label}{tag}: {} bytes in {} records, offsets +{lo:#X}..+{hi:#X}",
                g.bytes, g.records
            );
            for (r, o, ours, game) in &g.examples {
                println!(
                    "        record {r} +{o:#X}: D2RUST [{}] game [{}]",
                    hex(ours),
                    hex(game)
                );
            }
        }
    }
    let maps_ok = compare_maps(dir, &fixed, &manifest)?;
    println!(
        "\nsummary: {identical} identical, {pending_only} differ only in PENDING rows, \
         {bad} differ elsewhere; compared maps: {}",
        if maps_ok { "identical" } else { "DIFFER" }
    );
    println!(
        "(game d / D2RUST d: bytes the game's / D2RUST's fix-ups changed from the shipped .bin; \
         ptr d: pointer-field bytes, excluded)"
    );
    if bad > 0 || !maps_ok {
        std::process::exit(1);
    }
    Ok(())
}
