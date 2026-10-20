// Spec: specs/world/quests-act2.md §7 (A2Q5 The Summoner, chain 12, slot 13)
//! A2Q5 callback by callback: the Summoner AI hook `0x0059C330`, chat
//! and the active function, the kill (event 8) with its two-phase timer
//! `0x0059BFD0`, the messages 419–429 (event 11) and events 3, 10, 13.
//! Slot 13 is a constant in each.

use super::{
    add_guid, add_state, add_timer, clear_bit, completion_flag, guid_listed, in_act2, party, pf,
    quick_remove, remove_guid, set_bit, status_all, table_state, Timer, TOWN,
};
use crate::units::{RoomId, UnitId};
use crate::world::quests::{bit, event, EventArgs, QuestControl, QuestWorld, TextList};

/// The record's chain.
pub const CHAIN: u8 = 12;
/// The record's flag slot.
pub const SLOT: u8 = 13;
/// The Arcane Sanctuary.
pub const ARCANE_SANCTUARY: u32 = 74;
/// The Summoner's sound event (§7.2 timer).
pub const SOUND_SUMMONER: u16 = 51;
/// FX byte of the Summoner's death (0x89).
pub const FX_SUMMONER: u8 = 7;
/// `0x0073B278`: message state by record state 0–3.
const MSG_STATE: [i8; 4] = [-1, 0, 1, 2];
/// Wants-to-talk NPCs (`0x0059BB40`): warriv2, atma, elzix, drognan,
/// lysander, cain2, meshif1, jerhyn, geglash, fara (not greiz 198, edge
/// case 7).
const TALK_NPCS: [u16; 10] = [176, 175, 199, 177, 202, 244, 210, 201, 200, 178];
/// The Summoner's last messages (any NPC).
const MSGS: std::ops::RangeInclusive<u32> = 419..=429;

/// Extra data (§7.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00 killed.
    pub killed: bool,
    /// +0x01 seen.
    pub seen: bool,
    /// +0x04 the kill room.
    pub kill_room: Option<RoomId>,
    /// +0x08 the timer exists.
    pub timer: bool,
    /// +0x09 the timer phase.
    pub phase: u8,
}

/// Dispatches chain 12's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => chat(ctl, w, i, args, list),
        event::CHANGED_LEVEL => {
            // `0x0059C200` (a = old level, b = new level).
            if ctl.records[i].not_intro && args.b >= TOWN && args.a == TOWN {
                quick_remove(ctl, w, i, args.player);
            }
        }
        event::MONSTER_KILLED => kill(ctl, w, i, args),
        // `0x0059BF70` (§1.1: event 10 removes the player from the list).
        event::PLAYER_LEAVES_GAME => remove_guid(ctl, w, i, args.player),
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => {
            // `0x0059C220`.
            let Some(p) = args.player else { return true };
            let f = pf(w, p);
            if !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, bit::COMPLETED_BEFORE)
                && f.get(SLOT, bit::STARTED)
            {
                let r = &mut ctl.records[i];
                (r.status, r.state) = (2, 1);
            }
        }
        _ => return false,
    }
    true
}

/// The Summoner AI hook `0x0059C330` (called from `0x005F85ED`, §7.2,
/// §10): the Summoner was seen.
pub fn summoner_seen<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro {
        return;
    }
    ctl.records[i].extra.a2.q5.seen = true;
    if ctl.records[i].state == 0 {
        ctl.records[i].state = 1;
    }
    if ctl.records[i].status < 2 {
        status_all(ctl, w, i, 2);
        // `0x0059BD10`.
        let state = ctl.records[i].state;
        for p in w.players() {
            let f = pf(w, p);
            if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) && state == 1
            {
                set_bit(w, p, SLOT, bit::STARTED);
            }
        }
    }
}

