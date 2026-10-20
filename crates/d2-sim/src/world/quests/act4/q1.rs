// Spec: specs/world/quests-act4.md §3 (A4Q1 The Fallen Angel), §7, §8
//! A4Q1 The Fallen Angel (chain 22, slot 25) callback by callback:
//! events 0, 2, 3, 8, 10, 11, 13, the active function, the flag iterate
//! `0x005B3860`, the ghost timer `0x005B3E60` and the hooks the monster
//! creation switch and the NpcStationary / Izual AIs call (§3.6, §8).

use super::super::late::{
    add_guid, add_state, clear, completion_flag, flags, guid_listed, in_act, leave, party_of,
    quick_remove, refresh, reset_progress, s5d, set, set_state, status_silent, status_to_all,
    table_state,
};
use super::super::{
    bit, event, npc, EventArgs, QuestControl, QuestFlags, QuestWorld, TextList, TimerFn,
};
use crate::units::{RoomId, UnitId};

const CHAIN: u8 = 22;
const SLOT: u8 = 25;
/// `izualghost`.
pub const IZUAL_GHOST: u16 = 406;
/// `izual` (monster base id).
pub const IZUAL_BASE: u16 = 256;
/// `uberizual`: never linked.
pub const UBER_IZUAL: u16 = 706;
/// The Pandemonium Fortress.
const TOWN: u32 = 103;
/// `0x0073DBA0`: table state by record state 0–5.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];
/// Stat 5 `newskills`.
const NEWSKILLS: u16 = 5;
/// Sound of the completion (§3.6).
const SOUND_DONE: u16 = 74;
/// Ghost timer period (updater ticks).
const GHOST_PERIOD: u32 = 3;
/// Monster mode of the spawned ghost.
const GHOST_MODE: u8 = 8;
/// The spawn's `r` arguments tried in order (−1, 3, 5).
const GHOST_TRIES: [u32; 3] = [u32::MAX, 3, 5];
/// Distance under which a player keeps the ghost (§3.6).
const GHOST_RANGE: i32 = 5;

/// Quest extra data (record +0x18, 0x1C bytes).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x04 / +0x08: Izual's death position.
    pub x: i32,
    pub y: i32,
    /// +0x0C: "entered the area" (restored by game start, §3.7).
    pub entered: bool,
    /// +0x0D: the ghost timer exists.
    pub timer: bool,
    /// +0x0F: the ghost was talked to (written, never read).
    pub ghost_talked_0f: bool,
    /// +0x10: the ghost is to spawn.
    pub ghost_pending: bool,
    /// +0x11: Tyrael started the quest (chat end pending).
    pub started: bool,
    /// +0x12: the ghost was talked to.
    pub ghost_talked: bool,
    /// +0x14: Izual's unit (at the kill), then the ghost (§3.6).
    pub unit: Option<UnitId>,
    /// +0x18: a player is near the ghost (scratch).
    pub near: bool,
}

/// Timers this quest makes (`quests.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// `0x005B3E60`: Izual's ghost (§3.6).
    Ghost,
}

/// Init beyond `quests.tsv` (§2: state 1, set by `act4::init`).
pub fn init(r: &mut super::super::QuestRecord) {
    r.extra.a4.q1 = Extra::default();
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a4.q1
}

/// One callback of record `i`; false = not handled (reported).
pub fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => chat(ctl, w, i, args, list),
        event::NPC_DEACTIVATE => chat_end(ctl, w, i, args),
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => izual_killed(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            // `0x005B3E30`.
            match args.player {
                Some(p) => leave(ctl, w, i, p),
                None => ctl.records[i].guids.remove(u32::MAX),
            }
        }
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => started(ctl, w, i, args),
        _ => return false,
    }
    true
}

/// Flag iterate `0x005B3860` for every player (§3.2).
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let r = &ctl.records[i];
    let (state, entered) = (r.state, r.extra.a4.q1.entered);
    for p in w.players() {
        if flags(w, p).get(SLOT, bit::REWARD_GRANTED) {
            continue;
        }
        match state {
            2 => set(w, p, SLOT, bit::STARTED),
            3 if entered => set(w, p, SLOT, bit::ENTER_AREA),
            3 => set(w, p, SLOT, bit::LEAVE_TOWN),
            _ => {}
        }
    }
}

/// Event 0 `0x005B3BB0` (§3.3).
fn chat<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    // NPC chat always has a player.
    let Some(p) = args.player else { return };
    let class = args.target.and_then(|n| w.monster_class(n));
    let npc = class.unwrap_or(u16::MAX);
    let f = flags(w, p);
    if class == Some(IZUAL_GHOST) && !f.get(SLOT, bit::CUSTOM1) {
        return add_state(ctl, i, list, npc, 3);
    }
    if f.get(SLOT, bit::REWARD_PENDING) {
        return add_state(ctl, i, list, npc, 3);
    }
    if guid_listed(ctl, w, i, p) {
        return add_state(ctl, i, list, npc, 4);
    }
    let r = &ctl.records[i];
    if !f.get(SLOT, bit::REWARD_GRANTED)
        && (r.state < 4 || f.get(SLOT, bit::PRIMARY_GOAL_DONE))
        && r.not_intro
    {
        if let Some(m) = table_state(&MSG_STATE, r.state) {
            add_state(ctl, i, list, npc, m);
        }
    }
}

