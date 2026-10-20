// Spec: specs/world/quests-act2.md §6 (A2Q4 Arcane Sanctuary, chain 11, slot 12), §10 (chain 11 hooks), §4.9 (Arcane hook)
// Spec: specs/world/quests-act2-2.md §1 items 6, 7, 16; §2 (Jerhyn's objects and spawns, replaces act2 §6.10)
//! A2Q4 callback by callback: opening the palace (§6.2), the flag
//! iterate (§6.3), chat with Kaelan's quest-seed draw (§6.4), messages
//! and chat end (§6.5), level changes (§6.6), Horazon's journal (§6.7),
//! the harem blocker (§6.8), the Sanctuary portal (§6.9), Jerhyn's
//! objects and spawns (`quests-act2-2.md` §2: inits 18 and 19, event 3,
//! the palace spawn), game start (§6.11) and the hooks other systems call
//! (§10, §4.9).

use super::{
    add_guid, add_state, completion_flag, guid_listed, in_act2, party, pf, quick_remove, rec,
    remove_guid, set_bit, status_all, status_silent, table_state, TOWN,
};
use crate::units::{RoomId, UnitId};
use crate::world::quests::act3::InitPoint;
use crate::world::quests::{bit, event, EventArgs, QuestControl, QuestWorld, TextList};

/// Chain id.
pub const CHAIN: u8 = 11;
/// Flag slot.
pub const SLOT: u8 = 12;
/// act2guard2 (Kaelan).
const KAELAN: u16 = 331;
const DROGNAN: u16 = 177;
const JERHYN: u16 = 201;
/// Arcane Sanctuary.
const SANCTUARY: u32 = 74;
/// Harem Level 1.
const HAREM1: u32 = 50;
/// Palace Cellar 3.
const CELLAR3: u32 = 54;
/// Canyon of the Magi.
const CANYON: u32 = 46;
/// Horazon's journal message (§6.7).
const MSG_TOME: u16 = 396;
/// The harem blocker (objects.txt row 318 `eunuch`).
const BLOCKER_CLASS: u16 = 318;
/// `0x00738D44`: table state by record state 0–6 (§6.4).
const MSG_STATE: [i8; 7] = [-1, 0, 1, 2, 3, 4, 0];

/// Extra data (§6.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x04 Drognan started the quest.
    pub drognan_started: bool,
    /// +0x08 the tome's room.
    pub tome_room: Option<RoomId>,
    /// +0x0C Jerhyn spawned at the start position.
    pub jerhyn_start: bool,
    /// +0x0D Jerhyn spawned at the palace.
    pub jerhyn_palace: bool,
    /// +0x0E the palace is open.
    pub palace_open: bool,
    /// +0x0F guard-moved flag (first).
    pub guard_moved: bool,
    /// +0x10 guard-moved flag (second, set by `0x0059B6E0`).
    pub guard_moved2: bool,
    /// +0x11 the harem blocker exists.
    pub blocker_made: bool,
    /// +0x12 Jerhyn's palace walk armed (`0x0059F580`, §10).
    pub palace_walk: bool,
    /// +0x13 Jerhyn's walk to the blocker started (§10).
    pub palace_walked: bool,
    /// +0x14 Jerhyn placed beside the blocker (§10).
    pub palace_placed: bool,
    /// +0x15 Jerhyn's position stored.
    pub jerhyn_pos_stored: bool,
    /// +0x16 the portal to the Canyon is open.
    pub canyon_portal: bool,
    /// +0x18 guard position flag (first).
    pub guard_pos: bool,
    /// +0x19 guard position flag (second).
    pub guard_pos2: bool,
    /// +0x1A a player is near the blocker.
    pub near_blocker: bool,
    /// +0x1C that player's GUID.
    pub near_guid: u32,
    /// +0x20 / +0x24 the blocker position.
    pub blocker_x: i32,
    pub blocker_y: i32,
    /// +0x28 / +0x2C Jerhyn's position.
    pub jerhyn_x: i32,
    pub jerhyn_y: i32,
    /// +0x30 / +0x34 the guard position.
    pub guard_x: i32,
    pub guard_y: i32,
    /// +0x38 the blocker's GUID.
    pub blocker_guid: u32,
    /// +0x3C Jerhyn's GUID.
    pub jerhyn_guid: u32,
    /// u16 +0x40 the blocker mode.
    pub blocker_mode: u16,
    /// u16 +0x42 the portal mode in the Arcane Sanctuary.
    pub portal_mode_sanctuary: u16,
    /// u16 +0x44 the portal mode in Palace Cellar 3.
    pub portal_mode_cellar: u16,
    /// +0x46 the blocker was neutral when opened.
    pub blocker_was_neutral: bool,
}

