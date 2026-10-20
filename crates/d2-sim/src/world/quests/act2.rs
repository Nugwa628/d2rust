// Spec: specs/world/quests-act2.md §1 (conventions), §2 (records), §1.4 (sequence chain)
// Spec: specs/world/quests-act2-2.md §1 (answers QB-2, QB-18)
//! Act II quest callbacks: Radament ([`q1`]), the Horadric Staff
//! ([`q2`]), Tainted Sun ([`q3`]), Arcane Sanctuary ([`q4`]), the
//! Summoner ([`q5`]), the Seven Tombs ([`q6`]) and the gossip and intro
//! records ([`gossip`]). This file holds the §1.1 shorthands, the §2
//! record init, the §1.4 sequence chain and the dispatch from the shared
//! machinery (`act1::callback` and friends route chains 7–13, 26, 27, 38
//! here). Functions the spec only names are reported through
//! `QuestWorld::unhandled` with their 1.14d address.

pub mod gossip;
pub mod q1;
pub mod q2;
pub mod q3;
pub mod q4;
pub mod q5;
pub mod q6;

#[cfg(test)]
mod tests_common;
#[cfg(test)]
mod tests_gossip;
#[cfg(test)]
mod tests_q1;
#[cfg(test)]
mod tests_q2;
#[cfg(test)]
mod tests_q3;
#[cfg(test)]
mod tests_q4;
#[cfg(test)]
mod tests_q5;
#[cfg(test)]
mod tests_q6;

use super::{
    bit, flags_of, send_player_flags, EventArgs, GuidList, QuestControl, QuestFlags, QuestRecord,
    QuestWorld, TextList,
};
use crate::units::UnitId;

/// Lut Gholein.
pub const TOWN: u32 = 40;
/// Act II (0-based act id of its records and levels).
pub const ACT: u8 = 1;
/// Sound event of a refused operate or use.
pub const SOUND_REFUSED: u16 = 19;

/// Per-quest extra data of the Act II records (record +0x18). One
/// struct for every record; each chain uses its own part.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// A2Q0 (chain 7): the extra GUID list (§9).
    pub q0: GuidList,
    /// A2Q7 (chain 26) extra +0x00: the chat-end byte `0x0059E0B0` tests
    /// (init writes 0; no 1.14d code writes 1, `quests-act2-2.md` §1
    /// item 3).
    pub q7_chat: u8,
    pub q1: q1::Extra,
    pub q2: q2::Extra,
    pub q3: q3::Extra,
    pub q4: q4::Extra,
    pub q5: q5::Extra,
    pub q6: q6::Extra,
}

/// Act II quest timers (§3.7, §5.3, §5.7, §7.2, §8.7, §8.8, §8.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// `0x00598F70`: Radament status 3 (§3.7).
    RadamentStatus,
    /// `0x0059ED80`: the Tainted Sun darken delay (§5.3).
    Darken,
    /// `0x0059A700`: the altar's status 3 (§5.7).
    AltarStatus,
    /// `0x0059BFD0`: the Summoner's two-phase timer (§7.2).
    Summoner,
    /// `0x0059CEE0`: Duriel's status 3 (§8.11).
    DurielStatus,
    /// `0x0059D870`: the lair objects (§8.8).
    LairObjects,
}

/// §2: what the Act II init functions store beyond `quests.tsv` (state,
/// status, active; the extra data starts zeroed).
pub fn init(r: &mut QuestRecord) {
    match r.chain {
        8 => {
            r.active = true;
            r.state = 1;
        }
        9 => {
            r.active = true;
            r.state = 0;
            r.status = 13;
        }
        10..=13 => {
            r.active = true;
            r.state = 0;
        }
        _ => {}
    }
}

// ------------------------------------------------------------ §1.1

/// The player's current-difficulty record (zero without one).
pub(crate) fn pf<W: QuestWorld>(w: &mut W, p: UnitId) -> QuestFlags {
    flags_of(w, p).copied().unwrap_or_default()
}

/// The event player's record (zero when there is no player).
pub(crate) fn rec<W: QuestWorld>(w: &mut W, p: Option<UnitId>) -> QuestFlags {
    p.map_or_else(QuestFlags::default, |p| pf(w, p))
}

/// Set bit `b` of `slot` in the player's record.
pub(crate) fn set_bit<W: QuestWorld>(w: &mut W, p: UnitId, slot: u8, b: u8) {
    if let Some(f) = flags_of(w, p) {
        f.set(slot, b);
    }
}

/// Clear bit `b` of `slot` in the player's record.
pub(crate) fn clear_bit<W: QuestWorld>(w: &mut W, p: UnitId, slot: u8, b: u8) {
    if let Some(f) = flags_of(w, p) {
        f.clear(slot, b);
    }
}

