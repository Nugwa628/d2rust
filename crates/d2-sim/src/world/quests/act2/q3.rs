// Spec: specs/world/quests-act2.md §5 (A2Q3 Tainted Sun, chain 10, slot 11)
// Spec: specs/world/quests-act2-2.md §1 items 4, 5, 6, 7, 20
//! A2Q3 callback by callback: darken (§5.2), the darkening triggers and
//! the act-load hook (§5.3), the flag iterate (§5.4), chat and its
//! active function (§5.5), messages, game start and leave (§5.6), the
//! altar's operate and init functions (§5.7, §5.8) and both timers.

use super::{
    add_guid, add_state, add_timer, chest_gold, completion_flag, guid_listed, pf, quick_remove,
    rec, remove_guid, set_bit, status_all, status_silent, table_state, Timer, ACT, SOUND_REFUSED,
    TOWN,
};
use crate::units::{RoomId, UnitId};
use crate::world::quests::{bit, event, EventArgs, GuidList, QuestControl, QuestWorld, TextList};

/// Chain id.
pub const CHAIN: u8 = 10;
/// Flag slot.
pub const SLOT: u8 = 11;
/// Lost City and Valley of Snakes: entering either starts the darken
/// delay (§5.3).
const DARKEN_LEVELS: [u32; 2] = [44, 45];
/// Drognan.
const DROGNAN: u16 = 177;
/// Message Drognan's start talk ends with (§5.6).
const MSG_START: u32 = 348;
/// The reward messages (§5.6).
const MSG_REWARD: std::ops::RangeInclusive<u32> = 362..=372;
/// `0x0073A620`: table state by record state 0–5 (§5.5).
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 0];
/// NPCs that want to talk while 11.1 is set (§5.5; fara 178 is missing,
/// edge case 7): atma, warriv2, greiz, elzix, drognan, lysander, cain2,
/// meshif1, geglash, jerhyn.
const REWARD_NPCS: [u16; 10] = [176, 175, 198, 199, 177, 202, 244, 210, 200, 201];
/// Sound attached when the altar is destroyed (§5.7, `0x0059A730`).
const SOUND_TAINTED_SUN: u16 = 52;
/// FX byte of the altar (§5.7).
const FX_ALTAR: u8 = 6;
/// Drop code of the amulets (§5.7).
const VIPER_AMULET: [u8; 4] = *b"vip ";
/// The Horadric Staff (§5.7 step 1).
const HORADRIC_STAFF: [u8; 4] = *b"hst ";
/// Amulet quality (§5.7).
const AMULET_QUALITY: u8 = 7;
/// Altar status timer period (§5.7).
const ALTAR_PERIOD: u32 = 10;
/// S→C 0x53 of the darken (`0x0053C900`, §5.2).
const MSG_DARK: [u8; 10] = [0x53, 0x05, 0, 0, 0, 0, 0, 0, 0, 0x01];
/// S→C 0x53 of the altar (`0x0059A170`, §5.7).
const MSG_LIGHT: [u8; 10] = [0x53, 0x02, 0, 0, 0, 0, 0, 0, 0, 0x00];

/// Extra data (§5.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x01 the darken timer exists.
    pub darken_timer: bool,
    /// +0x02 darkness applied.
    pub dark: bool,
    /// +0x03 darkness pending (Act II not loaded).
    pub dark_pending: bool,
    /// +0x04 the altar is destroyed.
    pub altar_destroyed: bool,
    /// +0x05 the altar was seen.
    pub altar_seen: bool,
    /// +0x06 the status timer.
    pub status_timer: bool,
    /// +0x08 the altar mode (0 = neutral).
    pub altar_mode: i32,
    /// +0x0C the altar's GUID.
    pub altar_guid: u32,
    /// +0x10 the altar's room.
    pub altar_room: Option<RoomId>,
    /// +0x14 player list (only event 10 touches it).
    pub list: GuidList,
    /// +0x98 the altar's level.
    pub altar_level: u32,
    /// +0x9C amulet drop count.
    pub amulets: i32,
}

fn x3(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a2.q3
}

/// Dispatches chain 10's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => chat(ctl, w, i, args, list),
        event::CHANGED_LEVEL => changed_level(ctl, w, i, args),
        event::PLAYER_LEAVES_GAME => {
            // `0x00599FB0`: the record list and the +0x14 list.
            remove_guid(ctl, w, i, args.player);
            let g = super::guid_of(w, args.player);
            x3(ctl, i).list.remove(g);
        }
        event::SCROLL_MESSAGE => messages(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => game_start(ctl, w, i, args),
        _ => return false,
    }
    true
}

