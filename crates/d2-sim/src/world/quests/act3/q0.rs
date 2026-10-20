// Spec: specs/world/quests-act3.md §9.1 (A3Q0 Hratli gossip, chain 14); specs/world/quests-act3-2.md §11.6, §11.7
//! A3Q0: events 0, 11, 13, the active function and Hratli's start / end
//! dummies (init 49, 50).

use super::{add_state, npc, pf, set, InitPoint};
use crate::units::UnitId;
use crate::world::quests::{bit, event, EventArgs, QuestControl, QuestWorld, TextList};

const CHAIN: u8 = 14;
const SLOT: u8 = 16;
/// The sorceress' player class (table state 1).
const SORCERESS: u8 = 1;

/// Chain 14's extra data (§9.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {
    /// +0x00: the start Hratli was spawned.
    pub start_spawned: bool,
    /// +0x01: the end Hratli was spawned.
    pub end_spawned: bool,
    /// +0x02: the end object was seen.
    pub end_seen: bool,
    /// +0x03: the start Hratli is present.
    pub start_present: bool,
    /// +0x04 / +0x08: the end position.
    pub end_x: i32,
    pub end_y: i32,
    /// +0x0C: Hratli's GUID.
    pub hratli_guid: u32,
    /// +0x10: the map AI was applied.
    pub ai_applied: bool,
    /// +0x18: a map AI is stored. Only `0x005B7230` writes it and it has
    /// no caller in 1.14d (`quests-act3-2.md` §11.6), so it stays false.
    pub map_ai: bool,
}

fn x0(ctl: &mut QuestControl, i: usize) -> &mut Extra {
    &mut ctl.records[i].extra.act3.q0
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
        event::NPC_ACTIVATE => {
            // `0x005B6F20`.
            let Some(p) = args.player else { return true };
            if super::npc_of(w, &args) != Some(npc::HRATLI)
                || pf(w, p).get(SLOT, bit::REWARD_GRANTED)
            {
                return true;
            }
            let k = u8::from(w.player_class(p) == SORCERESS);
            add_state(ctl, w, i, list, args.target, k);
        }
        event::SCROLL_MESSAGE => {
            // `0x005B6ED0`.
            if args.a == u32::from(npc::HRATLI) && matches!(args.b, 465 | 466) {
                if let Some(p) = args.player {
                    set(w, p, SLOT, &[bit::REWARD_GRANTED]);
                }
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
            }
        }
        event::PLAYER_STARTED_GAME => {
            // `0x005B6FD0`.
            if args
                .player
                .is_some_and(|p| pf(w, p).get(SLOT, bit::REWARD_GRANTED))
            {
                ctl.game.set(SLOT, bit::PRIMARY_GOAL_DONE);
            }
        }
        _ => return false,
    }
    true
}

/// Active `0x005B6FA0`: hratli with 16.0 clear.
pub(super) fn active<W: QuestWorld>(w: &mut W, player: UnitId, npc_class: u16) -> bool {
    npc_class == npc::HRATLI && !pf(w, player).get(SLOT, bit::REWARD_GRANTED)
}

/// Hratli (+0x0C) exists.
fn hratli_exists<W: QuestWorld>(w: &W, guid: u32) -> bool {
    w.monster_by_guid(guid).is_some()
}

/// `0x005B2F20(game, room, x, y, 253, mode 1, −1, 0)` at the dummy's
/// init record (room, x, y) (`quests-act3-2.md` §11.7 rule 3).
fn spawn_hratli<W: QuestWorld>(w: &mut W, at: InitPoint) -> Option<UnitId> {
    w.spawn_monster(at.room, at.x, at.y, npc::HRATLI, 1, u32::MAX)
}

/// Init 49 `0x005B70B0` (start dummy 378); `at` is the object's init
/// record.
pub fn hratli_start_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, at: InitPoint) {
    let Some(i) = ctl.find(CHAIN) else { return };
    if ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE) {
        return;
    }
    let x = &ctl.records[i].extra.act3.q0;
    if x.start_spawned && hratli_exists(w, x.hratli_guid) {
        return;
    }
    if let Some(h) = spawn_hratli(w, at) {
        let g = w.guid(h);
        let x = x0(ctl, i);
        x.hratli_guid = g;
        x.start_spawned = true;
    }
}

/// Init 50 `0x005B7160` (end dummy 379); `at` is the object's init
/// record: +0x04 / +0x08 := its (x, y) before the game-flag test
/// (`quests-act3-2.md` §11.7 rule 3).
pub fn hratli_end_init<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, at: InitPoint) {
    let Some(i) = ctl.find(CHAIN) else { return };
    {
        let x = x0(ctl, i);
        x.end_seen = true;
        x.end_x = at.x;
        x.end_y = at.y;
    }
    if !ctl.game.get(SLOT, bit::PRIMARY_GOAL_DONE) || ctl.records[i].extra.act3.q0.end_spawned {
        return;
    }
    // "Hratli from the start exists": the init-49 test (+0x00 and the
    // +0x0C unit).
    let x = &ctl.records[i].extra.act3.q0;
    if x.start_spawned && hratli_exists(w, x.hratli_guid) {
        x0(ctl, i).start_present = true;
        return;
    }
    let Some(h) = spawn_hratli(w, at) else {
        return;
    };
    let g = w.guid(h);
    {
        let x = x0(ctl, i);
        x.hratli_guid = g;
        x.end_spawned = true;
    }
    w.or_unit_flags(h, 0x0300_0000);
    // The map AI (+0x18) is stored only by `0x005B7230`, which has no
    // caller in 1.14d (edge cases 17, 20; `quests-act3-2.md` §11.6): this
    // branch is dead there and `0x0058F000` is never reached.
    let x = x0(ctl, i);
    if x.map_ai && !x.ai_applied {
        x.ai_applied = true;
        w.unhandled(CHAIN, 0x0058_F000);
    }
}
