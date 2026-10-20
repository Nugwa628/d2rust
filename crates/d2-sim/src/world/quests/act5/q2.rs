// Spec: specs/world/quests-act5.md §4 (A5Q2 Rescue on Mount Arreat, chain 32); specs/world/quests.md §6.2 step 3 (barbarians left)
//! A5Q2 callback by callback: the flag iterate (§4.2), chat and active
//! function (§4.3), Qual-Kehk's messages and chat end (§4.4), level
//! changes (§4.5), kills (§4.6), the cages, the barbarians, the rescue
//! and its completion check (§4.7), the rescue portals (§4.8), game
//! start, join and leave (§4.9), the barbarian AI hooks (§4.10) and the
//! barbarians-left count of 0x50 / 0x5D (`quests.md` §6.2). Slot 36.

use super::super::late::{
    add_guid, add_state, clear, completion_flag, flags, guid_listed, in_act, iterate_all, leave,
    party_of, quick_remove, refresh, reset_progress, s5d, set, set_state, status_silent,
    status_to_all, table_state,
};
use super::super::{
    bit, event, flags_of, EventArgs, GuidList, QuestControl, QuestFlags, QuestRecord, QuestWorld,
    TextList,
};
use crate::units::UnitId;

const CHAIN: u8 = 32;
const SLOT: u8 = 36;
/// Qual-Kehk.
pub const QUAL_KEHK: u16 = 515;
/// The caged barbarians (act5pow).
pub const BARBARIAN: u16 = 534;
/// The prison doors (prisondoor).
pub const PRISON_DOOR: u16 = 434;
/// The rescue portal object.
pub const PORTAL: u16 = 189;
/// 36.3, 36.4 and the freed-count bits 36.5 / 36.6 / 36.7 (§4.1).
const LEFT_TOWN: u8 = 3;
const RESCUED_GROUP: u8 = 4;
const FREED_15: u8 = 5;
const FREED_14: u8 = 6;
const FREED_12: u8 = 7;
/// `0x00734270`: table state by record state.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];
/// `0x00732FEC`: the rune rewards (Tal, Ral, Ort).
pub const RUNES: [[u8; 4]; 3] = [*b"r07 ", *b"r08 ", *b"r09 "];
/// Monster mode dead.
const DEAD: i32 = 12;
/// Barbarians per cage group.
const GROUP: usize = 5;
/// The rescue sound (§4.7).
const SOUND_RESCUE: u16 = 81;
/// Object event delay of the rescue portals (frames).
const PORTAL_DELAY: i32 = 25;

/// Quest extra data (record +0x18, 0x160 bytes, §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: a GUID list (reset at init; event 10 removes from it).
    pub guids: GuidList,
    /// +0x84: Qual-Kehk started the quest (chat end pending).
    pub started: bool,
    /// +0x86 + g: cage group g spawned.
    pub cage_spawned: [bool; 3],
    /// +0x8C + 8g / +0x90 + 8g: group g's cage position.
    pub cage_pos: [(i32, i32); 3],
    /// +0xA4: barbarians spawned.
    pub spawned: i32,
    /// +0xA8: barbarians killed.
    pub killed: i32,
    /// +0xAC: barbarians freed.
    pub freed: i32,
    /// +0xB0 + 20g + 4n: GUID of barbarian n of group g (0: none).
    pub barbarians: [[u32; GROUP]; 3],
    /// +0xEC + 4g: group g's portal GUID.
    pub portal_guid: [u32; 3],
    /// +0xF8 + g: group g's portal spawned.
    pub portal_spawned: [bool; 3],
    /// +0xFC + 4g: group g's portal close counter.
    pub close_counter: [i32; 3],
    /// +0x108 + g: group g accounted for.
    pub accounted: [bool; 3],
    /// +0x10B + g: group g's portal may close.
    pub may_close: [bool; 3],
    /// +0x10E + g: group g's portal made.
    pub portal_made: [bool; 3],
    /// +0x114 + 4g: group g's counter.
    pub counter: [i32; 3],
    /// +0x120 (count +0x15C): GUIDs of the freed barbarians.
    pub freed_guids: Vec<u32>,
}

