//! Monster regions: creation at game creation, the record,
//! the monster list, appearance variants, entries added on
//! demand and the counters.

use super::data::{Mon2Pop, PopTables};
use super::GameInfo;
use crate::rng::Seed;

/// Entries per region (region list max).
pub const MAX_ENTRIES: usize = 13;
/// Variants per entry.
pub const MAX_VARIANTS: usize = 3;
/// Ranged re-draws.
pub const RANGED_REDRAWS: u32 = 20;

/// One region entry (0x34 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionEntry {
    /// +0 class.
    pub class: i16,
    /// +2 rarity.
    pub rarity: u8,
    /// +3 variant count.
    pub variant_count: u8,
    /// +4 three 16-byte appearance variants.
    pub variants: [[u8; 16]; MAX_VARIANTS],
}

/// One monster region (0x2E4 bytes, D2MOO `D2MonsterRegionStrc`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Region {
    /// +0x000 act.
    pub act: u8,
    /// +0x004 rooms visited.
    pub rooms_visited: i32,
    /// +0x008 rooms with spawns.
    pub rooms_with_spawns: i32,
    /// +0x00C room count (−1 until first use).
    pub room_count: i32,
    /// +0x010 `nMonCount`: entries the picker may choose.
    pub mon_count: u8,
    /// +0x011 total rarity (u8, wraps).
    pub total_rarity: u8,
    /// +0x012 `nSpawnCount`: entries in use.
    pub entry_count: u8,
    /// +0x014 entries.
    pub entries: [RegionEntry; MAX_ENTRIES],
    /// +0x2B8 MonDen.
    pub mon_den: i32,
    /// +0x2BC MonUMin.
    pub mon_umin: u8,
    /// +0x2BD MonUMax.
    pub mon_umax: u8,
    /// +0x2BE MonWndr.
    pub mon_wndr: u8,
    /// +0x2C0 level id.
    pub level_id: i32,
    /// +0x2C4 wanderers spawned.
    pub wanderers: i32,
    /// +0x2C8 bosses spawned (`dwUniqueCount`).
    pub bosses: i32,
    /// +0x2CC evil monsters spawned (`dwMonSpawnCount`).
    pub evil_spawned: i32,
    /// +0x2D0 evil monsters killed (`dwMonKillCount`).
    pub evil_killed: i32,
    /// +0x2D4 (−1 at init).
    pub ai_field: i32,
    /// +0x2D8 Quest.
    pub quest: u8,
    /// +0x2DC, +0x2E0 monster level (both the same).
    pub monster_level: [i32; 2],
}

impl Region {
    /// The zeroed record filled from levels.txt.
    fn new(t: &PopTables, info: GameInfo, level_id: i32) -> Self {
        let l = t.level(level_id).cloned().unwrap_or_default();
        let d = usize::from(info.difficulty.min(2));
        let lvl = if info.expansion {
            l.mon_lvl_ex[d]
        } else {
            l.mon_lvl[d]
        };
        Self {
            act: l.act,
            room_count: -1,
            mon_den: l.mon_den[d] as i32,
            mon_umin: l.mon_umin[d],
            mon_umax: l.mon_umax[d],
            mon_wndr: l.mon_wndr,
            level_id,
            ai_field: -1,
            quest: l.quest,
            monster_level: [i32::from(lvl); 2],
            ..Self::default()
        }
    }

    /// The class at entry `i` as the room pick's walk reads it: entry `i < 13`,
    /// and one past a full list the first bytes after the entries, MonDen
    /// (edge case 3).
    pub fn entry_class(&self, i: usize) -> i32 {
        match self.entries.get(i) {
            Some(e) => i32::from(e.class),
            None => i32::from(self.mon_den as i16),
        }
    }
}

/// The region array (game +0xF0): slot = level id, slot 0 unused.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Regions {
    pub slots: Vec<Option<Region>>,
}

