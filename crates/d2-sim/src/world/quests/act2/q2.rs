// Spec: specs/world/quests-act2.md §4 (A2Q2 The Horadric Staff, chain 9, slot 10)
// Spec: specs/world/quests-act2-2.md §1 items 13, 14, 20
//! A2Q2 callback by callback: Cain's text selector (§4.3), chat and the
//! active function (§4.4), Cain's messages (§4.5), the status function
//! (§4.6), the three quest chests (§4.7, the §1.3 pattern), pick-up and
//! drop (§4.8), the staff assembly hook (§4.9) and joining, starting and
//! leaving (§4.10). Slot 10 is a constant in each.

use super::{
    add_guid, add_state, chest_gold, clear_bit, guid_listed, pf, quick_remove, remove_guid,
    send_flags, set_bit, status_iterate, TOWN,
};
use crate::units::UnitId;
use crate::world::quests::{bit, event, EventArgs, QuestControl, QuestFlags, QuestWorld, TextList};

/// The record's chain.
pub const CHAIN: u8 = 9;
/// The record's flag slot.
pub const SLOT: u8 = 10;
/// cain2 (Act II Cain).
pub const CAIN2: u16 = 244;

/// Horadric Scroll.
pub const SCROLL: [u8; 4] = *b"tr1 ";
/// Staff of Kings.
pub const STAFF: [u8; 4] = *b"msf ";
/// Viper Amulet.
pub const AMULET: [u8; 4] = *b"vip ";
/// Horadric Cube.
pub const CUBE: [u8; 4] = *b"box ";
/// Horadric Staff.
pub const HSTAFF: [u8; 4] = *b"hst ";

/// 10.3 Cain read the scroll (or any Cain staff talk).
const READ: u8 = 3;
/// 10.4 told about the amulet.
const TOLD_AMULET: u8 = 4;
/// 10.5 told about the staff.
const TOLD_STAFF: u8 = 5;
/// 10.6 told about the cube.
const TOLD_CUBE: u8 = 6;
/// 10.10 told about the assembled staff.
const TOLD_HSTAFF: u8 = 10;
/// 10.11 staff assembled (cube).
const ASSEMBLED: u8 = 11;

/// Selector "no line" value.
const NONE: u8 = 0xFF;

/// Extra data (§4.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x14 drop count.
    pub drops: i32,
    /// +0x18 Staff of Kings held in the game.
    pub staff_count: i32,
    /// +0x1C Horadric Staffs held in the game.
    pub hstaff_count: i32,
    /// +0x20 cubes held in the game.
    pub cube_count: i32,
    /// +0x24 amulets held in the game.
    pub amulet_count: i32,
    /// +0x28 Staff of Kings dropped.
    pub staff_dropped: bool,
    /// +0x29 cube dropped.
    pub cube_dropped: bool,
    /// +0x2A staff assembled.
    pub assembled: bool,
    /// +0x2B "missing" already reported.
    pub missing_reported: bool,
    /// +0x2C GUID of the assembling player (`None`: −1).
    pub assembler: Option<u32>,
}

/// Dispatches chain 9's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => chat(ctl, w, i, args, list),
        event::CHANGED_LEVEL => {
            // `0x00599A00`: leaving Lut Gholein.
            if args.a == TOWN {
                quick_remove(ctl, w, i, args.player);
            }
        }
        event::ITEM_PICKED_UP => pick_up(ctl, w, i, args),
        event::ITEM_DROPPED => drop(w, args),
        // `0x00599A20`: a bare `ret`.
        event::MONSTER_KILLED => {}
        event::PLAYER_DROPPED_WITH_QUEST_ITEM => leaving_item(ctl, w, i, args),
        // `0x00599A10` (§1.1: event 10 removes the player from the list).
        event::PLAYER_LEAVES_GAME => remove_guid(ctl, w, i, args.player),
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => started(ctl, w, i, args),
        event::PLAYER_JOINED_GAME => joined(ctl, w, i, args),
        _ => return false,
    }
    true
}

/// Cain's text selector `0x00599590` (§4.3): (out, result).
pub fn selector<W: QuestWorld>(w: &W, p: UnitId, f: &QuestFlags) -> (u8, bool) {
    if w.has_item(p, HSTAFF) {
        return if f.get(SLOT, TOLD_HSTAFF) {
            (5, false)
        } else {
            (4, true)
        };
    }
    let mut out = NONE;
    for (code, told, first, again) in [
        (CUBE, TOLD_CUBE, 3, 9),
        (SCROLL, READ, 0, 6),
        (AMULET, TOLD_AMULET, 1, 7),
        (STAFF, TOLD_STAFF, 2, 8),
    ] {
        if w.has_item(p, code) {
            if !f.get(SLOT, told) {
                return (first, true);
            }
            out = again;
        }
    }
    (out, false)
}