fn x4(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.a2.q4
}

/// Dispatches chain 11's callbacks; false = no body (unhandled).
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
        // `0x0059AD40`.
        event::PLAYER_LEAVES_GAME => remove_guid(ctl, w, i, args.player),
        event::SCROLL_MESSAGE => messages(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => game_start(ctl, w, i, args),
        _ => return false,
    }
    true
}

// ------------------------------------------------------------ §6.2, §6.3

/// Opening the palace `0x0059AEF0` (§6.2).
fn open_palace<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    let x = x4(ctl, i);
    x.palace_open = true;
    if x.blocker_mode == 0 {
        x.blocker_was_neutral = true;
    }
    x.blocker_mode = 2;
    flag_iterate(ctl, w, i);
    let x = &ctl.records[i].extra.a2.q4;
    if x.blocker_made {
        if let Some((o, _)) = w.object_by_guid(x.blocker_guid) {
            w.set_object_mode(o, 2);
            w.free_object_collision(o);
        }
    }
}

/// Flag iterate `0x0059ADB0` (§6.3).
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let (state, status) = (ctl.records[i].state, ctl.records[i].status);
    for p in w.players() {
        let f = pf(w, p);
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match (state, status) {
            (2, _) => set_bit(w, p, SLOT, 2),
            (3, _) => set_bit(w, p, SLOT, 3),
            (4, 2 | 3) => set_bit(w, p, SLOT, 4),
            (4, 4) => set_bit(w, p, SLOT, 5),
            _ => {}
        }
    }
}

// ------------------------------------------------------------ §6.4

/// Event 0 `0x0059B1C0`.
fn chat<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    if args.target.and_then(|n| w.monster_class(n)) == Some(KAELAN) {
        let k = if ctl.records[i].extra.a2.q4.palace_open {
            // One quest-seed step on every chat open (msgs 187–189).
            8 + (ctl.seed.step() % 3) as u8
        } else {
            7
        };
        return add_state(ctl, w, i, list, args.target, k);
    }
    let r = rec(w, args.player);
    if r.get(SLOT, bit::REWARD_PENDING) {
        return add_state(ctl, w, i, list, args.target, 4);
    }
    if guid_listed(ctl, w, i, args.player) {
        return add_state(ctl, w, i, list, args.target, 5);
    }
    let rd = &ctl.records[i];
    if rd.state != 0
        && rd.not_intro
        && !r.get(SLOT, bit::REWARD_GRANTED)
        && (rd.state <= 4 || r.get(SLOT, bit::PRIMARY_GOAL_DONE))
    {
        if let Some(m) = table_state(&MSG_STATE, rd.state).filter(|&m| m <= 11) {
            add_state(ctl, w, i, list, args.target, m);
        }
    }
}

/// Active `0x0059ACA0` (§6.4).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc: u16,
) -> bool {
    let f = pf(w, player);
    let (r, x) = (&ctl.records[i], &ctl.records[i].extra.a2.q4);
    if f.get(SLOT, bit::REWARD_GRANTED) {
        return false;
    }
    let idle = !f.get(SLOT, bit::REWARD_PENDING);
    match npc {
        KAELAN => {
            idle && ((x.blocker_mode == 0 && !f.get(SLOT, 7))
                || (x.blocker_mode == 2 && x.blocker_was_neutral && !f.get(SLOT, 8)))
        }
        DROGNAN => idle && r.state == 1,
        JERHYN => idle && r.state == 2,
        _ => false,
    }
}

// ------------------------------------------------------------ §6.5

