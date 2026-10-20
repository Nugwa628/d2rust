// Spec: specs/world/quests-act1.md §10.5 (A1Q3 Tools of the Trade, chain 3)
// Spec: specs/world/quests-act1-rest.md §8 item 5
// Spec: specs/world/quests.md (the sections other than §10)
//! A1Q3 callback by callback: the Malus object's init and operate
//! functions, events 0, 2, 3, 4, 6, 9, 10, 11, 13, 14, the status and
//! active functions, the reset `0x005918D0` and the imbue grant
//! `0x00591790`, with the iterate functions K2–K4 (K1 is the shared
//! status iterate). Slot 3 is a constant in each.

use super::{add_state, broadcast, player_flags, rec, sequence, status_all, table_state};
use crate::units::UnitId;
use crate::world::quests::{
    bit, event, flags_of, npc, send_player_flags, EventArgs, QuestControl, QuestError, QuestFlags,
    QuestWorld, TextList,
};

const SLOT: u8 = 3;
const CHAIN: u8 = 3;
/// The Horadric Malus.
pub const MALUS: [u8; 4] = *b"hdm ";
/// The Malus object's class (the reset's test).
pub const MALUS_OBJECT: u16 = 108;
/// `0x00737630`: message state by quest state 0–5.
const MSG_STATE: [i8; 6] = [-1, 0, 1, 2, 3, 4];
/// The Malus object mode once taken.
const TAKEN: i32 = 2;
/// Sound event of a refused operate.
const SOUND_REFUSED: u16 = 19;

/// "level": the player's base stat 12, layer 0 (`0x006253B0`).
fn level<W: QuestWorld>(w: &W, p: UnitId) -> i32 {
    w.base_stat(p, 12)
}

/// Dispatches chain 3's callbacks; false = no body (unhandled).
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
        event::ITEM_PICKED_UP => {
            // `0x00591960`.
            if let Some(p) = args.player {
                if let Some(f) = flags_of(w, p) {
                    if !f.get(SLOT, 6) {
                        f.set(SLOT, 6);
                        w.attach_sound(p, 36);
                    }
                }
            }
            broadcast(ctl, w, i, 2, 0);
        }
        event::EVENT6 => {
            // `0x00591A90` (never raised in 1.14d, §4.1).
            let r = &mut ctl.records[i];
            if r.not_intro {
                r.extra.malus_items -= 1;
                if r.extra.malus_items == 0 && r.extra.malus_mode == TAKEN {
                    reset(ctl, w, i);
                }
            }
        }
        event::PLAYER_DROPPED_WITH_QUEST_ITEM => {
            // `0x00591A20`.
            let r = &mut ctl.records[i];
            if r.not_intro && r.state != 5 {
                r.extra.malus_items -= 1;
                if r.extra.malus_items == 0 && r.extra.malus_known && r.extra.malus_mode == TAKEN {
                    broadcast(ctl, w, i, 3, 0);
                }
            }
        }
        event::PLAYER_LEAVES_GAME => {
            // `0x00591A60`.
            let g = args.player.map_or(u32::MAX, |p| w.guid(p));
            ctl.records[i].extra.guids.remove(g);
        }
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => {
            // `0x00591ED0`.
            let Some(p) = args.player else { return true };
            if w.has_item(p, MALUS) {
                let x = &mut ctl.records[i].extra;
                x.malus_items += 1;
                x.started_with_malus = true;
            }
            let f = player_flags(w, p);
            if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::COMPLETED_BEFORE) {
                let r = &mut ctl.records[i];
                if f.get(SLOT, bit::STARTED) {
                    (r.state, r.status) = (2, 1);
                } else if f.get(SLOT, bit::LEAVE_TOWN) {
                    (r.state, r.status) = (3, 1);
                }
            }
        }
        event::PLAYER_JOINED_GAME => {
            // `0x005919D0`.
            let Some(p) = args.player else { return true };
            let r = &mut ctl.records[i];
            if w.has_item(p, MALUS) && r.not_intro && r.state != 5 {
                r.extra.malus_items += 1;
                if r.extra.malus_items == 1 && r.extra.malus_known && r.extra.malus_mode == TAKEN {
                    r.status = 2;
                }
            }
        }
        _ => return false,
    }
    true
}

/// K2 `0x00591340` for every player.
fn iterate_progress<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let state = ctl.records[i].state;
    for p in w.players() {
        let Some(f) = flags_of(w, p) else { continue };
        if f.get(SLOT, bit::REWARD_GRANTED) || f.get(SLOT, bit::REWARD_PENDING) {
            continue;
        }
        match state {
            2 => f.set(SLOT, bit::STARTED),
            3 | 4 => f.set(SLOT, bit::LEAVE_TOWN),
            _ => {}
        }
    }
}

