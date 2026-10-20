//! Unit tests of [`super::draws`] on synthetic light maps.

use super::draws::{
    self, floor_light_grid, roof_light_grid, tile_origin_sub_tile, unit_light, wall_block_corners,
    wall_light_words, DrawLightError, LightGrid, PointTable, WallPass, WallPoints,
};
use super::map::{LightCell, LightMap};
use crate::rules::shading::{floor_block_light, BlockLight};

/// A map around a player at sub-tile (100, 100) (origin 76) whose cell
/// `(gx, gy)` has `I = gx + 2·gy`, `R = gx`, `G = gy`, `B = 7`.
fn ramp_map() -> LightMap {
    let mut m = LightMap::new((100, 100));
    for gy in 0..48 {
        for gx in 0..48 {
            let c = m.cell_mut(gx, gy).unwrap();
            *c = LightCell {
                blocks: 0,
                i: (gx + 2 * gy) as u8,
                r: gx as u8,
                g: gy as u8,
                b: 7,
            };
        }
    }
    m
}

fn i_at(gx: i32, gy: i32) -> u8 {
    (gx.clamp(0, 47) + 2 * gy.clamp(0, 47)) as u8
}

#[test]
fn unit_light_reads_its_sub_tile_and_clamps() {
    let m = ramp_map();
    // Sub-tile (103, 105) = cell (27, 29).
    let w = unit_light(&m, (103, 105));
    assert_eq!(w, 7 << 24 | 29 << 16 | 27 << 8 | u32::from(i_at(27, 29)));
    assert_eq!(draws::light_byte(w), i_at(27, 29));
    // Outside the window: clamped to 0 / 47 on each axis.
    assert_eq!(unit_light(&m, (0, 500)), m.cell(0, 47).unwrap().word());
    assert_eq!(unit_light(&m, (500, -3)), m.cell(47, 0).unwrap().word());
}

#[test]
fn wall_points_tsv_has_108_rows_as_written() {
    let p = draws::wall_points();
    assert_eq!(p.rows().len(), 108);
    let text: Vec<&str> = draws::WALL_LIGHT_POINTS_TSV.lines().skip(1).collect();
    assert_eq!(text.len(), 108);
    for &k in &[0usize, 7, 50, 107] {
        let r = p.rows()[k];
        let t = match r.table {
            PointTable::Normal => "normal",
            PointTable::Faded => "faded",
        };
        let line = format!("{}\t{}\t{}\t{}\t{}", r.direction, t, r.point, r.dx, r.dy);
        assert_eq!(line, text[k]);
    }
    // Spot rows from the file.
    assert_eq!(
        p.offsets(1, PointTable::Normal).unwrap(),
        [(2, 5), (2, 3), (2, 1), (2, -1), (2, -3), (2, -5)]
    );
    assert_eq!(p.offsets(9, PointTable::Faded).unwrap()[5], (-2, -5));
}

#[test]
fn wall_points_parser_is_strict() {
    let good = draws::WALL_LIGHT_POINTS_TSV;
    assert!(WallPoints::parse(good).is_ok());
    let bad_row = good.replacen("1\tnormal\t0\t2\t5", "1\tnormal\t0\t2", 1);
    assert!(matches!(
        WallPoints::parse(&bad_row),
        Err(DrawLightError::Tsv { line: 2, .. })
    ));
    let bad_table = good.replacen("1\tnormal\t0\t2\t5", "1\tdim\t0\t2\t5", 1);
    assert!(matches!(
        WallPoints::parse(&bad_table),
        Err(DrawLightError::Tsv { line: 2, .. })
    ));
    let bad_int = good.replacen("1\tnormal\t0\t2\t5", "1\tnormal\t0\tx\t5", 1);
    assert!(WallPoints::parse(&bad_int).is_err());
    let bad_dir = good.replacen("1\tnormal\t0\t2\t5", "10\tnormal\t0\t2\t5", 1);
    assert!(WallPoints::parse(&bad_dir).is_err());
    let dup = good.replacen("1\tnormal\t1\t2\t3", "1\tnormal\t0\t2\t3", 1);
    assert!(WallPoints::parse(&dup).is_err());
    let short: String = good.lines().take(100).collect::<Vec<_>>().join("\n");
    assert!(WallPoints::parse(&short).is_err());
    assert!(WallPoints::parse("dir\ttable\tpoint\tdx\tdy\n").is_err());
}

