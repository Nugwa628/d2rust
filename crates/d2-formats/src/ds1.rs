//! DS1 map presets: layered tile grids, objects, groups and NPC paths.
//!
//! [`Ds1::parse`] takes the bytes of a `.ds1` file (a map preset from the player's
//! game data) and returns its layers, objects, groups and paths. D2RUST's map
//! builder combines these with DT1 tile libraries to assemble levels.

use crate::cursor::{invalid, Cursor, FormatError};

const FORMAT: &str = "ds1";
const MAX_WALLS: u32 = 4;
const MAX_FLOORS: u32 = 2;
/// Implementation limit; group fields past the end of the file read as 0,
/// so the count isn't bounded by the file size.
const MAX_GROUPS: u32 = 0x1_0000;

const ORIENTATION_LOOKUP: [u32; 25] = [
    0x00, 0x01, 0x02, 0x01, 0x02, 0x03, 0x03, 0x05, 0x05, 0x06, 0x06, 0x07, 0x07, 0x08, 0x09, 0x0A,
    0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x14,
];

/// One object placed in the map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds1Object {
    /// Object kind word.
    pub kind: u32,
    /// Object id within its kind.
    pub id: u32,
    /// Tile x position.
    pub x: u32,
    /// Tile y position.
    pub y: u32,
    /// Object flags word.
    pub flags: u32,
}

/// A group rectangle from the group section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds1Group {
    /// Left tile of the group.
    pub x: u32,
    /// Top tile of the group.
    pub y: u32,
    /// Group width in tiles.
    pub width: u32,
    /// Group height in tiles.
    pub height: u32,
    /// Group field kept as read; present in version 13 and later, otherwise 0.
    pub unknown: u32,
}

/// One point of an NPC path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds1PathPoint {
    /// Tile x position of the point.
    pub x: u32,
    /// Tile y position of the point.
    pub y: u32,
    /// 1 when the file has no action field (version < 15).
    pub action: u32,
}

/// An NPC path: the NPC's position and its points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds1Path {
    /// Position of the NPC object the path belongs to.
    pub x: u32,
    /// Tile y position of the NPC object.
    pub y: u32,
    /// The path's points, in order.
    pub points: Vec<Ds1PathPoint>,
}

/// A `width × height` grid of raw u32 cells, row-major.
pub type Layer = Vec<u32>;

/// A parsed DS1 map preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ds1 {
    /// File version, 1 to 18.
    pub version: u32,
    /// Map width in tiles.
    pub width: u32,
    /// Map height in tiles.
    pub height: u32,
    /// Act number; 0 for versions before 8.
    pub act: u32,
    /// Tag type word; 0 for versions before 10.
    pub tag_type: u32,
    /// Names of the files the map refers to, as raw bytes without the NUL.
    pub files: Vec<Vec<u8>>,
    /// Eight header bytes kept as read; present in versions 9 to 13.
    pub unknown_header: Option<[u8; 8]>,
    /// Wall layers.
    pub walls: Vec<Layer>,
    /// Orientation for each wall layer (already mapped for version < 7).
    pub orientations: Vec<Layer>,
    /// Floor layers.
    pub floors: Vec<Layer>,
    /// Shadow layer.
    pub shadow: Layer,
    /// Tag layer, when the file has one.
    pub tags: Option<Layer>,
    /// Objects placed in the map.
    pub objects: Vec<Ds1Object>,
    /// Four bytes ahead of the group records, kept as read; present in version 18.
    pub unknown_groups: Option<[u8; 4]>,
    /// Group rectangles.
    pub groups: Vec<Ds1Group>,
    /// True if the file ended inside the group records (missing fields are 0).
    pub groups_truncated: bool,
    /// NPC paths.
    pub paths: Vec<Ds1Path>,
    /// Bytes left in the file after the last section.
    pub trailing: Vec<u8>,
}

/// Cell field helpers (bit layout of a raw layer cell).
pub mod cell {
    /// Sub index (bits 8..16) of a cell.
    pub fn sub_index(cell: u32) -> u32 {
        (cell >> 8) & 0xFF
    }
    /// Main index (bits 20..26) of a cell.
    pub fn main_index(cell: u32) -> u32 {
        (cell >> 20) & 0x3F
    }
    /// True if the cell's hidden bit (bit 31) is set.
    pub fn hidden(cell: u32) -> bool {
        cell & 0x8000_0000 != 0
    }
}

/// Reads `count` records of `record_len` bytes each, checking first that
/// they can fit in what remains of the file.
fn check_count(
    c: &Cursor<'_>,
    data: &[u8],
    count: u32,
    record_len: usize,
    what: &str,
) -> Result<usize, FormatError> {
    let remaining = data.len() - c.pos();
    let count = count as usize;
    if count.saturating_mul(record_len) > remaining {
        return Err(invalid(
            FORMAT,
            format!("{count} {what} cannot fit in the file"),
        ));
    }
    Ok(count)
}

