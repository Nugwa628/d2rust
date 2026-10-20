//! The placement seams of `path::place` / `path::warp` on the unit
//! system and the DRLG: [`CollisionView`], [`PlaceHost`] and
//! [`LevelView`] on [`Shared`], three handles to one [`PathCtx`] (the
//! placement code takes them as separate arguments and calls them one
//! after another, so each call borrows the context for its own
//! duration). Entry points: [`place_unit`], [`game_entry`],
//! [`level_warp`]
//! (the same-act part of a level warp), [`warp_player`];
//! on [`Rooms`] (the DRLG alone): [`floor_drop`] and [`coarse_free_box`].

use std::cell::RefCell;

use crate::drlg::{act_of_level, TileRect};
use crate::game::Game;
use crate::path::collision::{box_value, find_room, point_value, size_value};
use crate::path::coords::Point;
use crate::path::place_seams::{
    CollisionView, LevelView, PlaceError, PlaceHost, PlaceMessage, RoomReveal,
};
use crate::path::search::ExpField;
use crate::path::warp::WarpOutcome;
use crate::path::CollisionRooms;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::action::{DrlgWorld, HirelingCall, Pending, View, WiringError};

use super::walk::PathCtx;

/// One handle to a shared [`PathCtx`].
pub struct Shared<'c, 'a, X>(pub &'c RefCell<PathCtx<'a, X>>);

/// The DRLG rooms alone, for the free-point searches, which
/// call only the room collision queries: its unit methods answer "no path" and its
/// moves do nothing (no search calls them). Any [`CollisionRooms`] (the
/// client's grids too); the server's DRLG by default.
pub struct Rooms<'a, R: ?Sized = DrlgWorld>(pub &'a R);

impl<R: CollisionRooms + ?Sized> CollisionView for Rooms<'_, R> {
    type Room = RoomId;
    type Unit = UnitId;

    fn cell_room(&self, hint: Option<RoomId>, x: i32, y: i32) -> Option<RoomId> {
        find_room(self.0, hint, x, y)
    }
    fn room_rect(&self, room: RoomId) -> TileRect {
        self.0
            .subtile_rect(room)
            .unwrap_or(TileRect::new(0, 0, 0, 0))
    }
    /// The room's collision value, unmasked (0x27 without a room or grid).
    fn cell_value(&self, room: RoomId, x: i32, y: i32) -> u32 {
        u32::from(point_value(self.0, Some(room), x, y, 0xFFFF))
    }
    fn point_query(&self, room: RoomId, x: i32, y: i32, mask: u32) -> u32 {
        u32::from(point_value(self.0, Some(room), x, y, mask as u16))
    }
    fn size_query(&self, room: RoomId, x: i32, y: i32, size: i32, mask: u32) -> u32 {
        u32::from(size_value(self.0, Some(room), x, y, size, mask as u16))
    }
    fn box_query(&self, room: RoomId, x: i32, y: i32, sx: u32, sy: u32, mask: u32) -> u32 {
        u32::from(box_value(self.0, Some(room), x, y, (sx, sy), mask as u16))
    }
    fn has_path(&self, _: UnitId) -> bool {
        false
    }
    fn unit_room(&self, _: UnitId) -> Option<RoomId> {
        None
    }
    fn unit_size(&self, _: UnitId) -> i32 {
        0
    }
    fn teleport(&mut self, _: UnitId, _: RoomId, _: i32, _: i32) {}
    fn add_player_to_world(&mut self, _: UnitId, _: RoomId, _: i32, _: i32) {}
}

impl<X: Pending> CollisionView for Shared<'_, '_, X> {
    type Room = RoomId;
    type Unit = UnitId;

