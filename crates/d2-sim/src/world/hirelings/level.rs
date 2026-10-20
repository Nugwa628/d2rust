//! Level stats, experience and level-up, the restore level
//! and the stat / experience / speech messages.

use super::{stat, threshold, HirelingError, HirelingRow, HirelingState, HirelingTables};
use super::{HirelingWorld, UNIT_PLAYER};
use crate::combat::pct;
use crate::combat::vitals::level_factor;
use crate::units::UnitId;

/// String id of the level speech.
pub const LEVEL_SPEECH: u16 = 0xD7C;
/// The defender experience cap.
pub const GAIN_CAP: i32 = 0x7F_FFFF;
/// The share of a kill the hireling did not land, /256.
pub const NON_KILLER_SHARE: i32 = 86;

/// The living hireling of `player` (`(7, 0)`) mapped to its
/// unit.
fn living<W: HirelingWorld>(w: &W, st: &HirelingState, player: UnitId) -> Option<UnitId> {
    let node = st.first_node(player, false)?;
    w.monster_by_guid(node.guid)
}

/// The row of the merc's pet node `Id` at `level`: the node by GUID,
/// then the row lookup ([`super::HirelingRows::row_at`]).
fn merc_row<'a, W: HirelingWorld>(
    w: &W,
    t: &'a HirelingTables,
    st: &HirelingState,
    player: UnitId,
    merc: UnitId,
    level: i32,
) -> Option<&'a HirelingRow> {
    let node = st.node_by_guid(player, w.guid(merc))?;
    let r = t.rows.row_at(w.expansion(), node.id, level)?;
    t.rows.rows.get(r)
}

/// S→C 0x27 (40 bytes) of the level speech (with flag 1): byte 0 0x27, byte 1 = 1, u32 merc GUID @2, byte 6 = 1,
/// byte 8 = 3, u16 string id @10.
///
/// The base game does not write the other bytes of its stack buffer; they are
/// left 0 here (in the base game they hold whatever the stack held).
pub fn speech_message(guid: u32) -> [u8; 40] {
    let mut b = [0u8; 40];
    b[0] = 0x27;
    b[1] = 1;
    b[2..6].copy_from_slice(&guid.to_le_bytes());
    b[6] = 1;
    b[8] = 3;
    b[10..12].copy_from_slice(&LEVEL_SPEECH.to_le_bytes());
    b
}

/// Apply a level to the hireling: `merc` `None` = the
/// player's living hireling; `level` 0 = merc level + 1.
pub fn apply_level<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &HirelingState,
    player: UnitId,
    merc: Option<UnitId>,
    level: i32,
) {
    // Step 1. The player-type assertion of the base game is fatal; here the call
    // does nothing.
    if w.unit_type(player) != UNIT_PLAYER {
        return;
    }
    let Some(merc) = merc.or_else(|| living(w, st, player)) else {
        return;
    };
    // Step 2.
    let level = if level == 0 {
        w.stat(merc, stat::LEVEL).wrapping_add(1)
    } else {
        level
    };
    // Step 3.
    let guid = w.guid(merc);
    w.send(player, &speech_message(guid));
    // Step 4.
    w.set_base_stat(merc, stat::LEVEL, level);
    // Step 5.
    let Some(r) = merc_row(w, t, st, player, merc, level).copied() else {
        return;
    };
    let d = level.wrapping_sub(r.level);
    // Truncating signed division (toward 0), not the `>> 3` of the offer
    // in `rows`.
    let div = |v: i32, n: i32| v.wrapping_div(n);
    let per = |lvl: i32| lvl.wrapping_mul(d);
    // Step 6.
    let next = if level < t.max_level.wrapping_sub(1) {
        threshold(r.exp_lvl, level.wrapping_add(1))
    } else {
        0
    };
    w.set_base_stat(merc, stat::NEXTEXP, next);
    // Step 7 (unsigned compare of the total value).
    let base_exp = threshold(r.exp_lvl, level);
    if (w.stat(merc, stat::EXPERIENCE) as u32) < base_exp as u32 {
        w.set_base_stat(merc, stat::EXPERIENCE, base_exp);
    }
    // Step 8, in the table's order.
    let strength = r.str_.wrapping_add(div(per(r.str_lvl), 8)).max(10);
    w.set_base_stat(merc, stat::STRENGTH, strength);
    let dexterity = r.dex.wrapping_add(div(per(r.dex_lvl), 8)).max(10);
    w.set_base_stat(merc, stat::DEXTERITY, dexterity);
    let life = per(r.hp_lvl)
        .wrapping_add(r.hp)
        .wrapping_mul(256)
        .max(0x2800);
    w.set_base_stat(merc, stat::MAXHP, life);
    w.set_base_stat(merc, stat::HITPOINTS, life);
    let ac = r.defense.wrapping_add(per(r.def_lvl)).max(0);
    w.set_base_stat(merc, stat::ARMORCLASS, ac);
    let min = r.dmg_min.wrapping_add(div(per(r.dmg_lvl), 8)).max(0);
    w.set_base_stat(merc, stat::SECONDARY_MINDAMAGE, min);
    let max = r.dmg_max.wrapping_add(div(per(r.dmg_lvl), 8)).max(1);
    w.set_base_stat(merc, stat::SECONDARY_MAXDAMAGE, max);
    let tohit = r.ar.wrapping_add(per(r.ar_lvl)).max(0);
    w.set_base_stat(merc, stat::TOHIT, tohit);
    let resist = r.resist.wrapping_add(div(per(r.resist_lvl), 4)).max(0);
    for s in [
        stat::FIRERESIST,
        stat::LIGHTRESIST,
        stat::COLDRESIST,
        stat::POISONRESIST,
    ] {
        w.set_base_stat(merc, s, resist);
    }
    let regen = div(w.base_stat(merc, stat::MAXHP), 2000).max(0);
    w.set_base_stat(merc, stat::HPREGEN, regen);
    // Step 9.
    let count = w.skill_count();
    for sk in r.skills {
        if sk.skill < 1 || sk.skill as u32 >= count || sk.mode > 15 {
            break;
        }
        let skill = sk.skill as u32;
        let Some(req) = w.skill_reqlevel(skill) else {
            continue;
        };
        if i32::from(req) > level {
            continue;
        }
        let s = (i32::from(sk.lvl_per_lvl).wrapping_mul(d) >> 5)
            .wrapping_add(i32::from(sk.level))
            .clamp(0, 32);
        if s >= 1 {
            w.set_skill_level(merc, skill, s);
        }
    }
}