/// Event 11 `0x0059AF50` (a = NPC class, b = message).
fn messages<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a == u32::from(KAELAN) {
        match args.b {
            186 => set_bit(w, p, SLOT, 7),
            187..=189 => set_bit(w, p, SLOT, 8),
            _ => {}
        }
        return;
    }
    match (args.a, args.b) {
        (_, 397..=407) => {
            refresh(ctl, w, p, args.target);
            if pf(w, p).get(SLOT, bit::REWARD_PENDING) {
                super::clear_bit(w, p, SLOT, bit::REWARD_PENDING);
                add_guid(ctl, w, i, p);
            }
        }
        (_, 396) => {
            let r = &ctl.records[i];
            if r.not_intro && r.status < 5 {
                status_silent(ctl, i, 5);
            }
            let x = &ctl.records[i].extra.a2.q4;
            let room = w.unit_position(p).map(|q| q.2);
            if !x.canyon_portal && room.is_some() && room == x.tome_room {
                if let Some((sx, sy)) = w.free_spot(p, 2, 0xBE11, 8, 100) {
                    if w.create_portal(p, sx, sy, 60, CANYON) {
                        x4(ctl, i).canyon_portal = true;
                        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                    }
                }
            }
        }
        (a, 373) if a == u32::from(DROGNAN) => {
            ctl.records[i].state = 2;
            x4(ctl, i).drognan_started = true;
            open_palace(ctl, w, i);
            refresh(ctl, w, p, args.target);
        }
        (a, 377) if a == u32::from(JERHYN) => {
            ctl.records[i].state = 3;
            refresh(ctl, w, p, args.target);
            status_all(ctl, w, i, 3);
            flag_iterate(ctl, w, i);
        }
        _ => {}
    }
}

/// "refresh" (`quests.md` §7.2) with the message's NPC.
fn refresh<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, p: UnitId, npc: Option<UnitId>) {
    if let Some(n) = npc {
        ctl.refresh_text(w, p, n);
    }
}

/// Event 2 `0x0059AE60`.
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let drognan = args.target.and_then(|n| w.monster_class(n)) == Some(DROGNAN);
    if drognan && ctl.records[i].extra.a2.q4.drognan_started {
        status_all(ctl, w, i, 2);
        x4(ctl, i).drognan_started = false;
        ctl.records[i].clear_callback(event::NPC_DEACTIVATE);
        flag_iterate(ctl, w, i);
    }
}

// ------------------------------------------------------------ §6.6

/// Event 3 `0x0059F0C0` (a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.b == SANCTUARY {
        let was = ctl.records[i].state;
        if was < 4 {
            ctl.records[i].state = 4;
        }
        if ctl.records[i].status < 4 {
            status_all(ctl, w, i, 4);
            flag_iterate(ctl, w, i);
        } else if was < 4 {
            flag_iterate(ctl, w, i);
        }
    }
    if args.b == HAREM1 {
        if let Some(p) = args.player {
            set_bit(w, p, SLOT, 8);
            set_bit(w, p, SLOT, 7);
        }
    }
    // The old-level-40 handling is skipped when the new level is the
    // Arcane Sanctuary: the level-74 branch of `0x0059F0C0` returns on
    // every path (`quests-act2.md` §6.6; L35 read 2026-10-14, settles
    // REC-1405). Recording a2-warp-arcane-ama: the start Jerhyn still
    // present to frame 143 (removed at the warp frame in the other warps:
    // a2-warp-maggot-lair-ama).
    if args.a == TOWN && args.b != SANCTUARY {
        jerhyn_leaving_town(ctl, w, i);
        quick_remove(ctl, w, i, args.player);
        let f = rec(w, args.player);
        if args.player.is_some()
            && !f.get(SLOT, bit::REWARD_GRANTED)
            && !f.get(SLOT, bit::REWARD_PENDING)
            && ctl.records[i].state == 3
        {
            ctl.records[i].state = 4;
            flag_iterate(ctl, w, i);
        }
    }
}

// ------------------------------------------------------------ §6.7

