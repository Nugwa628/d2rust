//! `d2s-tool`: generates, edits, dumps and round-trip-checks `.d2s`
//! character saves for local game testing, on the user's own tables
//! (`--game-dir` or `D2_GAME_DIR`). Saves are never committed; tests
//! build them in memory on the synthetic install.
//!
//! - [`tables`]: the loaded tables and the [`d2_formats::d2s::SaveTables`]
//!   view (itemstatcost save columns, item entry lengths).
//! - [`save`]: a full save from flags (`new`) and edits (`set`).
//! - [`items`]: one inventory item from a code.
//! - [`dump`]: the listing.
//! - [`cli`]: the command line.
//!
//! Status: a tool; its output is checked by reading it back through
//! `d2_formats::d2s::read` and the item reader of `d2-proto`. Whether the
//! game accepts the files is a local-run check (`docs/LOCAL-RUN.md`).

pub mod cli;
pub mod dump;
pub mod items;
pub mod save;
pub mod tables;

use d2_formats::d2s::{self, D2s, D2sError, ReadOptions, SaveTables};

/// Read options for a file: expansion game when forced, else when the
/// character's status has 0x20.
pub fn read_options(bytes: &[u8], force_expansion: Option<bool>) -> ReadOptions {
    let st = bytes
        .get(0x24..0x26)
        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
    ReadOptions {
        expansion: force_expansion.unwrap_or(st & d2s::status::EXPANSION != 0),
        game: None,
    }
}

/// What a read → write round trip found.
#[derive(Debug)]
pub enum RoundTrip {
    /// The rewrite is byte for byte the file.
    Same(D2s),
    /// First differing offset, the two lengths.
    Differs {
        save: D2s,
        offset: usize,
        file_len: usize,
        rewrite_len: usize,
    },
}

/// Reads `bytes`, writes the model back and compares (the real-save
/// check: the model keeps +0x30 and the corpse u32, so no
/// difference is allowed).
pub fn round_trip(
    bytes: &[u8],
    opts: &ReadOptions,
    t: &dyn SaveTables,
) -> anyhow::Result<RoundTrip> {
    let save = d2s::read(bytes, opts, t)
        .map_err(|e: D2sError| anyhow::anyhow!("read: {e} (load result {:?})", e.result()))?;
    let again = d2s::write(&save, t).map_err(|e| anyhow::anyhow!("write: {e}"))?;
    if again == bytes {
        return Ok(RoundTrip::Same(save));
    }
    let offset = again
        .iter()
        .zip(bytes)
        .position(|(a, b)| a != b)
        .unwrap_or(again.len().min(bytes.len()));
    Ok(RoundTrip::Differs {
        save,
        offset,
        file_len: bytes.len(),
        rewrite_len: again.len(),
    })
}
