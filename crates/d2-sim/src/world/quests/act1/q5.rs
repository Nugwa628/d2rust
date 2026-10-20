// Spec: specs/world/quests-act1.md §10.7 (A1Q5 The Forgotten Tower, chain 5)
// Spec: specs/world/quests-act1-rest.md §4 (the chest trap step)
// Spec: specs/world/quests.md (the sections other than §10)
//! A1Q5 callback by callback: events 0, 3, 8 (the Countess), 10, 11, 13,
//! the timer `0x005954C0`, the active function, the tome operate, the
//! chest init and event 7, the trap step `0x005954F0` and the object
//! init `0x00595A00`, with the iterate functions M1–M6. Slot 5 is a
//! constant in each.

use super::{add_state, player_flags, rec, send_completed_now, sequence, table_state};
use crate::units::UnitId;
use crate::world::quests::{
    bit, event, flags_of, npc, EventArgs, GuidList, QuestControl, QuestError, QuestWorld, TextList,
    TimerFn,
};

const SLOT: u8 = 5;
const CHAIN: u8 = 5;
/// The Forgotten Tower, Tower Cellar 5, the Rogue Encampment.
const TOWER: u32 = 20;
const CELLAR_5: u32 = 25;
const TOWN: u32 = 1;
/// The tome's message.
const TOME: u32 = 127;
/// `0x007382AC`: message state by quest state 0–5.
const MSG_STATE: [i8; 6] = [-1, -1, 0, 1, 2, 3];
/// The trap monster (trap-firebolt) and the missile each chest gets
/// (towerchestspawner).
const TRAP_MONSTER: u16 = 326;
const CHEST_MISSILE: u16 = 332;
/// Town NPCs whose messages 140–145 report the kill.
const REPORT_NPCS: [u16; 6] = [
    npc::CHARSI,
    npc::KASHYA,
    npc::CAIN5,
    npc::WARRIV1,
    npc::AKARA,
    147,
];

/// A1Q5's extra data (§10.7).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra5 {
    /// +0x00 list A (12): players who reported the kill in town.
    pub reported: Vec<u32>,
    /// +0x34 list B (12): players credited in Tower Cellar 5.
    pub credited: Vec<u32>,
    /// +0x68: chest object GUIDs for the trap step (8).
    pub chests: Vec<u32>,
    /// +0x8C list C: players in Tower Cellar 5 at the kill.
    pub in_cellar: GuidList,
    /// +0x110, +0x114: the Countess's death position.
    pub death_pos: (i32, i32),
    /// +0x118, +0x119: killed; trap spawned.
    pub killed: bool,
    pub trap_spawned: bool,
    /// +0x11A: the next town report finishes the quest.
    pub report_due: bool,
    /// +0x11B: the tome was read before any status.
    pub tome_early: bool,
    /// +0x11C (cleared by the timer).
    pub b11c: bool,
}

const LIST_CAP: usize = 12;

/// `0x00594740`: remove by swapping the last entry into the hole.
fn swap_remove(list: &mut Vec<u32>, g: u32) -> bool {
    match list.iter().position(|&x| x == g) {
        Some(k) => {
            list.swap_remove(k);
            true
        }
        None => false,
    }
}

fn x5(ctl: &mut QuestControl, i: usize) -> &mut Extra5 {
    &mut ctl.records[i].extra.q5
}

/// Dispatches chain 5's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => npc_text(ctl, w, i, args, list),
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => kill(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            // `0x00594BB0`.
            let g = args.player.map_or(u32::MAX, |p| w.guid(p));
            let x = x5(ctl, i);
            x.in_cellar.remove(g);
            swap_remove(&mut x.credited, g);
            swap_remove(&mut x.reported, g);
        }
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => {
            // `0x00595860`.
            let r = rec(w, args.player);
            if !r.get(SLOT, bit::REWARD_GRANTED) && !r.get(SLOT, bit::COMPLETED_BEFORE) {
                let rd = &mut ctl.records[i];
                let restored = if r.get(SLOT, 4) {
                    Some((3, 1))
                } else if r.get(SLOT, 6) {
                    Some((3, 4))
                } else if r.get(SLOT, 5) {
                    Some((2, 3))
                } else if r.get(SLOT, 3) {
                    Some((3, 1))
                } else if r.get(SLOT, 2) {
                    Some((2, 1))
                } else {
                    None
                };
                if let Some(s) = restored {
                    (rd.state, rd.status) = s;
                }
            }
        }
        _ => return false,
    }
    true
}