    fn cell_room(&self, hint: Option<RoomId>, x: i32, y: i32) -> Option<RoomId> {
        Rooms(&self.0.borrow().v.h.drlg).cell_room(hint, x, y)
    }
    fn room_rect(&self, room: RoomId) -> TileRect {
        Rooms(&self.0.borrow().v.h.drlg).room_rect(room)
    }
    fn cell_value(&self, room: RoomId, x: i32, y: i32) -> u32 {
        Rooms(&self.0.borrow().v.h.drlg).cell_value(room, x, y)
    }
    fn point_query(&self, room: RoomId, x: i32, y: i32, mask: u32) -> u32 {
        Rooms(&self.0.borrow().v.h.drlg).point_query(room, x, y, mask)
    }
    fn size_query(&self, room: RoomId, x: i32, y: i32, size: i32, mask: u32) -> u32 {
        Rooms(&self.0.borrow().v.h.drlg).size_query(room, x, y, size, mask)
    }
    fn box_query(&self, room: RoomId, x: i32, y: i32, sx: u32, sy: u32, mask: u32) -> u32 {
        Rooms(&self.0.borrow().v.h.drlg).box_query(room, x, y, sx, sy, mask)
    }
    fn has_path(&self, unit: UnitId) -> bool {
        self.0.borrow().v.h.path_has(unit)
    }
    fn unit_room(&self, unit: UnitId) -> Option<RoomId> {
        let c = self.0.borrow();
        c.v.h.paths.as_ref()?.record(unit)?.room()
    }
    fn unit_size(&self, unit: UnitId) -> i32 {
        self.0.borrow().v.path_size(unit)
    }
    fn teleport(&mut self, unit: UnitId, room: RoomId, x: i32, y: i32) {
        self.0.borrow_mut().teleport(unit, Some(room), x, y);
    }
    /// Places the game-entry player (add with flag 0): the
    /// player's path is allocated at the point, the unit moved to
    /// the room's list, and the room-changed flag (path flag 0x2) set
    fn add_player_to_world(&mut self, unit: UnitId, room: RoomId, x: i32, y: i32) {
        let mut c = self.0.borrow_mut();
        let c = &mut *c;
        if let Err(e) = c.game.lists.change_room(unit, room) {
            c.v.h
                .errors
                .push(WiringError::Unit(crate::units::modes::UnitError::Game(
                    e.into(),
                )));
            return;
        }
        let game: &crate::game::Game = c.game;
        c.v.path_place(game, unit, x, y);
        if let Some(d) = c.v.h.paths.as_mut().and_then(|p| p.dynamic_mut(unit)) {
            d.flags |= crate::path::record::flags::ROOM_CHANGED;
        }
    }
    /// Game entry's room switch for the client whose
    /// player is `player` ([`crate::wiring::action::View::room_switch`]);
    /// a player without a client: nothing.
    fn client_room_switch(&mut self, player: UnitId, room: RoomId) {
        let mut c = self.0.borrow_mut();
        let c = &mut *c;
        let client = c
            .game
            .lists
            .clients()
            .into_iter()
            .find(|&k| c.game.lists.client(k).and_then(|e| e.player) == Some(player));
        if let Some(client) = client {
            c.v.room_switch(c.game, client, Some(room));
        }
    }
}

/// S→C 0x07 MapReveal (x u16 @1, y u16 @3,
/// level u8 @5).
pub fn map_reveal(x: u16, y: u16, level: u8) -> [u8; 6] {
    let [x0, x1] = x.to_le_bytes();
    let [y0, y1] = y.to_le_bytes();
    [0x07, x0, x1, y0, y1, level]
}

