//! The client's unique-name draw from the name seed and the
//! init-owned fields of the monster assign message 0xAC. The
//! message header and the order of its other fields are
//! `server-messages.tsv`'s.

use crate::rng::Seed;

use super::{mode, type_flag, MonsterData};

/// last_sent_hp_pct at spawn: the life byte of the first assign.
pub const LIFE_AT_SPAWN: u8 = 128;

/// The mode sent: skill1, skill2, death and dead as they are, else 1.
pub fn assign_mode(m: u32) -> u8 {
    match m {
        mode::SKILL1 | mode::SKILL2 | mode::DEATH | mode::DEAD => m as u8,
        _ => mode::NEUTRAL as u8,
    }
}

/// Bits of one component: 1 when the choice count is < 3, else the bit
/// length of count − 1.
pub fn component_bits(count: u8) -> u32 {
    if count < 3 {
        1
    } else {
        u32::BITS - u32::from(count - 1).leading_zeros()
    }
}

/// The 16 components as (value, bits) pairs, `None` when all are 0
/// (the field is omitted). The reader learns it from 1 presence bit
/// before the field (full layout step 3; [`write_components`]).
pub fn components_field(components: &[u8; 16], counts: &[u8; 16]) -> Option<Vec<(u32, u32)>> {
    if components.iter().all(|&c| c == 0) {
        return None;
    }
    Some(
        components
            .iter()
            .zip(counts)
            .map(|(&c, &n)| (u32::from(c), component_bits(n)))
            .collect(),
    )
}

/// The components part of 0xAC (full layout step 3): 1 bit = any of
/// the 16 component bytes ≠ 0; set → each byte in its
/// width ([`component_bits`] of the class's choice count); clear → no
/// component bits.
pub fn write_components(w: &mut BitWriter, components: &[u8; 16], counts: &[u8; 16]) {
    match components_field(components, counts) {
        Some(f) => {
            w.write(1, 1);
            for (v, n) in f {
                w.write(v, n);
            }
        }
        None => w.write(0, 1),
    }
}

/// A bit writer, low bit first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BitWriter {
    pub bytes: Vec<u8>,
    pub bits: usize,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes the low `n` bits of `v` (n ≤ 32), low bit first.
    pub fn write(&mut self, v: u32, n: u32) {
        for i in 0..n {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (v >> i) & 1 != 0 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 1 << (self.bits % 8);
            }
            self.bits += 1;
        }
    }
}

/// The boss section of 0xAC, written only when the monster has umods
/// (returns false otherwise): type flags champion, unique, superunique,
/// minion, ghostly (one bit each, in that order); hcIdx (16 bits,
/// superunique only); umods (8 bits each, 0-terminated); name seed (16
/// bits).
pub fn write_boss_section(w: &mut BitWriter, m: &MonsterData) -> bool {
    let list = m.umod_list();
    if list.is_empty() {
        return false;
    }
    for f in [
        type_flag::CHAMPION,
        type_flag::UNIQUE,
        type_flag::SUPERUNIQUE,
        type_flag::MINION,
        type_flag::GHOSTLY,
    ] {
        w.write(u32::from(m.has_flag(f)), 1);
    }
    if m.has_flag(type_flag::SUPERUNIQUE) {
        w.write(u32::from(m.boss_hc_idx), 16);
    }
    for &u in list {
        w.write(u32::from(u), 8);
    }
    w.write(0, 8);
    w.write(u32::from(m.name_seed), 16);
    true
}

/// Row indices of a unique's name. `appellation` is set when the
/// `roll(100)` gave < 50.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UniqueName {
    pub prefix: u32,
    pub suffix: u32,
    pub appellation: Option<u32>,
}

/// The client draw on the seed {name seed, 666} for a
/// monster without a fixed name: suffix, prefix, roll(100); if < 50:
/// appellation, suffix, prefix (the second pair replaces the first).
pub fn unique_name(name_seed: u16, suffixes: i32, prefixes: i32, appellations: i32) -> UniqueName {
    let mut s = Seed::init_low(u32::from(name_seed));
    let mut suffix = s.roll(suffixes);
    let mut prefix = s.roll(prefixes);
    let mut appellation = None;
    if s.roll(100) < 50 {
        appellation = Some(s.roll(appellations));
        suffix = s.roll(suffixes);
        prefix = s.roll(prefixes);
    }
    UniqueName {
        prefix,
        suffix,
        appellation,
    }
}