/// Horazon's journal, operate 42 `0x0059B970` (§6.7, objects.txt row
/// 357): `player` reads `object`.
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
    // `0x005456A0`: S→C 0x27 type 2 (`quests-act2-2.md` §5.4).
    w.open_quest_message(player, object, MSG_TOME);
    let room = w.unit_position(object).map(|p| p.2);
    x4(ctl, i).tome_room = room;
    let r = &mut ctl.records[i];
    // The three player iterates run only inside "not-intro and state ≠ 5"
    // (`0x0059B9E4`–`0x0059BA34`, `quests-act2-2.md` §1 item 7).
    if !r.not_intro || r.state == 5 {
        return;
    }
    r.state = 5;
    // `0x0059B3F0`.
    for p in w.players() {
        if w.unit_level(p) == Some(SANCTUARY) && idle(w, p) {
            grant(w, p);
            // `0x0059B360`.
            for m in party(w, p) {
                if in_act2(w, m) && idle(w, m) {
                    grant(w, m);
                }
            }
        }
    }
    // `0x0059B320`; `0x0059B940` only reads 12.13.
    completion_flag(w, CHAIN, SLOT);
}

/// Lacks 12.0 and 12.1.
fn idle<W: QuestWorld>(w: &mut W, p: UnitId) -> bool {
    let f = pf(w, p);
    !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING)
}

/// §6.7's six changes: set 12.13, 12.1, 12.0; clear bits 2–11; set
/// 12.8, 12.7.
fn grant<W: QuestWorld>(w: &mut W, p: UnitId) {
    set_bit(w, p, SLOT, bit::PRIMARY_GOAL_DONE);
    set_bit(w, p, SLOT, bit::REWARD_PENDING);
    set_bit(w, p, SLOT, bit::REWARD_GRANTED);
    if let Some(f) = super::super::flags_of(w, p) {
        f.reset_progress(SLOT);
    }
    set_bit(w, p, SLOT, 8);
    set_bit(w, p, SLOT, 7);
}

// ------------------------------------------------------------ §6.8

/// `0x0059B710` (§6.8), object event 7 of class 0x7A: record 11 (no
/// record → nothing).
pub fn harem_blocker<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let x = &ctl.records[i].extra.a2.q4;
    if x.palace_open || x.blocker_made {
        return;
    }
    x4(ctl, i).blocker_mode = 0;
    let Some((ux, uy, room)) = w.unit_position(object) else {
        return;
    };
    let made = w
        .spawn_quest_object(room, ux - 2, uy - 1, BLOCKER_CLASS)
        .or_else(|| w.spawn_quest_object(room, ux - 2, uy, BLOCKER_CLASS));
    if let Some(o) = made {
        w.or_unit_flags(o, 0x0300_0000);
        let g = w.guid(o);
        let x = x4(ctl, i);
        x.blocker_made = true;
        x.blocker_guid = g;
    }
}

/// Harem blocker init 30 `0x0059B7D0` (§6.8, objects.txt row 318).
pub fn blocker_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let mode = ctl.records[i].extra.a2.q4.blocker_mode;
    w.set_object_mode(object, i32::from(mode));
    x4(ctl, i).blocker_guid = w.guid(object);
    if mode == 2 {
        w.free_object_collision(object);
    }
}

// ------------------------------------------------------------ §6.9

/// Sanctuary portal init 29 `0x0059BA40` (§6.9, objects.txt row 298).
pub fn portal_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let level = w.unit_level(object);
    let x = x4(ctl, i);
    let slot = match level {
        Some(SANCTUARY) => &mut x.portal_mode_sanctuary,
        Some(CELLAR3) => &mut x.portal_mode_cellar,
        _ => return,
    };
    // The object's mode is set to the stored value first; only the stored
    // u16 becomes 2 (`0x0059BA6B`–`0x0059BAE6`): the object stays in mode
    // 1 until its ENDANIM event.
    let was = *slot;
    if was == 1 {
        *slot = 2;
    }
    w.set_object_mode(object, i32::from(was));
    if was == 1 {
        let at = w.frame() + (w.object_anim_length(object) >> 8) + 1;
        w.schedule_object_event(object, 1, at);
    }
}