impl<X: Pending> PlaceHost<UnitId> for Shared<'_, '_, X> {
    /// Pets follow the player: queued for the host holding the pet lists
    /// ([`super::super::action::ActionHooks::pet_follows`]); none → nothing.
    fn pets_follow(&mut self, player: UnitId) {
        self.0.borrow_mut().v.h.queue_pet_follow(player);
    }
    fn is_player(&self, unit: UnitId) -> bool {
        self.0
            .borrow()
            .v
            .units
            .get(unit)
            .is_some_and(|r| r.ty == UnitType::Player)
    }
    fn queue_update(&mut self, unit: UnitId) {
        let mut c = self.0.borrow_mut();
        if let Err(e) = c.game.lists.queue_update(unit) {
            c.v.h
                .errors
                .push(WiringError::Unit(crate::units::modes::UnitError::Game(
                    e.into(),
                )));
        }
    }
    fn or_flags2(&mut self, unit: UnitId, bits: u32) {
        if let Some(r) = self.0.borrow_mut().v.units.get_mut(unit) {
            r.flags2 |= bits;
        }
    }
    fn room_change_messages(&mut self, unit: UnitId) {
        self.0.borrow_mut().room_change_messages(unit);
    }
    /// The message bytes go to [`Pending::send`] (the server transport):
    /// 0x07 by its layout, 0x15 / 0x0D by `path::walk::messages`, with
    /// the unit's type and GUID.
    fn send(&mut self, player: UnitId, msg: PlaceMessage<UnitId>) {
        let mut c = self.0.borrow_mut();
        let id = |c: &PathCtx<'_, X>, u: UnitId| {
            c.v.units.get(u).map_or((0, 0), |r| (r.ty as u8, r.guid))
        };
        let bytes: Vec<u8> = match msg {
            PlaceMessage::MapReveal { x, y, level } => map_reveal(x, y, level).to_vec(),
            PlaceMessage::ReassignPlayer { unit, x, y, flag } => {
                let (t, g) = id(&c, unit);
                crate::path::walk::messages::reassign_player(t, g, x, y, flag).to_vec()
            }
            PlaceMessage::PlayerStop {
                unit,
                a,
                x,
                y,
                b,
                life_pct,
            } => {
                let (t, g) = id(&c, unit);
                crate::path::walk::messages::player_stop(t, g, a, x, y, b, life_pct).to_vec()
            }
            PlaceMessage::GameEntryDone => crate::units::messages::GAME_ENTRY_DONE.to_vec(),
        };
        c.v.h.x.send(player, &bytes);
    }
    /// Timer event `event` at frame + `delay`; event 14 with the
    /// placement's callback.
    fn schedule_event(&mut self, unit: UnitId, event: u8, delay: u32) {
        let mut c = self.0.borrow_mut();
        let c = &mut *c;
        let callback = (event == crate::path::place_seams::PLACE_TIMER_EVENT)
            .then_some(crate::units::dispatch::SKILL_COOLDOWN);
        let expire = c.game.after(delay as i32);
        if let Err(e) = c
            .game
            .schedule_event(unit, u32::from(event), expire, callback, 0, 0)
        {
            c.v.h
                .errors
                .push(WiringError::Unit(crate::units::modes::UnitError::Game(e)));
        }
    }
    /// Writes the player's position history ([`super::PathState::history`]).
    fn history_write(&mut self, player: UnitId, x: i32, y: i32) {
        if let Some(p) = self.0.borrow_mut().v.h.paths.as_mut() {
            p.history.entry(player).or_default().place_write(x, y);
        }
    }
    fn request_walk(&mut self, player: UnitId, x: i32, y: i32) {
        self.0.borrow_mut().walk_to(player, 2, x, y);
    }
}

impl<X: Pending> LevelView<RoomId> for Shared<'_, '_, X> {
    type Act = u8;

    /// Streams the chosen spawn room on the act's DRLG; its active room and
    /// the position in tiles.
    fn spawn_room(&mut self, act: u8, level: u32, tile_index: u32) -> Option<(RoomId, i32, i32)> {
        let mut c = self.0.borrow_mut();
        let c = &mut *c;
        let r = c.v.h.drlg.with_act(act, &mut c.game.lists, |d, svc| {
            d.spawn_room(svc, level, tile_index)
        })?;
        match r {
            Ok(p) => Some((p.active?, p.x, p.y)),
            Err(e) => {
                c.v.h.errors.push(WiringError::Drlg(e));
                None
            }
        }
    }
    /// The warp destination ([`super::warp_dest::destination`]).
    fn warp_destination(
        &self,
        tile_room: RoomId,
        tile_class: u32,
    ) -> Option<crate::path::place_seams::WarpDestination<RoomId>> {
        super::warp_dest::destination(&mut self.0.borrow_mut(), tile_room, tile_class)
    }
    /// The quest gate on a warp ([`Pending::warp_quest_gate`]).
    fn quest_gate(&self, source_level: u32, level: u32) -> u32 {
        self.0.borrow().v.h.x.warp_quest_gate(source_level, level)
    }
    /// The act's town level id.
    fn act_start_level(&self, act: u8) -> u32 {
        crate::drlg::TOWN_LEVELS
            .iter()
            .copied()
            .find(|&l| act_of_level(l) == act)
            .unwrap_or(0)
    }
    /// The room's DRLG room tile origin and level id.
    fn room_reveal(&self, room: RoomId) -> RoomReveal {
        let c = self.0.borrow();
        match c.v.h.drlg.drlg_room(c.game, room) {
            Some((d, r)) => {
                let dr = d.room(r);
                RoomReveal {
                    tile_x: dr.rect.x,
                    tile_y: dr.rect.y,
                    level: d.level(dr.level).id,
                }
            }
            None => RoomReveal {
                tile_x: 0,
                tile_y: 0,
                level: 0,
            },
        }
    }
}

