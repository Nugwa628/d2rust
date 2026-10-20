// Spec: specs/world/quests-act1.md §10.3 (Act I intro, chain 37), §6.7
// Spec: specs/world/quests.md (the sections other than §10)
//! The Act I intro record: per-NPC first-talk text kept in the player's
//! NPC intro record (events 0 and 11, the active function).

use super::add_state;
use crate::units::UnitId;
use crate::world::quests::{bit, event, npc, EventArgs, QuestControl, QuestWorld, TextList};

/// (NPC class, the player class with special text, the event-11 messages
/// that set the intro bit).
const INTROS: [(u16, u8, [u32; 2]); 4] = [
    (147, 2, [45, 46]),
    (npc::AKARA, 1, [11, 12]),
    (npc::KASHYA, 0, [24, 25]),
    (npc::CHARSI, 4, [36, 37]),
];

fn heard<W: QuestWorld>(w: &mut W, player: UnitId, class: u16) -> bool {
    let d = usize::from(w.difficulty());
    w.quests(player).is_some_and(|q| q.heard(d, class))
}

/// Dispatches chain 37's callbacks; false = no body (unhandled).
pub(super) fn callback<W: QuestWorld>(
    ctl: &mut QuestControl,
    w: &mut W,
    i: usize,
    args: EventArgs,
    list: Option<&mut TextList>,
) -> bool {
    match args.event {
        event::NPC_ACTIVATE => {
            // `0x0058F8F0`.
            let c = args.target.and_then(|n| w.monster_class(n));
            let Some(&(class, special, _)) = INTROS.iter().find(|e| Some(e.0) == c) else {
                return true;
            };
            if args.player.is_some_and(|p| heard(w, p, class)) {
                return true;
            }
            let pc = args.player.map(|p| w.player_class(p));
            let k = u8::from(pc == Some(special));
            add_state(ctl, w, i, list, args.target, k);
        }
        event::SCROLL_MESSAGE => {
            // `0x0058F870`.
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

/// Active `0x0058F9C0` (§6.4).
pub(super) fn active<W: QuestWorld>(w: &mut W, player: UnitId, npc_class: u16) -> bool {
    let lacks_den = !super::player_flags(w, player).get(1, bit::REWARD_GRANTED);
    lacks_den && npc_class == npc::AKARA && !heard(w, player, npc::AKARA)
}