/// Timers this quest makes (`quests.md` §5): none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {}

/// Init `0x00588510` beyond `quests.tsv` (§2): extra zeroed, list +0x00
/// reset.
pub fn init(r: &mut QuestRecord) {
    r.extra.a5.q2 = Extra::default();
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a5.q2
}

/// The group of the barbarian with GUID `g` (`None`: in no group).
fn group_of(e: &Extra, g: u32) -> Option<usize> {
    e.barbarians.iter().position(|b| b.contains(&g))
}

/// A group counter step (§4.6 r2 and r3, §4.10): += 1; ≥ 5 → the group
/// is accounted for.
fn count(e: &mut Extra, g: usize) {
    e.counter[g] += 1;
    if e.counter[g] >= GROUP as i32 {
        e.accounted[g] = true;
    }
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
            // `0x00587F20`.
            if let Some(p) = args.player {
                leave(ctl, w, i, p);
                let g = w.guid(p);
                x(ctl, i).guids.remove(g);
            }
        }
        event::SCROLL_MESSAGE => messages(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => game_start(ctl, w, i, args),
        event::PLAYER_JOINED_GAME => {
            // `0x00588450`.
            if let Some(f) = args.player.and_then(|p| flags_of(w, p)) {
                f.clear(SLOT, RESCUED_GROUP);
            }
        }
        _ => return false,
    }
    true
}

/// Flag iterate `0x00587A90` (§4.2) for every player.
fn flag_iterate_all<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let state = ctl.records[i].state;
    for p in w.players() {
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match state {
            2 => f.set(SLOT, bit::STARTED),
            3 => f.set(SLOT, LEFT_TOWN),
            _ => {}
        }
    }
}

/// Event 0 `0x00587D30` (§4.3).
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
    let r = &ctl.records[i];
    let m = if f.get(SLOT, bit::REWARD_PENDING) {
        3
    } else if guid_listed(ctl, w, i, p) {
        4
    } else if f.get(SLOT, bit::REWARD_GRANTED)
        || (r.state > 3 && !f.get(SLOT, bit::PRIMARY_GOAL_DONE))
        || !r.not_intro
    {
        return;
    } else {
        match table_state(&MSG_STATE, r.state) {
            Some(2) if class == QUAL_KEHK && f.get(SLOT, RESCUED_GROUP) => 5,
            Some(m) if m <= 11 => m,
            _ => return,
        }
    };
    add_state(ctl, i, list, class, m);
}

/// Active `0x00588340` (§4.3).
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = f;
    let pf = flags(w, player);
    let lacking = !pf.get(SLOT, bit::REWARD_GRANTED) && !pf.get(SLOT, bit::REWARD_PENDING);
    match npc_class {
        QUAL_KEHK => (ctl.records[i].state == 1 && lacking) || pf.get(SLOT, bit::REWARD_PENDING),
        BARBARIAN => lacking,
        _ => false,
    }
}

/// The status function: chain 32 has none (`quests.md` §6.1 default
/// rule); reached only if a host registers one.
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