/// Runs `f` on three handles of one context.
pub(super) fn with_shared<X: Pending, R>(
    c: PathCtx<'_, X>,
    f: impl FnOnce(&mut Shared<'_, '_, X>, &mut Shared<'_, '_, X>, &mut Shared<'_, '_, X>) -> R,
) -> R {
    let cell = RefCell::new(c);
    let (mut a, mut b, mut l) = (Shared(&cell), Shared(&cell), Shared(&cell));
    f(&mut a, &mut b, &mut l)
}

/// Places a unit `(game, unit, room, x, y, exact, alt)`: `true` placed.
/// Fatal asserts are logged as [`WiringError::Place`].
#[allow(clippy::too_many_arguments)]
pub fn place_unit<X: Pending>(
    c: PathCtx<'_, X>,
    unit: UnitId,
    room: Option<RoomId>,
    x: i32,
    y: i32,
    exact: bool,
    alt: bool,
) -> bool {
    with_shared(c, |cv, host, lv| {
        let r = crate::path::place::place_unit(cv, host, &*lv, unit, room, x, y, exact, alt);
        log(cv, r).unwrap_or(false)
    })
}

/// Game entry of a player not yet placed: the
/// spawn point of act `act`'s town (tile index 0, the unit's size), S→C
/// 0x07 for the spawn room, the client's room switch (S→C 0x07 and the
/// add messages of every room of the spawn room's adjacency array,
/// add messages of every room of the spawn room's adjacency array),
/// the world add (flag 0), S→C 0x15 with
/// assert (no spawn room, no free point, no act) is logged as
/// [`WiringError::Place`] and gives `false`.
///
/// A placed player's join follow (the hireling joins the player)
/// is queued for the host that holds the
/// hireling lists ([`super::super::action::ActionHooks::hireling_calls`];
/// none → nothing).
pub fn game_entry<X: Pending>(c: PathCtx<'_, X>, player: UnitId, act: u8) -> bool {
    with_shared(c, |cv, host, lv| {
        let r = crate::path::place::game_entry(cv, host, lv, player, act);
        let placed = log(cv, r).unwrap_or(false);
        if placed {
            let mut sh = cv.0.borrow_mut();
            // The player's unit record carries the entry act from the join
            // on (0-based: an Act IV start carries act 3 from its first frames);
            // an act change updates it.
            if let Some(r) = sh.v.units.get_mut(player) {
                r.act = act;
            }
            // ByTime's t from the entry on: read
            // live from the act's environment record, 0 for a fresh act.
            let t = act_time_of(sh.game, act);
            sh.v.h.act_times.insert(player, t);
            if let Some(q) = sh.v.h.hireling_calls.as_mut() {
                q.push(HirelingCall::JoinFollow(player));
            }
        }
        placed
    })
}

/// ByTime's base time t of act `act`: the
/// built act's environment time, 0 for an act not advanced yet.
pub(crate) fn act_time_of(game: &Game, act: u8) -> i32 {
    game.lists
        .act(act)
        .filter(|a| a.built)
        .map_or(0, |a| a.environment.time())
}