/// `0x0059BAF0(level)` (§6.9): called by the portal's operate 34
/// (`0x005846B0`) with the level it is in.
pub fn portal_operate<W: QuestWorld>(ctl: &mut QuestControl, _w: &mut W, level: u32) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let x = x4(ctl, i);
    match level {
        SANCTUARY if x.portal_mode_cellar == 0 => {
            x.portal_mode_cellar = 1;
            x.portal_mode_sanctuary = 2;
        }
        CELLAR3 if x.portal_mode_sanctuary == 0 => {
            x.portal_mode_sanctuary = 1;
            x.portal_mode_cellar = 2;
        }
        _ => {}
    }
}

// ------------------------------------------------------------ §6.10

/// The start Jerhyn's +0x0C handling (`quests-act2-2.md` §2 items 3.1
/// and 4.1). True when he is talking (`0x00573180` ran).
fn start_jerhyn_check<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    let x = &ctl.records[i].extra.a2.q4;
    if !x.jerhyn_start {
        return false;
    }
    match w.monster_by_guid(x.jerhyn_guid) {
        None => x4(ctl, i).jerhyn_start = false,
        Some((j, _)) => {
            if w.npc_hold_chat(j) {
                return true;
            }
            w.remove_unit(j);
            x4(ctl, i).jerhyn_start = false;
        }
    }
    false
}

/// Event 3 with old level 40 (`0x0059F0C0`, before the quick remove;
/// `quests-act2-2.md` §2 item 3).
fn jerhyn_leaving_town<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) {
    start_jerhyn_check(ctl, w, i);
    let x = &ctl.records[i].extra.a2.q4;
    if x.jerhyn_palace || !x.blocker_made {
        return;
    }
    let Some((blocker, _)) = w.object_by_guid(x.blocker_guid) else {
        return;
    };
    match w.unit_position(blocker) {
        Some((bx, by, room)) => palace_spawn(ctl, w, i, bx, by, room),
        // An object without a room: not in the spec.
        None => w.unhandled(CHAIN, 0x0059_F0C0),
    }
}

/// True when chain 13 is not-intro with state < 2 (the palace offsets
/// and the init 18 / 19 tests).
fn tombs_not_started(ctl: &QuestControl) -> bool {
    ctl.record(13).is_some_and(|r| r.not_intro && r.state < 2)
}

/// Palace spawn `0x0059EF70(record, &point, room)` (`quests-act2-2.md`
/// §2 item 4): Jerhyn (201) near (x, y), the free-spot search starting
/// in `room`.
fn palace_spawn<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    x: i32,
    y: i32,
    room: RoomId,
) {
    if start_jerhyn_check(ctl, w, i) || ctl.records[i].extra.a2.q4.jerhyn_palace {
        return;
    }
    let x = if tombs_not_started(ctl) {
        x - 10
    } else {
        x + 15
    };
    let y = y - 3;
    // `0x00545340`: size 3, mask 0x100, sixth argument 9 (never read),
    // limit 100.
    let Some((sx, sy, spot_room)) = w.free_spot_at(room, x, y, 3, 0x100, 9, 100) else {
        // Not found: the spawn is tried at the unchanged point with a null
        // room (edge case 5); the monster spec decides. Reported.
        w.unhandled(CHAIN, 0x0059_EF70);
        return;
    };
    let Some(j) = w
        .spawn_monster_flags(spot_room, sx, sy, JERHYN, 1, -1, 0)
        .or_else(|| w.spawn_monster_flags(spot_room, sx, sy, JERHYN, 1, 2, 0))
    else {
        return;
    };
    w.or_unit_flags(j, 0x0300_0000);
    // The palace Jerhyn's GUID is not stored.
    let e = x4(ctl, i);
    e.jerhyn_palace = true;
    if !e.jerhyn_pos_stored {
        e.jerhyn_pos_stored = true;
        e.jerhyn_x = sx;
        e.jerhyn_y = sy;
    }
}