/// Event 11 `0x00587B00` (§4.4; a = NPC class, b = message).
fn messages<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a != u32::from(QUAL_KEHK) {
        return;
    }
    match args.b {
        20096 => {
            x(ctl, i).started = true;
            set_state(ctl, i, 2);
            flag_iterate_all(ctl, w, i);
            if let Some(n) = args.target {
                refresh(ctl, w, p, n);
            }
        }
        20110 => {
            let f = flags(w, p);
            if !f.get(SLOT, bit::REWARD_PENDING) {
                return;
            }
            if f.get(SLOT, bit::PRIMARY_GOAL_DONE) {
                if ctl.records[i].state != 5 {
                    set_state(ctl, i, 5);
                    super::sequence(ctl, w, CHAIN);
                    // The one Act V "(silent)" call clears the flags
                    // first (`0x00587C1E`, `quests-act5-2.md` §6.8).
                    ctl.records[i].flags = 0;
                    status_silent(ctl, i, 13);
                }
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            }
            let n = if f.get(SLOT, FREED_14) {
                2
            } else if f.get(SLOT, FREED_12) {
                1
            } else {
                3
            };
            let mut given = false;
            for code in &RUNES[..n] {
                given |= w.reward_item(p, *code, 0, 2, true).is_some();
            }
            if given {
                set(w, p, SLOT, bit::REWARD_GRANTED);
                clear(w, p, SLOT, bit::REWARD_PENDING);
                reset_progress(w, p, SLOT);
                add_guid(ctl, w, i, p);
                s5d(w, p, CHAIN, 0x02, 0);
            }
            if let Some(n) = args.target {
                refresh(ctl, w, p, n);
            }
        }
        _ => {}
    }
}

/// Event 2 `0x00587A30` (§4.4).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.target.and_then(|n| w.monster_class(n)) != Some(QUAL_KEHK) {
        return;
    }
    if x(ctl, i).started && x(ctl, i).killed < 5 {
        status_to_all(ctl, w, i, 1);
        x(ctl, i).started = false;
        ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
    }
}

/// Event 3 `0x00588200` (§4.5; a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let not_intro = ctl.records[i].not_intro;
    let killed = x(ctl, i).killed;
    if matches!(args.b, 111 | 112) && not_intro && killed < 5 {
        let b = matches!(ctl.records[i].state, 1 | 2);
        if b {
            set_state(ctl, i, 3);
        }
        if ctl.records[i].status == 0 {
            status_to_all(ctl, w, i, 1);
            ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            flag_iterate_all(ctl, w, i);
        } else if b {
            flag_iterate_all(ctl, w, i);
        }
    } else if args.a == 109 {
        quick_remove(ctl, w, i, p);
        let f = flags(w, p);
        // The whole tail is inside the state 2 test (`0x00588200`).
        if ctl.records[i].state == 2
            && !f.get(SLOT, bit::REWARD_GRANTED)
            && !f.get(SLOT, bit::REWARD_PENDING)
        {
            set_state(ctl, i, 3);
            flag_iterate_all(ctl, w, i);
            if ctl.records[i].status == 0 && not_intro && killed < 5 {
                status_to_all(ctl, w, i, 1);
                ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            }
        }
    }
}

/// Event 8 `0x00588040` (§4.6): prison doors and spawned barbarians.
fn killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !ctl.records[i].not_intro {
        return;
    }
    let Some(victim) = args.target else { return };
    let vg = w.guid(victim);
    if w.monster_class(victim) == Some(PRISON_DOOR) {
        // Step 1.
        if let Some((_, _, room)) = w.unit_position(victim) {
            w.set_room_portal(room, false);
            for u in w.adjacent_units(room) {
                if w.monster_class(u) != Some(BARBARIAN) {
                    continue;
                }
                let mode = w.unit_mode(u);
                if mode == DEAD
                    || mode == 0
                    || w.distance_between(u, victim).is_none_or(|d| d >= 15)
                {
                    continue;
                }
                let g = w.guid(u);
                let e = x(ctl, i);
                e.freed_guids.push(g);
                e.freed += 1;
            }
        }
    } else {
        // Step 2.
        let e = x(ctl, i);
        if e.freed_guids.contains(&vg) {
            if let Some(g) = group_of(e, vg) {
                count(e, g);
            }
            return;
        }
        e.killed += 1;
    }
    // Step 3.
    if x(ctl, i).killed > 4 && ctl.records[i].status != 12 {
        status_to_all(ctl, w, i, 12);
        x(ctl, i).started = false;
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
    }
    let e = x(ctl, i);
    if let Some(g) = group_of(e, vg) {
        count(e, g);
    }
}

