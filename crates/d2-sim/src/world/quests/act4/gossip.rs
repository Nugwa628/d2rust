// Spec: specs/world/quests-act4.md §6 (Act IV gossip records)
//! A4Q0 Tyrael (chain 21, slot 24) and A4Q4 Hadriel (chain 29, slot 33)
//! gossip records: their event 0 and 11 callbacks, active and status
//! functions (§6.1, §6.2). Halbu, Jamella and the rest have no quest
//! record (§6.3).

use super::super::late::{add_state, flags, set};
use super::super::{bit, event, npc, EventArgs, QuestControl, QuestFlags, QuestWorld, TextList};
use crate::units::UnitId;

/// Tyrael's gossip slot.
const TYRAEL_SLOT: u8 = 24;
/// Hell's Forge and Terror's End slots (Hadriel's tests).
const FORGE_SLOT: u8 = 27;
const TERROR_SLOT: u8 = 26;
/// `malachai` (Hadriel).
pub const HADRIEL: u16 = 408;
/// Tyrael's gossip message.
const MSG_TYRAEL: u32 = 664;

/// Quest extra data: none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extra {}

/// Timers these records make: none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {}

/// Init (`0x005B3780`, `0x005B6A70`) beyond `quests.tsv`: state 0, no
/// extra data (§2).
pub fn init(r: &mut super::super::QuestRecord) {
    let _ = r;
}

/// One callback of record `i`; false = not handled (reported).
pub fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match (ctl.records[i].chain, args.event) {
        (21, event::NPC_ACTIVATE) => {
            // `0x005B36D0`.
            let Some(p) = args.player else { return true };
            let class = args.target.and_then(|n| w.monster_class(n));
            if class == Some(npc::TYRAEL2) && !flags(w, p).get(TYRAEL_SLOT, bit::REWARD_GRANTED) {
                add_state(ctl, i, list, npc::TYRAEL2, 0);
            }
        }
        (21, event::SCROLL_MESSAGE) => {
            // `0x005B36A0`.
            if args.a == u32::from(npc::TYRAEL2) && args.b == MSG_TYRAEL {
                if let Some(p) = args.player {
                    set(w, p, TYRAEL_SLOT, bit::REWARD_GRANTED);
                }
            }
        }
        (29, event::NPC_ACTIVATE) => hadriel_chat(ctl, w, i, args, list),
        _ => return false,
    }
    true
}

/// "27.0, 27.1 and 27.13 all clear".
fn forge_untouched(f: &QuestFlags) -> bool {
    !f.get(FORGE_SLOT, bit::REWARD_GRANTED)
        && !f.get(FORGE_SLOT, bit::REWARD_PENDING)
        && !f.get(FORGE_SLOT, bit::PRIMARY_GOAL_DONE)
}

/// Chain 23's extra +0x14 (Diablo killed in this game, `quests-act4.md`
/// §5.1).
fn diablo_killed(ctl: &QuestControl, j: usize) -> bool {
    ctl.records[j].extra.a4.q2.killed
}

/// Event 0 `0x005B6940` (§6.2, edge case 17).
fn hadriel_chat<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) {
    let Some(p) = args.player else { return };
    if args.target.and_then(|n| w.monster_class(n)) != Some(HADRIEL) {
        return;
    }
    if forge_untouched(&flags(w, p)) {
        add_state(ctl, i, list, HADRIEL, 0);
    } else if ctl.find(23).is_none_or(|j| !diablo_killed(ctl, j)) {
        add_state(ctl, i, list, HADRIEL, 1);
    }
}

/// Active functions: Tyrael `0x005B3750`, Hadriel `0x005B69F0`.
pub fn active<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    npc_class: u16,
    f: u32,
) -> bool {
    let pf = flags(w, player);
    match ctl.records[i].chain {
        21 => npc_class == npc::TYRAEL2 && !pf.get(TYRAEL_SLOT, bit::REWARD_GRANTED),
        29 => {
            npc_class == HADRIEL
                && (forge_untouched(&pf)
                    || (!pf.get(TERROR_SLOT, bit::PRIMARY_GOAL_DONE)
                        && !pf.get(TERROR_SLOT, bit::REWARD_GRANTED)))
        }
        c => {
            w.unhandled(c, f);
            false
        }
    }
}

/// Status functions `0x005B3740` (chain 21) and `0x005B69E0` (chain 29):
/// both return false (nothing reported).
pub fn status<W: QuestWorld>(
    ctl: &QuestControl,
    w: &mut W,
    i: usize,
    player: UnitId,
    pf: &QuestFlags,
    f: u32,
) -> Option<u8> {
    let _ = (player, pf);
    match ctl.records[i].chain {
        21 | 29 => None,
        c => {
            w.unhandled(c, f);
            None
        }
    }
}

/// Runs a timer; true = remove it.
pub fn run_timer<W: QuestWorld>(ctl: &mut QuestControl, w: &mut W, t: Timer, chain: u8) -> bool {
    let _ = (ctl, w, chain);
    match t {}
}

#[cfg(test)]
#[path = "gossip_tests.rs"]
mod tests;
