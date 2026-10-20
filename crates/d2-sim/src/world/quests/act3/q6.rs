// Spec: specs/world/quests-act3.md §8 (A3Q6 The Guardian, chain 20); specs/world/quests-act3-2.md §11.2, §11.3, §11.6
//! A3Q6: events 0, 2, 3, 8, 10, 11, 13, the active function, the status
//! timer, the Hellgate (init 44), Mephisto's bridge (init 45, event 7),
//! Natalya (init 52) and the Durance warp `0x005BCFD0`.

use super::{
    add_guid, add_state, clear, completion_flag, guid_listed, in_act3, install, list_remove, npc,
    npc_of, pf, quick_remove, refresh, s_ab, set, sound, status_all, status_silent, table_state,
    Timer, DOCKS,
};
use crate::units::UnitId;
use crate::world::quests::{
    bit, event, raise_progression, EventArgs, QuestControl, QuestWorld, TextList, TimerFn,
};

const CHAIN: u8 = 20;
const SLOT: u8 = 22;
/// 22.11: Mephisto killed, NPC talk pending.
const PENDING: u8 = 11;
/// `0x00741518`: message table state by record state 0–5.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];
/// Ruined Fane, Durance of Hate 1 and 3, Outer Steppes.
const RUINED_FANE: u32 = 98;
const DURANCE_1: u32 = 100;
const DURANCE_3: u32 = 102;
const OUTER_STEPPES: u32 = 104;
/// The progression step of Mephisto's credit (`0x005BC17B`,
/// `quests-act3-2.md` §11.2).
const PROGRESSION_STEP: u8 = 3;
/// `0x0058F000`: applying Natalya's stored map AI (NPC map AI spec);
/// never reached from Act III code in 1.14d (`quests-act3-2.md` §11.6).
const APPLY_MAP_AI: u32 = 0x0058_F000;
const SOULSTONE: [u8; 4] = *b"mss ";
/// Mephisto's status timer period (updater ticks).
const STATUS_PERIOD: u32 = 12;

/// Chain 20's extra data (§8.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the status timer exists.
    pub timer: bool,
    /// +0x01: the Hellgate is initialised; +0x04 its GUID.
    pub gate_known: bool,
    pub gate_guid: u32,
    /// +0x02: the bridge is initialised; +0x08 its GUID.
    pub bridge_known: bool,
    pub bridge_guid: u32,
    /// +0x03: Ormus started the quest (chat end pending).
    pub ormus_started: bool,
    /// +0x0C: the Hellgate's mode.
    pub gate_mode: i32,
    /// +0x10: the bridge's mode.
    pub bridge_mode: i32,
    /// +0x14: soulstones dropped.
    pub stones_dropped: i32,
    /// +0x18: a soulstone was dropped.
    pub stone_dropped: bool,
    /// +0x1C: soulstones to drop.
    pub stones_to_drop: i32,
    /// +0x20: Natalya was spawned; +0x2C her GUID.
    pub natalya_spawned: bool,
    pub natalya_guid: u32,
    /// +0x24: her map AI is stored. Only `0x005BD040` writes it and it
    /// has no caller in 1.14d (`quests-act3-2.md` §11.6): stays false.
    pub map_ai: bool,
    /// +0x30: the map AI was applied.
    pub ai_applied: bool,
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.act3.q6
}

/// Dispatches the chain's callbacks; false = no body (unhandled).
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
            // Chat end `0x005BC370`.
            if npc_of(w, &args) == Some(npc::ORMUS) && x(ctl, i).ormus_started {
                status_all(ctl, w, i, 2);
                x(ctl, i).ormus_started = false;
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            }
        }
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => kill(ctl, w, i, args),
        // `0x005BC710`.
        event::PLAYER_LEAVES_GAME => list_remove(ctl, w, i, &args),
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => {
            if let Some(p) = args.player {
                game_start(ctl, w, i, p);
            }
        }
        _ => return false,
    }
    true
}