/// Event 13 `0x00588470` (§4.9).
fn game_start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let Some(f) = flags_of(w, p) else { return };
    f.clear(SLOT, RESCUED_GROUP);
    let f = *f;
    if f.get(SLOT, bit::REWARD_GRANTED)
        || f.get(SLOT, bit::COMPLETED_BEFORE)
        || f.get(SLOT, bit::REWARD_PENDING)
    {
        return;
    }
    let r = &mut ctl.records[i];
    if f.get(SLOT, LEFT_TOWN) {
        (r.state, r.status) = (3, 1);
    } else if f.get(SLOT, bit::STARTED) {
        (r.state, r.status) = (2, 1);
    }
}

/// Runs a timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    let _ = (ctl, w, chain);
    match t {}
}

/// `0x00588C50`: barbarians left for the u16 extra of 0x50 / 0x5D
/// (`quests.md` §6.2 step 3): 5 for each cage group not spawned, plus
/// spawned − freed − killed, floored at 0. 0 without a chain-32 record.
pub fn barbarians_left<W: QuestWorld>(ctl: &QuestControl, w: &mut W) -> u16 {
    let _ = w;
    let Some(r) = ctl.record(CHAIN) else { return 0 };
    let e = &r.extra.a5.q2;
    let unspawned = e.cage_spawned.iter().filter(|&&s| !s).count() as i32;
    let left = 5 * unspawned + e.spawned - e.freed - e.killed;
    left.max(0) as u16
}

/// Init 62 `0x005886A0` (object 473 `cagedwussie1`, §4.7).
pub fn cage_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if ctl.records[i].not_intro {
        if ctl.records[i].state < 2 {
            set_state(ctl, i, 2);
        }
        if ctl.records[i].status == 0 && x(ctl, i).killed < 5 {
            status_to_all(ctl, w, i, 1);
        }
    }
    let Some(g) = x(ctl, i).cage_spawned.iter().position(|&s| !s) else {
        return;
    };
    let Some((ox, oy, room)) = w.unit_position(object) else {
        return;
    };
    if x(ctl, i).cage_pos[..g].contains(&(ox, oy)) {
        return;
    }
    x(ctl, i).cage_pos[g] = (ox, oy);
    // `0x00588600`.
    let mut n = 0;
    for _ in 0..25 {
        if n == GROUP {
            break;
        }
        let Some(b) = w.spawn_monster(room, ox, oy, BARBARIAN, 1, 5) else {
            continue;
        };
        w.or_unit_flags(b, 0x0300_0000);
        let bg = w.guid(b);
        x(ctl, i).barbarians[g][n] = bg;
        ctl.add_link(w, b, CHAIN, None);
        n += 1;
    }
    let e = x(ctl, i);
    e.spawned += n as i32;
    e.cage_spawned[g] = true;
    e.portal_made[g] = false;
}

/// The bits of the completion check for `freed` barbarians: 36.5 for
/// 15, 36.6 for 14, 36.7 for 12–13 (§4.7).
fn freed_bit(freed: i32) -> u8 {
    // Equality tests only (`0x00588B96`–`0x00588BA8`): 15 → 36.5, 14 →
    // 36.6, any other count → 36.7.
    match freed {
        15 => FREED_15,
        14 => FREED_14,
        _ => FREED_12,
    }
}