#[test]
fn wall_direction_0_reuses_the_pass_words() {
    let m = ramp_map();
    assert_eq!(
        wall_light_words(&m, (100, 100), 0, 0),
        Err(DrawLightError::WallDirection0)
    );
    // First wall of a pass: no earlier words to reuse, an error.
    let mut pass = WallPass::default();
    assert_eq!(
        pass.words(&m, (100, 100), 0, 0),
        Err(DrawLightError::WallDirection0)
    );
    // After a direction-1 record, direction 0 keeps its words wherever it
    // is.
    let first = pass.words(&m, (100, 100), 1, 0).unwrap();
    assert_eq!(first, wall_light_words(&m, (100, 100), 1, 0).unwrap());
    assert_eq!(pass.words(&m, (120, 90), 0, 1), Ok(first));
    let second = pass.words(&m, (120, 90), 2, 0).unwrap();
    assert_ne!(second, first);
    assert_eq!(pass.words(&m, (100, 100), 0, 0), Ok(second));
    assert_eq!(
        wall_light_words(&m, (100, 100), 10, 0),
        Err(DrawLightError::WallDirection(10))
    );
}

#[test]
fn wall_words_and_block_corners_from_a_synthetic_map() {
    let m = ramp_map();
    // Room sub-tile origin (90, 95), record tile (2, 1) → origin (100, 100)
    // = cell (24, 24).
    let origin = tile_origin_sub_tile((90, 95), (2, 1));
    assert_eq!(origin, (100, 100));
    let words = wall_light_words(&m, origin, 1, 0).unwrap();
    let pts = draws::wall_points().offsets(1, PointTable::Normal).unwrap();
    for (w, (dx, dy)) in words.iter().zip(pts) {
        assert_eq!(*w, m.cell(24 + dx, 24 + dy).unwrap().word());
    }
    // Fade bit 0 selects the faded table; other bits do not.
    let faded = draws::wall_points().offsets(1, PointTable::Faded).unwrap();
    let wf = wall_light_words(&m, origin, 1, 0b11).unwrap();
    for (w, (dx, dy)) in wf.iter().zip(faded) {
        assert_eq!(*w, m.cell(24 + dx, 24 + dy).unwrap().word());
    }
    assert_eq!(wall_light_words(&m, origin, 1, 0b10).unwrap(), words);

    // Corners per 32-pixel block column.
    let syn = [0x0A0B_0C10, 0x20, 0x30, 0x0000_FF40, 0x50, 0x60];
    assert_eq!(
        wall_block_corners(&syn, 0).unwrap(),
        [0x10, 0x20, 0x20, 0x10]
    );
    assert_eq!(
        wall_block_corners(&syn, 31).unwrap(),
        [0x10, 0x20, 0x20, 0x10]
    );
    assert_eq!(
        wall_block_corners(&syn, 64).unwrap(),
        [0x30, 0x40, 0x40, 0x30]
    );
    assert_eq!(
        wall_block_corners(&syn, 128).unwrap(),
        [0x50, 0x60, 0x60, 0x50]
    );
    assert_eq!(
        wall_block_corners(&syn, 160),
        Err(DrawLightError::BlockColumn(160))
    );
    assert_eq!(
        wall_block_corners(&syn, -1),
        Err(DrawLightError::BlockColumn(-1))
    );
}

