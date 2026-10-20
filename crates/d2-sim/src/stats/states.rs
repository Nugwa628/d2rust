//! State bits on a unit's extended list, the toggle and
//! the queries. The state data comes from the states table and
//! its runtime flag bitsets.

use d2_data::bin::BinTable;
use d2_data::fixup::maps::{StateMaps, STATE_FLAGS};
use d2_data::tables::{decode_all, States, WrongTable};

use super::lists::{owner, StatLists};
use crate::units::UnitId;

/// State ids the simulation names (the base game states.txt rows).
pub mod state {
    pub const FREEZE: u32 = 1;
    pub const POISON: u32 = 2;
    pub const AI_DELAY: u32 = 21;
    pub const PREVENTHEAL: u32 = 52;
    pub const UNINTERRUPTABLE: u32 = 54;
    pub const OPENWOUNDS: u32 = 62;
    pub const NOMANAREGEN: u32 = 85;
    pub const DEATH_DELAY: u32 = 92;
    pub const HEALTHPOT: u32 = 100;
    pub const MANAPOT: u32 = 106;
}

/// State flag groups (states flag bit k = bitset k).
pub mod group {
    /// `hide` (states flag bit 2; data tables +0xD4).
    pub const HIDE: usize = 2;
    /// `disguise`.
    pub const DISGUISE: usize = 16;
    /// `plrstaydeath` (states flag bit 13; data tables +0x100).
    pub const PLR_STAY_DEATH: usize = 13;
    /// `monstaydeath` (bit 14; +0x104).
    pub const MON_STAY_DEATH: usize = 14;
    /// `bossstaydeath` (bit 15; +0x108).
    pub const BOSS_STAY_DEATH: usize = 15;
    /// `life`.
    pub const LIFE: usize = 32;
}

/// The states table as the stat code reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateTable {
    count: usize,
    words: usize,
    /// 40 bitsets of `words` u32.
    bitsets: Vec<u32>,
    /// `srvactivefunc` (u16 +0x36) by state.
    srvactivefunc: Vec<u16>,
    /// (state, `gfxtype` +0x2D, `gfxclass` +0x2E) of the `disguise`
    /// states whose `gfxtype` is 1 or 2, ascending: the draw identity's
    /// list.
    gfx: Vec<(u32, u8, u16)>,
}

impl StateTable {
    /// From the states table and its runtime maps
    /// (`d2_data::fixup::maps::states`).
    pub fn new(states: &BinTable, maps: &StateMaps) -> Result<Self, WrongTable> {
        let rows = decode_all::<States>(states)?;
        Ok(Self {
            count: rows.len(),
            words: maps.words,
            bitsets: maps.bitsets.clone(),
            srvactivefunc: rows.iter().map(|r| r.srvactivefunc).collect(),
            gfx: rows
                .iter()
                .enumerate()
                .filter(|(s, r)| {
                    let disguise = maps
                        .bitsets
                        .get(group::DISGUISE * maps.words + s / 32)
                        .is_some_and(|w| w & 1 << (s % 32) != 0);
                    disguise && matches!(r.gfxtype, 1 | 2)
                })
                .map(|(s, r)| (s as u32, r.gfxtype, r.gfxclass))
                .collect(),
        })
    }

    /// States count (`[0x744304]+0xC4`).
    pub fn count(&self) -> usize {
        self.count
    }

    /// W: words per half of a list's state bits.
    pub fn words(&self) -> usize {
        self.words
    }

    /// Whether state `s` has flag group `g`.
    pub fn has_flag(&self, s: u32, g: usize) -> bool {
        let s = s as usize;
        s < self.count && g < STATE_FLAGS && self.bitset(g)[s / 32] & 1 << (s % 32) != 0
    }

    fn bitset(&self, g: usize) -> &[u32] {
        &self.bitsets[g * self.words..(g + 1) * self.words]
    }

    /// The states of the draw identity substitution:
    /// (state, `gfxtype`, `gfxclass`)
    /// for `gfxtype` 1 or 2. The list is data tables +0x17C: the states
    /// with flag `disguise` (bit 16) in ascending state id, built by the
    /// states load through; the substitution
    /// keeps those of `gfxtype` 1 or 2.
    pub fn gfx_states(&self) -> &[(u32, u8, u16)] {
        &self.gfx
    }

    /// `srvactivefunc` of a state.
    pub fn srvactivefunc(&self, s: u32) -> Option<u16> {
        self.srvactivefunc.get(s as usize).copied()
    }
}

/// What a state toggle changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Toggle {
    /// The bit changed (and its "changed" bit was set).
    pub changed: bool,
    /// The new value of unit +0xC8 bit 8 when the state is a disguise
    /// state and the bit changed; the caller owns the unit field.
    pub disguise: Option<bool>,
}

