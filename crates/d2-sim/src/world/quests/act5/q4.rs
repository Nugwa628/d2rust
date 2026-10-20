// Spec: specs/world/quests-act5-2.md §6 (A5Q4 Betrayal of Harrogath), §10, §11
//! A5Q4 Betrayal of Harrogath (chain 34, slot 38): Anya's start and
//! reward lines, the temple portal (Anya in town, dummy 459, Anya's AI),
//! Nihlathak's death and its status timer, game start, the status
//! function and the personalize reward Anya gives (`world/npc.md` §8.1).
//! The sequence function (§6.10) is in `act5.rs`.

use super::super::late::{self, flags, set};
use super::super::{
    bit, event, EventArgs, QuestControl, QuestFlags, QuestRecord, QuestWorld, TextList, TimerFn,
};
use crate::units::{RoomId, UnitId};

const CHAIN: u8 = 34;
const SLOT: u8 = 38;
/// Prison of Ice's slot (the kill credit needs 37.0 or 37.1).
const PRISON: u8 = 37;
/// drehya (Anya in town).
pub const DREHYA: u16 = 512;
/// Harrogath, Nihlathak's Temple, Halls of Pain.
const HARROGATH: u32 = 109;
const TEMPLE: u32 = 121;
const HALLS_OF_PAIN: u32 = 123;
/// The temple portal's object class.
const PORTAL: u16 = 60;
/// `0x007350E8`: table state by record state.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];
/// Anya's start and reward messages.
const MSG_START: u32 = 20137;
const MSG_REWARD: u32 = 20148;
/// Sound and FX of the kill.
const SOUND_KILL: u16 = 82;
const FX_KILL: u8 = 17;
/// The status timer's period.
const TIMER_PERIOD: u32 = 8;
/// Dummy 459's event-7 retry delay.
const RETRY: i32 = 12;

/// Quest extra data (record +0x18, 0x90 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the extra GUID list (event 10 removes from it).
    pub guids: super::super::GuidList,
    /// +0x85: the status timer exists.
    pub timer: bool,
    /// +0x86: Anya started the quest (chat end pending).
    pub anya_started: bool,
    /// +0x87: the temple portal is wanted.
    pub portal_wanted: bool,
    /// +0x88: the temple portal was made.
    pub portal_made: bool,
    /// +0x89: Anya is to open a portal (no Halls of Pain waypoint).
    pub anya_portal: bool,
    /// +0x8C: the kill room.
    pub kill_room: Option<RoomId>,
}

/// Timers this quest makes (`quests.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// `0x0058B770` (period 8): status 4 once Nihlathak is dead.
    Status,
}

/// Init `0x0058BB20` beyond `quests.tsv` (part 1 §2): list +0x00 reset.
pub fn init(r: &mut QuestRecord) {
    r.extra.a5.q4 = Extra::default();
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a5.q4
}

/// Prison of Ice done or pending (37.0 or 37.1).
fn prison_done(f: &QuestFlags) -> bool {
    f.get(PRISON, bit::REWARD_GRANTED) || f.get(PRISON, bit::REWARD_PENDING)
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
        event::MONSTER_KILLED => killed(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            // `0x0058B460`: both lists.
            if let Some(p) = args.player {
                late::leave(ctl, w, i, p);
                let g = w.guid(p);
                x(ctl, i).guids.remove(g);
            }
        }
        event::SCROLL_MESSAGE => messages(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => game_start(ctl, w, i, args),
        _ => return false,
    }
    true
}

/// §6.2 flag iterate `0x0058B0B0`, for every player.
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let state = ctl.records[i].state;
    for p in w.players() {
        let f = flags(w, p);
        if !prison_done(&f) || f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING)
        {
            continue;
        }
        match state {
            2 => set(w, p, SLOT, bit::STARTED),
            3 => set(w, p, SLOT, bit::LEAVE_TOWN),
            _ => {}
        }
    }
}

/// §6.3 event 0 `0x0058B2C0`.
fn chat<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let (Some(p), Some(n)) = (args.player, args.target) else {
        return;
    };
    let Some(class) = w.monster_class(n) else {
        return;
    };
    let f = flags(w, p);
    if !prison_done(&f) {
        return;
    }
    let r = &ctl.records[i];
    if class == DREHYA && r.state == 1 {
        if !f.get(SLOT, bit::REWARD_PENDING) && !f.get(SLOT, bit::REWARD_GRANTED) {
            late::add_state(ctl, i, list, class, 0);
        }
        return;
    }
    let k = if f.get(SLOT, bit::REWARD_PENDING) {
        Some(if f.get(SLOT, 4) { 4 } else { 3 })
    } else if late::guid_listed(ctl, w, i, p) {
        Some(4)
    } else if f.get(SLOT, bit::REWARD_GRANTED) {
        None
    } else if (r.state < 4 || f.get(SLOT, bit::PRIMARY_GOAL_DONE)) && r.not_intro {
        // −1 or past the table → nothing (states never pass 5 here).
        late::table_state(&MSG_STATE, r.state)
    } else {
        None
    };
    if let Some(k) = k {
        late::add_state(ctl, i, list, class, k);
    }
}

