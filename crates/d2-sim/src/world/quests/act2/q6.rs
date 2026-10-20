// Spec: specs/world/quests-act2.md §8 (A2Q6 The Seven Tombs, chain 13, slot 14), §10
// Spec: specs/world/quests-act2-2.md §1 items 6, 9, 10, 11, 12, 17, 19; §3 (orifice, C→S 0x44, S→C 0x58)
//! A2Q6 callback by callback: the true-tomb choice (§8.1), chat and its
//! wants-to-talk test (§8.3), level changes (§8.4), the status function
//! (§8.5), the orifice and the staff hand-in (§8.6, §8.7), the lair
//! objects (§8.8), the Arcane Sanctuary dummy objects (§8.9), messages,
//! Duriel's kill, chat end, game start / join and the portal check
//! (§8.11), and the chain-13 hooks of §10. The clue item 0x50 (§8.10) is
//! `quests::true_tomb_clue`. The staff tomb level (+0x34) is the shared
//! `act1::Extra::tomb_level`.

use super::{
    add_guid, add_state, add_timer, call_seq, guid_listed, in_act2, party, pf, quick_remove, rec,
    send_flags, set_bit, status_all, status_silent, table_state, Timer, ACT, SOUND_REFUSED, TOWN,
};
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};
use crate::world::quests::{
    bit, event, flags_of, npc, raise_progression, EventArgs, QuestControl, QuestError, QuestFlags,
    QuestWorld, TextList,
};

const CHAIN: u8 = 13;
const SLOT: u8 = 14;
/// Slot of A2Q2 (the Horadric Staff) and of A1Q7 (Act I done).
const STAFF_SLOT: u8 = 10;
const ACT1_DONE_SLOT: u8 = 7;
/// Slot of A2Q4 (game 12.13 gates Drognan's table state 1).
const ARCANE_SLOT: u8 = 12;

/// NPC classes (`monstats.txt` hcIdx).
pub const WARRIV2: u16 = 175;
pub const ATMA: u16 = 176;
pub const DROGNAN: u16 = 177;
pub const FARA: u16 = 178;
pub const JERHYN: u16 = 201;
pub const LYSANDER: u16 = 202;
pub const CAIN2: u16 = 244;
pub const TYRAEL1: u16 = 251;

/// §8.3: the six NPCs of the "talk to everyone" round (with 14.13) and
/// the slot-14 bit each one's message sets (§8.11).
pub const TOWNSFOLK: [(u16, u8); 6] = [
    (ATMA, 6),
    (WARRIV2, 7),
    (DROGNAN, 8),
    (LYSANDER, 9),
    (CAIN2, 10),
    (FARA, 11),
];

/// §8.11 "any NPC": message index → slot-14 bit.
const TOWNSFOLK_MSGS: [(u32, u8); 6] =
    [(444, 9), (445, 6), (446, 7), (447, 11), (449, 8), (452, 10)];

/// `0x0073B8A8`: table state by record state 0–5.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];

/// Levels: Canyon of the Magi, the first tomb, Duriel's Lair.
pub const CANYON: u32 = 46;
pub const FIRST_TOMB: u32 = 66;
pub const DURIEL_LAIR: u32 = 73;

/// Objects: the town portal Tyrael opens, the lair entrance.
const PORTAL: u16 = 59;
const LAIR_ENTRANCE: u16 = 100;
/// The orifice (`objects.txt` row 152).
pub const ORIFICE: u16 = 152;

/// FX bytes (§1.1): orifice, Duriel.
const FX_ORIFICE: u8 = 3;
const FX_DURIEL: u8 = 8;

/// `missiles.txt` row of `horadricstaff` (its Range sets the lair timer).
pub const STAFF_MISSILE_ROW: u32 = 338;

/// `0x00738FAC`: Arcane Sanctuary dummy objects for tombs 66–72.
pub const ARCANE_BASE: [u16; 7] = [313, 312, 308, 310, 311, 309, 307];

/// The progression step of `0x00538680(client, 2, difficulty)` (Tyrael's
/// portal, `quests-act1-rest.md` §5).
const PROGRESSION_STEP: u8 = 2;

