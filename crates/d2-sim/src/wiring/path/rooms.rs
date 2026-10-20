//! [`CollisionRooms`] on the DRLG: an active room (`RoomId`) is found in
//! the act DRLG that owns it; its sub-tile rect, adjacency array (DRLG
//! rooms mapped to their active rooms, in array order: the order the act
//! room list's `adjacent` copy holds too) and collision grid are the
//! DRLG's. The path code's collision queries read rooms through this
//! trait. No game is needed, so setters without a game argument
//! (missile path set-up) can stamp footprints.

use crate::drlg::{CollisionGrid, Drlg, DrlgRoomId, TileRect};
use crate::path::CollisionRooms;
use crate::units::RoomId;

use crate::wiring::action::DrlgWorld;

impl DrlgWorld {
    /// The act DRLG owning an active room and the room's DRLG room.
    fn owner_of(&self, room: RoomId) -> Option<(usize, &Drlg, DrlgRoomId)> {
        self.dungeon.acts.iter().enumerate().find_map(|(i, d)| {
            let d = d.as_ref()?;
            Some((i, d, d.drlg_room_of(room)?))
        })
    }
}

impl DrlgWorld {
    /// The seed of an active room.
    pub fn active_seed_mut(&mut self, room: RoomId) -> Option<&mut crate::rng::Seed> {
        let (act, _, r) = self.owner_of(room)?;
        self.dungeon.acts[act].as_mut()?.active_room_seed_mut(r)
    }
}

impl CollisionRooms for DrlgWorld {
    fn subtile_rect(&self, room: RoomId) -> Option<TileRect> {
        let (_, d, r) = self.owner_of(room)?;
        d.active_room(r).map(|a| a.subtiles)
    }

    fn adjacent_count(&self, room: RoomId) -> usize {
        self.owner_of(room)
            .and_then(|(_, d, r)| d.active_room(r))
            .map_or(0, |a| a.adjacency.len())
    }

    fn adjacent(&self, room: RoomId, i: usize) -> Option<RoomId> {
        let (_, d, r) = self.owner_of(room)?;
        let n = *d.active_room(r)?.adjacency.get(i)?;
        d.active_room(n).map(|a| a.id)
    }

    fn grid(&self, room: RoomId) -> Option<&CollisionGrid> {
        let (_, d, r) = self.owner_of(room)?;
        d.active_room(r).map(|a| &a.collision)
    }

    fn grid_mut(&mut self, room: RoomId) -> Option<&mut CollisionGrid> {
        let (act, _, r) = self.owner_of(room)?;
        self.dungeon.acts[act].as_mut()?.active_grid_mut(r)
    }
}