/// §6.3 active function `0x0058B870`.
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = f;
    if npc_class != DREHYA {
        return false;
    }
    let pf = flags(w, player);
    if ctl.records[i].state == 1 {
        prison_done(&pf) && !pf.get(SLOT, bit::REWARD_GRANTED) && !pf.get(SLOT, bit::REWARD_PENDING)
    } else {
        pf.get(SLOT, bit::PRIMARY_GOAL_DONE) && !pf.get(SLOT, 4)
    }
}

/// §6.4 event 11 `0x0058B1C0` (NPC 512 only).
fn messages<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.a != u32::from(DREHYA) {
        return;
    }
    match args.b {
        MSG_START => {
            if ctl.records[i].not_intro {
                late::set_state(ctl, i, 2);
                let e = x(ctl, i);
                e.anya_started = true;
                e.portal_wanted = true;
                flag_iterate(ctl, w, i);
            }
        }
        MSG_REWARD => {
            // Edge case 2: the current status byte again, flags := 0.
            let s = ctl.records[i].status;
            late::status_to_all(ctl, w, i, s);
            let Some(p) = args.player else { return };
            set(w, p, SLOT, 4);
            let f = flags(w, p);
            let intro = !ctl.records[i].not_intro;
            if f.get(SLOT, bit::PRIMARY_GOAL_DONE) || (intro && f.get(SLOT, bit::REWARD_PENDING)) {
                late::set_state(ctl, i, 5);
                super::sequence(ctl, w, CHAIN);
            }
        }
        _ => {}
    }
}

/// §6.4 chat end `0x0058B050` (never cleared).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(n) = args.target else { return };
    if w.monster_class(n) != Some(DREHYA) {
        return;
    }
    if x(ctl, i).anya_started {
        late::status_to_all(ctl, w, i, 1);
        x(ctl, i).anya_started = false;
    }
    let e = x(ctl, i);
    if e.portal_wanted && !e.portal_made {
        temple_portal(ctl, w, i, n);
    }
}

/// §6.5 event 3 `0x0058BAB0`.
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    // Edge case 3: no player-bit test.
    if args.a == HARROGATH && ctl.records[i].state == 2 {
        late::set_state(ctl, i, 3);
        flag_iterate(ctl, w, i);
    }
    if args.b == TEMPLE && ctl.records[i].status == 1 {
        late::status_to_all(ctl, w, i, 2);
        x(ctl, i).anya_started = false;
    }
}

/// §6.6 event 8 `0x0058B7A0`: Nihlathak's death.
fn killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !ctl.records[i].not_intro {
        return;
    }
    let Some(victim) = args.target else { return };
    let Some((_, _, room)) = w.unit_position(victim) else {
        return;
    };
    x(ctl, i).kill_room = Some(room);
    // `0x0058B4A0`: the kill room or adjacent (edge case 1).
    let near = w.players_near(victim);
    for p in w.players() {
        let f = flags(w, p);
        if !f.get(SLOT, bit::COMPLETED_NOW)
            && !f.get(SLOT, bit::REWARD_GRANTED)
            && near.contains(&p)
            && prison_done(&f)
            && !f.get(SLOT, bit::REWARD_PENDING)
        {
            set(w, p, SLOT, bit::REWARD_PENDING);
            set(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
        }
    }
    // `0x0058B680`: 38.13 → party members `0x0058B5B0`.
    for p in w.players() {
        if !flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            continue;
        }
        for m in late::party_of(w, p) {
            let f = flags(w, m);
            if [
                bit::REWARD_GRANTED,
                bit::COMPLETED_NOW,
                bit::REWARD_PENDING,
                bit::PRIMARY_GOAL_DONE,
            ]
            .iter()
            .all(|&b| !f.get(SLOT, b))
                && late::in_act(w, m, 4)
                && prison_done(&f)
            {
                set(w, m, SLOT, bit::REWARD_PENDING);
                set(w, m, SLOT, bit::PRIMARY_GOAL_DONE);
            }
        }
    }
    // `0x0058B6D0`.
    late::completion_flag(
        w,
        CHAIN,
        SLOT,
        &[
            bit::REWARD_GRANTED,
            bit::REWARD_PENDING,
            bit::PRIMARY_GOAL_DONE,
        ],
    );
    // `0x0058B730`.
    for p in w.players() {
        if flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            w.attach_sound(p, SOUND_KILL);
        }
    }
    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
    ctl.unique_event(w, FX_KILL);
    late::set_state(ctl, i, 4);
    if !x(ctl, i).timer {
        x(ctl, i).timer = true;
        let t = TimerFn::Act5(super::Timer::Q4(Timer::Status));
        if let Err(e) = ctl.add_timer(CHAIN, t, TIMER_PERIOD) {
            ctl.faults.push(e);
        }
    }
}

