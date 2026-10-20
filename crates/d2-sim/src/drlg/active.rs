//! Active rooms: creation, adjacency arrays, room clients and the inactivity counter,
//! deactivation, and the server-side room change of a client.
//!
//! A DRLG room becomes an active room when it gets a tile grid; it then joins the act's
//! room list, which is `crate::units::UnitLists`, reached through the `ActRooms` trait.
//! This module implements that trait for `UnitLists`.

use crate::rng::Seed;
use crate::units::{ClientId, RoomId, UnitLists};

use super::collision::CollisionGrid;
use super::level::Drlg;
use super::seams::{ActRooms, Services};
use super::{room_flags, DrlgError, DrlgRoomId, TileRect, SUBTILES};

/// Removal threshold: a room is removed when its inactivity counter is greater than 10.
pub const REMOVAL_THRESHOLD: u32 = 10;

/// Active-room flags.
pub mod active_flags {
    /// The room's monsters and objects have been populated.
    pub const POPULATED: u32 = 1;
    /// Units in the room are active.
    pub const UNITS_ACTIVE: u32 = 2;
    /// The room is not updated.
    pub const NO_UPDATE: u32 = 4;
}

/// An active room, DRLG side: seed, flags, sub-tile rect, adjacency, clients and
/// collision. The act list record is `ActiveRoom::id` in the unit lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveRoom {
    /// The act room list record.
    pub id: RoomId,
    /// Active-room seed.
    pub seed: Seed,
    /// Flags at creation (`active_flags`).
    pub flags: u32,
    /// Sub-tile rect.
    pub subtiles: TileRect,
    /// Adjacency array: the neighbouring DRLG rooms that have active rooms.
    pub adjacency: Vec<DrlgRoomId>,
    /// Clients present in the room, kept sorted.
    pub clients: Vec<ClientId>,
    /// Inactivity counter.
    pub inactivity: u32,
    /// Collision grid.
    pub collision: CollisionGrid,
}

impl ActRooms for UnitLists {
    fn create_active_room(&mut self, act: u8, flags: u32) -> RoomId {
        self.ensure_act(act).expect("act index 0..4");
        let id = self.create_room(act).expect("act exists");
        let r = self.room_mut(id).expect("just created");
        r.populated = flags & active_flags::POPULATED != 0;
        r.units_active = flags & active_flags::UNITS_ACTIVE != 0;
        r.no_update = flags & active_flags::NO_UPDATE != 0;
        self.activate_room(id).expect("room exists");
        id
    }

    fn set_adjacent(&mut self, room: RoomId, adjacent: &[RoomId]) {
        if let Some(r) = self.room_mut(room) {
            r.adjacent = adjacent.to_vec();
        }
    }

    fn remove_active_room(&mut self, room: RoomId) -> u32 {
        let Some(r) = self.room(room) else { return 0 };
        let flags =
            u32::from(r.populated) | u32::from(r.units_active) << 1 | u32::from(r.no_update) << 2;
        // Units still in the room (tick step 9 compresses them first) are
        // unlinked from it by `free_room`. Per-unit flag and path updates on removal
        // are not done here; they belong to the unit code.
        let _ = self.free_room(room);
        flags
    }
}

impl Drlg {
    /// The DRLG room of an active room record.
    pub fn drlg_room_of(&self, room: RoomId) -> Option<DrlgRoomId> {
        self.active_index.get(&room).copied()
    }

    /// The active room of a DRLG room.
    pub fn active_room(&self, id: DrlgRoomId) -> Option<&ActiveRoom> {
        self.room(id).active.as_ref()
    }

    fn active_mut(&mut self, id: DrlgRoomId) -> Option<&mut ActiveRoom> {
        self.room_mut(id).active.as_mut()
    }

    /// The active-room seed of a DRLG room, mutable: monster population draws on it.
    pub fn active_room_seed_mut(&mut self, id: DrlgRoomId) -> Option<&mut crate::rng::Seed> {
        self.active_mut(id).map(|a| &mut a.seed)
    }

    /// The collision grid of a DRLG room's active room, mutable: unit footprints write
    /// into it.
    pub fn active_grid_mut(&mut self, id: DrlgRoomId) -> Option<&mut CollisionGrid> {
        self.active_mut(id).map(|a| &mut a.collision)
    }

