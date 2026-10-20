//! The map preview's deliberate simplifications, one synthetic map each: a key always
//! draws the first matching tile whatever the rarities, pixels go through no lighting or
//! palette transform, and the demo sprites draw over walls whatever the depth.

use d2_formats::dc6::{Dc6, Dc6Header};
use d2_formats::dcc::{Dcc, DccDirection, DccFrame};
use d2_formats::ds1::Ds1;
use d2_formats::dt1::{Dt1, Dt1Block, Dt1Tile, ISO_FORMAT};
use d2_formats::palette::{Palette, Rgb};

use super::add_sprites;
use super::cpu::{render_indexed, to_rgba, View};
use super::layout::{build, cell_origin, Source};
use super::tiles::{TileKey, TileLibrary};

fn block(fill: u8) -> Dt1Block {
    Dt1Block {
        x: 0,
        y: 0,
        unknown1: 0,
        grid_x: 0,
        grid_y: 0,
        format: ISO_FORMAT,
        unknown2: 0,
        pixels: vec![fill; 32 * 15],
    }
}

fn tile(orientation: u32, sub: u32, rarity: u32, light_direction: u32, fill: u8) -> Dt1Tile {
    Dt1Tile {
        light_direction,
        roof_height: 0,
        material_flags: 0,
        height: 0,
        width: 0,
        unknown_height: 0,
        orientation,
        main_index: 0,
        sub_index: sub,
        rarity,
        unknown_color: 0,
        subtile_flags: [0; 25],
        unknown_58: 0,
        cache_index: 0,
        unknown_5c: 0,
        blocks: vec![block(fill)],
    }
}

fn dt1(tiles: Vec<Dt1Tile>) -> Dt1 {
    Dt1 {
        version: 7,
        minor_version: 6,
        tiles,
    }
}

/// A `w × h` map: one floor layer and one wall layer.
fn ds1(w: u32, h: u32, floors: Vec<u32>, walls: Vec<u32>, orientations: Vec<u32>) -> Ds1 {
    Ds1 {
        version: 18,
        width: w,
        height: h,
        act: 0,
        tag_type: 0,
        files: vec![],
        unknown_header: None,
        walls: vec![walls],
        orientations: vec![orientations],
        floors: vec![floors],
        shadow: vec![0; (w * h) as usize],
        tags: None,
        objects: vec![],
        unknown_groups: None,
        groups: vec![],
        groups_truncated: false,
        paths: vec![],
        trailing: vec![],
    }
}

fn cell(sub: u32) -> u32 {
    0x01 | (sub << 8)
}

#[test]
fn tile_variant_is_the_first_match_whatever_the_rarity() {
    let key = TileKey {
        orientation: 0,
        main: 0,
        sub: 1,
    };
    // Rarity 0 first, a heavily weighted variant second: the first wins.
    let mut lib = TileLibrary::new();
    lib.add_dt1(&dt1(vec![tile(0, 1, 0, 0, 7), tile(0, 1, 255, 0, 8)]));
    let i = lib.lookup(key).unwrap().unwrap();
    assert_eq!(lib.images[i].pixels[0], 7);
    // Across files: the earlier file's tile wins over any rarity later.
    let mut lib = TileLibrary::new();
    lib.add_dt1(&dt1(vec![tile(0, 1, 1, 0, 3)]));
    lib.add_dt1(&dt1(vec![tile(0, 1, 1000, 0, 4)]));
    let i = lib.lookup(key).unwrap().unwrap();
    assert_eq!(lib.images[i].pixels[0], 3);
    // Every cell with the key draws that same image.
    let map = ds1(2, 2, vec![cell(1); 4], vec![0; 4], vec![0; 4]);
    let layout = build(&map, &lib, 80);
    assert_eq!(layout.items.len(), 4);
    assert!(layout.items.iter().all(|it| it.image == i));
}