/// Extra data (§8.2). The staff tomb level (+0x34) is the shared
/// `act1::Extra::tomb_level`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00 status timer (cleared by `0x0059CEE0`).
    pub status_timer: bool,
    /// +0x01 Duriel killed.
    pub duriel_killed: bool,
    /// +0x03 the lair-object timer is pending.
    pub timer_active: bool,
    /// +0x04 the orifice was initialised.
    pub orifice_seen: bool,
    /// +0x05 Tyrael's door was initialised.
    pub door_seen: bool,
    /// +0x06 the init-37 object was initialised.
    pub init37_seen: bool,
    /// +0x08 chat end pending for Jerhyn's start talk (message 430).
    pub chat_start: bool,
    /// +0x09 chat end pending for Tyrael (message 302).
    pub chat_tyrael: bool,
    /// +0x0A chat end pending for Jerhyn's end talk (message 442).
    pub chat_end: bool,
    /// +0x0B the lair entrance is open.
    pub lair_open: bool,
    /// +0x0C the staff was already handed in (10.0 at game start).
    pub staff_in: bool,
    /// +0x0D the lair objects need an update.
    pub objects_update: bool,
    /// +0x0E the staff items were removed.
    pub staff_removed: bool,
    /// +0x0F the portal to Lut Gholein was opened.
    pub portal_opened: bool,
    /// +0x10 the staff is missing.
    pub missing: bool,
    /// +0x14 the missing status (8 or 9, §4.10).
    pub missing_status: u8,
    /// +0x18 Tyrael's door mode.
    pub door_mode: i32,
    /// +0x20 the orifice's GUID.
    pub orifice_guid: u32,
    /// +0x28 the init-37 object's GUID.
    pub init37_guid: u32,
    /// +0x2C Tyrael's door's GUID.
    pub door_guid: u32,
    /// +0x38 the arcane list was made.
    pub arcane_made: bool,
    /// +0x3A the next arcane list index.
    pub arcane_next: u16,
    /// +0x48 the six arcane object ids.
    pub arcane: [u16; 6],
    /// +0x3C Tyrael's portal is being opened (portal check).
    pub portal_opening: bool,
    /// +0x3D the quest was completed before (14.0 or 14.15 at start).
    pub completed_before: bool,
    /// +0x60 Duriel's room.
    pub duriel_room: Option<RoomId>,
}

fn x(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a2.q6
}

fn xr(ctl: &QuestControl, i: usize) -> &Extra {
    &ctl.records[i].extra.a2.q6
}

/// "callback 2 := `0x0059C760`": event 2 now routes to [`chat_end`].
fn arm_chat_end(ctl: &mut QuestControl, i: usize) {
    ctl.records[i].callbacks |= 1 << event::NPC_DEACTIVATE;
}

// ------------------------------------------------------------ §8.1

/// §8.1 (`0x00642DA0`, the call site is the DRLG spec's): the true tomb
/// pair from the DRLG seed. Two draws per try (`lo' mod 7` each) until
/// they differ. Returns (staff tomb level, Duriel tomb level), i.e.
/// drlg +0x94 and +0x484.
pub fn true_tombs(seed: &mut Seed) -> (u32, u32) {
    loop {
        let staff = seed.step() % 7;
        let boss = seed.step() % 7;
        if staff != boss {
            return (FIRST_TOMB + staff, FIRST_TOMB + boss);
        }
    }
}

// ------------------------------------------------------------ dispatch

/// Dispatches chain 13's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
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
        event::MONSTER_KILLED => duriel_killed(ctl, w, i, args),
        // `0x0059C6B0`: event 10 removes the player (§1.1, `0x00545530`).
        event::PLAYER_LEAVES_GAME => super::remove_guid(ctl, w, i, args.player),
        event::SCROLL_MESSAGE => messages(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => start(ctl, w, i, args.player),
        event::PLAYER_JOINED_GAME => {
            // `0x0059D4D0`: clear 10.9.
            if let Some(p) = args.player {
                super::clear_bit(w, p, STAFF_SLOT, 9);
            }
        }
        _ => return false,
    }
    true
}

// ------------------------------------------------------------ §8.3

/// Event 0 `0x0059C3C0`.
fn chat<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let c = args.target.and_then(|n| w.monster_class(n));
    let r = rec(w, args.player);
    if c == Some(TYRAEL1) {
        // Table state 2 when the door mode is 2; always returns
        // (`quests-act2-2.md` §1 item 9).
        if xr(ctl, i).door_mode == 2 {
            add_state(ctl, w, i, list, args.target, 2);
        }
        return;
    }
    let done = r.get(SLOT, bit::PRIMARY_GOAL_DONE);
    if done {
        if let Some(&(_, b)) = TOWNSFOLK.iter().find(|t| Some(t.0) == c) {
            if !r.get(SLOT, b) {
                return add_state(ctl, w, i, list, args.target, 6);
            }
        }
    }
    if r.get(SLOT, bit::LEAVE_TOWN) {
        return add_state(ctl, w, i, list, args.target, 3);
    }
    if r.get(SLOT, bit::ENTER_AREA) {
        let k = if c == Some(npc::MESHIF1) { 5 } else { 4 };
        return add_state(ctl, w, i, list, args.target, k);
    }
    if guid_listed(ctl, w, i, args.player) {
        return add_state(ctl, w, i, list, args.target, 4);
    }
    let state = ctl.records[i].state;
    if state == 0 || (r.get(SLOT, bit::REWARD_GRANTED) && !done) || (state > 3 && !done) {
        return;
    }
    let Some(m) = table_state(&MSG_STATE, state) else {
        return;
    };
    if m > 7 || (m == 1 && c == Some(DROGNAN) && ctl.game.get(ARCANE_SLOT, bit::PRIMARY_GOAL_DONE))
    {
        return;
    }
    add_state(ctl, w, i, list, args.target, m);
}

