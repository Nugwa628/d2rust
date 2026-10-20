//! End to end on a synthetic install with one file of every image and
//! text kind: convert, verify, perturb each native file, audio skipped.

use std::fs;
use std::path::{Path, PathBuf};

use d2_convert::fsutil::{join, walk_files};
use d2_convert::{convert, kinds, verify, Options, VerifyOptions};
use d2_formats::mpq::writer::MpqWriter;
use test_fixtures::animdata::{self, Anim};
use test_fixtures::sprites::{dc6_file, dc6_frames, dcc_file, Dc6Shape, DccShape};
use test_fixtures::{ds1, dt1, tbl};

fn scratch(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn cof() -> Vec<u8> {
    let mut v = vec![1u8, 2, 2, 20, 0, 0, 0, 0];
    for x in [-10i32, 10, -20, 0] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.extend_from_slice(&256u32.to_le_bytes());
    v.extend_from_slice(&[0, 1, 1, 0, 0]);
    v.extend_from_slice(b"hth\0");
    v.extend_from_slice(&[0, 3]); // events
    v.extend_from_slice(&[0, 0, 0, 0]); // draw order
    v
}

fn font() -> Vec<u8> {
    let mut d = b"Woo!".to_vec();
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&[0, 0, 2, 0, 16, 12]);
    for r in [
        [b'A', 0, 0, 7, 16, 1, 0, 0, 33, 0, 0, 0, 0, 0],
        [b'B', 0, 0, 8, 16, 1, 0, 0, 34, 0, 0, 0, 0, 0],
    ] {
        d.extend_from_slice(&r);
    }
    d
}

fn pl2() -> Vec<u8> {
    (0..1024 + 1714 * 256)
        .map(|i| (i * 7 % 251) as u8)
        .collect()
}

fn expfield() -> Vec<u8> {
    let mut v = 266u16.to_le_bytes().to_vec();
    v.extend_from_slice(&3u32.to_le_bytes());
    v.extend_from_slice(&4u32.to_le_bytes());
    v.extend((0..12).map(|i| i as u8));
    v
}

/// (path, kind name, bytes).
fn sources() -> Vec<(&'static str, &'static str, Vec<u8>)> {
    let shape = Dc6Shape {
        directions: 1,
        frames: 2,
        width: 12,
        height: 9,
        panel: false,
    };
    let frames = dc6_frames(shape, 7);
    let dcc = dcc_file(
        DccShape {
            directions: 2,
            frames: 2,
            width: 20,
            height: 16,
        },
        3,
    )
    .file;
    let pal: Vec<u8> = (0..768).map(|i| (i % 253) as u8).collect();
    vec![
        ("data/global/ui/panel/a.dc6", "dc6", dc6_file(&frames, 1, 2)),
        ("data/global/monsters/zz/tr/zztrhth.dcc", "dcc", dcc),
        (
            "data/global/tiles/act1/t.dt1",
            "dt1",
            dt1::write(&dt1::file(vec![dt1::tile(0, 1, 2, 3)])),
        ),
        (
            "data/global/tiles/act1/t.ds1",
            "ds1",
            ds1::write(&ds1::blank(4, 3, 1)),
        ),
        ("data/global/palette/act1/pal.dat", "pal", pal),
        ("data/global/palette/act1/dark.pl2", "pl2", pl2()),
        ("data/global/monsters/zz/cof/zztrhth.cof", "cof", cof()),
        (
            "data/local/lng/eng/string.tbl",
            "tbl",
            tbl::write(&[(b"k1", b"one"), (b"k2", b"two")]),
        ),
        ("data/local/font/latin/font16.tbl", "font", font()),
        (
            "data/global/animdata.d2",
            "animdata",
            animdata::write(&[Anim {
                name: "ZZTRHTH".into(),
                frames: 8,
                speed: 256,
                events: vec![(2, 1)],
            }]),
        ),
        ("data/global/expfield.d2", "expfield", expfield()),
        (
            "data/global/excel/weapons.txt",
            "excel",
            b"name\tcode\nx\ty\n".to_vec(),
        ),
    ]
}

fn install(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    for name in d2_convert::convert::REQUIRED_ARCHIVES {
        let mut w = MpqWriter::new();
        if name == "d2data.mpq" {
            w = w.with_listfile();
            for (p, _, b) in sources() {
                w.add_file(&p.replace('/', "\\"), b);
            }
            w.add_file(r"data\global\sfx\x.wav", b"RIFF-not-converted".to_vec());
        }
        w.write(dir.join(name)).unwrap();
    }
}

#[test]
fn every_kind_converts_verifies_and_a_perturbed_file_fails() {
    let dir = scratch("kinds");
    install(&dir.join("game"));
    let out = dir.join("native");
    let k = kinds::builtin();
    let mut o = Options::new(dir.join("game"), &out);
    o.converter_commit = "test".into();
    let s = convert(&o, &k).unwrap();
    assert_eq!(s.failures, Vec::new());
    let m = s.manifest.unwrap();
    assert!(m.complete);
    for (path, kind, _) in sources() {
        assert_eq!(m.counts[kind].converted, 1, "{kind} {path}");
        assert_eq!(m.counts[kind].failed, 0, "{kind} {path}");
    }
    // Audio: no kind, listed as skipped with the reason.
    assert!(k.iter().all(|x| x.name() != "wav"));
    let report = fs::read_to_string(out.join("report.txt")).unwrap();
    assert!(
        report.contains(".wav: 1 (skipped: audio not converted for now"),
        "{report}"
    );
    assert!(!out.join("base/data/global/sfx/x.wav").exists());

    let vo = VerifyOptions {
        out: out.clone(),
        deep_install: Some(dir.join("game")),
        progress: false,
    };
    assert_eq!(verify(&vo, &k), Vec::<String>::new());

    // Perturb every native file in turn: verify fails and names it.
    let files = walk_files(&out.join("base")).unwrap();
    assert!(files.len() >= 17, "{files:?}");
    for f in files {
        let p = join(&out.join("base"), &f);
        let orig = fs::read(&p).unwrap();
        let mut bad = orig.clone();
        let at = bad.len() / 2;
        bad[at] ^= 0x01;
        fs::write(&p, &bad).unwrap();
        let problems = verify(&vo, &k);
        assert!(problems.iter().any(|x| x.contains(&f)), "{f}: {problems:?}");
        fs::write(&p, &orig).unwrap();
    }
    assert_eq!(verify(&vo, &k), Vec::<String>::new());
}