/// Active `0x005B37F0` (§3.3).
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = f;
    let r = &ctl.records[i];
    let pf = flags(w, player);
    r.not_intro
        && !pf.get(SLOT, bit::REWARD_GRANTED)
        && ((npc_class == npc::TYRAEL2 && (pf.get(SLOT, bit::REWARD_PENDING) || r.state == 1))
            || (npc_class == IZUAL_GHOST && !pf.get(SLOT, bit::CUSTOM1)))
}

/// Event 11 `0x005B39A0` (§3.4; a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let refresh_npc = |ctl: &mut QuestControl, w: &mut W| {
        if let Some(n) = args.target {
            refresh(ctl, w, p, n);
        }
    };
    match (args.a, args.b) {
        (a, 670) if a == u32::from(npc::TYRAEL2) => {
            x(ctl, i).started = true;
            set_state(ctl, i, 2);
            flag_iterate(ctl, w, i);
            refresh_npc(ctl, w);
        }
        (a, 676) if a == u32::from(npc::TYRAEL2) => {
            if !flags(w, p).get(SLOT, bit::REWARD_PENDING) {
                return;
            }
            if flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                if ctl.records[i].state != 5 {
                    set_state(ctl, i, 5);
                    super::sequence(ctl, w, CHAIN);
                    ctl.records[i].flags = 0;
                    status_silent(ctl, i, 13);
                }
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
            }
            set(w, p, SLOT, bit::REWARD_GRANTED);
            clear(w, p, SLOT, bit::REWARD_PENDING);
            reset_progress(w, p, SLOT);
            w.add_stat(p, NEWSKILLS, 2);
            s5d(w, p, CHAIN, 2, 0);
            add_guid(ctl, w, i, p);
            refresh_npc(ctl, w);
        }
        (a, 675) if a == u32::from(IZUAL_GHOST) => {
            let e = x(ctl, i);
            e.ghost_talked = true;
            e.ghost_talked_0f = true;
            set(w, p, SLOT, bit::CUSTOM1);
            if ctl.records[i].status != 4 {
                status_to_all(ctl, w, i, 4);
            }
        }
        _ => {}
    }
}

/// Event 2 `0x005B41D0` (§3.4).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.target.and_then(|n| w.monster_class(n)) != Some(npc::TYRAEL2) || !x(ctl, i).started {
        return;
    }
    status_to_all(ctl, w, i, 1);
    x(ctl, i).started = false;
    ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
}

/// "lacking 25.0 and 25.1".
fn uncredited(f: &QuestFlags) -> bool {
    !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING)
}

/// 25.13 and 25.1, then reset_progress(25) when `reset`.
fn credit<W: QuestWorld>(w: &mut W, p: UnitId, reset: bool) {
    set(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
    set(w, p, SLOT, bit::REWARD_PENDING);
    if reset {
        reset_progress(w, p, SLOT);
    }
}

/// Event 8 `0x005B4020`: Izual's death (§3.5).
fn izual_killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let r = &mut ctl.records[i];
    r.clear_callback(event::NPC_DEACTIVATE);
    r.state = 4;
    r.clear_callback(event::MONSTER_KILLED);
    r.extra.a4.q1.unit = args.target;
    if r.not_intro {
        if let Some(k) = args.player {
            if uncredited(&flags(w, k)) {
                credit(w, k, true);
            }
        }
        x(ctl, i).ghost_pending = true;
        // A killed monster stands in a room; without one the position
        // is left as it was.
        if let Some((px, py, _)) = args.target.and_then(|v| w.unit_position(v)) {
            let e = x(ctl, i);
            (e.x, e.y) = (px, py);
        }
        if let Some(v) = args.target {
            // `0x005B3F20`: same or adjacent room as the victim's.
            for p in w.players_near(v) {
                if uncredited(&flags(w, p)) {
                    credit(w, p, false);
                }
            }
        }
        // `0x005B3DE0`: the party of each player with 25.13
        // (`0x005B3D70` per member).
        for p in w.players() {
            if !flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                continue;
            }
            for m in party_of(w, p) {
                if in_act(w, m, 3) && uncredited(&flags(w, m)) {
                    credit(w, m, true);
                }
            }
        }
        // `0x005B3CD0`.
        completion_flag(w, CHAIN, SLOT, &[bit::REWARD_GRANTED, bit::REWARD_PENDING]);
    }
    if !x(ctl, i).timer {
        x(ctl, i).timer = true;
        let t = TimerFn::Act4(super::Timer::Q1(Timer::Ghost));
        if let Err(e) = ctl.add_timer(CHAIN, t, GHOST_PERIOD) {
            ctl.faults.push(e);
        }
    }
}