// ------------------------------------------------------------ §5.2

/// Darken `0x0059A350` (§5.2): true when the Tainted Sun started (the
/// game has Act II); false sets +0x03 (pending).
fn darken<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    status_all(ctl, w, i, 1);
    if ctl.records[i].state == 0 {
        ctl.records[i].state = 1;
    }
    if !w.has_act2() {
        x3(ctl, i).dark_pending = true;
        return false;
    }
    w.start_tainted_sun(ACT);
    for p in w.players() {
        if w.client_in_act(p, ACT) {
            w.send(p, &MSG_DARK);
            w.send(p, &[0x5D, CHAIN, 0x10, 0, 0, 0]);
        }
    }
    true
}

// ------------------------------------------------------------ §5.3

/// Event 3 `0x0059EE00` (a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    // Three independent tests in this order (`0x0059EE0E`, `0x0059EE37`,
    // `0x0059EE66`): one level change can meet two of them.
    if DARKEN_LEVELS.contains(&args.b) {
        let r = &ctl.records[i];
        if r.state == 0 && r.not_intro && !r.extra.a2.q3.darken_timer {
            // `0x0059EDC0`: `between(quest seed, 15, 17)` (`0x004BC500`).
            let period = ctl.seed.roll_range(15, 17 - 15);
            add_timer(ctl, CHAIN, Timer::Darken, period as u32);
            x3(ctl, i).darken_timer = true;
        }
    }
    if args.b == TOWN && ctl.records[i].extra.a2.q3.dark_pending && darken(ctl, w, i) {
        flag_iterate(ctl, w, i);
        x3(ctl, i).dark_pending = false;
    }
    if args.a == TOWN {
        quick_remove(ctl, w, i, args.player);
        if ctl.records[i].state == 2 {
            ctl.records[i].state = 3;
        }
    }
}

/// Timer `0x0059ED80` (§5.3); true = remove.
pub(super) fn darken_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    x3(ctl, i).darken_timer = false;
    if !ctl.records[i].extra.a2.q3.dark {
        if darken(ctl, w, i) {
            x3(ctl, i).dark = true;
            flag_iterate(ctl, w, i);
        } else {
            x3(ctl, i).dark_pending = true;
        }
    }
    true
}

/// The act-load hook `0x0059AC40(act, n)` (from `0x0053ACB3`, §5.3):
/// with n = 1 and the darkness pending, start the Tainted Sun on `act`.
pub fn act_load<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, act: u8, n: u32) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if n == 1 && ctl.records[i].extra.a2.q3.dark_pending {
        w.start_tainted_sun(act);
        let x = x3(ctl, i);
        x.dark_pending = false;
        x.dark = true;
    }
}

// ------------------------------------------------------------ §5.4

/// Flag iterate `0x0059A4F0` (§5.4).
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let state = ctl.records[i].state;
    for p in w.players() {
        let f = pf(w, p);
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        if state >= 1 {
            set_bit(w, p, SLOT, 2);
        }
        match state {
            2 => set_bit(w, p, SLOT, 3),
            3 => set_bit(w, p, SLOT, 4),
            _ => {}
        }
    }
}

// ------------------------------------------------------------ §5.5

/// Event 0 `0x0059A270`.
fn chat<W: QuestWorld>(
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
    if r.get(SLOT, bit::COMPLETED_NOW) {
        return;
    }
    if guid_listed(ctl, w, i, args.player) {
        return add_state(ctl, w, i, list, args.target, 4);
    }
    let rd = &ctl.records[i];
    if rd.state != 0 && rd.not_intro && !r.get(SLOT, bit::REWARD_GRANTED) && rd.state <= 3 {
        if let Some(m) = table_state(&MSG_STATE, rd.state).filter(|&m| m <= 5) {
            add_state(ctl, w, i, list, args.target, m);
        }
    }
}

/// Active `0x0059A1C0` (§5.5).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc: u16,
) -> bool {
    let f = pf(w, player);
    !f.get(SLOT, bit::REWARD_GRANTED)
        && ((f.get(SLOT, bit::REWARD_PENDING) && REWARD_NPCS.contains(&npc))
            || (npc == DROGNAN && ctl.records[i].state == 1))
}

