//! What population needs from systems owned by other specs. Each trait
//! names its expected provider; tests use a fake. The decisions (draws,
//! tests, counts) stay in the population modules.
//!
//! [`MonsterInit`] is the narrow seam to monster creation itself
//! (implemented in a parallel session): allocation,
//! alignment, init, boss modifiers and monster data writes.

use super::{CoordRect, PopState, PresetUnit, RoomBox, TileRec};
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};

/// Rooms, coordinates, collision, seeds and unit positions. Provider: the
/// DRLG, units
/// and quests.
pub trait PopWorld {
    /// The game seed (game +0xD0).
    fn game_seed(&mut self) -> &mut Seed;
    /// The active room seed (room +0x6C).
    fn room_seed(&mut self, room: RoomId) -> &mut Seed;
    /// A unit's seed (unit +0x20).
    fn unit_seed(&mut self, unit: UnitId) -> &mut Seed;
    /// Level id of the room.
    fn room_level(&self, room: RoomId) -> i32;
    /// Level id of the populated room (0 = none).
    fn populated_level(&self, room: RoomId) -> i32;
    /// The level's populated-room count. Allocates
    /// the level when absent.
    fn populated_room_count(&mut self, act: u8, level: i32) -> i32;
    /// The room's coordinate records, in `next` order.
    fn coord_list(&self, room: RoomId) -> Vec<CoordRect>;
    /// The coordinate record at a point.
    fn coord_at(&self, room: RoomId, x: i32, y: i32) -> Option<CoordRect>;
    /// Coordinate index at a subtile point.
    fn coord_index_at(&self, room: RoomId, x: i32, y: i32) -> i32;
    /// The room's subtile box.
    fn room_box(&self, room: RoomId) -> RoomBox;
    /// The room's warp points (subtiles).
    fn warp_points(&self, room: RoomId) -> Vec<(i32, i32)>;
    /// The level's spawn location of
    /// `kind` (tiles; −1 when none), for the room's level. Runs the
    /// spawn-room choice with its effects.
    fn spawn_location(&mut self, room: RoomId, kind: u8) -> Option<(i32, i32)>;
    /// The room holding a point.
    fn room_at(&self, room: RoomId, x: i32, y: i32) -> Option<RoomId>;
    /// The collision test `(room, x, y, size, mask) ≠ 0`.
    fn collides(&self, room: RoomId, x: i32, y: i32, size: i32, mask: u16) -> bool;
    /// The mask test `(room, x, y, mask) ≠ 0`.
    fn mask_at(&self, room: RoomId, x: i32, y: i32, mask: u16) -> bool;
    /// The room's tile records, in order.
    fn tile_records(&self, room: RoomId) -> Vec<TileRec>;
    /// The room's preset units, in list order.
    fn preset_units(&self, room: RoomId) -> Vec<PresetUnit>;
    /// Client count (room +0x78).
    fn client_count(&self, room: RoomId) -> u32;
    /// The room a unit stands in.
    fn unit_room(&self, unit: UnitId) -> Option<RoomId>;
    /// The unit's subtile position (path x/y, or static path).
    fn unit_position(&self, unit: UnitId) -> (i32, i32);
    /// A quest flag (quests).
    fn quest_flag(&self, flag: u8) -> bool;
    /// The Act 4 Diablo quest state blocks population.
    fn chaos_blocks_population(&self) -> bool;
    /// The nearest free point (mask 0x3C01, size 1).
    fn nearest_free_point(&self, room: RoomId, x: i32, y: i32) -> Option<(RoomId, i32, i32)>;
}

/// Who names as the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerKey {
    /// The unit's GUID.
    Guid(UnitId),
    /// The owner data of a unit.
    DataOf(UnitId),
}

/// Arguments for a monster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alloc {
    pub class: i32,
    pub x: i32,
    pub y: i32,
    pub room: RoomId,
    pub mode: u8,
    /// `Some` with creation flag 0x20 (allocation flag 3); else flag 1.
    pub guid: Option<u32>,
}

