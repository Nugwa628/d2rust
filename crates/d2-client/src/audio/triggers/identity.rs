//! The sound identity of a unit and the choice of its `monsounds` record
//! (the unit composite owns the substitution table). Plain integers; the
//! callers pass the table reads.

use super::{MONSTER, PLAYER};

/// Player→monster mode map T1 (by player mode 0…19), the unit
/// composite's substitution.
pub const PLAYER_TO_MONSTER_MODE: [u8; 20] = [
    0, 1, 2, 15, 3, 1, 2, 4, 5, 6, 7, 4, 11, 8, 9, 10, 11, 12, 14, 13,
];

/// Monster mode numbers used by the fallback.
const NU: u8 = 1;
const WL: u8 = 2;
const A1: u8 = 4;
const GH: u8 = 3;
const S1: u8 = 8;

/// The fallback of a monster mode whose `monstats2` mode bit is clear.
fn fallback(m: u8) -> u8 {
    match m {
        2..=4 => NU,   // WL, GH, A1
        5 => A1,       // A2
        6 => GH,       // BL
        7 => A1,       // SC
        8 => NU,       // S1
        9..=11 => S1,  // S2, S3, S4
        12..=14 => NU, // DD, KB, SQ
        15 => WL,      // RN
        _ => NU,       // DT and anything else
    }
}

/// The first state of the unit with `gfxtype` 1 or 2: its
/// `gfxtype` and `gfxclass`. `None` when flag-ex bit 3 is clear or no such
/// state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawSubstitution {
    pub gfxtype: u8,
    pub gfxclass: i32,
}

/// The (type, class, mode) the sound rules read. `seq_mode` is the
/// sequence mode (+0x40) of a running sequence (+0x30 ≠ 0); it applies to
/// types 0 and 1. Only a player (type 0) goes through the substitution;
/// `mode_bit(class, m)` is the `monstats2` mode bit of the player→monster
/// map's fallback loop.
pub fn sound_identity(
    unit_type: u8,
    class: i32,
    mode: u8,
    seq_mode: Option<u8>,
    subst: Option<DrawSubstitution>,
    mode_bit: &dyn Fn(i32, u8) -> bool,
) -> (u8, i32, u8) {
    let mode = if unit_type <= 1 {
        seq_mode.unwrap_or(mode)
    } else {
        mode
    };
    if unit_type != PLAYER {
        return (unit_type, class, mode);
    }
    let Some(sub) = subst else {
        return (unit_type, class, mode);
    };
    match sub.gfxtype {
        1 => {
            // Player→monster.
            let mut m = PLAYER_TO_MONSTER_MODE
                .get(usize::from(mode))
                .copied()
                .unwrap_or(0);
            while m != NU && !mode_bit(sub.gfxclass, m) {
                m = fallback(m);
            }
            (MONSTER, sub.gfxclass, m)
        }
        // gfxtype 2: type 0 stays a player; the mode map applies to
        // monsters only.
        2 => (PLAYER, sub.gfxclass, mode),
        _ => (unit_type, class, mode),
    }
}

/// The `monstats`/`superuniques` reads of the record choice.
#[derive(Clone, Copy, Debug, Default)]
pub struct RecordInputs {
    /// Raw unit type (+0x00).
    pub raw_type: u8,
    /// Identity class c.
    pub class: i32,
    /// Client monster data type flags (+0x16): 2 superunique, 8 unique,
    /// 0x10 minion.
    pub type_flags: u8,
    /// `superuniques` row [monster data +0x26] `MonSound` when the row
    /// exists.
    pub superunique_monsound: Option<i32>,
    /// `monstats` row c's `UMonSound` (i16).
    pub umonsound: i32,
    /// `monstats` row c's `MonSound` (i16).
    pub monsound: i32,
    /// `monstats` row count and `monsounds` row count.
    pub monstats_rows: i32,
    pub monsounds_rows: i32,
}

/// The `monsounds` row of the unit, `None` for no
/// record.
pub fn monsounds_row(r: &RecordInputs) -> Option<i32> {
    // c outside the row count → row 0 (the empty first data row).
    if r.class < 0 || r.class >= r.monstats_rows {
        return Some(0);
    }
    // Raw type 1 only.
    if r.raw_type == MONSTER {
        if r.type_flags & 2 != 0 {
            if let Some(m) = r.superunique_monsound.filter(|&m| m > 0) {
                return Some(m);
            }
        }
        if r.type_flags & (8 | 0x10) != 0 && r.umonsound > 0 {
            return Some(r.umonsound);
        }
    }
    // Step 3.3.
    (0..r.monsounds_rows)
        .contains(&r.monsound)
        .then_some(r.monsound)
}