/// Flag iterate `0x005BC020` (§8.3) for every player.
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let (state, status) = (ctl.records[i].state, ctl.records[i].status);
    let b = match (state, status) {
        (2, _) => 2,
        (3, _) => 3,
        (4, 2..=4) => status + 2,
        (5, 2..=4) => status + 5,
        _ => return,
    };
    for p in w.players() {
        let f = pf(w, p);
        if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, PENDING) {
            set(w, p, SLOT, &[b]);
        }
    }
}

/// Event 0 `0x005BC270` (§8.2).
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    let state = ctl.records[i].state;
    let k = if f.get(SLOT, PENDING) {
        5
    } else if f.get(SLOT, bit::REWARD_GRANTED) {
        if !guid_listed(ctl, w, i, p) {
            return;
        }
        6
    } else if state > 5 {
        if !f.get(SLOT, bit::PRIMARY_GOAL_DONE) {
            return;
        }
        6
    } else {
        match table_state(&MSG_STATE, state, 7) {
            Some(k) => k,
            None => return,
        }
    };
    add_state(ctl, w, i, list, args.target, k);
}

/// Active `0x005BBFE0` (§8.2).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let f = pf(w, player);
    if f.get(SLOT, PENDING) {
        return true;
    }
    npc_class == npc::ORMUS && !f.get(SLOT, bit::REWARD_GRANTED) && ctl.records[i].state == 1
}

/// Event 11 `0x005BC5A0` (a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if !f.get(SLOT, PENDING) && f.get(SLOT, bit::REWARD_GRANTED) {
        return;
    }
    match args.b {
        628 if args.a == u32::from(npc::ORMUS) => {
            ctl.records[i].state = s_ab(ctl, 2, 3);
            x(ctl, i).ormus_started = true;
            refresh(ctl, w, p, &args);
        }
        657..=663 => {
            if f.get(SLOT, PENDING) {
                if f.get(SLOT, bit::PRIMARY_GOAL_DONE) {
                    status_silent(ctl, i, 13);
                    ctl.records[i].state = 7;
                }
                clear(w, p, SLOT, &[PENDING]);
                add_guid(ctl, w, i, p);
            }
            refresh(ctl, w, p, &args);
        }
        _ => {}
    }
}

/// Event 3 `0x005BC3C0` (a = old level, b = new level), steps 1–4.
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    // 1.
    if args.a == DOCKS {
        if let Some(p) = args.player {
            quick_remove(ctl, w, i, p);
            let f = pf(w, p);
            if matches!(ctl.records[i].state, 2 | 3)
                && !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, PENDING)
            {
                if ctl.records[i].status < 3 {
                    status_silent(ctl, i, 2);
                }
                ctl.records[i].state = s_ab(ctl, 4, 5);
                flag_iterate(ctl, w, i);
            }
        }
    }
    // 2.
    if args.b >= RUINED_FANE && ctl.records[i].not_intro && ctl.records[i].state == 0 {
        ctl.records[i].state = 1;
        install(ctl, i, event::NPC_DEACTIVATE);
    }
    // 3.
    if args.b == DURANCE_1 {
        let sent = matches!(ctl.records[i].status, 0 | 2);
        if sent {
            status_all(ctl, w, i, 3);
        }
        if ctl.records[i].state == 1 {
            ctl.records[i].state = s_ab(ctl, 4, 5);
            flag_iterate(ctl, w, i);
        } else if sent {
            flag_iterate(ctl, w, i);
        }
    }
    // 4.
    if args.b == DURANCE_3 {
        if ctl.records[i].status != 4 {
            status_all(ctl, w, i, 4);
        }
        if !matches!(ctl.records[i].state, 4 | 5) {
            ctl.records[i].state = s_ab(ctl, 4, 5);
        }
        flag_iterate(ctl, w, i);
    }
}