/// "send flags" (`quests.md` §6.6): S→C 0x28.
pub(crate) fn send_flags<W: QuestWorld>(w: &mut W, p: UnitId) {
    send_player_flags(w, p, 6, 0);
}

/// "in Act II": the unit's room's level has act 1.
pub(crate) fn in_act2<W: QuestWorld>(w: &W, u: UnitId) -> bool {
    w.unit_act(u) == Some(ACT)
}

/// The event player's GUID (`u32::MAX` without one).
pub(crate) fn guid_of<W: QuestWorld>(w: &W, p: Option<UnitId>) -> u32 {
    p.map_or(u32::MAX, |p| w.guid(p))
}

/// "GUID listed" (`0x005452C0`).
pub(crate) fn guid_listed<W: QuestWorld>(
    ctl: &QuestControl,
    w: &W,
    i: usize,
    p: Option<UnitId>,
) -> bool {
    ctl.records[i].guids.contains(guid_of(w, p))
}

/// "add GUID" (`0x00545200`).
pub(crate) fn add_guid<W: QuestWorld>(ctl: &mut QuestControl, w: &W, i: usize, p: UnitId) {
    let g = w.guid(p);
    ctl.records[i].guids.add(g);
}

/// Event 10 (`0x00545530`, `quests-act2-2.md` §1 item 2): when the
/// leaving player's record has both s.1 and s.0 (s = the record's filter
/// slot) and the record list is not empty, remove the player's GUID (−1
/// without a player) from it. Otherwise nothing.
pub(crate) fn remove_guid<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    p: Option<UnitId>,
) {
    let slot = ctl.records[i].filter;
    let f = rec(w, p);
    if f.get(slot, bit::REWARD_PENDING)
        && f.get(slot, bit::REWARD_GRANTED)
        && !ctl.records[i].guids.0.is_empty()
    {
        let g = guid_of(w, p);
        ctl.records[i].guids.remove(g);
    }
}

/// "quick remove" (`0x00545310`): remove the player when the list is not
/// empty.
pub(crate) fn quick_remove<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &W,
    i: usize,
    p: Option<UnitId>,
) {
    if !ctl.records[i].guids.0.is_empty() {
        let g = guid_of(w, p);
        ctl.records[i].guids.remove(g);
    }
}

/// "add state k" (§1.2, `quests.md` §7.1) for the NPC `target`.
pub(crate) fn add_state<W: QuestWorld>(
    ctl: &QuestControl,
    w: &W,
    i: usize,
    list: Option<&mut TextList>,
    target: Option<UnitId>,
    k: u8,
) {
    let class = target.and_then(|n| w.monster_class(n));
    if let (Some(t), Some(list), Some(c)) = (ctl.records[i].msgs, list, class) {
        ctl.add_messages(t, list, c, k);
    }
}

/// An index table's entry for `state` (§Constants): `None` for −1 or a
/// state past the table.
pub(crate) fn table_state(table: &[i8], state: u8) -> Option<u8> {
    match table.get(usize::from(state)) {
        Some(&m) if m >= 0 => Some(m as u8),
        _ => None,
    }
}

/// F (§2 column F, §1.1): 0x5D for the record's chain to `p` when (s.0
/// clear and s.15 clear) or s.13 or s.14.
pub(crate) fn status_iterate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    p: UnitId,
) {
    let (chain, slot) = (ctl.records[i].chain, ctl.records[i].filter);
    let f = pf(w, p);
    if (!f.get(slot, bit::REWARD_GRANTED) && !f.get(slot, bit::COMPLETED_BEFORE))
        || f.get(slot, bit::PRIMARY_GOAL_DONE)
        || f.get(slot, bit::COMPLETED_NOW)
    {
        if let Err(e) = ctl.send_status(w, p, chain) {
            ctl.faults.push(e);
        }
    }
}

/// "status n to all": flags := 0, status := n, F for every player.
pub(crate) fn status_all<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, n: u8) {
    ctl.records[i].flags = 0;
    ctl.records[i].status = n;
    for p in w.players() {
        status_iterate(ctl, w, i, p);
    }
}

/// "status n (silent)": flags := 0, status := n, nothing sent.
pub(crate) fn status_silent(ctl: &mut QuestControl, i: usize, n: u8) {
    ctl.records[i].flags = 0;
    ctl.records[i].status = n;
}

/// "completion flag" (§1.1): every player without s.0 and s.1 gets s.14
/// and `5D <chain> 00 0C 0000` (`0x00545920`, act argument 0).
pub(crate) fn completion_flag<W: QuestWorld>(w: &mut W, chain: u8, slot: u8) {
    for p in w.players() {
        let f = pf(w, p);
        if f.get(slot, bit::REWARD_GRANTED) || f.get(slot, bit::REWARD_PENDING) {
            continue;
        }
        set_bit(w, p, slot, bit::COMPLETED_NOW);
        // `0x00545920(player, chain, 0)` (`quests.md` §6.3).
        let send = match w.unit_level(p) {
            None => true,
            Some(0) => false,
            Some(_) => true,
        };
        if send {
            w.send(p, &[0x5D, chain, 0, 12, 0, 0]);
        }
    }
}

