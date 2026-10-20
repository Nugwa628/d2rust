// D2RUST's own: dev tools.
//! Named save slots (`save <name>` / `load <name>`): the character save
//! the server writes, plus where the player stood, in
//! `<save folder>/dev-slots/<name>/` (`char.d2s`, `slot.txt`). A load
//! puts the slot's character back as the save file and starts the game
//! again on the slot's seed and difficulty; the dev server then takes the
//! player back to the slot's level and position ([`DevStart`]). The
//! world itself comes from the seed: monsters and items on the ground are
//! not kept.

use std::path::{Path, PathBuf};

use d2_proto::dev::DevSwitchState;

/// The slot format's version (METHODS M20).
pub const SLOT_VERSION: u32 = 1;
/// The first word of a slot file.
pub const HEADER: &str = "d2rust-dev-slot";
/// The first word earlier builds wrote (legacy-name): still read, so slots
/// saved before the rename keep loading; never written.
pub const LEGACY_HEADER: &str = "d2rs-dev-slot";

/// Where a loaded game takes the player, and the switches it turns on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DevStart {
    pub level: u32,
    pub x: i32,
    pub y: i32,
    pub switches: DevSwitchState,
}

/// One slot's `slot.txt`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    pub seed: u32,
    pub difficulty: u8,
    pub start: DevStart,
}

/// A slot name: 1–32 of `A–Z a–z 0–9 - _`.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 32
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!("slot name {name:?}: 1-32 of A-Z a-z 0-9 - _"));
    }
    Ok(())
}

/// The slot's folder.
pub fn slot_dir(base: &Path, name: &str) -> PathBuf {
    base.join("dev-slots").join(name)
}

impl Slot {
    pub fn to_text(&self) -> String {
        let s = &self.start;
        let w = &s.switches;
        format!(
            "{HEADER} {SLOT_VERSION}\nseed {}\ndifficulty {}\nlevel {}\npos {} {}\n\
             switches {} {} {} {}\n",
            self.seed,
            self.difficulty,
            s.level,
            s.x,
            s.y,
            u8::from(w.god),
            u8::from(w.nocd),
            u8::from(w.freemana),
            w.speed
        )
    }

    /// Strict: the version line, then every key once, in this order.
    pub fn parse(text: &str) -> Result<Self, String> {
        let renamed;
        let text = match text.strip_prefix(LEGACY_HEADER) {
            Some(rest) => {
                renamed = format!("{HEADER}{rest}");
                renamed.as_str()
            }
            None => text,
        };
        let mut lines = text.lines();
        let mut next = |key: &str| -> Result<Vec<i64>, String> {
            let l = lines.next().ok_or_else(|| format!("slot: no {key} line"))?;
            let mut t = l.split_ascii_whitespace();
            if t.next() != Some(key) {
                return Err(format!("slot: expected {key}, got {l:?}"));
            }
            t.map(|v| v.parse::<i64>().map_err(|_| format!("slot: {key}: {v:?}")))
                .collect()
        };
        let v = next(HEADER)?;
        if v != [i64::from(SLOT_VERSION)] {
            return Err(format!(
                "slot: version {v:?}, this build reads {SLOT_VERSION}"
            ));
        }
        let one = |v: Vec<i64>, k: &str| match v.as_slice() {
            [n] => Ok(*n),
            _ => Err(format!("slot: {k} takes one value")),
        };
        let seed = one(next("seed")?, "seed")? as u32;
        let difficulty = one(next("difficulty")?, "difficulty")?;
        if !(0..=2).contains(&difficulty) {
            return Err(format!("slot: difficulty {difficulty}"));
        }
        let level = one(next("level")?, "level")? as u32;
        let [x, y] = next("pos")?[..] else {
            return Err("slot: pos takes two values".into());
        };
        let [god, nocd, freemana, speed] = next("switches")?[..] else {
            return Err("slot: switches takes four values".into());
        };
        Ok(Slot {
            seed,
            difficulty: difficulty as u8,
            start: DevStart {
                level,
                x: x as i32,
                y: y as i32,
                switches: DevSwitchState {
                    god: god != 0,
                    nocd: nocd != 0,
                    freemana: freemana != 0,
                    speed: speed.clamp(0, 1000) as u32,
                },
            },
        })
    }
}

/// `save <name>`: copies the character save just written (`save_file`)
/// and writes `slot.txt`.
pub fn write_slot(
    base: &Path,
    name: &str,
    save_file: &Path,
    slot: &Slot,
) -> Result<PathBuf, String> {
    check_name(name)?;
    let dir = slot_dir(base, name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::copy(save_file, dir.join("char.d2s"))
        .map_err(|e| format!("copying {}: {e}", save_file.display()))?;
    std::fs::write(dir.join("slot.txt"), slot.to_text())
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// Reads a slot's `slot.txt` (the folder must hold `char.d2s` too).
pub fn read_slot(dir: &Path) -> Result<Slot, String> {
    if !dir.join("char.d2s").is_file() {
        return Err(format!("{}: no char.d2s", dir.display()));
    }
    let text = std::fs::read_to_string(dir.join("slot.txt"))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    Slot::parse(&text)
}

/// The slot names under `base`, sorted.
pub fn list_slots(base: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(base.join("dev-slots"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.path().join("slot.txt").is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_round_trip_strictly() {
        let s = Slot {
            seed: 1234,
            difficulty: 1,
            start: DevStart {
                level: 40,
                x: 5153,
                y: -2,
                switches: DevSwitchState {
                    god: true,
                    nocd: false,
                    freemana: true,
                    speed: 200,
                },
            },
        };
        assert_eq!(Slot::parse(&s.to_text()), Ok(s));
        let bad = s
            .to_text()
            .replace("d2rust-dev-slot 1", "d2rust-dev-slot 2");
        assert!(Slot::parse(&bad).unwrap_err().contains("version"));
        // A slot saved before the rename still loads.
        let old = s.to_text().replace(HEADER, LEGACY_HEADER);
        assert_eq!(Slot::parse(&old), Ok(s));
        let bad = s.to_text().replace("pos 5153 -2", "pos 5153");
        assert!(Slot::parse(&bad).is_err());
        assert!(check_name("town-2_a").is_ok());
        assert!(check_name("../x").is_err() && check_name("").is_err());

        let base = std::env::temp_dir().join(format!("d2rust-slots-{}", std::process::id()));
        let save = base.join("Dev.d2s");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(&save, b"d2s bytes").unwrap();
        let dir = write_slot(&base, "a1", &save, &s).unwrap();
        assert_eq!(read_slot(&dir), Ok(s));
        assert_eq!(std::fs::read(dir.join("char.d2s")).unwrap(), b"d2s bytes");
        assert_eq!(list_slots(&base), vec!["a1".to_string()]);
        let _ = std::fs::remove_dir_all(&base);
    }
}
