//! The destination of a warp tile on the DRLG: the source room's warp
//! link of the tile's class, the destination room's link back (its
//! lvlwarp row gives the arrival tile's class and the walk-out), the
//! destination room made active, and its first tile unit of that class
//! (none: no destination). The 0x13 handler warps only within distance 5,
//! so the far room is populated. Called by the place adapters in
//! [`super::place`].

use crate::path::coords::Point;
use crate::path::place_seams::WarpDestination;
use crate::units::RoomId;
use crate::wiring::action::Pending;

use super::walk::PathCtx;

/// What the DRLG side of the lookup found.
struct Found {
    room: RoomId,
    class: u32,
    exit: (i32, i32),
    source_level: u32,
    level: u32,
}

/// The destination of the warp tile of `class` in `tile_room`: `None` when a
/// link or the arrival tile is missing.
pub(super) fn destination<X: Pending>(
    c: &mut PathCtx<'_, X>,
    tile_room: RoomId,
    class: u32,
) -> Option<WarpDestination<RoomId>> {
    let act = c.game.lists.room(tile_room)?.act;
    let found = c.v.h.drlg.with_act(act, &mut c.game.lists, |d, svc| {
        let s = d.drlg_room_of(tile_room)?;
        let warps = &svc.data.warps;
        let link = d.room(s).warp_links.iter().find(|l| {
            warps
                .get(l.lvlwarp_row)
                .is_some_and(|w| w.id as u32 == class)
        })?;
        let dest = link.target;
        let back = d.room(dest).warp_links.iter().find(|l| l.target == s)?;
        let rec = warps.get(back.lvlwarp_row)?;
        let exit = svc.data.warp_exits.get(back.lvlwarp_row).copied();
        let source_level = d.level(d.room(s).level).id;
        let level = d.level(d.room(dest).level).id;
        let room = d.stream_room(svc, dest).ok()??;
        Some(Found {
            room,
            class: rec.id as u32,
            exit: exit.unwrap_or((0, 0)),
            source_level,
            level,
        })
    })??;
    let tile = c.v.room_tile(c.game, found.room, found.class)?;
    let room = c.game.lists.unit(tile).and_then(|e| e.room())?;
    let (x, y) = c.v.h.path_position(tile);
    Some(WarpDestination {
        room,
        point: Point { x, y },
        exit_walk_x: found.exit.0,
        exit_walk_y: found.exit.1,
        source_level: found.source_level,
        level: found.level,
    })
}
