//! Floor drop placement, placing a unit at a point,
//! the level spawn point and game entry.
//! Nothing here draws (the spawn tile pick of
//! the DRLG behind [`LevelView::spawn_room`] does).

use super::coords::Point;
use super::place_seams::{
    flags2, mask, CollisionView, LevelView, PlaceError, PlaceHost, PlaceMessage, PLACE_TIMER_DELAY,
    PLACE_TIMER_EVENT,
};
use super::search::{free_point, free_point_field, ExpField};

/// Floor drop start offset from the dropper.
pub const DROP_START_DX: i32 = 2;
pub const DROP_START_DY: i32 = 3;
/// Level spawn offset in sub-tiles on both axes.
pub const SPAWN_OFFSET: i32 = 3;
/// Sub-tiles per tile.
pub const SUBTILES_PER_TILE: i32 = 5;

/// Floor drop placement.
/// Returns the room (none: no place; with fallback the unchanged
/// start's room) and the out point (the point the search wrote).
pub fn floor_drop<C: CollisionView>(
    cv: &C,
    field: &ExpField,
    room: Option<C::Room>,
    from: Point,
    size: i32,
    fallback: bool,
) -> Result<(Option<C::Room>, Point), PlaceError> {
    // Rule 1.
    let shifted = Point::new(from.x + DROP_START_DX, from.y + DROP_START_DY);
    let mut start = if cv.cell_room(room, shifted.x, shifted.y).is_some() {
        shifted
    } else {
        from
    };
    // Rule 2.
    let r = free_point_field(
        cv,
        field,
        room,
        &mut start,
        from,
        size,
        mask::ITEM_FLOOR,
        mask::FIELD,
        fallback,
    )?;
    // Rule 3.
    Ok((r, start))
}

/// Placing a unit at a point `(game, unit, room, x, y, exact,
/// alt)`. `Ok(true)`: placed; `Ok(false)`: not.
#[allow(clippy::too_many_arguments)]
pub fn place_unit<C, H, L>(
    cv: &mut C,
    host: &mut H,
    levels: &L,
    unit: C::Unit,
    room: Option<C::Room>,
    x: i32,
    y: i32,
    exact: bool,
    alt: bool,
) -> Result<bool, PlaceError>
where
    C: CollisionView,
    H: PlaceHost<C::Unit>,
    L: LevelView<C::Room>,
{
    // Rule 1.
    if !cv.has_path(unit) {
        return Err(PlaceError::NoPath);
    }
    // Rule 2: the first lookup has a null hint (edge case 7).
    let room = match room {
        Some(r) => r,
        None => match cv
            .cell_room(None, x, y)
            .or_else(|| cv.cell_room(cv.unit_room(unit), x, y))
        {
            Some(r) => r,
            None => return Ok(false),
        },
    };
    // Rule 3.
    let mut p = Point::new(x, y);
    let room = if exact {
        room
    } else {
        let size = cv.unit_size(unit);
        match free_point(cv, Some(room), &mut p, size, mask::PLAYER_PLACE, false)? {
            Some(r) => r,
            None => return Ok(false),
        }
    };
    // Rule 4.
    cv.teleport(unit, room, p.x, p.y);
    let bits = if alt {
        flags2::PLACED_ALT
    } else {
        flags2::PLACED
    };
    if !host.is_player(unit) {
        // Rule 5.
        host.queue_update(unit);
        host.or_flags2(unit, bits);
        host.room_change_messages(unit);
        return Ok(true);
    }
    // Rule 6.
    let reveal = levels.room_reveal(room);
    host.send(
        unit,
        PlaceMessage::MapReveal {
            x: reveal.tile_x as u16,
            y: reveal.tile_y as u16,
            level: reveal.level as u8,
        },
    );
    host.queue_update(unit);
    host.or_flags2(unit, bits);
    host.room_change_messages(unit);
    host.set_player_point(unit, p.x, p.y);
    // Rule 7: the position history write.
    host.history_write(unit, p.x, p.y);
    host.schedule_event(unit, PLACE_TIMER_EVENT, PLACE_TIMER_DELAY);
    host.pets_follow(unit);
    Ok(true)
}

