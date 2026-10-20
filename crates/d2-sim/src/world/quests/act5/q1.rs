// Spec: specs/world/quests-act5.md §3 (A5Q1 Siege on Harrogath, chain 31)
//! A5Q1 callback by callback: the flag iterate (§3.2), Larzuk's chat,
//! messages and chat end (§3.3, §3.4), level changes (§3.5), Shenk's
//! death (§3.6), game start and leave (§3.7), Larzuk's start dummy, the
//! map-AI store and Shenk's activation hook (§3.8), the socket reward
//! (§3.9) and the status and active functions (§3.3, §3.10). Slot 35.

use super::super::late::{
    add_guid, add_state, completion_flag, flags, in_act, leave, party_of, quick_remove, refresh,
    reset_progress, set, set_state, status_to_all, table_state,
};
use super::super::{
    bit, event, flags_of, EventArgs, QuestControl, QuestError, QuestFlags, QuestRecord, QuestWorld,
    TextList, UnitKind,
};
use crate::units::{RoomId, UnitId};

const CHAIN: u8 = 31;
const SLOT: u8 = 35;
/// Larzuk.
pub const LARZUK: u16 = 511;
/// Malah (her intro bit starts the quest, §3.7).
const MALAH: u16 = 513;
/// Shenk's superunique id (Siege Boss).
pub const SIEGE_BOSS: u32 = 42;
/// 35.3 / 35.4 / 35.5 (§3.1).
const LEFT_TOWN: u8 = 3;
const LEFT_AFTER_SHENK: u8 = 4;
const TOLD_LARZUK: u8 = 5;
/// `0x00733928`: table state by record state.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];
/// Shenk's death sound and FX byte (§3.6).
const SOUND_SHENK: u16 = 80;
const FX_SHENK: u8 = 15;

/// Quest extra data (record +0x18, 0x18 bytes, §3.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: Larzuk's map-AI record (0: none stored).
    pub map_ai: u32,
    /// +0x04: the room Shenk died in.
    pub kill_room: Option<RoomId>,
    /// +0x08: Larzuk started the quest (chat end pending).
    pub started: bool,
    /// +0x10: Larzuk's GUID.
    pub larzuk_guid: u32,
    /// +0x15: Larzuk spawned.
    pub larzuk_spawned: bool,
    /// +0x16: the reward talk (chat end pending).
    pub reward_talk: bool,
    /// +0x17: the stored map AI was applied.
    pub map_ai_applied: bool,
}

/// Timers this quest makes (`quests.md` §5): none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {}

/// Init `0x005876E0` beyond `quests.tsv` (§2): extra zeroed.
pub fn init(r: &mut QuestRecord) {
    r.extra.a5.q1 = Extra::default();
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a5.q1
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
        event::MONSTER_KILLED => shenk_killed(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            // `0x005870D0`.
            if let Some(p) = args.player {
                leave(ctl, w, i, p);
            }
        }
        event::SCROLL_MESSAGE => messages(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => game_start(ctl, w, i, args),
        _ => return false,
    }
    true
}

/// Flag iterate `0x00586DE0` (§3.2) for every player.
fn flag_iterate_all<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let (state, status) = (ctl.records[i].state, ctl.records[i].status);
    for p in w.players() {
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match state {
            2 => f.set(SLOT, bit::STARTED),
            3 if status == 2 => f.set(SLOT, LEFT_AFTER_SHENK),
            3 => f.set(SLOT, LEFT_TOWN),
            _ => {}
        }
    }
}

/// Event 0 `0x00586FF0` (§3.3).
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
    if f.get(SLOT, bit::REWARD_PENDING) {
        if !f.get(SLOT, TOLD_LARZUK) {
            add_state(ctl, i, list, class, 3);
        }
        return;
    }
    if f.get(SLOT, bit::REWARD_GRANTED) {
        return;
    }
    let r = &ctl.records[i];
    if (r.state < 4 || f.get(SLOT, bit::PRIMARY_GOAL_DONE)) && r.not_intro {
        if let Some(m) = table_state(&MSG_STATE, r.state).filter(|&m| m < 8) {
            add_state(ctl, i, list, class, m);
        }
    }
}

/// Active `0x005874E0` (§3.3).
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = f;
    if npc_class != LARZUK {
        return false;
    }
    let pf = flags(w, player);
    let r = &ctl.records[i];
    (pf.get(SLOT, bit::REWARD_PENDING) && !pf.get(SLOT, TOLD_LARZUK))
        || (r.not_intro
            && r.state == 1
            && !pf.get(SLOT, bit::REWARD_GRANTED)
            && !pf.get(SLOT, bit::REWARD_PENDING))
}

