// Spec: specs/world/quests-act4.md §1.1, specs/world/quests-act5.md §1.1 (the notation of specs/world/quests-act3.md §1.1)
//! The shorthands the Act IV and Act V specs share: "s.b", "status n to
//! all", "status n (silent)", "completion flag", "S5D", "quick remove",
//! "in Act N", "party of P", "add GUID", "holds `code`", "FX b", "sound
//! n", "refresh", "send flags". Each helper names the 1.14d function the
//! notation stands for.

use super::{bit, flags_of, send_player_flags, QuestControl, QuestFlags, QuestWorld, TextList};
use crate::units::UnitId;

/// The player's record of the game difficulty (zero when it has none).
pub fn flags<W: QuestWorld>(w: &mut W, p: UnitId) -> QuestFlags {
    flags_of(w, p).copied().unwrap_or_default()
}

/// Set `s.b` on the player (no-op without a record).
pub fn set<W: QuestWorld>(w: &mut W, p: UnitId, slot: u8, b: u8) {
    if let Some(f) = flags_of(w, p) {
        f.set(slot, b);
    }
}

/// Clear `s.b` on the player (no-op without a record).
pub fn clear<W: QuestWorld>(w: &mut W, p: UnitId, slot: u8, b: u8) {
    if let Some(f) = flags_of(w, p) {
        f.clear(slot, b);
    }
}

/// `QuestFlags::reset_progress` on the player.
pub fn reset_progress<W: QuestWorld>(w: &mut W, p: UnitId, slot: u8) {
    if let Some(f) = flags_of(w, p) {
        f.reset_progress(slot);
    }
}

/// "in Act N" (`0x006427F0`): the player's room's level has act `act`
/// (0-based: Act IV = 3, Act V = 4).
pub fn in_act<W: QuestWorld>(w: &W, p: UnitId, act: u8) -> bool {
    w.unit_act(p) == Some(act)
}

/// The Act II iterate rule every Act IV and Act V F uses (act4 §2, act5
/// §2): 0x5D for the record's chain to `p` when (s.0 clear and s.15
/// clear) or s.13 or s.14.
pub fn iterate<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    let (chain, slot) = (ctl.records[i].chain, ctl.records[i].filter);
    let f = flags(w, p);
    if (!f.get(slot, bit::REWARD_GRANTED) && !f.get(slot, bit::COMPLETED_BEFORE))
        || f.get(slot, bit::PRIMARY_GOAL_DONE)
        || f.get(slot, bit::COMPLETED_NOW)
    {
        if let Err(e) = ctl.send_status(w, p, chain) {
            ctl.faults.push(e);
        }
    }
}

/// The flag iterate for every player (`0x005537D0` with F, every F
/// returns 0).
pub fn iterate_all<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    for p in w.players() {
        iterate(ctl, w, i, p);
    }
}

/// "status n to all": flags := 0, then `0x00544300(record, n, 0, F, 1)`.
pub fn status_to_all<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, n: u8) {
    ctl.records[i].flags = 0;
    ctl.records[i].status = n;
    iterate_all(ctl, w, i);
}

/// "status n (silent)": `0x00544300` with iterate 0 — status := n,
/// nothing sent (`quests-act1.md` §10.1 status(S)).
pub fn status_silent(ctl: &mut QuestControl, i: usize, n: u8) {
    ctl.records[i].status = n;
}

/// "state := n" (`0x00544350`; only a debug log besides the write).
pub fn set_state(ctl: &mut QuestControl, i: usize, n: u8) {
    ctl.records[i].state = n;
}

/// `0x00545920(player, chain, act)` (`quests.md` §6.3): `5D chain 00 0C
/// 0000` when the player has no room, or its room's level is ≠ 0 and in
/// an act ≥ `act`.
pub fn send_completed_now<W: QuestWorld>(w: &mut W, p: UnitId, chain: u8, act: u8) {
    let send = match w.unit_level(p) {
        None => true,
        Some(0) => false,
        Some(_) => w.unit_act(p).is_none_or(|a| a >= act),
    };
    if send {
        w.send(p, &[0x5D, chain, 0, 12, 0, 0]);
    }
}