/// Event 0 `0x0059BBC0` (§7.2).
fn chat<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let f = super::rec(w, args.player);
    let r = &ctl.records[i];
    let done = f.get(SLOT, bit::PRIMARY_GOAL_DONE);
    let k = if f.get(SLOT, bit::REWARD_PENDING) {
        1
    } else if guid_listed(ctl, w, i, args.player) {
        2
    } else if !r.not_intro
        || r.state == 0
        || (f.get(SLOT, bit::REWARD_GRANTED) && !done)
        || (r.state > 1 && !done)
    {
        return;
    } else {
        let Some(k) = table_state(&MSG_STATE, r.state) else {
            return;
        };
        k
    };
    add_state(ctl, w, i, list, args.target, k);
}

/// Active function `0x0059BB40` (§7.2): 13.1 and a listed NPC.
pub(super) fn active<W: QuestWorld>(
    _ctl: &QuestControl,
    w: &mut W,
    _i: usize,
    player: UnitId,
    npc: u16,
) -> bool {
    pf(w, player).get(SLOT, bit::REWARD_PENDING) && TALK_NPCS.contains(&npc)
}

/// Event 8 `0x0059C150` (§7.2): the Summoner's death.
fn kill<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if ctl.records[i].not_intro {
        ctl.records[i].state = 2;
        if !ctl.records[i].extra.a2.q5.timer {
            ctl.records[i].extra.a2.q5.timer = true;
            add_timer(ctl, CHAIN, Timer::Summoner, 3);
        }
        let room = args
            .target
            .and_then(|v| w.unit_position(v))
            .map(|(_, _, r)| r);
        let x = &mut ctl.records[i].extra.a2.q5;
        x.phase = 0;
        x.killed = true;
        x.kill_room = room;
        // `0x0059BE70`: the kill room or an adjacent one.
        let near = args.target.map(|v| w.players_near(v)).unwrap_or_default();
        for p in w.players() {
            let f = pf(w, p);
            if near.contains(&p)
                && !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, bit::REWARD_PENDING)
            {
                set_bit(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
                set_bit(w, p, SLOT, bit::REWARD_PENDING);
            }
        }
        // `0x0059C0A0` → `0x0059C020` (A2Q5 bits, edge case 14).
        for p in w.players() {
            if !pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                continue;
            }
            for m in party(w, p) {
                let f = pf(w, m);
                if in_act2(w, m)
                    && !f.get(SLOT, bit::REWARD_GRANTED)
                    && !f.get(SLOT, bit::REWARD_PENDING)
                {
                    set_bit(w, m, SLOT, bit::PRIMARY_GOAL_DONE);
                    set_bit(w, m, SLOT, bit::REWARD_PENDING);
                }
            }
        }
        // `0x0059C0F0`.
        completion_flag(w, CHAIN, SLOT);
    }
    ctl.unique_event(w, FX_SUMMONER);
}

/// Timer `0x0059BFD0` (§7.2); true = remove.
pub(super) fn timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if ctl.records[i].extra.a2.q5.phase == 0 {
        ctl.records[i].extra.a2.q5.phase = 1;
        // `0x0059BF80`.
        for p in w.players() {
            if pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE)
                && w.unit_level(p) == Some(ARCANE_SANCTUARY)
            {
                w.attach_sound(p, SOUND_SUMMONER);
            }
        }
        return false;
    }
    status_all(ctl, w, i, 4);
    ctl.records[i].extra.a2.q5.timer = false;
    true
}

/// Event 11 `0x0059BD70` (§7.2): messages 419–429 from any NPC.
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !MSGS.contains(&args.b) {
        return;
    }
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_PENDING) {
        if f.get(SLOT, bit::PRIMARY_GOAL_DONE) {
            status_all(ctl, w, i, 13);
            ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
            ctl.records[i].state = 3;
        }
        add_guid(ctl, w, i, p);
        set_bit(w, p, SLOT, bit::REWARD_GRANTED);
        clear_bit(w, p, SLOT, bit::REWARD_PENDING);
    }
    if let Some(n) = args.target {
        ctl.refresh_text(w, p, n);
    }
}
