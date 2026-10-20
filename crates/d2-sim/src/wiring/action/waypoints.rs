//! Waypoints ↔ DRLG levels and warps: [`WaypointView`] implements
//! [`WaypointWorld`]. Real providers: the frame and difficulty, the
//! player's records (kept per player in [`super::ActionHooks`]), objects
//! and players from the unit lists and records with the level of their
//! room (DRLG), room rectangles (active-room sub-tiles), the ENDANIM
//! timer, and the DRLG spawn search; with the path provider
//! ([`crate::wiring::path`]) the same-act warp (spawn point, free
//! coordinates, placement) and the arrival mode request; object modes on
//! the object state ([`super::objects`]). The act change, the modes of
//! objects without object data, sounds and messages go to [`Pending`];
//! the player's interact info is the unit record's
//! ([`crate::units::record::InteractInfo`]).

use crate::drlg::act_of_level;
use crate::game::Game;
use crate::tick::events::event;
use crate::units::{RoomId, UnitId, UnitType};
use crate::world::waypoints::{ObjectFacts, PlayerFacts, RoomRect, WaypointRecords, WaypointWorld};

use super::{Pending, View, WiringError};

/// Warps `player` to `tile_code` of `level`: with the path provider, the
/// same-act warp is the spawn point and placement of
/// [`crate::wiring::path::place::level_warp`], an act change is
/// [`crate::wiring::path::act_change::run`]; a warp without the provider
/// (or without the destination act) goes to [`Pending::warp`]. The one
/// level warp of the action wiring (waypoints, quests).
pub fn level_warp<X: Pending>(
    v: &mut View<'_, X>,
    game: &mut Game,
    player: UnitId,
    level: u32,
    tile_code: u8,
) {
    if v.h.paths.is_some() {
        let c = crate::wiring::path::PathCtx::of(v, game);
        if crate::wiring::path::place::level_warp(c, player, level, u32::from(tile_code)).is_some()
        {
            return;
        }
        // Another act: the act change (`wiring::path::act_change`).
        let c = crate::wiring::path::PathCtx::of(v, game);
        if crate::wiring::path::act_change::run(c, player, level, u32::from(tile_code)) {
            return;
        }
    }
    v.h.x.warp(game, player, level, tile_code);
}

/// The waypoint code's view of a game.
pub struct WaypointView<'a, X> {
    /// The game being read and changed.
    pub game: &'a mut Game,
    /// The action wiring's view of the unit systems.
    pub v: View<'a, X>,
}

impl<X: Pending> WaypointView<'_, X> {
    /// The room of a unit: with the path provider, the path's room, as the
    /// position; without it, the unit list's room.
    fn room_and_level(&self, u: UnitId) -> (Option<RoomId>, Option<u32>) {
        let room = match &self.v.h.paths {
            Some(p) => p.record(u).and_then(|r| r.room()),
            None => self.game.lists.unit(u).and_then(|e| e.room()),
        };
        let level = room.and_then(|r| self.v.h.drlg.level_id(self.game, r));
        (room, level)
    }
}