/// Active function `0x0059D300` (wants to talk).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc: u16,
) -> bool {
    let f = pf(w, player);
    let r = &ctl.records[i];
    let x = &r.extra.a2.q6;
    match npc {
        JERHYN => {
            !f.get(SLOT, bit::REWARD_GRANTED)
                && ((r.state == 1
                    && !f.get(SLOT, bit::LEAVE_TOWN)
                    && !f.get(SLOT, bit::ENTER_AREA))
                    || f.get(SLOT, bit::LEAVE_TOWN))
        }
        npc::MESHIF1 => f.get(SLOT, bit::ENTER_AREA),
        TYRAEL1 => r.not_intro && x.duriel_killed && !x.portal_opened,
        _ => {
            f.get(SLOT, bit::PRIMARY_GOAL_DONE)
                && TOWNSFOLK.iter().any(|&(c, b)| c == npc && !f.get(SLOT, b))
        }
    }
}

// ------------------------------------------------------------ §8.4

/// The flag iterate (`0x0059C6C0` for one player): unless 14.0, state > 1
/// → set 14.2.
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize, p: UnitId) {
    let state = ctl.records[i].state;
    if let Some(f) = flags_of(w, p) {
        if !f.get(SLOT, bit::REWARD_GRANTED) && state > 1 {
            f.set(SLOT, bit::STARTED);
        }
    }
}

/// The flag iterate for every player (`0x0059C710`; `0x0059C750` from
/// Tyrael's AI, §10).
fn flag_iterate_all<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    for p in w.players() {
        flag_iterate(ctl, w, i, p);
    }
}

/// Event 3 `0x0059D1C0` (a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let new = args.b;
    if !(TOWN..=74).contains(&new) {
        return;
    }
    if args.a == TOWN {
        quick_remove(ctl, w, i, args.player);
    }
    if !ctl.records[i].not_intro {
        return;
    }
    let iterate = |ctl: &QuestControl, w: &mut W| {
        if let Some(p) = args.player {
            flag_iterate(ctl, w, i, p);
        }
    };
    if !xr(ctl, i).duriel_killed {
        let mut tomb = ctl.records[i].extra.tomb_level;
        if tomb == 0 {
            tomb = w.true_tomb_level();
            ctl.records[i].extra.tomb_level = tomb;
        }
        if tomb == 0 {
            return;
        }
        if new == tomb {
            if ctl.records[i].state == 2 {
                return;
            }
            ctl.records[i].state = 2;
            if ctl.records[i].status <= 1 {
                status_silent(ctl, i, 2);
            }
            iterate(ctl, w);
            return;
        }
        if new == CANYON {
            if ctl.records[i].state == 0 {
                ctl.records[i].state = 2;
                iterate(ctl, w);
            }
            if ctl.records[i].status == 0 {
                status_all(ctl, w, i, 1);
            }
            return;
        }
    }
    if new == DURIEL_LAIR && ctl.records[i].state <= 1 {
        ctl.records[i].state = 2;
        if ctl.records[i].status > 1 {
            return;
        }
        status_silent(ctl, i, 2);
        iterate(ctl, w);
    }
}

// ------------------------------------------------------------ §8.5

/// Status function `0x0059CA50` (§8.5; always true).
pub(super) fn status<W: QuestWorld>(
    ctl: &QuestControl,
    _w: &mut W,
    i: usize,
    _player: UnitId,
    pf: &QuestFlags,
) -> u8 {
    let r = &ctl.records[i];
    let x = &r.extra.a2.q6;
    if !pf.get(ACT1_DONE_SLOT, bit::REWARD_GRANTED) {
        0
    } else if x.missing {
        x.missing_status
    } else if pf.get(SLOT, bit::REWARD_GRANTED) {
        0
    } else if pf.get(SLOT, bit::LEAVE_TOWN) {
        5
    } else if pf.get(SLOT, bit::ENTER_AREA) {
        6
    } else if !r.not_intro {
        0
    } else if pf.get(SLOT, bit::CUSTOM1) || r.state < 3 {
        r.status
    } else {
        12
    }
}

// ------------------------------------------------------------ §8.6

/// S→C 0x58 (`0x0053D8D0`, `quests-act2-2.md` §3.3): u8 0x58, u32 object
/// GUID, u8 result, u8 "accepted with effect". Byte 6 is not written by
/// the original for results 0, 1 and 4 (stale stack data, open question
/// 1 there): D2RUST sends 0.
fn send_insert<W: QuestWorld + ?Sized>(
    w: &mut W,
    player: UnitId,
    guid: u32,
    result: u8,
    effect: u8,
) {
    let g = guid.to_le_bytes();
    w.send(player, &[0x58, g[0], g[1], g[2], g[3], result, effect]);
}

