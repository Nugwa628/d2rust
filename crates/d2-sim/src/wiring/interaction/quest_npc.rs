//! Quests → the NPC control block: the mercenary reward
//! ([`NpcControl::quest_mercenary`]) that an Act I quest grants from
//! C→S 0x31 (Kashya's message 92).
//!
//! The quest code reaches the reward through its world
//! ([`crate::world::quests::QuestWorld::mercenary_reward`]) while it holds
//! the quest control block and the economy; the reward needs the NPC
//! control block and the desk as its world, which the quest call is
//! using. So [`Desk::quest_message`] queues the reward during the quest
//! call, with every send the call makes after it
//! ([`crate::wiring::economy::QuestDeferred`]), and runs the queue right
//! after the call, before the message's result is returned: the reward's
//! 0x50 and the hireling's creation messages, then the text refresh's
//! 0x27 / 0x29, which is the order the game sends them in.
//!
//! Order of the other effects: of the records the list dispatch visits
//! after chain 2's message-92 handler (`act1::scroll`), chains 3–6 have no
//! message-92 case, but chain 37 (Act I intro) has an event-11 function
//! without a written body (it reaches
//! [`crate::wiring::economy::QuestRest::unhandled`]). In the game that
//! function runs after the reward; here before it.
//!
//! TODO(quests.tsv chain 37): the deferral matches the game's order only
//! if that event-11 function does nothing observable for message 92 (an
//! open question).
//!
//! A reward reached through another entry (no queue) still goes to
//! [`crate::wiring::economy::QuestRest::mercenary_reward`].

use crate::units::lifecycle::LifecycleHooks;
use crate::units::UnitId;
use crate::wiring::economy::{EconomyQuests, QuestRest};
use crate::world::npc::NpcControl;

use super::{Desk, InteractionError, NpcRest, PlayerQuestsRef};

impl<H: LifecycleHooks, R: NpcRest + QuestRest + PlayerQuestsRef> Desk<'_, '_, H, R> {
    /// C→S 0x31 with the mercenary rewards it grants run on `npc`.
    /// Returns the quest handler's result code.
    pub fn quest_message(&mut self, npc: &mut NpcControl, player: UnitId, msg: &[u8]) -> u32 {
        let mut deferred = Vec::new();
        let code = {
            let mut w = EconomyQuests::new(self.econ, self.rest);
            w.deferred = Some(&mut deferred);
            self.quests.quest_message(&mut w, player, msg)
        };
        for d in deferred {
            let Some((p, class)) = d.run(&mut *self.rest) else {
                continue;
            };
            if let Err(e) = npc.quest_mercenary(self, p, class) {
                self.state.errors.push(InteractionError::Npc(e));
            }
        }
        code
    }
}
