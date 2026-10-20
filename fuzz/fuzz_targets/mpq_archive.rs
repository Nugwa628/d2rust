#![no_main]
use libfuzzer_sys::fuzz_target;

// Writes the input to a per-process temp file and opens it as an archive:
// header, hash and block tables; then reads every block (up to 64), the
// listfile and a few names, with no key and with a key taken from the input.
use d2_formats::mpq::Archive;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

fn path() -> &'static PathBuf {
    static P: OnceLock<PathBuf> = OnceLock::new();
    P.get_or_init(|| std::env::temp_dir().join(format!("d2fuzz-{}.mpq", std::process::id())))
}

fuzz_target!(|data: &[u8]| {
    d2_fuzz::arm("mpq_archive", data);
    let p = path();
    let Ok(mut f) = std::fs::File::create(p) else {
        return;
    };
    if f.write_all(data).is_err() {
        return;
    }
    drop(f);
    let Ok(a) = Archive::open(p) else {
        return;
    };
    let _ = a.sector_size();
    let _ = a.find("(listfile)");
    let _ = a.contains("a\\b.txt");
    let _ = a.listfile();
    let key = data
        .get(8..12)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    for i in 0..a.block_table().len().min(64) {
        let _ = a.read_block(i, None);
        let _ = a.read_block(i, key);
        let _ = a.recover_key(i);
    }
    let _ = a.read("(listfile)");
});
