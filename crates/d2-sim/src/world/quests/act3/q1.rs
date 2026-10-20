// Spec: specs/world/quests-act3.md §3 (A3Q1 Lam Esen's Tome, chain 15); specs/world/quests-act3-2.md §11.1, §11.3
//! A3Q1: events 0, 2, 3, 4, 5, 9, 10, 11, 13, 14, the status and active
//! functions and the tome object (operate 28, init 23).

use super::{
    add_guid, add_state, guid_listed, in_act3, install, list_remove, npc, npc_of, pf, quick_remove,
    refresh, send_reward_ack, sequence, set, sound, status_all, status_silent, table_state, DOCKS,
};
use crate::units::UnitId;
use crate::world::quests::GuidList;
use crate::world::quests::{
    bit, event, send_player_flags, EventArgs, QuestControl, QuestError, QuestFlags, QuestWorld,
    TextList,
};

const CHAIN: u8 = 15;
const SLOT: u8 = 17;
/// Lam Esen's Tome.
pub const TOME: [u8; 4] = *b"bbb ";
/// `0x0073F188`: table state by record state 0–3 (−1 or > 5 → nothing).
const MSG_STATE: [i8; 4] = [-1, 0, 1, 2];
/// Lower Kurast.
const LOWER_KURAST: u32 = 79;
/// Stat 4 (`statpts`).
const STATPTS: u16 = 4;
/// 17.3, read by event 13 (§3.7). No 1.14d code sets it
/// (`quests-act3-2.md` §11.1, edge case 18): only a save already carrying
/// it reaches that branch, so nothing here writes it.
const SAVED_LEFT_TOWN: u8 = 3;

/// Chain 15's extra data (§3.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the reward is not yet handed out in this game (1 at init).
    pub reward_open: bool,
    /// +0x01: the tome was dropped.
    pub tome_dropped: bool,
    /// +0x02: the tome is active.
    pub tome_active: bool,
    /// +0x04: party scratch (§3.8).
    pub party_scratch: bool,
    /// +0x05: the last tome holder left.
    pub holder_left: bool,
    /// +0x08: tomes in the game.
    pub tomes: i32,
    /// +0x0C: the tome was brought to Alkor (chat end pending).
    pub brought: bool,
    /// +0x10: GUID of the player who brought it.
    pub brought_by: u32,
    /// +0x14: the tome object's GUID.
    pub tome_guid: u32,
    /// +0x18: the tome object's mode.
    pub tome_mode: i32,
    /// +0x1C: tome holders.
    pub holders: GuidList,
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
        event::ITEM_PICKED_UP => {
            // `0x005B7970`.
            if ctl.records[i].not_intro {
                status_all(ctl, w, i, 2);
                ctl.records[i].state = 4;
            }
            if let Some(p) = args.player {
                let g = w.guid(p);
                ctl.records[i].extra.act3.q1.holders.add(g);
            }
        }
        event::ITEM_DROPPED => {
            // `0x005B7A00`.
            if let Some(p) = args.player {
                let g = w.guid(p);
                ctl.records[i].extra.act3.q1.holders.remove(g);
            }
            if ctl.records[i].not_intro {
                ctl.records[i].state = 3;
                status_all(ctl, w, i, 1);
            }
        }
        event::PLAYER_DROPPED_WITH_QUEST_ITEM => {
            // `0x005B7B80`.
            let r = &mut ctl.records[i];
            r.extra.act3.q1.tomes -= 1;
            let x = &r.extra.act3.q1;
            if x.tomes == 0 && r.not_intro && x.tome_active && r.state <= 3 {
                r.extra.act3.q1.holder_left = true;
                status_all(ctl, w, i, 8);
            }
        }
        event::PLAYER_LEAVES_GAME => list_remove(ctl, w, i, &args),
        event::SCROLL_MESSAGE => scroll(ctl, w, i, args),
        event::PLAYER_STARTED_GAME => started(ctl, w, i, args),
        event::PLAYER_JOINED_GAME => {
            // `0x005B7BC0`.
            if args.player.is_some_and(|p| w.has_item(p, TOME)) {
                ctl.records[i].extra.act3.q1.tomes += 1;
            }
            let r = &ctl.records[i];
            if r.extra.act3.q1.tomes == 1 && r.not_intro && r.extra.act3.q1.holder_left {
                ctl.records[i].extra.act3.q1.holder_left = false;
                status_all(ctl, w, i, 2);
            }
        }
        _ => return false,
    }
    true
}