impl Ds1 {
    /// Parses a DS1 file. Fails on an unsupported version, counts that cannot fit in
    /// the file, or truncated data.
    pub fn parse(data: &[u8]) -> Result<Ds1, FormatError> {
        let mut c = Cursor::new(data, FORMAT);
        let v = c.u32()?;
        if !(1..=18).contains(&v) {
            return Err(invalid(FORMAT, format!("version {v}")));
        }
        let width = c.u32()?.wrapping_add(1);
        let height = c.u32()?.wrapping_add(1);
        let act = if v >= 8 { c.u32()? } else { 0 };
        let tag_type = if v >= 10 { c.u32()? } else { 0 };

        let mut files = Vec::new();
        if v >= 3 {
            let n = c.u32()?;
            let n = check_count(&c, data, n, 1, "file names")?;
            for _ in 0..n {
                let rest = &data[c.pos()..];
                let len = rest
                    .iter()
                    .position(|&b| b == 0)
                    .ok_or_else(|| invalid(FORMAT, "unterminated file name"))?;
                files.push(c.bytes(len)?.to_vec());
                c.u8()?;
            }
        }
        let unknown_header = if (9..=13).contains(&v) {
            let mut u = [0u8; 8];
            u.copy_from_slice(c.bytes(8)?);
            Some(u)
        } else {
            None
        };
        let (wall_count, floor_count) = if v >= 4 {
            let w = c.u32()?;
            let f = if v >= 16 { c.u32()? } else { 1 };
            (w, f)
        } else {
            (1, 1)
        };
        if wall_count > MAX_WALLS || floor_count > MAX_FLOORS {
            return Err(invalid(
                FORMAT,
                format!("{wall_count} walls / {floor_count} floors"),
            ));
        }
        let has_tags = v < 4 || tag_type == 1 || tag_type == 2;

        let cells = u64::from(width) * u64::from(height);
        let layer_count = 2 * wall_count as u64 + floor_count as u64 + 1 + u64::from(has_tags);
        let grid_bytes = cells.checked_mul(4 * layer_count);
        if grid_bytes.is_none_or(|n| n > (data.len() - c.pos()) as u64) {
            return Err(invalid(
                FORMAT,
                format!("{width}x{height} grid with {layer_count} layers cannot fit"),
            ));
        }
        let cells = cells as usize;
        let read_layer = |c: &mut Cursor<'_>| -> Result<Layer, FormatError> {
            (0..cells).map(|_| c.u32()).collect()
        };
        let map_orientations = |layer: Layer| -> Result<Layer, FormatError> {
            if v >= 7 {
                return Ok(layer);
            }
            layer
                .into_iter()
                .map(|o| {
                    ORIENTATION_LOOKUP
                        .get(o as usize)
                        .copied()
                        .ok_or_else(|| invalid(FORMAT, format!("orientation {o}")))
                })
                .collect()
        };

        let mut walls = Vec::new();
        let mut orientations = Vec::new();
        let mut floors = Vec::new();
        let shadow;
        let mut tags = None;
        if v < 4 {
            walls.push(read_layer(&mut c)?);
            floors.push(read_layer(&mut c)?);
            orientations.push(map_orientations(read_layer(&mut c)?)?);
            tags = Some(read_layer(&mut c)?);
            shadow = read_layer(&mut c)?;
        } else {
            for _ in 0..wall_count {
                walls.push(read_layer(&mut c)?);
                orientations.push(map_orientations(read_layer(&mut c)?)?);
            }
            for _ in 0..floor_count {
                floors.push(read_layer(&mut c)?);
            }
            shadow = read_layer(&mut c)?;
            if has_tags {
                tags = Some(read_layer(&mut c)?);
            }
        }

        let mut objects = Vec::new();
        if v >= 2 {
            let n = c.u32()?;
            let record = if v >= 6 { 20 } else { 16 };
            let n = check_count(&c, data, n, record, "objects")?;
            for _ in 0..n {
                objects.push(Ds1Object {
                    kind: c.u32()?,
                    id: c.u32()?,
                    x: c.u32()?,
                    y: c.u32()?,
                    flags: if v >= 6 { c.u32()? } else { 0 },
                });
            }
        }

