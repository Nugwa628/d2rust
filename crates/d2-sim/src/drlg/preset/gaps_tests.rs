//! Gap tests: preset rules not yet claimed by other tests.

#[allow(unused_imports)]
use super::*;

use super::tests::{cell, ds1, preset_data, World};
use crate::drlg::NoLevelTypes;
use crate::rng::Seed;

#[test]
fn parser_keeps_version_and_stored_size() {
    let pd = preset_data();
    for (v, w, h) in [(18u32, 5u32, 3u32), (12, 8, 1), (14, 1, 7)] {
        let mut f = ds1(w, h);
        f.version = v;
        let p = Ds1File::from_input(&f, &pd).unwrap();
        // The stored values, not the cell counts (W + 1, H + 1).
        assert_eq!((p.version, p.width, p.height), (v, w, h));
        assert_eq!(p.stride(), w as usize + 1);
    }
}

/// A `w × h` DS1 with one warp marker (orientation 10, style 2, sub 0)
/// at tile (x, y).
fn marked(w: u32, h: u32, x: usize, y: usize) -> Ds1Input {
    let mut f = ds1(w, h);
    let i = y * (w as usize + 1) + x;
    f.orientations[0][i] = 10;
    f.walls[0][i] = cell(2, 0);
    f
}

#[test]
fn area_cell_grid_is_w8_plus_1_by_h8_plus_1() {
    // 20 × 12 tiles: 3 × 2 cells (a w/8 × h/8 grid would have no cell for
    // the 4-tile edge rooms). The marker at (17, 9) is cell (2, 1).
    let mut w = World::new();
    let l = w.level(20, 202, 20, 12);
    w.pd.defs[202].scan = 1;
    w.file(202, b"m.ds1", marked(20, 12, 17, 9));
    w.init(l);
    w.generate(l).unwrap();
    let origin = w.drlg.level(l).rect;
    let rooms = w.drlg.level_rooms(l);
    assert_eq!(rooms.len(), 6);
    let warp = 1 << (2 + 4);
    for &r in &rooms {
        let room = w.drlg.room(r);
        let corner = (room.rect.x - origin.x, room.rect.y - origin.y);
        assert_eq!(room.flags & warp != 0, corner == (16, 8), "{:?}", room.rect);
    }
    // Single-room mode: one cell is used, wherever the marker is.
    let m = w
        .run(|p, d, c| p.alloc_map(d, c, l, 202, TileRect::new(0, 0, 20, 12)))
        .unwrap();
    let r = w
        .run(|p, d, c| p.build_area(d, c, l, m, 0, true))
        .unwrap()
        .unwrap();
    assert_ne!(w.drlg.room(r).flags & warp, 0);
}

#[test]
fn outdoor_caller_overwrites_file_after_the_draw() {
    // An outdoor-style caller: the map draws roll(Files) on the level
    // seed, then the caller sets the picked file; the build loads that
    // file.
    let mut w = World::new();
    let l = w
        .drlg
        .get_or_alloc_level(&w.dd, &mut NoLevelTypes, 2)
        .unwrap();
    let def = 120;
    w.pd.defs[def].scan = 1;
    let names: [&[u8]; 3] = [b"a.ds1", b"b.ds1", b"c.ds1"];
    for (k, n) in names.iter().enumerate() {
        w.pd.defs[def].file[k] = n.to_vec();
        w.files.0.insert(n.to_vec(), ds1(8, 8));
    }
    w.pd.defs[def].files = 3;
    w.drlg.level_mut(l).seed = Seed::init_low(4242);
    let s0 = w.drlg.level(l).seed;
    let mut c = s0;
    let drawn = c.roll(3) as i32;
    let m = w
        .run(|p, d, ctx| p.alloc_map(d, ctx, l, def as u32, TileRect::new(16, 24, 8, 8)))
        .unwrap();
    assert_eq!(w.drlg.level(l).seed, c);
    assert_eq!(w.p.map(m).unwrap().picked_file, drawn);
    let forced = (drawn + 1) % 3;
    w.p.map_mut(m).unwrap().picked_file = forced;
    w.run(|p, d, ctx| p.build_area(d, ctx, l, m, 0, false))
        .unwrap();
    for (k, n) in names.iter().enumerate() {
        let want = i32::from(k as i32 == forced);
        assert_eq!(w.cache.refs(n), want, "{}", String::from_utf8_lossy(n));
    }
}