/// Event 0 `0x005B7400` (§3.2).
fn npc_text<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        return;
    }
    let k = if w.has_item(p, TOME) {
        Some(3)
    } else if f.get(SLOT, bit::REWARD_GRANTED) {
        guid_listed(ctl, w, i, p).then_some(4)
    } else {
        let r = &ctl.records[i];
        if r.not_intro && r.state <= 3 {
            table_state(&MSG_STATE, r.state, 5)
        } else {
            None
        }
    };
    if let Some(k) = k {
        add_state(ctl, w, i, list, args.target, k);
    }
}

/// Chat end `0x005B76E0` (§3.7; installed by message 564, never
/// cleared).
fn chat_end<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if npc_of(w, &args) != Some(npc::ALKOR) {
        return;
    }
    let x = &ctl.records[i].extra.act3.q1;
    if x.brought && w.guid(p) == x.brought_by {
        if !matches!(w.player_class(p), 5 | 6) {
            w.attach_sound(p, sound::TOME);
        }
        ctl.records[i].extra.act3.q1.brought = false;
    }
}

/// `0x005B73A0` for every player: players without 17.0 and 17.1 get
/// 17.2 while the state is 2 or 3.
fn flag_iterate<W: QuestWorld>(ctl: &QuestControl, w: &mut W, i: usize) {
    let state = ctl.records[i].state;
    for p in w.players() {
        let f = pf(w, p);
        if !f.get(SLOT, bit::REWARD_GRANTED)
            && !f.get(SLOT, bit::REWARD_PENDING)
            && matches!(state, 2 | 3)
        {
            set(w, p, SLOT, &[bit::STARTED]);
        }
    }
}

/// Event 3 `0x005B7620` (§3.4; a = old level, b = new level).
fn changed_level<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let r = &mut ctl.records[i];
    if args.b == LOWER_KURAST && r.not_intro && r.state == 0 {
        r.state = 1;
    }
    if args.a == DOCKS {
        let Some(p) = args.player else { return };
        quick_remove(ctl, w, i, p);
        if ctl.records[i].state == 2 && !pf(w, p).get(SLOT, bit::REWARD_GRANTED) {
            if ctl.records[i].status != 1 {
                status_all(ctl, w, i, 1);
            }
            ctl.records[i].state = 3;
            flag_iterate(ctl, w, i);
        }
    }
}

/// Event 11 `0x005B7740` (§3.3; a = NPC class, b = message).
fn scroll<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    if args.a != u32::from(npc::ALKOR) || pf(w, p).get(SLOT, bit::REWARD_GRANTED) {
        return;
    }
    match args.b {
        549 => {
            ctl.records[i].state = 2;
            status_all(ctl, w, i, 1);
            flag_iterate(ctl, w, i);
            refresh(ctl, w, p, &args);
        }
        564 => {
            refresh(ctl, w, p, &args);
            if w.has_item(p, TOME) {
                w.delete_item(p, TOME);
                let g = w.guid(p);
                let x = &mut ctl.records[i].extra.act3.q1;
                x.brought = true;
                x.brought_by = g;
                install(ctl, i, event::NPC_DEACTIVATE);
            }
            if ctl.records[i].extra.act3.q1.reward_open {
                // `0x005B74E0` from the NPC (edge case 1: every player in
                // Act III, whether or not a tome was handed in).
                for q in w.players() {
                    let f = pf(w, q);
                    if in_act3(w, q)
                        && !f.get(SLOT, bit::REWARD_GRANTED)
                        && !f.get(SLOT, bit::REWARD_PENDING)
                    {
                        set(
                            w,
                            q,
                            SLOT,
                            &[
                                bit::PRIMARY_GOAL_DONE,
                                bit::REWARD_GRANTED,
                                bit::REWARD_PENDING,
                            ],
                        );
                        send_player_flags(w, q, 6, 0);
                    }
                }
                ctl.records[i].extra.act3.q1.reward_open = false;
                // `0x005B7580`: the stat points.
                for q in w.players() {
                    if pf(w, q).get(SLOT, bit::REWARD_PENDING) {
                        w.add_stat(q, STATPTS, 5);
                        send_reward_ack(w, q, CHAIN);
                        super::clear(w, q, SLOT, &[bit::REWARD_PENDING]);
                    }
                }
                // `0x005B75D0`: the others get 17.14, nothing sent.
                for q in w.players() {
                    let f = pf(w, q);
                    if !f.get(SLOT, bit::REWARD_GRANTED) && !f.get(SLOT, bit::REWARD_PENDING) {
                        set(w, q, SLOT, &[bit::COMPLETED_NOW]);
                    }
                }
            }
            // Literal reading: the tail runs after the reward test
            // whether or not the reward was still open.
            status_silent(ctl, i, 13);
            add_guid(ctl, w, i, p);
            if pf(w, p).get(SLOT, bit::PRIMARY_GOAL_DONE) && ctl.records[i].not_intro {
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                if ctl.records[i].state != 5 {
                    ctl.records[i].state = 5;
                    sequence(ctl, w, CHAIN);
                }
            }
        }
        _ => {}
    }
}

