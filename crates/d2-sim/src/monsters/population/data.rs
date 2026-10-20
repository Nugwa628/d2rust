//! The table view population reads, built from `d2_data::tables` records
//! (`levels`, `monstats`, `monstats2`, `superuniques`). Only the columns
//! population reads are kept. Three inputs are fix-up or callback results
//! with no typed field: the levels list counts (recomputed here with the
//! fix-up rule), the monstats chain length (+0x4A) and the monstats2
//! composit counts (+0x15…+0x25); the last two are read from the `.bin` records
//! by [`chain_lengths`] and [`composits`].

use d2_data::bin::BinTable;
use d2_data::tables::{Levels, Monstats, Monstats2, Superuniques};

/// A stored i16 link (all-ones = −1).
fn i16_of(v: u16) -> i16 {
    v as i16
}

/// The entries before the first negative one.
fn counted(list: &[u16]) -> Vec<i16> {
    list.iter()
        .map(|&v| i16_of(v))
        .take_while(|&v| v >= 0)
        .collect()
}

/// One `levels` row (row index = level id).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LevelPop {
    /// `Act` (+0x03).
    pub act: u8,
    /// `WarpDist` (+0x0C).
    pub warp_dist: u32,
    /// `MonLvl1..3` (+0x10).
    pub mon_lvl: [u16; 3],
    /// `MonLvl1Ex..3Ex` (+0x16).
    pub mon_lvl_ex: [u16; 3],
    /// `MonDen`, `MonDen(N)`, `MonDen(H)` (+0x1C).
    pub mon_den: [u32; 3],
    /// `MonUMin…` (+0x28 + d).
    pub mon_umin: [u8; 3],
    /// `MonUMax…` (+0x2B + d).
    pub mon_umax: [u8; 3],
    /// `MonWndr` (+0x2E).
    pub mon_wndr: u8,
    /// `Quest` (+0x30).
    pub quest: u8,
    /// `rangedspawn` (+0x31).
    pub ranged_spawn: u8,
    /// `NumMon` (+0x32).
    pub num_mon: u8,
    /// `mon1…` (+0x36), counted (+0x33).
    pub mon: Vec<i16>,
    /// `nmon1…` (+0x68), counted (+0x34).
    pub nmon: Vec<i16>,
    /// `umon1…` (+0x9A), counted (+0x35).
    pub umon: Vec<i16>,
}

impl LevelPop {
    pub fn from_record(r: &Levels) -> Self {
        Self {
            act: r.act,
            warp_dist: r.warpdist,
            mon_lvl: [r.monlvl1, r.monlvl2, r.monlvl3],
            mon_lvl_ex: [r.monlvl1ex, r.monlvl2ex, r.monlvl3ex],
            mon_den: [r.monden, r.monden_n, r.monden_h],
            mon_umin: [r.monumin, r.monumin_n, r.monumin_h],
            mon_umax: [r.monumax, r.monumax_n, r.monumax_h],
            mon_wndr: r.monwndr,
            quest: r.quest,
            ranged_spawn: r.rangedspawn,
            num_mon: r.nummon,
            mon: counted(&[
                r.mon1, r.mon2, r.mon3, r.mon4, r.mon5, r.mon6, r.mon7, r.mon8, r.mon9, r.mon10,
                r.mon11, r.mon12, r.mon13, r.mon14, r.mon15, r.mon16, r.mon17, r.mon18, r.mon19,
                r.mon20, r.mon21, r.mon22, r.mon23, r.mon24, r.mon25,
            ]),
            nmon: counted(&[
                r.nmon1, r.nmon2, r.nmon3, r.nmon4, r.nmon5, r.nmon6, r.nmon7, r.nmon8, r.nmon9,
                r.nmon10, r.nmon11, r.nmon12, r.nmon13, r.nmon14, r.nmon15, r.nmon16, r.nmon17,
                r.nmon18, r.nmon19, r.nmon20, r.nmon21, r.nmon22, r.nmon23, r.nmon24, r.nmon25,
            ]),
            umon: counted(&[
                r.umon1, r.umon2, r.umon3, r.umon4, r.umon5, r.umon6, r.umon7, r.umon8, r.umon9,
                r.umon10, r.umon11, r.umon12, r.umon13, r.umon14, r.umon15, r.umon16, r.umon17,
                r.umon18, r.umon19, r.umon20, r.umon21, r.umon22, r.umon23, r.umon24, r.umon25,
            ]),
        }
    }
}