// ------------------------------------------------------------ §5.6

/// Event 11 `0x0059EBE0` (a = NPC class, b = message).
fn messages<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.b == MSG_START && args.a == u32::from(DROGNAN) {
        if !ctl.records[i].not_intro {
            return;
        }
        if ctl.records[i].status == 1 {
            status_silent(ctl, i, 2);
        }
        if ctl.records[i].state == 1 {
            ctl.records[i].state = 2;
            flag_iterate(ctl, w, i);
        }
        return;
    }
    if !MSG_REWARD.contains(&args.b) {
        return;
    }
    let Some(p) = args.player else { return };
    let r = &ctl.records[i];
    if r.state != 5 && r.not_intro && pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
        status_silent(ctl, i, 13);
        ctl.records[i].state = 5;
        super::sequence(ctl, w, CHAIN);
    }
    if let Some(n) = args.target {
        ctl.refresh_text(w, p, n);
    }
    if pf(w, p).get(SLOT, bit::REWARD_PENDING) {
        set_bit(w, p, SLOT, bit::REWARD_GRANTED);
        super::clear_bit(w, p, SLOT, bit::REWARD_PENDING);
        add_guid(ctl, w, i, p);
        // Inside the 11.1 block (`0x0059ED63`, `quests-act2-2.md` §1
        // item 7).
        if !ctl.records[i].not_intro {
            ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        }
    }
}

/// Event 13 `0x0059A5D0`.
fn game_start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        return;
    }
    if !f.get(SLOT, bit::STARTED) {
        return;
    }
    // `0x0059A570`.
    if darken(ctl, w, i) {
        x3(ctl, i).dark = true;
    } else {
        x3(ctl, i).dark_pending = true;
        w.send(p, &[0x5D, CHAIN, 0x01, 0, 0, 0]);
    }
    // "status n, state m": direct byte stores, flags kept, nothing sent
    // (`quests-act2-2.md` §1 item 6).
    let r = &mut ctl.records[i];
    (r.status, r.state) = if f.get(SLOT, 4) {
        (2, 3)
    } else if f.get(SLOT, 3) {
        (2, 2)
    } else {
        (1, 1)
    };
}

// ------------------------------------------------------------ §5.7

/// Altar operate 24 `0x0059A7E0` (§5.7, objects.txt row 149): `player`
/// operates `object`. Returns the operate result (always 0). It never
/// calls the quest-chest gate `0x00545850` (`quests-act2-2.md` §1 item
/// 4): steps 1–2 are its own guards.
pub fn altar_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) -> u32 {
    let f = pf(w, player);
    let done = f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING);
    if done
        && (f.get(10, bit::REWARD_GRANTED)
            || w.has_item(player, VIPER_AMULET)
            || w.has_item(player, HORADRIC_STAFF))
    {
        w.attach_sound(player, SOUND_REFUSED);
        return 0;
    }
    if w.object_mode(object) != 0 {
        return 0;
    }
    let Some(i) = ctl.find(CHAIN) else { return 0 };
    let level = w.unit_level(object).unwrap_or(0);
    if !ctl.records[i].not_intro {
        // Step 4.
        w.set_object_mode(object, 1);
        let x = x3(ctl, i);
        x.altar_mode = 2;
        x.altar_destroyed = true;
        drop_amulets(ctl, w, i, object);
        w.object_treasure(object, player, 4);
        chest_gold(ctl, w, object);
        return 0;
    }
    // Step 3.
    let room = w.unit_position(object).map(|p| p.2);
    let guid = w.guid(object);
    let x = x3(ctl, i);
    x.altar_room = room;
    x.altar_level = level;
    w.set_object_mode(object, 1);
    let x = x3(ctl, i);
    x.altar_mode = 2;
    x.altar_seen = true;
    x.altar_guid = guid;
    x.altar_destroyed = true;
    if w.has_act2() {
        x3(ctl, i).dark = false;
        w.end_tainted_sun();
        for p in w.players() {
            if w.client_in_act(p, ACT) {
                w.send(p, &MSG_LIGHT);
            }
        }
    }
    ctl.records[i].state = 4;
    ctl.unique_event(w, FX_ALTAR);
    // This player.
    if !pf(w, player).get(SLOT, bit::REWARD_GRANTED) {
        set_bit(w, player, SLOT, bit::REWARD_PENDING);
        set_bit(w, player, SLOT, bit::PRIMARY_GOAL_DONE);
        super::clear_bit(w, player, SLOT, bit::COMPLETED_NOW);
    }
    // `0x0059A680`: players on the altar's level.
    for p in w.players() {
        let f = pf(w, p);
        if w.unit_level(p) == Some(level)
            && !f.get(SLOT, bit::REWARD_GRANTED)
            && !f.get(SLOT, bit::COMPLETED_BEFORE)
        {
            set_bit(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
            set_bit(w, p, SLOT, bit::REWARD_PENDING);
        }
    }
    // `0x0059A0B0`: the party members of every player with 11.13.
    for p in w.players() {
        if !pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            continue;
        }
        for m in super::party(w, p) {
            let f = pf(w, m);
            if super::in_act2(w, m)
                && !f.get(SLOT, bit::REWARD_GRANTED)
                && !f.get(SLOT, bit::REWARD_PENDING)
            {
                set_bit(w, m, SLOT, bit::PRIMARY_GOAL_DONE);
                set_bit(w, m, SLOT, bit::REWARD_PENDING);
            }
        }
    }
    // `0x0059A730`.
    for p in w.players() {
        if pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) {
            w.attach_sound(p, SOUND_TAINTED_SUN);
        }
    }
    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
    drop_amulets(ctl, w, i, object);
    w.object_treasure(object, player, 4);
    chest_gold(ctl, w, object);
    // `0x00599FE0`.
    completion_flag(w, CHAIN, SLOT);
    add_timer(ctl, CHAIN, Timer::AltarStatus, ALTAR_PERIOD);
    0
}

