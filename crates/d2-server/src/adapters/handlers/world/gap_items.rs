//! Who owns what at save time: the units whose item lists a save writes
//! besides the player's (the hireling `jf` and the Iron Golem `kf` lists)
//! and the item index of a mouse skill's owner item. Reads only; the lists
//! themselves go through [`WiredWorld::save_items`] /
//! [`WiredWorld::load_items`]. Part of the wired world host; the character
//! save adapter calls these when it assembles a save file.

use d2_sim::game::Game;
use d2_sim::units::{UnitId, UnitType};

use super::{ActionEvents, WiredWorld};

/// Class id of the Iron Golem monster (the golem pet node is identified by it).
pub const IRON_GOLEM_CLASS: u32 = 0x123;
/// The golem's pet type: pet node type 3.
const GOLEM_PET_TYPE: usize = 3;

/// The live hireling's header block of a save, field by field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HirelingBlock {
    /// The hireling is dead.
    pub dead: bool,
    /// The node's seed.
    pub seed: u32,
    /// The name index relative to the row's `NameFirst`.
    pub name_index: u16,
    /// The hireling row id.
    pub id: u16,
    /// Experience (stat 13 of the unit, 0 without a unit).
    pub experience: u32,
}

impl<R, S> WiredWorld<R, S> {
    /// The block of the player's hireling node (type 7) whose
    /// row is found: seed and name of the node, the name index relative
    /// to the row's `NameFirst`, experience from the unit's stat 13 (0
    /// without a unit). `None`: no node or no row. D2RUST's own, unverified
    /// Field by field as [`HirelingBlock`].
    pub fn hireling_block<D: ActionEvents>(
        &self,
        game: &mut Game,
        events: &mut D,
        player: UnitId,
    ) -> Option<HirelingBlock> {
        let node = *self.state.hirelings.first_node(player, true)?;
        let tables = self.state.hireling_tables.as_ref()?;
        let expansion = events.action().sys.data.expansion;
        let version = if expansion { 100 } else { 0 };
        let row = tables
            .rows
            .rows
            .iter()
            .find(|r| r.id == node.id && r.version == version)?;
        let merc = self.hireling_unit(game, player);
        let experience = merc.map_or(0, |m| {
            events
                .action()
                .with(game, |_, v| v.stats.unit_base(m, 13, 0))
        });
        Some(HirelingBlock {
            dead: node.dead,
            seed: node.seed,
            name_index: node.name.wrapping_sub(row.name_first),
            id: node.id as u16,
            experience: experience.max(0) as u32,
        })
    }

    /// The player's hireling unit: the first node of its hireling list
    /// (dead nodes included) whose unit exists.
    pub fn hireling_unit(&self, game: &Game, player: UnitId) -> Option<UnitId> {
        self.state
            .hirelings
            .list(player)?
            .nodes
            .iter()
            .find_map(|n| game.lists.find_unit(UnitType::Monster, n.guid))
    }

    /// The player's Iron Golem unit: the golem pet node whose unit class
    /// is 0x123.
    pub fn golem_unit<D: ActionEvents>(
        &self,
        game: &Game,
        events: &mut D,
        player: UnitId,
    ) -> Option<UnitId> {
        let guids: Vec<i32> = events
            .action()
            .hooks()
            .pet_lists
            .get(&player)?
            .entries
            .get(GOLEM_PET_TYPE)?
            .nodes
            .iter()
            .map(|n| n.guid)
            .collect();
        guids.into_iter().find_map(|g| {
            let u = game
                .lists
                .find_unit(UnitType::Monster, u32::try_from(g).ok()?)?;
            let class = events.action().sys.units.get(u)?.class;
            (class == IRON_GOLEM_CLASS).then_some(u)
        })
    }

    /// The GUIDs of `owner`'s inventory item list in link order
    /// (a save's item index is the position in this list + 1).
    pub fn item_guids(&self, owner: UnitId) -> Vec<u32> {
        let Some(inv) = self.inventory.as_ref() else {
            return Vec::new();
        };
        inv.state
            .items_of(owner)
            .into_iter()
            .filter_map(|u| inv.state.items.get(&u).map(|d| d.guid))
            .collect()
    }
}
