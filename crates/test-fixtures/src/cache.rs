//! Content-addressed synthetic installs shared by nextest processes.
//!
//! Only the archive build is shared. Each caller receives copies so tests may
//! patch archives without changing another test's inputs. OS file locks release
//! on process exit; a completion marker is written only after all stages pass.

use std::fs::{self, File};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::install::{self, FixtureError};
use crate::synth::Synthetic;

const ARCHIVES: [&str; 3] = ["d2data.mpq", "d2exp.mpq", "patch_d2.mpq"];

fn io<T>(path: &Path, result: std::io::Result<T>) -> Result<T, FixtureError> {
    result.map_err(|error| FixtureError::Io(path.to_owned(), error))
}

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

/// Hash every archive input, preserving order and boundaries, plus the builder
/// fingerprint (fixture, compiler, schema and archive writer sources).
pub fn input_hash(data: &Synthetic) -> String {
    let mut hash = Sha256::new();
    field(&mut hash, env!("FIXTURE_BUILDER_HASH").as_bytes());
    field(&mut hash, &(data.tables.files.len() as u64).to_le_bytes());
    for (name, bytes) in data.tables.render() {
        field(&mut hash, name.as_bytes());
        field(&mut hash, &bytes);
    }
    for strings in [
        &data.strings.base,
        &data.strings.expansion,
        &data.strings.patch,
    ] {
        field(
            &mut hash,
            &crate::tbl::write(
                &strings
                    .iter()
                    .map(|(key, value)| (key.as_bytes(), value.as_bytes()))
                    .collect::<Vec<_>>(),
            ),
        );
    }
    field(&mut hash, &data.soundenviron);
    field(&mut hash, &crate::animdata::write(&data.animdata));
    for files in [&data.files, &data.stored] {
        field(&mut hash, &(files.len() as u64).to_le_bytes());
        for (name, bytes) in files {
            field(&mut hash, name.as_bytes());
            field(&mut hash, bytes);
        }
    }
    format!("{:x}", hash.finalize())
}

/// Build once per input hash under `target_tmp`, then copy the three archives to
/// `dir`. Pass `Path::new(env!("CARGO_TARGET_TMPDIR"))` from integration tests.
/// The cache is versioned and contains synthetic data only.
pub fn copy_install(target_tmp: &Path, dir: &Path, data: &Synthetic) -> Result<(), FixtureError> {
    copy_from(&target_tmp.join("synthetic-install-cache-v1"), dir, data)
}

fn copy_from(root: &Path, dir: &Path, data: &Synthetic) -> Result<(), FixtureError> {
    io(root, fs::create_dir_all(root))?;
    let key = input_hash(data);
    let lock_path = root.join(format!("{key}.lock"));
    let lock = io(
        &lock_path,
        File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path),
    )?;
    io(&lock_path, lock.lock())?;
    let entry = root.join(key);
    let ready = entry.join("ready-v1");
    if !ready.is_file() || ARCHIVES.iter().any(|name| !entry.join(name).is_file()) {
        if entry.exists() {
            io(&entry, fs::remove_dir_all(&entry))?;
        }
        install::write_text_install(&entry, data)?;
        install::compile_and_pack(&entry, data)?;
        let archives = d2_formats::mpq::ArchiveSet::open_dir(&entry)?;
        d2_data::bin::load(&archives, d2_data::bin::DEFAULT_LANGUAGE)?;
        drop(archives);
        io(&ready, fs::write(&ready, b"synthetic-install 1\n"))?;
    }
    io(dir, fs::create_dir_all(dir))?;
    for name in ARCHIVES {
        let destination = dir.join(name);
        io(&destination, fs::copy(entry.join(name), &destination))?;
    }
    // Keep the lock until copying ends, so incomplete entries cannot be rebuilt
    // while a caller is reading them. Dropping the file releases the OS lock.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_input_changes_the_key() {
        let data = crate::synth::synthetic();
        let key = input_hash(&data);
        let mut variants = Vec::new();
        let mut changed = data.clone();
        changed
            .tables
            .files
            .values_mut()
            .next()
            .unwrap()
            .rows
            .push(vec!["changed".into()]);
        variants.push(changed);
        for group in 0..3 {
            let mut changed = data.clone();
            let strings = match group {
                0 => &mut changed.strings.base,
                1 => &mut changed.strings.expansion,
                _ => &mut changed.strings.patch,
            };
            strings.push(("new".into(), "value".into()));
            variants.push(changed);
        }
        let mut changed = data.clone();
        changed.soundenviron.push(0);
        variants.push(changed);
        let mut changed = data.clone();
        changed.animdata.clear();
        variants.push(changed);
        for stored in [false, true] {
            let mut changed = data.clone();
            let files = if stored {
                &mut changed.stored
            } else {
                &mut changed.files
            };
            files.push(("new".into(), vec![1, 2]));
            variants.push(changed);
        }
        assert_eq!(key, input_hash(&data));
        for changed in variants {
            assert_ne!(key, input_hash(&changed));
        }
    }

    #[test]
    fn cache_copies_are_isolated_and_incomplete_entries_recover() {
        let root = std::env::temp_dir().join(format!("fixture-cache-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let data = crate::synth::synthetic();
        let cache = root.join("cache");
        let entry = cache.join(input_hash(&data));
        fs::create_dir_all(&entry).unwrap();
        fs::write(entry.join("partial"), b"interrupted").unwrap();
        let first = root.join("first");
        copy_from(&cache, &first, &data).unwrap();
        assert!(!entry.join("partial").exists());
        let ready_time = fs::metadata(entry.join("ready-v1"))
            .unwrap()
            .modified()
            .unwrap();
        fs::write(first.join(ARCHIVES[0]), b"modified by a test").unwrap();
        let second = root.join("second");
        copy_from(&cache, &second, &data).unwrap();
        assert_eq!(
            ready_time,
            fs::metadata(entry.join("ready-v1"))
                .unwrap()
                .modified()
                .unwrap()
        );
        assert_eq!(
            fs::read(second.join(ARCHIVES[0])).unwrap(),
            fs::read(entry.join(ARCHIVES[0])).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
