// Spec: specs/world/quests-act4.md §5 (A4Q2 Terror's End, chain 23, slot 26), §7, §8
//! A4Q2 Terror's End callback by callback: chat and messages (events 0,
//! 2, 11; the active function), the flag iterate, level change, start
//! and join (events 3, 13, 14), the seals and the seal-boss dummy, the
//! Sanctum clear, Diablo's spawn and death (event 8), the end of a
//! classic game, the portal to Harrogath and the classic-only gate.
//! The object functions and the hooks other systems call are `pub fn`s
//! here (§1.4, §8).
//!
//! The classic end-of-game schedule reads `GetTickCount` in 1.14d
//! (§5.8); `d2-sim` takes the elapsed time as 40 ms × frames since the
//! kill (open question 2), so +0x18 holds the kill's frame.

use super::super::late::{self, flags};
use super::super::{bit, event, npc, EventArgs, QuestControl, QuestFlags, QuestWorld, TextList};
use crate::units::{RoomId, UnitId};
use crate::world::quests::TimerFn;

const CHAIN: u8 = 23;
const SLOT: u8 = 26;
/// Slot of the Act V "able to go" bit (28.0, 28.13).
const ACT5_SLOT: u8 = 28;
/// Monster classes: diablo, cain4.
const DIABLO: u16 = 243;
const CAIN4: u16 = 246;
/// Levels: The Pandemonium Fortress, Chaos Sanctum, Harrogath.
const FORTRESS: u32 = 103;
const SANCTUM: u32 = 108;
const HARROGATH: u32 = 109;
/// Objects: the seal-boss dummy, the first seal, the Harrogath portal.
const DUMMY: u16 = 131;
const SEAL_FIRST: u16 = 392;
const PORTAL: u16 = 566;
/// FX bytes (0x89): Sanctum cleared, Diablo killed (classic).
const FX_CLEARED: u8 = 12;
const FX_DIABLO: u8 = 13;
/// Sounds: refused, Diablo completion.
const SOUND_REFUSED: u16 = 19;
const SOUND_DONE: u16 = 75;
/// `0x0073D56C`: message state by quest state 0–4.
const MSG_STATE: [i8; 5] = [-1, 0, 1, -1, -1];
/// Unit flags given to Diablo and the portal.
const SPAWN_FLAGS: u32 = 0x0300_0000;
/// The boss seals (§5.4): (offset x, offset y, free-spot radius, data
/// tables entry) of seals 392, 394, 396; index = pair (+0x24, +0x2C,
/// +0x34).
const BOSS_SEALS: [(i32, i32, u32, u8); 3] =
    [(-12, -52, 13, 36), (-39, 33, 14, 37), (32, 16, 15, 38)];

/// Quest extra data (record +0x18, 0x4C bytes, §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: Tyrael started the quest (chat end pending).
    pub started: bool,
    /// +0x01: the timer exists.
    pub timer: bool,
    /// +0x02: Diablo to spawn.
    pub spawn_pending: bool,
    /// +0x03: the end sequence runs.
    pub ending: bool,
    /// +0x04: end the game pending.
    pub end_pending: bool,
    /// +0x05: the warp pending.
    pub warp_pending: bool,
    /// +0x06: Diablo's start point is initialised; +0x08 its GUID.
    pub start_known: bool,
    pub start_guid: u32,
    /// +0x0C … +0x10: seals 392 … 396 opened.
    pub seals: [bool; 5],
    /// +0x11: Diablo spawned.
    pub spawned: bool,
    /// +0x13: the Sanctum is cleared.
    pub cleared: bool,
    /// +0x14: Diablo killed.
    pub killed: bool,
    /// +0x15: the save pass is done.
    pub saved: bool,
    /// +0x18: timer firings before the spawn, then the kill's frame
    /// (open question 2: frames, not `GetTickCount`).
    pub counter: i32,
    /// +0x1C: players credited.
    pub credited: u32,
    /// +0x20: the last player the end-game warp handled (never read).
    pub last_warped: Option<UnitId>,
    /// +0x24, +0x2C, +0x34: the seal-boss positions.
    pub bosses: [(i32, i32); 3],
    /// +0x3C: Diablo's death room.
    pub diablo_room: Option<RoomId>,
    /// +0x40: the portal mode seen.
    pub portal_mode: u8,
    /// +0x44: portal request (never set to 1 in 1.14d).
    pub portal_request: bool,
    /// +0x45: the portal spawned.
    pub portal_spawned: bool,
    /// +0x48: non-Diablo chain-23 kills (the seal bosses).
    pub kills: u32,
}