/// broadcast(S, f) with M1 (§10.7: no 5.15 test).
fn broadcast<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, status: u8, flags: u8) {
    ctl.records[i].flags = flags;
    ctl.records[i].status = status;
    for p in w.players() {
        let f = player_flags(w, p);
        if f.get(SLOT, bit::REWARD_GRANTED)
            && !f.get(SLOT, bit::PRIMARY_GOAL_DONE)
            && !f.get(SLOT, bit::COMPLETED_NOW)
        {
            continue;
        }
        if let Err(e) = ctl.send_status(w, p, CHAIN) {
            ctl.faults.push(e);
        }
    }
}

/// M2 `0x00594890` for every player.
fn iterate_progress<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let (state, status) = (ctl.records[i].state, ctl.records[i].status);
    for p in w.players() {
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match (state, status) {
            (2, _) => f.set(SLOT, 2),
            (3, 1..=4) => f.set(SLOT, status + 2),
            _ => {}
        }
    }
}

/// Event 0 `0x00594C50`.
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let r = rec(w, args.player);
    let rd = &ctl.records[i];
    if !rd.not_intro || (r.get(SLOT, bit::REWARD_GRANTED) && !r.get(SLOT, bit::PRIMARY_GOAL_DONE)) {
        return;
    }
    let g = args.player.map_or(u32::MAX, |p| w.guid(p));
    let (in_a, in_b) = (
        rd.extra.q5.reported.contains(&g),
        rd.extra.q5.credited.contains(&g),
    );
    if rd.state >= 4 && !in_a && !in_b {
        return;
    }
    if in_b {
        add_state(ctl, w, i, list, args.target, 2);
    } else if r.get(SLOT, bit::REWARD_GRANTED) && r.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        if in_a {
            add_state(ctl, w, i, list, args.target, 3);
        }
    } else if let Some(m) = table_state(&MSG_STATE, rd.state) {
        add_state(ctl, w, i, list, args.target, m);
    }
}

/// Event 3 `0x00595010` (a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let rd = &ctl.records[i];
    if rd.not_intro && args.b == TOWER {
        if rd.state == 0 {
            ctl.records[i].state = 2;
            broadcast(ctl, w, i, 3, 0);
            iterate_progress(ctl, w, i);
        } else if rd.state <= 3 && rd.status == 1 {
            broadcast(ctl, w, i, 4, 0);
            iterate_progress(ctl, w, i);
        }
        return;
    }
    if rd.not_intro && args.b == CELLAR_5 {
        if rd.state < 4 && rd.status != 2 {
            ctl.records[i].state = 3;
            broadcast(ctl, w, i, 2, 0);
            iterate_progress(ctl, w, i);
        }
        return;
    }
    if args.a == TOWN {
        let r = rec(w, args.player);
        if rd.state == 2 && !r.get(SLOT, bit::REWARD_GRANTED) {
            ctl.records[i].state = 3;
        } else if rd.state == 5 && rd.not_intro {
            let g = args.player.map_or(u32::MAX, |p| w.guid(p));
            let x = x5(ctl, i);
            if swap_remove(&mut x.reported, g) && x.reported.is_empty() && x.credited.is_empty() {
                ctl.records[i].active = false;
            }
        }
    }
}

