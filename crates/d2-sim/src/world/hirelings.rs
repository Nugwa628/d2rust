//! Hirelings: the `hireling.txt` rows, offer and costs ([`rows`]), the
//! hireling pet list and its messages ([`pets`]), creating, following,
//! death and revive ([`life`]), level stats, experience and the stat
//! messages ([`level`]) and the item swap ([`items`]).
//!
//! The hire list and the hire / resurrect handlers live in the NPC module
//! ([`crate::world::npc::hire`]); they reach this module through the
//! `NpcWorld` mercenary calls, which the interaction wiring maps here.
//!
//! The pet list kept here is the hireling's (pet type 7) only: the other
//! pet types live in `d2-sim::player::pets`. Everything outside this
//! module (unit records, stats, skills,
//! messages, rooms, the AI, the owner link) is reached through one seam,
//! [`HirelingWorld`].

pub mod items;
pub mod level;
pub mod life;
pub mod pets;
pub mod rows;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use d2_data::bin::BinTable;
use d2_data::tables::{Experience, Pettype, Record};

use crate::units::UnitId;

pub use rows::{resurrect_cost, threshold, HirelingRow, HirelingRows, Offer, RowSkill};

/// Pet type of a hireling (`pettype` row 7, `hireable`).
pub const PET_HIRELING: u8 = 7;
/// Unit type of a player.
pub const UNIT_PLAYER: u8 = 0;
/// `saved_id` of a new hire.
pub const NEW_HIRE: u16 = 0xFFFF;
/// Highest level gained by experience: MaxLvl − 1.
pub const XP_LEVEL_CAP: i32 = 98;
/// Hireling classes; the Act 3 shield rule's class (item swap step 2).
pub mod class {
    pub const ACT1: u32 = 271;
    pub const ACT2: u32 = 338;
    pub const ACT3: u32 = 359;
    pub const ACT5: u32 = 561;
}

/// Unit flags (+0xC4) and flags 2 (+0xC8) the hireling rules write.
pub mod flags {
    /// NOXP | NOTC | ISMERC.
    pub const INIT: u32 = 0x0402_0200;
    /// Owned (ISREVIVE in D2MOO).
    pub const OWNED: u32 = 0x8000_0000;
    /// Dead.
    pub const DEAD: u32 = 0x1_0000;
    /// NOXP set on an expired pet.
    pub const NOXP: u32 = 0x400_0000;
    /// NOXP, NOTC, ISVALIDTARGET, CANBEATTACKED, TARGETABLE.
    pub const REVIVE: u32 = 0x0402_000E;
    /// Cleared from flags 2.
    pub const REVIVE_CLEAR2: u32 = 0x4_0000;
    /// Set in flags 2 by a warp.
    pub const WARP2: u32 = 0x1_0000;
}

/// Stat ids.
pub mod stat {
    pub const STRENGTH: u16 = 0;
    pub const DEXTERITY: u16 = 2;
    pub const HITPOINTS: u16 = 6;
    pub const MAXHP: u16 = 7;
    pub const LEVEL: u16 = 12;
    pub const EXPERIENCE: u16 = 13;
    pub const GOLD: u16 = 14;
    pub const TOHIT: u16 = 19;
    pub const MINDAMAGE: u16 = 21;
    pub const MAXDAMAGE: u16 = 22;
    pub const SECONDARY_MINDAMAGE: u16 = 23;
    pub const SECONDARY_MAXDAMAGE: u16 = 24;
    pub const NEXTEXP: u16 = 30;
    pub const ARMORCLASS: u16 = 31;
    pub const FIRERESIST: u16 = 39;
    pub const LIGHTRESIST: u16 = 41;
    pub const COLDRESIST: u16 = 43;
    pub const POISONRESIST: u16 = 45;
    pub const HPREGEN: u16 = 74;
    pub const ADDEXPERIENCE: u16 = 85;
    pub const ALIGNMENT: u16 = 172;
}

/// State ids.
pub mod state {
    pub const FREEZE: u16 = 1;
    pub const ALIGNMENT: u16 = 105;
    pub const SHATTER: u16 = 107;
    pub const RECYCLED: u16 = 154;
}