/// Orifice operate 25 (`0x0059DC70`, `quests-act2-2.md` §3.1). Returns
/// the operate function's result (0 or 1).
pub fn orifice_operate_checked<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) -> u32 {
    if ctl.find(CHAIN).is_none() {
        return 0;
    }
    orifice_operate(w, object, player)
}

/// The orifice operate after its chain-13 test (`0x0059DC70` past the
/// record lookup; [`orifice_operate_checked`] is the whole function).
pub fn orifice_operate<W: QuestWorld>(w: &mut W, object: UnitId, player: UnitId) -> u32 {
    let guid = w.guid(object);
    match w.object_mode(object) {
        0 => {
            if w.player_busy(player) {
                // Busy: return 1, no sound, no mode change, no 0x58
                // (`0x0059DCFC`, `quests-act2-2.md` §1 item 19).
                return 1;
            }
            if w.has_item(player, *b"hst ") {
                w.set_interact_unit(player, Some((2, guid)));
                w.set_object_mode(object, 1);
                send_insert(w, player, guid, 0, 0);
                0
            } else {
                w.attach_sound(player, SOUND_REFUSED);
                1
            }
        }
        1 if w.interact_unit(player) == Some((2, guid)) => {
            w.set_interact_unit(player, None);
            w.set_object_mode(object, 2);
            0
        }
        // Any other mode: return 0 (§3.1).
        _ => 0,
    }
}

/// C→S 0x44 action 2 (cancel).
pub const INSERT_CANCEL: u16 = 2;
/// C→S 0x44 action 3 (insert the cursor item).
pub const INSERT_ITEM: u16 = 3;

/// `0x005852E0` action 2 (cancel, `quests-act2-2.md` §3.2 step 3) on an
/// existing object: S→C 0x58 result 1, object mode 0, the interact unit
/// reset. Also the obelisk close of the interaction end
/// (`quests-helpers.md` §5 kind 2).
pub fn insert_cancel<W: QuestWorld + ?Sized>(
    w: &mut W,
    player: UnitId,
    object: UnitId,
    object_guid: u32,
) {
    send_insert(w, player, object_guid, 1, 0);
    w.set_object_mode(object, 0);
    w.set_interact_unit(player, None);
}

/// C→S 0x44 past its size / busy / `0x00549520` checks: `0x005852E0(game,
/// player, object GUID, item, action)` (`quests-act2-2.md` §3.2 steps
/// 2–5). `item` is the cursor item (type 4 by its GUID). An object that
/// does not exist: nothing. Insert into an object other than the orifice
/// (step 5: `0x0055EEA0`, `0x00585240`) is the object spec's and is
/// reported.
pub fn item_to_object<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    player: UnitId,
    object_guid: u32,
    item: Option<UnitId>,
    action: u16,
) {
    let Some((object, class)) = w.object_by_guid(object_guid) else {
        return;
    };
    // The gate `0x00549520` (`quests-act2-2.md` §3.2 step 1): an insert
    // whose item is not the player's cursor item (`0x005490E0`) is
    // refused before `0x005852E0`: no 0x58 (check `w2-14-orifice-0x58`).
    if action == INSERT_ITEM && !item.is_some_and(|t| w.is_cursor_item(player, t)) {
        return;
    }
    match action {
        INSERT_CANCEL => insert_cancel(w, player, object, object_guid),
        INSERT_ITEM if class == ORIFICE => {
            if item.and_then(|t| w.item_code(t)) != Some(*b"hst ") {
                send_insert(w, player, object_guid, 4, 0);
                return;
            }
            // No `0x0055EEA0` for the orifice: the staff stays on the
            // cursor until the hand-in deletes it.
            w.set_interact_unit(player, None);
            send_insert(w, player, object_guid, 5, 1);
            w.set_object_mode(object, 1);
            w.set_object_mode(object, 2);
            hand_in(ctl, w, player, object);
        }
        INSERT_ITEM => w.unhandled(CHAIN, 0x0058_52E0),
        _ => {}
    }
}

// ------------------------------------------------------------ §8.7

/// `0x0059DBD0` / the player's own part: 10.0, 10.13 and the three
/// staff items deleted.
fn staff_done<W: QuestWorld>(w: &mut W, p: UnitId, delete: bool) {
    set_bit(w, p, STAFF_SLOT, bit::REWARD_GRANTED);
    set_bit(w, p, STAFF_SLOT, bit::PRIMARY_GOAL_DONE);
    if delete {
        for code in [*b"hst ", *b"vip ", *b"msf "] {
            w.delete_item(p, code);
        }
    }
}