/// K4 `0x00591CD0`: the player, or a member of its party, has the Malus
/// (`0x00591CA0` per member). The original keeps the party result in
/// the scratch byte extra +0xA0, which only K4 reads; it is not kept.
fn party_has_malus<W: QuestWorld>(w: &W, p: UnitId) -> bool {
    if w.has_item(p, MALUS) {
        return true;
    }
    w.party_members(p)
        .is_some_and(|members| members.iter().any(|&m| w.has_item(m, MALUS)))
}

/// Event 0 `0x005916A0`.
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    // `quests-act1-rest.md` §8 item 5: the player's data is read first
    // (`0x006221A0` at `0x005916B3`), an internal error without one.
    let Some(p) = args.player else {
        return ctl.faults.push(QuestError::Fatal(0x0059_16B3));
    };
    let r = player_flags(w, p);
    if r.get(SLOT, bit::REWARD_GRANTED) && !r.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        return;
    }
    if w.has_item(p, MALUS) {
        if level(w, p) >= 8 && !r.get(SLOT, bit::REWARD_GRANTED) {
            add_state(ctl, w, i, list, args.target, 3);
        }
        return;
    }
    let state = ctl.records[i].state;
    if r.get(SLOT, bit::REWARD_GRANTED) || state == 0 || state == 4 {
        return;
    }
    if let Some(m) = table_state(&MSG_STATE, state) {
        add_state(ctl, w, i, list, args.target, m);
    }
}

/// Event 2 `0x005913C0`.
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if args.target.and_then(|n| w.monster_class(n)) != Some(npc::CHARSI) {
        return;
    }
    if ctl.records[i].extra.talked {
        iterate_progress(ctl, w, i);
        broadcast(ctl, w, i, 1, 0);
        ctl.records[i].extra.talked = false;
    } else if ctl.records[i].extra.rewarded {
        broadcast(ctl, w, i, 13, 0);
        ctl.records[i].extra.rewarded = false;
    }
}

/// Event 3 `0x00591810` (a = old level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    if !ctl.records[i].not_intro {
        ctl.records[i].clear_callback(event::CHANGED_LEVEL);
        return;
    }
    let r = rec(w, args.player);
    if args.a == 1
        && ctl.records[i].state == 2
        && !r.get(SLOT, bit::REWARD_GRANTED)
        && !r.get(SLOT, bit::REWARD_PENDING)
    {
        if ctl.records[i].status != 1 {
            broadcast(ctl, w, i, 1, 0);
        }
        ctl.records[i].state = 3;
        iterate_progress(ctl, w, i);
        ctl.records[i].clear_callback(event::CHANGED_LEVEL);
    }
}

/// Event 11 `0x00591490` (a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a != u32::from(npc::CHARSI) {
        return;
    }
    let refresh = |ctl: &mut QuestControl, w: &mut W| {
        if let Some(n) = args.target {
            ctl.refresh_text(w, p, n);
        }
    };
    match args.b {
        146 => {
            ctl.records[i].state = 2;
            ctl.records[i].extra.talked = true;
            refresh(ctl, w);
        }
        163 => {
            if player_flags(w, p).get(SLOT, bit::REWARD_GRANTED) {
                return;
            }
            let g = w.guid(p);
            ctl.records[i].extra.guids.add(g);
            if !w.has_item(p, MALUS) {
                return refresh(ctl, w);
            }
            if let Some(f) = flags_of(w, p) {
                f.set(SLOT, bit::PRIMARY_GOAL_DONE);
                f.set(SLOT, bit::REWARD_PENDING);
            }
            send_player_flags(w, p, 6, 0);
            w.delete_item(p, MALUS);
            ctl.records[i].extra.malus_items -= 1;
            if let Some(members) = w.party_members(p) {
                for m in members {
                    // K3 `0x00591430`.
                    let lvl = level(w, m);
                    let Some(f) = flags_of(w, m) else { continue };
                    if !f.get(SLOT, bit::REWARD_GRANTED)
                        && !f.get(SLOT, bit::REWARD_PENDING)
                        && lvl >= 8
                    {
                        f.set(SLOT, bit::PRIMARY_GOAL_DONE);
                        f.set(SLOT, bit::REWARD_PENDING);
                    }
                }
            }
            if ctl.records[i].not_intro {
                if ctl.records[i].state == 4 {
                    let r = &mut ctl.records[i];
                    r.state = 5;
                    r.extra.rewarded = true;
                    ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                    sequence(ctl, w, CHAIN);
                } else if ctl.records[i].extra.started_with_malus {
                    if let Some(next) = ctl.records[i].seq_id.filter(|&s| ctl.find(s).is_some()) {
                        sequence(ctl, w, next);
                    }
                }
            }
            refresh(ctl, w);
        }
        _ => {}
    }
}