/// What can go wrong outside the base-game rules (table decode, a fatal
/// assertion of the base game).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HirelingError {
    #[error("hireling table: {0}")]
    Table(String),
    /// A fatal error of the base game, by its string id
    /// (0x744 no row for an `Id` 0xFFFF restore, 0x1660 a null row in the
    /// old loader's level loop).
    #[error("hireling fatal error (string {0:#x})")]
    Fatal(u16),
    /// The pet count went negative (fatal assertion).
    #[error("hireling pet count below zero")]
    NegativeCount,
    /// A stat id above 0xFE (fatal assertion).
    #[error("stat {0} does not fit a hireling stat message")]
    StatId(u16),
}

/// The hire slot handed to the init ({name id u16, seed u32}).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Slot {
    pub name: u16,
    pub seed: u32,
}

/// One pet node of the hireling list (24 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PetNode {
    /// +0x00 bit 0.
    pub dead: bool,
    /// +0x04.
    pub guid: u32,
    /// +0x08: the slot seed.
    pub seed: u32,
    /// +0x0C.
    pub name: u16,
    /// +0x10: the `hireling` `Id`.
    pub id: u32,
}

/// A player's hireling list (pet list entry 7: head, count,
/// max). Nodes are held oldest first (head at index 0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PetList {
    pub nodes: Vec<PetNode>,
    /// +0x08. 0: not yet set (the pet max pass recomputes it).
    pub max: i32,
}

/// The `ExpRatio` column of `experience` as the ratio lookup reads it:
/// the `MaxLvl` row (row 0) gives class 0's `MaxLvl`
/// (table word 0) and its `ExpRatio` (word 7); row L + 1 holds level L
/// (word `8·L + 15`). Empty: no table (every ratio 0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExpRatios {
    /// Table word 0: class 0 `MaxLvl`.
    pub max_level: u32,
    /// Table word 7: the `MaxLvl` row's `ExpRatio`.
    pub max_row: u32,
    /// `ExpRatio` of level L at index L (0 ..= `max_level`).
    pub levels: Vec<u32>,
}

impl ExpRatios {
    /// From the `experience` table. Strict: a table without a row for
    /// every level up to its `MaxLvl` is an error (the base-game lookup reads
    /// those rows unchecked).
    pub fn from_table(experience: &BinTable) -> Result<Self, HirelingError> {
        if experience.name != Experience::TABLE || experience.record_size != Experience::SIZE {
            return Err(HirelingError::Table(format!(
                "{} ({}-byte records) is not experience",
                experience.name, experience.record_size
            )));
        }
        let rows: Vec<Experience> = experience.iter().map(Experience::decode).collect();
        let head = rows
            .first()
            .ok_or_else(|| HirelingError::Table("experience has no MaxLvl row".into()))?;
        let max_level = head.amazon;
        let levels: Vec<u32> = rows[1..].iter().map(|r| r.expratio).collect();
        if (levels.len() as u64) <= u64::from(max_level) {
            return Err(HirelingError::Table(format!(
                "experience has {} level rows, MaxLvl {max_level}",
                levels.len()
            )));
        }
        Ok(Self {
            max_level,
            max_row: head.expratio,
            levels,
        })
    }

    /// The ratio of level L: L < 1 → the `MaxLvl` row's
    /// `ExpRatio`; 1 ≤ L ≤ `MaxLvl` → level L's; above, or no table → 0.
    pub fn ratio(&self, level: i32) -> i32 {
        if self.levels.is_empty() {
            return 0;
        }
        if level < 1 {
            return self.max_row as i32;
        }
        if level as u32 > self.max_level {
            return 0;
        }
        self.levels.get(level as usize).map_or(0, |&r| r as i32)
    }

    /// The `ExpRatio` step with (e, alvl): e ≤ 0 or
    /// s − 1 ≥ 31 (unsigned) → e; else with limit = 0x7FFFFFFF >>
    /// (((r >> s) + s) & 31): e > limit → (e >> s)·r, else (r·e) >> s
    /// (32-bit signed `imul`, `sar`).
    pub fn apply(&self, e: i32, alvl: i32) -> i32 {
        if e <= 0 {
            return e;
        }
        let r = self.ratio(alvl);
        let s = self.ratio(0);
        if (s as u32).wrapping_sub(1) >= 31 {
            return e;
        }
        let sh = (r >> s).wrapping_add(s) as u32 & 31;
        let limit = 0x7FFF_FFFFi32 >> sh;
        if e > limit {
            (e >> s).wrapping_mul(r)
        } else {
            r.wrapping_mul(e) >> s
        }
    }
}