/// Timers this quest makes (`quests.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// `0x005B4BE0`: Diablo's spawn, then the end of a classic game
    /// (period 1).
    Diablo,
}

fn x2(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a4.q2
}

/// Init beyond `quests.tsv` (§2: state 0, extra zeroed).
pub fn init(r: &mut super::super::QuestRecord) {
    r.extra.a4.q2 = Extra::default();
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
        event::NPC_ACTIVATE => npc_text(ctl, w, i, args, list),
        event::NPC_DEACTIVATE => chat_end(ctl, w, i, args),
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => kill(ctl, w, i, args),
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => started(ctl, w, i, args),
        event::PLAYER_JOINED_GAME => {
            // `0x005B5030`.
            if let Some(p) = args.player {
                forget_portal_talk(w, p);
            }
        }
        _ => return false,
    }
    true
}

/// The active function `0x005B4450` (§5.2 "wants to talk").
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = f;
    let r = flags(w, player);
    let x = w.expansion();
    let done = r.get(SLOT, bit::REWARD_GRANTED);
    match npc_class {
        npc::TYRAEL2 => {
            (!done && !r.get(SLOT, bit::COMPLETED_BEFORE) && ctl.records[i].state == 1)
                || (done && if x { !r.get(SLOT, 9) } else { r.get(SLOT, 7) })
        }
        CAIN4 => done && if x { !r.get(SLOT, 8) } else { r.get(SLOT, 6) },
        _ => false,
    }
}

/// The status function; `f` is its address (chain 23 has none, §2).
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
        Timer::Diablo => ctl.find(chain).is_none_or(|i| diablo_timer(ctl, w, i)),
    }
}

// ------------------------------------------------------------ §5.2

/// Event 0 `0x005B4790`.
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let (Some(p), Some(n)) = (args.player, args.target) else {
        return;
    };
    // npc = −1 adds nothing: the table rows all name a class.
    let Some(c) = w.monster_class(n) else { return };
    let r = flags(w, p);
    let x = w.expansion();
    let add = |k: u8, list: Option<&mut TextList>| late::add_state(ctl, i, list, c, k);
    // Step 1 (classic).
    if !x {
        if r.get(SLOT, 7) && c == npc::TYRAEL2 {
            return add(2, list);
        }
        if r.get(SLOT, 6) && c == CAIN4 {
            return add(2, list);
        }
        if r.get(SLOT, 7) {
            if c == CAIN4 {
                add(3, list);
            }
            return;
        }
        if r.get(SLOT, 6) {
            if c == npc::TYRAEL2 {
                add(3, list);
            }
            return;
        }
    }
    // Step 2.
    if r.get(SLOT, bit::REWARD_GRANTED) {
        if !x {
            return;
        }
        let mut list = list;
        if !r.get(SLOT, 9) {
            match c {
                npc::TYRAEL2 => add(4, list.as_deref_mut()),
                CAIN4 => add(5, list.as_deref_mut()),
                _ => {}
            }
        }
        if !r.get(SLOT, 8) {
            match c {
                CAIN4 => add(4, list),
                npc::TYRAEL2 => add(5, list),
                _ => {}
            }
        }
        return;
    }
    // Step 3.
    let rd = &ctl.records[i];
    let state = rd.state;
    if (state < 4 || r.get(SLOT, bit::PRIMARY_GOAL_DONE)) && rd.not_intro && state != 0 {
        if let Some(k) = late::table_state(&MSG_STATE, state) {
            add(k, list);
        }
    }
}

