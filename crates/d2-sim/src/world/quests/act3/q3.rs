// Spec: specs/world/quests-act3.md §5 (A3Q3 Blade of the Old Religion, chain 17); specs/world/quests-act3-2.md §11.3, §11.4, §11.7
//! A3Q3: events 0, 2, 3, 4, 8, 9, 11, 13, 14, the status and active
//! functions, the decoy (operate 31, init 25), its timer, the boss, the
//! altar (init 39) and Ormus' map-AI hooks.

use super::{
    add_guid, add_state, guid_listed, in_act3, install, npc, npc_of, pf, quick_remove, sequence,
    set, sound, status_all, status_silent, table_state, InitPoint, Timer, DOCKS,
};
use crate::units::{RoomId, UnitId};
use crate::world::quests::{
    bit, event, EventArgs, QuestControl, QuestFlags, QuestWorld, TextList, TimerFn,
};

const SLOT: u8 = 19;
const CHAIN: u8 = 17;
/// The Gidbinn.
pub const GIDBINN: [u8; 4] = *b"g33 ";
/// Ormus' ring.
pub const RING: [u8; 4] = *b"rin ";
/// `0x007401B8`: table state by quest state 0–3 (−1 or > 7: nothing).
const MSG_STATE: [i8; 4] = [-1, 0, 1, 2];
/// Flayer Jungle.
const FLAYER_JUNGLE: u32 = 78;
/// The boss timer's period (updater ticks).
const BOSS_PERIOD: u32 = 7;
/// `0x005A0180(victim, 0x0E)`: monster data type flags (+0x16)
/// superunique (2), champion (4) or unique (8) (`quests-act3-2.md` §11.4).
pub const KILL_TYPE_FLAGS: u8 = 0x0E;

/// Chain 17's extra data (§5.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the Gidbinn was dropped.
    pub gidbinn_dropped: bool,
    /// +0x01: Hratli started the quest (chat end pending).
    pub hratli_started: bool,
    /// +0x02: the boss was spawned.
    pub boss_spawned: bool,
    /// +0x03: the decoy was activated.
    pub decoy_active: bool,
    /// +0x04: the boss is being spawned.
    pub boss_spawning: bool,
    /// +0x05: the spawn timer exists.
    pub timer: bool,
    /// +0x06: the altar may activate.
    pub altar_ready: bool,
    /// +0x07: the Gidbinn was brought (chat end pending).
    pub brought: bool,
    /// +0x08 / +0x0C: the decoy's position.
    pub decoy_x: i32,
    pub decoy_y: i32,
    /// +0x10 / +0x14: the altar's position.
    pub altar_x: i32,
    pub altar_y: i32,
    /// +0x18: the decoy is initialised.
    pub decoy_known: bool,
    /// +0x1C: Gidbinns held in the game.
    pub held: i32,
    /// +0x20: the last holder left.
    pub holder_left: bool,
    /// +0x24: the boss's GUID.
    pub boss_guid: u32,
    /// +0x28: the altar's GUID.
    pub altar_guid: u32,
    /// +0x2C: the altar's mode.
    pub altar_mode: i32,
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.act3.q3
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
        event::NPC_DEACTIVATE => chat_end(ctl, w, i, args),
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::ITEM_PICKED_UP => picked_up(ctl, w, i, args),
        event::MONSTER_KILLED => killed(ctl, w, i, args),
        event::PLAYER_DROPPED_WITH_QUEST_ITEM => {
            // `0x005B96C0`.
            if args.target.and_then(|t| w.item_code(t)) == Some(GIDBINN) {
                let (not_intro, state) = (ctl.records[i].not_intro, ctl.records[i].state);
                let e = x(ctl, i);
                e.held -= 1;
                if e.held == 0 && e.gidbinn_dropped && not_intro && state < 5 {
                    e.holder_left = true;
                }
            }
        }
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => started(ctl, w, i, args),
        event::PLAYER_JOINED_GAME => {
            // `0x005B9680`.
            let Some(p) = args.player else { return true };
            if w.has_item(p, GIDBINN) {
                let e = x(ctl, i);
                e.held += 1;
                if e.holder_left && e.held == 1 {
                    e.holder_left = false;
                }
            }
        }
        _ => return false,
    }
    true
}

