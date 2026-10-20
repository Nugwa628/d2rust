//! Whole-install sweeps of the formats against the user's own LoD archives: every
//! file of a format decodes, and the file counts and header values recorded in the
//! comments below hold. Reads archives through `d2_formats::mpq` and parses each file with
//! its format reader.
//!
//! Ignored by default. Run (release: the DCC sweep decodes ~22k files):
//! `D2_GAME_DIR=<install> cargo test --release -p d2-formats --test game_sweep -- --ignored --nocapture`
//!
//! Counting scope: the recorded counts were measured with `mpq-tool formats`, which counts
//! files per archive (so the 3,605 COF files are files, not distinct names) and finds the
//! 6 DC6 of `patch_d2.mpq`, which has no `(listfile)`. This file rebuilds that scope:
//! every name of the union of all listfiles and of `EXTRA_NAMES`
//! (`d2_formats::mpq::names`, shared with `mpq-tool`), one per archive lookup key (case
//! and `/` vs `\` folded), in every archive that holds it. Counts taken earlier from
//! `mpq-tool formats`, which kept names case-sensitively and counted a file twice when two
//! listfiles spelled it differently, are inflated. A count that differs while every file
//! decodes is a scope difference to record, not a decoder failure.
//!
//! The expected values were written without game files, so they are unconfirmed until a
//! local run passes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use d2_formats::animdata::{self, AnimData, AnimRecord};
use d2_formats::cof::Cof;
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::font::FontTable;
use d2_formats::mpq::names::known_names;
use d2_formats::mpq::{Archive, ArchiveSet};
use d2_formats::palette::{Palette, Pl2};
use d2_formats::tbl::StringTable;