impl Regions {
    /// The regions of a new game (one game-seed step → monster-region
    /// seed). Returns the regions and `lo'` (game +0xEC).
    pub fn create(t: &PopTables, info: GameInfo, game_seed: &mut Seed) -> (Self, u32) {
        let lo = game_seed.step();
        let mut seed = Seed::init_low(lo);
        let mut slots = vec![None];
        for id in 1..t.levels.len() as i32 {
            let mut r = Region::new(t, info, id);
            draw_list(t, info, &mut r, &mut seed);
            for e in 0..usize::from(r.entry_count) {
                let mut entry = r.entries[e];
                let m2 = t.mon2(i32::from(entry.class));
                variants(&mut seed, &mut entry, 3, m2);
                r.entries[e] = entry;
            }
            slots.push(Some(r));
        }
        (Self { slots }, lo)
    }

    pub fn get(&self, level: i32) -> Option<&Region> {
        usize::try_from(level)
            .ok()
            .and_then(|i| self.slots.get(i))
            .and_then(Option::as_ref)
    }

    pub fn get_mut(&mut self, level: i32) -> Option<&mut Region> {
        usize::try_from(level)
            .ok()
            .and_then(|i| self.slots.get_mut(i))
            .and_then(Option::as_mut)
    }

    /// The appearance entry for a monster's class,
    /// adding one on demand. `level` is the level id of the monster's
    /// room; `unit_seed` the monster's seed. Returns the entry index.
    pub fn entry_for(
        &mut self,
        t: &PopTables,
        level: i32,
        class: i32,
        unit_seed: &mut Seed,
    ) -> Option<usize> {
        if matches!(class, 195 | 196 | 294 | 296) {
            return None;
        }
        let r = self.get_mut(level)?;
        let n = usize::from(r.entry_count);
        if let Some(i) = r.entries[..n.min(MAX_ENTRIES)]
            .iter()
            .position(|e| i32::from(e.class) == class)
        {
            return Some(i);
        }
        if n >= MAX_ENTRIES {
            return None;
        }
        let m2 = t.mon2(class);
        if t.mon(class).is_some() && m2.is_some_and(|m| m.total_pieces > 2) {
            r.entries[n].class = class as i16;
            r.entry_count += 1;
            variants(unit_seed, &mut r.entries[n], 3, m2);
        }
        Some(n)
    }

    /// The region part: with
    /// `nc` the caller sets monster flag 2 instead; returns whether it must.
    pub fn count_spawn(&mut self, level: i32, nc: bool) -> bool {
        if nc {
            return true;
        }
        if let Some(r) = self.get_mut(level) {
            r.evil_spawned = r.evil_spawned.wrapping_add(1);
        }
        false
    }

    /// An alignment change of a monster on `level`.
    pub fn alignment_changed(&mut self, level: i32, dead: bool, flag2: bool, old: u8, new: u8) {
        if dead || flag2 || old == new {
            return;
        }
        let Some(r) = self.get_mut(level) else {
            return;
        };
        if r.quest != 0 {
            return;
        }
        if old == 0 && matches!(new, 1 | 2) {
            r.evil_spawned = r.evil_spawned.wrapping_sub(1);
        } else if new == 0 && old != 4 {
            // Old = 4 is not an alignment: the setter passes it when the
            // unit has no alignment state list yet, so the first set
            // (already counted at creation) is skipped.
            r.evil_spawned = r.evil_spawned.wrapping_add(1);
        }
    }

    /// (`up`) / (`!up`).
    pub fn count_kill(&mut self, level: i32, flag2: bool, align: u8, up: bool) {
        if flag2 || align != 0 {
            return;
        }
        if let Some(r) = self.get_mut(level) {
            r.evil_killed = if up {
                r.evil_killed.wrapping_add(1)
            } else {
                r.evil_killed.wrapping_sub(1)
            };
        }
    }

    /// `unit` = (alignment, type flag 1) of the unit, if any. Returns the
    /// level id the caller sets on the unit when the regions
    /// differ.
    #[allow(clippy::too_many_arguments)]
    pub fn inactive_unit(
        &mut self,
        lvl_a: i32,
        lvl_b: i32,
        align: u8,
        dead: bool,
        keep: bool,
        unit: Option<(u8, bool)>,
    ) -> Option<i32> {
        match unit {
            Some((ualign, boss)) => {
                let differ = lvl_a != lvl_b;
                if !keep && ualign == 0 {
                    if let Some(r) = self.get_mut(lvl_b) {
                        r.evil_spawned = r.evil_spawned.wrapping_sub(1);
                        if boss {
                            r.bosses = r.bosses.wrapping_sub(1);
                        }
                    }
                }
                differ.then_some(lvl_b)
            }
            None => {
                if !keep && !dead && align == 0 {
                    if let Some(r) = self.get_mut(lvl_a) {
                        r.evil_spawned = r.evil_spawned.wrapping_sub(1);
                    }
                }
                None
            }
        }
    }