/// The flag iterate `0x005B8B20` for one player (§5.2).
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize, p: UnitId) {
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, 6) {
        return;
    }
    if w.has_item(p, GIDBINN) {
        set(w, p, SLOT, &[5]);
    } else if ctl.records[i].extra.act3.q3.gidbinn_dropped {
        set(w, p, SLOT, &[4]);
        if !f.get(SLOT, bit::LEAVE_TOWN) {
            set(w, p, SLOT, &[bit::STARTED]);
        }
    } else if ctl.records[i].state == 2 {
        set(w, p, SLOT, &[bit::LEAVE_TOWN]);
    }
}

/// The flag iterate for every player.
fn flag_iterate_all<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    for p in w.players() {
        flag_iterate(ctl, w, i, p);
    }
}

/// Event 0 `0x005B8EC0` (§5.3).
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    let class = npc_of(w, &args);
    let holds = w.has_item(p, GIDBINN);
    let k = if f.get(SLOT, bit::REWARD_GRANTED) {
        guid_listed(ctl, w, i, p).then_some(4)
    } else if holds && f.get(SLOT, 5) && !f.get(SLOT, 8) {
        Some(3)
    } else if f.get(SLOT, 6) {
        let (no_merc, no_ring) = (!f.get(SLOT, 7), !f.get(SLOT, 8));
        if class == Some(npc::ASHEARA) && no_merc {
            Some(5)
        } else if class == Some(npc::ORMUS) && no_ring {
            Some(6)
        } else if class != Some(npc::ASHEARA) && no_merc {
            Some(5)
        } else if no_ring {
            Some(6)
        } else {
            None
        }
    } else if f.get(SLOT, 5) {
        Some(4)
    } else {
        let state = ctl.records[i].state;
        if state > 3 || holds {
            None
        } else {
            table_state(&MSG_STATE, state, 7)
        }
    };
    if let Some(k) = k {
        add_state(ctl, w, i, list, args.target, k);
    }
}

/// Active function `0x005B8E10` ("wants to talk", §5.3).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let f = pf(w, player);
    if f.get(SLOT, bit::REWARD_GRANTED) {
        return false;
    }
    let holds = w.has_item(player, GIDBINN);
    match npc_class {
        npc::HRATLI => !holds && ctl.records[i].state == 1,
        npc::ORMUS => holds || (f.get(SLOT, 6) && !f.get(SLOT, 8)),
        npc::ASHEARA => f.get(SLOT, 6) && !f.get(SLOT, 7),
        _ => false,
    }
}

/// Event 11 `0x005B9240` (§5.4).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) {
        return;
    }
    match (args.a, args.b) {
        (a, 571) if a == u32::from(npc::HRATLI) => {
            ctl.records[i].state = 2;
            x(ctl, i).hratli_started = true;
        }
        (a, 587) if a == u32::from(npc::ORMUS) => {
            if w.has_item(p, GIDBINN) && f.get(SLOT, 5) && !f.get(SLOT, 8) {
                x(ctl, i).brought = true;
                w.delete_item(p, GIDBINN);
                x(ctl, i).held -= 1;
                set(w, p, SLOT, &[6]);
                // `0x005B91B0`.
                for m in w.party_members(p).unwrap_or_default() {
                    if in_act3(w, m) && lacks_all(w, m) {
                        set(w, m, SLOT, &[6]);
                    }
                }
                // `0x005B9130`: nothing sent.
                for m in w.players() {
                    if lacks_all(w, m) && !w.has_item(m, GIDBINN) {
                        set(w, m, SLOT, &[bit::COMPLETED_NOW]);
                    }
                }
            }
            if ctl.records[i].not_intro {
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
            }
            status_silent(ctl, i, 13);
            if ctl.records[i].not_intro && ctl.records[i].state != 5 {
                ctl.records[i].state = 5;
                sequence(ctl, w, CHAIN);
            }
        }
        (a, 593) if a == u32::from(npc::ORMUS) && f.get(SLOT, 6) && !f.get(SLOT, 8) => {
            set(w, p, SLOT, &[8]);
            ring(w, p);
            if f.get(SLOT, 7) {
                finish(ctl, w, i, p);
            }
        }
        (a, 589) if a == u32::from(npc::ASHEARA) && !f.get(SLOT, 7) => {
            set(w, p, SLOT, &[7]);
            w.mercenary_reward(p, npc::ASHEARA);
            if f.get(SLOT, 8) {
                finish(ctl, w, i, p);
            }
        }
        _ => {}
    }
}