fn set() -> ArchiveSet {
    let dir = PathBuf::from(std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set"));
    ArchiveSet::open_dir(&dir).expect("archives in D2_GAME_DIR open")
}

/// Distinct names over every archive of `set` (listfiles and
/// `EXTRA_NAMES`, from `d2_formats::mpq::names`, the source `mpq-tool
/// formats` uses), lowercase with `\` separators: one per archive lookup
/// key (case and `/` vs `\` folded).
fn listed(set: &ArchiveSet) -> BTreeSet<String> {
    known_names(set.archives())
        .unwrap()
        .into_values()
        .map(|n| n.to_ascii_lowercase().replace('/', "\\"))
        .collect()
}

/// One file: an archive (index in `set.archives()`) and a name it holds.
type FileRef = (usize, String);

/// Every file whose name ends with `ext` and passes `keep`: each name of
/// the union of listfiles, once per archive that holds it.
fn files_where(set: &ArchiveSet, ext: &str, keep: impl Fn(&str) -> bool) -> Vec<FileRef> {
    let names = listed(set);
    let mut out = Vec::new();
    for (i, a) in set.archives().iter().enumerate() {
        for n in names.iter().filter(|n| n.ends_with(ext) && keep(n)) {
            if a.contains(n) {
                out.push((i, n.clone()));
            }
        }
    }
    out
}

fn files(set: &ArchiveSet, ext: &str) -> Vec<FileRef> {
    files_where(set, ext, |_| true)
}

/// `archive:name`, for messages.
fn label(set: &ArchiveSet, f: &FileRef) -> String {
    format!("{}:{}", archive_file(&set.archives()[f.0]), f.1)
}

/// The bytes of one file, from its own archive.
fn bytes(set: &ArchiveSet, f: &FileRef) -> Vec<u8> {
    set.archives()[f.0]
        .read(&f.1)
        .unwrap_or_else(|e| panic!("{}: {e}", label(set, f)))
}

/// Reads a listed name; a listed name that does not read is a failure.
fn read(set: &ArchiveSet, name: &str) -> Vec<u8> {
    set.read(name).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Lowercase file name of an archive (`d2char.mpq`).
fn archive_file(a: &Archive) -> String {
    a.path()
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// The archives (file names) that hold `name`.
fn holders(set: &ArchiveSet, name: &str) -> Vec<String> {
    let mut v: Vec<String> = set
        .archives()
        .iter()
        .filter(|a| a.contains(name))
        .map(archive_file)
        .collect();
    v.sort();
    v
}

// All `.dc6` files decode. `mpq-tool formats` with its case-sensitive name set counted
// 1,657 files (29,117 frames; 140 frames with flip = 1; termination EE×4 in 1,195 files,
// CD×4 in 400, 00×4 in 62), but 4 of those were the same file twice. Measured
// case-insensitively, this sweep prints 1,653 files, 26,317 frames, 140 flipped, EE×4
// 1,193 / CD×4 400 / 00×4 60 (the 4 removed files: 2 EE + 2 00).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn dc6_every_file_decodes() {
    let set = set();
    let names = files(&set, ".dc6");
    let (mut frames, mut flipped) = (0usize, 0usize);
    let mut termination: BTreeMap<[u8; 4], usize> = BTreeMap::new();
    for f in &names {
        let name = &label(&set, f);
        let dc6 = Dc6::parse(&bytes(&set, f)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(dc6.header.version, 6, "{name}");
        let per = dc6.header.directions as usize * dc6.header.frames_per_direction as usize;
        assert_eq!(dc6.frames.len(), per, "{name}: D × F frames");
        for f in &dc6.frames {
            assert_eq!(
                f.pixels.len(),
                f.width as usize * f.height as usize,
                "{name}"
            );
        }
        frames += dc6.frames.len();
        flipped += dc6.frames.iter().filter(|f| f.flip == 1).count();
        *termination.entry(dc6.header.termination).or_default() += 1;
    }
    println!(
        "dc6: {} files, {frames} frames, {flipped} flipped, termination {termination:02X?}",
        names.len()
    );
    assert_eq!(names.len(), 1_653);
    assert_eq!(frames, 26_317);
    assert_eq!(flipped, 140);
    let want: BTreeMap<[u8; 4], usize> =
        [([0xEE; 4], 1_193), ([0xCD; 4], 400), ([0x00; 4], 60)].into();
    assert_eq!(termination, want);
}

// All 21,717 `.dcc` files decode with every sub-stream
// exactly consumed (the parser's end checks) and fewer than 8 leftover PCD
// bits per direction; version 6 throughout; no frame has bottom-up = 1.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn dcc_every_file_decodes() {
    let set = set();
    let names = files(&set, ".dcc");
    let (mut directions, mut frames) = (0usize, 0usize);
    for f in &names {
        let name = &label(&set, f);
        let dcc = Dcc::parse(&bytes(&set, f)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(dcc.version, 6, "{name}");
        assert!(dcc.frames_per_direction <= 256, "{name}");
        for (d, dir) in dcc.directions.iter().enumerate() {
            assert!(dir.pcd_leftover_bits < 8, "{name} dir {d}");
            assert_eq!(
                dir.frames.len(),
                dcc.frames_per_direction as usize,
                "{name} dir {d}"
            );
            for f in &dir.frames {
                assert!(!f.bottom_up, "{name} dir {d}");
                assert_eq!(
                    f.pixels.len(),
                    f.width as usize * f.height as usize,
                    "{name}"
                );
            }
            frames += dir.frames.len();
        }
        directions += dcc.directions.len();
    }
    println!(
        "dcc: {} files, {directions} directions, {frames} frames",
        names.len()
    );
    assert_eq!(names.len(), 21_717);
}

// All 256 live `.dt1` files (names case-insensitive) are read: 250 decode, 6 version-4
// leftovers are refused; block formats 0x0001 (226,996), 0x1001 (108,905), 0x2005
// (15,712). Header: minor version 6 in the supported install. This sweep and `mpq-tool
// formats` both measure these counts.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn dt1_every_live_file_decodes() {
    let set = set();
    let names = files(&set, ".dt1");
    let mut formats: BTreeMap<u16, usize> = BTreeMap::new();
    let (mut live, mut tiles) = (0usize, 0usize);
    let mut v4 = Vec::new();
    for f in &names {
        let name = &label(&set, f);
        let bytes = bytes(&set, f);
        let major = i32::from_le_bytes(bytes[..4].try_into().unwrap());
        if major == 4 {
            assert!(Dt1::parse(&bytes).is_err(), "{name}: version 4 is refused");
            v4.push(name.clone());
            continue;
        }
        let dt1 = Dt1::parse(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((dt1.version, dt1.minor_version), (7, 6), "{name}");
        for t in &dt1.tiles {
            for b in &t.blocks {
                *formats.entry(b.format).or_default() += 1;
                let (w, h) = b.size();
                assert_eq!(b.pixels.len(), w * h, "{name}");
            }
        }
        tiles += dt1.tiles.len();
        live += 1;
    }
    println!(
        "dt1: {live} live files, {tiles} tiles, version-4 {v4:?}, block formats {formats:04X?}"
    );
    assert_eq!(live, 250);
    assert_eq!(v4.len(), 6);
    let want: BTreeMap<u16, usize> =
        [(0x0001, 226_996), (0x1001, 108_905), (0x2005, 15_712)].into();
    assert_eq!(formats, want);
}

// All 2,372 `.ds1` files parse (names case-insensitive); versions seen 3, 8, 12, 13, 15,
// 16, 17, 18 (1,926 at v18). This sweep and `mpq-tool formats` both measure these counts.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn ds1_every_file_parses() {
    let set = set();
    let names = files(&set, ".ds1");
    let mut versions: BTreeMap<u32, usize> = BTreeMap::new();
    for f in &names {
        let name = &label(&set, f);
        let ds1 = Ds1::parse(&bytes(&set, f)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let cells = ds1.width as usize * ds1.height as usize;
        for layer in ds1.walls.iter().chain(&ds1.floors) {
            assert_eq!(layer.len(), cells, "{name}");
        }
        assert_eq!(ds1.shadow.len(), cells, "{name}");
        assert!(ds1.walls.len() <= 4 && ds1.floors.len() <= 2, "{name}");
        *versions.entry(ds1.version).or_default() += 1;
    }
    println!("ds1: {} files, versions {versions:?}", names.len());
    assert_eq!(names.len(), 2_372);
    let seen: Vec<u32> = versions.keys().copied().collect();
    assert_eq!(seen, [3, 8, 12, 13, 15, 16, 17, 18]);
    assert_eq!(versions[&18], 1_926);
}

// All 3,605 live `.cof` files parse, every version byte is
// 20; Edge cases: 3 files of 42 bytes, 1 layer, 1 frame, 1 direction (K =
// 4: 3 padding bytes); `chars\am\cof\amblxbow.cof` (d2char.mpq) is 72
// bytes of junk and the only failure. There is no `amblxbw.cof` in the supported install:
// the Amazon block COFs are `ambl1hs`, `ambl1ht` and `amblhth`.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn cof_every_live_file_parses() {
    let set = set();
    let names = files(&set, ".cof");
    let junk = r"data\global\chars\am\cof\amblxbow.cof";
    let (mut parsed, mut padded) = (0usize, 0usize);
    let mut failed = Vec::new();
    for f in &names {
        let name = &label(&set, f);
        let bytes = bytes(&set, f);
        match Cof::parse(&bytes) {
            Ok(cof) => {
                assert_eq!(cof.version, 20, "{name}");
                assert_eq!(cof.events.len(), usize::from(cof.frames), "{name}");
                assert_eq!(
                    cof.draw_order.len(),
                    usize::from(cof.directions)
                        * usize::from(cof.frames)
                        * usize::from(cof.layers_count),
                    "{name}"
                );
                if bytes.len() == 42 && (cof.layers_count, cof.frames, cof.directions) == (1, 1, 1)
                {
                    assert_eq!(cof.event_padding.len(), 3, "{name}");
                    padded += 1;
                }
                parsed += 1;
            }
            Err(_) => failed.push((name.clone(), bytes.len())),
        }
    }
    println!("cof: {parsed} parse, failed {failed:?}, 42-byte padded {padded}");
    // The listed Amazon block (`ambl*`) COFs are
    // `ambl1hs`, `ambl1ht`, `amblhth` (which parse) and the junk file.
    let ambl: Vec<String> = listed(&set)
        .into_iter()
        .filter(|n| n.starts_with(r"data\global\chars\am\cof\ambl"))
        .collect();
    for n in &ambl {
        println!("  {n}: {:?}", holders(&set, n));
    }
    assert_eq!(parsed, 3_605);
    assert_eq!(failed, [(format!("d2char.mpq:{junk}"), 72)]);
    assert_eq!(
        ambl,
        [
            r"data\global\chars\am\cof\ambl1hs.cof",
            r"data\global\chars\am\cof\ambl1ht.cof",
            r"data\global\chars\am\cof\amblhth.cof",
            junk,
        ]
    );
    for n in &ambl[..3] {
        Cof::parse(&read(&set, n)).unwrap_or_else(|e| panic!("{n}: {e}"));
    }
    assert_eq!(padded, 3);
}

// All 19 `pal.dat` and 17 `.pl2` files parse; one PL2 has 12 text colors, the rest 13.
// Every `.dat` under `data\global\palette` parses.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn palettes_every_file_parses() {
    let set = set();
    let dats = files_where(&set, ".dat", |n| n.starts_with(r"data\global\palette\"));
    let mut pal_dat = 0;
    for f in &dats {
        let name = label(&set, f);
        Palette::parse(&bytes(&set, f)).unwrap_or_else(|e| panic!("{name}: {e}"));
        if name.ends_with(r"\pal.dat") {
            pal_dat += 1;
        }
    }
    let mut text_colors: BTreeMap<usize, usize> = BTreeMap::new();
    let pl2s = files(&set, ".pl2");
    for f in &pl2s {
        let name = label(&set, f);
        let pl2 = Pl2::parse(&bytes(&set, f)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(pl2.text_color_shifts.len(), pl2.text_colors.len(), "{name}");
        *text_colors.entry(pl2.text_colors.len()).or_default() += 1;
    }
    println!(
        "palettes: {} .dat ({pal_dat} pal.dat), {} .pl2, text colors {text_colors:?}",
        dats.len(),
        pl2s.len()
    );
    assert_eq!(pal_dat, 19);
    assert_eq!(pl2s.len(), 17);
    assert_eq!(text_colors, [(12, 1), (13, 16)].into());
}

/// Listed files with their bytes.
type Files = Vec<(String, Vec<u8>)>;

/// Every listed `.tbl`, split by content (printable text files are not string tables):
/// (font tables, string tables); printable-text files are left out.
fn tables(set: &ArchiveSet) -> (Files, Files) {
    let is_text = |b: &[u8]| {
        b.iter()
            .all(|&c| c == b'\t' || c == b'\r' || c == b'\n' || (0x20..0x7F).contains(&c))
    };
    let (mut fonts, mut strings) = (Vec::new(), Vec::new());
    for f in files(set, ".tbl") {
        let (name, b) = (label(set, &f), bytes(set, &f));
        if FontTable::is_font_table(&b) {
            fonts.push((name, b));
        } else if !is_text(&b) {
            strings.push((name, b));
        }
    }
    (fonts, strings)
}

// All 14 font tables parse, each with 256 records.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn font_tables_every_file_parses() {
    let (fonts, _) = tables(&set());
    for (name, b) in &fonts {
        let f = FontTable::parse(b).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(f.version, 1, "{name}");
        assert_eq!(f.glyphs.len(), (b.len() - 12) / 14, "{name}");
        assert_eq!(f.glyphs.len(), 256, "{name}");
    }
    println!("font tables: {}", fonts.len());
    assert_eq!(fonts.len(), 14);
}

// 29 string-table copies (20 distinct
// paths, 10 language folders plus ENG\BETA, names case-insensitive) in
// d2data, d2exp and Patch_D2; every key is ASCII, every version byte is 1,
// used entries equal `num_elements`, the header file size equals the file
// length. 63,167 used entries, 16,786 values hold
// a byte >= 0x80 and all decode as strict UTF-8, none holds a raw `FF`,
// 130 hold `C3 BF`. For the eng tables, every key resolves to its
// own slot, or to an earlier slot holding the same key (a duplicate first
// in the probe sequence).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn string_tables_every_key_resolves() {
    let (_, strings) = tables(&set());
    let mut languages = BTreeSet::new();
    let mut paths = BTreeSet::new();
    let (mut keys, mut to_duplicate) = (0usize, 0usize);
    let (mut non_ascii, mut raw_ff, mut c3bf) = (0usize, 0usize, 0usize);
    for (name, b) in &strings {
        let t = StringTable::parse(b).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(t.header.version, 1, "{name}");
        assert_eq!(t.header.file_size as usize, b.len(), "{name}");
        let path = name.split_once(':').unwrap().1;
        paths.insert(path.to_string());
        let rest = path
            .strip_prefix(r"data\local\lng\")
            .unwrap_or_else(|| panic!("{name}: string table outside data\\local\\lng"));
        languages.insert(rest.split('\\').next().unwrap().to_string());
        let used = t.entries.iter().filter(|e| e.used != 0).count();
        assert_eq!(used, usize::from(t.header.num_elements), "{name}");
        for (slot, e) in t.entries.iter().enumerate().filter(|(_, e)| e.used != 0) {
            assert!(e.key.is_ascii(), "{name} slot {slot}");
            let found = t
                .find_slot(&e.key)
                .unwrap_or_else(|| panic!("{name} slot {slot}: key not found"));
            assert_eq!(t.entries[found].key, e.key, "{name} slot {slot}");
            if found != slot {
                to_duplicate += 1;
            }
            keys += 1;
            if !e.value.is_ascii() {
                non_ascii += 1;
                std::str::from_utf8(&e.value)
                    .unwrap_or_else(|err| panic!("{name} slot {slot}: not UTF-8: {err}"));
            }
            // A valid UTF-8 string never holds an `FF` byte; counted
            // separately so a failure names that count.
            if e.value.contains(&0xFF) {
                raw_ff += 1;
            }
            if e.value.windows(2).any(|w| w == [0xC3, 0xBF]) {
                c3bf += 1;
            }
        }
    }
    println!(
        "string tables: {} ({} paths, {} languages {languages:?}), {keys} used entries, \
         {to_duplicate} resolve to an earlier duplicate, {non_ascii} non-ASCII values, \
         {raw_ff} raw FF, {c3bf} with C3 BF",
        strings.len(),
        paths.len(),
        languages.len()
    );
    assert_eq!(strings.len(), 29);
    assert_eq!(paths.len(), 20);
    let expected: BTreeSet<String> = [
        "chi", "deu", "eng", "esp", "fra", "ita", "jpn", "kor", "pol", "por",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect();
    assert_eq!(languages, expected);
    assert!(
        paths
            .iter()
            .any(|p| p.starts_with(r"data\local\lng\eng\beta\")),
        "ENG\\BETA"
    );
    assert_eq!(keys, 63_167);
    assert_eq!(non_ascii, 16_786);
    assert_eq!(raw_ff, 0);
    assert_eq!(c3bf, 130);
}

fn name_of(r: &AnimRecord) -> String {
    let n = r.name.iter().position(|&b| b == 0).unwrap_or(8);
    String::from_utf8_lossy(&r.name[..n]).into_owned()
}

/// Bucket and position of the first record named `name`.
fn position(a: &AnimData, name: &str) -> (usize, usize) {
    a.buckets
        .iter()
        .enumerate()
        .find_map(|(b, recs)| recs.iter().position(|r| name_of(r) == name).map(|i| (b, i)))
        .unwrap_or_else(|| panic!("{name} not in the file"))
}

/// The non-zero event bytes of a record, as (frame, value).
fn events(r: &AnimRecord) -> Vec<(usize, u8)> {
    r.events
        .iter()
        .enumerate()
        .filter(|(_, &v)| v != 0)
        .map(|(i, &v)| (i, v))
        .collect()
}

// The `AnimData.d2` copies in the archives, with fixed values from the supported install.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn animdata_real_vectors() {
    let set = set();
    // Only d2exp (X) and d2data (D) hold the file; the set loads X.
    assert_eq!(holders(&set, animdata::PATH), ["d2data.mpq", "d2exp.mpq"]);
    let x = read(&set, animdata::PATH);
    let exp = set
        .archives()
        .iter()
        .find(|a| archive_file(a) == "d2exp.mpq")
        .unwrap();
    assert_eq!(x, exp.read(animdata::PATH).unwrap());

    // Whole file.
    assert_eq!(x.len(), 570_304);
    assert_eq!(x.len(), 256 * 4 + 3_558 * 160);
    assert_eq!(u32::from_le_bytes(x[..4].try_into().unwrap()), 0x36);
    let a = AnimData::parse(&x).unwrap();
    let all: Vec<&AnimRecord> = a.buckets.iter().flatten().collect();
    assert_eq!(all.len(), 3_558);
    assert_eq!(a.buckets.iter().filter(|b| !b.is_empty()).count(), 136);
    assert_eq!(a.buckets.iter().filter(|b| b.is_empty()).count(), 120);
    assert_eq!(a.buckets.iter().map(Vec::len).max(), Some(67));
    assert_eq!(a.buckets[0].len(), 54);
    assert_eq!(name_of(&a.buckets[0][0]), "L1OPHTH");

    // Every record: 7 characters, uppercase, NUL-padded, in its own bucket.
    for (b, bucket) in a.buckets.iter().enumerate() {
        for r in bucket {
            let name = name_of(r);
            assert_eq!(name.len(), 7, "{name}");
            assert!(r.name[7] == 0, "{name}");
            assert!(!name.bytes().any(|c| c.is_ascii_lowercase()), "{name}");
            assert_eq!(animdata::hash(&r.name), b, "{name}");
        }
    }

    // Ranges.
    let frames: Vec<u32> = all.iter().map(|r| r.frames).collect();
    assert_eq!(frames.iter().min(), Some(&1));
    assert_eq!(frames.iter().max(), Some(&200));
    assert_eq!(frames.iter().filter(|&&f| f > 144).count(), 1);
    let speeds: BTreeSet<u32> = all.iter().map(|r| r.speed).collect();
    assert_eq!(speeds.first(), Some(&0));
    assert_eq!(speeds.last(), Some(&512));
    assert_eq!(speeds.len(), 41);

    // Event bytes.
    let mut values: BTreeMap<u8, usize> = BTreeMap::new();
    let mut multi = 0;
    for r in &all {
        let ev = events(r);
        for &(i, v) in &ev {
            assert!((i as u32) < r.frames, "{}: event at {i}", name_of(r));
            *values.entry(v).or_default() += 1;
        }
        if ev.len() > 1 {
            multi += 1;
        }
    }
    assert_eq!(values, [(1, 433), (2, 148), (3, 4)].into());
    assert_eq!(multi, 10);

    // Named records. A lowercase query is uppercased first.
    assert_eq!(position(&a, "SKWL1HS"), (13, 40));
    let r = a.find(b"skwl1hs").unwrap().expect("SKWL1HS found");
    assert_eq!((name_of(r), r.frames, r.speed), ("SKWL1HS".into(), 8, 128));
    assert!(events(r).is_empty());

    assert_eq!(position(&a, "SKA11HS").0, 220);
    let r = a.find(b"SKA11HS").unwrap().unwrap();
    assert_eq!((r.frames, r.speed, r.events[10]), (16, 224, 1));
    assert_eq!(a.info(b"SKA11HS").unwrap().first_event, 10);

    assert_eq!(position(&a, "AMA1BOW").0, 232);
    let r = a.find(b"AMA1BOW").unwrap().unwrap();
    assert_eq!((r.frames, r.speed, r.events[6]), (14, 256, 2));

    let r = a.find(b"10A1HTH").unwrap().unwrap();
    assert_eq!(
        (r.frames, r.speed, r.events[14], r.events[17]),
        (38, 256, 3, 1)
    );
    assert_eq!(a.info(b"10A1HTH").unwrap().first_event, 14);

    let r = a.find(b"42DTHTH").unwrap().unwrap();
    assert_eq!((r.frames, r.speed), (200, 168));
    assert!(events(r).is_empty());
    let i = a.info(b"42DTHTH").unwrap();
    assert_eq!(
        (i.frames, i.speed, i.first_event, i.found),
        (200, 168, 144, true)
    );

    // The first of the two VMS1HTH copies wins.
    assert_eq!(position(&a, "VMS1HTH"), (11, 36));
    let r = a.find(b"VMS1HTH").unwrap().unwrap();
    assert!(std::ptr::eq(r, &a.buckets[11][36]));
    assert_eq!((r.frames, r.speed, r.events[10]), (17, 200, 2));

    // Not found → the default record.
    assert_eq!(animdata::hash(b"GOWLHTH"), 29);
    assert!(a.find(b"GOWLHTH").unwrap().is_none());
    assert_eq!(*a.record(b"GOWLHTH").unwrap(), animdata::DEFAULT_RECORD);
    let i = a.info(b"GOWLHTH").unwrap();
    assert_eq!(
        (i.frames, i.speed, i.first_event, i.found),
        (2048, 256, 2048, false)
    );

    // The shadowed D copy.
    let data = set
        .archives()
        .iter()
        .find(|a| archive_file(a) == "d2data.mpq")
        .unwrap();
    let d = data.read(animdata::PATH).unwrap();
    assert_eq!(d.len(), 411_104);
    assert_eq!(d.len(), 1_024 + 2_563 * 160);
    let d = AnimData::parse(&d).unwrap();
    assert_eq!(d.buckets.iter().flatten().count(), 2_563);
    assert_eq!(d.buckets.iter().filter(|b| b.is_empty()).count(), 130);
    let mut seen = BTreeMap::<String, usize>::new();
    for r in d.buckets.iter().flatten() {
        *seen.entry(name_of(r)).or_default() += 1;
    }
    assert_eq!(seen.values().filter(|&&n| n > 1).count(), 7);
}

/// The `.cof` of an AnimData name: `data\global\{monsters,chars,objects}\
/// <first 2 chars>\cof\<name>.cof`, archives in search order.
fn cof_for(set: &ArchiveSet, name: &str) -> Option<Cof> {
    ["monsters", "chars", "objects"].iter().find_map(|dir| {
        let path = format!(r"data\global\{dir}\{}\cof\{name}.cof", &name[..2]);
        set.read(&path)
            .ok()
            .map(|b| Cof::parse(&b).unwrap_or_else(|e| panic!("{path}: {e}")))
    })
}

// COF cross-check: 3,529 of 3,558 names have a
// `.cof`; for each, frames = COF frames, speed = COF animation rate,
// events = COF events zero-filled to 144, except, for each of the 9
// differing duplicates, the copy that does not match: the second copy for
// 64A1HTH, 64NUHTH, MINUHTH, the first for the other 6.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn animdata_matches_every_cof() {
    let set = set();
    let a = AnimData::parse(&read(&set, animdata::PATH)).unwrap();
    let differing = [
        "VMS1HTH", "VMGHHTH", "MINUHTH", "VMWLHTH", "VMNUHTH", "64A1HTH", "64NUHTH", "VMA1HTH",
        "3DNUHTH",
    ];
    let first_matches = ["64A1HTH", "64NUHTH", "MINUHTH"];
    let (mut records, mut with_cof, mut skipped) = (0usize, 0usize, 0usize);
    let mut names_with_cof = BTreeSet::new();
    let mut first_seen = BTreeSet::new();
    let mut mismatched = Vec::new();
    for r in a.buckets.iter().flatten() {
        records += 1;
        let name = name_of(r);
        let first = first_seen.insert(name.clone());
        let Some(cof) = cof_for(&set, &name) else {
            continue;
        };
        with_cof += 1;
        names_with_cof.insert(name.clone());
        let mismatching_copy = if first_matches.contains(&name.as_str()) {
            !first
        } else {
            first
        };
        if mismatching_copy && differing.contains(&name.as_str()) {
            skipped += 1;
            continue;
        }
        let mut ev = cof.events.clone();
        ev.resize(animdata::EVENTS, 0);
        if r.frames != u32::from(cof.frames)
            || r.speed != cof.animation_rate
            || r.events[..] != ev[..]
        {
            mismatched.push(name);
        }
    }
    println!(
        "animdata: {records} records, {with_cof} with a .cof ({} distinct names), {skipped} non-matching copies skipped",
        names_with_cof.len()
    );
    assert_eq!(records, 3_558);
    assert_eq!(with_cof, 3_529);
    assert_eq!(skipped, 9);
    assert!(mismatched.is_empty(), "{mismatched:?}");
}

// `expfield.d2`, data facts only (D2RUST has no expfield reader, so nothing here exercises
// one): only in d2data.mpq, 65,546
// bytes, u16 266, 256 rows × 256 columns, the cell counts, centre (128,
// 128) = 8, row 128 columns 120–127 = 2 and 129–136 = 6.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn expfield_layout() {
    let set = set();
    let path = r"data\global\expfield.d2";
    assert_eq!(holders(&set, path), ["d2data.mpq"]);
    let b = read(&set, path);
    assert_eq!(b.len(), 65_546);
    assert_eq!(u16::from_le_bytes([b[0], b[1]]), 266);
    let rows = u32::from_le_bytes(b[2..6].try_into().unwrap()) as usize;
    let cols = u32::from_le_bytes(b[6..10].try_into().unwrap()) as usize;
    assert_eq!((rows, cols), (256, 256));
    let cells = &b[10..];
    assert_eq!(cells.len(), rows * cols);
    let mut counts = [0usize; 9];
    for &c in cells {
        assert!(c <= 8, "cell value {c}");
        counts[usize::from(c)] += 1;
    }
    assert_eq!(
        counts,
        [8_128, 8_192, 8_256, 8_256, 8_256, 8_191, 8_128, 8_128, 1]
    );
    let at = |r: usize, c: usize| cells[r * 256 + c];
    assert_eq!(at(128, 128), 8);
    assert!((120..=127).all(|c| at(128, c) == 2));
    assert!((129..=136).all(|c| at(128, c) == 6));
}