/// Status `0x00591D30` (§6.1; always reports).
pub(super) fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    r: &QuestFlags,
) -> u8 {
    if r.get(SLOT, bit::REWARD_PENDING) {
        return 10;
    }
    if party_has_malus(w, player) {
        return if r.get(SLOT, bit::REWARD_GRANTED) {
            0
        } else {
            2
        };
    }
    let rd = &ctl.records[i];
    if !rd.not_intro {
        0
    } else if r.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        13
    } else if r.get(SLOT, bit::COMPLETED_NOW) {
        12
    } else if rd.state < 5 {
        rd.status
    } else if ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        if level(w, player) >= 8 {
            12
        } else {
            4
        }
    } else {
        0
    }
}

/// Active `0x00591C30` (§6.4).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    let r = player_flags(w, player);
    let state = ctl.records[i].state;
    npc_class == npc::CHARSI
        && !r.get(SLOT, bit::REWARD_GRANTED)
        && ((state == 1 && !r.get(SLOT, bit::REWARD_PENDING))
            || (level(w, player) >= 8 && w.has_item(player, MALUS)))
}

/// Reset `0x005918D0` (from event 6 only). Returns its result.
fn reset<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if !ctl.records[i].not_intro {
        return false;
    }
    ctl.records[i].extra.malus_mode = 0;
    if !ctl.records[i].extra.malus_known {
        return false;
    }
    ctl.records[i].state = 3;
    ctl.records[i].flags = 1;
    status_all(ctl, w, i, 1);
    ctl.records[i].extra.b08 = 0;
    match w.object_by_guid(ctl.records[i].extra.malus_guid) {
        Some((o, MALUS_OBJECT)) => w.set_object_mode(o, 0),
        _ => ctl.records[i].extra.malus_known = false,
    }
    true
}

/// The Malus object's init (pointer at `0x00731BFC` → `0x00544950`).
pub fn malus_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else {
        if w.object_mode(object) != TAKEN {
            w.set_object_mode(object, TAKEN);
        }
        return;
    };
    let g = w.guid(object);
    let r = &mut ctl.records[i];
    r.extra.malus_known = true;
    r.extra.malus_guid = g;
    if !r.not_intro {
        r.extra.malus_mode = TAKEN;
    }
    let mode = r.extra.malus_mode;
    w.set_object_mode(object, mode);
}

/// Operate `0x00591AC0` (pointer at `0x00732D6C`; returns 0).
pub fn malus_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let mode = w.object_mode(object);
    if !ctl.records[i].not_intro {
        w.set_object_mode(object, TAKEN);
        w.attach_sound(player, SOUND_REFUSED);
        return;
    }
    let r = player_flags(w, player);
    if mode != 0 || r.get(SLOT, bit::REWARD_GRANTED) || r.get(SLOT, bit::REWARD_PENDING) {
        return;
    }
    if level(w, player) < 8 {
        w.attach_sound(player, SOUND_REFUSED);
        return;
    }
    if !w.drop_item_at(object, MALUS, 2) {
        return;
    }
    w.set_object_mode(object, TAKEN);
    let g = w.guid(object);
    let x = &mut ctl.records[i].extra;
    x.malus_known = true;
    x.malus_mode = TAKEN;
    x.malus_items += 1;
    x.malus_guid = g;
    if ctl.records[i].state != 4 {
        ctl.records[i].state = 4;
        iterate_progress(ctl, w, i);
    }
    if ctl.records[i].status != 1 {
        ctl.records[i].flags = 0;
        ctl.records[i].status = 1;
    }
}

/// Imbue granted `0x00591790` (from the NPC imbue, `world/npc.md`).
pub fn imbue_granted<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, player: UnitId) {
    let Some(f) = flags_of(w, player) else { return };
    f.set(SLOT, bit::REWARD_GRANTED);
    f.clear(SLOT, bit::REWARD_PENDING);
    if !f.get(SLOT, bit::COMPLETED_BEFORE) {
        match ctl.record_mut(CHAIN) {
            Some(r) => r.active = false,
            None => ctl.faults.push(QuestError::Fatal(0x0059_1790)),
        }
    }
}