/// One stat message: value < 0xFF → 0x9E (u8
/// stat @1, u32 GUID @2, u8 value @6); < 0xFFFF → 0x9F (u16 @6); else
/// 0xA0 (u32 @6). Stat id > 0xFE → [`HirelingError::StatId`] (a fatal
/// assertion in the base game).
pub fn stat_message(stat: u16, guid: u32, value: u32) -> Result<Vec<u8>, HirelingError> {
    if stat > 0xFE {
        return Err(HirelingError::StatId(stat));
    }
    let mut b = Vec::with_capacity(10);
    let op = if value < 0xFF {
        0x9E
    } else if value < 0xFFFF {
        0x9F
    } else {
        0xA0
    };
    b.push(op);
    b.push(stat as u8);
    b.extend_from_slice(&guid.to_le_bytes());
    match op {
        0x9E => b.push(value as u8),
        0x9F => b.extend_from_slice(&(value as u16).to_le_bytes()),
        _ => b.extend_from_slice(&value.to_le_bytes()),
    }
    Ok(b)
}

/// One experience message (client, merc, stat, old, new): δ = new − old
/// (unsigned); δ < 0xFF → 0xA1 (u8 δ @6); δ < 0xFFFF → 0xA2 (u16 δ @6);
/// else 0xA0 carrying the **old** value (edge case 7). u8 stat @1, u32
/// GUID @2.
pub fn exp_delta_message(stat: u8, guid: u32, old: u32, new: u32) -> Vec<u8> {
    let delta = new.wrapping_sub(old);
    let mut b = Vec::with_capacity(10);
    let op = if delta < 0xFF {
        0xA1
    } else if delta < 0xFFFF {
        0xA2
    } else {
        0xA0
    };
    b.push(op);
    b.push(stat);
    b.extend_from_slice(&guid.to_le_bytes());
    match op {
        0xA1 => b.push(delta as u8),
        0xA2 => b.extend_from_slice(&(delta as u16).to_le_bytes()),
        _ => b.extend_from_slice(&old.to_le_bytes()),
    }
    b
}

