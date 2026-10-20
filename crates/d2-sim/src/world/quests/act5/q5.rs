// Spec: specs/world/quests-act5-2.md §7 (A5Q5 Rite of Passage), §10
//! A5Q5 Rite of Passage (chain 35, slot 39): Qual-Kehk's start, the
//! altar, the three statues and the Ancients they release, the reset on
//! a town portal or a wiped summit, the experience reward with its level
//! gate, the doors, the invisible Ancient, the summit warp check and the
//! hooks of the Ancients' AI. The sequence function (§7.10) is in
//! `act5.rs`; chains 35 and 36 use the default status rule (part 1 §2).

use super::super::late::{self, flags, set};
use super::super::{
    bit, event, EventArgs, QuestControl, QuestFlags, QuestRecord, QuestWorld, TextList, TimerFn,
};
use crate::units::UnitId;

const CHAIN: u8 = 35;
const SLOT: u8 = 39;
/// NPCs (`monstats.txt`).
pub const LARZUK: u16 = 511;
pub const DREHYA: u16 = 512;
pub const MALAH: u16 = 513;
pub const QUAL_KEHK: u16 = 515;
pub const CAIN6: u16 = 520;
/// ancientstatue1–3.
pub const STATUE_NPCS: [u16; 3] = [537, 538, 539];
/// ancientbarb1–3 (the Ancients).
pub const ANCIENT_FIRST: u16 = 540;
/// Levels: Harrogath, Arreat Summit.
const HARROGATH: u32 = 109;
pub const SUMMIT: u32 = 120;
/// Objects: the statues 474 / 475 / 476, the altar, the door to the
/// Worldstone Keep, the invisible Ancient, the summit door.
pub const STATUES: [u16; 3] = [474, 475, 476];
pub const ALTAR: u16 = 546;
pub const KEEP_DOOR: u16 = 547;
pub const INVISIBLE_ANCIENT: u16 = 561;
pub const SUMMIT_DOOR: u16 = 564;
/// The order the extra data keeps the statues in: GUIDs +0x14 / +0x18 /
/// +0x1C, respawn bytes +0x3C / +0x3D / +0x3E.
const ORDER: [u16; 3] = [476, 474, 475];
/// `0x007357E4`: table state by record state.
const MSG_STATE: [i8; 7] = [-1, 0, 1, 2, 3, 4, 0];
/// Messages.
const MSG_START: u32 = 20153;
pub const MSG_ALTAR: u16 = 20002;
pub const MSG_INVISIBLE: u16 = 20169;
/// Post-quest lines: (message, NPC, bit).
const POST_QUEST: [(u32, u16, u8); 5] = [
    (20167, LARZUK, 5),
    (20165, CAIN6, 6),
    (20166, DREHYA, 7),
    (20168, MALAH, 8),
    (20164, QUAL_KEHK, 9),
];
const SOUND_REFUSED: u16 = 19;
const FX_ANCIENT: u8 = 18;
/// The statue timer's period; object event delays.
const TIMER_PERIOD: u32 = 2;
const ARM_DELAY: i32 = 20;
const SPAWN_RETRY: i32 = 10;
/// Missile 541 (Ancient to statue), flags, level.
const MISSILE: u16 = 541;
const MISSILE_FLAGS: u32 = 0x420;
/// Player modes `0x0058D510` skips: death, dead.
const MODE_DEATH: i32 = 0;
const MODE_DEAD: i32 = 0x11;
/// Ancients reward by difficulty (§7.7).
const REWARD: [u32; 3] = [1_400_000, 20_000_000, 40_000_000];
/// Stats: level, experience, last experience, next experience.
const STAT_LEVEL: u16 = 12;
const STAT_EXP: u16 = 13;
const STAT_LAST_EXP: u16 = 29;
const STAT_NEXT_EXP: u16 = 30;