/// The data this module reads besides the rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HirelingTables {
    pub rows: HirelingRows,
    /// `experience` `ExpRatio`.
    pub exp_ratios: ExpRatios,
    /// `experience` `MaxLvl` of class 0 (99 in the base game).
    pub max_level: i32,
    /// `pettype` row 7 flags byte +4 (warp 0x1, range 0x2) and `basemax`
    /// (base game: warp 1, range 0, basemax 1).
    pub pet_flags: u8,
    pub pet_basemax: i32,
}

impl HirelingTables {
    /// `pettype` row 7 `warp` bit.
    pub const WARP: u8 = 0x1;
    /// `pettype` row 7 `range` bit.
    pub const RANGE: u8 = 0x2;

    /// The tables from the fixed-up `hireling`, `pettype` and
    /// `experience` tables; `max_level` is the `experience` `MaxLvl` of
    /// class 0 (table word 0).
    pub fn from_tables(
        hireling: &BinTable,
        pettype: &BinTable,
        experience: &BinTable,
    ) -> Result<Self, HirelingError> {
        let exp_ratios = ExpRatios::from_table(experience)?;
        let max_level = exp_ratios.max_level as i32;
        if pettype.name != Pettype::TABLE || pettype.record_size != Pettype::SIZE {
            return Err(HirelingError::Table(format!(
                "{} ({}-byte records) is not pettype",
                pettype.name, pettype.record_size
            )));
        }
        let row = pettype
            .iter()
            .nth(usize::from(PET_HIRELING))
            .map(Pettype::decode)
            .ok_or_else(|| HirelingError::Table("pettype has no row 7".into()))?;
        Ok(Self {
            rows: HirelingRows::from_table(hireling)?,
            exp_ratios,
            max_level,
            pet_flags: (u8::from(row.warp) * Self::WARP) | (u8::from(row.range) * Self::RANGE),
            pet_basemax: i32::from(row.basemax),
        })
    }
}

/// The game's hireling state: one [`PetList`] per player with lists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HirelingState {
    pub lists: BTreeMap<UnitId, PetList>,
}

impl HirelingState {
    pub fn list(&self, player: UnitId) -> Option<&PetList> {
        self.lists.get(&player)
    }
    pub fn list_mut(&mut self, player: UnitId) -> &mut PetList {
        self.lists.entry(player).or_default()
    }
    /// The node part of the first-pet lookup: the first node with `any`
    /// or bit 0 clear.
    pub fn first_node(&self, player: UnitId, any: bool) -> Option<&PetNode> {
        self.list(player)?.nodes.iter().find(|n| any || !n.dead)
    }
    /// The GUID lookup (type 7 part): the node of a GUID.
    pub fn node_by_guid(&self, player: UnitId, guid: u32) -> Option<&PetNode> {
        self.list(player)?.nodes.iter().find(|n| n.guid == guid)
    }
}

/// Everything the hireling rules reach outside this module. Unit handles
/// are [`UnitId`]; GUIDs are the unit's +0x0C.
pub trait HirelingWorld {
    // ---- game
    /// Game +0x6D: difficulty (0-based).
    fn difficulty(&self) -> u8;
    fn expansion(&self) -> bool;
    /// The players in broadcast order (players without state 7).
    fn players(&self) -> Vec<UnitId>;
    /// A message to the player's client.
    fn send(&mut self, player: UnitId, bytes: &[u8]);
    /// A stat record (`stat`, `value`) queued
    /// on the unit's message list (+0xEC) and the unit queued for update.
    /// The client pass flushes the list ([`level::flush_stats`]) to each
    /// client updating the unit; the room update frees it.
    fn queue_stat(&mut self, unit: UnitId, stat: u16, value: u32);

    // ---- units
    fn guid(&self, unit: UnitId) -> u32;
    /// The monster of a GUID.
    fn monster_by_guid(&self, guid: u32) -> Option<UnitId>;
    /// The player of a GUID.
    fn player_by_guid(&self, guid: u32) -> Option<UnitId>;
    /// Unit type (0 player, 1 monster, …).
    fn unit_type(&self, unit: UnitId) -> u8;
    /// Monstats class.
    fn class(&self, unit: UnitId) -> u32;
    fn mode(&self, unit: UnitId) -> u32;
    /// A mode change.
    fn set_mode(&mut self, unit: UnitId, mode: u8);
    /// +0xC4.
    fn flags(&self, unit: UnitId) -> u32;
    fn set_flags(&mut self, unit: UnitId, flags: u32);
    /// +0xC8.
    fn flags2(&self, unit: UnitId) -> u32;
    fn set_flags2(&mut self, unit: UnitId, flags: u32);
    /// Whether the unit is in a room.
    fn in_room(&self, unit: UnitId) -> bool;
    /// The unit's position as the range follow reads it: the
    /// static path +0x0C / +0x10 for objects, items and tiles, else the
    /// path's x / y; no path → (0, 0).
    fn position(&self, unit: UnitId) -> (i32, i32);