    /// The Den of Evil reading of region 8: (remaining, complete).
    pub fn den_of_evil(&self) -> Option<(i32, bool)> {
        let r = self.get(8)?;
        Some((
            r.evil_spawned.wrapping_sub(r.evil_killed),
            r.rooms_visited >= r.room_count && r.evil_killed == r.evil_spawned,
        ))
    }
}

/// The region's monster list.
fn draw_list(t: &PopTables, info: GameInfo, r: &mut Region, seed: &mut Seed) {
    let Some(l) = t.level(r.level_id) else {
        return;
    };
    let src = if info.difficulty != 0 {
        &l.nmon
    } else {
        &l.mon
    };
    let mut list: Vec<i16> = src.clone();
    let mut avail = list.len() as i32;
    let n = i32::from(l.num_mon).min(MAX_ENTRIES as i32).min(avail);
    let ranged = |c: i16| t.mon(i32::from(c)).is_some_and(|m| m.ranged_type);
    for i in 0..n {
        if avail <= 0 {
            break;
        }
        let mut idx = seed.roll(avail) as usize;
        let mut class = list[idx];
        if i == 0 && l.ranged_spawn != 0 {
            for _ in 0..RANGED_REDRAWS {
                if ranged(class) {
                    break;
                }
                idx = seed.roll(avail) as usize;
                class = list[idx];
            }
        }
        list.remove(idx);
        avail -= 1;
        if let Some(m) = t.mon(i32::from(class)).filter(|m| m.is_spawn) {
            let e = usize::from(r.entry_count);
            if e < MAX_ENTRIES {
                r.entries[e].class = class;
                r.entries[e].rarity = m.rarity;
                r.total_rarity = r.total_rarity.wrapping_add(m.rarity);
                r.entry_count += 1;
                r.mon_count += 1;
            }
        }
    }
}

/// Up to `k` new appearance variants.
pub fn variants(seed: &mut Seed, entry: &mut RegionEntry, k: i32, m2: Option<&Mon2Pop>) {
    let Some(m2) = m2 else {
        return;
    };
    let c = m2.components;
    let v = i32::from(entry.variant_count);
    // Step 1: a 32-bit shift by T & 31, compared signed.
    let room = (1i32 << (u32::from(m2.composit_total) & 31)).wrapping_sub(v);
    let mut k = k.min(room);
    if k <= 0 {
        return;
    }
    if v + k > MAX_VARIANTS as i32 {
        k = MAX_VARIANTS as i32 - v;
        if k <= 0 {
            return;
        }
    }
    let mut v = v as usize;
    if v == 0 {
        for (slot, &n) in entry.variants[0].iter_mut().zip(c.iter()) {
            *slot = if n > 1 {
                seed.roll(i32::from(n)) as u8
            } else {
                0
            };
        }
        v = 1;
        entry.variant_count = 1;
        k -= 1;
        if k == 0 {
            return;
        }
    }
    let mut l: Vec<usize> = (0..16).filter(|&i| c[i] > 1).collect();
    let m = l.len();
    if m == 0 {
        return;
    }
    let (a, b) = if m == 1 {
        (l[0], l[0])
    } else {
        let r = seed.roll(m as i32) as usize;
        let a = l[r];
        l[r] = l[m - 1];
        (a, l[seed.roll(m as i32 - 1) as usize])
    };
    for _ in 0..k {
        let mut new = entry.variants[0];
        let mut tries = 3i32;
        loop {
            new[a] = seed.roll(i32::from(c[a])) as u8;
            if b != a {
                new[b] = seed.roll(i32::from(c[b])) as u8;
            }
            let mut dup = false;
            for old in &entry.variants[..v] {
                if *old == new {
                    dup = true;
                    tries -= 1;
                }
            }
            if !dup || tries == 0 {
                break;
            }
        }
        entry.variants[v] = new;
        v += 1;
        entry.variant_count = v as u8;
    }
}
