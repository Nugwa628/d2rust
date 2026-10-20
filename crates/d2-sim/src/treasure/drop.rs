//! Monster drops and chest drops: which TC, at which level,
//! with which walk arguments. No draws outside the walk.

use d2_data::tables::{Levels, Monstats, Superuniques};

use super::walk::{
    item_level, walk, DropSink, Dropper, DropperKind, GameFacts, Recipient, WalkArgs,
};
use super::{TreasureData, TreasureError};
use crate::rng::Seed;

/// monstats record 344 `bonewall`.
pub const CLASS_BONEWALL: u32 = 344;
/// Unit flag bit 17 (unit +0xC4): no drop.
pub const UNIT_FLAG_NO_DROP: u32 = 0x20000;

/// `treasure-chest-acts.tsv`: per act, the level ids whose
/// area levels bound the chest tiers. Checked against the TSV by
/// `chest_acts_match_tsv`.
pub const CHEST_ACTS: [(i32, i32); 5] = [(2, 37), (41, 73), (76, 102), (104, 108), (109, 136)];

/// gate: whether a dead monster drops. `flags` is
/// unit +0xC4, `collision` the collision at its position with mask
/// 0x801 (collision). A `bonewall` without the flag
/// is fatal.
pub fn monster_drop_gate(flags: u32, collision: u32, class: u32) -> Result<bool, TreasureError> {
    if flags & UNIT_FLAG_NO_DROP != 0 {
        return Ok(false);
    }
    // Spec order: flag, then the bonewall check
    // **before** the collision test (a bone wall on a blocked spot is
    // fatal too), then collision.
    if class == CLASS_BONEWALL {
        return Err(TreasureError::BonewallDrop);
    }
    if collision != 0 {
        return Ok(false);
    }
    Ok(true)
}

/// The monster's rank for the monster drop (monster fields; seam: monsters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonsterRank<'a> {
    /// hcIdx ≠ −1: its superuniques record, if found.
    Superunique(Option<&'a Superuniques>),
    /// Type flag 4.
    Champion,
    /// Type flag 8.
    Unique,
    Normal,
}

/// monstats `TreasureClass<column>` for the difficulty (+0x86 + 8 ×
/// difficulty + 2 × column; column 1–4).
pub fn tc_column(m: &Monstats, difficulty: u8, column: usize) -> Result<u16, TreasureError> {
    let cols = match difficulty {
        0 => [
            m.treasureclass1,
            m.treasureclass2,
            m.treasureclass3,
            m.treasureclass4,
        ],
        1 => [
            m.treasureclass1_n,
            m.treasureclass2_n,
            m.treasureclass3_n,
            m.treasureclass4_n,
        ],
        2 => [
            m.treasureclass1_h,
            m.treasureclass2_h,
            m.treasureclass3_h,
            m.treasureclass4_h,
        ],
        d => return Err(TreasureError::Difficulty(d)),
    };
    Ok(cols[column - 1])
}

/// Which TC with the quest replacement. `quest_open`
/// is called only when monstats `TCQuestId` ≠ 0, column 4 ≠ 0 and `R`
/// exists; given `TCQuestCP`, it answers whether the quest owner `P`
/// is a player with none of flags 15, 1 and `TCQuestCP` set for the
/// difficulty. Seam: quests (flags) and units (owner
/// resolution).
pub fn monster_tc(
    m: &Monstats,
    rank: MonsterRank,
    difficulty: u8,
    has_recipient: bool,
    quest_open: impl FnOnce(u8) -> bool,
) -> Result<u16, TreasureError> {
    let tc = match rank {
        MonsterRank::Superunique(Some(su)) => match difficulty {
            0 => su.tc,
            1 => su.tc_n,
            2 => su.tc_h,
            d => return Err(TreasureError::Difficulty(d)),
        },
        MonsterRank::Superunique(None) | MonsterRank::Unique => tc_column(m, difficulty, 3)?,
        MonsterRank::Champion => tc_column(m, difficulty, 2)?,
        MonsterRank::Normal => tc_column(m, difficulty, 1)?,
    };
    let quest = tc_column(m, difficulty, 4)?;
    if m.tcquestid != 0 && quest != 0 && has_recipient && quest_open(m.tcquestcp) {
        return Ok(quest);
    }
    Ok(tc)
}