/// Lacking 19.0, 19.6, 19.8 and 19.7.
fn lacks_all<W: QuestWorld>(w: &mut W, p: UnitId) -> bool {
    let f = pf(w, p);
    [bit::REWARD_GRANTED, 6, 8, 7]
        .iter()
        .all(|&b| !f.get(SLOT, b))
}

/// Both rewards taken: set 19.13, 19.0, add GUID.
fn finish<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    set(w, p, SLOT, &[bit::PRIMARY_GOAL_DONE, bit::REWARD_GRANTED]);
    add_guid(ctl, w, i, p);
}

/// Ormus' ring (§5.5): rare, droppable, level by difficulty.
fn ring<W: QuestWorld>(w: &mut W, p: UnitId) {
    let level = match w.difficulty() {
        0 => 21,
        1 => 35,
        _ => 75,
    };
    w.reward_item(p, RING, level, 6, true);
}

/// Event 2 `0x005B8C50` (§5.7; never cleared).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    match npc_of(w, &args) {
        Some(npc::HRATLI) if x(ctl, i).hratli_started => {
            status_all(ctl, w, i, 2);
            x(ctl, i).hratli_started = false;
            flag_iterate_all(ctl, w, i);
        }
        Some(npc::ORMUS) if x(ctl, i).brought => {
            let e = x(ctl, i);
            e.brought = false;
            e.altar_ready = true;
        }
        _ => {}
    }
}

/// Event 3 `0x005B9090` (§5.8; a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let r = &mut ctl.records[i];
    if args.b == FLAYER_JUNGLE && r.not_intro && r.state == 0 {
        r.state = 1;
    }
    if args.a != DOCKS {
        return;
    }
    let Some(p) = args.player else { return };
    quick_remove(ctl, w, i, p);
    let f = pf(w, p);
    if ctl.records[i].state == 2
        && !f.get(SLOT, bit::REWARD_GRANTED)
        && !f.get(SLOT, bit::COMPLETED_BEFORE)
    {
        ctl.records[i].state = 3;
        // "the flag iterate" (not "for all"): the moving player.
        flag_iterate(ctl, w, i, p);
    }
}

/// Event 4 `0x005B9520` (§5.8; edge case 5: status 4 then 3).
fn picked_up<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let is_gidbinn = args.target.and_then(|t| w.item_code(t)) == Some(GIDBINN);
    if is_gidbinn && ctl.records[i].not_intro {
        status_all(ctl, w, i, 4);
        if let Some(p) = args.player {
            if !pf(w, p).get(SLOT, 9) {
                set(w, p, SLOT, &[9]);
                w.attach_sound(p, sound::GIDBINN);
            }
        }
        ctl.records[i].state = 4;
        status_all(ctl, w, i, 3);
    }
    flag_iterate_all(ctl, w, i);
}

/// The Gidbinn kill test (§5.6, `quests-act3-2.md` §11.4), what the host's
/// [`QuestWorld::special_monster`] computes: `0x005A0180(victim, 0x0E)`,
/// the victim's monster type flags & 0x0E ≠ 0, else `0x0063E9F0(0,
/// victim)`, its monstats `boss` column (flag word +0x0C bit 6). Either
/// one passes. The boss itself (a random boss, unique or champion; edge
/// case 19) carries type flags 1 | 8 and always passes.
pub fn gidbinn_kill_test(type_flags: u8, monstats_boss: bool) -> bool {
    type_flags & KILL_TYPE_FLAGS != 0 || monstats_boss
}

