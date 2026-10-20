// Spec: specs/world/quests-act3.md §9.3 (Act III intro, chain 39)
//! The Act III intro record: per-NPC first-talk text kept in the
//! player's NPC intro record (events 0, 11 and the active function).

use super::{add_state, npc};
use crate::units::UnitId;
use crate::world::quests::{event, EventArgs, QuestControl, QuestWorld, TextList};

/// (NPC class, the player class with special text, the event-11
/// messages that set the intro bit). Class 7 matches no player.
const INTROS: [(u16, u8, &[u32]); 6] = [
    (npc::CAIN3, 7, &[458]),
    (npc::ASHEARA, 0, &[490, 491]),
    (npc::ALKOR, 2, &[501, 502]),
    (npc::ORMUS, 3, &[514, 515]),
    (npc::MESHIF2, 4, &[478, 479]),
    (npc::NATALYA, 7, &[453]),
];

/// The intro bit (`0x005723C0`) of `class` for the game difficulty.
fn heard<W: QuestWorld>(w: &mut W, player: UnitId, class: u16) -> bool {
    let d = usize::from(w.difficulty());
    w.quests(player).is_some_and(|q| q.heard(d, class))
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
            // `0x005B6D30`.
            let c = super::npc_of(w, &args);
            let Some(&(class, special, _)) = INTROS.iter().find(|e| Some(e.0) == c) else {
                return true;
            };
            let Some(p) = args.player else { return true };
            if heard(w, p, class) {
                return true;
            }
            let k = u8::from(w.player_class(p) == special);
            add_state(ctl, w, i, list, args.target, k);
        }
        event::SCROLL_MESSAGE => {
            // `0x005B6C60`: the intro bit `0x00572360`.
            let Some(p) = args.player else { return true };
            let hit = INTROS
                .iter()
                .find(|e| u32::from(e.0) == args.a && e.2.contains(&args.b));
            if let Some(&(class, _, _)) = hit {
                let d = usize::from(w.difficulty());
                if let Some(q) = w.quests(p) {
                    q.hear(d, class);
                }
            }
        }
        _ => return false,
    }
    true
}

/// Active `0x005B6E30`: cain3 with its intro bit clear.
pub(super) fn active<W: QuestWorld>(w: &mut W, player: UnitId, npc_class: u16) -> bool {
    npc_class == npc::CAIN3 && !heard(w, player, npc::CAIN3)
}
