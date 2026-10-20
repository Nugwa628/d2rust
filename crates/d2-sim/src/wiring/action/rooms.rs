//! DRLG room activation and the act room lists: the room lookups the other
//! adapters use (DRLG room of an active room, its level, the town test, the
//! collision grids, the room search), and the DRLG parts of the tick's room
//! removal and inactive-room steps and of the client room change.
//!
//! The act room list is [`crate::units::UnitLists`]: active rooms are
//! created in it by the DRLG through its [`crate::drlg::ActRooms`]
//! implementation (`drlg/active.rs`).

use crate::drlg::{is_town, Drlg, DrlgRoomId, Services, TileRect};
use crate::game::Game;
use crate::units::{ClientId, RoomId, UnitLists};

use super::{DrlgWorld, WiringError};

/// One room of a client room switch ([`DrlgWorld::client_switches_room`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwitchedRoom {
    /// The active room (act room list).
    pub room: RoomId,
    /// Its DRLG room's tile origin and level id (the 0x07 / 0x08 fields).
    pub tile_x: i32,
    pub tile_y: i32,
    pub level: u32,
    /// Clients in the room after the change.
    pub clients: usize,
}

/// What a client room switch joined and left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomSwitch {
    /// New adjacency array order.
    pub joined: Vec<SwitchedRoom>,
    /// Old adjacency array order.
    pub left: Vec<SwitchedRoom>,
    /// The new room's active view extras (a game wider than 4:3; empty
    /// otherwise): the client's per-tick unit updates cover them too.
    pub view: Vec<RoomId>,
}