/// Event 8 `0x00595710`: the Countess's death (victim = target).
fn kill<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if let Some((x, y, _)) = args.target.and_then(|v| w.unit_position(v)) {
        x5(ctl, i).death_pos = (x, y);
    }
    if ctl.records[i].not_intro {
        ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
        ctl.records[i].state = 5;
        x5(ctl, i).report_due = true;
        for p in w.players() {
            credit_in_cellar(ctl, w, i, p);
        }
        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        if x5(ctl, i).credited.is_empty() {
            ctl.records[i].active = false;
        } else {
            ctl.records[i].callbacks |= 1 << event::PLAYER_LEAVES_GAME;
        }
        x5(ctl, i).killed = true;
        ctl.records[i].clear_callback(event::MONSTER_KILLED);
        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        for p in w.players() {
            // M3 `0x00594DD0`.
            let lvl = w.unit_level(p);
            let Some(f) = flags_of(w, p) else { continue };
            if !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, bit::REWARD_PENDING)
                && lvl == Some(CELLAR_5)
            {
                f.set(SLOT, bit::REWARD_GRANTED);
                f.set(SLOT, bit::PRIMARY_GOAL_DONE);
                let g = w.guid(p);
                x5(ctl, i).in_cellar.add(g);
            }
        }
        // `0x00595420(game, list C, 5, 37)`.
        for g in x5(ctl, i).in_cellar.0.clone() {
            let Some(p) = w.player_by_guid(g) else {
                continue;
            };
            let Some(f) = flags_of(w, p) else { continue };
            if !f.get(SLOT, bit::REWARD_GRANTED) {
                f.set(SLOT, bit::PRIMARY_GOAL_DONE);
                f.set(SLOT, bit::REWARD_GRANTED);
                w.attach_sound(p, 37);
            }
        }
        for p in w.players() {
            // M5 `0x005953D0` / `0x00595370`.
            if !player_flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                continue;
            }
            for m in w.party_members(p).unwrap_or_default() {
                let in_act1 = w.unit_level(m).is_some_and(|l| l != 0) && w.unit_act(m) == Some(0);
                let Some(f) = flags_of(w, m) else { continue };
                if !f.get(SLOT, bit::REWARD_GRANTED) && in_act1 {
                    f.set(SLOT, bit::PRIMARY_GOAL_DONE);
                    f.set(SLOT, bit::REWARD_GRANTED);
                }
            }
        }
        for p in w.players() {
            // M6 `0x00595320`.
            let Some(f) = flags_of(w, p) else { continue };
            if !f.get(SLOT, bit::REWARD_GRANTED) {
                f.set(SLOT, bit::COMPLETED_NOW);
                send_completed_now(w, p, CHAIN, 0);
            }
        }
        if let Err(e) = ctl.add_timer(CHAIN, TimerFn::TowerStatus, 7) {
            ctl.faults.push(e);
        }
    }
    if let Some(v) = args.target {
        trap_step(ctl, w, i, v);
    }
}

/// M4 `0x00594F10` for one player.
fn credit_in_cellar<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    let Some(lvl) = w.unit_level(p) else { return };
    let g = w.guid(p);
    let Some(f) = flags_of(w, p) else { return };
    if f.get(SLOT, bit::REWARD_GRANTED) {
        return;
    }
    if lvl == CELLAR_5 {
        f.set(SLOT, bit::PRIMARY_GOAL_DONE);
        f.set(SLOT, bit::REWARD_GRANTED);
        f.clear(SLOT, bit::REWARD_PENDING);
        w.attach_sound(p, 37);
        let x = x5(ctl, i);
        if x.credited.len() < LIST_CAP {
            x.credited.push(g);
        }
    } else {
        f.set(SLOT, bit::COMPLETED_NOW);
    }
}

/// The trap step `0x005954F0(record, extra)` (`quests-act1-rest.md` §4):
/// one trap-firebolt per game, then a towerchestspawner missile per
/// listed chest, owned by it.
fn trap<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    {
        let x = x5(ctl, i);
        if !x.killed || x.trap_spawned || x.chests.is_empty() {
            return;
        }
    }
    let mut trap: Option<UnitId> = None;
    let mut k = 0;
    // The count is re-read every iteration.
    while k < x5(ctl, i).chests.len() {
        let g = x5(ctl, i).chests[k];
        k += 1;
        let Some((c, _)) = w.object_by_guid(g) else {
            continue;
        };
        // The chest's position (static path); its room (static path
        // +0x00) is null for a chest left in a freed room: both room
        // lookups are then null and both spawns fail, while an existing
        // T still gets its missile at the chest (`quests-act1-rest.md`
        // §9 item 2).
        let Some((cx, cy)) = w.unit_xy(c) else {
            // An object found by GUID always has a static path.
            ctl.faults.push(QuestError::Fatal(0x0059_54F0));
            continue;
        };
        let croom = w.unit_position(c).map(|p| p.2);
        if trap.is_none() {
            let (x, y) = x5(ctl, i).death_pos;
            trap = croom
                .and_then(|r| w.room_at(r, x, y))
                .and_then(|room| w.spawn_monster_flags(room, x, y, TRAP_MONSTER, 12, -1, 8));
            if trap.is_none() {
                // The retry's room is not tested by the original; no
                // room here means no spawn (§9 item 1).
                let (x, y) = (cx + 5, cy + 5);
                trap = croom
                    .and_then(|r| w.room_at(r, x, y))
                    .and_then(|room| w.spawn_monster_flags(room, x, y, TRAP_MONSTER, 12, -1, 8));
            }
        }
        let Some(t) = trap else { continue };
        x5(ctl, i).trap_spawned = true;
        if let Some(m) = w.create_missile(t, 0, 1, CHEST_MISSILE, cx, cy) {
            w.set_missile_target(m, g, 0);
            w.refresh_room(m);
        }
    }
}