    /// Creates the active room for a DRLG room. Does nothing if the tile grid has no
    /// floor and no wall records.
    pub(super) fn create_active_room(
        &mut self,
        svc: &mut Services<'_>,
        id: DrlgRoomId,
    ) -> Result<(), DrlgError> {
        let has_tiles = self
            .room(id)
            .tiles
            .as_ref()
            .is_some_and(|t| !t.floors.is_empty() || !t.walls.is_empty());
        if !has_tiles {
            return Ok(());
        }
        let r = self.room(id);
        let flags = if r.flags & room_flags::AUTOMAP_REVEAL != 0 {
            active_flags::NO_UPDATE
        } else if r.other_flags & 1 != 0 {
            active_flags::POPULATED
        } else {
            0
        };
        let rect = r.rect;
        let seed = self.room_mut(id).seed.derive();
        let list_id = svc.rooms.create_active_room(self.act, flags);
        let sub = TileRect::new(
            rect.x * SUBTILES,
            rect.y * SUBTILES,
            rect.w * SUBTILES,
            rect.h * SUBTILES,
        );
        self.room_mut(id).active = Some(ActiveRoom {
            id: list_id,
            seed,
            flags,
            subtiles: sub,
            adjacency: Vec::new(),
            clients: Vec::new(),
            inactivity: 0,
            collision: CollisionGrid::new(sub),
        });
        self.active_index.insert(list_id, id);
        // Fill the new room's adjacency array, then refill every neighbour's.
        self.fill_adjacency(svc, id);
        let adj = self
            .room(id)
            .active
            .as_ref()
            .expect("set")
            .adjacency
            .clone();
        for n in adj {
            if n != id {
                self.fill_adjacency(svc, n);
            }
        }
        // Build the collision grid last.
        self.build_collision(id);
        Ok(())
    }

    /// Fills a room's adjacency array: its rooms-near order restricted to rooms that
    /// have active rooms.
    fn fill_adjacency(&mut self, svc: &mut Services<'_>, id: DrlgRoomId) {
        let near = self.room(id).near.clone().unwrap_or_default();
        let adj: Vec<DrlgRoomId> = near
            .into_iter()
            .filter(|&n| self.room(n).active.is_some())
            .collect();
        self.set_adjacency(svc, id, adj);
    }

    fn set_adjacency(&mut self, svc: &mut Services<'_>, id: DrlgRoomId, adj: Vec<DrlgRoomId>) {
        let ids: Vec<RoomId> = adj
            .iter()
            .map(|&n| self.room(n).active.as_ref().expect("active").id)
            .collect();
        let a = self.active_mut(id).expect("active room");
        a.adjacency = adj;
        let list_id = a.id;
        svc.rooms.set_adjacent(list_id, &ids);
    }

    /// The adjacency array of an active room as act-list ids.
    pub fn adjacent_rooms(&self, id: DrlgRoomId) -> Vec<RoomId> {
        self.active_room(id).map_or_else(Vec::new, |a| {
            a.adjacency
                .iter()
                .map(|&n| self.room(n).active.as_ref().expect("active").id)
                .collect()
        })
    }

    // ---- clients and inactivity ------------------------------------------

    /// Adds a client to a room: append, then sort ascending.
    ///
    /// The sort key is the client slot.
    pub fn add_room_client(&mut self, id: DrlgRoomId, client: ClientId) {
        if let Some(a) = self.active_mut(id) {
            a.clients.push(client);
            a.clients.sort();
        }
    }

    /// Removes a client from a room: the last entry fills the hole, then the array is
    /// sorted again.
    pub fn remove_room_client(&mut self, id: DrlgRoomId, client: ClientId) {
        if let Some(a) = self.active_mut(id) {
            if let Some(i) = a.clients.iter().position(|&c| c == client) {
                a.clients.swap_remove(i);
                a.clients.sort();
            }
        }
    }

    /// Updates the inactivity counter: 0 when the room has clients, else one more.
    /// Returns the new value. Tick step 9's `room_inactivity` hook.
    pub fn room_inactivity(&mut self, room: RoomId) -> Result<u32, DrlgError> {
        let id = self.drlg_room_of(room).ok_or(DrlgError::NotActive)?;
        let a = self.active_mut(id).ok_or(DrlgError::NotActive)?;
        a.inactivity = if a.clients.is_empty() {
            a.inactivity.wrapping_add(1)
        } else {
            0
        };
        Ok(a.inactivity)
    }