/// §5.7: amulet count := the players (`0x0059A770`) holding neither
/// `vip ` nor `hst ` and lacking 10.0; that many `vip ` drops of quality
/// 7 (the item level is computed by `0x00559A30`, edge case 8); the
/// created ones identified and added to chain 9's amulet count.
fn drop_amulets<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, object: UnitId) {
    let mut n = 0;
    for p in w.players() {
        if !w.has_item(p, VIPER_AMULET)
            && !w.has_item(p, HORADRIC_STAFF)
            && !pf(w, p).get(10, bit::REWARD_GRANTED)
        {
            n += 1;
        }
    }
    x3(ctl, i).amulets = n;
    let mut made = 0;
    for _ in 0..n {
        // The level variable holds the altar's level id, but `&level` is
        // an out parameter that `0x00559A30` overwrites unread
        // (`quests-act2-2.md` §1 item 20): passed as `None`.
        if let Some(item) = w.quest_drop(object, VIPER_AMULET, AMULET_QUALITY, None, false) {
            w.identify_item(item);
            made += 1;
        }
    }
    if let Some(r) = ctl.record_mut(9) {
        r.extra.a2.q2.amulet_count += made;
    }
}

/// Timer `0x0059A700` (§5.7); true = remove.
pub(super) fn altar_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if ctl.records[i].state == 4 {
        status_all(ctl, w, i, 3);
    }
    x3(ctl, i).status_timer = false;
    true
}

// ------------------------------------------------------------ §5.8

/// Altar init 20 (`0x00544910` → `0x0059A3F0`, §5.8).
pub fn altar_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else {
        if w.object_mode(object) != 2 {
            w.set_object_mode(object, 2);
        }
        return;
    };
    let guid = w.guid(object);
    let x = x3(ctl, i);
    x.altar_seen = true;
    x.altar_guid = guid;
    if ctl.records[i].not_intro {
        if ctl.records[i].state <= 1 {
            if darken(ctl, w, i) {
                x3(ctl, i).dark = true;
            } else {
                x3(ctl, i).dark_pending = true;
            }
            // A direct byte store (`0x0059A434`).
            ctl.records[i].state = 3;
            ctl.records[i].flags = 0;
        }
        // Still inside not-intro, also when the state was ≥ 2
        // (`0x0059A455`); never true in 1.14d: every path that moves the
        // state off 0 writes status ≥ 1 first (`quests-act2-2.md` §1
        // item 5). Kept as the original has it.
        if ctl.records[i].status == 0 {
            status_all(ctl, w, i, 2);
        }
    }
    let mode = ctl.records[i].extra.a2.q3.altar_mode;
    w.set_object_mode(object, mode);
}