/// "completion flag": for each player (in `players()` order) lacking
/// every bit of `bits` in `slot`: set `slot.14` and
/// `0x00545920(player, chain, 0)`.
pub fn completion_flag<W: QuestWorld>(w: &mut W, chain: u8, slot: u8, bits: &[u8]) {
    for p in w.players() {
        let f = flags(w, p);
        if bits.iter().any(|&b| f.get(slot, b)) {
            continue;
        }
        set(w, p, slot, bit::COMPLETED_NOW);
        send_completed_now(w, p, chain, 0);
    }
}

/// "S5D(c, f, v)": `5D c f 00 v` (u16 v) to that player only; the
/// record's status byte is not touched.
pub fn s5d<W: QuestWorld>(w: &mut W, p: UnitId, chain: u8, flags: u8, v: u16) {
    let v = v.to_le_bytes();
    w.send(p, &[0x5D, chain, flags, 0, v[0], v[1]]);
}

/// "quick remove" (`0x00545310`): if the record's GUID list is not
/// empty, remove the player's GUID.
pub fn quick_remove<W: QuestWorld>(ctl: &mut QuestControl, w: &W, i: usize, p: UnitId) {
    if !ctl.records[i].guids.0.is_empty() {
        let g = w.guid(p);
        ctl.records[i].guids.remove(g);
    }
}

/// "add GUID" (`0x00545200`) to the record's list.
pub fn add_guid<W: QuestWorld>(ctl: &mut QuestControl, w: &W, i: usize, p: UnitId) {
    let g = w.guid(p);
    ctl.records[i].guids.add(g);
}

/// "GUID listed" (`0x005452C0`).
pub fn guid_listed<W: QuestWorld>(ctl: &QuestControl, w: &W, i: usize, p: UnitId) -> bool {
    ctl.records[i].guids.contains(w.guid(p))
}

/// Event 10's removal from the record list (`0x00545530`).
pub fn leave<W: QuestWorld>(ctl: &mut QuestControl, w: &W, i: usize, p: UnitId) {
    let g = w.guid(p);
    ctl.records[i].guids.remove(g);
}

/// "party of P" (`0x00540510` when P's party id ≠ −1): the members, or
/// empty for no party.
pub fn party_of<W: QuestWorld>(w: &W, p: UnitId) -> Vec<UnitId> {
    w.party_members(p).unwrap_or_default()
}

/// "add state k" (`0x00543790`) for the NPC `npc` of class `class`.
pub fn add_state(ctl: &QuestControl, i: usize, list: Option<&mut TextList>, class: u16, k: u8) {
    if let (Some(t), Some(list)) = (ctl.records[i].msgs, list) {
        ctl.add_messages(t, list, class, k);
    }
}

/// The table state of a record state through an image index table
/// (`None`: −1 or past the table).
pub fn table_state(table: &[i8], state: u8) -> Option<u8> {
    match table.get(usize::from(state)) {
        Some(&m) if m >= 0 => Some(m as u8),
        _ => None,
    }
}

/// "send flags" (`quests.md` §6.6): 0x28 to the player.
pub fn send_flags<W: QuestWorld>(w: &mut W, p: UnitId) {
    send_player_flags(w, p, 6, 0);
}

/// "refresh" (`quests.md` §7.2) of `npc` for `p`.
pub fn refresh<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, p: UnitId, npc: UnitId) {
    ctl.refresh_text(w, p, npc);
}

/// `0x0061AED0(room, flag)` on the unit's room: flag 0 is
/// `refresh_room`; flag 1 finds the room (nothing when it has none).
pub fn unit_room_portal<W: QuestWorld>(w: &mut W, unit: UnitId, on: bool) {
    if !on {
        w.refresh_room(unit);
    } else if let Some((_, _, room)) = w.unit_position(unit) {
        w.set_room_portal(room, true);
    }
}