impl<X: Pending> WaypointWorld for WaypointView<'_, X> {
    fn frame(&self) -> i32 {
        self.game.frame
    }
    fn tick_rate(&self) -> crate::tick::TickRate {
        self.game.tick_rate
    }
    fn difficulty(&self) -> u8 {
        self.v.data.difficulty
    }
    /// A player unit's records (created empty on first use; `None` for a
    /// unit that is not a player).
    fn records(&mut self, player: UnitId) -> Option<&mut WaypointRecords> {
        let is_player = self
            .game
            .lists
            .unit(player)
            .is_some_and(|e| e.ty == UnitType::Player);
        is_player.then(|| self.v.h.waypoints.entry(player).or_default())
    }
    /// The object with `guid`, with its class, mode, room, level and position.
    fn object(&self, guid: u32) -> Option<(UnitId, ObjectFacts)> {
        let u = self.game.lists.find_unit(UnitType::Object, guid)?;
        let r = self.v.units.get(u)?;
        let (room, level) = self.room_and_level(u);
        let (x, y) = self.v.h.path_position(u);
        Some((
            u,
            ObjectFacts {
                guid,
                class: r.class as u16,
                mode: r.mode as u8,
                room,
                level,
                x,
                y,
            },
        ))
    }
    fn player(&self, player: UnitId) -> PlayerFacts {
        let (room, level) = self.room_and_level(player);
        let (x, y) = self.v.h.path_position(player);
        let r = self.v.units.get(player);
        PlayerFacts {
            guid: self.game.lists.unit(player).map_or(0, |e| e.guid),
            class: r.map_or(0, |r| r.class as u8),
            room,
            level,
            x,
            y,
        }
    }
    /// The active room's sub-tile rectangle.
    fn room_rect(&self, room: RoomId) -> RoomRect {
        let t = self.v.h.drlg.subtiles(self.game, room).unwrap_or_default();
        RoomRect {
            x: t.x,
            y: t.y,
            width: t.w,
            height: t.h,
        }
    }
    /// Sets the object's mode on the object state for an object with object
    /// data; otherwise [`Pending::set_object_mode`].
    fn set_object_mode(&mut self, object: UnitId, mode: u8) {
        if self.v.object_set_mode(self.game, object, mode) {
            return;
        }
        self.v.h.x.set_object_mode(self.game, object, mode);
    }
    /// Schedules the ENDANIM event on the object at `frame`.
    fn schedule_endanim(&mut self, object: UnitId, frame: i32) {
        if let Err(e) =
            self.game
                .schedule_event(object, u32::from(event::END_ANIM), frame, None, 0, 0)
        {
            self.v.unit_error(e.into());
        }
    }
    /// Whether the player is busy: the interact info on the player's unit
    /// record is active, or [`Pending::busy`] (the cursor) says so.
    fn player_busy(&self, player: UnitId) -> bool {
        self.v.units.get(player).is_some_and(|r| r.interact.active) || self.v.h.x.busy(player)
    }
    /// Sets the interact info on the player's unit record (ignored while
    /// active).
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32) {
        if let Some(r) = self.v.units.get_mut(player) {
            r.interact.set(unit_type, guid);
        }
    }
    /// Resets the interact info on the player's unit record.
    fn reset_interact(&mut self, player: UnitId) {
        if let Some(r) = self.v.units.get_mut(player) {
            r.interact.reset();
        }
    }
    /// The GUID of the player's interact unit, if any; every caller compares
    /// it with an existing unit's GUID.
    fn interact_guid(&self, player: UnitId) -> Option<u32> {
        let (_, guid) = self.v.units.get(player)?.interact.get()?;
        Some(guid)
    }
    fn hostile_delay(&self, player: UnitId) -> bool {
        self.v.h.x.hostile_delay(player)
    }
    fn attach_sound(&mut self, player: UnitId, ev: u8) {
        self.v.h.x.attach_sound(player, ev);
    }
    fn send(&mut self, player: UnitId, msg: &[u8]) {
        self.v.h.x.send(player, msg);
    }
    /// Warps the player: with the path provider, the same-act warp is the
    /// spawn point and placement of
    /// [`crate::wiring::path::place::level_warp`], an act change is
    /// [`crate::wiring::path::act_change::run`]; a warp without the
    /// provider (or without the destination act) goes to [`Pending::warp`].
    fn warp(&mut self, player: UnitId, level: u32, tile_code: u8) {
        level_warp(&mut self.v, self.game, player, level, tile_code);
    }
    /// The room the DRLG spawn search chooses for `tile_code` of `level`,
    /// as an active room.
    ///
    /// Known gap: it is not known whether the game streams the chosen room
    /// in this search as it does in the full spawn search. The DRLG search
    /// is run as is (it streams the room). An act without a DRLG gives
    /// none.
    fn spawn_room(&mut self, level: u32, tile_code: u8) -> Option<RoomId> {
        let act = act_of_level(level);
        let r = self
            .v
            .h
            .drlg
            .with_act(act, &mut self.game.lists, |d, svc| {
                d.spawn_room(svc, level, u32::from(tile_code))
            })?;
        match r {
            Ok(p) => p.active,
            Err(e) => {
                self.v.h.errors.push(WiringError::Drlg(e));
                None
            }
        }
    }
    /// Requests the arrival mode (walk mode 2) at the player's own position
    /// with the path provider.
    fn set_player_mode_arrival(&mut self, player: UnitId) {
        if self.v.h.paths.is_some() {
            let (x, y) = self.v.h.path_position(player);
            let mut c = crate::wiring::path::PathCtx::of(&mut self.v, self.game);
            c.walk_to(player, 2, x, y);
            return;
        }
        self.v.h.x.set_player_mode_arrival(self.game, player);
    }
}