/// Event 8 `0x005B9980` (§5.6; installed by the boss timer). The drop's
/// item level is the victim's stat 12 (`quests-act3-2.md` §11.3, the
/// `&level` output; no incoming value).
fn killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !x(ctl, i).boss_spawned || !ctl.records[i].not_intro {
        return;
    }
    let Some(victim) = args.target else { return };
    if !w.special_monster(victim) {
        return;
    }
    if w.quest_drop(victim, GIDBINN, 2, None, false).is_some() {
        x(ctl, i).gidbinn_dropped = true;
        flag_iterate_all(ctl, w, i);
        ctl.records[i].clear_callback(event::MONSTER_KILLED);
    } else {
        x(ctl, i).boss_spawned = false;
    }
}

/// Event 13 `0x005B9700` (§5.8).
fn started<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let holds = w.has_item(p, GIDBINN);
    if holds {
        x(ctl, i).held += 1;
    }
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) {
        x(ctl, i).altar_mode = 2;
        return;
    }
    if holds {
        status_silent(ctl, i, 4);
        ctl.records[i].state = 4;
        flag_iterate(ctl, w, i, p);
        return;
    }
    if !f.get(SLOT, 6) {
        if f.get(SLOT, 5) {
            ctl.records[i].state = 3;
            status_silent(ctl, i, 2);
        }
        if f.get(SLOT, 4) || f.get(SLOT, bit::LEAVE_TOWN) {
            ctl.records[i].state = 2;
            status_silent(ctl, i, 2);
        } else if f.get(SLOT, bit::STARTED) {
            ctl.records[i].state = 3;
            status_silent(ctl, i, 1);
        }
        return;
    }
    x(ctl, i).altar_mode = 2;
    ctl.records[i].state = 5;
    if !f.get(SLOT, 7) {
        status_silent(ctl, i, 5);
    } else if f.get(SLOT, 8) {
        set(w, p, SLOT, &[bit::REWARD_GRANTED]);
        ctl.records[i].not_intro = false;
    } else {
        status_silent(ctl, i, 6);
    }
}

/// Status function `0x005B8CC0` (always reports, §5.9).
pub(super) fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    r: &QuestFlags,
) -> u8 {
    let e = &ctl.records[i].extra.act3.q3;
    if !r.get(15, bit::REWARD_GRANTED) {
        0
    } else if r.get(SLOT, bit::REWARD_GRANTED) {
        11 + 2 * u8::from(r.get(SLOT, bit::PRIMARY_GOAL_DONE))
    } else if w.has_item(player, GIDBINN) {
        4
    } else if e.holder_left {
        7 + u8::from(w.game_type() == 3)
    } else if r.get(SLOT, 6) {
        let mut s = 0;
        if !r.get(SLOT, 8) {
            s = 6;
        }
        if !r.get(SLOT, 7) {
            s = 5;
        }
        s
    } else if r.get(SLOT, 4) {
        2 + u8::from(e.gidbinn_dropped)
    } else if r.get(SLOT, bit::LEAVE_TOWN) {
        2
    } else if r.get(SLOT, bit::STARTED) {
        1
    } else {
        0
    }
}

/// End-animation event (type 1) at frame + (`FrameCnt1` >> 8) + `extra`.
fn end_animation<W: QuestWorld>(w: &mut W, object: UnitId, extra: i32) {
    let at = w.frame() + (w.object_anim_length(object) >> 8) + extra;
    w.schedule_object_event(object, 1, at);
}