/// Event 11 `0x005B4670` (a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    match (args.a as u16, args.b) {
        (npc::TYRAEL2, 681) => {
            // Not state-guarded (edge case 1).
            x2(ctl, i).started = true;
            late::set_state(ctl, i, 2);
            flag_iterate_all(ctl, w, i);
            if let Some(n) = args.target {
                late::refresh(ctl, w, p, n);
            }
        }
        (npc::TYRAEL2, 684) => late::clear(w, p, SLOT, 7),
        (npc::TYRAEL2, 20000) => {
            if w.expansion() {
                late::set(w, p, SLOT, 9);
                if !x2(ctl, i).portal_spawned {
                    if let Some(n) = args.target {
                        portal_spawn(ctl, w, i, n);
                    }
                }
            }
        }
        (CAIN4, 685) => late::clear(w, p, SLOT, 6),
        (CAIN4, 20001) => late::set(w, p, SLOT, 8),
        _ => {}
    }
}

/// Event 2 `0x005B4F90` (never cleared).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(n) = args.target else { return };
    if w.monster_class(n) != Some(npc::TYRAEL2) {
        return;
    }
    if x2(ctl, i).started {
        late::status_to_all(ctl, w, i, 1);
        x2(ctl, i).started = false;
    }
    // Dead in 1.14d: +0x44 is never set (edge case 14).
    let x = &ctl.records[i].extra.a4.q2;
    if w.expansion() && x.portal_request && !x.portal_spawned {
        portal_spawn(ctl, w, i, n);
    }
}

// ------------------------------------------------------------ §5.3

/// The flag iterate `0x005B4560` for every player: players lacking 26.0
/// get 26.2 (state 2), 26.3 (state 3, status 1) or 26.4 (state 3).
fn flag_iterate_all<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let (state, status) = (ctl.records[i].state, ctl.records[i].status);
    for p in w.players() {
        if flags(w, p).get(SLOT, bit::REWARD_GRANTED) {
            continue;
        }
        match state {
            2 => late::set(w, p, SLOT, bit::STARTED),
            3 if status == 1 => late::set(w, p, SLOT, bit::LEAVE_TOWN),
            3 => late::set(w, p, SLOT, bit::ENTER_AREA),
            _ => {}
        }
    }
}

/// Event 3 `0x005B4EA0` (a = old level, b = new level; no quick remove).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.a == FORTRESS && ctl.records[i].state == 2 {
        let has = args
            .player
            .is_some_and(|p| flags(w, p).get(SLOT, bit::REWARD_GRANTED));
        if has {
            return;
        }
        if ctl.records[i].state < 3 {
            late::set_state(ctl, i, 3);
        }
        if ctl.records[i].status < 1 {
            ctl.records[i].flags = 0;
            late::status_silent(ctl, i, 1);
        }
        flag_iterate_all(ctl, w, i);
    }
    if args.b == SANCTUM && ctl.records[i].not_intro {
        if ctl.records[i].state < 3 {
            late::set_state(ctl, i, 3);
        }
        if ctl.records[i].status < 1 {
            late::status_to_all(ctl, w, i, 1);
        }
        flag_iterate_all(ctl, w, i);
    }
}

/// Events 13 and 14's first step: expansion, 26.9 set and 28.0 clear →
/// clear 26.9 (edge case 15).
fn forget_portal_talk<W: QuestWorld>(w: &mut W, p: UnitId) {
    let r = flags(w, p);
    if w.expansion() && r.get(SLOT, 9) && !r.get(ACT5_SLOT, bit::REWARD_GRANTED) {
        late::clear(w, p, SLOT, 9);
    }
}

/// Event 13 `0x005B5080`.
fn started<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    forget_portal_talk(w, p);
    let r = flags(w, p);
    if r.get(SLOT, bit::REWARD_GRANTED) || r.get(SLOT, bit::COMPLETED_BEFORE) {
        return;
    }
    let rd = &mut ctl.records[i];
    if r.get(SLOT, bit::ENTER_AREA) {
        (rd.status, rd.state) = (2, 3);
    } else if r.get(SLOT, bit::LEAVE_TOWN) {
        (rd.state, rd.status) = (3, 1);
    } else if r.get(SLOT, bit::STARTED) {
        (rd.state, rd.status) = (2, 1);
    }
}

