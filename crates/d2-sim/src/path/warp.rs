//! Warp tiles: the warp tile preset the tile grid fill
//! calls for a hidden exit cell (the `LevelTypes::warp_unit`
//! seam), and a player walking into a warp.

use super::place::place_unit;
use super::place_seams::{
    mask, CollisionView, LevelView, PlaceError, PlaceHost, PlaceMessage, WarpTileView,
};
use super::search::free_point;

/// Exit cell type of a left warp; the other type (11) is
/// 'r'.
pub const EXIT_LEFT: u32 = 10;
/// Preset unit type of a warp tile.
pub const TILE_UNIT_TYPE: u8 = 5;
/// Destination levels with a quest warp gate.
pub const GATED_LEVELS: [u32; 5] = [73, 100, 118, 128, 132];
/// Mode of the walk-out request.
pub const WALK_MODE: u32 = 2;

/// Warp slot of a packed exit cell value: bits 20–25.
pub fn warp_slot(v: u32) -> u32 {
    (v >> 20) & 0x3F
}

/// Warp tile preset for a hidden exit cell of type
/// `t` (10 or 11) at world tile (wx, wy) with packed value `v` in DRLG
/// room `room`. `Ok(true)`: a tile was added (result 1); `Ok(false)`: the
/// cell lies on the room's far edge.
pub fn warp_tile_preset<W: WarpTileView>(
    w: &mut W,
    room: W::DrlgRoom,
    t: u32,
    wx: i32,
    wy: i32,
    v: u32,
) -> Result<bool, PlaceError> {
    // Rule 1.
    let letter = if t == EXIT_LEFT { b'l' } else { b'r' };
    let slot = warp_slot(v);
    let rec = w
        .lvlwarp(room, slot, letter)
        .ok_or(PlaceError::NoLvlWarp { slot })?;
    // Rule 2 (edge case 9).
    let r = w.tile_rect(room);
    let (lx, ly) = (wx - r.x, wy - r.y);
    if lx == r.w || ly == r.h {
        return Ok(false);
    }
    // Rule 3. 32-bit two's complement arithmetic (the
    // lvlwarp values keep every result small).
    w.add_preset_unit(
        room,
        TILE_UNIT_TYPE,
        rec.id,
        0,
        lx.wrapping_mul(5).wrapping_add(rec.offset_x),
        ly.wrapping_mul(5).wrapping_add(rec.offset_y),
    );
    Ok(true)
}

/// What [`warp_player`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpOutcome {
    /// Placed at the destination and walking out.
    Arrived,
    /// Rule 1: no destination tile.
    NoDestination,
    /// Rule 2: no free point.
    NoFreePoint,
    /// Rule 3: the quest gate refused.
    QuestGate,
    /// Rule 4: the placement failed.
    NotPlaced,
}

/// Walking into a warp `(game, player, tile)` for the
/// tile of class `tile_class` in `tile_room`.
#[allow(clippy::too_many_arguments)]
pub fn warp_player<C, H, L>(
    cv: &mut C,
    host: &mut H,
    levels: &L,
    player: C::Unit,
    tile_room: C::Room,
    tile_class: u32,
) -> Result<WarpOutcome, PlaceError>
where
    C: CollisionView,
    H: PlaceHost<C::Unit>,
    L: LevelView<C::Room>,
{
    // Rule 1.
    let Some(dest) = levels.warp_destination(tile_room, tile_class) else {
        return Ok(WarpOutcome::NoDestination);
    };
    // Rule 2 (fallback 1, edge case 3).
    let mut p = dest.point;
    let size = cv.unit_size(player);
    if free_point(cv, Some(dest.room), &mut p, size, mask::PLAYER_PLACE, true)?.is_none() {
        return Ok(WarpOutcome::NoFreePoint);
    }
    // Rule 3.
    if GATED_LEVELS.contains(&dest.level) && levels.quest_gate(dest.source_level, dest.level) != 0 {
        return Ok(WarpOutcome::QuestGate);
    }
    // Rule 4.
    if !place_unit(
        cv,
        host,
        levels,
        player,
        Some(dest.room),
        p.x,
        p.y,
        false,
        false,
    )? {
        return Ok(WarpOutcome::NotPlaced);
    }
    // Rule 5: from rule 2's point, not the point placement may have
    // moved the player to; 32-bit two's complement adds.
    let (tx, ty) = (
        p.x.wrapping_add(dest.exit_walk_x),
        p.y.wrapping_add(dest.exit_walk_y),
    );
    host.request_walk(player, tx, ty);
    // Rule 6.
    let life_pct = host.life_percent(player);
    host.send(
        player,
        PlaceMessage::PlayerStop {
            unit: player,
            a: 1,
            x: tx as u16,
            y: ty as u16,
            b: 0,
            life_pct,
        },
    );
    Ok(WarpOutcome::Arrived)
}

#[cfg(test)]
mod tests {
    use super::super::coords::Point;
    use super::super::place::tests::{unit, Host};
    use super::super::place_seams::{LvlWarp, WarpDestination};
    use super::super::search::tests::Grid;
    use super::*;
    use crate::drlg::TileRect;

    #[derive(Default)]
    struct Drlg {
        rect: Option<TileRect>,
        recs: Vec<(u32, u8, LvlWarp)>,
        added: Vec<(u8, u32, u32, i32, i32)>,
    }

