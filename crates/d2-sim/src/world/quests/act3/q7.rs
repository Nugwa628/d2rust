// Spec: specs/world/quests-act3.md §9.2 (A3Q7 Dark Wanderer, chain 28); specs/world/quests-act3-2.md §11.7
//! A3Q7: event 13, the wanderer object (init 43), its walk target and
//! the minion hook and timer.

use super::{in_act3, npc, pf, set, InitPoint, Timer};
use crate::units::UnitId;
use crate::world::quests::{bit, event, EventArgs, QuestControl, QuestWorld, TextList, TimerFn};

const CHAIN: u8 = 28;
/// The record's table slot (it reports in slot 16).
const SLOT: u8 = 32;
/// The minion dummy object.
const MINION_DUMMY: u16 = 131;
/// `0x00741538`: minion offsets; (−3, 3) twice, no (3, 3) (edge case
/// 15).
pub const MINION_OFFSETS: [(i32, i32); 8] = [
    (-3, -3),
    (-3, 0),
    (-3, 3),
    (0, -3),
    (0, 3),
    (3, -3),
    (3, 0),
    (-3, 3),
];
/// The walk-target candidates relative to the stored target, in order;
/// (0, −3) when all are blocked.
const TARGET_TRIES: [(i32, i32); 5] = [(0, -20), (0, -11), (2, -11), (-2, -11), (0, -8)];

/// Chain 28's extra data (§9.2).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the wanderer object was seen.
    pub seen: bool,
    /// +0x01: the wanderer is still to spawn (1 at init).
    pub to_spawn: bool,
    /// +0x02: the walk target is fixed; +0x04 / +0x08 the target.
    pub target_fixed: bool,
    pub target_x: i32,
    pub target_y: i32,
    /// +0x0C: minions spawned.
    pub minions: bool,
    /// +0x0D: the minion timer exists.
    pub timer: bool,
    /// +0x10: the wanderer's GUID.
    pub wanderer_guid: u32,
}

fn x7(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.act3.q7
}

/// Dispatches the chain's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    let _ = list;
    match args.event {
        event::PLAYER_STARTED_GAME => {
            // `0x005BD2C0`.
            if args
                .player
                .is_some_and(|p| pf(w, p).get(SLOT, bit::REWARD_GRANTED))
            {
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
                x7(ctl, i).to_spawn = false;
            }
            true
        }
        _ => false,
    }
}

/// Init 43 `0x005BD1F0` (object 368); `at` is the object's init record
/// (`quests-act3-2.md` §11.7 rule 4). With +0x01: target := (x + 7, y);
/// its room is `0x00463740(init room, x + 7, y)`. No room → the spawn
/// `0x005B2F20` fails before any draw and +0x01 stays 1, so a later init
/// tries again (edge case 21). +0x00 := 1 in every case.
pub fn wanderer_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, at: InitPoint) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if ctl.records[i].extra.act3.q7.to_spawn {
        let (tx, ty) = (at.x + 7, at.y);
        {
            let e = x7(ctl, i);
            e.target_x = tx;
            e.target_y = ty;
        }
        if let Some(r) = w.room_at(at.room, tx, ty) {
            if w.spawn_monster(r, tx, ty, npc::DARK_WANDERER, 1, u32::MAX)
                .is_some()
            {
                x7(ctl, i).to_spawn = false;
            }
        }
    }
    x7(ctl, i).seen = true;
}

/// `0x005BD0D0` (the wanderer's AI): the walk target. The collision
/// tests `0x006229F0` use the wanderer's own room; a wanderer without a
/// room tests every point free, so the first candidate (X, Y − 20)
/// becomes the target (`quests-act3-2.md` §11.7 rule 5, part 2 edge
/// case 1).
pub fn wanderer_target<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    wanderer: UnitId,
) -> Option<(i32, i32)> {
    let i = ctl.find(CHAIN)?;
    let e = &ctl.records[i].extra.act3.q7;
    if !e.seen {
        return None;
    }
    if e.target_fixed {
        return Some((e.target_x, e.target_y));
    }
    let (x, y) = (e.target_x, e.target_y);
    let room = w.unit_position(wanderer).map(|(_, _, r)| r);
    x7(ctl, i).target_fixed = true;
    let (tx, ty) = TARGET_TRIES
        .iter()
        .map(|&(dx, dy)| (x + dx, y + dy))
        .find(|&(tx, ty)| room.is_none_or(|r| !w.blocked(r, tx, ty, 0x3C01)))
        .unwrap_or((x, y - 3));
    let e = x7(ctl, i);
    e.target_x = tx;
    e.target_y = ty;
    Some((tx, ty))
}

/// `0x005BD4A0` (the wanderer's AI): the minion hook.
pub fn wanderer_minions<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, wanderer: UnitId) {
    let Some(i) = ctl.find(CHAIN) else { return };
    let e = &ctl.records[i].extra.act3.q7;
    if e.minions || e.timer {
        return;
    }
    let g = w.guid(wanderer);
    {
        let e = x7(ctl, i);
        e.timer = true;
        e.wanderer_guid = g;
    }
    if let Err(err) = ctl.add_timer(CHAIN, TimerFn::Act3(Timer::WandererMinions), 2) {
        ctl.faults.push(err);
    }
}

/// Timer `0x005BD390`; returns 1. The free-spot search starts in the
/// wanderer's room from its position + offset; dummy 131 goes into the
/// room the search returns. A wanderer without a room finds no spot (no
/// dummies); the bits, +0x0C, +0x0D and the quest-seed draw still happen
/// (`quests-act3-2.md` §11.7 rule 6).
pub(super) fn minion_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, i: usize) -> bool {
    if ctl.records[i].extra.act3.q7.minions {
        return true;
    }
    // `0x005BD260` for each player.
    for p in w.players() {
        if !pf(w, p).get(SLOT, bit::REWARD_GRANTED) && in_act3(w, p) {
            set(w, p, SLOT, &[bit::REWARD_GRANTED]);
        }
    }
    let g = {
        let e = x7(ctl, i);
        e.minions = true;
        e.timer = false;
        e.wanderer_guid
    };
    let Some((wanderer, _)) = w.monster_by_guid(g) else {
        return true;
    };
    let lo = ctl.seed.step();
    let Some((x, y, room)) = w.unit_position(wanderer) else {
        return true;
    };
    for &(dx, dy) in &MINION_OFFSETS[(lo & 1) as usize..] {
        // Radius 11 is passed but never read by `0x00545340`.
        if let Some((sx, sy, sr)) = w.free_spot_at(room, x + dx, y + dy, 3, 0x3F11, 11, 100) {
            w.spawn_quest_object(sr, sx, sy, MINION_DUMMY);
        }
    }
    true
}