// ------------------------------------------------------------ §5.4

/// The seal activation `0x005B5630` (operate 52, seals 393 and 395, and
/// the tail of the boss seals). `class` is the seal's object class.
/// Returns 0 in 1.14d; no player test.
pub fn seal_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
    class: u16,
) {
    let _ = player;
    seal_activate(ctl, w, object, class);
}

fn seal_activate<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId, class: u16) {
    if w.object_mode(object) != 0 {
        return;
    }
    w.set_object_mode(object, 1);
    // Twice the animation's frames, one per base tick.
    let fc1 = w.object_frame_count1(object);
    let at = w.frame() + w.tick_rate().ticks(2 * fc1);
    w.schedule_object_event(object, 1, at);
    let Some(i) = ctl.find(CHAIN) else { return };
    // The jump table `0x005B571C`: class − 392 → +0x0C … +0x10.
    if let Some(k) = class.checked_sub(SEAL_FIRST).filter(|&k| k < 5) {
        x2(ctl, i).seals[usize::from(k)] = true;
    }
    trigger(ctl, w, i);
}

/// Operate 54 `0x005B6B70` (seal 392, Infector of Souls).
pub fn infector_seal_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
    class: u16,
) {
    let _ = player;
    boss_seal(ctl, w, object, class, 0);
}

/// Operate 55 `0x005B6BC0` (seal 394, Lord De Seis).
pub fn de_seis_seal_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
    class: u16,
) {
    let _ = player;
    boss_seal(ctl, w, object, class, 1);
}

/// Operate 56 `0x005B6C10` (seal 396, Grand Vizier of Chaos).
pub fn vizier_seal_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
    class: u16,
) {
    let _ = player;
    boss_seal(ctl, w, object, class, 2);
}

/// A boss seal: the boss spot (seal + offset) into pair `k`, then
/// `0x005B6AD0`: the room covering it, a free spot (the pair updated in
/// place), the dummy 131 there, the room refresh and the activation. No
/// spot or no object → the seal stays in mode 0.
fn boss_seal<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    class: u16,
    k: usize,
) {
    if w.object_mode(object) != 0 {
        return;
    }
    let Some(i) = ctl.find(CHAIN) else { return };
    let Some((sx, sy, room)) = w.unit_position(object) else {
        return;
    };
    let (dx, dy, r, _) = BOSS_SEALS[k];
    let (bx, by) = (sx + dx, sy + dy);
    x2(ctl, i).bosses[k] = (bx, by);
    let Some(room) = w.room_at(room, bx, by) else {
        return;
    };
    let Some((fx, fy, room)) = w.free_spot_at(room, bx, by, 3, 0x3F11, r, 100) else {
        return;
    };
    x2(ctl, i).bosses[k] = (fx, fy);
    if w.spawn_quest_object(room, fx, fy, DUMMY).is_none() {
        return;
    }
    w.set_room_portal(room, false);
    seal_activate(ctl, w, object, class);
}

/// Init 59 `0x0054FE10` of the dummy (class 131, `quests.md` §9 rule 7):
/// in mode 0, mode 1, object event 7 at f + 27 and event 1 at
/// f + `FrameCnt1` + 1.
pub fn dummy_init<W: QuestWorld>(w: &mut W, object: UnitId) {
    if w.object_mode(object) != 0 {
        return;
    }
    w.set_object_mode(object, 1);
    let f = w.frame();
    w.schedule_quest_event(object, f + w.tick_rate().ticks(27));
    let fc1 = w.object_frame_count1(object);
    let at = f + w.tick_rate().ticks(fc1) + 1;
    w.schedule_object_event(object, 1, at);
}