/// Init 18, the start Jerhyn object 121 (`0x005448B0` → `0x0059F380`,
/// `quests-act2-2.md` §2 item 1): the only writer of +0x3C. The object's
/// room and (x, y) are the InitFn record's (`at`): the init runs inside
/// the allocation, before the unit has a room (`sim/units.md` §3.1 r7.2).
pub fn start_jerhyn_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, at: InitPoint) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if ctl.records[i].extra.a2.q4.jerhyn_palace
        || ctl.game.get(8, bit::PRIMARY_GOAL_DONE)
        || ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE)
        || !tombs_not_started(ctl)
    {
        return;
    }
    let (ox, oy, room) = (at.x, at.y, at.room);
    // `0x00545340`: size 2, mask 0x100, sixth argument 10 (never read),
    // limit 100.
    // No free spot: the spawn is still made with room 0 at the object's
    // own (x, y) (`quests-act2-2.md` §2 item 1), and a null-room spawn
    // returns null before any allocation or draw (`quests-act1-rest.md`
    // §9 item 1): nothing is spawned.
    let Some((sx, sy, spot_room)) = w.free_spot_at(room, ox, oy, 2, 0x100, 10, 100) else {
        return;
    };
    if let Some(j) = w.spawn_monster_flags(spot_room, sx, sy, JERHYN, 1, -1, 0) {
        let g = w.guid(j);
        let e = x4(ctl, i);
        e.jerhyn_start = true;
        e.jerhyn_guid = g;
    }
}

/// Init 19, the palace Jerhyn object 122 (`0x005448E0` → `0x0059F440`,
/// `quests-act2-2.md` §2 item 2): Kaelan, the harem blocker's event 7,
/// then the palace spawn. Room and (x, y): the InitFn record's (`at`, as
/// [`start_jerhyn_init`]).
pub fn palace_jerhyn_init<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    at: InitPoint,
) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let (ox, oy, room) = (at.x, at.y, at.room);
    if !ctl.game.get(14, bit::PRIMARY_GOAL_DONE) {
        let e = x4(ctl, i);
        e.guard_x = ox + 1;
        e.guard_y = oy;
        // `0x005B3090`: mode 1, spread −1, flags 0.
        w.spawn_monster_flags(room, ox + 1, oy, KAELAN, 1, -1, 0);
    }
    // `0x005417D0(game, object, 7, frame + 1)`: the harem blocker (§6.8).
    let at = w.frame() + 1;
    w.schedule_object_event(object, 7, at);
    if !ctl.records[i].extra.a2.q4.jerhyn_palace
        && (ctl.game.get(8, bit::PRIMARY_GOAL_DONE)
            || ctl.game.get(9, bit::PRIMARY_GOAL_DONE)
            || !tombs_not_started(ctl))
    {
        palace_spawn(ctl, w, i, ox, oy, room);
    }
}

// ------------------------------------------------------------ §6.11

/// Event 13 `0x0059B530`.
fn game_start<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    let open = |ctl: &mut QuestControl| {
        let x = x4(ctl, i);
        x.palace_open = true;
        x.blocker_mode = 2;
    };
    if f.get(11, bit::REWARD_GRANTED) {
        open(ctl);
    }
    if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
        open(ctl);
        return;
    }
    // "status n, state m": direct byte stores, flags kept, nothing sent
    // (`quests-act2-2.md` §1 item 6).
    if w.has_item(p, *b"hst ") {
        let r = &mut ctl.records[i];
        (r.status, r.state) = (1, 1);
    }
    let restored = if f.get(SLOT, 5) {
        Some((4, 4))
    } else if f.get(SLOT, 4) {
        Some((3, 4))
    } else if f.get(SLOT, 3) {
        Some((3, 3))
    } else if f.get(SLOT, 2) {
        Some((2, 2))
    } else {
        None
    };
    if let Some((status, state)) = restored {
        let r = &mut ctl.records[i];
        (r.status, r.state) = (status, state);
        open(ctl);
    }
    if f.get(10, bit::REWARD_GRANTED) {
        open(ctl);
    }
}

// ------------------------------------------------------------ §4.9, §10

/// `0x0059B660` (§4.9): the Arcane hook after the staff is assembled.
pub(crate) fn arcane_hook<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro {
        return open_palace(ctl, w, i);
    }
    // Two independent tests (`0x0059B67A`, `0x0059B696`); the status
    // step clears the flags byte before the iterate.
    if ctl.records[i].state == 0 {
        ctl.records[i].state = 1;
    }
    if ctl.records[i].status == 0 {
        status_all(ctl, w, i, 1);
    }
}

