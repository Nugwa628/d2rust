// Spec: specs/world/quests-act1.md §10.4 (A1Q1 Den of Evil, chain 1)
// Spec: specs/world/quests.md (the sections other than §10)
//! A1Q1 callback by callback: events 0, 2, 3, 8, 10, 11, 13, the status
//! timer, the active function and the iterate functions I2–I5
//! (I1 is the shared status iterate). Slot 1 is a constant in each.

use super::{
    add_state, broadcast, party_goal, player_flags, rec, restore, send_completed_now, sequence,
    table_state,
};
use crate::units::UnitId;
use crate::world::quests::{
    bit, event, flags_of, grant_pending, npc, send_player_flags, EventArgs, QuestControl,
    QuestWorld, TextList, TimerFn,
};

const SLOT: u8 = 1;
/// The Den of Evil level.
const DEN: u32 = 8;
/// The Rogue Encampment.
const TOWN: u32 = 1;
/// Message state by quest state 0–5.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];

/// Dispatches chain 1's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => npc_text(ctl, w, i, args, list),
        event::NPC_DEACTIVATE => chat_end(ctl, w, i, args),
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => kill(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            let g = args.player.map_or(u32::MAX, |p| w.guid(p));
            ctl.records[i].guids.remove(g);
            ctl.records[i].extra.guids.remove(g);
        }
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => {
            if let Some(p) = args.player {
                restore(ctl, w, i, p);
            }
        }
        _ => return false,
    }
    true
}

/// I2 for every player.
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

/// Event 0.
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
    let rd = &ctl.records[i];
    if r.get(SLOT, bit::REWARD_GRANTED)
        || (rd.state >= 4 && !r.get(SLOT, bit::PRIMARY_GOAL_DONE))
        || !rd.not_intro
    {
        return;
    }
    if let Some(m) = table_state(&MSG_STATE, rd.state).filter(|&m| m < 8) {
        add_state(ctl, w, i, list, args.target, m);
    }
}

/// Event 2.
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let akara = args.target.and_then(|n| w.monster_class(n)) == Some(npc::AKARA);
    if akara && ctl.records[i].extra.talked {
        broadcast(ctl, w, i, 1, 0);
        ctl.records[i].extra.talked = false;
        ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    }
}

/// Event 3 (a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.b == DEN {
        if !ctl.records[i].not_intro {
            return;
        }
        let r = &mut ctl.records[i];
        let changed = r.state == 1 || r.state == 2;
        if changed {
            r.state = 3;
        }
        r.extra.entered = true;
        if r.status < 2 {
            broadcast(ctl, w, i, 2, 0);
            ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
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
            // I2 runs before the status changes (bug kept, §10.4).
            iterate_progress(ctl, w, i);
            if ctl.records[i].status != 1 {
                broadcast(ctl, w, i, 1, 0);
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            }
        }
    }
}

/// Event 8: a monster with a chain-1 link dies.
fn kill<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !ctl.records[i].not_intro {
        return;
    }
    // The region tracks all population rooms, including rooms not yet streamed.
    let (spawned, killed, visited, populated) = w.den_region();
    let left = (spawned as i32).wrapping_sub(killed as i32);
    ctl.records[i].extra.monsters_left = left;
    if let Some(k) = args.player {
        let g = w.guid(k);
        ctl.records[i].extra.guids.add(g);
    }
    if populated != 0 && populated <= visited && spawned != 0 && killed == spawned {
        let r = &mut ctl.records[i];
        r.extra.done = true;
        r.clear_callback(event::NPC_DEACTIVATE);
        r.state = 4;
        r.clear_callback(event::MONSTER_KILLED);
        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        let killers = ctl.records[i].extra.guids.clone();
        grant_pending(w, &killers, SLOT, 0);
        for p in w.players() {
            party_goal(w, p, SLOT);
        }
        for p in w.players() {
            completed_now(w, p);
        }
        for p in w.players() {
            if player_flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                w.attach_sound(p, 35);
            }
        }
        ctl.unique_event(w, 0);
        if !ctl.records[i].extra.timer {
            ctl.records[i].extra.timer = true;
            if let Err(e) = ctl.add_timer(1, TimerFn::DenOfEvilStatus, 8) {
                ctl.faults.push(e);
            }
        }
    } else if populated <= visited && left <= 5 {
        broadcast(ctl, w, i, 4, 0x20);
        ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    } else if ctl.records[i].status == 4 && left > 5 {
        broadcast(ctl, w, i, 4, 0x20);
    }
}

/// I4.
fn completed_now<W: QuestWorld>(w: &mut W, p: UnitId) {
    let Some(f) = flags_of(w, p) else { return };
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
        return;
    }
    f.set(SLOT, bit::COMPLETED_NOW);
    send_completed_now(w, p, 1, 0);
    send_player_flags(w, p, 6, 0);
}

/// Timer: status 5 while state 4; runs once.
pub(super) fn timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    if ctl.records[i].state == 4 {
        broadcast(ctl, w, i, 5, 0);
    }
    ctl.records[i].extra.timer = false;
}

/// Event 11 (a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a != u32::from(npc::AKARA) {
        return;
    }
    match args.b {
        64 => {
            ctl.records[i].extra.talked = true;
            ctl.records[i].state = 2;
            iterate_progress(ctl, w, i);
            if let Some(n) = args.target {
                ctl.refresh_text(w, p, n);
            }
        }
        76 if player_flags(w, p).get(SLOT, bit::REWARD_PENDING) => {
            if player_flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                if ctl.records[i].state != 5 {
                    ctl.records[i].state = 5;
                    sequence(ctl, w, 1);
                    ctl.records[i].flags = 0;
                    ctl.records[i].status = 13;
                }
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            }
            let flag2 = ctl.records[i].flag2;
            if let Some(f) = flags_of(w, p) {
                f.set(SLOT, bit::REWARD_GRANTED);
                f.clear(SLOT, bit::REWARD_PENDING);
                if let Some(s2) = flag2 {
                    f.set(s2, bit::PRIMARY_GOAL_DONE);
                    f.set(s2, bit::REWARD_PENDING);
                }
                f.reset_progress(SLOT);
            }
            w.add_stat(p, 5, 1);
            let g = w.guid(p);
            ctl.records[i].guids.add(g);
            if let Some(n) = args.target {
                ctl.refresh_text(w, p, n);
            }
        }
        _ => {}
    }
}

/// Active (§6.4).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let r = player_flags(w, player);
    let rd = &ctl.records[i];
    npc_class == npc::AKARA
        && !r.get(SLOT, bit::REWARD_GRANTED)
        && (r.get(SLOT, bit::REWARD_PENDING) || (rd.not_intro && rd.state == 1))
}
