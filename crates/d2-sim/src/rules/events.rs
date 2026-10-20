//! A mod game's event hooks (`docs/plans/mods/seams.md` MS11).
//!
//! A mod game reacts to what happens in a game through [`EventHooks`],
//! handed out by [`super::ModRules::events`]: the game created and freed,
//! a player joined, a unit killed, a periodic tick, a level entered and a
//! chat line that may be the mod's command. Each hook runs at a fixed
//! point, after D2RUST's own work for that point (the chat command, which
//! may take the line, is asked before the line goes out). Under
//! [`super::Ruleset::Original`] nothing is asked.
//!
//! A hook acts on the game through an [`EventHost`]. Per-game and
//! per-player state a mod keeps lives in its own `ModRules` object (behind
//! its own lock) and in its own saves, never in D2RUST's. Hooks stay
//! deterministic: no I/O, no clock, randomness only from a unit's seed.

use std::fmt::Debug;

use crate::game::Game;
use crate::rng::Seed;
use crate::units::{UnitId, UnitType};

/// The game as an event hook sees it: the game's lists (frame, units,
/// rooms, clients, timers), the units' stats and seeds, and the clients.
pub trait EventHost {
    /// The game: its frame, unit, room and client lists and its timers.
    fn game(&mut self) -> &mut Game;
    /// The unit's type; `None` for a unit not in the game.
    fn unit_type(&self, unit: UnitId) -> Option<UnitType>;
    /// The unit's class (monstats or player class row); 0 when absent.
    fn class_id(&self, unit: UnitId) -> i32;
    /// The unit's total value of `stat` at `layer`.
    fn stat(&self, unit: UnitId, stat: u16, layer: u16) -> i32;
    /// Sets the unit's base value of `stat` (layer 0); its client is told
    /// as for any stat change.
    fn set_stat(&mut self, unit: UnitId, stat: u16, value: i32);
    /// The unit has `state`.
    fn has_state(&self, unit: UnitId, state: u16) -> bool;
    /// The unit's seed, the only RNG a hook may draw from; `None` for a
    /// unit not in the game.
    fn seed(&mut self, unit: UnitId) -> Option<&mut Seed>;
    /// The id of the level the unit stands in; `None` outside a room.
    fn level(&self, unit: UnitId) -> Option<u32>;
    /// Queues the server message `bytes` to the client of `player` (a
    /// player without a client receives nothing).
    fn send(&mut self, player: UnitId, bytes: &[u8]);
}

/// A mod game's event hooks (MS11). Every hook does nothing by default.
pub trait EventHooks: Send + Sync + Debug {
    /// The game was created, before its first client loads.
    fn game_created(&self, h: &mut dyn EventHost) {
        let _ = h;
    }
    /// The game's last client left: the game ends and is freed.
    fn game_freed(&self, h: &mut dyn EventHost) {
        let _ = h;
    }
    /// `player` joined the game, after its join messages.
    fn player_joined(&self, h: &mut dyn EventHost, player: UnitId) {
        let _ = (h, player);
    }
    /// `victim` (a player or a monster) was killed, by `killer` when there
    /// is one, after the kill's experience, drops and death mode request.
    fn unit_killed(&self, h: &mut dyn EventHost, killer: Option<UnitId>, victim: UnitId) {
        let _ = (h, killer, victim);
    }
    /// How often [`EventHooks::periodic`] runs, in base ticks (25 a
    /// second, whatever the game's tick rate); 0: never.
    fn period(&self) -> u32 {
        0
    }
    /// Runs every [`EventHooks::period`] base ticks, at the end of the
    /// tick.
    fn periodic(&self, h: &mut dyn EventHost) {
        let _ = h;
    }
    /// `player` entered level `to`, from level `from` (`None`: the
    /// player's first level in the game), after D2RUST's level change.
    fn level_entered(&self, h: &mut dyn EventHost, player: UnitId, from: Option<u32>, to: u32) {
        let _ = (h, player, from, to);
    }
    /// `player` said `text` (an open chat line, not a whisper). `true`
    /// takes the line as the mod's command: it goes to no one. Asked after
    /// the text filter, before the line goes out.
    fn chat_command(&self, h: &mut dyn EventHost, player: UnitId, text: &[u8]) -> bool {
        let _ = (h, player, text);
        false
    }
}