/// Palace guard AI hook `0x0059B6E0` (from `0x005E7130`, §10): +0x0F
/// set and +0x10 clear → +0x10 := 1, true.
pub fn guard_moved(ctl: &mut QuestControl) -> bool {
    let Some(i) = ctl.find(CHAIN) else {
        return false;
    };
    let x = x4(ctl, i);
    if x.guard_moved && !x.guard_moved2 {
        x.guard_moved2 = true;
        return true;
    }
    false
}

/// Palace guard AI hook `0x0059B8B0` (from `0x005E7590`, §10): the guard
/// is at its end position (+0x18 and +0x19 clear, +0x40 = 2); true
/// without chain 11.
pub fn guard_at_end(ctl: &QuestControl) -> bool {
    let Some(r) = ctl.record(CHAIN) else {
        return true;
    };
    let x = &r.extra.a2.q4;
    !x.guard_pos && !x.guard_pos2 && x.blocker_mode == 2
}

/// Palace guard AI hook `0x0059B8F0` (§10): with +0x18 set the guard's
/// target (+0x30, +0x34); else with +0x19 set (+0x30, +0x34 − 4); else
/// `None` (the point untouched, return 0). `None` without chain 11.
pub fn guard_target(ctl: &QuestControl) -> Option<(i32, i32)> {
    let x = &ctl.record(CHAIN)?.extra.a2.q4;
    if x.guard_pos {
        Some((x.guard_x, x.guard_y))
    } else if x.guard_pos2 {
        Some((x.guard_x, x.guard_y.wrapping_sub(4)))
    } else {
        None
    }
}

/// Palace guard AI hook `0x0059AEC0` (§10): the blocker is open (+0x40
/// = 2). False without chain 11 (not specified).
pub fn blocker_open(ctl: &QuestControl) -> bool {
    ctl.record(CHAIN)
        .is_some_and(|r| r.extra.a2.q4.blocker_mode == 2)
}

/// Jerhyn / palace NPC hook `0x0059B820` (from `0x0059F580`, §10):
/// +0x1A := 0, +0x1C := 0 (`0x0059F75F`), then the player walk
/// `0x005537D0`: the **first** player without 14.0 and 14.1 within 30 of
/// the stored blocker point +0x20 / +0x24 (`0x005DC5C0`, unsigned ≤ 30)
/// → +0x1A := 1, +0x1C := its GUID, and the walk stops.
pub fn jerhyn_near_blocker<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let x = x4(ctl, i);
    x.near_blocker = false;
    x.near_guid = 0;
    let (bx, by) = (x.blocker_x, x.blocker_y);
    for p in w.players() {
        let f = pf(w, p);
        if f.get(14, bit::REWARD_GRANTED) || f.get(14, bit::REWARD_PENDING) {
            continue;
        }
        let Some(at) = w.unit_xy(p) else { continue };
        if path_distance(at, bx, by) <= 30 {
            let g = w.guid(p);
            let x = x4(ctl, i);
            x.near_blocker = true;
            x.near_guid = g;
            return;
        }
    }
}

/// `0x005DC5C0(unit, x, y)` (`monsters/ai.md` §6): dx, dy = |Δ| on the
/// unit's path position; (2·max + min) / 2, truncated.
fn path_distance(at: (i32, i32), x: i32, y: i32) -> u32 {
    let dx = at.0.abs_diff(x);
    let dy = at.1.abs_diff(y);
    (2 * dx.max(dy) + dx.min(dy)) / 2
}

/// `0x0059DFB0` (§10): chain 13 not-intro with state < 4 → false, else
/// true (a missing record answers true).
fn tombs_past_open(ctl: &QuestControl) -> bool {
    !ctl.record(13).is_some_and(|r| r.not_intro && r.state < 4)
}

/// Jerhyn AI hook `0x0059F570` → `0x0059F510` (from `0x005E7130`, §10):
/// false ("Jerhyn held at the palace") only when chain 11 exists with
/// +0x0D set, chain 8 exists not-intro with state ≠ 5, `0x0059DFB0` is
/// false, and chain 11 is not-intro with state < 2. No draw.
pub fn jerhyn_palace_active(ctl: &QuestControl) -> bool {
    let held = ctl.record(CHAIN).is_some_and(|r11| {
        r11.extra.a2.q4.jerhyn_palace
            && ctl
                .record(8)
                .is_some_and(|r8| r8.not_intro && r8.state != 5)
            && !tombs_past_open(ctl)
            && r11.not_intro
            && r11.state < 2
    });
    !held
}