#[test]
fn floor_grid_layout_and_material_flag() {
    let m = ramp_map();
    // Tile origin sub-tile (100, 100): cell (i, j) = sub-tile (99 + i,
    // 99 + j) = map cell (23 + i, 23 + j).
    let g = floor_light_grid(&m, (100, 100), 0);
    for j in 0..8 {
        for i in 0..8 {
            let (gx, gy) = (23 + i as i32, 23 + j as i32);
            assert_eq!(g.cell(i, j), [i_at(gx, gy), gx as u8, gy as u8, 7]);
            assert_eq!(g.cells[j * 8 + i], g.cell(i, j));
        }
    }
    let values = g.light_values();
    assert_eq!(values[9], i_at(24, 24));
    // The grid feeds the floor block light (`e[g+9]`
    // is the block's own sub-tile).
    let flat = LightGrid::filled([0x80, 1, 2, 3]);
    assert_eq!(
        floor_block_light(&flat.light_values(), 0, 0, false),
        Ok(BlockLight::Flat(16))
    );
    assert!(floor_block_light(&values, 0, 0, false).is_ok());
    // Near the map edge reads clamp.
    let e = floor_light_grid(&m, (76, 76), 0);
    assert_eq!(e.cell(0, 0)[0], i_at(0, 0));
    assert_eq!(e.cell(1, 1)[0], i_at(0, 0));
    // Material flag 0x100: every byte 0xFF; other bits change nothing.
    assert_eq!(
        floor_light_grid(&m, (100, 100), 0x100 | 0x3),
        LightGrid::filled([0xFF; 4])
    );
    assert_eq!(floor_light_grid(&m, (100, 100), 0x0FF), g);
}

#[test]
fn roof_grid_height_rule() {
    let m = ramp_map();
    let env = LightCell {
        blocks: 0,
        i: 200,
        r: 10,
        g: 20,
        b: 30,
    };
    // Draw entry in 1/8 sub-tile: (803, 807) >> 3 = sub-tile (100, 100).
    let g = roof_light_grid(&m, (803, 807), 0, env);
    assert_eq!(g, floor_light_grid(&m, (100, 100), 0));
    assert_eq!(
        roof_light_grid(&m, (803, 807), 1, env),
        LightGrid::filled([200, 10, 20, 30])
    );
}

fn tables() -> crate::rules::shading::ShadeTables {
    use crate::scene::MapId;
    crate::rules::shading::ShadeTables {
        light0: MapId(0),
        highlight: MapId(32),
        red: MapId(33),
        zero: MapId(34),
        remap0: MapId(35),
        alpha: [MapId(200), MapId(456), MapId(712)],
        additive: MapId(968),
        multiplicative: MapId(1224),
        max_component: MapId(1480),
    }
}

#[test]
fn wall_block_shades_use_column_corners() {
    use crate::rules::blend::wall_block_ops;
    use crate::rules::view::BlockRect;
    let t = tables();
    let words = [200, 200, 0, 255, 100, 100];
    let rect = |x| BlockRect {
        x,
        y: 0,
        width: 32,
        height: 32,
    };
    let blocks = [rect(0), rect(32), rect(128)];
    let lit = draws::wall_block_shades(&t, &words, &blocks, 0xFF, false).unwrap();
    assert_eq!(lit.len(), 3);
    for (s, b) in lit.iter().zip(&blocks) {
        let c = wall_block_corners(&words, b.x).unwrap();
        let (shade, blend) = wall_block_ops(&t, 0xFF, c, false, 0, 0).unwrap();
        assert_eq!((s.block, s.shade, s.blend), (*b, shade, blend));
    }
    // Alpha below 0x40 hides every block; a block past column 4 errors.
    assert!(draws::wall_block_shades(&t, &words, &blocks, 0x10, false)
        .unwrap()
        .is_empty());
    assert_eq!(
        draws::wall_block_shades(&t, &words, &[rect(160)], 0xFF, false),
        Err(DrawLightError::BlockColumn(160))
    );
}
