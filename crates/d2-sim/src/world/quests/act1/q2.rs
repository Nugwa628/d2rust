// Spec: specs/world/quests-act1.md §10.5 (A1Q2 Sisters' Burial Grounds, chain 2)
// Spec: specs/world/quests.md (the sections other than §10)
//! A1Q2 callback by callback: events 0, 2, 3, 8, 10, 11, 13, the timer
//! `0x00590BF0` and the active function, with the iterate functions
//! J2–J7 (J1 is the shared status iterate). Slot 2 is a constant in each.

use super::{
    add_state, broadcast, member_goal, player_flags, rec, restore, send_completed_now, sequence,
    status_all, table_state,
};
use crate::units::UnitId;
use crate::world::quests::{
    bit, event, flags_of, npc, send_player_flags, EventArgs, QuestControl, QuestError, QuestWorld,
    TextList, TimerFn,
};

const SLOT: u8 = 2;
/// The Burial Grounds (1.14d constant).
const BURIAL_GROUNDS: u32 = 17;
/// The Rogue Encampment.
const TOWN: u32 = 1;
/// `0x00737180`: message state by quest state 0–3.
const MSG_STATE: [i8; 4] = [-1, 0, 1, 2];
/// J3's function, for its fatal asserts.
const J3: u32 = 0x0059_0C40;

/// Dispatches chain 2's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => npc_text(ctl, w, i, args, list),
        event::NPC_DEACTIVATE => {
            // `0x00590920`.
            let kashya = args.target.and_then(|n| w.monster_class(n)) == Some(npc::KASHYA);
            if kashya && ctl.records[i].extra.talked {
                broadcast(ctl, w, i, 1, 0);
                ctl.records[i].extra.talked = false;
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
                iterate_progress(ctl, w, i);
            }
        }
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => kill(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            // `0x00590C10`.
            let g = args.player.map_or(u32::MAX, |p| w.guid(p));
            ctl.records[i].guids.remove(g);
        }
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => {
            // `0x00591180`.
            if let Some(p) = args.player {
                restore(ctl, w, i, p);
            }
        }
        _ => return false,
    }
    true
}

/// J2 `0x00590890` for every player (as A1Q1's I2).
fn iterate_progress<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let (state, status) = (ctl.records[i].state, ctl.records[i].status);
    for p in w.players() {
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match state {
            2 => f.set(SLOT, bit::STARTED),
            3 if status == 1 => f.set(SLOT, bit::LEAVE_TOWN),
            3 => f.set(SLOT, bit::ENTER_AREA),
            _ => {}
        }
    }
}

/// Event 0 `0x00590B10` (no not-intro test).
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let r = rec(w, args.player);
    if r.get(SLOT, bit::REWARD_PENDING) {
        return add_state(ctl, w, i, list, args.target, 3);
    }
    let g = args.player.map_or(u32::MAX, |p| w.guid(p));
    if ctl.records[i].guids.contains(g) {
        return add_state(ctl, w, i, list, args.target, 4);
    }
    let state = ctl.records[i].state;
    if state != 0 && !r.get(SLOT, bit::REWARD_GRANTED) && state < 4 {
        if let Some(m) = table_state(&MSG_STATE, state) {
            add_state(ctl, w, i, list, args.target, m);
        }
    }
}

/// Event 3 `0x00590FA0` (a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.b == BURIAL_GROUNDS && ctl.records[i].not_intro {
        let r = &mut ctl.records[i];
        let changed = r.state < 3;
        if changed {
            r.state = 3;
            r.flags = 0;
        }
        if r.status <= 1 {
            status_all(ctl, w, i, 2);
            iterate_progress(ctl, w, i);
        } else if changed {
            iterate_progress(ctl, w, i);
        }
    } else if args.a == TOWN {
        let g = args.player.map_or(u32::MAX, |p| w.guid(p));
        if !ctl.records[i].guids.0.is_empty() {
            ctl.records[i].guids.remove(g);
        }
        let r = rec(w, args.player);
        if ctl.records[i].state == 2
            && !r.get(SLOT, bit::REWARD_GRANTED)
            && !r.get(SLOT, bit::REWARD_PENDING)
        {
            ctl.records[i].state = 3;
            iterate_progress(ctl, w, i);
        }
    }
}