#[test]
fn no_lighting_or_palette_transform() {
    // Two floor tiles of one index, one with a light direction set, at
    // cells far apart: every drawn pixel is that index, presented as
    // palette[index] exactly.
    let mut lib = TileLibrary::new();
    lib.add_dt1(&dt1(vec![tile(0, 1, 0, 0, 42), tile(0, 2, 0, 3, 42)]));
    let mut floors = vec![0; 16];
    floors[0] = cell(1);
    floors[15] = cell(2);
    let map = ds1(4, 4, floors, vec![0; 16], vec![0; 16]);
    let layout = build(&map, &lib, 80);
    assert_eq!(layout.items.len(), 2);
    let b = layout.bounds.unwrap();
    let view = View {
        left: b.x0,
        top: b.y0,
        width: (b.x1 - b.x0) as u32,
        height: (b.y1 - b.y0) as u32,
    };
    let indexed = render_indexed(&layout, &lib, view);
    assert!(indexed.iter().all(|&p| p == 0 || p == 42));
    let at = |x: i32, y: i32| indexed[((y - b.y0) * view.width as i32 + x - b.x0) as usize];
    let (far_x, far_y) = cell_origin(3, 3);
    assert_eq!(at(0, 0), 42);
    assert_eq!(at(far_x, far_y), 42);
    let mut palette = Palette {
        colors: [Rgb::default(); 256],
    };
    palette.colors[42] = Rgb {
        r: 201,
        g: 3,
        b: 77,
    };
    let rgba = to_rgba(&indexed, &palette);
    for (p, c) in indexed.iter().zip(rgba.chunks(4)) {
        if *p == 42 {
            assert_eq!(c, [201, 3, 77, 255]);
        }
    }
}

#[test]
fn sprites_draw_over_walls_whatever_the_depth() {
    // 3×3 map, walls (orientation 1) on every cell, so some walls stand
    // in front of (larger x + y than) the bonfire's center cell (1, 1).
    // The wall block stands 85 rows above the wall line (y = −85), so the
    // wall of cell (2, 1) covers the bonfire's anchor pixel.
    let mut wall = tile(1, 1, 0, 0, 5);
    wall.blocks[0].y = -85;
    let mut lib = TileLibrary::new();
    lib.add_dt1(&dt1(vec![tile(0, 1, 0, 0, 2), wall]));
    let map = ds1(3, 3, vec![cell(1); 9], vec![cell(1); 9], vec![1; 9]);
    let mut layout = build(&map, &lib, 80);
    let walls = layout
        .items
        .iter()
        .filter(|it| matches!(it.source, Source::Wall(_)))
        .count();
    assert_eq!(walls, 9);
    assert!(layout.items.iter().any(|it| it.cell.0 + it.cell.1 > 2));

    let dcc = Dcc {
        version: 6,
        frames_per_direction: 1,
        tag: 1,
        final_dc6_size: 0,
        directions: vec![DccDirection {
            outsize_coded: 0,
            compression_flags: 0,
            x_min: 0,
            y_min: 0,
            width: 4,
            height: 4,
            frames: vec![DccFrame {
                variable0: 0,
                width: 4,
                height: 4,
                x_offset: 0,
                y_offset: 3,
                coded_bytes: 0,
                bottom_up: false,
                optional_data: vec![],
                x_min: 0,
                y_min: 0,
                pixels: vec![9; 16],
            }],
            pcd_leftover_bits: 0,
        }],
    };
    let dc6 = Dc6 {
        header: Dc6Header {
            version: 6,
            flags: 0,
            encoding: 0,
            termination: [0; 4],
            directions: 1,
            frames_per_direction: 0,
        },
        frames: vec![],
    };
    add_sprites(&mut layout, &mut lib, &map, &dcc, &dc6);
    // The sprite is the last item: no wall-versus-unit depth rule.
    let last = *layout.items.last().unwrap();
    assert_eq!(last.source, Source::Sprite);
    let (sx, sy) = cell_origin(1, 1);
    assert_eq!((last.x, last.y), (sx + 80, sy + 40));
    // A wall in front of the sprite's cell covers its pixel, yet the
    // sprite is on top.
    let hits = layout.probe(&lib, last.x, last.y);
    assert!(hits.len() > 1, "{hits:?}");
    assert_eq!(hits[0].1.source, Source::Sprite);
    assert!(hits[1..]
        .iter()
        .any(|h| matches!(h.1.source, Source::Wall(_)) && h.1.cell.0 + h.1.cell.1 > 2));
}