        let mut unknown_groups = None;
        let mut groups = Vec::new();
        let mut groups_truncated = false;
        if v >= 12 && (tag_type == 1 || tag_type == 2) {
            if v >= 18 {
                let mut u = [0u8; 4];
                u.copy_from_slice(c.bytes(4)?);
                unknown_groups = Some(u);
            }
            // Group records may be cut off by the end of the file; fields
            // that are missing read as 0.
            let n = c.u32()?;
            if n > MAX_GROUPS {
                return Err(invalid(FORMAT, format!("{n} groups")));
            }
            let mut field = |c: &mut Cursor<'_>| -> Result<u32, FormatError> {
                if data.len() - c.pos() >= 4 {
                    c.u32()
                } else {
                    groups_truncated = true;
                    Ok(0)
                }
            };
            for _ in 0..n {
                let x = field(&mut c)?;
                let y = field(&mut c)?;
                let width = field(&mut c)?;
                let height = field(&mut c)?;
                let unknown = if v >= 13 { field(&mut c)? } else { 0 };
                groups.push(Ds1Group {
                    x,
                    y,
                    width,
                    height,
                    unknown,
                });
            }
        }

        let mut paths = Vec::new();
        if v >= 14 && c.pos() < data.len() {
            let n = c.u32()?;
            let n = check_count(&c, data, n, 12, "paths")?;
            for _ in 0..n {
                let points = c.u32()?;
                let x = c.u32()?;
                let y = c.u32()?;
                let record = if v >= 15 { 12 } else { 8 };
                let points = check_count(&c, data, points, record, "path points")?;
                let mut pts = Vec::with_capacity(points);
                for _ in 0..points {
                    pts.push(Ds1PathPoint {
                        x: c.u32()?,
                        y: c.u32()?,
                        action: if v >= 15 { c.u32()? } else { 1 },
                    });
                }
                paths.push(Ds1Path { x, y, points: pts });
            }
        }

        let trailing = data[c.pos()..].to_vec();
        Ok(Ds1 {
            version: v,
            width,
            height,
            act,
            tag_type,
            files,
            unknown_header,
            walls,
            orientations,
            floors,
            shadow,
            tags,
            objects,
            unknown_groups,
            groups,
            groups_truncated,
            paths,
            trailing,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a DS1 of version `v` with a 2×2 grid; cell values are
    /// `layer_index * 100 + cell_index`.
    fn build(v: u32, tag_type: u32, walls: u32, floors: u32, orientation: u32) -> Vec<u8> {
        let mut d = Vec::new();
        let mut put = |x: u32| d.extend_from_slice(&x.to_le_bytes());
        put(v);
        put(1); // width - 1
        put(1); // height - 1
        if v >= 8 {
            put(2);
        }
        if v >= 10 {
            put(tag_type);
        }
        let mut d2 = d;
        if v >= 3 {
            d2.extend_from_slice(&1u32.to_le_bytes());
            d2.extend_from_slice(b"tiles\\a.dt1\0");
        }
        if (9..=13).contains(&v) {
            d2.extend_from_slice(&[9; 8]);
        }
        let mut d = d2;
        let mut put = |x: u32| d.extend_from_slice(&x.to_le_bytes());
        if v >= 4 {
            put(walls);
            if v >= 16 {
                put(floors);
            }
        }
        let has_tags = v < 4 || tag_type == 1 || tag_type == 2;
        let layers = if v < 4 {
            5
        } else {
            2 * walls + floors + 1 + u32::from(has_tags)
        };
        for l in 0..layers {
            for i in 0..4 {
                // orientation layers (odd positions for v >= 4) get `orientation`
                let is_orientation = if v < 4 {
                    l == 2
                } else {
                    l < 2 * walls && l % 2 == 1
                };
                put(if is_orientation {
                    orientation
                } else {
                    l * 100 + i
                });
            }
        }
        if v >= 2 {
            put(1);
            for x in [1u32, 42, 3, 4] {
                put(x);
            }
            if v >= 6 {
                put(7);
            }
        }
        if v >= 12 && (tag_type == 1 || tag_type == 2) {
            if v >= 18 {
                put(0);
            }
            put(1);
            for x in [0u32, 0, 2, 2] {
                put(x);
            }
            if v >= 13 {
                put(5);
            }
        }
        if v >= 14 {
            put(1); // one path
            put(2); // two points
            put(3);
            put(4);
            for p in [10u32, 11, 12, 13] {
                put(p);
                if p % 2 == 1 && v >= 15 {
                    put(1);
                }
            }
        }
        d
    }

    #[test]
    fn v18_without_tags() {
        let ds1 = Ds1::parse(&build(18, 0, 1, 1, 3)).unwrap();
        assert_eq!((ds1.width, ds1.height, ds1.act), (2, 2, 2));
        assert_eq!(ds1.files, [b"tiles\\a.dt1".to_vec()]);
        assert_eq!(ds1.walls[0], [0, 1, 2, 3]);
        assert_eq!(ds1.orientations[0], [3; 4]);
        assert_eq!(ds1.floors[0], [200, 201, 202, 203]);
        assert_eq!(ds1.shadow, [300, 301, 302, 303]);
        assert!(ds1.tags.is_none());
        assert!(ds1.groups.is_empty());
        assert_eq!(ds1.objects[0].id, 42);
        assert_eq!(ds1.objects[0].flags, 7);
        assert_eq!(ds1.paths[0].points.len(), 2);
        assert!(ds1.trailing.is_empty());
    }

    #[test]
    fn v18_with_tags_and_groups() {
        let ds1 = Ds1::parse(&build(18, 1, 2, 2, 5)).unwrap();
        assert_eq!(ds1.walls.len(), 2);
        assert_eq!(ds1.floors.len(), 2);
        assert_eq!(ds1.tags.as_ref().unwrap()[0], 700);
        assert_eq!(ds1.groups[0].unknown, 5);
        assert!(ds1.trailing.is_empty());
    }

    #[test]
    fn old_version_maps_orientation() {
        let ds1 = Ds1::parse(&build(6, 0, 1, 1, 7)).unwrap();
        assert_eq!(ds1.orientations[0], [0x05; 4]);
        assert!(
            Ds1::parse(&build(6, 0, 1, 1, 25)).is_err(),
            "out of lookup range"
        );
    }

    /// The live-data half of the edge-cases section is the game test
    /// `ds1_layer_limits_and_truncated_trees_groups`, which claims it.
    #[test]
    fn four_walls_two_floors_one_shadow() {
        let ds1 = Ds1::parse(&build(18, 0, 4, 2, 1)).unwrap();
        assert_eq!((ds1.walls.len(), ds1.orientations.len()), (4, 4));
        assert_eq!(ds1.floors[1], [900, 901, 902, 903]);
        assert_eq!(ds1.shadow, [1000, 1001, 1002, 1003]);
        assert!(Ds1::parse(&build(18, 0, 4, 3, 1)).is_err());
    }

    #[test]
    fn version_3_layer_order() {
        let ds1 = Ds1::parse(&build(3, 0, 1, 1, 1)).unwrap();
        assert_eq!(ds1.floors[0], [100, 101, 102, 103]);
        assert_eq!(ds1.tags.as_ref().unwrap()[0], 300);
        assert_eq!(ds1.shadow[0], 400);
    }

    #[test]
    fn errors() {
        assert!(
            Ds1::parse(&build(18, 0, 5, 1, 0)).is_err(),
            "too many walls"
        );
        assert!(Ds1::parse(&build(19, 0, 1, 1, 0)).is_err(), "version");
        let data = build(18, 0, 1, 1, 0);
        assert!(Ds1::parse(&data[..40]).is_err(), "truncated");
    }

    #[test]
    fn truncated_groups_read_as_zero() {
        let data = build(13, 1, 1, 1, 0);
        let full = Ds1::parse(&data).unwrap();
        assert!(!full.groups_truncated);
        assert_eq!(full.groups[0].unknown, 5);
        // Cut the last 8 bytes: the group's height and unknown are missing.
        let cut = Ds1::parse(&data[..data.len() - 8]).unwrap();
        assert!(cut.groups_truncated);
        assert_eq!(cut.groups[0].width, 2);
        assert_eq!((cut.groups[0].height, cut.groups[0].unknown), (0, 0));
    }

    #[test]
    fn cell_fields() {
        let cell = 0x8150_2A07u32;
        assert_eq!(cell::sub_index(cell), 0x2A);
        assert_eq!(cell::main_index(cell), 0x15);
        assert!(cell::hidden(cell));
    }

    #[test]
    fn regress_grid_size_overflow() {
        // Width and height 0xFFFFFFFF: cells × 4 × layers overflowed u64
        // (panic in debug builds).
        let mut d = Vec::new();
        for x in [1u32, 0xFFFF_FFFE, 0xFFFF_FFFE] {
            d.extend_from_slice(&x.to_le_bytes());
        }
        let err = Ds1::parse(&d).unwrap_err();
        assert!(err.to_string().contains("cannot fit"), "{err}");
    }

    mod robust {
        use super::*;
        use crate::robust::mutated;
        use crate::robust_tests::{check, config};
        use proptest::prelude::*;

        fn valid() -> [Vec<u8>; 4] {
            [
                build(18, 1, 2, 2, 5),
                build(13, 1, 1, 1, 0),
                build(6, 0, 1, 1, 7),
                build(3, 0, 1, 1, 1),
            ]
        }

        #[test]
        fn builders_are_valid() {
            for v in valid() {
                assert!(Ds1::parse(&v).is_ok());
            }
        }

        fn files() -> impl Strategy<Value = Vec<u8>> {
            let [a, b, c, d] = valid();
            prop_oneof![mutated(a), mutated(b), mutated(c), mutated(d)]
        }

        proptest! {
            #![proptest_config(config(64))]

            #[test]
            fn mutated_file(data in files()) {
                check(data, Ds1::parse);
            }
        }
    }
}
