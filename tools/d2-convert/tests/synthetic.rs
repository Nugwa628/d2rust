//! Converter tests on a synthetic install (no game files): a stub kind
//! stands in for the image and text kinds, the real excel copy is used as is.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use d2_convert::fsutil::{join, walk_files};
use d2_convert::kinds::Excel;
use d2_convert::{
    convert, verify, ConvertError, Failure, Kind, NativeData, Options, VerifyOptions, Written,
};
use d2_formats::mpq::writer::MpqWriter;

/// `.stub` files: native form is `P.hex` (hex text of the source) and
/// `P.len`. `ref:<path>` lines name other files (name-set closure).
struct Stub {
    version: u32,
    /// Path whose written bytes are wrong (the perturbation).
    corrupt: Option<String>,
    writes: Arc<AtomicUsize>,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Kind for Stub {
    fn name(&self) -> &'static str {
        "stub"
    }
    fn native_version(&self) -> u32 {
        self.version
    }
    fn claims(&self, canon: &str) -> bool {
        canon.ends_with(".stub")
    }
    fn has_references(&self) -> bool {
        true
    }
    fn references(&self, _: &str, src: &[u8]) -> Vec<String> {
        String::from_utf8_lossy(src)
            .lines()
            .filter_map(|l| l.strip_prefix("ref:"))
            .map(str::to_owned)
            .collect()
    }
    fn write(&self, canon: &str, src: &[u8]) -> Result<Written, Failure> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if src.starts_with(b"BAD") {
            return Err(Failure::new("decode", "bad magic"));
        }
        let mut h = hex(src);
        if self.corrupt.as_deref() == Some(canon) {
            h.replace_range(0..1, "f");
        }
        Ok(Written {
            files: vec![
                NativeData {
                    name: format!("{canon}.hex"),
                    bytes: h.into_bytes(),
                },
                NativeData {
                    name: format!("{canon}.len"),
                    bytes: format!("{}\n", src.len()).into_bytes(),
                },
            ],
            notes: vec!["note".into()],
        })
    }
    fn check(&self, canon: &str, src: &[u8], native: &[NativeData]) -> Result<(), Failure> {
        let h = native
            .iter()
            .find(|n| n.name.ends_with(".hex"))
            .ok_or_else(|| Failure::new("C-STUB", "no .hex"))?;
        let want = hex(src);
        let got = String::from_utf8_lossy(&h.bytes);
        if let Some(at) = want.bytes().zip(got.bytes()).position(|(a, b)| a != b) {
            return Err(Failure::new(
                "C-STUB",
                format!("{canon}: first difference at hex digit {at}"),
            ));
        }
        Ok(())
    }
}

fn kinds(version: u32, corrupt: Option<&str>) -> (Vec<Box<dyn Kind>>, Arc<AtomicUsize>) {
    let writes = Arc::new(AtomicUsize::new(0));
    let stub = Stub {
        version,
        corrupt: corrupt.map(str::to_owned),
        writes: writes.clone(),
    };
    (vec![Box::new(Excel), Box::new(stub)], writes)
}