/// Event 3 `0x005B4140` (a = old level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.a != TOWN {
        return;
    }
    let Some(p) = args.player else { return };
    quick_remove(ctl, w, i, p);
    if ctl.records[i].state == 2 && !flags(w, p).get(SLOT, bit::REWARD_GRANTED) {
        set_state(ctl, i, 3);
        flag_iterate(ctl, w, i);
        if ctl.records[i].status == 0 {
            ctl.records[i].flags = 0;
            status_silent(ctl, i, 1);
        }
    }
}

/// Event 13 `0x005B4220` (§3.7).
fn started<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = flags(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::COMPLETED_BEFORE) {
        return;
    }
    let r = &mut ctl.records[i];
    if f.get(SLOT, bit::ENTER_AREA) {
        r.extra.a4.q1.entered = true;
        (r.status, r.state) = (2, 3);
    } else if f.get(SLOT, bit::LEAVE_TOWN) {
        (r.state, r.status) = (3, 1);
    } else if f.get(SLOT, bit::STARTED) {
        (r.state, r.status) = (2, 1);
    }
}

/// The status function: chain 22 has none (§2); never reached.
pub fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    let _ = (player, pf);
    w.unhandled(ctl.records[i].chain, f);
    None
}

/// Runs a timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    match t {
        Timer::Ghost => {
            if let Some(i) = ctl.find(chain) {
                ghost_timer(ctl, w, i);
            }
            true
        }
    }
}

/// Timer `0x005B3E60` (§3.6): status 3, then Izual's ghost at the death
/// position. Returns 1 (one firing).
fn ghost_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    let r = &ctl.records[i];
    if r.state == 4 && !matches!(r.status, 4 | 13) {
        status_to_all(ctl, w, i, 3);
    }
    if x(ctl, i).ghost_pending {
        x(ctl, i).ghost_pending = false;
        let (gx, gy) = (x(ctl, i).x, x(ctl, i).y);
        if let Some(room) = w.room_in_act_at(3, gx, gy) {
            spawn_ghost(w, room, gx, gy);
        }
    }
    x(ctl, i).timer = false;
}

/// `0x005B2F20(game, room, x, y, 406, 8, r, 0)` with r = −1, 3, 5 until
/// one succeeds.
fn spawn_ghost<W: QuestWorld>(w: &mut W, room: RoomId, gx: i32, gy: i32) {
    for r in GHOST_TRIES {
        if w.spawn_monster(room, gx, gy, IZUAL_GHOST, GHOST_MODE, r)
            .is_some()
        {
            return;
        }
    }
}

// ------------------------------------------------------------ §8 hooks

/// `0x005B43F0(game, ghost)`, asked by the ghost's NpcStationary AI
/// (§3.6): true when the ghost was talked to and no player is within 5
/// of it. Without chain 22 or before the talk: false.
pub fn ghost_may_leave<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, ghost: UnitId) -> bool {
    let Some(i) = ctl.find(CHAIN) else {
        return false;
    };
    if !x(ctl, i).ghost_talked {
        return false;
    }
    let e = x(ctl, i);
    e.near = false;
    e.unit = Some(ghost);
    // `0x005538D0`: every player.
    for p in w.players() {
        if w.distance_between(p, ghost)
            .is_some_and(|d| d < GHOST_RANGE)
        {
            x(ctl, i).near = true;
        }
    }
    !x(ctl, i).near
}

/// `0x005B4440`, after the AI removed the ghost (§3.6): sound 74 to every
/// player with 25.13 (`0x005B3D30`).
pub fn ghost_removed<W: QuestWorld>(w: &mut W) {
    for p in w.players() {
        if flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            w.attach_sound(p, SOUND_DONE);
        }
    }
}

/// `0x005B4390`, from Izual's AI on its first think (§3.6): status 2 to
/// all while chain 22 is not-intro with status < 2.
pub fn izual_first_think<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if ctl.records[i].not_intro && ctl.records[i].status < 2 {
        status_to_all(ctl, w, i, 2);
    }
}

/// The monster creation switch `0x005B1CF0` for base 256 (§8): link to
/// chain 22 unless the class is 706 (`0x005436B0`). The switch's own
/// `0x005A4850(…, 22, 1)` call belongs to the monster spec (Open
/// question 5).
pub fn link_izual<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    unit: UnitId,
    class: u16,
) -> bool {
    class != UBER_IZUAL && ctl.add_link(w, unit, CHAIN, None)
}

/// Item creation `0x00555D20` for `mss ` (§8): link to chain 22.
pub fn link_soulstone<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, item: UnitId) -> bool {
    ctl.add_link(w, item, CHAIN, None)
}

#[cfg(test)]
#[path = "q1_tests.rs"]
mod tests;