/// `0x005B5750`: object event 7 of the seal-boss dummy (class 131) in
/// level 108 (§5.4): at a boss pair, spawn that pair's superunique;
/// failure → event 7 again at f + 10.
pub fn dummy_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let Some((x, y, _)) = w.unit_position(object) else {
        return;
    };
    let bosses = ctl.records[i].extra.a4.q2.bosses;
    let Some(k) = bosses.iter().position(|&b| b == (x, y)) else {
        return;
    };
    let id = w.superunique_id(BOSS_SEALS[k].3);
    let (px, py) = bosses[k];
    if w.spawn_superunique(object, px, py, 2, id).is_none() {
        let at = w.frame() + w.tick_rate().ticks(10);
        w.schedule_quest_event(object, at);
    }
}

// ------------------------------------------------------------ §5.5, §5.6

/// The Sanctum clear `0x005B5230` (once): FX 12, then every evil, live,
/// non-Diablo monster of level 108's active rooms dies.
fn clear_sanctum<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    if x2(ctl, i).cleared {
        return;
    }
    x2(ctl, i).cleared = true;
    ctl.unique_event(w, FX_CLEARED);
    for m in w.level_monsters(SANCTUM) {
        if w.monster_class(m) == Some(DIABLO) || w.unit_dead(m) || w.alignment(m) != 0 {
            continue;
        }
        w.remove_monster(m);
    }
}

/// The spawn trigger (§5.6): all five seals open and exactly three
/// non-Diablo kills → the Sanctum clear, then the timer when none
/// exists (edge case 10).
fn trigger<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    let x = &ctl.records[i].extra.a4.q2;
    if !x.seals.iter().all(|&s| s) || x.kills != 3 {
        return;
    }
    clear_sanctum(ctl, w, i);
    if !x2(ctl, i).timer {
        let x = x2(ctl, i);
        x.counter = 0;
        x.spawn_pending = true;
        x.ending = false;
        x.timer = true;
        start_timer(ctl);
    }
}

fn start_timer(ctl: &mut QuestControl) {
    let func = TimerFn::Act4(super::Timer::Q2(Timer::Diablo));
    if let Err(e) = ctl.add_timer(CHAIN, func, 1) {
        ctl.faults.push(e);
    }
}

/// Init 55 `0x005B5590` of object 255 (Diablo's start point, §5.6).
pub fn start_point_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if x2(ctl, i).spawned {
        return;
    }
    let g = w.guid(object);
    let x = x2(ctl, i);
    x.start_known = true;
    x.start_guid = g;
    trigger(ctl, w, i);
}

/// Timer `0x005B4BE0`; true = remove it.
fn diablo_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if x2(ctl, i).spawn_pending {
        let x = x2(ctl, i);
        x.counter += 1;
        if x.counter < 10 || !x.start_known {
            return false;
        }
        let g = x.start_guid;
        // A missing start point spawns nothing (a failed spawn, retried).
        let Some((o, _)) = w.object_by_guid(g) else {
            return false;
        };
        let Some((sx, sy, room)) = w.unit_position(o) else {
            return false;
        };
        // `0x005B4B60`: `0x005B2F20` with r = −1, then 5, then 10.
        for r in [u32::MAX, 5, 10] {
            if let Some(d) = w.spawn_monster(room, sx, sy, DIABLO, 1, r) {
                w.or_unit_flags(d, SPAWN_FLAGS);
                let x = x2(ctl, i);
                x.spawned = true;
                x.spawn_pending = false;
                x.timer = false;
                return true;
            }
        }
        return false;
    }
    if !x2(ctl, i).ending {
        return true;
    }
    end_of_game(ctl, w, i)
}

// ------------------------------------------------------------ §5.7