/// Monster creation and monster data. Provider:
/// (monster data, modifiers, init), (allocation, flags),
/// (owner data), objects and quests.
pub trait MonsterInit {
    /// One game-seed step, a GUID, the unit.
    /// The allocator's monster type init
    /// reads and extends the game's regions, so they
    /// come with the call.
    fn allocate_monster(&mut self, a: Alloc, state: &mut PopState) -> Option<UnitId>;
    fn set_monster_flag(&mut self, unit: UnitId, flag: u32);
    /// The coordinate record: `rect`, or with `None` the one
    /// of the room at (x, y).
    fn set_coord_record(
        &mut self,
        unit: UnitId,
        rect: Option<CoordRect>,
        room: RoomId,
        x: i32,
        y: i32,
    );
    /// The creation alignment (0 evil, 1, 2).
    fn set_alignment(&mut self, unit: UnitId, align: u8);
    /// Unit flags (+0xC4) `|=`.
    fn set_unit_flags(&mut self, unit: UnitId, flags: u32);
    /// Per-class extras. `state` is lent back for their
    /// umods' boss count.
    fn class_extras(&mut self, unit: UnitId, state: &mut PopState);
    /// Monster init (boss mods); `state` as
    /// [`Self::class_extras`].
    fn init_monster(&mut self, unit: UnitId, state: &mut PopState);
    /// Monstats row of a unit.
    fn unit_class(&self, unit: UnitId) -> i32;
    /// The monster's level id.
    fn unit_level(&self, unit: UnitId) -> i32;
    /// Monster type flags (monster data +0x16).
    fn type_flags(&self, unit: UnitId) -> u16;
    /// Monster type flags `|=`.
    fn set_type_flags(&mut self, unit: UnitId, flags: u16);
    fn boss_modifiers(&mut self, boss: UnitId, champion_allowed: bool);
    /// The tail of the boss modifiers: init functions of modifiers 1–4 and the
    /// boss's own modifiers.
    fn boss_modifier_init(&mut self, boss: UnitId);
    /// Adds modifier `m` (as for modifier 16 or 22).
    /// Counts the unit in its region,
    /// so the regions come with the call.
    fn add_modifier(&mut self, unit: UnitId, m: u8, state: &mut PopState);
    /// Transfer the boss modifiers with `xfer`.
    fn transfer_modifiers(&mut self, boss: UnitId, minion: UnitId);
    fn set_owner_data(&mut self, unit: UnitId, owner: OwnerKey, a: i32, b: i32, c: i32);
    /// Add to the leader's minion list.
    fn add_minion(&mut self, leader: UnitId, minion: UnitId);
    /// Owner GUID and type.
    fn set_owner(&mut self, minion: UnitId, owner: UnitId);
    /// The quest hook of a new boss.
    fn boss_quest_hook(&mut self, boss: UnitId);
    /// and the rest of superunique init.
    fn superunique_init(&mut self, boss: UnitId, su: i32);
    /// An object.
    fn create_object(&mut self, room: RoomId, class: i32, x: i32, y: i32);
    /// Then when the preset has data.
    fn preset_created(&mut self, unit: UnitId, preset: &PresetUnit);
    /// then:
    /// the path (+0x10) of the room's preset unit `index` moves to the
    /// created monster's AI control +0x38 (the Npc map-AI nodes),
    /// and the preset's is cleared. Default:
    /// nothing.
    fn move_preset_path(&mut self, _unit: UnitId, _room: RoomId, _index: usize) {}
    /// Event 7 at frame + 250 + `roll(50)` on the
    /// monster's own seed (`unit-events.tsv`).
    fn schedule_monumod(&mut self, unit: UnitId);
    /// The alignment change `(game, unit, a, b)`.
    fn change_alignment(&mut self, unit: UnitId, a: i32, b: i32);
    /// Inactive unit restore.
    fn restore_inactive_units(&mut self, room: RoomId);
    /// Object population (objects).
    fn populate_objects(&mut self, room: RoomId);
}

/// Everything population calls.
pub trait PopHost: PopWorld + MonsterInit {}
impl<T: PopWorld + MonsterInit + ?Sized> PopHost for T {}