/// Quest extra data (record +0x18, 0x64 bytes).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the Ancients are defeated.
    pub defeated: bool,
    /// +0x01: Qual-Kehk started the quest (chat end pending).
    pub qual_started: bool,
    /// +0x04: the statue timer exists.
    pub timer: bool,
    /// +0x0C: town portals open in level 120.
    pub portals: i32,
    /// +0x10: the altar was used.
    pub altar_used: bool,
    /// +0x11: the fight is armed.
    pub armed: bool,
    /// Statue GUIDs by `STATUES` index (474, 475, 476; +0x18, +0x1C,
    /// +0x14).
    pub statue_guids: [u32; 3],
    /// +0x20: the Ancients' GUIDs by spawn slot.
    pub ancient_guids: [u32; 3],
    /// +0x2C: "Ancient spawned" by spawn slot.
    pub ancient_spawned: [bool; 3],
    /// Stored statue modes by `STATUES` index (+0x34, +0x30, +0x38).
    pub stored_modes: [i32; 3],
    /// Statue respawn wanted by `STATUES` index (+0x3D, +0x3E, +0x3C).
    pub respawn: [bool; 3],
    /// +0x40: Ancients spawned; +0x44: alive.
    pub spawned: i32,
    pub alive: i32,
    /// +0x48: the fight started.
    pub fight_started: bool,
    /// +0x4A: completed before (door 547 open).
    pub done_before: bool,
    /// +0x4C: the altar mode; +0x50: its GUID.
    pub altar_mode: i32,
    pub altar_guid: u32,
    /// +0x54: living players counted on the summit.
    pub living: i32,
    /// +0x58: the summit door was seen; +0x5C its GUID; +0x60 its mode.
    pub door_seen: bool,
    pub door_guid: u32,
    pub door_mode: i32,
}

/// Timers this quest makes (`quests.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
    /// `0x0058BD50` (period 2): the statues' respawn.
    Statues,
}

/// Init `0x0058CE30` beyond `quests.tsv` (part 1 §2): extra zeroed.
pub fn init(r: &mut QuestRecord) {
    r.extra.a5.q5 = Extra::default();
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a5.q5
}

fn xr(ctl: &QuestControl, i: usize) -> &Extra {
    &ctl.records[i].extra.a5.q5
}

/// `STATUES` index of an object class.
fn statue_index(class: u16) -> Option<usize> {
    STATUES.iter().position(|&c| c == class)
}

/// Gate G (§7.1): stat 12 ≥ 20 × (difficulty + 1).
pub fn passes_gate<W: QuestWorld>(w: &W, p: UnitId) -> bool {
    w.stat(p, STAT_LEVEL) >= 20 * (i32::from(w.difficulty()) + 1)
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
        event::NPC_DEACTIVATE => {
            // `0x0058BE80`.
            let qual = args.target.and_then(|n| w.monster_class(n)) == Some(QUAL_KEHK);
            if qual && xr(ctl, i).qual_started {
                late::status_to_all(ctl, w, i, 1);
                x(ctl, i).qual_started = false;
            }
        }
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::MONSTER_KILLED => killed(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            // `0x0058C750`.
            if let Some(p) = args.player {
                late::leave(ctl, w, i, p);
            }
        }
        event::SCROLL_MESSAGE => messages(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => {
            // `0x0058CDA0`: plain status writes.
            let Some(p) = args.player else { return true };
            let f = flags(w, p);
            if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::COMPLETED_BEFORE) {
                x(ctl, i).done_before = true;
            } else if f.get(SLOT, bit::LEAVE_TOWN) {
                late::status_silent(ctl, i, 1);
                late::set_state(ctl, i, 3);
            } else if f.get(SLOT, bit::STARTED) {
                late::status_silent(ctl, i, 1);
                late::set_state(ctl, i, 2);
            }
        }
        _ => return false,
    }
    true
}

/// §7.2 flag iterate `0x0058C210`, for every player.
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let state = ctl.records[i].state;
    for p in w.players() {
        if flags(w, p).get(SLOT, bit::REWARD_GRANTED) || !passes_gate(w, p) {
            continue;
        }
        match state {
            2 => set(w, p, SLOT, bit::STARTED),
            3 => set(w, p, SLOT, bit::LEAVE_TOWN),
            _ => {}
        }
    }
}

