//! The game fields item creation, treasure and the cube read
//! ([`crate::items::ItemGame`], [`crate::treasure::GameFacts`]).
//! [`crate::game::Game`] does not hold them yet, so the wiring does.
//!
//! A host that runs the economy on the action wiring keeps the creation
//! fields in one place, the action wiring's (`ActionHooks::game_seed`,
//! `ActionHooks::ai_info`, `UnitData::expansion`), and builds a
//! [`GameFields`] from them for each call ([`GameFields::from_action`]);
//! [`GameFields::ai_info`] is the inverse for game creation.

use crate::items::{ItemGame, UniqueBits};
use crate::monsters::ai::GameInfo;
use crate::rng::Seed;
use crate::tick::TickRate;
use crate::treasure::GameFacts;

/// Game-creation fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameFields {
    /// The game seed.
    pub seed: Seed,
    /// Difficulty: 0 normal, 1 nightmare, 2 hell.
    pub difficulty: u8,
    /// Whether the game is an expansion game.
    pub expansion: bool,
    /// The game type (3 = single player); item quality reads any non-zero
    /// value as a ladder flag.
    pub game_type: u8,
    /// The extended game type, read as a ladder flag when non-zero.
    pub ladder: bool,
    /// Which unique items have been found in this game.
    pub uniques: UniqueBits,
    /// The game's tick rate (item timers are stated in base ticks). Set
    /// from the game before each item creation.
    pub tick_rate: TickRate,
}

impl GameFields {
    /// Fields with the given game seed, normal difficulty, no ladder.
    pub fn new(seed: Seed, expansion: bool) -> Self {
        Self {
            seed,
            difficulty: 0,
            expansion,
            game_type: 0,
            ladder: false,
            uniques: UniqueBits::default(),
            tick_rate: TickRate::BASE,
        }
    }

    /// The fields of a game whose game seed is `seed`, whose AI fields are
    /// `ai` (`game_type`, `game_type_ex`, `difficulty`) and whose expansion
    /// flag is `expansion`, with the unique bits `uniques`. `game_type_ex` is
    /// read as a flag (non-zero).
    pub fn from_action(seed: Seed, ai: &GameInfo, expansion: bool, uniques: UniqueBits) -> Self {
        Self {
            seed,
            difficulty: ai.difficulty,
            expansion,
            game_type: ai.game_type,
            ladder: ai.game_type_ex != 0,
            uniques,
            tick_rate: TickRate::BASE,
        }
    }

    /// The AI's copy of these fields (game type, extended type, difficulty).
    pub fn ai_info(&self) -> GameInfo {
        GameInfo {
            game_type: self.game_type,
            game_type_ex: u32::from(self.ladder),
            difficulty: self.difficulty,
        }
    }

    /// The treasure walk's game facts. The
    /// living-player count and the `players` setting come from the host.
    pub fn treasure_facts(&self, living_players: i32, players_setting: i32) -> GameFacts {
        GameFacts {
            expansion: self.expansion,
            difficulty: self.difficulty,
            game_type: self.game_type,
            living_players,
            players_setting,
            item_format: u32::from(ItemGame::item_format(self)),
        }
    }
}

impl ItemGame for GameFields {
    fn seed(&mut self) -> &mut Seed {
        &mut self.seed
    }
    fn difficulty(&self) -> u8 {
        self.difficulty
    }
    fn expansion(&self) -> bool {
        self.expansion
    }
    /// The ladder flags: game type non-zero, extended type non-zero.
    fn ladder_flags(&self) -> (bool, bool) {
        (self.game_type != 0, self.ladder)
    }
    fn uniques(&mut self) -> &mut UniqueBits {
        &mut self.uniques
    }
    fn tick_rate(&self) -> TickRate {
        self.tick_rate
    }
}