/// Event 8 `0x005B52E0` (victim = target, killing player = player).
fn kill<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(v) = args.target else { return };
    if w.monster_class(v) != Some(DIABLO) {
        let x = x2(ctl, i);
        x.kills = x.kills.wrapping_add(1);
        if x.kills == 3 {
            trigger(ctl, w, i);
        }
        return;
    }
    let classic = !w.expansion();
    // Step 1 (edge case 11: intro games too).
    if classic {
        ctl.unique_event(w, FX_DIABLO);
    }
    x2(ctl, i).killed = true;
    let Some((_, _, room)) = w.unit_position(v) else {
        return;
    };
    x2(ctl, i).diablo_room = Some(room);
    // Step 2.
    if args.player.is_some() && !x2(ctl, i).timer {
        let now = w.frame();
        let x = x2(ctl, i);
        x.spawn_pending = false;
        x.ending = true;
        x.end_pending = true;
        x.warp_pending = true;
        x.saved = false;
        x.timer = true;
        x.counter = now;
        start_timer(ctl);
    }
    // Step 3.
    if !ctl.records[i].not_intro {
        return;
    }
    x2(ctl, i).credited = 0;
    // "status 13 to all" without clearing the flags byte.
    ctl.records[i].status = 13;
    late::iterate_all(ctl, w, i);
    // `0x005B5140`: same or adjacent room as Diablo's death room.
    let near = w.players_near(v);
    for p in w.players() {
        let r = flags(w, p);
        if near.contains(&p)
            && !r.get(SLOT, bit::REWARD_GRANTED)
            && !r.get(SLOT, bit::REWARD_PENDING)
        {
            credit(ctl, w, i, p);
        }
    }
    // `0x005B4DF0` / `0x005B4DA0`: party members in Act IV of credited
    // players.
    for p in w.players() {
        if !flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            continue;
        }
        for m in late::party_of(w, p) {
            if late::in_act(w, m, 3) && !flags(w, m).get(SLOT, bit::REWARD_GRANTED) {
                credit(ctl, w, i, m);
            }
        }
    }
    // `0x005B4970`.
    late::completion_flag(w, CHAIN, SLOT, &[bit::REWARD_GRANTED]);
    // `0x005B49C0`.
    for p in w.players() {
        if flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            late::s5d(w, p, CHAIN, 2, 0);
            w.attach_sound(p, SOUND_DONE);
        }
    }
    // Then `0x00545990` (a `ret 4` stub) +0x1C times when the killer
    // lacked 26.0: no effect (edge case 18).
}

/// Credit `0x005B4D20`: 26.13, 26.0, reset_progress(26); classic: 26.6,
/// 26.7 and `0x00538680(client, 4, difficulty)` (`0x005B4D77`,
/// `quests-act1-rest.md` §5: save progression raised); +0x1C += 1.
fn credit<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    late::set(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
    late::set(w, p, SLOT, bit::REWARD_GRANTED);
    late::reset_progress(w, p, SLOT);
    if !w.expansion() {
        late::set(w, p, SLOT, 6);
        late::set(w, p, SLOT, 7);
        let d = w.difficulty();
        super::super::raise_progression(w, p, 4, d);
    }
    let x = x2(ctl, i);
    x.credited = x.credited.wrapping_add(1);
}

// ------------------------------------------------------------ §5.8

/// The timer's end part (§5.8), checked at each firing. Elapsed time =
/// the real time since the kill (40 ms per base tick). True = remove.
fn end_of_game<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    let elapsed = i64::from(w.frame()) - i64::from(x2(ctl, i).counter);
    let ms = elapsed * 1000 / i64::from(w.tick_rate().hz());
    let classic = !w.expansion();
    if x2(ctl, i).end_pending && ms > 95_000 {
        let x = x2(ctl, i);
        x.timer = false;
        x.ending = false;
        x.end_pending = false;
        if classic {
            // `0x00530590(game, 0)`: a host request
            // (`quests-helpers.md` §6).
            ctl.end_game();
        }
        return true;
    }
    if ms > 90_000 {
        if x2(ctl, i).warp_pending {
            if classic {
                for p in w.players() {
                    end_warp(ctl, w, i, p);
                }
            }
            x2(ctl, i).warp_pending = false;
        }
        return false;
    }
    if ms > 75_000 && !x2(ctl, i).saved {
        x2(ctl, i).saved = true;
        ctl.save_pass();
    }
    false
}