impl StatLists {
    /// Has state: unit type 0, 1 or 3, 0 ≤ s < count and the
    /// bit is set.
    pub fn has_state(&self, unit: UnitId, s: u32) -> bool {
        let Some(r) = self.unit_list(unit) else {
            return false;
        };
        if !matches!(self.owner_type(r), owner::PLAYER | owner::MONSTER | 3) {
            return false;
        }
        if s as usize >= self.data().states.count() {
            return false;
        }
        self.state_words(r)
            .is_some_and(|w| w[s as usize / 32] & 1 << (s % 32) != 0)
    }

    /// Has any state of flag group g`(unit, g), g < 40.
    pub fn has_group(&self, unit: UnitId, g: usize) -> bool {
        let states = &self.data().states;
        if g >= STATE_FLAGS {
            return false;
        }
        let Some(w) = self.unit_list(unit).and_then(|r| self.state_words(r)) else {
            return false;
        };
        let words = states.words();
        states
            .bitset(g)
            .iter()
            .zip(&w[..words])
            .any(|(a, b)| a & b != 0)
    }

    /// Toggle`(unit, state, on). Units without an
    /// extended list have no states.
    pub fn toggle_state(&mut self, unit: UnitId, s: u32, on: bool) -> Toggle {
        let words = self.data().states.words();
        let disguise_state = self.data().states.has_flag(s, group::DISGUISE);
        let Some(r) = self.unit_list(unit) else {
            return Toggle::default();
        };
        let Some(w) = self.state_words_mut(r) else {
            return Toggle::default();
        };
        let (i, bit) = (s as usize / 32, 1u32 << (s % 32));
        if i >= words {
            return Toggle::default();
        }
        let was = w[i] & bit != 0;
        if was == on {
            return Toggle::default();
        }
        if on {
            crate::cov!(State, s, 0);
            w[i] |= bit;
        } else {
            w[i] &= !bit;
        }
        w[words + i] |= bit;
        let disguise = disguise_state.then(|| on || self.has_group(unit, group::DISGUISE));
        Toggle {
            changed: true,
            disguise,
        }
    }

    /// Sets or clears a "changed" bit only.
    pub fn set_state_changed(&mut self, unit: UnitId, s: u32, on: bool) {
        let words = self.data().states.words();
        let Some(w) = self.unit_list(unit).and_then(|r| self.state_words_mut(r)) else {
            return;
        };
        let (i, bit) = (s as usize / 32, 1u32 << (s % 32));
        if i >= words {
            return;
        }
        if on {
            w[words + i] |= bit;
        } else {
            w[words + i] &= !bit;
        }
    }

    ///`(unit, boss)'s bit walk:
    /// every set state bit outside flag group `keep` is cleared and marked
    /// in the changed half. The state lists are not touched; units without
    /// an extended list have no states.
    pub fn clear_states_except(&mut self, unit: UnitId, keep: usize) {
        let words = self.data().states.words();
        let mask: Vec<u32> = (0..words)
            .map(|i| self.data().states.bitset(keep).get(i).copied().unwrap_or(0))
            .collect();
        let Some(w) = self.unit_list(unit).and_then(|r| self.state_words_mut(r)) else {
            return;
        };
        for (i, m) in mask.iter().enumerate() {
            let gone = w[i] & !m;
            w[i] &= !gone;
            w[words + i] |= gone;
        }
    }

    /// Zeroes all the unit's state-changed bits.
    pub fn clear_states_changed(&mut self, unit: UnitId) {
        let words = self.data().states.words();
        if let Some(w) = self.unit_list(unit).and_then(|r| self.state_words_mut(r)) {
            for c in w.iter_mut().skip(words).take(words) {
                *c = 0;
            }
        }
    }

    /// The unit's state bits and changed bits (W words each).
    pub fn state_bits(&self, unit: UnitId) -> Option<(Vec<u32>, Vec<u32>)> {
        let words = self.data().states.words();
        let w = self.unit_list(unit).and_then(|r| self.state_words(r))?;
        Some((w[..words].to_vec(), w[words..].to_vec()))
    }
}

#[cfg(any(test, feature = "bench-fixtures"))]
#[cfg_attr(not(test), allow(dead_code))]
impl StateTable {
    /// A synthetic table: `count` states, `flags` (state, group) pairs.
    pub(crate) fn synthetic(count: usize, flags: &[(u32, usize)]) -> Self {
        let words = count.div_ceil(32);
        let mut bitsets = vec![0; STATE_FLAGS * words];
        for &(s, g) in flags {
            bitsets[g * words + s as usize / 32] |= 1 << (s % 32);
        }
        Self {
            count,
            words,
            bitsets,
            srvactivefunc: vec![0; count],
            gfx: Vec::new(),
        }
    }

    /// Sets a state's `srvactivefunc` (test-only).
    pub(crate) fn set_srvactivefunc(&mut self, s: u32, f: u16) {
        self.srvactivefunc[s as usize] = f;
    }

    /// Sets the draw-identity list (state, `gfxtype`, `gfxclass`)
    /// (test-only).
    pub(crate) fn set_gfx(&mut self, gfx: Vec<(u32, u8, u16)>) {
        self.gfx = gfx;
    }
}