    impl WarpTileView for Drlg {
        type DrlgRoom = u8;
        fn tile_rect(&self, _room: u8) -> TileRect {
            self.rect.unwrap()
        }
        fn lvlwarp(&self, _room: u8, slot: u32, letter: u8) -> Option<LvlWarp> {
            self.recs
                .iter()
                .find(|(s, l, _)| *s == slot && *l == letter)
                .map(|r| r.2)
        }
        fn add_preset_unit(&mut self, _room: u8, ty: u8, class: u32, mode: u32, x: i32, y: i32) {
            self.added.push((ty, class, mode, x, y));
        }
    }

    #[test]
    fn warp_tile_presets() {
        let rec_l = LvlWarp {
            id: 17,
            offset_x: 2,
            offset_y: -1,
        };
        let rec_r = LvlWarp {
            id: 18,
            offset_x: 0,
            offset_y: 4,
        };
        let mut d = Drlg {
            rect: Some(TileRect {
                x: 100,
                y: 50,
                w: 8,
                h: 8,
            }),
            recs: vec![(3, b'l', rec_l), (3, b'r', rec_r)],
            ..Drlg::default()
        };
        let v = 3 << 20 | 0x1234;
        // Type 10 → 'l'.
        assert_eq!(warp_tile_preset(&mut d, 0, 10, 103, 52, v), Ok(true));
        assert_eq!(d.added, [(5, 17, 0, 5 * 3 + 2, 5 * 2 - 1)]);
        // Type 11 → 'r'.
        assert_eq!(warp_tile_preset(&mut d, 0, 11, 100, 50, v), Ok(true));
        assert_eq!(d.added[1], (5, 18, 0, 0, 4));
        // Far column or far row alone: nothing.
        assert_eq!(warp_tile_preset(&mut d, 0, 10, 108, 52, v), Ok(false));
        assert_eq!(warp_tile_preset(&mut d, 0, 10, 101, 58, v), Ok(false));
        assert_eq!(d.added.len(), 2);
        // No record: fatal (before the edge test).
        assert_eq!(
            warp_tile_preset(&mut d, 0, 10, 108, 52, 4 << 20),
            Err(PlaceError::NoLvlWarp { slot: 4 })
        );
        assert_eq!(warp_slot(0xFFFF_FFFF), 0x3F);
    }

    fn dest(level: u32) -> WarpDestination<usize> {
        WarpDestination {
            room: 0,
            point: Point::new(10, 10),
            exit_walk_x: 3,
            exit_walk_y: -2,
            source_level: 1,
            level,
        }
    }

    #[test]
    fn warp_arrival() {
        let mut g = Grid::vec20();
        g.set(10, 10, 0x1);
        let p = unit(&mut g, Some(0), true);
        let mut h = Host {
            players: vec![p],
            life: 87,
            ..Host::default()
        };
        let lv = Host {
            warp: Some(dest(2)),
            ..Host::default()
        };
        assert_eq!(
            warp_player(&mut g, &mut h, &lv, p, 0, 17),
            Ok(WarpOutcome::Arrived)
        );
        // Size 2 around a single wall: ring 1 plus cells touch it except the
        // corners; (9, 9) first.
        assert_eq!(g.teleports, [(p, 0, 9, 9)]);
        let n = h.log.len();
        assert_eq!(
            h.log[n - 2..],
            [
                "walk 0 (12,7)".to_string(),
                "send 0 PlayerStop { unit: 0, a: 1, x: 12, y: 7, b: 0, life_pct: 87 }".to_string(),
            ]
        );
        // No destination.
        let lv0 = Host::default();
        assert_eq!(
            warp_player(&mut g, &mut h, &lv0, p, 0, 17),
            Ok(WarpOutcome::NoDestination)
        );
    }

    #[test]
    fn warp_gate_and_fallback() {
        // Gate: only the five levels, only when the gate answers non-zero.
        let mut g = Grid::vec20();
        let p = unit(&mut g, Some(0), true);
        let mut h = Host {
            players: vec![p],
            ..Host::default()
        };
        let mut lv = Host {
            warp: Some(dest(73)),
            gate: 1,
            ..Host::default()
        };
        assert_eq!(
            warp_player(&mut g, &mut h, &lv, p, 0, 1),
            Ok(WarpOutcome::QuestGate)
        );
        assert!(g.teleports.is_empty());
        lv.warp = Some(dest(74));
        assert_eq!(
            warp_player(&mut g, &mut h, &lv, p, 0, 1),
            Ok(WarpOutcome::Arrived)
        );
        lv.warp = Some(dest(132));
        lv.gate = 0;
        assert_eq!(
            warp_player(&mut g, &mut h, &lv, p, 0, 1),
            Ok(WarpOutcome::Arrived)
        );
        // Fallback 1: everything blocked → the search returns the room at
        // the unchanged point, then the placement's own search (no
        // fallback) fails.
        let mut gw = Grid::vec20();
        for y in 0..20 {
            for x in 0..20 {
                gw.set(x, y, 0x1);
            }
        }
        let q = unit(&mut gw, Some(0), true);
        let lv = Host {
            warp: Some(dest(2)),
            ..Host::default()
        };
        assert_eq!(
            warp_player(&mut gw, &mut h, &lv, q, 0, 1),
            Ok(WarpOutcome::NotPlaced)
        );
        // Destination point outside every room: no room at the unchanged
        // point either → no free point.
        let lv = Host {
            warp: Some(WarpDestination {
                point: Point::new(-60, -60),
                ..dest(2)
            }),
            ..Host::default()
        };
        assert_eq!(
            warp_player(&mut gw, &mut h, &lv, q, 0, 1),
            Ok(WarpOutcome::NoFreePoint)
        );
    }
}
