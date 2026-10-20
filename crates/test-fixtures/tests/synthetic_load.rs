//! End to end without game files: synthetic MPQs → text compile → `.bin`
//! pack → `d2_data::bin::load` → typed decode → fix-ups. Everything is
//! generated at test time into `CARGO_TARGET_TMPDIR`.

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_data::bin::{self, excel_path};
use d2_data::schema::schema;
use d2_data::tables::{self, decode_all, Charstats, Experience, Leveldefs, Levels, Weapons};
use d2_data::txt::TxtTable;
use d2_data::{crosscheck, fixup, links};
use d2_formats::mpq::ArchiveSet;
use test_fixtures::content::{self, CLASSES};
use test_fixtures::install::{self, FixtureError, Install};
use test_fixtures::synth::{self, Synthetic};

/// One directory per test process: nextest runs each test in its own
/// process, and two processes writing the same archives race.
fn dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()))
}

fn built() -> &'static Install {
    static I: OnceLock<Install> = OnceLock::new();
    I.get_or_init(|| {
        install::build(&dir("synthetic"), &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"))
    })
}

/// Every runtime table, `hitclass` and the four code buffers load; the
/// loaded `.bin` bytes are the compiled bytes (txt → bin → load round
/// trip), and the cross-check reports every table identical.
#[test]
fn load_round_trips_the_compile() {
    let i = built();
    let runtime: Vec<_> = schema().runtime().collect();
    assert_eq!(runtime.len(), 73);
    assert_eq!(i.loaded.tables.len(), 73);
    for (def, t) in runtime.iter().zip(&i.loaded.tables) {
        assert_eq!(def.name, t.name);
        assert_eq!(t.source, "patch_d2.mpq", "{}", t.name);
        let c = &i.compiled.table(&t.name).unwrap().compiled;
        assert_eq!(t.count, c.count, "{}", t.name);
        assert_eq!(t.records, c.records, "{}", t.name);
        assert!(t.count > 0, "{}", t.name);
    }
    let hit = &i.compiled.table("hitclass").unwrap().compiled;
    assert_eq!(i.loaded.hitclass.records, hit.records);
    assert_eq!(i.loaded.code.len(), 4);
    for (buffer, file) in &i.loaded.code {
        assert_eq!(
            &file.bytes,
            &i.compiled.buffers[buffer],
            "{}",
            buffer.name()
        );
        assert!(!file.bytes.is_empty(), "{} has formulas", buffer.name());
    }

    let report = crosscheck::compare_sets(&i.archives, &i.loaded, &i.compiled).unwrap();
    assert_eq!(report.runtime_matching(), 73);
    for t in &report.tables {
        if t.compared() {
            assert!(t.identical(), "{}: {:?}", t.name, t.mismatches);
        }
    }
    assert!(report.buffers.iter().all(|b| b.identical()));
}

/// The rows are coherent: the compile reports no diagnostic (no missed
/// link, no cut text, no bad integer), and no link in the loaded set
/// points past its target table.
#[test]
fn synthetic_set_is_coherent() {
    let i = built();
    let diags: Vec<_> = i
        .compiled
        .tables
        .iter()
        .flat_map(|t| {
            t.compiled
                .diagnostics
                .iter()
                .map(move |d| format!("{} {:?} line {} {:?}", t.name, d.kind, d.line, d.field))
        })
        .collect();
    assert!(diags.is_empty(), "{diags:#?}");
    assert!(i.compiled.unspecified_callbacks.is_empty());
    assert!(
        i.compiled.calc_diagnostics.is_empty(),
        "{:?}",
        i.compiled.calc_diagnostics
    );

    let lookups = links::load_lookups(&i.archives).unwrap();
    assert!(!lookups.is_empty());
    let (_, report) = links::validate_set(&i.loaded, &lookups);
    assert!(report.is_clean(), "{:#?}", report.broken);
    assert!(report.valid > 100, "{} links checked", report.valid);
}

/// Every runtime table decodes to its typed records, and values written
/// in the synthetic rows come back.
#[test]
fn typed_decode_returns_the_rows() {
    let i = built();
    for t in &i.loaded.tables {
        let n = tables::decode_by_name(t)
            .unwrap_or_else(|| panic!("{}: no typed struct", t.name))
            .unwrap();
        assert_eq!(n, t.count, "{}", t.name);
    }
    let table = |n: &str| i.loaded.table(n).unwrap();

    let cs: Vec<Charstats> = decode_all(table("charstats")).unwrap();
    assert_eq!(cs.len(), CLASSES.len());
    for (k, (c, name)) in cs.iter().zip(CLASSES).enumerate() {
        assert_eq!(tables::text(&c.class), name.as_bytes());
        assert_eq!(u32::from(c.str), 15 + 2 * k as u32);
    }

    let exp: Vec<Experience> = decode_all(table("experience")).unwrap();
    assert_eq!(exp.len(), 101);
    assert_eq!(exp[0].amazon, content::MAX_LEVEL);
    for l in 1..=content::MAX_LEVEL + 1 {
        assert_eq!(exp[l as usize].sorceress, content::exp_for_level(l));
    }

    let lv: Vec<Levels> = decode_all(table("levels")).unwrap();
    let defs: Vec<Leveldefs> = decode_all(table("leveldefs")).unwrap();
    assert_eq!(lv.len(), content::LEVELS.len());
    assert_eq!(defs.len(), lv.len());
    assert_eq!(lv[5].act, 1);
    assert_eq!(defs[3].drlgtype, 1);

    let w: Vec<Weapons> = decode_all(table("weapons")).unwrap();
    assert_eq!(&w[1].code, b"lb1 ");
    assert_eq!((w[1].mindam, w[1].maxdam), (5, 14));
}

