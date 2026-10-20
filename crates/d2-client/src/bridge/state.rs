//! The debug state export's way to the server: a link that can take a
//! read-only snapshot of its game (`d2_sim::debug::state`) implements
//! [`StateSource`]; the bridge forwards the request, so tools reach the
//! game through the bridge like everything else (step 5).

use d2_sim::debug::state::StateSnapshot;

/// A link whose server game can be snapshotted between frames.
pub trait StateSource {
    type Error;
    /// The game state after the last tick (frame = the game's frame).
    fn state_snapshot(&mut self) -> Result<StateSnapshot, Self::Error>;
    /// Each server player's path new direction (+0x65) after the last
    /// tick, as (GUID, direction) (`ServerPath::new_dir`).
    fn player_new_directions(&mut self) -> Result<Vec<(u32, u8)>, Self::Error>;
}

impl<L: StateSource> super::Bridge<L> {
    /// The server game's state snapshot, between two frames.
    pub fn state_snapshot(&mut self) -> Result<StateSnapshot, L::Error> {
        self.link.state_snapshot()
    }

    /// The server players' path new directions, between two frames.
    pub fn player_new_directions(&mut self) -> Result<Vec<(u32, u8)>, L::Error> {
        self.link.player_new_directions()
    }
}
