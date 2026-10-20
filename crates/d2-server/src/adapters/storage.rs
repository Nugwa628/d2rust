//! Character storage: the server's character save, the `.d2s` file of a
//! player.
//!
//! The server runs the save at two points: the leave handler of C→S 0x69,
//! before S→C 0x05 ([`super::session_flow::SessionFlow::leave`]), and the
//! tick's client pass every 8192 frames (the head of step 5, run by
//! [`SimGame`]'s `Tick` between the host's step halves). Both save every
//! client with a player, in client-list order ([`SimGame::save_characters`]).
//!
//! What a save holds and where the file goes is the host's
//! [`CharacterStore`]: the game's rest of the player (quests, hireling,
//! the app's tables) is typed by the host that built the game, so the
//! host installs the writer ([`SimGame::set_storage`]); the server
//! decides when it runs.

use crate::seams::ClientId;

use super::SimGame;

/// Writes one client's character.
pub trait CharacterStore<D, W> {
    /// Saves the character of `client`'s player from the running game.
    fn save(&mut self, sim: &mut SimGame<D, W>, client: ClientId) -> Result<(), String>;
}

/// Why a client's character was not written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SaveFault {
    /// The game has no storage ([`SimGame::set_storage`] never ran: a
    /// test host, or a character without a save file).
    #[error("no character storage")]
    NoStorage,
    /// The storage refused (for example the save file could not be
    /// opened); the text says why.
    #[error("{0}")]
    Failed(String),
}