/// Handing in the staff (`0x0059DD80`, §8.7). `object` is the orifice.
pub fn hand_in<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId, object: UnitId) {
    staff_done(w, player, true);
    for m in party(w, player) {
        if !in_act2(w, m) || pf(w, m).get(STAFF_SLOT, bit::REWARD_GRANTED) {
            continue;
        }
        let trading = w.is_trading(m);
        staff_done(w, m, !trading);
    }
    ctl.unique_event(w, FX_ORIFICE);
    // Chain 13's record is read without a null test after the bits, the
    // deletions, the party step and the FX (`0x0059DE53`): absent is a
    // null read (unreachable: every record exists from game start).
    let Some(i) = ctl.find(CHAIN) else {
        return ctl.faults.push(QuestError::Fatal(0x0059_DE53));
    };
    let e = x(ctl, i);
    e.objects_update = true;
    e.staff_removed = true;
    if !e.timer_active {
        match w.missile_range(STAFF_MISSILE_ROW) {
            Some(r) => {
                // Signed division; the original passes the int as the
                // period.
                let period = (r.wrapping_sub(75) / 20) as u32;
                add_timer(ctl, CHAIN, Timer::LairObjects, period);
                x(ctl, i).timer_active = true;
            }
            // Edge case 10: a table without row 338 reads a null row.
            None => ctl.faults.push(QuestError::Fatal(0x0059_DD80)),
        }
    }
    w.set_object_mode(object, 1);
    if let Some((ox, oy, room)) = w.unit_position(object) {
        w.set_room_portal(room, false);
        if let Some(r2) = w.room_at(room, ox, oy + 3) {
            w.set_room_portal(r2, false);
        }
    }
}

// ------------------------------------------------------------ §8.8

/// Mode 1 plus the end-animation event (object event 1).
fn animate<W: QuestWorld>(w: &mut W, o: UnitId) {
    w.set_object_mode(o, 1);
    let at = w.frame() + (w.object_anim_length(o) >> 8);
    w.schedule_object_event(o, 1, at);
}

/// Timer `0x0059D870` (§8.8); true = remove.
pub(super) fn lair_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if xr(ctl, i).objects_update {
        let e = xr(ctl, i).clone();
        let mut a = false;
        if e.init37_seen {
            if let Some((o, _)) = w.object_by_guid(e.init37_guid) {
                if w.object_mode(o) == 0 {
                    a = true;
                    if e.staff_in {
                        w.set_object_mode(o, 2);
                    } else {
                        animate(w, o);
                    }
                }
            }
        }
        let mut b = false;
        if e.orifice_seen {
            if let Some((o, _)) = w.object_by_guid(e.orifice_guid) {
                if let Some((ox, oy, room)) = w.unit_position(o) {
                    let (lx, ly) = (ox - 13, oy + 3);
                    if let Some(r2) = w.room_at(room, lx, ly) {
                        if let Some(lair) = w.spawn_quest_object(r2, lx, ly, LAIR_ENTRANCE) {
                            b = true;
                            if e.staff_in {
                                w.set_object_mode(lair, 2);
                            } else {
                                w.set_object_mode(lair, 1);
                                w.set_room_portal(room, true);
                                w.set_room_portal(r2, true);
                            }
                        }
                    }
                }
            }
        }
        if !a && !b {
            return false;
        }
        let e = x(ctl, i);
        e.lair_open = true;
        e.objects_update = false;
    }
    x(ctl, i).timer_active = false;
    true
}

/// Orifice init 21 (`0x0059DB50`).
pub fn orifice_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let guid = w.guid(object);
    let e = x(ctl, i);
    e.orifice_seen = true;
    e.orifice_guid = guid;
    if e.staff_in && !e.timer_active && !e.lair_open {
        add_timer(ctl, CHAIN, Timer::LairObjects, 1);
        x(ctl, i).timer_active = true;
        w.set_object_mode(object, 2);
    } else if e.lair_open {
        w.set_object_mode(object, 2);
    }
}

/// Init 37 (`0x0059DA50`; no 1.14d `objects.txt` user, open question 8).
pub fn init37<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let guid = w.guid(object);
    let intro = !ctl.records[i].not_intro;
    let e = x(ctl, i);
    e.init37_seen = true;
    e.init37_guid = guid;
    let mode = if intro || e.lair_open || e.objects_update {
        2
    } else {
        0
    };
    w.set_object_mode(object, mode);
}

/// Tyrael's door init 38 (`0x0059DAD0`, object 153).
pub fn door_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let guid = w.guid(object);
    let intro = !ctl.records[i].not_intro;
    let e = x(ctl, i);
    e.door_seen = true;
    e.door_guid = guid;
    let mode = if intro { 2 } else { e.door_mode };
    w.set_object_mode(object, mode);
}

/// Warp check `0x0059DB20` for level 73 (`quests.md` §8.2): true = open.
/// Closed while not-intro and the lair is not open. `None`: no chain-13
/// record (the spec does not say; the caller decides).
pub fn lair_warp_open(ctl: &QuestControl) -> Option<bool> {
    let r = ctl.record(CHAIN)?;
    Some(!(r.not_intro && !r.extra.a2.q6.lair_open))
}

// ------------------------------------------------------------ §8.9