/// The rescue `0x005888D0` (from the prisoner AI at `0x005EE533`):
/// player `p` freed barbarian `b` (§4.7).
pub fn rescue<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, p: UnitId, b: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let bg = w.guid(b);
    let Some(g) = group_of(x(ctl, i), bg) else {
        return;
    };
    if x(ctl, i).portal_made[g] {
        return;
    }
    // The dead door near B in a room adjacent to P's room.
    let Some((_, _, proom)) = w.unit_position(p) else {
        return;
    };
    let mut door = None;
    for u in w.adjacent_units(proom) {
        if w.monster_class(u) == Some(PRISON_DOOR)
            && w.unit_mode(u) == DEAD
            && w.distance_between(u, b).is_some_and(|d| d < 15)
        {
            door = Some(u);
            break;
        }
    }
    let Some((dx, dy, room)) = door.and_then(|d| w.unit_position(d)) else {
        return;
    };
    let (tx, ty) = (dx + 2, dy);
    x(ctl, i).portal_made[g] = true;
    let mut made = w
        .place_object(room, tx, ty, PORTAL, [1, 1, 0])
        .map(|o| (o, room));
    if made.is_none() {
        made = w
            .place_object(room, tx - 2, ty, PORTAL, [1, 1, 0])
            .map(|o| (o, room));
    }
    if made.is_none() {
        if let Some((sx, sy, sroom)) = w.free_spot_at(room, tx, ty, 2, 0x8000, 17, 100) {
            made = w
                .place_object(sroom, sx, sy, PORTAL, [1, 1, 0])
                .map(|o| (o, sroom));
        }
    }
    if let Some((o, oroom)) = made {
        let og = w.guid(o);
        let e = x(ctl, i);
        e.portal_spawned[g] = true;
        e.portal_guid[g] = og;
        let frame = w.frame() + w.tick_rate().ticks(PORTAL_DELAY);
        w.schedule_quest_event(o, frame);
        w.set_room_portal(oroom, false);
    }
    completion_check(ctl, w, i, p);
}

/// The completion check of the rescue (§4.7, P's flags).
fn completion_check<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    let e = x(ctl, i).clone();
    let all = e.spawned == e.killed + e.freed && e.cage_spawned.iter().all(|&s| s);
    let lacking = [
        bit::REWARD_GRANTED,
        bit::REWARD_PENDING,
        bit::PRIMARY_GOAL_DONE,
    ];
    if all {
        if e.freed < 12 {
            completion_flag(w, CHAIN, SLOT, &lacking);
            return;
        }
        status_to_all(ctl, w, i, 3);
        let fb = freed_bit(e.freed);
        let f = flags(w, p);
        if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) {
            set(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
            set(w, p, SLOT, bit::REWARD_PENDING);
            set(w, p, SLOT, fb);
        }
        // `0x005887A0` → `0x00587E60` for each player.
        for q in w.players() {
            if !flags(w, q).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                continue;
            }
            for m in party_of(w, q) {
                let f = flags(w, m);
                if !f.get(SLOT, bit::REWARD_GRANTED)
                    && !f.get(SLOT, bit::REWARD_PENDING)
                    && in_act(w, m, 4)
                {
                    set(w, m, SLOT, bit::PRIMARY_GOAL_DONE);
                    set(w, m, SLOT, bit::REWARD_PENDING);
                    set(w, m, SLOT, fb);
                }
            }
        }
        // `0x00587F60`.
        completion_flag(w, CHAIN, SLOT, &lacking);
        // `0x005887F0`.
        for q in w.players() {
            if flags(w, q).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                w.attach_sound(q, SOUND_RESCUE);
            }
        }
    } else {
        set(w, p, SLOT, RESCUED_GROUP);
        if e.killed < 5 {
            // The flags byte is set to 0x20, not cleared, before the
            // status call (`0x00588C2A`).
            ctl.records[i].flags = 0x20;
            ctl.records[i].status = 2;
            iterate_all(ctl, w, i);
        }
        x(ctl, i).started = false;
    }
}

/// `0x00588830` (prisoner AI `0x005EE3F6`, §4.10): not-intro and B's
/// group counter ≠ 0.
pub fn group_counting<W: QuestWorld>(ctl: &QuestControl, w: &mut W, b: UnitId) -> bool {
    let Some(r) = ctl.record(CHAIN) else {
        return false;
    };
    let e = &r.extra.a5.q2;
    r.not_intro && group_of(e, w.guid(b)).is_some_and(|g| e.counter[g] != 0)
}