/// §7.3 event 0 `0x0058C440`.
fn chat<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    mut list: Option<&mut TextList>,
) {
    if !ctl.records[i].not_intro {
        return;
    }
    let (Some(p), Some(n)) = (args.player, args.target) else {
        return;
    };
    let Some(class) = w.monster_class(n) else {
        return;
    };
    let f = flags(w, p);
    if !f.get(SLOT, bit::REWARD_GRANTED) {
        if STATUE_NPCS.contains(&class) && !xr(ctl, i).altar_used {
            late::add_state(ctl, i, list.as_deref_mut(), class, 4);
        }
        // −1 or past the table → nothing.
        if let Some(k) = late::table_state(&MSG_STATE, ctl.records[i].state) {
            late::add_state(ctl, i, list, class, k);
        }
        return;
    }
    let Some(&(_, _, b)) = POST_QUEST.iter().find(|e| e.1 == class) else {
        return;
    };
    if !f.get(SLOT, b) {
        late::add_state(ctl, i, list, class, 5);
    } else if f.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        late::add_state(ctl, i, list, class, 3);
    }
}

/// §7.3 active function `0x0058CCF0`.
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let _ = f;
    npc_class == QUAL_KEHK
        && ctl.records[i].state == 1
        && !flags(w, player).get(SLOT, bit::REWARD_GRANTED)
}

/// §7.4 event 11 `0x0058C290`.
fn messages<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    match args.b {
        MSG_START => {
            if args.a != u32::from(QUAL_KEHK) {
                return;
            }
            if ctl.records[i].not_intro && ctl.records[i].state == 1 {
                late::set_state(ctl, i, 2);
                if ctl.records[i].status != 1 {
                    late::status_to_all(ctl, w, i, 1);
                }
                flag_iterate(ctl, w, i);
                x(ctl, i).qual_started = true;
            }
        }
        b if b == u32::from(MSG_ALTAR) => {
            let r = &ctl.records[i];
            if r.not_intro && r.state < 4 && r.status != 3 {
                late::status_to_all(ctl, w, i, 3);
            }
            let e = xr(ctl, i);
            if e.portals <= 0 && e.stored_modes.iter().all(|&m| m == 0) && arm(ctl, w, i) {
                let e = x(ctl, i);
                e.altar_used = true;
                e.armed = true;
            }
        }
        b if b == u32::from(MSG_INVISIBLE) => {
            if let Some(p) = args.player {
                set(w, p, SLOT, 4);
            }
        }
        b => {
            // `0x00543520`: the current record.
            let hit = POST_QUEST
                .iter()
                .find(|e| e.0 == b && u32::from(e.1) == args.a);
            if let (Some(&(_, _, bit)), Some(p)) = (hit, args.player) {
                set(w, p, SLOT, bit);
            }
        }
    }
}

/// §7.5 event 3 `0x0058CBA0`.
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a == HARROGATH {
        late::quick_remove(ctl, w, i, p);
        if !ctl.records[i].not_intro {
            return;
        }
        if ctl.records[i].state == 2 && !flags(w, p).get(SLOT, bit::REWARD_GRANTED) {
            late::set_state(ctl, i, 3);
            if ctl.records[i].status != 1 {
                late::status_to_all(ctl, w, i, 1);
            }
            flag_iterate(ctl, w, i);
        }
    }
    if args.b == SUMMIT {
        let r = &ctl.records[i];
        let b = r.status < 2 && r.state > 2;
        if b {
            late::status_to_all(ctl, w, i, 2);
        }
        if ctl.records[i].state < 3 {
            late::set_state(ctl, i, 3);
            flag_iterate(ctl, w, i);
        } else if b {
            flag_iterate(ctl, w, i);
        }
        let e = xr(ctl, i);
        if !e.defeated && e.door_seen && e.door_mode == 0 {
            if let Some((door, _)) = w.object_by_guid(e.door_guid) {
                x(ctl, i).door_mode = 2;
                w.set_object_mode(door, 1);
            }
        }
    }
}

/// §7.6 arm `0x0058BF40`: false when a statue GUID does not resolve.
fn arm<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    let mut units = Vec::with_capacity(3);
    for class in ORDER {
        let k = statue_index(class).expect("statue class");
        match w.object_by_guid(xr(ctl, i).statue_guids[k]) {
            Some((o, _)) => units.push((k, o)),
            None => return false,
        }
    }
    for &(_, o) in &units {
        late::unit_room_portal(w, o, false);
    }
    let at = w.frame() + w.tick_rate().ticks(ARM_DELAY);
    for &(k, o) in &units {
        w.set_object_mode(o, 3);
        w.free_object_collision(o);
        w.schedule_quest_event(o, at);
        x(ctl, i).stored_modes[k] = 3;
    }
    let e = x(ctl, i);
    e.spawned = 0;
    e.alive = 0;
    true
}