/// §8.9 first use: the six base entries whose index ≠ staff tomb − 66,
/// in order. A tomb outside 66–72 skips no entry: the copy loop runs 7
/// times and stops copying at six, so the first six are kept
/// (`0x0059D756`–`0x0059D784`).
pub fn arcane_list(tomb: u32) -> [u16; 6] {
    let skip = tomb.wrapping_sub(FIRST_TOMB) as usize;
    let mut out = [0u16; 6];
    let mut n = 0;
    for (k, &id) in ARCANE_BASE.iter().enumerate() {
        if k != skip && n < 6 {
            out[n] = id;
            n += 1;
        }
    }
    out
}

/// `0x0059D830` → `0x0059D720` (from `0x0054F439`): the next Arcane
/// Sanctuary dummy object id.
pub fn arcane_object<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) -> u16 {
    let Some(i) = ctl.find(CHAIN) else {
        return ARCANE_BASE[6];
    };
    if !xr(ctl, i).arcane_made {
        let tomb = w.true_tomb_level();
        if tomb == 0 {
            return ARCANE_BASE[6];
        }
        let e = x(ctl, i);
        e.arcane = arcane_list(tomb);
        e.arcane_made = true;
    }
    let e = x(ctl, i);
    if e.arcane_next == 6 {
        e.arcane_next = 0;
    }
    let id = e.arcane[usize::from(e.arcane_next)];
    e.arcane_next += 1;
    id
}

// ------------------------------------------------------------ §8.11

/// Event 11 `0x0059CB20` (a = NPC class, b = message index).
fn messages<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let r = pf(w, p);
    let class = u16::try_from(args.a).ok();
    match (class, args.b) {
        (Some(TYRAEL1), 302) if ctl.records[i].not_intro && !xr(ctl, i).portal_opened => {
            // Tyrael then falls through to the 444–452 switch, where 302
            // matches nothing (`quests-act2-2.md` §1 item 10).
            tyrael_portal(ctl, w, i, p)
        }
        (Some(JERHYN), 430) => {
            if let Some(n) = args.target {
                ctl.refresh_text(w, p, n);
            }
            ctl.records[i].state = 2;
            x(ctl, i).chat_start = true;
            arm_chat_end(ctl, i);
            call_seq(ctl, w, 10);
        }
        (Some(JERHYN), 442) if r.get(SLOT, bit::LEAVE_TOWN) => {
            if r.get(SLOT, bit::PRIMARY_GOAL_DONE) {
                ctl.records[i].state = 5;
            }
            x(ctl, i).chat_end = true;
            arm_chat_end(ctl, i);
            set_bit(w, p, SLOT, bit::ENTER_AREA);
            super::clear_bit(w, p, SLOT, bit::LEAVE_TOWN);
            if let Some(n) = args.target {
                ctl.refresh_text(w, p, n);
            }
        }
        (Some(npc::MESHIF1), 450) if r.get(SLOT, bit::ENTER_AREA) => {
            if r.get(STAFF_SLOT, bit::REWARD_GRANTED) {
                for code in [*b"hst ", *b"vip ", *b"msf "] {
                    w.delete_item(p, code);
                }
            }
            if r.get(SLOT, bit::PRIMARY_GOAL_DONE) {
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                ctl.records[i].state = 5;
                status_all(ctl, w, i, 13);
            }
            set_bit(w, p, SLOT, bit::REWARD_GRANTED);
            send_flags(w, p);
            super::clear_bit(w, p, SLOT, bit::ENTER_AREA);
            add_guid(ctl, w, i, p);
        }
        // Jerhyn and Meshif return for any other message: 444–452 count
        // for every other NPC class (`quests-act2-2.md` §1 item 10).
        (Some(JERHYN | npc::MESHIF1), _) => {}
        (_, m) => {
            // Jump table `0x0059CEB4` read raw (448, 450, 451: nothing).
            if let Some(&(_, b)) = TOWNSFOLK_MSGS.iter().find(|t| t.0 == m) {
                set_bit(w, p, SLOT, b);
            }
        }
    }
}

/// Tyrael's message 302: the portal to Lut Gholein and the credit.
fn tyrael_portal<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: UnitId) {
    x(ctl, i).portal_opening = true;
    let created = w
        .unit_position(p)
        .is_some_and(|(px, py, _)| w.create_tyrael_portal(p, px, py, PORTAL, TOWN));
    if created {
        ctl.records[i].state = 4;
        // `0x0059C860` for each player from Tyrael: room level 73 and
        // lacks 14.13, 14.3 and 14.4.
        for q in w.players() {
            let f = pf(w, q);
            if w.unit_level(q) == Some(DURIEL_LAIR)
                && !f.get(SLOT, bit::PRIMARY_GOAL_DONE)
                && !f.get(SLOT, bit::LEAVE_TOWN)
                && !f.get(SLOT, bit::ENTER_AREA)
            {
                portal_grant(w, q);
            }
        }
        // `0x0059C9A0` for each player with 14.13: every party member
        // gets `0x0059C920` (`quests-act2-2.md` §1 item 17).
        for q in w.players() {
            if !pf(w, q).get(SLOT, bit::PRIMARY_GOAL_DONE) {
                continue;
            }
            for m in party(w, q) {
                let f = pf(w, m);
                // Chain 13 exists here; the member's own 14.13 and level
                // are not tested.
                if !f.get(SLOT, bit::REWARD_GRANTED)
                    && !f.get(SLOT, bit::LEAVE_TOWN)
                    && !f.get(SLOT, bit::ENTER_AREA)
                    && in_act2(w, m)
                {
                    portal_grant(w, m);
                }
            }
        }
        completion_flag(w);
        let e = x(ctl, i);
        e.portal_opened = true;
        e.chat_tyrael = true;
        arm_chat_end(ctl, i);
    }
    x(ctl, i).portal_opening = false;
}