/// The trap step, then event 7 on `unit` at frame + 10 while no trap
/// was spawned.
fn trap_step<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, unit: UnitId) {
    trap(ctl, w, i);
    if !x5(ctl, i).trap_spawned {
        let at = w.frame() + w.tick_rate().ticks(10);
        w.schedule_quest_event(unit, at);
    }
}

/// Timer `0x005954C0`: status 13 while state 5; runs once.
pub(super) fn timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    if ctl.records[i].state == 5 {
        broadcast(ctl, w, i, 13, 0);
    }
    x5(ctl, i).b11c = false;
}

/// Event 11 `0x00594960` (a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.b == TOME && ctl.records[i].not_intro {
        let mut changed = false;
        if x5(ctl, i).tome_early {
            let status = ctl.records[i].status;
            if status < 1 {
                broadcast(ctl, w, i, 1, 0);
                changed = true;
            } else if status == 3 {
                broadcast(ctl, w, i, 2, 0);
                changed = true;
                if ctl.records[i].state < 3 {
                    ctl.records[i].state = 3;
                }
            }
        }
        if ctl.records[i].state < 2 {
            ctl.records[i].state = 2;
            iterate_progress(ctl, w, i);
        } else if changed {
            iterate_progress(ctl, w, i);
        }
        return;
    }
    let report = REPORT_NPCS.iter().any(|&c| u32::from(c) == args.a);
    if !report || !(140..=145).contains(&args.b) {
        return;
    }
    let Some(p) = args.player else { return };
    if player_flags(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) && x5(ctl, i).report_due {
        x5(ctl, i).report_due = false;
        ctl.records[i].state = 5;
        sequence(ctl, w, CHAIN);
    }
    let g = w.guid(p);
    let x = x5(ctl, i);
    if swap_remove(&mut x.credited, g) && x.reported.len() < LIST_CAP {
        x.reported.push(g);
    }
}

/// Active `0x005952C0` (§6.4).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let g = w.guid(player);
    ctl.records[i].extra.q5.credited.contains(&g) && npc_class != npc::WARRIV1 && npc_class != 147
}

/// Tome operate `0x00594E70` (operate pointer `0x00732D30`).
pub fn tome_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    if w.object_mode(object) == 0 {
        w.set_object_mode(object, 1);
        let at = w.frame() + (w.object_anim_length(object) >> 8);
        w.schedule_object_event(object, 1, at);
    }
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro {
        return;
    }
    w.open_quest_message(player, object, TOME as u16);
    let r = &mut ctl.records[i];
    if r.state <= 1 {
        r.state = 2;
        if r.status < 1 {
            r.extra.q5.tome_early = true;
        }
    }
}

/// Chest init `0x00595A50` (object init pointer `0x00731C7C`).
pub fn chest_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    let x = x5(ctl, i);
    if !x.chests.contains(&g) && x.chests.len() < 8 {
        x.chests.push(g);
    }
    chest_event(ctl, w, object);
}

/// Chest event 7 `0x005956C0` (§9.5 class 0x173): the trap step, then
/// event 7 again at frame + 10 once the Countess died, while no trap was
/// spawned.
pub fn chest_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    trap(ctl, w, i);
    let x = x5(ctl, i);
    if x.killed && !x.trap_spawned {
        let at = w.frame() + w.tick_rate().ticks(10);
        w.schedule_quest_event(object, at);
    }
}

/// Object init `0x00595A00` (pointer `0x00731BD0`).
pub fn object_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    match ctl.record(CHAIN) {
        None => ctl.faults.push(QuestError::Fatal(0x0059_5A00)),
        Some(r) if !r.not_intro => w.set_object_mode(object, 3),
        Some(_) => {}
    }
}
