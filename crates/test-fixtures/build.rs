//! Fingerprint the synthetic install builder and its schema/compiler inputs.
use sha2::{Digest, Sha256};
use std::path::Path;

fn sources(path: &Path, hash: &mut Sha256) {
    println!("cargo:rerun-if-changed={}", path.display());
    let name = path.to_string_lossy();
    hash.update((name.len() as u64).to_le_bytes());
    hash.update(name.as_bytes());
    if path.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for entry in entries {
            sources(&entry, hash);
        }
    } else {
        let bytes = std::fs::read(path).unwrap();
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
}

fn main() {
    let mut hash = Sha256::new();
    hash.update(b"synthetic-install-builder 1");
    for input in [
        "src",
        "build.rs",
        "../../Cargo.lock",
        "../../Cargo.toml",
        "Cargo.toml",
        "../d2-data/Cargo.toml",
        "../d2-formats/Cargo.toml",
        "../d2-data/src",
        "../d2-formats/src",
        "../../tables",
    ] {
        sources(Path::new(input), &mut hash);
    }
    println!("cargo:rustc-env=FIXTURE_BUILDER_HASH={:x}", hash.finalize());
}