/// Level spawn point `(act, level, tile index, &x, &y, 3)`:
/// the room and point, `None` without a spawn room or a free
/// point (the portal creation's destination).
pub fn level_spawn<X: Pending>(
    v: &mut View<'_, X>,
    game: &mut Game,
    act: u8,
    level: u32,
    tile_index: u32,
) -> Option<(RoomId, i32, i32)> {
    let c = PathCtx::of(v, game);
    with_shared(c, |cv, _, lv| {
        let r = crate::path::place::level_spawn_point(cv, lv, Some(act), level, tile_index, 3);
        log(cv, r).flatten().map(|(room, p)| (room, p.x, p.y))
    })
}

/// The same-act level warp: spawn point then placement (exact 0, alt 0).
/// `None`: the destination is in another act: the caller runs the act
/// change ([`super::act_change::run`]).
pub fn level_warp<X: Pending>(
    mut c: PathCtx<'_, X>,
    player: UnitId,
    level: u32,
    tile_index: u32,
) -> Option<bool> {
    let act = act_of_level(level);
    let own = c
        .game
        .lists
        .unit(player)
        .and_then(|e| e.room())
        .and_then(|r| c.game.lists.room(r))
        .map(|r| r.act);
    if own.is_some_and(|a| a != act) {
        // The act change: its hireling part (the hireling follows across
        // acts) is queued for the host that holds the hireling
        // lists ([`super::super::action::ActionHooks::hireling_calls`]).
        if let Some(q) = c.v.h.hireling_calls.as_mut() {
            q.push(HirelingCall::ActChange(player));
        }
        // The town-leave refresh runs first, from the level
        // of the player's room (still the old act's): its vendor part
        // (the vendors' stock reset) is queued for the host
        // ([`super::super::action::ActionHooks::town_leaves`]); its quest
        // part (the act's NPC intro list and, for
        // Lut Gholein, Cain's act change) runs here.
        let from = c
            .game
            .lists
            .unit(player)
            .and_then(|e| e.room())
            .and_then(|r| c.v.h.drlg.level_id(c.game, r));
        if let Some(from) = from {
            let empty = super::super::action::town_left_empty(c.game, c.v.stats, c.v.h, from);
            c.v.h.town_leaves.push((player, from, empty));
        }
        if let Some(mut host) = c.v.h.quest_host.take() {
            host.town_leave(c.game, &mut c.v, player, level);
            c.v.h.quest_host = Some(host);
        }
        return None;
    }
    Some(with_shared(c, |cv, host, lv| {
        let r = crate::path::place::level_warp_place(cv, host, lv, player, act, level, tile_index);
        log(cv, r).unwrap_or(false)
    }))
}

/// Warps a player to a tile `(game, player, tile)`.
pub fn warp_player<X: Pending>(
    c: PathCtx<'_, X>,
    player: UnitId,
    tile_room: RoomId,
    tile_class: u32,
) -> Option<WarpOutcome> {
    with_shared(c, |cv, host, lv| {
        let r = crate::path::warp::warp_player(cv, host, &*lv, player, tile_room, tile_class);
        log(cv, r)
    })
}

/// Floor drop `(room, &start, origin = from, size, 0x3E01,
/// 0x801, fallback)` on `field`: the room
/// (`None`: no place) and the point.
pub fn floor_drop(
    drlg: &DrlgWorld,
    field: &ExpField,
    room: Option<RoomId>,
    from: Point,
    size: i32,
    fallback: bool,
) -> Result<(Option<RoomId>, Point), PlaceError> {
    crate::path::place::floor_drop(&Rooms(drlg), field, room, from, size, fallback)
}

/// Coarse free-box search `(room, &point, n, mask)`.
pub fn coarse_free_box(
    drlg: &DrlgWorld,
    room: RoomId,
    point: &mut Point,
    n: i32,
    mask: u32,
) -> Option<RoomId> {
    crate::path::search::coarse_free_box(&Rooms(drlg), room, point, n, mask)
}

pub(super) fn log<X: Pending, T>(
    cv: &mut Shared<'_, '_, X>,
    r: Result<T, PlaceError>,
) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            cv.0.borrow_mut().v.h.errors.push(WiringError::Place(e));
            None
        }
    }
}
