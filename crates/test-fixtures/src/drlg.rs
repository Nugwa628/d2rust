//! The DRLG files of the synthetic install: every DS1 a lvlprest or
//! lvlsub row of [`crate::content`] names, every DT1 a lvltypes row
//! names, and the three fixed library files, written
//! by [`crate::ds1`] / [`crate::dt1`] and packed into `d2data.mpq` by
//! [`crate::install`]. Made-up content, no Blizzard file.
//!
//! The town (`PRESET_DS1[0]`, lvlprest Def 0, level 1) is one room:
//! a `PRESET_SIZE + 1` square v18 preset, every floor cell with the
//! floor bit (bit 1, 0x2) and key (0, 0), no walls, and
//! one preset object, the waypoint ([`TOWN_WAYPOINT`]). The keep, the
//! cave and the field are the same grid without the object; the substitution file is a
//! 3 × 3 tag-type-1 preset with one group.
//!
//! Tiles: each floor DT1 holds one floor tile of key (0, 0, 0), the wall
//! DT1 one wall tile (1, 0, 0), all rarity 1 and walkable (sub-tile
//! flags 0). The fixed library: `Blank.dt1` keys (0, 30, 0) and
//! (0, 30, 1), `InvisWal.dt1` no tile, `Warp.dt1` the (10, 0, 0) tile
//! the tile choice falls back to.

use d2_formats::ds1::{Ds1, Ds1Group, Ds1Object};

use crate::content::{FLOOR_DT1, PRESET_DS1, PRESET_SIZE, SUB_DS1, WALL_DT1};

/// `DATA\GLOBAL\TILES\`: a table tile path's archive
/// name is this prefix + the path.
pub const TILE_PREFIX: &str = "DATA\\GLOBAL\\TILES\\";

/// The fixed library, archive names as the sim asks.
pub const FIXED_LIBRARY: [&str; 3] = [
    "DATA\\GLOBAL\\Tiles\\Act1\\Outdoors\\Blank.dt1",
    "DATA\\GLOBAL\\Tiles\\Act1\\Barracks\\InvisWal.dt1",
    "DATA\\GLOBAL\\Tiles\\Act1\\Barracks\\Warp.dt1",
];

/// The floor cell: the floor bit (0x2), main 0, sub 0.
pub const FLOOR_CELL: u32 = 0x2;

/// DS1 object ids ≥ 150 give class `id − 150`, so
/// 150 is `objects` row 0, the synthetic waypoint (operate function 23).
pub const WAYPOINT_DS1_ID: u32 = 150;
/// The town waypoint record: (type 2 object, id, sub-tile x, y) in the
/// preset's sub-tile coordinates (5 per tile), the middle of tile (4, 4).
pub const TOWN_WAYPOINT: (u32, u32, u32, u32) = (2, WAYPOINT_DS1_ID, 22, 22);

/// The archive name of a table tile path.
pub fn archive_name(table_path: &str) -> String {
    format!("{TILE_PREFIX}{table_path}")
}

/// A preset of the lvlprest size with every floor cell set.
fn preset_room(act: u32) -> Ds1 {
    let side = PRESET_SIZE + 1;
    let mut d = crate::ds1::blank(side, side, act);
    d.files = FLOOR_DT1
        .iter()
        .chain([&WALL_DT1])
        .map(|f| f.as_bytes().to_vec())
        .collect();
    d.floors[0].fill(FLOOR_CELL);
    d
}

/// The one-room town.
pub fn town() -> Ds1 {
    let mut d = preset_room(0);
    let (kind, id, x, y) = TOWN_WAYPOINT;
    d.objects.push(Ds1Object {
        kind,
        id,
        x,
        y,
        flags: 0,
    });
    d
}

/// The keep (level 4): one room and a waypoint, the second act 0
/// waypoint level.
pub fn keep() -> Ds1 {
    let mut d = preset_room(0);
    let (kind, id, x, y) = TOWN_WAYPOINT;
    d.objects.push(Ds1Object {
        kind,
        id,
        x,
        y,
        flags: 0,
    });
    d
}

/// The lvlsub file: 3 × 3, tag type 1 (tag layer and groups), one
/// 2 × 2 group with one variant.
pub fn substitution() -> Ds1 {
    let mut d = crate::ds1::blank(3, 3, 0);
    d.tag_type = 1;
    d.tags = Some(vec![0; 9]);
    d.unknown_groups = Some([0; 4]);
    d.floors[0].fill(FLOOR_CELL);
    d.groups.push(Ds1Group {
        x: 0,
        y: 0,
        width: 2,
        height: 2,
        unknown: 1,
    });
    d
}

/// Every DRLG file of the install: (archive name, bytes).
pub fn files() -> Vec<(String, Vec<u8>)> {
    use crate::dt1::{file, tile, write};
    let mut out = vec![
        (archive_name(PRESET_DS1[0]), crate::ds1::write(&town())),
        (archive_name(PRESET_DS1[1]), crate::ds1::write(&keep())),
        (
            archive_name(PRESET_DS1[2]),
            crate::ds1::write(&preset_room(0)),
        ),
        (
            archive_name(PRESET_DS1[3]),
            crate::ds1::write(&preset_room(0)),
        ),
        (archive_name(SUB_DS1), crate::ds1::write(&substitution())),
    ];
    for f in FLOOR_DT1 {
        out.push((archive_name(f), write(&file(vec![tile(0, 0, 0, 1)]))));
    }
    out.push((archive_name(WALL_DT1), write(&file(vec![tile(1, 0, 0, 1)]))));
    let blank = |sub| tile(0, 30, sub, 1);
    out.push((
        FIXED_LIBRARY[0].to_owned(),
        write(&file(vec![blank(0), blank(1)])),
    ));
    out.push((FIXED_LIBRARY[1].to_owned(), write(&file(Vec::new()))));
    out.push((
        FIXED_LIBRARY[2].to_owned(),
        write(&file(vec![tile(10, 0, 0, 1)])),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::dt1::Dt1;

    #[test]
    fn every_file_parses() {
        let files = files();
        assert_eq!(files.len(), 5 + 3 + 3);
        for (name, bytes) in &files {
            if name.ends_with(".ds1") {
                Ds1::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            } else {
                Dt1::parse(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            }
        }
        let town = Ds1::parse(&files[0].1).unwrap();
        assert_eq!(town, super::town());
        assert_eq!((town.width, town.height), (9, 9));
        assert_eq!(town.objects.len(), 1);
        assert_eq!(Ds1::parse(&files[4].1).unwrap(), substitution());
    }
}
