// Spec: specs/world/quests-act2.md §3 (A2Q1 Radament's Lair, chain 8, slot 9)
// Spec: specs/world/quests-act2-2.md §1 items 1, 2, 19
//! A2Q1 callback by callback: events 0, 2, 3, 8, 10, 11, 13, the status
//! timer `0x00598F70`, the active function `0x00598910`, Radament's AI
//! hook `0x00599420` ([`radament_ai`]) and the Book of Skill use
//! `0x0055E170` ([`use_book_of_skill`]). Slot 9 is a constant in each.

use super::{
    add_guid, add_state, add_timer, call_seq, clear_bit, completion_flag, guid_listed, in_act2,
    party, pf, quick_remove, rec, remove_guid, send_flags, set_bit, status_all, status_silent,
    table_state, Timer, SOUND_REFUSED, TOWN,
};
use crate::units::{RoomId, UnitId};
use crate::world::quests::{bit, event, EventArgs, QuestControl, QuestWorld, TextList};

/// Chain id.
pub const CHAIN: u8 = 8;
/// Flag slot.
pub const SLOT: u8 = 9;
/// atma (monstats hcIdx).
pub const ATMA: u16 = 176;
/// Sewers Level 3.
pub const SEWERS_3: u32 = 49;
/// Book of Skill item code.
pub const BOOK: [u8; 4] = *b"ass ";
/// Atma's start message.
pub const MSG_START: u32 = 304;
/// Atma's reward message.
pub const MSG_REWARD: u32 = 334;
/// Sound attached to the players with 9.13 at the kill.
pub const SOUND_RADAMENT: u16 = 50;
/// Status timer period (updater ticks).
pub const TIMER_PERIOD: u32 = 12;
/// Stat 5: new skill points.
const STAT_NEW_SKILLS: u16 = 5;
/// `0x007398FC`: table state by record state 0–6.
const MSG_STATE: [i8; 7] = [-1, 0, 1, 2, 3, 4, 0];

/// Extra data (§3.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00 Radament killed.
    pub killed: bool,
    /// +0x04 the room of the kill.
    pub kill_room: Option<RoomId>,
    /// +0x08 Atma started the quest (chat end pending).
    pub atma_started: bool,
    /// +0x09 "first entry status sent".
    pub entry_sent: bool,
    /// +0x0A the status timer exists.
    pub timer: bool,
    /// +0x0C Book of Skill drop count.
    pub books: i32,
    /// +0x10 reward pending from an earlier game.
    pub reward_pending: bool,
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a2.q1
}

/// Dispatches chain 8's callbacks; false = no body (unhandled).
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
        // `0x00598980`: a jump to `0x00545530` (`quests-act2-2.md` §1
        // item 2).
        event::PLAYER_LEAVES_GAME => remove_guid(ctl, w, i, args.player),
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => start(ctl, w, i, args),
        _ => return false,
    }
    true
}

/// `0x005989E0` for every player: players without 9.0 and 9.1 get 9.2
/// at state 2, and at state 3 9.3 when the status is 1, else 9.4.
fn iterate_progress<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let (state, status) = (ctl.records[i].state, ctl.records[i].status);
    for p in w.players() {
        let f = pf(w, p);
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match state {
            2 => set_bit(w, p, SLOT, bit::STARTED),
            3 if status == 1 => set_bit(w, p, SLOT, bit::LEAVE_TOWN),
            3 => set_bit(w, p, SLOT, bit::ENTER_AREA),
            _ => {}
        }
    }
}

/// Event 0 `0x00598C40` (§3.2).
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
    if guid_listed(ctl, w, i, args.player) {
        return add_state(ctl, w, i, list, args.target, 4);
    }
    let state = ctl.records[i].state;
    if !r.get(SLOT, bit::REWARD_GRANTED)
        && state != 0
        && (state < 4 || r.get(SLOT, bit::PRIMARY_GOAL_DONE))
    {
        // TODO(quests-act2 §3.2): the image table has 7 entries; states
        // 7 and 8 would read past it (unreachable: chain 8 stops at 5).
        if let Some(m) = table_state(&MSG_STATE, state).filter(|&m| m < 9) {
            add_state(ctl, w, i, list, args.target, m);
        }
    }
}

