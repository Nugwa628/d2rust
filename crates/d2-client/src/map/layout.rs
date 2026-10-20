//! Map layout: a DS1 map plus a tile library, turned into one ordered list of draw items.
//!
//! Every map cell sits on an isometric grid ([`cell_origin`]). The list is painted in
//! order, so its order is the painter's order:
//! 1. floors, layer by layer, each layer row by row;
//! 2. lower walls, then ordinary walls and corners, then roofs. Each of these three groups
//!    is sorted back to front by depth (see `Depth`).
//!
//! Tiles the map asks for but the library does not have are counted in
//! [`Layout::missing`] and skipped.

use std::collections::BTreeMap;

use d2_formats::ds1::Ds1;

use super::tiles::{TileKey, TileLibrary};

/// Width of one map cell on screen, in pixels.
pub const CELL_WIDTH: i32 = 160;
/// Height of one map cell on screen, in pixels.
pub const CELL_HEIGHT: i32 = 80;

/// Cell flag: the tile is not drawn (boundary and collision markers).
pub const HIDDEN: u32 = 0x8000_0000;

/// Tile orientation of floors.
const FLOOR: u32 = 0;
/// Orientation of a corner wall piece. A corner draws two tiles from its cell: itself,
/// then [`CORNER_HALF`].
const CORNER: u32 = 3;
/// The second half of a corner.
const CORNER_HALF: u32 = 4;
/// Orientation of roofs. Roofs are raised by their tile's roof height.
const ROOF: u32 = 15;

/// Lower walls (orientations 16-19): drawn before every other wall piece.
fn is_lower_wall(orientation: u32) -> bool {
    (16..=19).contains(&orientation)
}

/// Wall-layer orientations that never draw: no piece (0), the two special marker tiles
/// (10, 11) and shadows (13).
fn is_skipped(orientation: u32) -> bool {
    matches!(orientation, 0 | 10 | 11 | 13)
}

/// Which DS1 layer an item came from (for debugging and probes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A floor tile of this floor layer.
    Floor(usize),
    /// A piece of this wall layer.
    Wall(usize),
    /// A demo sprite added on top of the map ([`super::add_sprites`]).
    Sprite,
}

/// One tile image drawn at a screen position (top-left, y down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawItem {
    /// Index into the tile library's `images`.
    pub image: usize,
    pub x: i32,
    pub y: i32,
    /// The key the image was looked up with.
    pub key: TileKey,
    /// Map cell of the item; `(-1, -1)` for sprites.
    pub cell: (i32, i32),
    pub source: Source,
}

/// Screen-space bounding box `[x0, x1) × [y0, y1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

/// The ordered draw list of one map.
#[derive(Debug, Default)]
pub struct Layout {
    /// Back to front.
    pub items: Vec<DrawItem>,
    /// Keys the map uses that the library does not have, with how often each was asked for.
    pub missing: BTreeMap<TileKey, usize>,
    /// Bounding box of all items; `None` when nothing is drawn.
    pub bounds: Option<Bounds>,
}

/// Screen origin of cell `(x, y)`: one cell along x goes right and down, one cell along y
/// goes left and down.
pub fn cell_origin(x: i32, y: i32) -> (i32, i32) {
    ((x - y) * CELL_WIDTH / 2, (x + y) * CELL_HEIGHT / 2)
}

/// `(main, sub)` of a drawn cell, or `None` if the cell is empty (low byte zero) or
/// [`HIDDEN`]. Main is bits 20-25 of the cell value, sub bits 8-15.
pub fn cell_indices(cell: u32) -> Option<(u32, u32)> {
    if cell & 0xFF == 0 || cell & HIDDEN != 0 {
        return None;
    }
    Some(((cell >> 20) & 0x3F, (cell >> 8) & 0xFF))
}

/// Cell coordinates of entry `i` of a row-major layer `width` cells wide.
fn cell_at(i: usize, width: usize) -> (i32, i32) {
    ((i % width) as i32, (i / width) as i32)
}

/// Back-to-front sort key of a wall-layer piece: the diagonal `x + y` (further down the
/// screen is nearer), then `x`, then the wall layer, then the piece of a corner (its main
/// tile before its second half).
type Depth = (i32, i32, usize, u8);

/// A wall-layer piece before sorting.
struct Piece {
    depth: Depth,
    item: DrawItem,
}