/// Event 0 `0x00599910` (§4.4).
fn chat<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    if args.target.and_then(|n| w.monster_class(n)) != Some(CAIN2) {
        return;
    }
    // TODO(quests-act2 §4.4): event 0 without a player is not described;
    // NPC chat always has one.
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    let k = if f.get(SLOT, bit::REWARD_PENDING) {
        4
    } else {
        let (out, ok) = selector(w, p, &f);
        if ok && out != NONE {
            out
        } else if guid_listed(ctl, w, i, Some(p)) {
            5
        } else if f.get(SLOT, bit::REWARD_GRANTED) {
            return;
        } else if out > 9 {
            if !f.get(SLOT, READ) {
                return;
            }
            6
        } else {
            out
        }
    };
    add_state(ctl, w, i, list, args.target, k);
}

/// Active function `0x005996C0` (§4.4): Cain, and 10.1 or the selector
/// is true.
pub(super) fn active<W: QuestWorld>(
    _ctl: &QuestControl,
    w: &mut W,
    _i: usize,
    player: UnitId,
    npc: u16,
) -> bool {
    if npc != CAIN2 {
        return false;
    }
    let f = pf(w, player);
    f.get(SLOT, bit::REWARD_PENDING) || selector(w, player, &f).1
}

/// Event 11 `0x005997F0` (§4.5): Cain's messages.
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.a != u32::from(CAIN2) {
        return;
    }
    let Some(p) = args.player else { return };
    match args.b {
        335 => {
            w.delete_item(p, SCROLL);
            set_bit(w, p, SLOT, READ);
        }
        336..=338 => {
            ctl.records[i].flags = 0;
            let b = match args.b {
                336 => TOLD_AMULET,
                337 => TOLD_STAFF,
                _ => TOLD_CUBE,
            };
            set_bit(w, p, SLOT, b);
            set_bit(w, p, SLOT, READ);
        }
        339 => {
            clear_bit(w, p, SLOT, bit::REWARD_PENDING);
            set_bit(w, p, SLOT, READ);
            add_guid(ctl, w, i, p);
            for b in [TOLD_HSTAFF, TOLD_AMULET, TOLD_CUBE, TOLD_STAFF] {
                set_bit(w, p, SLOT, b);
            }
            if let Some(n) = args.target {
                ctl.refresh_text(w, p, n);
            }
        }
        _ => {}
    }
}

/// Status function `0x0059E630` (§4.6, always returns true).
pub(super) fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
) -> u8 {
    if !pf.get(7, bit::REWARD_GRANTED) {
        return 0;
    }
    let told_hstaff = u8::from(pf.get(SLOT, TOLD_HSTAFF));
    if pf.get(SLOT, bit::REWARD_GRANTED) || pf.get(SLOT, bit::REWARD_PENDING) {
        return if pf.get(14, bit::REWARD_GRANTED) {
            11 + 2 * u8::from(pf.get(SLOT, bit::PRIMARY_GOAL_DONE))
        } else {
            6 - told_hstaff
        };
    }
    let (msf, bx, vip) = (
        w.has_item(player, STAFF),
        w.has_item(player, CUBE),
        w.has_item(player, AMULET),
    );
    if msf && bx && vip {
        return if pf.get(SLOT, TOLD_CUBE) {
            3
        } else if pf.get(SLOT, READ) {
            2
        } else {
            4
        };
    }
    if w.has_item(player, HSTAFF) {
        return 6 - told_hstaff;
    }
    if w.has_item(player, SCROLL) && !pf.get(SLOT, READ) {
        return 1;
    }
    let mut s = 0;
    if missing(ctl, &ctl.records[i].extra.a2.q2) {
        s = 9;
    }
    if pf.get(SLOT, READ) {
        s = 2;
    } else if vip || msf || bx {
        s = 4;
    }
    s
}

/// Tainted Sun's altar is destroyed (`0x0059AC80`, chain 10 extra +0x04).
fn altar_destroyed(ctl: &QuestControl) -> bool {
    ctl.record(10)
        .is_some_and(|r| r.extra.a2.q3.altar_destroyed)
}