/// Decoy operate 31 `0x005B9B40` (returns 0).
pub fn decoy_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    // "object mode 0": read as the operate's precondition (as "Operate,
    // object mode 0:" in `quests-act2.md` and §3.6 here).
    if w.object_mode(object) != 0 {
        return;
    }
    let f = pf(w, player);
    if [bit::REWARD_GRANTED, 7, 8].iter().any(|&b| f.get(SLOT, b)) {
        w.attach_sound(player, sound::REFUSED);
        return;
    }
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro {
        return;
    }
    if ctl.records[i].state == 0 {
        ctl.records[i].state = 1;
    }
    w.set_object_mode(object, 1);
    end_animation(w, object, 0);
    x(ctl, i).decoy_active = true;
    if !x(ctl, i).timer {
        x(ctl, i).timer = true;
        if let Err(e) = ctl.add_timer(CHAIN, TimerFn::Act3(Timer::GidbinnBoss), BOSS_PERIOD) {
            ctl.faults.push(e);
        }
    }
}

/// Decoy init 25 `0x005B9AE0`; `at` is the object's init record
/// (`quests-act3-2.md` §11.7 rule 1): +0x08 / +0x0C := its (x, y), the
/// boss spawn uses its room.
pub fn decoy_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId, at: InitPoint) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro {
        w.set_object_mode(object, 2);
        return;
    }
    let e = x(ctl, i);
    e.decoy_known = true;
    (e.decoy_x, e.decoy_y) = (at.x, at.y);
    if e.decoy_active && !e.boss_spawned {
        spawn_boss(ctl, w, i, at.room);
    }
}

/// Timer `0x005B9A30`; returns 1 (remove): one attempt per operation
/// (edge case 4). The room is the Act III room covering (+0x08, +0x0C)
/// (`0x00619DA0` on game +0xC4); the boss spawns in that covering room,
/// not the player's (`quests-act3-2.md` §11.7 rule 1).
pub(super) fn boss_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    x(ctl, i).timer = false;
    let e = x(ctl, i).clone();
    if !(e.decoy_known && e.decoy_active && !e.boss_spawned) || !w.has_act3() {
        return true;
    }
    let Some(room) = w.room_covering(e.decoy_x, e.decoy_y) else {
        return true;
    };
    if w.player_in_rooms(room) {
        spawn_boss(ctl, w, i, room);
        install(ctl, i, event::MONSTER_KILLED);
    }
    true
}

/// Boss spawn `0x005B9930`: monster 407 `fetish11` in `room`, the random
/// boss of `monsters/init.md` §16.1 with champions allowed
/// (`0x005A43E0(game, room, 0, 407, 1, 0, 0, 1)`, `quests-act3-2.md`
/// §11.4; edge case 19).
fn spawn_boss<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, room: RoomId) {
    x(ctl, i).boss_spawning = true;
    if let Some(u) = w.spawn_monster_in_room(room, npc::FETISH11) {
        let g = w.guid(u);
        let e = x(ctl, i);
        e.decoy_active = false;
        e.boss_spawned = true;
        e.boss_guid = g;
    }
    x(ctl, i).boss_spawning = false;
}

/// Altar init 39 `0x005B9D40` (object 251); `at` is the object's init
/// record (`quests-act3-2.md` §11.7 rule 2): +0x10 / +0x14 := its (x, y).
pub fn altar_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId, at: InitPoint) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    let e = x(ctl, i);
    e.altar_guid = g;
    (e.altar_x, e.altar_y) = (at.x, at.y);
    let mode = e.altar_mode;
    w.set_object_mode(object, mode);
}

/// `0x005B9CA0` (Ormus' map AI): the altar position, only while +0x06.
pub fn altar_position(ctl: &QuestControl) -> Option<(i32, i32)> {
    let e = &ctl.record(CHAIN)?.extra.act3.q3;
    e.altar_ready.then_some((e.altar_x, e.altar_y))
}

/// `0x005B9CD0` (Ormus' map AI): activate the altar. +0x06 is cleared
/// first; when the altar unit (+0x28, unit type 2) is not found nothing
/// else happens and +0x2C keeps its value (`quests-act3-2.md` §11.7
/// rule 2).
pub fn activate_altar<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    x(ctl, i).altar_ready = false;
    let Some((altar, _)) = w.object_by_guid(x(ctl, i).altar_guid) else {
        return;
    };
    w.set_object_mode(altar, 1);
    end_animation(w, altar, 1);
    x(ctl, i).altar_mode = 2;
}