/// Builds the draw list. `wall_base` is how far below a cell's origin its wall line is, in
/// pixels: wall pieces are moved down by it, roofs by it minus their roof height.
pub fn build(ds1: &Ds1, lib: &TileLibrary, wall_base: i32) -> Layout {
    let mut layout = Layout::default();
    let width = ds1.width as usize;
    // The item for `key` at cell (x, y). A key the library lacks is counted as missing; a
    // key whose tile is empty draws nothing.
    let place =
        |layout: &mut Layout, key: TileKey, x: i32, y: i32, source: Source| -> Option<DrawItem> {
            let Some(found) = lib.lookup(key) else {
                *layout.missing.entry(key).or_insert(0) += 1;
                return None;
            };
            let image = found?;
            let img = &lib.images[image];
            let (sx, sy) = cell_origin(x, y);
            let dy = match key.orientation {
                FLOOR => 0,
                ROOF => wall_base - img.roof_height,
                _ => wall_base,
            };
            Some(DrawItem {
                image,
                x: sx + img.x0,
                y: sy + img.y0 + dy,
                key,
                cell: (x, y),
                source,
            })
        };

    for (layer, floor) in ds1.floors.iter().enumerate() {
        for (i, &cell) in floor.iter().enumerate() {
            let Some((main, sub)) = cell_indices(cell) else {
                continue;
            };
            let (x, y) = cell_at(i, width);
            let key = TileKey {
                orientation: FLOOR,
                main,
                sub,
            };
            if let Some(item) = place(&mut layout, key, x, y, Source::Floor(layer)) {
                layout.items.push(item);
            }
        }
    }

    let (mut lower, mut walls, mut roofs) = (Vec::new(), Vec::new(), Vec::new());
    for (layer, (cells, orientations)) in ds1.walls.iter().zip(&ds1.orientations).enumerate() {
        for (i, (&cell, &orientation)) in cells.iter().zip(orientations).enumerate() {
            if is_skipped(orientation) {
                continue;
            }
            let Some((main, sub)) = cell_indices(cell) else {
                continue;
            };
            let (x, y) = cell_at(i, width);
            let group = if is_lower_wall(orientation) {
                &mut lower
            } else if orientation == ROOF {
                &mut roofs
            } else {
                &mut walls
            };
            let pieces: &[(u32, u8)] = if orientation == CORNER {
                &[(CORNER, 0), (CORNER_HALF, 1)]
            } else {
                &[(orientation, 0)]
            };
            for &(orientation, piece) in pieces {
                let key = TileKey {
                    orientation,
                    main,
                    sub,
                };
                if let Some(item) = place(&mut layout, key, x, y, Source::Wall(layer)) {
                    group.push(Piece {
                        depth: (x + y, x, layer, piece),
                        item,
                    });
                }
            }
        }
    }
    for mut group in [lower, walls, roofs] {
        // Stable: pieces of equal depth keep their cell order.
        group.sort_by_key(|p| p.depth);
        layout.items.extend(group.into_iter().map(|p| p.item));
    }

    layout.update_bounds(lib);
    layout
}