/// Active `0x00598910` (§3.2): Atma wants to talk.
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc: u16,
) -> bool {
    let r = pf(w, player);
    let rd = &ctl.records[i];
    npc == ATMA
        && !r.get(SLOT, bit::REWARD_GRANTED)
        && ((rd.not_intro && rd.state == 1 && !r.get(SLOT, bit::COMPLETED_BEFORE))
            || r.get(SLOT, bit::REWARD_PENDING))
}

/// Event 11 `0x00598A70` (§3.3; a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a != u32::from(ATMA) {
        return;
    }
    match args.b {
        MSG_START => {
            x(ctl, i).atma_started = true;
            ctl.records[i].state = 2;
            iterate_progress(ctl, w, i);
        }
        MSG_REWARD if pf(w, p).get(SLOT, bit::REWARD_PENDING) => {
            if pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                if ctl.records[i].state != 5 {
                    status_all(ctl, w, i, 13);
                    ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
                    ctl.records[i].state = 5;
                    super::sequence(ctl, w, CHAIN);
                }
                if !ctl.records[i].not_intro {
                    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                }
            } else if x(ctl, i).reward_pending {
                call_seq(ctl, w, 13);
            }
            set_bit(w, p, SLOT, bit::REWARD_GRANTED);
            clear_bit(w, p, SLOT, bit::REWARD_PENDING);
            add_guid(ctl, w, i, p);
        }
        _ => return,
    }
    if let Some(n) = args.target {
        ctl.refresh_text(w, p, n);
    }
}

/// Event 2 `0x00598990` (§3.4).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let atma = args.target.and_then(|n| w.monster_class(n)) == Some(ATMA);
    if atma && x(ctl, i).atma_started {
        status_all(ctl, w, i, 1);
        x(ctl, i).atma_started = false;
        ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    }
}

/// Event 3 `0x00599130` (§3.5; a = old level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.a != TOWN {
        return;
    }
    quick_remove(ctl, w, i, args.player);
    let r = rec(w, args.player);
    if ctl.records[i].state == 2
        && !r.get(SLOT, bit::REWARD_GRANTED)
        && !r.get(SLOT, bit::REWARD_PENDING)
    {
        ctl.records[i].state = 3;
        iterate_progress(ctl, w, i);
    }
}

/// `0x00599420` (§3.6): called by Radament's AI (`0x005F2C47`) with
/// Radament's unit.
pub fn radament_ai<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, unit: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let r = &ctl.records[i];
    if !r.not_intro || !(r.state < 3 || r.status < 2) || w.unit_level(unit) != Some(SEWERS_3) {
        return;
    }
    ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    let changed = ctl.records[i].state < 3;
    if changed {
        ctl.records[i].state = 3;
    }
    if ctl.records[i].status < 2 {
        if x(ctl, i).entry_sent {
            status_silent(ctl, i, 2);
        } else {
            x(ctl, i).entry_sent = true;
            status_all(ctl, w, i, 2);
        }
        iterate_progress(ctl, w, i);
    } else if changed {
        // Status ≥ 2: the flag iterate only when the state was just
        // raised, no 0x5D (`quests-act2-2.md` §1 item 19).
        iterate_progress(ctl, w, i);
    }
}