/// `0x0058C0E0`: object event 7 of an Ancient statue (classes 474–476,
/// §7.6): release its Ancient.
pub fn statue_event<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let class = w.object_by_guid(w.guid(object)).map(|o| o.1);
    let Some(k) = class.and_then(statue_index) else {
        return;
    };
    if w.object_mode(object) == 4 || xr(ctl, i).spawned >= 3 {
        return;
    }
    // Ids from datatables +0xB36..+0xB3A: 474 → 45, 475 → 43, 476 → 44.
    let superunique = [45, 43, 44][k];
    if xr(ctl, i).portals > 0 {
        reset(ctl, w, i);
        return;
    }
    let Some(m) = w.spawn_superunique_at_unit(object, superunique) else {
        let at = w.frame() + w.tick_rate().ticks(SPAWN_RETRY);
        w.schedule_quest_event(object, at);
        return;
    };
    w.set_object_mode(object, 4);
    let g = w.guid(m);
    let e = x(ctl, i);
    e.stored_modes[k] = 4;
    let slot = e.spawned as usize;
    if let (Some(s), Some(gs)) = (
        e.ancient_spawned.get_mut(slot),
        e.ancient_guids.get_mut(slot),
    ) {
        *s = true;
        *gs = g;
    }
    e.spawned += 1;
    e.alive += 1;
    e.fight_started = true;
}

/// §7.6 event 8 `0x0058C9A0`: an Ancient dies.
fn killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let e = xr(ctl, i);
    if !e.armed || !e.altar_used || e.portals > 0 {
        return;
    }
    let Some(victim) = args.target else { return };
    if !e.timer {
        x(ctl, i).timer = true;
        let t = TimerFn::Act5(super::Timer::Q5(Timer::Statues));
        if let Err(err) = ctl.add_timer(CHAIN, t, TIMER_PERIOD) {
            ctl.faults.push(err);
        }
    }
    // 540 → statue 475, 541 → 476, 542 → 474.
    let statue = match w.monster_class(victim) {
        Some(540) => 475,
        Some(541) => 476,
        Some(542) => 474,
        _ => return,
    };
    let k = statue_index(statue).expect("statue class");
    x(ctl, i).respawn[k] = true;
    // `0x0058C8D0` looks the statue up by GUID first; none → nothing
    // (`quests-helpers.md` §4.1 step 1).
    if let Some((s, _)) = w.object_by_guid(xr(ctl, i).statue_guids[k]) {
        w.quest_missile(victim, s, MISSILE, MISSILE_FLAGS, 1);
    }
    ctl.unique_event(w, FX_ANCIENT);
    x(ctl, i).alive -= 1;
    if xr(ctl, i).alive != 0 {
        return;
    }
    x(ctl, i).defeated = true;
    // The steps after "then with not-intro:" are read as one block.
    if !ctl.records[i].not_intro {
        return;
    }
    if let Some(k) = args.player {
        if !flags(w, k).get(SLOT, bit::REWARD_GRANTED) && passes_gate(w, k) {
            set(w, k, SLOT, bit::REWARD_GRANTED);
            set(w, k, SLOT, bit::PRIMARY_GOAL_DONE);
            experience_reward(w, k);
        }
    }
    // `0x0058C7E0` per player, its party through `0x0058C6D0`.
    for p in w.players() {
        if w.unit_level(p) == Some(SUMMIT)
            && passes_gate(w, p)
            && !flags(w, p).get(SLOT, bit::REWARD_GRANTED)
        {
            set(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
            set(w, p, SLOT, bit::REWARD_GRANTED);
            experience_reward(w, p);
        }
        // Edge case 4: members anywhere in Act V.
        for m in late::party_of(w, p) {
            if !flags(w, m).get(SLOT, bit::REWARD_GRANTED)
                && passes_gate(w, m)
                && late::in_act(w, m, 4)
            {
                set(w, m, SLOT, bit::REWARD_GRANTED);
                set(w, m, SLOT, bit::PRIMARY_GOAL_DONE);
                experience_reward(w, m);
            }
        }
    }
    // `0x0058C780`.
    late::completion_flag(
        w,
        CHAIN,
        SLOT,
        &[bit::REWARD_GRANTED, bit::PRIMARY_GOAL_DONE],
    );
    late::set_state(ctl, i, 5);
    w.create_object_at(victim, INVISIBLE_ANCIENT, 1);
    if let Some(j) = ctl.find(36) {
        super::q6::sequence(ctl, w, j);
    }
    if ctl.records[i].status != 13 {
        late::status_to_all(ctl, w, i, 13);
    }
}

