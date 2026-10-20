//! Gap tests from the placement rules: the three wrappers of the nearest-free
//! search (max distance 50, step, fallback, field), the floor drop's item
//! mask and size, and the warp tile preset's inputs. Fakes from
//! `search::tests` and `warp`'s own view are reused where public.

use super::coords::Point;
use super::place::floor_drop;
use super::place_seams::{mask, LvlWarp, PlaceError, WarpTileView};
use super::search::tests::{sign_field, Grid};
use super::search::{free_point, free_point_field, free_point_step, FREE_MAX_DISTANCE};
use super::warp::warp_tile_preset;
use crate::drlg::TileRect;

/// A 120 × 120 room with every cell a wall (0x1) except `free`.
fn walled(free: &[(i32, i32)]) -> Grid {
    let mut g = Grid::with_rooms(&[TileRect::new(0, 0, 120, 120)]);
    for y in 0..120 {
        for x in 0..120 {
            if !free.contains(&(x, y)) {
                g.set(x, y, 0x1);
            }
        }
    }
    g
}

#[test]
fn wrapper_max_distance_fifty() {
    assert_eq!(FREE_MAX_DISTANCE, 50);
    // Ring r reaches Chebyshev distance r; with D = 50 and step 1 the
    // last ring searched is 49.
    let g = walled(&[(109, 60)]);
    let mut p = Point::new(60, 60);
    assert_eq!(
        free_point(&g, Some(0), &mut p, 1, mask::PLAYER_PLACE, false),
        Ok(Some(0))
    );
    assert_eq!(p, Point::new(109, 60));
    let g = walled(&[(110, 60)]);
    let mut p = Point::new(60, 60);
    assert_eq!(
        free_point(&g, Some(0), &mut p, 1, mask::PLAYER_PLACE, false),
        Ok(None)
    );
    assert_eq!(p, Point::new(60, 60));
    // The nearest-free search's fallback argument: the room at the unchanged point.
    assert_eq!(
        free_point(&g, Some(0), &mut p, 1, mask::PLAYER_PLACE, true),
        Ok(Some(0))
    );
    assert_eq!(p, Point::new(60, 60));
}

#[test]
fn step_wrapper_steps_and_never_falls_back() {
    // Step 2: ring 2 lies at distance 3 and its side columns are tested
    // every second row, so (62, 60) is never a candidate.
    let g = walled(&[(62, 60)]);
    let mut p = Point::new(60, 60);
    assert_eq!(
        free_point_step(&g, Some(0), &mut p, 1, mask::PLAYER_PLACE, 2),
        Ok(None)
    );
    assert_eq!(p, Point::new(60, 60));
    let mut p = Point::new(60, 60);
    assert_eq!(
        free_point_step(&g, Some(0), &mut p, 1, mask::PLAYER_PLACE, 1),
        Ok(Some(0))
    );
    assert_eq!(p, Point::new(62, 60));
    let g = walled(&[(63, 61)]);
    let mut p = Point::new(60, 60);
    assert_eq!(
        free_point_step(&g, Some(0), &mut p, 1, mask::PLAYER_PLACE, 2),
        Ok(Some(0))
    );
    assert_eq!(p, Point::new(63, 61));
}

#[test]
fn field_wrapper_needs_a_walk_back_to_the_origin() {
    // A wall column at x = 12; the search starts right of it.
    let mut g = Grid::vec20();
    for y in 0..20 {
        g.set(12, y, 0x1);
    }
    let start = Point::new(13, 10);
    // Without a field the free start is the answer.
    let mut p = start;
    assert_eq!(
        free_point(&g, Some(0), &mut p, 1, mask::ITEM_FLOOR, false),
        Ok(Some(0))
    );
    assert_eq!(p, start);
    // With the field (origin (10, 10), mask 0x801) every cell right of
    // the wall walks into it; ring 2's left column is nearest at (11, 10).
    let field = sign_field();
    let mut p = start;
    assert_eq!(
        free_point_field(
            &g,
            &field,
            Some(0),
            &mut p,
            Point::new(10, 10),
            1,
            mask::ITEM_FLOOR,
            mask::FIELD,
            false
        ),
        Ok(Some(0))
    );
    assert_eq!(p, Point::new(11, 10));
}

#[test]
fn floor_drop_is_one_cell_against_item_mask() {
    assert_eq!(mask::ITEM_FLOOR, 0x3E01);
    let drop = |g: &Grid| floor_drop(g, &sign_field(), Some(0), Point::new(10, 10), 1, true);
    // Bits outside 0x3E01 (here 0x8, NOPLAYER) never block the start
    // (12, 13); a wall next to it does not matter for size 1.
    let mut g = Grid::vec20();
    g.set(12, 13, 0x8);
    g.set(13, 13, 0x1);
    g.set(12, 14, 0x1);
    assert_eq!(drop(&g), Ok((Some(0), Point::new(12, 13))));
    // Each bit of 0x3E01 blocks it.
    for bit in [0x1, 0x200, 0x400, 0x800, 0x1000, 0x2000] {
        let mut g = Grid::vec20();
        g.set(12, 13, bit);
        assert_eq!(drop(&g), Ok((Some(0), Point::new(11, 13))), "bit {bit:#x}");
    }
}

#[derive(Default)]
struct Drlg {
    added: Vec<(u8, u32, u32, i32, i32)>,
}

impl WarpTileView for Drlg {
    type DrlgRoom = u8;
    fn tile_rect(&self, _room: u8) -> TileRect {
        TileRect {
            x: 40,
            y: 24,
            w: 8,
            h: 8,
        }
    }
    fn lvlwarp(&self, _room: u8, slot: u32, letter: u8) -> Option<LvlWarp> {
        // Records exist for slot 5 only.
        (slot == 5).then_some(LvlWarp {
            id: 30 + u32::from(letter == b'l'),
            offset_x: 1,
            offset_y: 2,
        })
    }
    fn add_preset_unit(&mut self, _room: u8, ty: u8, class: u32, mode: u32, x: i32, y: i32) {
        self.added.push((ty, class, mode, x, y));
    }
}

#[test]
fn warp_tile_inputs_type_tile_and_packed_value() {
    let mut d = Drlg::default();
    // Bits outside 20–25 of the packed value do not change the slot.
    let v = 0xFC00_0000 | (5 << 20) | 0x000F_FFFF;
    // Type 11 at world tile (41, 27) of a room at tile (40, 24).
    assert_eq!(warp_tile_preset(&mut d, 0, 11, 41, 27, v), Ok(true));
    // Type 10 at the room's first tile.
    assert_eq!(warp_tile_preset(&mut d, 0, 10, 40, 24, v), Ok(true));
    assert_eq!(d.added, [(5, 30, 0, 6, 17), (5, 31, 0, 1, 2)]);
    // Another slot has no record: fatal.
    assert_eq!(
        warp_tile_preset(&mut d, 0, 10, 40, 24, 6 << 20),
        Err(PlaceError::NoLvlWarp { slot: 6 })
    );
}