/// One `monstats` row (row index = class).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonPop {
    /// `BaseId` (+0x02).
    pub base_id: i16,
    /// `NextInClass` (+0x04).
    pub next_in_class: i16,
    /// `MonStatsEx` (+0x18): monstats2 row.
    pub mon_stats_ex: i16,
    /// `spawn` (+0x20).
    pub spawn: i16,
    /// `spawnx`, `spawny` (+0x22, +0x23), signed bytes.
    pub spawn_x: i8,
    pub spawn_y: i8,
    /// `spawnmode` (+0x24).
    pub spawn_mode: u8,
    /// `minion1`, `minion2` (+0x26, +0x28).
    pub minion1: i16,
    pub minion2: i16,
    /// `PartyMin`, `PartyMax` (+0x2C, +0x2D).
    pub party_min: u8,
    pub party_max: u8,
    /// `Rarity` (+0x2E).
    pub rarity: u8,
    /// `MinGrp`, `MaxGrp` (+0x2F, +0x30).
    pub min_grp: u8,
    pub max_grp: u8,
    /// `sparsePopulate` (+0x31).
    pub sparse_populate: u8,
    /// `Align` (+0x4C).
    pub align: u8,
    /// `Level` (+0xAA).
    pub level: u16,
    /// Chain length (+0x4A); 0 until set from
    /// [`chain_lengths`].
    pub chain_len: u8,
    pub is_spawn: bool,
    pub ranged_type: bool,
    pub place_spawn: bool,
    pub set_boss: bool,
    pub boss_xfer: bool,
    pub never_count: bool,
}

impl MonPop {
    pub fn from_record(r: &Monstats) -> Self {
        Self {
            base_id: i16_of(r.baseid),
            next_in_class: i16_of(r.nextinclass),
            mon_stats_ex: i16_of(r.monstatsex),
            spawn: i16_of(r.spawn),
            spawn_x: r.spawnx as i8,
            spawn_y: r.spawny as i8,
            spawn_mode: r.spawnmode,
            minion1: i16_of(r.minion1),
            minion2: i16_of(r.minion2),
            party_min: r.partymin,
            party_max: r.partymax,
            rarity: r.rarity,
            min_grp: r.mingrp,
            max_grp: r.maxgrp,
            sparse_populate: r.sparsepopulate,
            align: r.align,
            level: r.level,
            chain_len: 0,
            is_spawn: r.isspawn,
            ranged_type: r.rangedtype,
            place_spawn: r.placespawn,
            set_boss: r.setboss,
            boss_xfer: r.bossxfer,
            never_count: r.nevercount,
        }
    }
}

/// One `monstats2` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mon2Pop {
    /// `SizeX` (+0x08), sign-extended.
    pub size_x: i8,
    /// `spawnCol` (+0x0A).
    pub spawn_col: u8,
    /// Component choice counts (+0x15 + i) and their total (+0x25);
    /// zero until set from [`composits`].
    pub components: [u8; 16],
    pub composit_total: u8,
    /// `TotalPieces` (+0xEC).
    pub total_pieces: u8,
    pub critter: bool,
    pub obj_col: bool,
}

impl Mon2Pop {
    pub fn from_record(r: &Monstats2) -> Self {
        Self {
            size_x: r.sizex as i8,
            spawn_col: r.spawncol,
            components: [0; 16],
            composit_total: 0,
            total_pieces: r.totalpieces,
            critter: r.critter,
            obj_col: r.objcol,
        }
    }
}

/// One `superuniques` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SuperPop {
    /// `Class` (+0x04).
    pub class: i32,
    /// `hcIdx` (+0x08).
    pub hc_idx: u32,
    /// `MinGrp`, `MaxGrp` (+0x1C, +0x20).
    pub min_grp: u32,
    pub max_grp: u32,
    /// `AutoPos` (+0x24).
    pub auto_pos: u8,
    /// `Stacks` (+0x26).
    pub stacks: u8,
}

impl SuperPop {
    pub fn from_record(r: &Superuniques) -> Self {
        Self {
            class: r.class as i32,
            hc_idx: r.hcidx,
            min_grp: r.mingrp,
            max_grp: r.maxgrp,
            auto_pos: r.autopos,
            stacks: r.stacks,
        }
    }
}

/// The tables population reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PopTables {
    pub levels: Vec<LevelPop>,
    pub monstats: Vec<MonPop>,
    pub monstats2: Vec<Mon2Pop>,
    pub superuniques: Vec<SuperPop>,
}

impl PopTables {
    /// From typed records. Chain lengths and composit counts stay 0; set
    /// them with [`PopTables::with_bins`].
    pub fn from_records(
        levels: &[Levels],
        monstats: &[Monstats],
        monstats2: &[Monstats2],
        superuniques: &[Superuniques],
    ) -> Self {
        Self {
            levels: levels.iter().map(LevelPop::from_record).collect(),
            monstats: monstats.iter().map(MonPop::from_record).collect(),
            monstats2: monstats2.iter().map(Mon2Pop::from_record).collect(),
            superuniques: superuniques.iter().map(SuperPop::from_record).collect(),
        }
    }