/// Send the hireling's stats (game, player, flag): the living hireling's
/// stats queued on the merc unit ([`HirelingWorld::queue_stat`])
/// in the order 12 (total), 0, 2, 7 (base), 6 (total), 31 (base), 13, 30
/// (total), then the damage sums under ids **21** (base 23 + base 21) and
/// **22** (base 24 + base 22), then 39, 41, 43, 45 (base). They are not
/// sent here: the client pass flushes the merc's queue (see
/// [`flush_stats`]) to every client that updates the merc, twice for a
/// unit new to the client. No reader of `flag` was found, so it is not
/// taken.
pub fn send_stats<W: HirelingWorld>(
    w: &mut W,
    st: &HirelingState,
    player: UnitId,
) -> Result<(), HirelingError> {
    let Some(merc) = living(w, st, player) else {
        return Ok(());
    };
    let base = |w: &W, s: u16| w.base_stat(merc, s);
    let values: [(u16, i32); 14] = [
        (stat::LEVEL, w.stat(merc, stat::LEVEL)),
        (stat::STRENGTH, base(w, stat::STRENGTH)),
        (stat::DEXTERITY, base(w, stat::DEXTERITY)),
        (stat::MAXHP, base(w, stat::MAXHP)),
        (stat::HITPOINTS, w.stat(merc, stat::HITPOINTS)),
        (stat::ARMORCLASS, base(w, stat::ARMORCLASS)),
        (stat::EXPERIENCE, w.stat(merc, stat::EXPERIENCE)),
        (stat::NEXTEXP, w.stat(merc, stat::NEXTEXP)),
        (
            stat::MINDAMAGE,
            base(w, stat::SECONDARY_MINDAMAGE).wrapping_add(base(w, stat::MINDAMAGE)),
        ),
        (
            stat::MAXDAMAGE,
            base(w, stat::SECONDARY_MAXDAMAGE).wrapping_add(base(w, stat::MAXDAMAGE)),
        ),
        (stat::FIRERESIST, base(w, stat::FIRERESIST)),
        (stat::LIGHTRESIST, base(w, stat::LIGHTRESIST)),
        (stat::COLDRESIST, base(w, stat::COLDRESIST)),
        (stat::POISONRESIST, base(w, stat::POISONRESIST)),
    ];
    for (s, v) in values {
        w.queue_stat(merc, s, v as u32);
    }
    Ok(())
}

/// The hireling-stat part of the per-unit flush (unit, client; record
/// id 0x9E): one message per queued `(stat, value)` of the
/// unit with GUID `guid`, in queue order. The client pass calls it for
/// every client whose update reaches the unit (once in the full unit
/// send of a unit new to the client, once in the monster update); the
/// queue is freed in the room update step, not here.
pub fn flush_stats(guid: u32, queued: &[(u16, u32)]) -> Result<Vec<Vec<u8>>, HirelingError> {
    queued
        .iter()
        .map(|&(s, v)| stat_message(s, guid, v))
        .collect()
}

/// Add experience (game, player, merc_level; gain, merc): add a gain
/// to the hireling (2·gain in the base game), level up while the threshold allows
/// (never past MaxLvl − 1 = 98).
pub fn add_experience<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &HirelingState,
    player: UnitId,
    merc: UnitId,
    merc_level: i32,
    gain: i32,
) -> Result<(), HirelingError> {
    // Step 1.
    if gain <= 0 || merc_level >= w.stat(player, stat::LEVEL) {
        return Ok(());
    }
    let Some(r) = merc_row(w, t, st, player, merc, merc_level).copied() else {
        return Ok(());
    };
    // Step 2.
    let old = w.base_stat(merc, stat::EXPERIENCE) as u32;
    let new = old.wrapping_add(gain.wrapping_mul(2) as u32);
    // Step 3.
    let cap = t.max_level.wrapping_sub(1);
    if merc_level >= cap {
        return Ok(());
    }
    // Step 4.
    w.set_base_stat(merc, stat::EXPERIENCE, new as i32);
    let guid = w.guid(merc);
    w.send(
        player,
        &exp_delta_message(stat::EXPERIENCE as u8, guid, old, new),
    );
    // Step 5.
    let mut lvl = merc_level;
    loop {
        let next = lvl.wrapping_add(1);
        if threshold(r.exp_lvl, next) as u32 > new {
            break;
        }
        lvl = next;
        if next >= cap {
            break;
        }
    }
    // Step 6.
    if lvl > merc_level {
        apply_level(w, t, st, player, Some(merc), lvl);
        send_stats(w, st, player)?;
        w.level_events(player, merc);
    }
    Ok(())
}

/// The experience gain (for the hireling unit `merc` owned by `player`):
/// the gain of a defender experience `exp` for attacker level `alvl` and
/// defender level `dlvl`. The `ExpRatio` step (step 3) is
/// [`super::ExpRatios::apply`] on the tables' `experience` column.
#[allow(clippy::too_many_arguments)]
pub fn gain<W: HirelingWorld>(
    w: &W,
    t: &HirelingTables,
    st: &HirelingState,
    player: UnitId,
    merc: UnitId,
    exp: i32,
    alvl: i32,
    dlvl: i32,
) -> i32 {
    // Step 1.
    if exp <= 0 {
        return 1;
    }
    let exp = exp.min(GAIN_CAP);
    // Step 2.
    if alvl >= t.max_level {
        return 0;
    }
    // Step 3.
    let mut g = level_factor(exp, alvl, dlvl);
    g = t.exp_ratios.apply(g, alvl);
    let add = w.stat(merc, stat::ADDEXPERIENCE);
    if add != 0 {
        g = g.wrapping_add(pct(g, add, 100));
    }
    // Step 4: a non-player whose owner is this player, with
    // a pet node and a row at alvl.
    let owned = w.unit_type(merc) != UNIT_PLAYER
        && w.owner(merc) == Some((w.guid(player), UNIT_PLAYER))
        && w.unit_type(player) == UNIT_PLAYER;
    if owned {
        if let Some(r) = merc_row(w, t, st, player, merc, alvl) {
            let step = (threshold(r.exp_lvl, alvl.wrapping_add(1))
                .wrapping_sub(threshold(r.exp_lvl, alvl)) as u32)
                >> 6;
            g = (g as u32).min(step) as i32;
        }
    }
    g
}