/// Credit `0x005BC140`: 22.13, 22.0, 22.11, then the character
/// progression `0x00538680(P's client, 3, game difficulty)` at
/// `0x005BC182` (`quests-act3-2.md` §11.2, `quests-act1-rest.md` §5):
/// n = (4 or 5)·difficulty + 3, never lowered.
fn credit<W: QuestWorld>(w: &mut W, p: UnitId) {
    set(
        w,
        p,
        SLOT,
        &[bit::PRIMARY_GOAL_DONE, bit::REWARD_GRANTED, PENDING],
    );
    let d = w.difficulty();
    raise_progression(w, p, PROGRESSION_STEP, d);
}

/// Event 8 `0x005BC8B0` (§8.5): Mephisto's death (victim = target,
/// killer = player).
fn kill<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    ctl.records[i].state = 6;
    install(ctl, i, event::PLAYER_LEAVES_GAME);
    if ctl.records[i].not_intro {
        // 1. The killer: stub `0x00545990` (`ret 4`), then the credit.
        if let Some(k) = args.player {
            let f = pf(w, k);
            if !f.get(SLOT, bit::REWARD_GRANTED) {
                if !f.get(SLOT, PENDING) {
                    x(ctl, i).stones_to_drop += 1;
                }
                credit(w, k);
            }
        }
        // `0x005BC190`: players in Durance of Hate 3.
        for p in w.players() {
            let f = pf(w, p);
            if w.unit_level(p) == Some(DURANCE_3)
                && !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, PENDING)
            {
                credit(w, p);
                x(ctl, i).stones_to_drop += 1;
            }
        }
        // `0x005BC7C0` / `0x005BC750`: parties of the credited players.
        for p in w.players() {
            if !pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                continue;
            }
            for m in w.party_members(p).unwrap_or_default() {
                if in_act3(w, m) && !pf(w, m).get(SLOT, bit::REWARD_GRANTED) {
                    credit(w, m);
                    x(ctl, i).stones_to_drop += 1;
                }
            }
        }
        // `0x005BC810`.
        for p in w.players() {
            completion_flag(w, p, SLOT, CHAIN, &[bit::REWARD_GRANTED, PENDING]);
        }
        // `0x005BC870`.
        for p in w.players() {
            if pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                w.attach_sound(p, sound::MEPHISTO);
            }
        }
        if !x(ctl, i).timer {
            let t = TimerFn::Act3(Timer::MephistoStatus);
            if let Err(e) = ctl.add_timer(CHAIN, t, STATUS_PERIOD) {
                ctl.faults.push(e);
            }
            x(ctl, i).timer = true;
        }
    }
    // 2. Always.
    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
    if x(ctl, i).gate_known {
        let g = x(ctl, i).gate_guid;
        if let Some((gate, _)) = w.object_by_guid(g) {
            w.set_object_mode(gate, 1);
            let at = w.frame() + (w.object_anim_length(gate) >> 8);
            w.schedule_object_event(gate, 1, at);
        }
    }
    x(ctl, i).gate_mode = 2;
    if let Some(victim) = args.target {
        for _ in 0..x(ctl, i).stones_to_drop {
            if w.quest_drop(victim, SOULSTONE, 2, None, false).is_some() {
                let e = x(ctl, i);
                e.stones_dropped += 1;
                e.stone_dropped = true;
            }
        }
    }
    // Stub `0x005B6930` (no body).
    ctl.unique_event(w, 11);
}

/// Timer `0x005BC720`; returns 1.
pub(super) fn status_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if ctl.records[i].status != 4 {
        status_all(ctl, w, i, 4);
    }
    x(ctl, i).timer = false;
    true
}