/// `0x0059C810`: set 14.13, 14.3 and character progression
/// `0x00538680(client, 2, difficulty)` (`quests-act1-rest.md` §5). No
/// 0x28 is sent.
fn portal_grant<W: QuestWorld>(w: &mut W, p: UnitId) {
    set_bit(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
    set_bit(w, p, SLOT, bit::LEAVE_TOWN);
    let d = w.difficulty();
    raise_progression(w, p, PROGRESSION_STEP, d);
}

/// `0x0059C9F0`: players lacking 14.0, 14.3, 14.4 get 14.14 and
/// `5D 0D 00 0C 0000` (`0x00545920`, act argument 0).
fn completion_flag<W: QuestWorld>(w: &mut W) {
    for p in w.players() {
        let f = pf(w, p);
        if f.get(SLOT, bit::REWARD_GRANTED)
            || f.get(SLOT, bit::LEAVE_TOWN)
            || f.get(SLOT, bit::ENTER_AREA)
        {
            continue;
        }
        set_bit(w, p, SLOT, bit::COMPLETED_NOW);
        if w.unit_level(p) != Some(0) {
            w.send(p, &[0x5D, CHAIN, 0, 12, 0, 0]);
        }
    }
}

/// 14.5 for a player lacking 14.0, 14.3, 14.4, 14.5.
fn kill_credit<W: QuestWorld>(w: &mut W, p: UnitId) -> bool {
    let f = pf(w, p);
    if [
        bit::REWARD_GRANTED,
        bit::LEAVE_TOWN,
        bit::ENTER_AREA,
        bit::CUSTOM1,
    ]
    .iter()
    .any(|&b| f.get(SLOT, b))
    {
        return false;
    }
    set_bit(w, p, SLOT, bit::CUSTOM1);
    true
}

/// A player's and his party's kill credit. Each member gets the member
/// function `0x0059CF20`: lacks 14.0, 14.3, 14.4, 14.5 and is in Act II
/// → 14.5 (`quests-act2-2.md` §1 item 12; chain 13 exists here).
fn kill_credit_party<W: QuestWorld>(w: &mut W, p: UnitId) -> bool {
    if !kill_credit(w, p) {
        return false;
    }
    for m in party(w, p) {
        if in_act2(w, m) {
            kill_credit(w, m);
        }
    }
    true
}

/// Duriel's death (event 8, `0x0059D050`).
fn duriel_killed<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if ctl.records[i].not_intro {
        ctl.records[i].state = 3;
        add_timer(ctl, CHAIN, Timer::DurielStatus, 8);
        if let Some(k) = args.player {
            // `0x00545990` (a `ret 4` stub, edge case 13) is called only
            // when the killer qualified, after his party.
            let _qualified = kill_credit_party(w, k);
        }
    }
    // Edge case 12: even in an intro game.
    let r = &mut ctl.records[i];
    r.clear_callback(event::NPC_DEACTIVATE);
    r.clear_callback(event::MONSTER_KILLED);
    let room = args.target.and_then(|v| w.unit_position(v)).map(|v| v.2);
    let e = x(ctl, i);
    e.duriel_killed = true;
    e.duriel_room = room;
    ctl.unique_event(w, FX_DURIEL);
    for p in w.players() {
        if w.unit_level(p) == Some(DURIEL_LAIR) {
            kill_credit_party(w, p);
        }
    }
    let e = xr(ctl, i).clone();
    if e.door_seen {
        if let Some((o, _)) = w.object_by_guid(e.door_guid) {
            animate(w, o);
        }
    }
    x(ctl, i).door_mode = 2;
}

/// Timer `0x0059CEE0` (§8.11); true = remove.
pub(super) fn duriel_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if !matches!(ctl.records[i].status, 3..=5) {
        status_all(ctl, w, i, 3);
    }
    x(ctl, i).status_timer = false;
    true
}