/// Level spawn point
/// (rules 1–4). `Ok(None)`: no spawn room (result 0).
pub fn level_spawn_point<C, L>(
    cv: &C,
    levels: &mut L,
    act: Option<L::Act>,
    level: u32,
    tile_index: u32,
    size: i32,
) -> Result<Option<(C::Room, Point)>, PlaceError>
where
    C: CollisionView,
    L: LevelView<C::Room>,
{
    // Rule 1.
    let act = act.ok_or(PlaceError::NoAct)?;
    let Some((room, tx, ty)) = levels.spawn_room(act, level, tile_index) else {
        return Ok(None);
    };
    // Rule 2.
    let mut p = Point::new(
        tx * SUBTILES_PER_TILE + SPAWN_OFFSET,
        ty * SUBTILES_PER_TILE + SPAWN_OFFSET,
    );
    // Rule 3 (edge case 6).
    let found = free_point(cv, Some(room), &mut p, size, mask::PLAYER_PLACE, false)?
        .ok_or(PlaceError::SpawnNotFree)?;
    // Rule 4.
    Ok(cv.cell_room(Some(found), p.x, p.y).map(|r| (r, p)))
}

/// Game entry for a player not yet placed: the spawn
/// point of the act's start level with tile index 0 and size = the
/// unit's size (2 for a player); no spawn room is a fatal
/// assert ([`PlaceError::NoSpawnRoom`]); S→C 0x07 for the spawn room,
/// the client's room switch to it ([`CollisionView::client_room_switch`]:
/// a 0x07 for every room of its adjacency array);
/// puts the player in the world (the room-changed
/// flag is set), S→C 0x15 with flag 1, S→C 0x7E. Every message goes to
/// the player's own client. Followers are the caller's.
pub fn game_entry<C, H, L>(
    cv: &mut C,
    host: &mut H,
    levels: &mut L,
    player: C::Unit,
    act: L::Act,
) -> Result<bool, PlaceError>
where
    C: CollisionView,
    H: PlaceHost<C::Unit>,
    L: LevelView<C::Room>,
{
    let level = levels.act_start_level(act);
    let size = cv.unit_size(player);
    let Some((room, p)) = level_spawn_point(cv, levels, Some(act), level, 0, size)? else {
        return Err(PlaceError::NoSpawnRoom);
    };
    let reveal = levels.room_reveal(room);
    host.send(
        player,
        PlaceMessage::MapReveal {
            x: reveal.tile_x as u16,
            y: reveal.tile_y as u16,
            level: reveal.level as u8,
        },
    );
    cv.client_room_switch(player, room);
    cv.add_player_to_world(player, room, p.x, p.y);
    host.send(
        player,
        PlaceMessage::ReassignPlayer {
            unit: player,
            x: p.x as u16,
            y: p.y as u16,
            flag: 1,
        },
    );
    host.send(player, PlaceMessage::GameEntryDone);
    Ok(true)
}