/// §7.7 `0x0058C5C0`: the experience reward, at most one level's span.
pub fn experience_reward<W: QuestWorld>(w: &mut W, p: UnitId) {
    late::send_flags(w, p);
    let d = usize::from(w.difficulty()).min(2);
    let mut a = REWARD[d];
    let mut l = w.stat(p, STAT_LEVEL);
    let m = w.max_level(p);
    if l >= m {
        return;
    }
    let span = w
        .experience_threshold(p, l + 1)
        .wrapping_sub(w.experience_threshold(p, l));
    a = a.min(span);
    while a != 0 && l < m {
        // Experience stats hold u32 values.
        let next = w.stat(p, STAT_NEXT_EXP) as u32;
        let exp = w.stat(p, STAT_EXP) as u32;
        let gap = next.wrapping_sub(exp);
        if a < gap {
            w.add_stat(p, STAT_EXP, a as i32);
            a = 0;
        } else {
            // stat 13 := stat 30, stat 29 := gap (the spec names no
            // setter; the values written are these).
            w.add_stat(p, STAT_EXP, gap as i32);
            let last = w.stat(p, STAT_LAST_EXP);
            w.add_stat(p, STAT_LAST_EXP, (gap as i32).wrapping_sub(last));
            w.level_up(p);
            a -= gap;
            l = w.stat(p, STAT_LEVEL);
        }
    }
}

/// Runs a timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    match t {
        Timer::Statues => {
            let Some(i) = ctl.find(chain) else {
                return true;
            };
            statue_timer(ctl, w, i)
        }
    }
}

/// §7.6 statue timer `0x0058BD50`.
fn statue_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if xr(ctl, i).portals <= 0 && xr(ctl, i).armed {
        for class in ORDER {
            let k = statue_index(class).expect("statue class");
            if !xr(ctl, i).respawn[k] {
                continue;
            }
            let Some((o, _)) = w.object_by_guid(xr(ctl, i).statue_guids[k]) else {
                return false;
            };
            w.set_object_mode(o, 1);
            let at = w.frame() + (w.object_anim_length(o) >> 8);
            w.schedule_object_event(o, 1, at);
            let e = x(ctl, i);
            e.respawn[k] = false;
            e.stored_modes[k] = 2;
        }
    }
    x(ctl, i).timer = false;
    true
}

/// §7.6 reset `0x0058C000`.
fn reset<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    for slot in 0..3 {
        if !xr(ctl, i).ancient_spawned[slot] {
            continue;
        }
        match w.monster_by_guid(xr(ctl, i).ancient_guids[slot]) {
            Some((m, _)) => {
                w.remove_ancient(m);
                let e = x(ctl, i);
                e.spawned -= 1;
                e.alive -= 1;
            }
            None => w.drop_preset_monster(4, ANCIENT_FIRST + slot as u16),
        }
    }
    let e = x(ctl, i);
    e.ancient_spawned = [false; 3];
    e.fight_started = false;
    for class in ORDER {
        let k = statue_index(class).expect("statue class");
        x(ctl, i).respawn[k] = false;
        if let Some((o, _)) = w.object_by_guid(xr(ctl, i).statue_guids[k]) {
            w.set_object_mode(o, 0);
        }
        x(ctl, i).stored_modes[k] = 0;
    }
    if let Some((a, _)) = w.object_by_guid(xr(ctl, i).altar_guid) {
        w.set_object_mode(a, 0);
    }
    x(ctl, i).altar_mode = 0;
}

/// `0x0058CF00` (from `0x005BE389`, `0x005BE393`): a town portal opened
/// in level 120.
pub fn town_portal_opened<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if xr(ctl, i).defeated {
        return;
    }
    let e = x(ctl, i);
    e.portals += 1;
    e.armed = false;
    e.altar_used = false;
    reset(ctl, w, i);
}