/// level for the TC upgrade: the monster's `level` stat in an
/// expansion game above Normal unless its monstats has `noRatio` or
/// `boss`; else 0.
pub fn upgrade_level(game: &GameFacts, dropper: &Dropper, m: &Monstats) -> i32 {
    match dropper.kind {
        DropperKind::Monster { level, .. }
            if game.expansion && game.difficulty >= 1 && !m.noratio && !m.boss =>
        {
            level
        }
        _ => 0,
    }
}

/// The monster-specific inputs of the monster drop.
pub struct MonsterDrop<'a> {
    pub monstats: &'a Monstats,
    pub rank: MonsterRank<'a>,
    /// `F`: true for Find Item (the caller skips the gate).
    pub find_item: bool,
}

///–: the monster drop after the gate (or Find Item).
#[allow(clippy::too_many_arguments)]
pub fn monster_drop<S: DropSink>(
    data: &TreasureData,
    game: &GameFacts,
    md: &MonsterDrop,
    dropper: &Dropper,
    seed: &mut Seed,
    recipient: Option<&Recipient>,
    quest_open: impl FnOnce(u8) -> bool,
    sink: &mut S,
) -> Result<Vec<S::Item>, TreasureError> {
    let id = monster_tc(
        md.monstats,
        md.rank,
        game.difficulty,
        recipient.is_some(),
        quest_open,
    )?;
    let lvl = upgrade_level(game, dropper, md.monstats);
    let Some(tc) = data.tcs.get(id, lvl) else {
        return Ok(Vec::new());
    };
    let args = WalkArgs {
        tc: Some(tc),
        quality: 0,
        level: item_level(dropper),
        find_item: md.find_item,
        list: false,
        max: 6,
    };
    walk(data, game, dropper, seed, recipient, &args, sink)
}

/// Area level `a(l)`: levels `MonLvlEx` or
/// `MonLvl` for the difficulty, read as i16; 1 for a level id ≤ 0, ≥ the
/// count, or difficulty ≥ 3.
pub fn area_level(levels: &[Levels], level: i32, difficulty: u8, expansion: bool) -> i32 {
    let Some(r) = usize::try_from(level)
        .ok()
        .filter(|&l| l > 0)
        .and_then(|l| levels.get(l))
    else {
        return 1;
    };
    let v = match (expansion, difficulty) {
        (true, 0) => r.monlvl1ex,
        (true, 1) => r.monlvl2ex,
        (true, 2) => r.monlvl3ex,
        (false, 0) => r.monlvl1,
        (false, 1) => r.monlvl2,
        (false, 2) => r.monlvl3,
        _ => return 1,
    };
    i32::from(v as i16)
}

/// Chest tier for an object in `level` of `act` (0–4).
pub fn chest_tier(
    levels: &[Levels],
    act: u8,
    level: i32,
    difficulty: u8,
    expansion: bool,
) -> Result<i32, TreasureError> {
    let &(first, last) = CHEST_ACTS
        .get(usize::from(act))
        .ok_or(TreasureError::Act(act))?;
    let lo = area_level(levels, first, difficulty, expansion);
    let hi = area_level(levels, last, difficulty, expansion);
    let s = (hi.wrapping_sub(lo).wrapping_abs() + 1) / 3;
    let cur = area_level(levels, level, difficulty, expansion);
    Ok(if cur < lo + s {
        0
    } else if cur < lo + 2 * s {
        1
    } else {
        2
    })
}

/// Chest drop with forced quality `q`. `has_room`:
/// the object has a room; `act` the act of its level (0–4); `level` its
/// level id. Returns the first item created.
#[allow(clippy::too_many_arguments)]
pub fn chest_drop<S: DropSink>(
    data: &TreasureData,
    game: &GameFacts,
    levels: &[Levels],
    has_room: bool,
    act: u8,
    level: i32,
    q: u8,
    dropper: &Dropper,
    seed: &mut Seed,
    recipient: Option<&Recipient>,
    sink: &mut S,
) -> Result<Option<S::Item>, TreasureError> {
    if !has_room {
        return Ok(None);
    }
    let tier = chest_tier(levels, act, level, game.difficulty, game.expansion)?;
    let Some(tc) = data
        .tcs
        .chest_tc(i32::from(game.difficulty), i32::from(act), tier)
    else {
        return Ok(None);
    };
    let args = WalkArgs {
        tc: Some(tc),
        quality: q,
        // The tier, not a level (Edge case 1).
        level: tier,
        find_item: false,
        list: true,
        max: 6,
    };
    let items = walk(data, game, dropper, seed, recipient, &args, sink)?;
    Ok(items.first().copied())
}