/// A staff piece is gone for good (§4.6 r3, §4.10 event 9): (cube
/// dropped and cube count 0) or (amulet count 0 and the altar destroyed)
/// or (staff dropped and staff count 0).
fn missing(ctl: &QuestControl, x: &Extra) -> bool {
    (x.cube_dropped && x.cube_count == 0)
        || (x.amulet_count == 0 && altar_destroyed(ctl))
        || (x.staff_dropped && x.staff_count == 0)
}

// ------------------------------------------------------------ §4.7

/// One §4.7 chest: the §1.3 pattern with this chest's code, qualifying
/// test, quality and per-item step.
fn chest<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
    (code, quality): ([u8; 4], u8),
    qualifies: impl Fn(&W, UnitId, &QuestFlags) -> bool,
    mut per_item: impl FnMut(&mut QuestControl, &mut W, UnitId),
) -> u32 {
    if !w.quest_chest_gate(object, player) {
        return 0;
    }
    // The drop code is stored once, before the count (`quests-act2-2.md`
    // §1 item 20); `0x00559A30` reads it for every item.
    w.set_drop_code(object, code);
    // Without chain 9 the count and the items are skipped; treasure and
    // gold still drop (§1 item 20).
    if ctl.find(CHAIN).is_some() {
        // "for each player, starting at the operating player": the count
        // does not depend on the order.
        let mut n = 0;
        for p in w.players() {
            let f = pf(w, p);
            if qualifies(w, p, &f) {
                n += 1;
            }
        }
        for _ in 0..n {
            // `&level` is an out parameter: `0x00559A30` computes the
            // item level itself (the chest's area level, §1 item 20).
            if let Some(item) = w.quest_drop(object, code, quality, None, true) {
                per_item(ctl, w, item);
            }
        }
    }
    w.object_treasure(object, player, 4);
    chest_gold(ctl, w, object);
    0
}

/// Scroll chest (object 355) operate `0x00599C10` (§4.7): one `tr1 ` per
/// player lacking 10.0 and 10.3 (`0x00599700`). Returns 0.
pub fn scroll_chest<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) -> u32 {
    chest(
        ctl,
        w,
        object,
        player,
        (SCROLL, 7),
        |_, _, f| !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, READ),
        |_, _, _| {},
    )
}

/// Staff chest (object 356) operate `0x00599CF0` (§4.7): one identified
/// `msf ` per player lacking 10.0 and holding no `msf ` and no `hst `
/// (`0x00599750`); each adds 1 to the Staff-of-Kings count and sets
/// "staff dropped" (edge case 4). Returns 0.
pub fn staff_chest<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) -> u32 {
    chest(
        ctl,
        w,
        object,
        player,
        (STAFF, 7),
        |w, p, f| {
            !f.get(SLOT, bit::REWARD_GRANTED) && !w.has_item(p, STAFF) && !w.has_item(p, HSTAFF)
        },
        |ctl, w, item| {
            w.identify_item(item);
            if let Some(r) = ctl.record_mut(CHAIN) {
                r.extra.a2.q2.staff_count += 1;
                r.extra.a2.q2.staff_dropped = true;
            }
        },
    )
}

/// Cube chest (object 354) operate `0x00599DF0` (§4.7): one normal
/// `box ` per player holding no `box ` (`0x005997C0`); each adds 1 to the
/// cube count and sets "cube dropped". Returns 0.
pub fn cube_chest<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) -> u32 {
    chest(
        ctl,
        w,
        object,
        player,
        (CUBE, 2),
        |w, p, _| !w.has_item(p, CUBE),
        |ctl, _, _| {
            if let Some(r) = ctl.record_mut(CHAIN) {
                r.extra.a2.q2.cube_count += 1;
                r.extra.a2.q2.cube_dropped = true;
            }
        },
    )
}

// ------------------------------------------------------------ §4.8

/// Event 4 `0x00599A30` (§4.8; only active records reach it).
fn pick_up<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    ctl.records[i].flags = 0;
    let Some(p) = args.player else { return };
    let code = args.target.and_then(|t| w.item_code(t));
    let read = pf(w, p).get(SLOT, READ);
    let status = match code {
        Some(SCROLL) => (!read).then_some(1),
        Some(AMULET | CUBE | STAFF) => Some(if read { 2 } else { 6 }),
        _ => return,
    };
    // `tr1 ` with 10.3 set: flags := 0 only, no status and no 0x5D
    // (`quests-act2-2.md` §1 item 13).
    let Some(s) = status else { return };
    // Edge case 3: the record's status byte, shared by all players.
    ctl.records[i].status = s;
    status_iterate(ctl, w, i, p);
}