/// `0x0058CF50` (from `0x0053548D`, `0x005354D2`, `0x00584C1B`): a town
/// portal of level 120 closed.
pub fn town_portal_closed(ctl: &mut QuestControl) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let e = x(ctl, i);
    e.portals -= 1;
    if e.portals == 0 && e.altar_used {
        e.armed = true;
    }
}

/// `0x0058D560` (from `0x00535141`, `0x0053515A`, `0x0053572C`,
/// `0x005359FC`): a player died; a wiped summit resets the fight (edge
/// case 5).
pub fn player_died<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let e = xr(ctl, i);
    if w.unit_level(player) != Some(SUMMIT)
        || !ctl.records[i].not_intro
        || e.portals > 0
        || e.defeated
        || !e.armed
    {
        return;
    }
    // `0x0058D510`.
    let mut living = 0;
    for p in w.players() {
        if w.unit_level(p) == Some(SUMMIT) {
            let m = w.unit_mode(p);
            if m != MODE_DEATH && m != MODE_DEAD {
                living += 1;
            }
        }
    }
    x(ctl, i).living = living;
    if living == 0 {
        let e = x(ctl, i);
        e.armed = false;
        e.altar_used = false;
        reset(ctl, w, i);
    }
}

/// `0x0058CFB0` (from the Ancients' AI `0x005EEAB1`): the town portals
/// open in level 120.
pub fn portal_count(ctl: &QuestControl) -> i32 {
    ctl.find(CHAIN).map_or(0, |i| xr(ctl, i).portals)
}

/// The fight's armed byte (+0x11), what the Ancients' gate `0x0058CF90`
/// tests (REC-1561).
pub fn armed(ctl: &QuestControl) -> bool {
    ctl.find(CHAIN).is_some_and(|i| xr(ctl, i).armed)
}

/// `0x0058CF90` (from the Ancients' AI `0x005EEB83`, `0x005EEDB7`,
/// `0x005EF027`): armed := 0.
pub fn disarm(ctl: &mut QuestControl) {
    if let Some(i) = ctl.find(CHAIN) {
        x(ctl, i).armed = false;
    }
}

/// `0x0058D090` (`quests.md` §8.2): leaving the summit for 118 or 128 is
/// open unless the record is not-intro and the Ancients live.
pub fn summit_warp_open(ctl: &QuestControl) -> bool {
    ctl.find(CHAIN)
        .is_none_or(|i| !ctl.records[i].not_intro || xr(ctl, i).defeated)
}

/// Whether the altar was operated (+0x4C altar mode 2) or the fight started.
pub fn altar_used(ctl: &QuestControl) -> bool {
    ctl.find(CHAIN)
        .is_some_and(|i| xr(ctl, i).altar_mode == 2 || xr(ctl, i).fight_started)
}

/// Altar init 72 `0x0058D240` (object 546).
pub fn altar_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    x(ctl, i).altar_guid = g;
    w.set_object_mode(object, xr(ctl, i).altar_mode);
}

/// Altar operate 65 `0x0058D310`; returns 0.
pub fn altar_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) -> i32 {
    let Some(i) = ctl.find(CHAIN) else { return 0 };
    if w.object_mode(object) != 0 {
        return 0;
    }
    let (not_intro, state) = (ctl.records[i].not_intro, ctl.records[i].state);
    if !not_intro || state < 4 {
        if xr(ctl, i).portals > 0 {
            close_summit_portals(ctl, w);
        }
        w.open_quest_message(player, object, MSG_ALTAR);
        if not_intro {
            if state < 2 {
                late::set_state(ctl, i, 2);
            }
            if ctl.records[i].status != 3 {
                // Edge case 6: the flags byte is kept.
                ctl.records[i].status = 3;
                late::iterate_all(ctl, w, i);
            }
        }
    }
    w.set_object_mode(object, 1);
    x(ctl, i).altar_mode = 2;
    0
}

/// `0x0058D2C0`: each player's town portal (`0x005353F0`) whose room's
/// level is 120 is closed (`0x00535430`, `quests-helpers.md` §7).
fn close_summit_portals<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    for p in w.players() {
        let Some(g) = w.town_portal_guid(p) else {
            continue;
        };
        let in_summit = w
            .object_by_guid(g)
            .is_some_and(|(o, _)| w.unit_level(o) == Some(SUMMIT));
        if in_summit {
            crate::world::quests::helpers::close_town_portal(ctl, w, p);
        }
    }
}