    /// Fills the fix-up and callback bytes from the fixed-up `monstats` and
    /// `monstats2` `.bin` tables.
    pub fn with_bins(mut self, monstats: &BinTable, monstats2: &BinTable) -> Self {
        for (m, len) in self.monstats.iter_mut().zip(chain_lengths(monstats)) {
            m.chain_len = len;
        }
        for (m, (c, t)) in self.monstats2.iter_mut().zip(composits(monstats2)) {
            m.components = c;
            m.composit_total = t;
        }
        self
    }

    /// The monstats row of a class, `None` out of range.
    pub fn mon(&self, class: i32) -> Option<&MonPop> {
        usize::try_from(class)
            .ok()
            .and_then(|c| self.monstats.get(c))
    }

    /// The monstats2 row of a class (via `MonStatsEx`).
    pub fn mon2(&self, class: i32) -> Option<&Mon2Pop> {
        let ex = self.mon(class)?.mon_stats_ex;
        usize::try_from(ex).ok().and_then(|e| self.monstats2.get(e))
    }

    /// The levels row of a level id.
    pub fn level(&self, id: i32) -> Option<&LevelPop> {
        usize::try_from(id).ok().and_then(|i| self.levels.get(i))
    }

    /// `BaseId` of a class.
    pub fn base_id(&self, class: i32) -> Option<i32> {
        self.mon(class).map(|m| i32::from(m.base_id))
    }
}

/// The chain length byte (+0x4A) of every `monstats.bin` record.
pub fn chain_lengths(t: &BinTable) -> Vec<u8> {
    t.iter()
        .map(|r| r.get(0x4A).copied().unwrap_or(0))
        .collect()
}

/// The chain position byte (+0x4B) of a class (pass A of the monstats
/// fix-up, own implementation): the number of `NextInClass`
/// steps from the row's `BaseId` to the row itself; 0 for an invalid class
/// or a row missing from its own chain.
pub fn chain_position(monstats: &[Monstats], class: i32) -> i32 {
    let Ok(row) = usize::try_from(class) else {
        return 0;
    };
    if row >= monstats.len() {
        return 0;
    }
    let link = |v: u16| i32::from(v as i16);
    let (mut cur, mut steps, mut pos) = (link(monstats[row].baseid), 0i32, 0i32);
    while let Some(m) = usize::try_from(cur).ok().and_then(|c| monstats.get(c)) {
        if cur == class {
            pos = steps & 0xFF;
        }
        let next = link(m.nextinclass);
        steps += 1;
        if cur == next || steps > 255 || next < 0 {
            break;
        }
        cur = next;
    }
    pos
}

/// The composit counts (+0x15…+0x24) and total (+0x25) of every
/// `monstats2.bin` record.
pub fn composits(t: &BinTable) -> Vec<([u8; 16], u8)> {
    t.iter()
        .map(|r| {
            let c = r
                .get(0x15..0x25)
                .and_then(|b| b.try_into().ok())
                .unwrap_or([0; 16]);
            (c, r.get(0x25).copied().unwrap_or(0))
        })
        .collect()
}

#[cfg(test)]
mod chain_position_tests {
    use super::*;
    use d2_data::tables::Record;

    fn row(base: i16, next: i16) -> Monstats {
        let mut r = vec![0u8; Monstats::SIZE];
        r[2..4].copy_from_slice(&base.to_le_bytes());
        r[4..6].copy_from_slice(&next.to_le_bytes());
        Monstats::decode(&r)
    }

    /// test vectors: 3 rows, BaseId 0, Next 0→1, 1→2,
    /// 2→−1 give +0x4B = 0, 1, 2; an invalid class gives 0.
    #[test]
    fn chain_position_counts_steps_from_base() {
        let t = [row(0, 1), row(0, 2), row(0, -1)];
        assert_eq!([0, 1, 2].map(|c| chain_position(&t, c)), [0, 1, 2]);
        assert_eq!(chain_position(&t, 3), 0);
        assert_eq!(chain_position(&t, -1), 0);
    }

    /// 2 rows, BaseId 0, Next 0→1, 1→0: the cycle runs 255 steps more
    /// (+0x4B = 254, 255).
    #[test]
    fn chain_position_cycle_wraps() {
        let t = [row(0, 1), row(0, 0)];
        assert_eq!([0, 1].map(|c| chain_position(&t, c)), [254, 255]);
    }
}
