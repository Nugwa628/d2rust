//! Wide text of a string key, the bounded wide copy, and the tile path fix.
//! `records` uses these to fill the wide name fields and tile path
//! fields of records; failures are reported as [`FixupError`].

use super::{err, FixupError};
use crate::strings::StringTables;

/// The miss suffix as stored; the conversion drops its last character.
const MISS_SUFFIX: &[u8] = b" -not xlated call ken w";
/// Tile path prefix.
pub const TILE_PREFIX: &[u8] = b"DATA\\GLOBAL\\TILES\\";
/// Tile path field and format buffer sizes.
const PATH_FIELD: usize = 60;
const PATH_BUFFER: usize = 64;

/// Wide text of `key`, as UCS-2 units: empty key → empty; hit
/// (patch → expansion → base) → the text; miss → key + the suffix minus
/// its last character. A byte ≥ 0x80 is an error.
pub fn wide_text(strings: &StringTables, table: &str, key: &[u8]) -> Result<Vec<u16>, FixupError> {
    if key.is_empty() {
        return Ok(Vec::new());
    }
    let hit = [&strings.patch, &strings.expansion, &strings.base]
        .into_iter()
        .flatten()
        .find_map(|t| t.get(key));
    let bytes: Vec<u8> = match hit {
        Some(text) => text.to_vec(),
        None => {
            let mut s = key.to_vec();
            s.extend_from_slice(MISS_SUFFIX);
            s.pop();
            s
        }
    };
    if let Some(b) = bytes.iter().find(|&&b| b >= 0x80) {
        return Err(err(
            table,
            format!(
                "text of {:?} has byte {b:#04x} (fixups.md open question 1)",
                String::from_utf8_lossy(key)
            ),
        ));
    }
    Ok(bytes.into_iter().map(u16::from).collect())
}

/// Bounded wide copy: `text` into `units` u16 at `dst`, stopping at
/// `units`, the rest zero-filled.
pub fn copy_wide(r: &mut [u8], dst: usize, text: &[u16], units: usize) {
    for k in 0..units {
        let u = text.get(k).copied().unwrap_or(0);
        r[dst + 2 * k..dst + 2 * k + 2].copy_from_slice(&u.to_le_bytes());
    }
}

/// The path fix on the 60-byte string field at `at`: `/` → `\`,
/// then the prefix for strings of 2 or more characters. Bytes after the
/// new NUL keep their values. A result that does not fit the field is an
/// error.
pub fn fix_path(r: &mut [u8], at: usize, table: &str) -> Result<(), FixupError> {
    let field = &mut r[at..at + PATH_FIELD];
    let len = field
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| err(table, format!("tile path at +{at:#X} has no NUL")))?;
    for b in &mut field[..len] {
        if *b == b'/' {
            *b = b'\\';
        }
    }
    if len > 1 {
        let total = TILE_PREFIX.len() + len;
        if total + 1 > PATH_FIELD.min(PATH_BUFFER) {
            return Err(err(
                table,
                format!("tile path at +{at:#X}: {len} characters, at most 41 fit"),
            ));
        }
        let mut s = TILE_PREFIX.to_vec();
        s.extend_from_slice(&field[..len]);
        s.push(0);
        field[..s.len()].copy_from_slice(&s);
    }
    Ok(())
}