/// Event 13 `0x005B7C10` (§3.7).
fn started<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize, args: EventArgs) {
    let Some(p) = args.player else { return };
    let f = pf(w, p);
    if f.get(SLOT, bit::REWARD_GRANTED) {
        ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
    } else if w.has_item(p, TOME) {
        status_silent(ctl, i, 2);
        let g = w.guid(p);
        let r = &mut ctl.records[i];
        r.state = 4;
        let x = &mut r.extra.act3.q1;
        x.tomes += 1;
        x.tome_mode = 2;
        x.tome_active = true;
        x.holders.add(g);
    } else if f.get(SLOT, SAVED_LEFT_TOWN) {
        ctl.records[i].state = 3;
        status_silent(ctl, i, 1);
    } else if f.get(SLOT, bit::STARTED) {
        ctl.records[i].state = 2;
        status_silent(ctl, i, 1);
    }
}

/// Active function `0x005B72B0` ("wants to talk", §3.2).
pub(super) fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
) -> bool {
    if npc_class != npc::ALKOR {
        return false;
    }
    let f = pf(w, player);
    (ctl.records[i].state == 1
        && !f.get(SLOT, bit::REWARD_GRANTED)
        && !f.get(SLOT, bit::COMPLETED_BEFORE))
        || w.has_item(player, TOME)
}

/// Status function `0x005B7D70` (§3.8, always reports).
pub(super) fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    r: &QuestFlags,
) -> u8 {
    if !r.get(15, bit::REWARD_GRANTED) {
        return 0;
    }
    if r.get(SLOT, bit::REWARD_GRANTED) {
        return 11 + 2 * u8::from(r.get(SLOT, bit::PRIMARY_GOAL_DONE));
    }
    if w.has_item(player, TOME) {
        return 2;
    }
    // +0x04 is a scratch the status function owns; the record is read
    // only here, so it lives in a local.
    if let Some(members) = w.party_members(player) {
        let mut scratch = false;
        for m in members {
            if w.has_item(m, TOME) {
                scratch = true;
            }
        }
        if scratch {
            return 2;
        }
    }
    let rec = &ctl.records[i];
    let x = &rec.extra.act3.q1;
    if !rec.not_intro {
        0
    } else if rec.state < 4 {
        if x.tome_active && x.tomes == 0 {
            8 + u8::from(w.game_type() != 3)
        } else {
            1
        }
    } else if in_act3(w, player) && x.tomes != 0 {
        1
    } else {
        12
    }
}

/// Init 23 (`0x00544E30` → `0x005B7310`): chain 15 lookup (fatal when
/// absent), then the mode.
pub fn tome_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, object: UnitId) {
    let Some(i) = ctl.find(CHAIN) else {
        ctl.faults.push(QuestError::Fatal(0x0054_4E30));
        return;
    };
    let mode = if ctl.records[i].not_intro {
        ctl.records[i].extra.act3.q1.tome_mode
    } else {
        2
    };
    w.set_object_mode(object, mode);
}

/// Operate 28 `0x005B7A60` (returns 0).
pub fn tome_operate<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    object: UnitId,
    player: UnitId,
) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if !ctl.records[i].not_intro || w.object_mode(object) != 0 {
        return;
    }
    if pf(w, player).get(SLOT, bit::REWARD_GRANTED) {
        w.attach_sound(player, sound::REFUSED);
        return;
    }
    // Item level: the area level of the object's level (§11.3).
    if w.quest_drop(object, TOME, 2, None, false).is_none() {
        return;
    }
    w.set_object_mode(object, 2);
    let g = w.guid(object);
    let x = &mut ctl.records[i].extra.act3.q1;
    x.tomes += 1;
    x.tome_active = true;
    x.tome_dropped = true;
    x.tome_mode = 2;
    x.tome_guid = g;
    install(ctl, i, event::NPC_ACTIVATE);
    install(ctl, i, event::SCROLL_MESSAGE);
    if ctl.records[i].state != 3 {
        ctl.records[i].state = 3;
    }
    if ctl.records[i].status != 1 {
        status_silent(ctl, i, 1);
    }
}