/// Statue inits 63–65 (`0x0058D150`, `0x0058D190`, `0x0058D110` →
/// `0x0058D0C0`): the GUID is stored and the statue takes its stored
/// mode.
pub fn statue_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId, class: u16) {
    let (Some(i), Some(k)) = (ctl.find(CHAIN), statue_index(class)) else {
        return;
    };
    let g = w.guid(object);
    x(ctl, i).statue_guids[k] = g;
    w.set_object_mode(object, xr(ctl, i).stored_modes[k]);
}

/// Statue operates 62–64 (`0x0058D1E0`, `0x0058D200`, `0x0058D220`):
/// sound 19; returns 0.
pub fn statue_operate<W: QuestWorld>(w: &mut W, player: UnitId) -> i32 {
    w.attach_sound(player, SOUND_REFUSED);
    0
}

/// Door 547 init 73 `0x0058D280`.
pub fn keep_door_init<W: QuestWorld>(ctl: &QuestControl, w: &mut W, object: UnitId) {
    let open = ctl.find(CHAIN).is_some_and(|i| xr(ctl, i).done_before);
    w.set_object_mode(object, if open { 2 } else { 0 });
}

/// Door 547 operate 66 `0x0058D400`; returns 0.
pub fn keep_door_operate<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) -> i32 {
    let Some(i) = ctl.find(CHAIN) else { return 0 };
    let f = flags(w, player);
    let e = xr(ctl, i);
    if (!f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING))
        || (!e.defeated && (ctl.records[i].not_intro || e.fight_started))
    {
        w.attach_sound(player, SOUND_REFUSED);
        return 0;
    }
    match w.object_mode(object) {
        0 => {
            w.set_object_mode(object, 1);
            let at = w.frame() + (w.object_anim_length(object) >> 8);
            w.schedule_object_event(object, 1, at);
            late::unit_room_portal(w, object, false);
        }
        2 => {
            w.object_stairs_warp(player, object);
            late::unit_room_portal(w, object, true);
        }
        _ => {}
    }
    0
}

/// Summit door 564 init 76 `0x0058D640`.
pub fn summit_door_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(object);
    let e = x(ctl, i);
    e.door_seen = true;
    e.door_guid = g;
    w.set_object_mode(object, xr(ctl, i).door_mode);
    let e = x(ctl, i);
    if !e.defeated && e.door_mode != 2 {
        e.door_mode = 2;
        w.set_object_mode(object, 1);
    }
}

/// Summit door 564 operate 71 `0x0058D6A0`; returns 0.
pub fn summit_door_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) -> i32 {
    let Some(i) = ctl.find(CHAIN) else { return 0 };
    let defeated = xr(ctl, i).defeated;
    match (w.object_mode(object), defeated) {
        (0, false) => {
            w.set_object_mode(object, 1);
            x(ctl, i).door_mode = 2;
        }
        // The warp (`0x0058D758`) is `0x0059D9D0`, as door 547's.
        (0, true) => w.object_stairs_warp(player, object),
        (1, false) => {
            w.attach_sound(player, SOUND_REFUSED);
            w.set_object_mode(object, 2);
        }
        (1, true) => {
            w.set_object_mode(object, 0);
            x(ctl, i).door_mode = 0;
        }
        (2, false) => {
            w.attach_sound(player, SOUND_REFUSED);
            w.attach_sound(player, SOUND_REFUSED);
            w.set_object_mode(object, 2);
        }
        (2, true) => {
            if xr(ctl, i).fight_started {
                w.set_object_mode(object, 0);
                x(ctl, i).door_mode = 0;
            }
            w.set_object_mode(object, 0);
            x(ctl, i).door_mode = 0;
        }
        _ => {}
    }
    0
}

/// Object 561 operate 69 `0x0058D5E0`: 39.0 set and 39.4 clear → scroll
/// message 20169. Returns 0.
pub fn invisible_ancient_operate<W: QuestWorld>(w: &mut W, object: UnitId, player: UnitId) -> i32 {
    let f = flags(w, player);
    if f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, 4) {
        w.open_quest_message(player, object, MSG_INVISIBLE);
    }
    0
}

/// The status function: chain 35 has none (the default rule, part 1
/// §2), so this is never reached; reported if it is.
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

#[cfg(test)]
#[path = "q5_tests.rs"]
mod tests;