/// The placement of a same-act level warp: the spawn
/// point of `level` with the caller's tile index, then
/// There, so the free search runs twice.
/// The rest of the act-change exit and the act change itself
/// live elsewhere. The size is the unit's size, as
/// [`game_entry`]; no spawn room → nothing (`Ok(false)`, the player
/// stays).
pub fn level_warp_place<C, H, L>(
    cv: &mut C,
    host: &mut H,
    levels: &mut L,
    player: C::Unit,
    act: L::Act,
    level: u32,
    tile_index: u32,
) -> Result<bool, PlaceError>
where
    C: CollisionView,
    H: PlaceHost<C::Unit>,
    L: LevelView<C::Room>,
{
    let size = cv.unit_size(player);
    let Some((room, p)) = level_spawn_point(cv, levels, Some(act), level, tile_index, size)? else {
        return Ok(false);
    };
    place_unit(cv, host, levels, player, Some(room), p.x, p.y, false, false)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::place_seams::{RoomReveal, WarpDestination};
    use super::super::search::tests::{sign_field, FakeUnit, Grid};
    use super::*;

    /// Records every host call in order.
    #[derive(Default, Debug)]
    pub(crate) struct Host {
        pub players: Vec<usize>,
        pub log: Vec<String>,
        /// `spawn_room` answers: (room, tile x, tile y).
        pub spawn: Option<(usize, i32, i32)>,
        pub spawn_calls: Vec<(u32, u32, u32)>,
        pub warp: Option<WarpDestination<usize>>,
        pub gate: u32,
        pub life: u8,
    }

    impl PlaceHost<usize> for Host {
        fn is_player(&self, unit: usize) -> bool {
            self.players.contains(&unit)
        }
        fn queue_update(&mut self, unit: usize) {
            self.log.push(format!("queue {unit}"));
        }
        fn or_flags2(&mut self, unit: usize, bits: u32) {
            self.log.push(format!("flags2 {unit} {bits:#x}"));
        }
        fn room_change_messages(&mut self, unit: usize) {
            self.log.push(format!("roomchange {unit}"));
        }
        fn send(&mut self, player: usize, msg: PlaceMessage<usize>) {
            self.log.push(format!("send {player} {msg:?}"));
        }
        fn set_player_point(&mut self, player: usize, x: i32, y: i32) {
            self.log.push(format!("point {player} ({x},{y})"));
        }
        fn schedule_event(&mut self, unit: usize, event: u8, delay: u32) {
            self.log.push(format!("timer {unit} {event} +{delay}"));
        }
        fn pets_follow(&mut self, player: usize) {
            self.log.push(format!("pets {player}"));
        }
        fn request_walk(&mut self, player: usize, x: i32, y: i32) {
            self.log.push(format!("walk {player} ({x},{y})"));
        }
        fn life_percent(&self, _unit: usize) -> u8 {
            self.life
        }
    }

    impl LevelView<usize> for Host {
        type Act = u8;
        fn spawn_room(&mut self, act: u8, level: u32, tile: u32) -> Option<(usize, i32, i32)> {
            self.spawn_calls.push((u32::from(act), level, tile));
            self.spawn
        }
        fn act_start_level(&self, act: u8) -> u32 {
            [1, 40, 75, 103, 109][usize::from(act)]
        }
        fn room_reveal(&self, room: usize) -> RoomReveal {
            RoomReveal {
                tile_x: 100 + room as i32,
                tile_y: 200,
                level: 7,
            }
        }
        fn warp_destination(&self, _room: usize, _class: u32) -> Option<WarpDestination<usize>> {
            self.warp
        }
        fn quest_gate(&self, _s: u32, _d: u32) -> u32 {
            self.gate
        }
    }

    pub(crate) fn unit(g: &mut Grid, room: Option<usize>, player: bool) -> usize {
        g.units.push(FakeUnit {
            room,
            pos: Point::new(1, 1),
            size: if player { 2 } else { 1 },
            has_path: true,
        });
        g.units.len() - 1
    }

    fn drop_at(g: &Grid, x: i32, y: i32) -> (Option<usize>, Point) {
        floor_drop(g, &sign_field(), Some(0), Point::new(x, y), 1, true).unwrap()
    }

    #[test]
    fn d1_wall_column() {
        let mut g = Grid::vec20();
        for y in 0..20 {
            g.set(12, y, 0x1);
        }
        assert_eq!(drop_at(&g, 10, 10), (Some(0), Point::new(11, 13)));
    }

    #[test]
    fn d2_d3_floor_drop() {
        let g = Grid::vec20();
        assert_eq!(drop_at(&g, 10, 10), (Some(0), Point::new(12, 13)));
        let mut g = Grid::vec20();
        g.set(12, 13, 0x200);
        assert_eq!(drop_at(&g, 10, 10), (Some(0), Point::new(11, 13)));
        // Start (x + 2, y + 3) outside every room: the search starts at
        // the dropper's own cell.
        let g = Grid::vec20();
        assert_eq!(drop_at(&g, 18, 17), (Some(0), Point::new(18, 17)));
    }

    #[test]
    fn place_monster_searches_and_flags() {
        let mut g = Grid::vec20();
        g.set(10, 10, 0x1);
        let u = unit(&mut g, Some(0), false);
        let mut h = Host::default();
        let lv = Host::default();
        assert_eq!(
            place_unit(&mut g, &mut h, &lv, u, Some(0), 10, 10, false, false),
            Ok(true)
        );
        // Size 1, ring 1: (9, 9) d 2 kept first, (9, 10) d 1 replaces it.
        assert_eq!(g.teleports, [(u, 0, 9, 10)]);
        assert_eq!(h.log, ["queue 0", "flags2 0 0x10000", "roomchange 0"]);
        let mut h = Host::default();
        place_unit(&mut g, &mut h, &lv, u, Some(0), 3, 3, false, true).unwrap();
        assert_eq!(h.log[1], "flags2 0 0x800");
    }

    #[test]
    fn place_player_sequence() {
        let mut g = Grid::vec20();
        let u = unit(&mut g, Some(0), true);
        let mut h = Host {
            players: vec![u],
            ..Host::default()
        };
        let lv = Host::default();
        assert_eq!(
            place_unit(&mut g, &mut h, &lv, u, Some(0), 4, 5, false, false),
            Ok(true)
        );
        assert_eq!(
            h.log,
            [
                "send 0 MapReveal { x: 100, y: 200, level: 7 }",
                "queue 0",
                "flags2 0 0x10000",
                "roomchange 0",
                "point 0 (4,5)",
                "timer 0 14 +50",
                "pets 0",
            ]
        );
    }

    #[test]
    fn place_room_lookup_exact_and_failures() {
        let lv = Host::default();
        // No room given: looked up from the unit's room.
        let mut g = Grid::vec20();
        let u = unit(&mut g, Some(0), false);
        let mut h = Host::default();
        assert_eq!(
            place_unit(&mut g, &mut h, &lv, u, None, 4, 4, false, false),
            Ok(true)
        );
        // Unit without a room and no room given: not placed.
        let v = unit(&mut g, None, false);
        assert_eq!(
            place_unit(&mut g, &mut h, &lv, v, None, 4, 4, false, false),
            Ok(false)
        );
        // Exact: no search, even onto a wall.
        g.set(6, 6, 0x1);
        g.teleports.clear();
        place_unit(&mut g, &mut h, &lv, u, Some(0), 6, 6, true, false).unwrap();
        assert_eq!(g.teleports, [(u, 0, 6, 6)]);
        assert_eq!(g.units[u].pos, Point::new(6, 6));
        // No free point: not placed, nothing moved.
        let mut gw = Grid::vec20();
        for y in 0..20 {
            for x in 0..20 {
                gw.set(x, y, 0x8);
            }
        }
        let w = unit(&mut gw, Some(0), false);
        let mut h = Host::default();
        assert_eq!(
            place_unit(&mut gw, &mut h, &lv, w, Some(0), 6, 6, false, false),
            Ok(false)
        );
        assert!(gw.teleports.is_empty() && h.log.is_empty());
        // No path: fatal.
        g.units[u].has_path = false;
        assert_eq!(
            place_unit(&mut g, &mut h, &lv, u, Some(0), 6, 6, false, false),
            Err(PlaceError::NoPath)
        );
    }

    #[test]
    fn spawn_point() {
        let g = Grid::vec20();
        let mut lv = Host {
            spawn: Some((0, 2, 1)),
            ..Host::default()
        };
        // Tile (2, 1) → (13, 8).
        assert_eq!(
            level_spawn_point(&g, &mut lv, Some(0), 1, 0, 2),
            Ok(Some((0, Point::new(13, 8))))
        );
        assert_eq!(lv.spawn_calls, [(0, 1, 0)]);
        // A wall there: the free search moves it.
        let mut g2 = Grid::vec20();
        g2.set(13, 8, 0x1);
        assert_eq!(
            level_spawn_point(&g2, &mut lv, Some(0), 1, 0, 1),
            Ok(Some((0, Point::new(12, 8))))
        );
        // No spawn room: result 0.
        lv.spawn = None;
        assert_eq!(level_spawn_point(&g, &mut lv, Some(0), 1, 0, 2), Ok(None));
        // No act: fatal.
        assert_eq!(
            level_spawn_point(&g, &mut lv, None, 1, 0, 2),
            Err(PlaceError::NoAct)
        );
        // No free point: fatal.
        let mut gw = Grid::vec20();
        for y in 0..20 {
            for x in 0..20 {
                gw.set(x, y, 0x1);
            }
        }
        lv.spawn = Some((0, 2, 1));
        assert_eq!(
            level_spawn_point(&gw, &mut lv, Some(0), 1, 0, 2),
            Err(PlaceError::SpawnNotFree)
        );
    }

    #[test]
    fn game_entry_and_level_warp() {
        let mut g = Grid::vec20();
        let p = unit(&mut g, None, true);
        let mut h = Host {
            players: vec![p],
            ..Host::default()
        };
        let mut lv = Host {
            spawn: Some((0, 2, 1)),
            ..Host::default()
        };
        assert_eq!(game_entry(&mut g, &mut h, &mut lv, p, 0), Ok(true));
        // The act's start level, tile index 0.
        assert_eq!(lv.spawn_calls, [(0, 1, 0)]);
        assert_eq!(
            h.log,
            [
                "send 0 MapReveal { x: 100, y: 200, level: 7 }",
                "send 0 ReassignPlayer { unit: 0, x: 13, y: 8, flag: 1 }",
                "send 0 GameEntryDone",
            ]
        );
        // The room switch before the placement.
        assert_eq!(g.log, ["switch 0 r0", "add 0 r0 (13,8)"]);
        // Level warp: spawn of the given level and tile index, then a
        // placement whose second search finds the same point.
        let mut g = Grid::vec20();
        g.set(13, 8, 0x1);
        let p = unit(&mut g, Some(0), true);
        let mut h = Host {
            players: vec![p],
            ..Host::default()
        };
        lv.spawn_calls.clear();
        assert_eq!(
            level_warp_place(&mut g, &mut h, &mut lv, p, 0, 3, 5),
            Ok(true)
        );
        assert_eq!(lv.spawn_calls, [(0, 3, 5)]);
        let (_, _, x, y) = g.teleports[0];
        let mut q = Point::new(13, 8);
        free_point(&g, Some(0), &mut q, 2, mask::PLAYER_PLACE, false).unwrap();
        assert_eq!((x, y), (q.x, q.y));
        assert_ne!((x, y), (13, 8));
        assert_eq!(h.log[4], format!("point 0 ({x},{y})"));
    }
}
