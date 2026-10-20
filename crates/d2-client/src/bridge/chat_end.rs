//! The play preview's chat close. The base game closes the NPC chat
//! (C→S 0x30) when the dialog ends: the speech played out, the menu or
//! box closed. The preview has neither the speech (sound is deferred)
//! nor the NPC menu (left item 1), so the dialog branch
//! is followed at once by the close, in the
//! same frame, after the 0x2F and the quest message 0x31. The server
//! then runs the chat end (`QuestControl::npc_deactivate`) and a second
//! talk starts a fresh interaction.
//!
//! D2RUST's own, unverified (the base game's close comes 15 frames
//! after the message).

use super::link::ServerLink;
use super::{Bridge, BridgeError};
use crate::ui::panels::npc::msg_chat_end;

impl<L: ServerLink> Bridge<L> {
    /// C→S 0x30 for the NPC `guid` (flag 1, as recorded), sent at once.
    pub fn preview_chat_end(&mut self, guid: u32) -> Result<usize, BridgeError> {
        self.world.outgoing.push(msg_chat_end(guid).to_vec());
        self.send_outgoing()
    }
}