/// The party members of `p` (none without a party).
pub(crate) fn party<W: QuestWorld>(w: &W, p: UnitId) -> Vec<UnitId> {
    w.party_members(p).unwrap_or_default()
}

/// §1.3's last step: one quest-seed step, n = (lo' mod 5) + 5 gold piles.
pub(crate) fn chest_gold<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let n = ctl.seed.step() % 5 + 5;
    for _ in 0..n {
        w.drop_gold(object);
    }
}

/// Add a quest timer, recording a fault on failure.
pub(crate) fn add_timer(ctl: &mut QuestControl, chain: u8, t: Timer, period: u32) {
    if let Err(e) = ctl.add_timer(chain, super::TimerFn::Act2(t), period) {
        ctl.faults.push(e);
    }
}

/// "Call seq(c)" (§1.4): the record's own sequence function, `None`
/// when chain `c` has no record.
pub(crate) fn call_seq<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, c: u8) -> Option<bool> {
    ctl.find(c)?;
    Some(super::act1::sequence(ctl, w, c))
}

// ------------------------------------------------------------ dispatch

/// The sequence functions of chains 8, 10, 11, 13 (§1.4).
pub fn sequence<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, chain: u8) -> bool {
    let Some(i) = ctl.find(chain) else {
        return false;
    };
    let (state, not_intro) = (ctl.records[i].state, ctl.records[i].not_intro);
    match chain {
        // `0x005991C0`.
        8 => {
            if state != 5 && not_intro {
                return true;
            }
            call_seq(ctl, w, 13).unwrap_or(false)
        }
        // `0x0059A480`.
        10 => {
            if state != 5 && not_intro {
                return true;
            }
            call_seq(ctl, w, 11).unwrap_or(false)
        }
        // `0x0059B500`.
        11 => {
            if state == 0 && not_intro {
                ctl.records[i].state = 1;
                call_seq(ctl, w, 13);
            }
            true
        }
        // `0x0059D450`.
        13 => {
            if state == 0 && not_intro {
                ctl.records[i].state = 1;
            }
            call_seq(ctl, w, 10).unwrap_or(true)
        }
        _ => false,
    }
}

/// Dispatches one callback of an Act II record; false = no body
/// (reported as unhandled by the caller).
pub fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match ctl.records[i].chain {
        8 => q1::callback(ctl, w, i, args, list),
        9 => q2::callback(ctl, w, i, args, list),
        10 => q3::callback(ctl, w, i, args, list),
        11 => q4::callback(ctl, w, i, args, list),
        12 => q5::callback(ctl, w, i, args, list),
        13 => q6::callback(ctl, w, i, args, list),
        7 | 26 | 27 | 38 => gossip::callback(ctl, w, i, args, list),
        _ => false,
    }
}

/// Active functions (wants to talk, `quests.md` §6.4).
pub fn active_fn<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    match ctl.records[i].chain {
        8 => q1::active(ctl, w, i, player, npc_class),
        9 => q2::active(ctl, w, i, player, npc_class),
        10 => q3::active(ctl, w, i, player, npc_class),
        11 => q4::active(ctl, w, i, player, npc_class),
        12 => q5::active(ctl, w, i, player, npc_class),
        13 => q6::active(ctl, w, i, player, npc_class),
        7 | 26 | 27 | 38 => gossip::active(ctl, w, i, player, npc_class),
        c => {
            w.unhandled(c, f);
            false
        }
    }
}

/// Status functions (§2): chain 9 (§4.6), chain 13 (§8.5); chains 7
/// (`0x00598770`), 26 (`0x0059E2B0`) and 27 (`0x0059E4A0`) return false
/// and write nothing (`quests-act2-2.md` §1 item 18). The others are
/// reported.
pub fn status_fn<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    match ctl.records[i].chain {
        7 | 26 | 27 => None,
        9 => Some(q2::status(ctl, w, i, player, pf)),
        13 => Some(q6::status(ctl, w, i, player, pf)),
        c => {
            w.unhandled(c, f);
            None
        }
    }
}

/// Runs an Act II timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    let Some(i) = ctl.find(chain) else {
        return true;
    };
    match t {
        Timer::RadamentStatus => q1::timer(ctl, w, i),
        Timer::Darken => q3::darken_timer(ctl, w, i),
        Timer::AltarStatus => q3::altar_timer(ctl, w, i),
        Timer::Summoner => q5::timer(ctl, w, i),
        Timer::DurielStatus => q6::duriel_timer(ctl, w, i),
        Timer::LairObjects => q6::lair_timer(ctl, w, i),
    }
}