impl DrlgWorld {
    /// The act DRLG and its services over `lists` (the act room list).
    /// `None` when the act has no DRLG yet.
    pub fn with_act<R>(
        &mut self,
        act: u8,
        lists: &mut UnitLists,
        f: impl FnOnce(&mut Drlg, &mut Services<'_>) -> R,
    ) -> Option<R> {
        let drlg = self.dungeon.acts.get_mut(usize::from(act))?.as_mut()?;
        let mut svc = Services {
            data: &self.data,
            tiles: &*self.tiles,
            types: &mut *self.types,
            rooms: lists,
        };
        Some(f(drlg, &mut svc))
    }

    /// The act's populated-room count of `level`, without allocating the
    /// level (an absent level has no rooms).
    pub fn populated_rooms(&self, act: u8, level: u32) -> u32 {
        self.dungeon
            .acts
            .get(usize::from(act))
            .and_then(|d| d.as_ref())
            .map_or(0, |d| d.populated_room_count_if_present(level))
    }

    /// The Act II DRLG's staff-tomb level, the true tomb; `None` while the
    /// act has no DRLG.
    pub fn staff_tomb(&self, act: u8) -> Option<u32> {
        let d = self.dungeon.acts.get(usize::from(act))?.as_ref()?;
        Some(d.staff_tomb)
    }

    /// The DRLG of an active room's act and the room's DRLG room.
    pub fn drlg_room(&self, game: &Game, room: RoomId) -> Option<(&Drlg, DrlgRoomId)> {
        let act = game.lists.room(room)?.act;
        let drlg = self.dungeon.acts.get(usize::from(act))?.as_ref()?;
        Some((drlg, drlg.drlg_room_of(room)?))
    }

    /// The client count of an active room (its client array); 0 for a room without an active DRLG room.
    pub fn client_count(&self, game: &Game, room: RoomId) -> usize {
        self.drlg_room(game, room)
            .and_then(|(d, r)| d.active_room(r))
            .map_or(0, |a| a.clients.len())
    }

    /// The level id of an active room (`levels.txt` row).
    pub fn level_id(&self, game: &Game, room: RoomId) -> Option<u32> {
        let (d, r) = self.drlg_room(game, room)?;
        Some(d.level(d.room(r).level).id)
    }

    /// The room's "LOS draw" test: an outdoor-grid room (type 1) answers true; a preset room (type 2)
    /// answers its room flag 0x80000 (set from an `Outdoors` lvlprest
    /// row); every other type, and a room without a DRLG room, false.
    pub fn los_draw(&self, game: &Game, room: RoomId) -> bool {
        use crate::drlg::room::RoomKind;
        let Some((d, r)) = self.drlg_room(game, room) else {
            return false;
        };
        let dr = d.room(r);
        match dr.kind {
            RoomKind::Outdoor => true,
            RoomKind::Preset => dr.flags & crate::drlg::room_flags::NO_LOS_DRAW != 0,
            RoomKind::Other(_) => false,
        }
    }

    /// Whether the room's level is a town. A room without a DRLG room is
    /// not.
    pub fn in_town(&self, game: &Game, room: RoomId) -> bool {
        self.level_id(game, room).is_some_and(is_town)
    }

    /// The sub-tile rectangle of an active room.
    pub fn subtiles(&self, game: &Game, room: RoomId) -> Option<TileRect> {
        let (d, r) = self.drlg_room(game, room)?;
        d.active_room(r).map(|a| a.subtiles)
    }

    /// The collision mask at sub-tile (x, y) in the act of `room` (each
    /// sub-tile belongs to one active room's grid).
    pub fn collision(&self, game: &Game, room: RoomId, x: i32, y: i32) -> Option<u16> {
        let act = game.lists.room(room)?.act;
        self.dungeon
            .acts
            .get(usize::from(act))?
            .as_ref()?
            .collision_at(x, y)
    }

    /// Mutable collision mask at (x, y) in the act of `room`.
    pub fn collision_mut(&mut self, game: &Game, room: RoomId, x: i32, y: i32) -> Option<&mut u16> {
        let act = game.lists.room(room)?.act;
        self.dungeon
            .acts
            .get_mut(usize::from(act))?
            .as_mut()?
            .collision_at_mut(x, y)
    }

    /// The room containing (x, y) among `near` and its adjacency array, in
    /// that order.
    pub fn find_room(&self, game: &Game, near: RoomId, x: i32, y: i32) -> Option<RoomId> {
        let adjacent = game.lists.room(near)?.adjacent.clone();
        std::iter::once(near)
            .chain(adjacent.into_iter().filter(|&r| r != near))
            .find(|&r| self.subtiles(game, r).is_some_and(|t| t.contains(x, y)))
    }

    /// Sets or clears the DRLG room's flag 0x400000 (the one that makes the
    /// removal test false): set for `clear` false, cleared for true. `false`: `room` has no DRLG room here.
    pub fn refresh_room(&mut self, game: &Game, room: RoomId, clear: bool) -> bool {
        let Some(act) = game.lists.room(room).map(|r| r.act) else {
            return false;
        };
        let Ok(drlg) = self.act_mut(act) else {
            return false;
        };
        let Some(id) = drlg.drlg_room_of(room) else {
            return false;
        };
        let f = &mut drlg.room_mut(id).flags;
        if clear {
            *f &= !crate::drlg::room_flags::PORTAL;
        } else {
            *f |= crate::drlg::room_flags::PORTAL;
        }
        true
    }

    /// The tick's room-removal step: the room's inactivity counter.
    pub fn room_inactivity(&mut self, game: &Game, room: RoomId) -> Result<u32, WiringError> {
        let act = game.lists.room(room).map_or(0, |r| r.act);
        let drlg = self.act_mut(act)?;
        drlg.room_inactivity(room).map_err(WiringError::Drlg)
    }

    /// The tick's room-removal step: whether the act allows removing the room.
    pub fn allows_removal(&self, act: u8, room: RoomId) -> Result<bool, WiringError> {
        let drlg = self
            .dungeon
            .acts
            .get(usize::from(act))
            .and_then(Option::as_ref)
            .ok_or(WiringError::Drlg(crate::drlg::DrlgError::NotActive))?;
        drlg.allows_removal(room).map_err(WiringError::Drlg)
    }

    /// The rest of the active-room removal after the tick's room-removal
    /// step unlinked the room: neighbours' arrays, record free, other flags,
    /// tiles.
    pub fn remove_active_room(
        &mut self,
        lists: &mut UnitLists,
        act: u8,
        room: RoomId,
    ) -> Result<(), WiringError> {
        self.with_act(act, lists, |d, svc| d.remove_active_room(svc, room))
            .unwrap_or(Err(crate::drlg::DrlgError::NotActive))
            .map_err(WiringError::Drlg)
    }

    /// The tick's inactive-room step: frees the act's inactive levels.
    pub fn free_inactive_rooms(&mut self, act: u8) -> Result<(), WiringError> {
        let Some(drlg) = self
            .dungeon
            .acts
            .get_mut(usize::from(act))
            .and_then(Option::as_mut)
        else {
            return Ok(());
        };
        drlg.free_inactive_levels(&self.data, &mut *self.types)
            .map_err(WiringError::Drlg)
    }

    /// The server room change of a client from `old` to `new` active rooms
    /// of one act: the rooms the client joined (new adjacency order) and
    /// left (old adjacency order), each with the fields of its S->C 0x07 /
    /// 0x08 (the DRLG room's tile x, tile y and level id) and its client
    /// count after the change.
    pub fn client_switches_room(
        &mut self,
        lists: &mut UnitLists,
        act: u8,
        client: ClientId,
        old: Option<RoomId>,
        new: Option<RoomId>,
    ) -> Result<RoomSwitch, WiringError> {
        self.with_act(act, lists, |d, svc| {
            let old = old.and_then(|r| d.drlg_room_of(r));
            let new = new.and_then(|r| d.drlg_room_of(r));
            let (joined, left) = d.client_switches_room(svc, client, old, new)?;
            let side = |rooms: Vec<DrlgRoomId>| -> Vec<SwitchedRoom> {
                rooms
                    .into_iter()
                    .filter_map(|r| {
                        let room = d.room(r);
                        let a = room.active()?;
                        Some(SwitchedRoom {
                            room: a.id,
                            tile_x: room.rect.x,
                            tile_y: room.rect.y,
                            level: d.level(room.level).id,
                            clients: a.clients.len(),
                        })
                    })
                    .collect()
            };
            let view = new
                .map(|n| {
                    let adjacency = d.active_room(n).map(|a| a.adjacency.clone());
                    d.view_rooms(n)
                        .into_iter()
                        .filter(|r| !adjacency.as_ref().is_some_and(|a| a.contains(r)))
                        .filter_map(|r| d.room(r).active().map(|a| a.id))
                        .collect()
                })
                .unwrap_or_default();
            Ok(RoomSwitch {
                joined: side(joined),
                left: side(left),
                view,
            })
        })
        .unwrap_or(Err(crate::drlg::DrlgError::NotActive))
        .map_err(WiringError::Drlg)
    }

    /// Whether an active room is ready; `None` when the room has no DRLG
    /// room.
    pub fn room_ready(&self, game: &Game, room: RoomId) -> Option<bool> {
        let act = game.lists.room(room)?.act;
        let drlg = self.dungeon.acts.get(usize::from(act))?.as_ref()?;
        drlg.room_ready(room, |r| game.lists.room(r).is_some_and(|e| e.populated))
    }

    fn act_mut(&mut self, act: u8) -> Result<&mut Drlg, WiringError> {
        self.dungeon
            .acts
            .get_mut(usize::from(act))
            .and_then(Option::as_mut)
            .ok_or(WiringError::Drlg(crate::drlg::DrlgError::NotActive))
    }
}

impl DrlgWorld {
    /// Update live view holds and return the map/unit membership changes for
    /// every client. The caller sends the room switch messages and stores the
    /// refreshed view room list, without moving any player.
    pub fn resize_view_gap(
        &mut self,
        lists: &mut UnitLists,
        gap: i32,
    ) -> Result<Vec<(ClientId, RoomSwitch)>, WiringError> {
        let mut changes = Vec::new();
        for act in 0..5 {
            if self.dungeon.acts[usize::from(act)].is_none() {
                continue;
            }
            let clients: Vec<_> = lists
                .clients()
                .into_iter()
                .filter_map(|client| {
                    let room = lists.client(client)?.room?;
                    (lists.room(room)?.act == act).then_some((client, room))
                })
                .collect();
            let refreshed = self
                .with_act(act, lists, |d, svc| {
                    let anchors: Vec<_> = clients
                        .iter()
                        .filter_map(|&(c, r)| d.drlg_room_of(r).map(|r| (c, r)))
                        .collect();
                    let delta = d.resize_view_gap(svc, gap, &anchors)?;
                    let mut result = Vec::new();
                    for change in delta {
                        let side = |rooms: Vec<DrlgRoomId>| {
                            rooms
                                .into_iter()
                                .filter_map(|r| {
                                    let room = d.room(r);
                                    let a = room.active()?;
                                    Some(SwitchedRoom {
                                        room: a.id,
                                        tile_x: room.rect.x,
                                        tile_y: room.rect.y,
                                        level: d.level(room.level).id,
                                        clients: a.clients.len(),
                                    })
                                })
                                .collect()
                        };
                        let adjacency = d
                            .active_room(change.anchor)
                            .map(|a| a.adjacency.clone())
                            .unwrap_or_default();
                        let view = d
                            .view_rooms(change.anchor)
                            .into_iter()
                            .filter(|r| !adjacency.contains(r))
                            .filter_map(|r| d.active_room(r).map(|a| a.id))
                            .collect();
                        result.push((
                            change.client,
                            RoomSwitch {
                                joined: side(change.joined),
                                left: side(change.left),
                                view,
                            },
                        ));
                    }
                    Ok(result)
                })
                .unwrap_or(Err(crate::drlg::DrlgError::NotActive))
                .map_err(WiringError::Drlg)?;
            changes.extend(refreshed);
        }
        // Future acts inherit the full integer range too.
        self.dungeon.view_gap = gap.max(crate::drlg::NEAR_GAP);
        Ok(changes)
    }
}