/// Event 5 `0x00599B30` (§4.8).
fn drop<W: QuestWorld>(w: &mut W, args: EventArgs) {
    let Some(p) = args.player else { return };
    let b = match args.target.and_then(|t| w.item_code(t)) {
        Some(AMULET) => TOLD_AMULET,
        Some(CUBE) => TOLD_CUBE,
        Some(STAFF) => TOLD_STAFF,
        _ => return,
    };
    if pf(w, p).get(SLOT, b) {
        clear_bit(w, p, SLOT, b);
    }
}

// ------------------------------------------------------------ §4.9

/// `0x0059E5C0` (§4.9), called by the cube when a transmute places an
/// `hst ` (`world/cube.md` §8 step 3) for `player`. No chain 9 record →
/// nothing.
pub fn staff_assembled<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let g = w.guid(player);
    let x = &mut ctl.records[i].extra.a2.q2;
    x.assembled = true;
    x.assembler = Some(g);
    x.staff_count -= 1;
    x.amulet_count -= 1;
    x.hstaff_count += 1;
    send_flags(w, player);
    // After the 0x28: the client sees 10.11 only with the next one.
    set_bit(w, player, SLOT, ASSEMBLED);
    super::q4::arcane_hook(ctl, w);
}

// ------------------------------------------------------------ §4.10

/// Adds the player's held `msf`, `vip`, `box`, `hst` to the counts
/// (events 13 and 14): one per code at most (`0x00558110` returns the
/// first match, `quests-act2-2.md` §1 item 14).
fn count_held<W: QuestWorld>(ctl: &mut QuestControl, w: &W, i: usize, p: UnitId) {
    let held = |c| i32::from(w.has_item(p, c));
    let x = &mut ctl.records[i].extra.a2.q2;
    x.staff_count += held(STAFF);
    x.amulet_count += held(AMULET);
    x.cube_count += held(CUBE);
    x.hstaff_count += held(HSTAFF);
}

/// Chain 13 is present, not-intro and its staff items were not removed
/// (extra +0x0E ≠ 1).
fn tombs_open(ctl: &QuestControl) -> bool {
    ctl.record(13)
        .is_some_and(|r| r.not_intro && !r.extra.a2.q6.staff_removed)
}

/// Event 13 `0x0059E850` (§4.10).
fn started<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    count_held(ctl, w, i, p);
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) {
        return;
    }
    let lair_closed = ctl
        .record(13)
        .is_some_and(|r| r.not_intro && !r.extra.a2.q6.lair_open);
    if !lair_closed || w.has_item(p, HSTAFF) {
        return;
    }
    for (code, b) in [
        (STAFF, TOLD_STAFF),
        (CUBE, TOLD_CUBE),
        (AMULET, TOLD_AMULET),
    ] {
        if !w.has_item(p, code) {
            clear_bit(w, p, SLOT, b);
        }
    }
}

/// Event 14 `0x0059E970` (§4.10).
fn joined<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    count_held(ctl, w, i, p);
    let x = &ctl.records[i].extra.a2.q2;
    let whole =
        x.hstaff_count > 0 || (x.cube_count != 0 && x.amulet_count != 0 && x.staff_count != 0);
    if x.missing_reported && whole && tombs_open(ctl) {
        if let Some(r) = ctl.record_mut(13) {
            r.extra.a2.q6.missing = false;
        }
    }
}

/// Event 9 `0x0059EA20` (§4.10): one quest item of a leaving player.
fn leaving_item<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    {
        let x = &mut ctl.records[i].extra.a2.q2;
        match args.target.and_then(|t| w.item_code(t)) {
            Some(STAFF) => x.staff_count -= 1,
            Some(AMULET) => x.amulet_count -= 1,
            Some(CUBE) => x.cube_count -= 1,
            Some(HSTAFF) => x.hstaff_count -= 1,
            _ => {}
        }
    }
    let x = &ctl.records[i].extra.a2.q2;
    if x.missing_reported || x.hstaff_count != 0 || !missing(ctl, x) {
        return;
    }
    if tombs_open(ctl) {
        let status = if w.game_type() == 3 { 8 } else { 9 };
        if let Some(r) = ctl.record_mut(13) {
            r.extra.a2.q6.missing = true;
            r.extra.a2.q6.missing_status = status;
        }
    }
    ctl.records[i].extra.a2.q2.missing_reported = true;
}