/// §6.8 event 13 `0x0058B990`.
fn game_start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if !ctl.records[i].not_intro {
        ctl.records[i].state = 5;
        if !w.waypoint_active(p, HALLS_OF_PAIN) {
            x(ctl, i).anya_portal = true;
        }
        return;
    }
    let f = flags(w, p);
    if f.get(SLOT, bit::REWARD_PENDING) {
        return;
    }
    if f.get(SLOT, bit::LEAVE_TOWN) {
        x(ctl, i).portal_wanted = true;
        // Status 2 to all, the flags byte kept.
        ctl.records[i].status = 2;
        late::iterate_all(ctl, w, i);
        late::set_state(ctl, i, 3);
    } else if f.get(SLOT, bit::STARTED) {
        x(ctl, i).portal_wanted = true;
        // Status 1 to all, the flags byte kept (`0x0058BA86`: iterate 1,
        // no flags write; `quests-act5-2.md` §6.8).
        ctl.records[i].status = 1;
        late::iterate_all(ctl, w, i);
        late::set_state(ctl, i, 2);
    }
}

/// §6.9 status function `0x0058AF00` (always true).
pub fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    let _ = (w, player, f);
    let r = &ctl.records[i];
    Some(if pf.get(SLOT, bit::REWARD_GRANTED) {
        0
    } else if pf.get(SLOT, bit::REWARD_PENDING) || pf.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        4 + u8::from(pf.get(SLOT, 4))
    } else if !r.not_intro {
        0
    } else if pf.get(SLOT, bit::COMPLETED_NOW) {
        12
    } else if r.state > 3 {
        0
    } else {
        r.status
    })
}

/// Runs a timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    match t {
        Timer::Status => {
            // `0x0058B770`: returns 1.
            if let Some(i) = ctl.find(chain) {
                if ctl.records[i].status != 4 {
                    late::status_to_all(ctl, w, i, 4);
                }
                x(ctl, i).timer = false;
            }
            true
        }
    }
}

/// §6.7 `0x0058AF90`: the temple portal at (U.x + 10, U.y + 5) in U's
/// room, once; true when made.
fn temple_portal<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, u: UnitId) -> bool {
    if x(ctl, i).portal_made {
        return false;
    }
    let Some((ux, uy, _)) = w.unit_position(u) else {
        return false;
    };
    if !w.create_portal(u, ux + 10, uy + 5, PORTAL, TEMPLE) {
        return false;
    }
    let e = x(ctl, i);
    e.portal_made = true;
    e.portal_wanted = false;
    true
}

/// `0x0058B940`: object event 7 of dummy 459 (the temple portal, §6.7):
/// +0x87 set → try; not made → event 7 again at frame + 12.
pub fn temple_portal_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !x(ctl, i).portal_wanted {
        return;
    }
    if !temple_portal(ctl, w, i, object) {
        let at = w.frame() + w.tick_rate().ticks(RETRY);
        w.schedule_quest_event(object, at);
    }
}

/// `0x0058BC80` (§6.7, from Anya's AI at `0x005E72B3`): +0x89 set and
/// Anya in Act V → the temple portal at (U.x + 10, U.y + 5); made →
/// +0x89 := 0.
pub fn anya_ai_portal<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, anya: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !x(ctl, i).anya_portal || !late::in_act(w, anya, 4) {
        return;
    }
    let Some((ux, uy, _)) = w.unit_position(anya) else {
        return;
    };
    if w.create_portal(anya, ux + 10, uy + 5, PORTAL, TEMPLE) {
        x(ctl, i).anya_portal = false;
    }
}

/// `0x0058BC00` (§6.11, from Anya's personalize service, `world/npc.md`
/// §8.1): set 38.0, clear 38.1 (38.15 is read, unused). Nothing is
/// sent.
pub fn personalize_reward<W: QuestWorld>(w: &mut W, player: UnitId) {
    set(w, player, SLOT, bit::REWARD_GRANTED);
    late::clear(w, player, SLOT, bit::REWARD_PENDING);
}

/// `0x0058BC40` (§6.11, from the Nihlathak AI at `0x005EE5F9`):
/// not-intro and status < 3 → status 3 to all.
pub fn nihlathak_ai_status<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let r = &ctl.records[i];
    if r.not_intro && r.status < 3 {
        late::status_to_all(ctl, w, i, 3);
    }
}

#[cfg(test)]
#[path = "q4_tests.rs"]
mod tests;