/// The fix-ups run on the loaded set and the AnimData the install holds.
#[test]
fn fixups_apply() {
    let i = built();
    let anim = fixup::read_animdata(&i.archives).unwrap();
    assert!(anim.find(b"BEA1HTH").unwrap().is_some());
    let f = fixup::apply(&i.loaded, &anim).unwrap();
    assert_eq!(f.tables.len(), 73);
    assert!(f.superunique_hc.iter().all(Option::is_some));
    assert_eq!(f.item_codes.len(), 6, "weapons, armor, misc codes");
    assert_eq!(f.uniques.len(), 1);
    assert_eq!(f.sets.len(), 2);
    assert_eq!(f.stat_stuff, 6);
}

/// The `.txt` files read back from the archives byte for byte, and parse.
#[test]
fn text_files_read_back() {
    let i = built();
    let data = synth::synthetic();
    for (name, bytes) in data.tables.render() {
        let got = i.archives.read(&excel_path(&name)).unwrap();
        assert_eq!(got, bytes, "{name}");
        TxtTable::parse(&name, &got).unwrap();
    }
    assert_eq!(i.archives.archives().len(), 3);
    assert!(i.archives.has_archive("d2exp.mpq"));
    assert!(i.loaded.lod);
}

/// Two builds give identical archives (no clock, no ambient randomness).
#[test]
fn builds_are_deterministic() {
    let data = synth::synthetic();
    let a = dir("determinism-a");
    let b = dir("determinism-b");
    install::build(&a, &data).unwrap();
    install::build(&b, &data).unwrap();
    for f in ["d2data.mpq", "d2exp.mpq", "patch_d2.mpq"] {
        assert_eq!(
            std::fs::read(a.join(f)).unwrap(),
            std::fs::read(b.join(f)).unwrap(),
            "{f}"
        );
    }
}

fn build_err(name: &str, data: &Synthetic) -> FixtureError {
    install::build(&dir(name), data).expect_err("the change must fail the load")
}

/// M08: a changed cell changes exactly its field in the loaded bytes, and
/// rows the load checks count are caught.
#[test]
fn perturbations_are_reported() {
    let mut data = synth::synthetic();
    data.tables.set("charstats", 1, "str", "77");
    let changed = install::build(&dir("perturb-cell"), &data).unwrap();
    let base = built();
    let a = &base.loaded.table("charstats").unwrap().records;
    let b = &changed.loaded.table("charstats").unwrap().records;
    let size = schema().table("charstats").unwrap().record_size;
    let at = size
        + schema()
            .table("charstats")
            .unwrap()
            .field("str")
            .unwrap()
            .offset as usize;
    let diff: Vec<usize> = (0..a.len()).filter(|&k| a[k] != b[k]).collect();
    assert_eq!(diff, [at]);
    assert_eq!(b[at], 77);

    // One inventory row short of the 32 the load requires.
    let mut data = synth::synthetic();
    data.tables
        .files
        .get_mut("inventory.txt")
        .unwrap()
        .rows
        .pop();
    let e = build_err("perturb-inventory", &data);
    assert!(
        matches!(e, FixtureError::Load(bin::LoadError::Check { ref table, .. }) if table == "inventory"),
        "{e}"
    );

    // A superunique hcIdx missing.
    let mut data = synth::synthetic();
    data.tables.set("superuniques", 7, "hcIdx", "70");
    let e = build_err("perturb-hcidx", &data);
    assert!(e.to_string().contains("superuniques"), "{e}");
}

/// The archive set the loader sees is exactly the three written archives.
#[test]
fn install_dir_opens_as_an_archive_set() {
    let i = built();
    let set = ArchiveSet::open_dir(&i.dir).unwrap();
    assert!(set.contains("data\\global\\excel\\levels.bin"));
    assert!(set.contains("data\\local\\lng\\eng\\expansionstring.tbl"));
    assert_eq!(
        bin::read_excel(&set, "levels.bin").unwrap().unwrap().0,
        "patch_d2.mpq"
    );
    assert_eq!(
        bin::read_excel(&set, "levels.txt").unwrap().unwrap().0,
        "d2data.mpq"
    );
}