/// Chat end `0x0059C760` (callback 2).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    match args.target.and_then(|n| w.monster_class(n)) {
        Some(TYRAEL1) if xr(ctl, i).chat_tyrael => {
            status_all(ctl, w, i, 4);
            x(ctl, i).chat_tyrael = false;
            ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
        }
        Some(JERHYN) => {
            if xr(ctl, i).chat_start {
                status_all(ctl, w, i, 1);
                x(ctl, i).chat_start = false;
            } else if xr(ctl, i).chat_end {
                status_all(ctl, w, i, 6);
                x(ctl, i).chat_end = false;
            } else {
                return;
            }
            ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
            flag_iterate_all(ctl, w, i);
        }
        _ => {}
    }
}

/// Event 13 `0x0059D4F0`.
fn start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, p: Option<UnitId>) {
    let Some(p) = p else { return };
    super::clear_bit(w, p, STAFF_SLOT, 9);
    let f = pf(w, p);
    if f.get(STAFF_SLOT, bit::REWARD_GRANTED) {
        let e = x(ctl, i);
        e.staff_in = true;
        e.objects_update = true;
    }
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::COMPLETED_BEFORE) {
        x(ctl, i).completed_before = true;
        return;
    }
    // "status n, state m": direct byte stores, flags kept, nothing sent
    // (`quests-act2-2.md` §1 item 6); the three tests as consecutive ifs
    // (the last one set wins).
    let r = &mut ctl.records[i];
    if f.get(SLOT, bit::STARTED) {
        r.status = 1;
        r.state = 2;
    }
    if f.get(SLOT, bit::LEAVE_TOWN) {
        r.status = 5;
        r.state = 5;
    }
    if f.get(SLOT, bit::ENTER_AREA) {
        r.status = 6;
        r.state = 5;
    }
}

/// Portal check result (`0x0059DFD0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortalDest {
    /// Not redirected: the portal's own destination.
    Default,
    /// Tyrael's portal: this spot in Lut Gholein.
    Spot(i32, i32, RoomId),
    /// Tyrael's portal, but no spawn location or free spot was found.
    NoSpot,
}

/// Portal check `quests.md` §8.3 (`0x0059DFD0`): while +0x3C = 1 the
/// destination is the Act II spawn location of type 12 in level 40, then
/// a free spot (size 3, mask 0xBE11, radius 7).
pub fn portal_destination<W: QuestWorld>(ctl: &QuestControl, w: &mut W) -> PortalDest {
    if !ctl
        .record(CHAIN)
        .is_some_and(|r| r.extra.a2.q6.portal_opening)
    {
        return PortalDest::Default;
    }
    let Some((sx, sy, room)) = w.spawn_location(ACT, TOWN, 12) else {
        return PortalDest::NoSpot;
    };
    match w.free_spot_near(room, sx, sy, 3, 0xBE11, 7) {
        Some((fx, fy, r)) => PortalDest::Spot(fx, fy, r),
        None => PortalDest::NoSpot,
    }
}

// ------------------------------------------------------------ §10

fn not_intro_state(ctl: &QuestControl) -> Option<u8> {
    ctl.record(CHAIN).filter(|r| r.not_intro).map(|r| r.state)
}

/// `0x0059D7C0` (Jerhyn / palace NPC logic): chain 13 not-intro with
/// state < 2 → false, else true.
pub fn palace_closed_hook(ctl: &QuestControl) -> bool {
    !not_intro_state(ctl).is_some_and(|s| s < 2)
}

/// `0x0059D7E0`: chain 13 not-intro with state 1.
pub fn jerhyn_waiting_hook(ctl: &QuestControl) -> bool {
    not_intro_state(ctl) == Some(1)
}

/// `0x0059DFB0` (from `0x0059F510`): chain 13 not-intro with state < 4 →
/// false, else true.
pub fn tyrael_portal_hook(ctl: &QuestControl) -> bool {
    !not_intro_state(ctl).is_some_and(|s| s < 4)
}

/// `0x0059DF50` (Tyrael's AI, §10): chain 13 absent → false; completed
/// before → true; portal opened → every player without state 7
/// (`0x005538D0`, callback `0x0059DF30`) is tested with the size-adjusted
/// distance `0x006416D0(player, Tyrael)` < 12, and the result is true
/// when none is that close; else false.
pub fn tyrael_leave_hook<W: QuestWorld>(ctl: &QuestControl, w: &mut W, tyrael: UnitId) -> bool {
    let Some(r) = ctl.record(CHAIN) else {
        return false;
    };
    let e = &r.extra.a2.q6;
    if e.completed_before {
        return true;
    }
    if !e.portal_opened {
        return false;
    }
    let mut near = false;
    for p in w.players() {
        if w.distance_between(p, tyrael).is_some_and(|d| d < 12) {
            near = true;
        }
    }
    !near
}

/// `0x0059C750` (Tyrael's AI): the flag iterate for every player.
pub fn tyrael_iterate_hook<W: QuestWorld>(ctl: &QuestControl, w: &mut W) {
    if let Some(i) = ctl.find(CHAIN) {
        flag_iterate_all(ctl, w, i);
    }
}