impl Layout {
    /// Items whose non-transparent pixels cover screen point `(x, y)`, topmost first, as
    /// (item index, item, palette index).
    pub fn probe(&self, lib: &TileLibrary, x: i32, y: i32) -> Vec<(usize, DrawItem, u8)> {
        let mut hits: Vec<(usize, DrawItem, u8)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                let img = &lib.images[it.image];
                let (px, py) = (x - it.x, y - it.y);
                if px < 0 || py < 0 || px >= img.width as i32 || py >= img.height as i32 {
                    return None;
                }
                let p = img.pixels[(py * img.width as i32 + px) as usize];
                (p != 0).then_some((i, *it, p))
            })
            .collect();
        hits.reverse();
        hits
    }

    /// Recomputes `bounds` from the items.
    pub fn update_bounds(&mut self, lib: &TileLibrary) {
        self.bounds = self
            .items
            .iter()
            .map(|it| {
                let img = &lib.images[it.image];
                Bounds {
                    x0: it.x,
                    y0: it.y,
                    x1: it.x + img.width as i32,
                    y1: it.y + img.height as i32,
                }
            })
            .reduce(|a, b| Bounds {
                x0: a.x0.min(b.x0),
                y0: a.y0.min(b.y0),
                x1: a.x1.max(b.x1),
                y1: a.y1.max(b.y1),
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::add_sprites;
    use d2_formats::dt1::{Dt1, Dt1Block, Dt1Tile};

    #[test]
    fn origins() {
        assert_eq!(cell_origin(0, 0), (0, 0));
        assert_eq!(cell_origin(1, 0), (80, 40));
        assert_eq!(cell_origin(0, 1), (-80, 40));
    }

    #[test]
    fn cell_fields() {
        assert_eq!(cell_indices(0x0050_00C2), Some((5, 0)));
        assert_eq!(cell_indices(0x0150_2A07), Some((0x15, 0x2A)));
        assert_eq!(cell_indices(0x0150_2A00), None, "prop1 = 0 is empty");
        // Real value from townN1.ds1 (river boundary): hidden, not drawn.
        assert_eq!(cell_indices(0x8050_0081), None, "hidden");
    }

    fn block(fill: u8) -> Dt1Block {
        Dt1Block {
            x: 0,
            y: 0,
            unknown1: 0,
            grid_x: 0,
            grid_y: 0,
            format: d2_formats::dt1::ISO_FORMAT,
            unknown2: 0,
            pixels: vec![fill; 32 * 15],
        }
    }

    fn tile(orientation: u32, sub: u32, roof_height: u16) -> Dt1Tile {
        Dt1Tile {
            light_direction: 0,
            roof_height,
            material_flags: 0,
            height: 0,
            width: 0,
            unknown_height: 0,
            orientation,
            main_index: 0,
            sub_index: sub,
            rarity: 0,
            unknown_color: 0,
            subtile_flags: [0; 25],
            unknown_58: 0,
            cache_index: 0,
            unknown_5c: 0,
            blocks: vec![block(orientation as u8 + 1)],
        }
    }

    /// 2×1 map: floors everywhere; wall layer 0 has a corner (3) at (1,0)
    /// and a roof at (0,0); wall layer 1 has a lower wall at (0,0).
    fn map() -> (Ds1, TileLibrary) {
        let cell = |sub: u32| 0x01 | (sub << 8);
        let ds1 = Ds1 {
            version: 18,
            width: 2,
            height: 1,
            act: 0,
            tag_type: 0,
            files: vec![],
            unknown_header: None,
            walls: vec![vec![cell(1), cell(1)], vec![cell(1), 0]],
            orientations: vec![vec![ROOF, 3], vec![16, 0]],
            floors: vec![vec![cell(1), cell(1)]],
            shadow: vec![0, 0],
            tags: None,
            objects: vec![],
            unknown_groups: None,
            groups: vec![],
            groups_truncated: false,
            paths: vec![],
            trailing: vec![],
        };
        let dt1 = Dt1 {
            version: 7,
            minor_version: 6,
            tiles: vec![
                tile(0, 1, 0),
                tile(3, 1, 0),
                tile(4, 1, 0),
                tile(ROOF, 1, 50),
                tile(16, 1, 0),
            ],
        };
        let mut lib = TileLibrary::new();
        lib.add_dt1(&dt1);
        (ds1, lib)
    }

    #[test]
    fn draw_order_and_positions() {
        let (ds1, lib) = map();
        let layout = build(&ds1, &lib, 80);
        let orient: Vec<u8> = layout
            .items
            .iter()
            .map(|it| lib.images[it.image].pixels[0] - 1)
            .collect();
        // floors, lower wall, corner (3 then 4), roof
        assert_eq!(orient, [0, 0, 16, 3, 4, 15]);
        let corner = &layout.items[3..5];
        assert_eq!((corner[0].x, corner[0].y), (80, 40 + 80));
        assert_eq!((corner[1].x, corner[1].y), (80, 40 + 80));
        let roof = layout.items[5];
        assert_eq!((roof.x, roof.y), (0, 80 - 50));
        assert!(layout.missing.is_empty());
        let b = layout.bounds.unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (0, 0, 112, 135));
    }

    /// 2×2 map with every wall-layer piece of `orientations` (per layer,
    /// row-major) and two floor layers: layer 0 full, layer 1 at (1,0) and
    /// (0,1).
    fn depth_map(orientations: Vec<Vec<u32>>) -> (Ds1, TileLibrary) {
        let cell = |sub: u32| 0x01 | (sub << 8);
        let walls = orientations
            .iter()
            .map(|l| {
                l.iter()
                    .map(|&o| if o == 0 { 0 } else { cell(1) })
                    .collect()
            })
            .collect();
        let ds1 = Ds1 {
            version: 18,
            width: 2,
            height: 2,
            act: 0,
            tag_type: 0,
            files: vec![],
            unknown_header: None,
            walls,
            orientations,
            floors: vec![vec![cell(1); 4], vec![0, cell(2), cell(2), 0]],
            shadow: vec![0; 4],
            tags: None,
            objects: vec![],
            unknown_groups: None,
            groups: vec![],
            groups_truncated: false,
            paths: vec![],
            trailing: vec![],
        };
        let dt1 = Dt1 {
            version: 7,
            minor_version: 6,
            tiles: [(0, 1), (0, 2), (1, 1), (2, 1), (3, 1), (4, 1)]
                .map(|(o, sub)| tile(o, sub, 0))
                .to_vec(),
        };
        let mut lib = TileLibrary::new();
        lib.add_dt1(&dt1);
        (ds1, lib)
    }

    #[test]
    fn floors_layer_by_layer_row_major() {
        let (ds1, lib) = depth_map(vec![vec![0; 4]]);
        let layout = build(&ds1, &lib, 80);
        let floors: Vec<_> = layout.items.iter().map(|it| (it.source, it.cell)).collect();
        assert_eq!(
            floors,
            [
                (Source::Floor(0), (0, 0)),
                (Source::Floor(0), (1, 0)),
                (Source::Floor(0), (0, 1)),
                (Source::Floor(0), (1, 1)),
                (Source::Floor(1), (1, 0)),
                (Source::Floor(1), (0, 1)),
            ]
        );
    }

    #[test]
    fn walls_sorted_by_depth_x_layer_piece() {
        // Layer 0: (0,0) wall 1, (1,0) corner 3, (0,1) wall 2, (1,1) wall 1.
        // Layer 1: (0,0) wall 2, (1,0) wall 1.
        let (ds1, lib) = depth_map(vec![vec![1, 3, 2, 1], vec![2, 1, 0, 0]]);
        let layout = build(&ds1, &lib, 80);
        let walls: Vec<_> = layout
            .items
            .iter()
            .filter(|it| matches!(it.source, Source::Wall(_)))
            .map(|it| (it.cell, it.source, it.key.orientation))
            .collect();
        assert_eq!(
            walls,
            [
                ((0, 0), Source::Wall(0), 1),
                ((0, 0), Source::Wall(1), 2),
                // x + y = 1: x 0 before x 1, although (1,0) comes first in
                // cell order.
                ((0, 1), Source::Wall(0), 2),
                // Same cell: layer 0's corner, main piece then its
                // orientation-4 half, before layer 1.
                ((1, 0), Source::Wall(0), 3),
                ((1, 0), Source::Wall(0), 4),
                ((1, 0), Source::Wall(1), 1),
                ((1, 1), Source::Wall(0), 1),
            ]
        );
    }

    #[test]
    fn sprites_go_on_top() {
        use d2_formats::dc6::{Dc6, Dc6Frame, Dc6Header};
        use d2_formats::dcc::{Dcc, DccDirection, DccFrame};
        let (ds1, mut lib) = map();
        let mut layout = build(&ds1, &lib, 80);
        let before = layout.items.len();
        let dcc = Dcc {
            version: 6,
            frames_per_direction: 1,
            tag: 1,
            final_dc6_size: 0,
            directions: vec![DccDirection {
                outsize_coded: 0,
                compression_flags: 0,
                x_min: -2,
                y_min: -3,
                width: 2,
                height: 3,
                frames: vec![DccFrame {
                    variable0: 0,
                    width: 2,
                    height: 3,
                    x_offset: -2,
                    y_offset: -1,
                    coded_bytes: 0,
                    bottom_up: false,
                    optional_data: vec![],
                    x_min: -2,
                    y_min: -3,
                    pixels: vec![9; 6],
                }],
                pcd_leftover_bits: 0,
            }],
        };
        let frame = |w: u32| Dc6Frame {
            flip: 0,
            width: w,
            height: 1,
            offset_x: 0,
            offset_y: 0,
            unknown: 0,
            next_block: 0,
            pixels: vec![7; w as usize],
        };
        let dc6 = Dc6 {
            header: Dc6Header {
                version: 6,
                flags: 0,
                encoding: 0,
                termination: [0; 4],
                directions: 1,
                frames_per_direction: 2,
            },
            frames: vec![frame(4), frame(5)],
        };
        add_sprites(&mut layout, &mut lib, &ds1, &dcc, &dc6);
        let sprites = &layout.items[before..];
        assert_eq!(sprites.len(), 3);
        // Center cell (1, 0) anchor = origin (80, 40) + (80, 40).
        assert_eq!((sprites[0].x, sprites[0].y), (160 - 2, 80 - 3));
        // DC6 frames in a row at the bounds' top-left + 32.
        let b = layout.bounds.unwrap();
        assert_eq!((sprites[1].x, sprites[1].y), (b.x0 + 32, b.y0 + 32));
        assert_eq!(sprites[2].x, b.x0 + 32 + 4);
        assert!(sprites.iter().all(|s| s.source == Source::Sprite));
    }

    #[test]
    fn missing_tiles_are_counted() {
        let (mut ds1, lib) = map();
        ds1.floors[0][1] = 0x01 | (9 << 8);
        let layout = build(&ds1, &lib, 80);
        let key = TileKey {
            orientation: 0,
            main: 0,
            sub: 9,
        };
        assert_eq!(layout.missing.get(&key), Some(&1));
    }
}
