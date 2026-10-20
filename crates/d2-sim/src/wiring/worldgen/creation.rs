//! Game creation's seeded controls in one sequence ([`WorldSim::create_game`]).
//!
//! Game creation derives, in this order, each from one game-seed step: the
//! monster regions, the object control, the NPC control and the quest
//! control. The order is fixed because each step advances the shared game
//! seed. The regions and the object control stay on the sim; the NPC and
//! quest controls are handed to the host (`d2-server` `WiredWorld` keeps them).

use std::sync::Arc;

use d2_data::tables::Monstats;

use crate::world::npc::{HireRow, NpcControl, NpcError};
use crate::world::objects::ObjectTables;
use crate::world::quests::{QuestControl, QuestError, QuestTables};

use super::{WorldPending, WorldSim};

/// The tables game creation reads beyond the sim's own.
pub struct CreationTables<'t> {
    pub objects: Arc<ObjectTables>,
    pub monstats: &'t [Monstats],
    pub hirelings: Vec<HireRow>,
    pub quests: &'t QuestTables,
}

/// The host-held controls of a new game.
pub struct CreatedControls {
    pub npc: NpcControl,
    pub quests: QuestControl,
}

/// A fatal path of game creation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CreationError {
    #[error(transparent)]
    Npc(#[from] NpcError),
    #[error(transparent)]
    Quest(#[from] QuestError),
}

impl<X: WorldPending> WorldSim<X> {
    /// Game creation's four game-seed derivations, in the order the game does them:
    /// [`WorldSim::create_regions`], [`WorldSim::create_objects`],
    /// `NpcControl::new` (expansion and difficulty from the creation
    /// fields' home, `UnitData`), `QuestControl::new`. Call after the
    /// creation fields are written (`d2-server` `ActionEvents::create_game`)
    /// and before any unit is allocated.
    pub fn create_game(&mut self, t: CreationTables<'_>) -> Result<CreatedControls, CreationError> {
        self.create_regions();
        self.create_objects(t.objects);
        let (expansion, difficulty) = (
            self.action.sys.data.expansion,
            self.action.sys.data.difficulty,
        );
        let seed = &mut self.action.sys.hooks.game_seed;
        let npc = NpcControl::new(t.monstats, t.hirelings, expansion, difficulty, seed)?;
        let quests = QuestControl::new(t.quests, seed)?;
        Ok(CreatedControls { npc, quests })
    }
}