/// Event 11 `0x00586E70` (§3.4; a = NPC class, b = message).
fn messages<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a != u32::from(LARZUK) {
        return;
    }
    match args.b {
        20077 => {
            x(ctl, i).started = true;
            set_state(ctl, i, 2);
            flag_iterate_all(ctl, w, i);
            if let Some(n) = args.target {
                refresh(ctl, w, p, n);
            }
        }
        20090 => {
            if !flags(w, p).get(SLOT, bit::REWARD_PENDING) {
                return;
            }
            x(ctl, i).reward_talk = true;
            reset_progress(w, p, SLOT);
            set(w, p, SLOT, TOLD_LARZUK);
            if flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) && ctl.records[i].state != 5 {
                set_state(ctl, i, 5);
                super::sequence(ctl, w, CHAIN);
                status_to_all(ctl, w, i, 13);
            }
            add_guid(ctl, w, i, p);
        }
        _ => {}
    }
}

/// Event 2 `0x00586D80` (§3.4; never cleared).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.target.and_then(|n| w.monster_class(n)) != Some(LARZUK) {
        return;
    }
    if x(ctl, i).started {
        status_to_all(ctl, w, i, 1);
        x(ctl, i).started = false;
    }
    if x(ctl, i).reward_talk {
        status_to_all(ctl, w, i, 4);
        x(ctl, i).reward_talk = false;
    }
}

/// Event 3 `0x005873E0` (§3.5; a = old level, b = new level). The
/// 110–112 test reads no player; only the old-level-109 branch needs one
/// (L35 read, settles REC-3100's order).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let r = &ctl.records[i];
    if (110..=112).contains(&args.b) && r.not_intro && matches!(r.state, 1 | 2) {
        set_state(ctl, i, 3);
        flag_iterate_all(ctl, w, i);
    } else if args.a == 109 {
        let Some(p) = args.player else { return };
        quick_remove(ctl, w, i, p);
        let f = flags(w, p);
        // The status and iterate sends sit inside the state 2 test (the
        // same shape as `0x00588200`, `0x00589D80`; `0x005873E0` is not in
        // the export, the shape is measured: the `a5-wp-34-lv115` packets
        // check MATCHes 540/540, 1.14d sends no chain-31 0x5D on leaving
        // 109; settles REC-3100).
        if ctl.records[i].state == 2
            && !f.get(SLOT, bit::REWARD_GRANTED)
            && !f.get(SLOT, bit::REWARD_PENDING)
        {
            set_state(ctl, i, 3);
            if ctl.records[i].status == 0 {
                status_to_all(ctl, w, i, 1);
            }
            flag_iterate_all(ctl, w, i);
        }
    }
}

/// Event 8 `0x00587330` (§3.6): reached only through the chain-31 link
/// of the Siege Boss; the victim is not tested (edge case 1).
fn shenk_killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !ctl.records[i].not_intro {
        return;
    }
    let Some(victim) = args.target else { return };
    let Some((_, _, room)) = w.unit_position(victim) else {
        return;
    };
    x(ctl, i).kill_room = Some(room);
    // `0x00587100`: players in the kill room or one adjacent to it.
    let near = w.players_near(victim);
    for p in w.players() {
        if !near.contains(&p) || flags(w, p).get(SLOT, bit::REWARD_GRANTED) {
            continue;
        }
        set(w, p, SLOT, bit::REWARD_PENDING);
        set(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
    }
    // `0x00587240` → `0x005871C0` for each player.
    for p in w.players() {
        if !flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            continue;
        }
        for m in party_of(w, p) {
            let f = flags(w, m);
            if !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, bit::REWARD_PENDING)
                && !f.get(SLOT, bit::PRIMARY_GOAL_DONE)
                && in_act(w, m, 4)
            {
                set(w, m, SLOT, bit::REWARD_PENDING);
                set(w, m, SLOT, bit::PRIMARY_GOAL_DONE);
            }
        }
    }
    // `0x00587290`.
    completion_flag(
        w,
        CHAIN,
        SLOT,
        &[
            bit::REWARD_GRANTED,
            bit::REWARD_PENDING,
            bit::PRIMARY_GOAL_DONE,
        ],
    );
    // `0x005872F0`.
    for p in w.players() {
        if flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            w.attach_sound(p, SOUND_SHENK);
        }
    }
    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
    ctl.unique_event(w, FX_SHENK);
    if ctl.records[i].status < 3 {
        status_to_all(ctl, w, i, 3);
    }
}