/// Event 13 `0x005BCC80` (§8.8).
fn game_start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, PENDING) || f.get(23, bit::REWARD_GRANTED) {
        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        x(ctl, i).gate_mode = 2;
        return;
    }
    // The first set bit, 22.9 down to 22.2 (edge case 10: 22.3 → state
    // 2, 22.2 → state 3).
    const MAP: [(u8, u8, u8); 8] = [
        (9, 5, 4),
        (8, 5, 3),
        (7, 5, 2),
        (6, 4, 4),
        (5, 4, 3),
        (4, 4, 2),
        (3, 2, 2),
        (2, 3, 2),
    ];
    if let Some(&(_, state, status)) = MAP.iter().find(|m| f.get(SLOT, m.0)) {
        let r = &mut ctl.records[i];
        (r.state, r.status) = (state, status);
    }
}

/// Hellgate init 44 `0x005BCBF0` (object 342).
pub fn hellgate_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    if w.unit_level(object) == Some(OUTER_STEPPES) {
        w.set_object_mode(object, 2);
        return;
    }
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    let e = x(ctl, i);
    e.gate_known = true;
    e.gate_guid = g;
    let mode = e.gate_mode;
    w.set_object_mode(object, mode);
}

/// Bridge init 45 `0x005BCB90` (object 341).
pub fn bridge_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    let e = x(ctl, i);
    e.bridge_known = true;
    e.bridge_guid = g;
    let mode = e.bridge_mode;
    w.set_object_mode(object, mode);
    if mode != 2 {
        let at = w.frame() + w.tick_rate().ticks(20);
        w.schedule_object_event(object, 7, at);
    }
}

/// Bridge object event 7 `0x005BCAC0` (class 341).
pub fn bridge_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let mode = w.object_mode(object);
    if mode == 2 {
        return;
    }
    if x(ctl, i).gate_mode == 2 && w.player_near_object(object, 18).is_some() {
        match mode {
            0 => {
                x(ctl, i).bridge_mode = 2;
                w.set_object_mode(object, 1);
            }
            1 => {
                x(ctl, i).bridge_mode = 2;
                w.set_object_mode(object, 2);
                w.free_object_collision(object);
            }
            _ => {}
        }
    }
    let at = w.frame() + w.tick_rate().ticks(24);
    w.schedule_object_event(object, 7, at);
}

/// Natalya init 52 `0x005BCE80` (object 382).
pub fn natalya_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    if ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        return;
    }
    let Some(i) = ctl.find(CHAIN) else { return };
    let e = x(ctl, i);
    if e.natalya_spawned && w.monster_by_guid(e.natalya_guid).map(|m| m.1) == Some(npc::NATALYA) {
        return;
    }
    let spawned = w.unit_position(object).and_then(|(px, py, room)| {
        w.spawn_monster(room, px, py, npc::NATALYA, 1, u32::MAX)
            .or_else(|| w.spawn_monster(room, px, py, npc::NATALYA, 1, 3))
    });
    let Some(m) = spawned else {
        x(ctl, i).natalya_spawned = false;
        return;
    };
    let g = w.guid(m);
    let e = x(ctl, i);
    e.natalya_spawned = true;
    e.natalya_guid = g;
    // +0x24 is stored only by `0x005BD040`, which has no caller in 1.14d
    // (edge cases 17, 20; `quests-act3-2.md` §11.6): this branch never
    // runs there.
    if e.map_ai && !e.ai_applied {
        e.ai_applied = true;
        w.unhandled(CHAIN, APPLY_MAP_AI);
    }
}

/// The Durance warp's Act III part `0x005BCFD0` (`quests.md` §8.1):
/// opens the Hellgate and the bridge.
pub fn durance_warp<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let e = x(ctl, i);
    e.gate_mode = 2;
    e.bridge_mode = 2;
    let (gate, bridge) = (
        e.gate_known.then_some(e.gate_guid),
        e.bridge_known.then_some(e.bridge_guid),
    );
    if let Some((o, _)) = gate.and_then(|g| w.object_by_guid(g)) {
        w.set_object_mode(o, 1);
    }
    if let Some((o, _)) = bridge.and_then(|g| w.object_by_guid(g)) {
        w.set_object_mode(o, 2);
    }
}