    /// Whether a room may be removed: not a portal room, past its initial status, and
    /// not in a level that keeps all its rooms until every one is past that status.
    /// Tick step 9's `act_allows_room_removal` hook.
    pub fn allows_removal(&self, room: RoomId) -> Result<bool, DrlgError> {
        if self.on_client {
            return Err(DrlgError::ClientCopyRemoval);
        }
        let id = self.drlg_room_of(room).ok_or(DrlgError::NotActive)?;
        let r = self.room(id);
        if r.flags & room_flags::PORTAL != 0 {
            return Ok(false);
        }
        // A further per-room check is not modelled; rooms are treated as passing it.
        if r.status <= 1 {
            return Ok(false);
        }
        let l = r.level;
        if Self::no_removal_level(self.level(l).id)
            && self
                .level_rooms(l)
                .into_iter()
                .any(|x| self.room(x).status <= 1)
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// Removes an active room after tick step 9 compressed its units: drops it from its
    /// neighbours' adjacency arrays, unlinks and frees the act list record, keeps the
    /// populated flag in `other_flags`, and frees the tiles.
    pub fn remove_active_room(
        &mut self,
        svc: &mut Services<'_>,
        room: RoomId,
    ) -> Result<(), DrlgError> {
        let id = self.drlg_room_of(room).ok_or(DrlgError::NotActive)?;
        let adj = self
            .room(id)
            .active
            .as_ref()
            .expect("indexed")
            .adjacency
            .clone();
        for n in adj {
            if n == id {
                continue;
            }
            let Some(mut arr) = self.active_room(n).map(|a| a.adjacency.clone()) else {
                continue;
            };
            if let Some(p) = arr.iter().position(|&x| x == id) {
                arr.swap_remove(p);
                self.set_adjacency(svc, n, arr);
            }
        }
        let flags = svc.rooms.remove_active_room(room);
        self.active_index.remove(&room);
        let r = self.room_mut(id);
        r.active = None;
        r.other_flags = flags & active_flags::POPULATED;
        if r.flags & room_flags::HAS_ROOM != 0 {
            self.free_room_tiles(svc, id);
        }
        Ok(())
    }

    // ---- client room change ---------------------------------------------

    /// The server room change of a client (leaving a room is the same call with `new`
    /// = none): updates DRLG statuses and level activity, then the client joins every
    /// room of the new adjacency array missing from the old and leaves every room of
    /// the old array missing from the new. Returns the rooms the client joined, in the
    /// new array's order.
    pub fn client_changes_room(
        &mut self,
        svc: &mut Services<'_>,
        client: ClientId,
        old: Option<DrlgRoomId>,
        new: Option<DrlgRoomId>,
    ) -> Result<Vec<DrlgRoomId>, DrlgError> {
        self.client_switches_room(svc, client, old, new)
            .map(|(joined, _)| joined)
    }

    /// `Drlg::client_changes_room` returning both sides: the rooms the client joined
    /// (new array order) and the rooms it left (old array order).
    pub fn client_switches_room(
        &mut self,
        svc: &mut Services<'_>,
        client: ClientId,
        old: Option<DrlgRoomId>,
        new: Option<DrlgRoomId>,
    ) -> Result<(Vec<DrlgRoomId>, Vec<DrlgRoomId>), DrlgError> {
        self.change_status_room(svc, old, new)?;
        self.update_level_activity(svc.data, svc.types, old, new)?;
        // The rooms seen from each side: the adjacency array, plus the
        // active view extras in a game wider than 4:3.
        let adj = |d: &Self, r: Option<DrlgRoomId>| r.map(|r| d.view_rooms(r)).unwrap_or_default();
        let (old_adj, new_adj) = (adj(self, old), adj(self, new));
        let mut joined = Vec::new();
        for &r in &new_adj {
            if !old_adj.contains(&r) {
                self.add_room_client(r, client);
                joined.push(r);
            }
        }
        let mut left = Vec::new();
        for &r in &old_adj {
            if !new_adj.contains(&r) {
                self.remove_room_client(r, client);
                left.push(r);
            }
        }
        Ok((joined, left))
    }

    /// Whether the active room `room` is ready: its adjacency count equals its DRLG
    /// room's rooms-near count and every room of its adjacency array is populated
    /// (the `populated` flag). `None` when it is not an active room of this DRLG.
    pub fn room_ready(&self, room: RoomId, populated: impl Fn(RoomId) -> bool) -> Option<bool> {
        let id = self.drlg_room_of(room)?;
        let r = self.room(id);
        let a = r.active.as_ref()?;
        let near = r.near.as_ref().map_or(0, Vec::len);
        Some(a.adjacency.len() == near && self.adjacent_rooms(id).into_iter().all(populated))
    }

    /// The active rooms of this DRLG in act-list id order (lookup only).
    pub fn active_rooms(&self) -> Vec<(RoomId, DrlgRoomId)> {
        self.active_index.iter().map(|(&a, &b)| (a, b)).collect()
    }
}