/// Event 13 `0x005875F0` (§3.7).
fn game_start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = flags(w, p);
    let r = &mut ctl.records[i];
    if f.get(SLOT, bit::REWARD_GRANTED)
        || f.get(SLOT, bit::COMPLETED_BEFORE)
        || f.get(SLOT, bit::REWARD_PENDING)
    {
        r.status = 5;
    } else if f.get(SLOT, LEFT_AFTER_SHENK) {
        (r.status, r.state) = (2, 3);
    } else if f.get(SLOT, LEFT_TOWN) {
        (r.status, r.state) = (1, 3);
    } else if f.get(SLOT, bit::STARTED) {
        (r.status, r.state) = (1, 2);
    } else {
        let d = usize::from(w.difficulty());
        let heard = w.quests(p).is_some_and(|q| q.heard(d, MALAH));
        let r = &mut ctl.records[i];
        if heard && r.state == 0 && r.not_intro {
            r.state = 1;
        }
    }
}

/// Status `0x00586CE0` (§3.10; always true).
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
    Some(
        if pf.get(SLOT, bit::REWARD_PENDING) || pf.get(SLOT, bit::PRIMARY_GOAL_DONE) {
            if pf.get(SLOT, TOLD_LARZUK) {
                4
            } else {
                3
            }
        } else if !r.not_intro || pf.get(SLOT, bit::REWARD_GRANTED) {
            0
        } else if pf.get(SLOT, bit::COMPLETED_NOW) {
            12
        } else if r.state >= 5 {
            0
        } else {
            r.status
        },
    )
}

/// Runs a timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    let _ = (ctl, w, chain);
    match t {}
}

/// Larzuk's map AI applied once (§3.8): a stored record (+0x00 ≠ 0)
/// whose +4 ≠ 0 (`apply_map_ai`), then +0x17 := 1.
fn apply_map_ai<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, larzuk: UnitId) {
    let e = x(ctl, i);
    if e.map_ai != 0 && !e.map_ai_applied && w.apply_map_ai(larzuk, e.map_ai) {
        x(ctl, i).map_ai_applied = true;
    }
}

/// Init 71 `0x00587840` (object 543, Larzuk's start dummy, §3.8).
pub fn larzuk_dummy_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if x(ctl, i).larzuk_spawned {
        return;
    }
    let Some((ox, oy, room)) = w.unit_position(object) else {
        return;
    };
    let Some((sx, sy, sroom)) = w.free_spot_at(room, ox, oy, 2, 0x100, 16, 100) else {
        return;
    };
    let Some(l) = w.spawn_monster(sroom, sx, sy, LARZUK, 1, 5) else {
        return;
    };
    let g = w.guid(l);
    let e = x(ctl, i);
    e.larzuk_spawned = true;
    e.larzuk_guid = g;
    w.or_unit_flags(l, 0x0300_0000);
    apply_map_ai(ctl, w, i, l);
}

/// `0x00587950` (the map-AI store, from `0x00545CB3`): keep Larzuk's map
/// AI at +0x00 and apply it the same way when Larzuk exists (§3.8).
pub fn larzuk_map_ai<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, map_ai: u32) {
    let Some(i) = ctl.find(CHAIN) else { return };
    x(ctl, i).map_ai = map_ai;
    if !x(ctl, i).larzuk_spawned {
        return;
    }
    let g = x(ctl, i).larzuk_guid;
    if let Some((l, _)) = w.monster_by_guid(g) {
        apply_map_ai(ctl, w, i, l);
    }
}

/// `0x00587900` (Shenk activated, from the AI at `0x005E27D0`, §3.8).
pub fn shenk_activated<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, unit: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let r = &ctl.records[i];
    let boss = matches!(
        w.unit_kind(unit),
        UnitKind::Monster {
            superunique: Some(SIEGE_BOSS),
            ..
        }
    );
    if r.not_intro && r.status < 2 && boss {
        status_to_all(ctl, w, i, 2);
    }
}

/// `0x005877C0` (§3.9, from `world/npc.md` §8.1): after Larzuk sockets
/// an item for a player with 35.1. Nothing is sent here.
pub fn socket_reward<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) {
    let Some(f) = flags_of(w, player) else { return };
    f.set(SLOT, bit::REWARD_GRANTED);
    f.clear(SLOT, bit::REWARD_PENDING);
    if !f.get(SLOT, bit::COMPLETED_BEFORE) && ctl.find(CHAIN).is_none() {
        ctl.faults.push(QuestError::Fatal(0x0058_77C0));
    }
}

#[cfg(test)]
#[path = "q1_tests.rs"]
mod tests;