/// `0x00588880` (prisoner AI `0x005EE423`, §4.10): not-intro: B's group
/// counter += 1; > 4 → accounted for.
pub fn group_count<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, b: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro {
        return;
    }
    let bg = w.guid(b);
    let e = x(ctl, i);
    if let Some(g) = group_of(e, bg) {
        count(e, g);
    }
}

/// [`group_count`] for the barbarian with GUID `bg` (the prisoner AI
/// removes the unit in the same call, so a host that runs the hook after
/// the AI step passes the GUID).
pub fn group_count_guid(ctl: &mut QuestControl, bg: u32) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro {
        return;
    }
    let e = x(ctl, i);
    if let Some(g) = group_of(e, bg) {
        count(e, g);
    }
}

/// Every spawned barbarian's (GUID, [`group_counting`], the group's
/// portal GUID when spawned): what the prisoner AI's reads see.
pub fn barbarian_states(ctl: &QuestControl) -> Vec<(u32, bool, Option<u32>)> {
    let Some(r) = ctl.record(CHAIN) else {
        return Vec::new();
    };
    let e = &r.extra.a5.q2;
    let mut v = Vec::new();
    for g in 0..3 {
        for &b in &e.barbarians[g] {
            if b != 0 {
                let portal = e.portal_spawned[g].then_some(e.portal_guid[g]);
                v.push((b, r.not_intro && e.counter[g] != 0, portal));
            }
        }
    }
    v
}

/// `0x00588D60` (prisoner AI `0x005EE3DB`, §4.10): B's group portal,
/// when spawned and the unit exists.
pub fn group_portal<W: QuestWorld>(ctl: &QuestControl, w: &mut W, b: UnitId) -> Option<UnitId> {
    let e = &ctl.record(CHAIN)?.extra.a5.q2;
    let g = group_of(e, w.guid(b))?;
    if !e.portal_spawned[g] {
        return None;
    }
    w.object_by_guid(e.portal_guid[g]).map(|o| o.0)
}

/// `0x00588DD0` (prisoner AI `0x005EE562`, §4.10): not-intro, status ≠
/// 5 and killed < 5 → status 5 to all.
pub fn rescue_status<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let r = &ctl.records[i];
    if r.not_intro && r.status != 5 && r.extra.a5.q2.killed < 5 {
        status_to_all(ctl, w, i, 5);
    }
}

/// `0x00588E10` (prisoner AI `0x005EE525`, §4.10): a dead prison door in
/// a room adjacent to the player's room (without a player: game type
/// 3's first client's player, else B's room).
pub fn door_open_near<W: QuestWorld>(w: &mut W, player: Option<UnitId>, b: UnitId) -> bool {
    let u = player
        .or_else(|| {
            if w.game_type() == 3 {
                w.first_client_player()
            } else {
                None
            }
        })
        .unwrap_or(b);
    let Some((_, _, room)) = w.unit_position(u) else {
        return false;
    };
    for d in w.adjacent_units(room) {
        if w.monster_class(d) == Some(PRISON_DOOR) && w.unit_mode(d) == DEAD {
            return true;
        }
    }
    false
}

/// `0x00588CA0`: object event 7 of a rescue portal (class 189 outside
/// levels 109 and ≥ 113, §4.8).
pub fn portal_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let og = w.guid(object);
    let Some(g) = x(ctl, i).portal_guid.iter().position(|&p| p == og) else {
        return;
    };
    match w.object_mode(object) {
        1 => w.set_object_mode(object, 2),
        2 => {
            let e = x(ctl, i);
            if e.accounted[g] {
                e.close_counter[g] += 1;
                if e.close_counter[g] > 5 {
                    e.may_close[g] = true;
                }
            }
            if e.may_close[g] {
                w.set_object_mode(object, 3);
            }
        }
        3 => w.set_object_mode(object, 4),
        _ => {}
    }
    let frame = w.frame() + w.tick_rate().ticks(PORTAL_DELAY);
    w.schedule_quest_event(object, frame);
}

#[cfg(test)]
#[path = "q2_tests.rs"]
mod tests;