    // ---- stats, states, skills
    /// The total value of `stat`.
    fn stat(&self, unit: UnitId, stat: u16) -> i32;
    /// The base value of `stat`.
    fn base_stat(&self, unit: UnitId, stat: u16) -> i32;
    /// Set `stat` as a base value.
    fn set_base_stat(&mut self, unit: UnitId, stat: u16, value: i32);
    /// Maximum life.
    fn max_life(&self, unit: UnitId) -> i32;
    /// The stat part of adding a state: `stat` :=
    /// `value` in the stat list of `state`, creating the list if missing.
    fn set_state_stat(&mut self, unit: UnitId, state: u16, stat: u16, value: i32);
    /// Remove a state (and its stat list) from the unit.
    fn remove_state(&mut self, unit: UnitId, state: u16);
    /// `skills` count.
    fn skill_count(&self) -> u32;
    /// `skills` `reqlevel` (+0x174, u16 read signed) of a skill record;
    /// `None`: no record.
    fn skill_reqlevel(&self, skill: u32) -> Option<i16>;
    /// The unit's skill level of `skill` := `level`.
    fn set_skill_level(&mut self, unit: UnitId, skill: u32, level: i32);
    /// Over the player's skills (skill list
    /// order) whose `skills` `pettype` (+0xBE) is `pet_type`, the largest
    /// `petmax` value (the calc +0xC0 at the skill's level;
    /// a value below 1 counts as 1). `None`: no such skill (or no skill
    /// list).
    fn skill_pet_max(&self, player: UnitId, pet_type: u8) -> Option<i32>;

    // ---- other owners (AI, rooms, monsters, items, events)
    /// Set the owner (GUID, type); GUID −1 clears the owner.
    fn set_owner(&mut self, merc: UnitId, guid: u32, unit_type: u8);
    /// The owner (GUID, type).
    fn owner(&self, merc: UnitId) -> Option<(u32, u8)>;
    /// Join the player's team (player +0xD0).
    fn join_team(&mut self, merc: UnitId, player: UnitId);
    /// The init's step 10 (monster modifier 0x13) and step 11
    /// (monster data bytes +4, +5, +9, +10, +11 := 0).
    fn hireling_ai(&mut self, merc: UnitId);
    /// Queue the unit's removal from its room
    /// (a room removal notice with the GUID) and free it, items
    /// included.
    fn free_unit(&mut self, unit: UnitId);
    /// Queue the unit's removal from its room only.
    fn queue_room_removal(&mut self, unit: UnitId);
    /// The death mode request on a unit in a room, with the unit's target
    /// (as the expired-pet kill in `player::pets`), then the mode request
    /// itself.
    fn death_event(&mut self, unit: UnitId);
    /// Cancel the unit's pending timer
    /// events of `ty` whose first argument (+0x14) equals `a`; `a` = 0:
    /// any argument. Nothing is sent.
    fn cancel_timers(&mut self, unit: UnitId, ty: u8, a: u32);
    /// Unit +0x60 = 0 (no inventory) →
    /// a new inventory is created; an
    /// inventory already there: nothing.
    fn ensure_inventory(&mut self, unit: UnitId);
    /// Flags |= NOXP, then kill
    /// or a death mode request.
    fn dismiss(&mut self, unit: UnitId);
    /// Move the pet to the
    /// player's room and position (room lists, act, update queue, path
    /// reset). The flags 2 bit is set by the caller.
    fn warp_to(&mut self, pet: UnitId, player: UnitId);
    /// Event 0x5B on the merc for the player
    /// and unit event 12.
    fn level_events(&mut self, player: UnitId, merc: UnitId);
    /// Re-apply the stats of every active
    /// equipped item.
    fn reapply_item_stats(&mut self, merc: UnitId);
}