/// Event 8 `0x00599020` (§3.7): Radament dies.
fn kill<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !ctl.records[i].not_intro {
        return;
    }
    ctl.records[i].state = 4;
    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
    ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    let room = args.target.and_then(|v| w.unit_position(v)).map(|p| p.2);
    let e = x(ctl, i);
    e.killed = true;
    e.kill_room = room;
    // 1. `0x00598E20`, with `0x00598D40` for the party.
    let near = args.target.map(|v| w.players_near(v)).unwrap_or_default();
    for p in w.players() {
        if !near.contains(&p) || !goal(w, p) {
            continue;
        }
        for m in party(w, p) {
            if in_act2(w, m) {
                goal(w, m);
            }
        }
    }
    // 2. `0x00598DC0`.
    completion_flag(w, CHAIN, SLOT);
    // 3. `0x00598FA0`.
    x(ctl, i).books = 0;
    for p in w.players() {
        let f = pf(w, p);
        if f.get(SLOT, bit::CUSTOM1) && !w.has_item(p, BOOK) {
            x(ctl, i).books += 1;
        }
        if f.get(SLOT, bit::PRIMARY_GOAL_DONE) {
            w.attach_sound(p, SOUND_RADAMENT);
        }
    }
    if !x(ctl, i).timer {
        x(ctl, i).timer = true;
        add_timer(ctl, CHAIN, Timer::RadamentStatus, TIMER_PERIOD);
    }
    if let Some(v) = args.target {
        // The victim's drop code := `ass `, then `0x00559A30(game,
        // victim, 2, &0, 0, −1, 0)` per book.
        for _ in 0..x(ctl, i).books.max(0) {
            w.drop_item_at(v, BOOK, 2);
        }
    }
}

/// A player without 9.0 and 9.1 gets 9.13, 9.1, 9.5 and 0x28; true when
/// it did.
fn goal<W: QuestWorld>(w: &mut W, p: UnitId) -> bool {
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
        return false;
    }
    set_bit(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
    set_bit(w, p, SLOT, bit::REWARD_PENDING);
    set_bit(w, p, SLOT, bit::CUSTOM1);
    send_flags(w, p);
    true
}

/// Timer `0x00598F70` (§3.7); true = remove.
pub(super) fn timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if ctl.records[i].state == 4 {
        status_all(ctl, w, i, 3);
    }
    x(ctl, i).timer = false;
    true
}

/// Event 13 `0x00599230` (§3.9): from the first entering player.
fn start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) {
        // Edge case 1.
        if f.get(SLOT, bit::CUSTOM1) && !w.has_item(p, BOOK) {
            for b in [
                bit::REWARD_GRANTED,
                bit::REWARD_PENDING,
                bit::COMPLETED_BEFORE,
                bit::CUSTOM1,
            ] {
                clear_bit(w, p, SLOT, b);
            }
        }
        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        let r = &mut ctl.records[i];
        r.not_intro = false;
        r.state = 0;
    } else if f.get(SLOT, bit::COMPLETED_BEFORE) {
        if f.get(SLOT, bit::REWARD_PENDING) {
            x(ctl, i).reward_pending = true;
        }
        let r = &mut ctl.records[i];
        r.state = 0;
        r.not_intro = false;
    } else {
        let r = &mut ctl.records[i];
        if f.get(SLOT, bit::ENTER_AREA) {
            (r.state, r.status) = (3, 2);
        } else if f.get(SLOT, bit::LEAVE_TOWN) {
            (r.state, r.status) = (3, 1);
        } else if f.get(SLOT, bit::STARTED) {
            (r.state, r.status) = (2, 1);
        }
    }
}

/// Book of Skill use (§3.8, `0x0055E170`; the item-use spec owns the
/// usability test `0x0055CC90` and removing the item). With 9.5: clear
/// it, +1 new skill point (stat 5, `0x006272B0`), `5D 08 02 00 0000`
/// (`0x005458E0`); returns true (the book is consumed). Without 9.5:
/// sound 19; returns false (the book stays).
pub fn use_book_of_skill<W: QuestWorld>(w: &mut W, player: UnitId) -> bool {
    if !pf(w, player).get(SLOT, bit::CUSTOM1) {
        w.attach_sound(player, SOUND_REFUSED);
        return false;
    }
    clear_bit(w, player, SLOT, bit::CUSTOM1);
    w.add_stat(player, STAT_NEW_SKILLS, 1);
    w.send(player, &[0x5D, CHAIN, 2, 0, 0, 0]);
    true
}