/// A scratch folder under the target dir, removed first.
fn scratch(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

/// Nine required archives; `d2data` has a listfile and 30 stub files, one
/// bad file, an excel table (shadowed by `patch_d2`), a file nothing
/// converts; `patch_d2` has no listfile and holds the file `one.stub`
/// names.
fn install(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    for name in d2_convert::convert::REQUIRED_ARCHIVES {
        let mut w = MpqWriter::new();
        if name == "d2data.mpq" {
            w = w.with_listfile();
            for i in 0..30 {
                w.add_file(
                    &format!(r"Data\Global\Stubs\s{i:02}.STUB"),
                    format!("payload {i}").into_bytes(),
                );
            }
            w.add_file(
                r"data\global\stubs\one.stub",
                b"ref:data\\global\\hidden\\two.stub\n".to_vec(),
            );
            w.add_file(r"data\global\stubs\bad.stub", b"BAD".to_vec());
            w.add_file(r"data\global\excel\weapons.txt", b"name\told\n".to_vec());
            w.add_file(r"data\video\intro.bik", b"video".to_vec());
        }
        if name == "patch_d2.mpq" {
            w.add_file(
                r"data\global\excel\weapons.txt",
                b"name\tpatched\n".to_vec(),
            );
            w.add_file(r"data\global\hidden\two.stub", b"hidden".to_vec());
        }
        w.write(dir.join(name)).unwrap();
    }
}

fn opts(install: &Path, out: &Path) -> Options {
    let mut o = Options::new(install, out);
    o.converter_commit = "test".into();
    o
}

fn snapshot(out: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = vec!["files.tsv".to_owned(), "manifest.toml".to_owned()];
    files.extend(
        walk_files(&out.join("base"))
            .unwrap()
            .into_iter()
            .map(|f| format!("base/{f}")),
    );
    files
        .into_iter()
        .map(|f| {
            let bytes = fs::read(join(out, &f)).unwrap();
            (f, bytes)
        })
        .collect()
}

#[test]
fn full_run_converts_checks_and_verifies() {
    let dir = scratch("full");
    install(&dir.join("game"));
    let (k, _) = kinds(1, None);
    let out = dir.join("native");
    fs::create_dir_all(dir.join("game")).unwrap();
    let s = convert(&opts(&dir.join("game"), &out), &k).unwrap();
    // The failing file is recorded, never moved into base/.
    assert_eq!(s.exit_code(), 1);
    assert_eq!(s.failures.len(), 1);
    assert_eq!(s.failures[0].0, "data/global/stubs/bad.stub");
    assert_eq!(s.failures[0].1, "decode");
    assert!(!out.join("base/data/global/stubs/bad.stub.hex").exists());
    assert!(out.join(".work/failed/data/global/stubs").is_dir());
    let m = s.manifest.unwrap();
    assert!(!m.complete);
    let c = m.counts["stub"];
    assert_eq!((c.converted, c.failed), (32, 1)); // 30 + one + hidden two
    assert_eq!(m.counts["excel"].converted, 1);
    assert_eq!(m.kinds["stub"], 1);
    // Winning copy (patch_d2) and the name found only through `ref:`.
    assert_eq!(
        fs::read(out.join("base/data/global/excel/weapons.txt")).unwrap(),
        b"name\tpatched\n"
    );
    assert!(out.join("base/data/global/hidden/two.stub.hex").is_file());
    let report = fs::read_to_string(out.join("report.txt")).unwrap();
    assert!(report.contains(".bik: 1"), "{report}");
    assert!(report.contains("bad.stub: decode: bad magic"), "{report}");
    assert!(report.contains("d2data.mpq"));
    // Archive identity is recorded, with no fixed hash compared.
    assert_eq!(m.archives.len(), 9);
    assert!(m
        .archives
        .iter()
        .all(|a| a.sha256.len() == 64 && a.size > 0));
    // verify: the recorded failure is reported, nothing else.
    let v = verify(
        &VerifyOptions {
            out: out.clone(),
            deep_install: Some(dir.join("game")),
            progress: false,
        },
        &k,
    );
    assert!(v.iter().any(|p| p.contains("complete = false")), "{v:?}");
    assert!(
        v.iter()
            .any(|p| p.contains("bad.stub") && p.contains("failed:decode")),
        "{v:?}"
    );
    assert_eq!(v.len(), 2, "{v:?}");
}

#[test]
fn missing_archive_is_not_an_install() {
    let dir = scratch("noinstall");
    install(&dir.join("game"));
    fs::remove_file(dir.join("game/d2xtalk.mpq")).unwrap();
    let (k, _) = kinds(1, None);
    let e = convert(&opts(&dir.join("game"), &dir.join("n")), &k).unwrap_err();
    assert!(
        matches!(e, ConvertError::NotAnInstall(ref m) if m.contains("d2xtalk.mpq")),
        "{e}"
    );
}

// Without the failing file a run completes and verifies clean.
fn clean_install(dir: &Path) {
    install(dir);
    let mut w = MpqWriter::new().with_listfile();
    for i in 0..30 {
        w.add_file(
            &format!(r"data\global\stubs\s{i:02}.stub"),
            format!("payload {i}").into_bytes(),
        );
    }
    w.add_file(
        r"data\global\stubs\one.stub",
        b"ref:data\\global\\hidden\\two.stub\n".to_vec(),
    );
    w.add_file(r"data\global\excel\weapons.txt", b"name\told\n".to_vec());
    w.write(dir.join("d2data.mpq")).unwrap();
}

#[test]
fn output_does_not_depend_on_thread_order() {
    let dir = scratch("det");
    clean_install(&dir.join("game"));
    let mut snaps = Vec::new();
    for (i, threads) in [1usize, 8, 3].into_iter().enumerate() {
        let (k, _) = kinds(1, None);
        let out = dir.join(format!("out{i}"));
        let mut o = opts(&dir.join("game"), &out);
        o.threads = threads;
        let s = convert(&o, &k).unwrap();
        assert_eq!(s.exit_code(), 0);
        assert!(s.manifest.unwrap().complete);
        snaps.push(snapshot(&out));
    }
    assert!(snaps[0].len() > 60);
    assert_eq!(snaps[0], snaps[1]);
    assert_eq!(snaps[0], snaps[2]);
    // No timestamps in the deterministic files.
    let m = String::from_utf8(
        snaps[0]
            .iter()
            .find(|(n, _)| n == "manifest.toml")
            .unwrap()
            .1
            .clone(),
    )
    .unwrap();
    assert!(!m.contains("UTC"));
    // A second run with nothing changed converts nothing.
    let (k, writes) = kinds(1, None);
    convert(&opts(&dir.join("game"), &dir.join("out0")), &k).unwrap();
    assert_eq!(writes.load(Ordering::SeqCst), 0);
    assert_eq!(snapshot(&dir.join("out0")), snaps[0]);
    // A new kind version redoes every file of that kind, only that kind.
    let (k2, writes2) = kinds(2, None);
    let s = convert(&opts(&dir.join("game"), &dir.join("out0")), &k2).unwrap();
    assert_eq!(writes2.load(Ordering::SeqCst), 32);
    assert_eq!(s.manifest.unwrap().kinds["stub"], 2);
    // `--force` redoes all.
    let mut o = opts(&dir.join("game"), &dir.join("out0"));
    o.force = true;
    convert(&o, &k2).unwrap();
    assert_eq!(writes2.load(Ordering::SeqCst), 64);
}

#[test]
fn one_perturbed_file_is_the_only_failure() {
    let dir = scratch("perturb");
    clean_install(&dir.join("game"));
    let target = "data/global/stubs/s07.stub";
    let (k, _) = kinds(1, Some(target));
    let out = dir.join("native");
    let s = convert(&opts(&dir.join("game"), &out), &k).unwrap();
    assert_eq!(s.exit_code(), 1);
    assert_eq!(s.failures.len(), 1);
    let (path, check, detail) = &s.failures[0];
    assert_eq!(path, target);
    assert_eq!(check, "C-STUB");
    assert!(
        detail.contains("first difference at hex digit 0"),
        "{detail}"
    );
    assert!(!out.join("base").join(format!("{target}.hex")).exists());
    assert!(out.join("base/data/global/stubs/s08.stub.hex").is_file());
    assert!(!s.manifest.unwrap().complete);

    // A clean run, then one native byte changed on disk: verify names that
    // file and no other.
    let out2 = dir.join("native2");
    let (k, _) = kinds(1, None);
    let s = convert(&opts(&dir.join("game"), &out2), &k).unwrap();
    assert!(s.manifest.unwrap().complete);
    let vo = VerifyOptions {
        out: out2.clone(),
        deep_install: Some(dir.join("game")),
        progress: false,
    };
    assert_eq!(verify(&vo, &k), Vec::<String>::new());
    let f = out2.join("base/data/global/stubs/s03.stub.hex");
    let mut bytes = fs::read(&f).unwrap();
    bytes[2] = b'0';
    fs::write(&f, bytes).unwrap();
    let v = verify(&vo, &k);
    assert_eq!(v.len(), 2, "{v:?}");
    assert!(v.iter().all(|p| p.contains("s03.stub")), "{v:?}");
    // The quick (non-deep) verify finds it by hash.
    let quick = verify(
        &VerifyOptions {
            deep_install: None,
            ..vo
        },
        &k,
    );
    assert_eq!(quick.len(), 1);
    assert!(quick[0].contains("SHA-256 differs"), "{quick:?}");
}

#[test]
fn killed_run_resumes_to_the_same_bytes() {
    let dir = scratch("resume");
    clean_install(&dir.join("game"));
    let (k, _) = kinds(1, None);
    let mut o = opts(&dir.join("game"), &dir.join("whole"));
    o.threads = 2;
    convert(&o, &k).unwrap();
    let whole = snapshot(&dir.join("whole"));

    for stop in [1usize, 17, 33] {
        let out = dir.join(format!("cut{stop}"));
        let mut o = opts(&dir.join("game"), &out);
        o.threads = 4;
        o.stop_after = Some(stop);
        let s = convert(&o, &k).unwrap();
        assert!(!s.finished);
        // Killed: no manifest, and no file in base/ without its row.
        assert!(!out.join("manifest.toml").exists());
        let tsv = d2_native::manifest::parse_files_tsv(
            &fs::read_to_string(out.join("files.tsv")).unwrap(),
            true,
        )
        .unwrap();
        assert!(tsv.len() >= stop.min(34) - 1, "{stop}: {}", tsv.len());
        let named: std::collections::BTreeSet<String> = tsv
            .iter()
            .flat_map(|r| r.native.iter().map(|n| n.name.clone()))
            .collect();
        for f in walk_files(&out.join("base")).unwrap() {
            assert!(named.contains(&f), "{f} in base/ without a row");
        }
        // A file renamed into base/ whose row never got written, and a
        // torn last row: both are cleaned up by the rerun.
        let orphan = out.join("base/data/global/stubs/orphan.stub.hex");
        fs::create_dir_all(orphan.parent().unwrap()).unwrap();
        fs::write(&orphan, b"x").unwrap();
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(out.join("files.tsv"))
            .unwrap();
        std::io::Write::write_all(&mut f, b"data/global/stubs/torn.stub\tstu").unwrap();
        drop(f);
        let mut o = opts(&dir.join("game"), &out);
        o.threads = 1;
        let s = convert(&o, &k).unwrap();
        assert!(s.finished && s.exit_code() == 0);
        assert_eq!(snapshot(&out), whole, "stop after {stop}");
    }
}