/// The kill share (game, attacker, defender), hireling part:
/// the share of a kill credited to the living hireling of the attacker's
/// player. `player_owner` is the player owner of a non-player attacker
/// (another owner); the player's own share (step 3) belongs to the
/// vitals module. Defender experience, defender level and the
/// hireling's level are base reads (step 2).
pub fn kill_share<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &HirelingState,
    attacker: UnitId,
    defender: UnitId,
    player_owner: Option<UnitId>,
) -> Result<(), HirelingError> {
    // Step 1.
    let ty = w.unit_type(attacker);
    if ty > 1 {
        return Ok(());
    }
    let exp = w.base_stat(defender, stat::EXPERIENCE);
    if exp <= 0 {
        return Ok(());
    }
    let Some(p) = (if ty == UNIT_PLAYER {
        Some(attacker)
    } else {
        player_owner
    }) else {
        return Ok(());
    };
    // Step 2.
    let Some(h) = living(w, st, p) else {
        return Ok(());
    };
    let alvl = w.base_stat(h, stat::LEVEL);
    let dlvl = w.base_stat(defender, stat::LEVEL);
    let mut g = gain(w, t, st, p, h, exp, alvl, dlvl);
    if attacker != h {
        g = g.wrapping_mul(NON_KILLER_SHARE) / 256;
    }
    add_experience(w, t, st, p, h, alvl, g)
}

/// The level of a restored hireling: from 1, while next =
/// level + 1 ≤ MaxLvl and threshold(next) ≤ experience (unsigned, `Exp/Lvl`
/// of the row at the current level): level := next. Reaches 99 (edge
/// case 6). The row (`Id`, level) is looked up once per step;
/// `None`: a step found no row (live rows never give one).
pub fn restore_level(t: &HirelingTables, id: u32, experience: u32, expansion: bool) -> Option<i32> {
    let mut level = 1i32;
    loop {
        let next = level.wrapping_add(1);
        if next > t.max_level {
            break;
        }
        let r = t.rows.row_at(expansion, id, level)?;
        if threshold(t.rows.rows[r].exp_lvl, next) as u32 > experience {
            break;
        }
        level = next;
    }
    Some(level)
}

/// The restore's experience and level for the restored `merc`:
/// experience (stat 13) := `saved` if higher (unsigned), the level from
/// it (step 6) and [`apply_level`] at that level; stat 13 is queued on
/// the unit by step 5 (its value at step 5; flushed by the client pass),
/// the level speech is sent at once.
///
/// A null row in the level loop: the ≥
/// 0x5C loader (`old` false) ends the loop, frees the unit
/// and restores no hireling (`Ok(false)`, no error; the pet node is left
/// as it is); the old loader fails fatally (string 0x1660). `Ok(true)`:
/// the hireling is restored. No node of the merc's GUID: nothing
/// (`Ok(true)`).
pub fn restore_experience<W: HirelingWorld>(
    w: &mut W,
    t: &HirelingTables,
    st: &HirelingState,
    player: UnitId,
    merc: UnitId,
    saved: u32,
    old: bool,
) -> Result<bool, HirelingError> {
    let Some(node) = st.node_by_guid(player, w.guid(merc)).copied() else {
        return Ok(true);
    };
    // Step 5.
    if saved > w.stat(merc, stat::EXPERIENCE) as u32 {
        w.set_base_stat(merc, stat::EXPERIENCE, saved as i32);
    }
    let exp = w.stat(merc, stat::EXPERIENCE) as u32;
    w.queue_stat(merc, stat::EXPERIENCE, exp);
    // Step 6.
    let Some(level) = restore_level(t, node.id, exp, w.expansion()) else {
        if old {
            return Err(HirelingError::Fatal(FATAL_LEVEL_ROW));
        }
        w.free_unit(merc);
        return Ok(false);
    };
    apply_level(w, t, st, player, Some(merc), level);
    Ok(true)
}

/// Fatal string of a null row in the old loader's level loop.
pub const FATAL_LEVEL_ROW: u16 = 0x1660;