/// What the Jerhyn AI does after `0x0059F580` (§10): the outputs (a, b)
/// and the action the hook takes on the AI unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JerhynStep {
    /// Nothing to do on the unit.
    Out(i32, i32),
    /// Walk to (x, y) (`0x005DED90`), then (a, b) = (0, 0).
    Walk(i32, i32),
    /// Place the unit at (x, y) = the blocker's position with x + 4, in
    /// the blocker's room (free test sizes 1, 2, 3, then `0x00554EA0`);
    /// on success [`jerhyn_placed`]. (a, b) = (0, 1) either way.
    PlaceAt(i32, i32, RoomId),
}

/// Jerhyn / palace NPC logic `0x0059F580(game, unit, &a, &b)` (§10). The
/// walk and placement are the AI's ([`JerhynStep`]); the record writes
/// before them are made here. `at` is the unit's path position.
pub fn jerhyn_npc_state<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    at: (i32, i32),
) -> JerhynStep {
    let Some(i) = ctl.find(CHAIN) else {
        return JerhynStep::Out(1, 0);
    };
    if !ctl.records[i].not_intro || !ctl.records[i].extra.a2.q4.jerhyn_palace {
        return JerhynStep::Out(1, 0);
    }
    // `0x0059D7C0`: chain 13 not-intro with state < 2 → 0, else 1.
    if ctl.game.get(12, 13) || ctl.game.get(14, 13) || !tombs_not_started(ctl) {
        return JerhynStep::Out(1, 0);
    }
    if ctl.records[i].extra.a2.q4.palace_placed {
        return JerhynStep::Out(1, 0);
    }
    // `0x0059D7E0`: "not-intro with state 1" on chain 13's record
    // (asm read, `quests-act2.md` exact-reads row; settles REC-1631).
    if ctl.record(13).is_some_and(|r| r.not_intro && r.state == 1) {
        x4(ctl, i).palace_walk = true;
    }
    if !ctl.records[i].extra.a2.q4.palace_walk {
        return JerhynStep::Out(1, 0);
    }
    let x = &ctl.records[i].extra.a2.q4;
    if !x.palace_walked || x.guard_moved {
        jerhyn_near_blocker(ctl, w);
        let x = x4(ctl, i);
        if x.near_blocker {
            x.palace_walked = true;
            x.palace_walk = false;
            return JerhynStep::Walk(x.blocker_x, x.blocker_y);
        }
        return JerhynStep::Out(0, 1);
    }
    if !x.blocker_made {
        return JerhynStep::Out(0, 1);
    }
    let (bx, by, guid) = (x.blocker_x, x.blocker_y, x.blocker_guid);
    if path_distance(at, bx, by) > 2 {
        let x = x4(ctl, i);
        x.guard_pos2 = true;
        x.guard_pos = false;
        return JerhynStep::Walk(bx, by);
    }
    let found = w.object_by_guid(guid).and_then(|(b, _)| w.unit_position(b));
    match found {
        Some((ox, oy, room)) => JerhynStep::PlaceAt(ox + 4, oy, room),
        None => JerhynStep::Out(0, 1),
    }
}

/// [`JerhynStep::PlaceAtBlocker`] succeeded: +0x0F := 1, +0x14 := 1.
pub fn jerhyn_placed(ctl: &mut QuestControl) {
    if let Some(i) = ctl.find(CHAIN) {
        let x = x4(ctl, i);
        x.guard_moved = true;
        x.palace_placed = true;
    }
}

/// Monster class hook `0x0059B6C0` (jerhyn, from `0x005447A0`): a bare
/// `ret`.
pub fn jerhyn_class_hook() {}

/// Monster class hook `0x0059B6D0` (act2guard2, from `0x005447A0`): a
/// bare `ret`.
pub fn guard_class_hook() {}