/// `0x005B4A80` for one player: end its interaction, +0x4C := 1; 26.13
/// → warp to 103 and `5D 17 01 00 0000`; else a 0x50 with u16 23.
fn end_warp<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    w.end_interaction(p);
    w.set_player_byte_4c(p, 1);
    x2(ctl, i).last_warped = Some(p);
    if flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
        w.warp_to_level(p, FORTRESS, 0);
        late::s5d(w, p, CHAIN, 1, 0);
    } else {
        // Bytes 3–14 are caller-frame stack in 1.14d, never written and
        // never read by the client (open question 3, edge case 22): 0
        // here, masked by the traces.
        let mut m = [0u8; 15];
        m[0] = 0x50;
        m[1..3].copy_from_slice(&u16::from(CHAIN).to_le_bytes());
        w.send(p, &m);
    }
}

// ------------------------------------------------------------ §5.9

/// `0x005B45E0`: the portal to Harrogath beside Tyrael (at most one per
/// game).
fn portal_spawn<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, tyrael: UnitId) {
    let Some((tx, ty, room)) = w.unit_position(tyrael) else {
        return;
    };
    let Some((x, y, room)) = w.free_spot_at(room, tx + 5, ty, 2, 0x400, 12, 100) else {
        return;
    };
    let Some(o) = w.place_object(room, x, y, PORTAL, [1, 1, 0]) else {
        return;
    };
    let x = x2(ctl, i);
    x.portal_request = false;
    x.portal_spawned = true;
    w.or_unit_flags(o, SPAWN_FLAGS);
}

/// Init 78 `0x005B5840` of object 566: mode 1 the first time, else 2.
pub fn portal_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if x2(ctl, i).portal_mode != 2 {
        x2(ctl, i).portal_mode = 2;
        w.set_object_mode(object, 1);
    } else {
        w.set_object_mode(object, 2);
    }
}

/// Operate 73 `0x005B5880` of object 566 (returns 0): the act change
/// to Harrogath (edge case 14: 26.13 without 26.0 is accepted).
pub fn portal_operate<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    let _ = (ctl, object);
    if !w.expansion() {
        return;
    }
    let r = flags(w, player);
    if !r.get(SLOT, bit::REWARD_GRANTED) && !r.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        w.attach_sound(player, SOUND_REFUSED);
        return;
    }
    if !late::in_act(w, player, 3) {
        return;
    }
    if !r.get(ACT5_SLOT, bit::REWARD_GRANTED) {
        late::set(w, player, ACT5_SLOT, bit::REWARD_GRANTED);
        late::set(w, player, ACT5_SLOT, bit::PRIMARY_GOAL_DONE);
        if w.client_idle(player) {
            w.set_interact_unit(player, None);
            w.set_player_byte_4c(player, 1);
            late::s5d(w, player, CHAIN, 2, 0);
            w.send(player, &[0x61, 5]);
        }
    }
    w.act_change(player, HARROGATH, 5);
    late::send_flags(w, player);
    let d = w.difficulty();
    w.activate_waypoint(player, HARROGATH, d);
}

// ------------------------------------------------------------ §5.10, §8

/// `0x005B5810` (§5.10): true = refuse the interaction (callers
/// `0x00566E60`, `0x00567620`, `0x00568060`): a classic game with no
/// chain 23 or with Diablo killed.
pub fn interaction_refused<W: QuestWorld>(ctl: &QuestControl, w: &W) -> bool {
    if w.expansion() {
        return false;
    }
    ctl.record(CHAIN).is_none_or(|r| r.extra.a4.q2.killed)
}

/// `0x005B5210` (§8, `monsters/population.md` §3.1 step 6): +0x13 of
/// chain 23, the Sanctum cleared (level 108 is no longer populated).
/// Without chain 23 it returns 0 (`0x005B5226`).
pub fn sanctum_cleared(ctl: &QuestControl) -> bool {
    ctl.record(CHAIN).is_some_and(|r| r.extra.a4.q2.cleared)
}

#[cfg(test)]
#[path = "q2_tests.rs"]
mod tests;