/// Event 8 `0x00590EC0`: any monster with a chain-2 link dies (the
/// killer is not read). Callback 8 stays: a second linked death repeats
/// all of it.
fn kill<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !ctl.records[i].not_intro {
        return;
    }
    let victim = args.target.map_or(u32::MAX, |v| w.guid(v));
    let r = &mut ctl.records[i];
    r.state = 4;
    r.extra.kill_b1 = true;
    r.extra.kill_b2 = true;
    r.extra.victim = victim;
    for p in w.players() {
        reward_pending_near(ctl, w, i, p);
    }
    for p in w.players() {
        // J7 `0x00590E70`: J4 for each party member (as I3).
        if player_flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            if let Some(members) = w.party_members(p) {
                for m in members {
                    member_goal(w, m, SLOT);
                }
            }
        }
    }
    for p in w.players() {
        // J5 `0x00590DD0` (no 0x28).
        let Some(f) = flags_of(w, p) else { continue };
        if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) {
            f.set(SLOT, bit::COMPLETED_NOW);
            send_completed_now(w, p, 2, 0);
        }
    }
    for p in w.players() {
        // J6 `0x00590E30`.
        if player_flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            w.attach_sound(p, 34);
        }
    }
    if let Err(e) = ctl.add_timer(2, TimerFn::BurialStatus, 15) {
        ctl.faults.push(e);
    }
    let r = &mut ctl.records[i];
    r.clear_callback(event::NPC_DEACTIVATE);
    r.extra.killed = true;
    r.extra.kill_d4 = 1;
    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
}

/// J3 `0x00590C40`: a player near the victim with neither 2.0 nor 2.1
/// gets 2.13, then 2.1.
fn reward_pending_near<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    let x = &ctl.records[i].extra;
    if !x.kill_b2 {
        return;
    }
    let Some((v, _)) = w.monster_by_guid(x.victim) else {
        ctl.faults.push(QuestError::Fatal(J3));
        return;
    };
    // `players_near` holds the room test (P has a room; P's room is V's,
    // or V's room is in P's room list).
    if !w.players_near(v).contains(&p) {
        return;
    }
    let Some(f) = flags_of(w, p) else { return };
    if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) {
        f.set(SLOT, bit::PRIMARY_GOAL_DONE);
        f.set(SLOT, bit::REWARD_PENDING);
    }
}

/// Event 11 `0x00590980` (a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a != u32::from(npc::KASHYA) {
        return;
    }
    match args.b {
        81 => {
            ctl.records[i].extra.talked = true;
            ctl.records[i].state = 2;
            iterate_progress(ctl, w, i);
            if let Some(n) = args.target {
                ctl.refresh_text(w, p, n);
            }
        }
        92 if player_flags(w, p).get(SLOT, bit::REWARD_PENDING) => {
            if player_flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) && ctl.records[i].state != 5 {
                let r = &mut ctl.records[i];
                r.flags = 0;
                r.status = 13;
                r.state = 5;
                sequence(ctl, w, 2);
            }
            if let Some(f) = flags_of(w, p) {
                f.set(SLOT, bit::REWARD_GRANTED);
                f.clear(SLOT, bit::REWARD_PENDING);
            }
            send_player_flags(w, p, 6, 0);
            let g = w.guid(p);
            ctl.records[i].guids.add(g);
            w.mercenary_reward(p, npc::KASHYA);
            if let Some(n) = args.target {
                ctl.refresh_text(w, p, n);
            }
        }
        _ => {}
    }
}

/// Active `0x00591080` (§6.4): as A1Q1's with Kashya and slot 2.
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let r = player_flags(w, player);
    let rd = &ctl.records[i];
    npc_class == npc::KASHYA
        && !r.get(SLOT, bit::REWARD_GRANTED)
        && (r.get(SLOT, bit::REWARD_PENDING) || (rd.not_intro && rd.state == 1))
}
