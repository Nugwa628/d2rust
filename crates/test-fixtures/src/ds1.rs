//! DS1 writer: the inverse of `d2_formats::ds1::Ds1::parse`. Fields are
//! written in the order and under the version conditions the parser
//! reads them, from a [`Ds1`] value.
//!
//! What the parser derives, the writer takes back:
//! - `width` / `height` are the grid size; the file stores them − 1.
//! - Orientation layers are written as given. For `v < 7` the parser maps
//!   them through `ORIENTATION_LOOKUP`, so there they are the *stored*
//!   (pre-lookup) values and a round trip returns the mapped ones.
//! - `unknown_header` (`9 ≤ v ≤ 13`) and `unknown_groups` (`v ≥ 18`) are
//!   written when the version has them (zeros if `None`).
//! - Groups are written whole: `groups_truncated` is not reproduced.
//! - For `v ≥ 14` a path count is always written (0 if there are none),
//!   then `trailing`.

use d2_formats::ds1::Ds1;

/// The file bytes of `d`. Panics if a layer is not `width × height`
/// cells or the layer counts do not fit the version (test data only).
pub fn write(d: &Ds1) -> Vec<u8> {
    let v = d.version;
    assert!((1..=18).contains(&v), "DS1 version {v}");
    let mut out = Vec::new();

    u32_(&mut out, v);
    u32_(&mut out, d.width.wrapping_sub(1));
    u32_(&mut out, d.height.wrapping_sub(1));
    if v >= 8 {
        u32_(&mut out, d.act);
    }
    if v >= 10 {
        u32_(&mut out, d.tag_type);
    }
    if v >= 3 {
        u32_(&mut out, d.files.len() as u32);
        for f in &d.files {
            assert!(!f.contains(&0), "DS1 file name with a NUL");
            out.extend_from_slice(f);
            out.push(0);
        }
    }
    if (9..=13).contains(&v) {
        out.extend_from_slice(&d.unknown_header.unwrap_or_default());
    }
    let has_tags = v < 4 || d.tag_type == 1 || d.tag_type == 2;
    if v >= 4 {
        u32_(&mut out, d.walls.len() as u32);
        if v >= 16 {
            u32_(&mut out, d.floors.len() as u32);
        } else {
            assert_eq!(d.floors.len(), 1, "v < 16 has one floor layer");
        }
    } else {
        assert!(
            d.walls.len() == 1 && d.floors.len() == 1,
            "v < 4 has one wall and one floor layer"
        );
    }
    assert_eq!(
        d.walls.len(),
        d.orientations.len(),
        "one orientation per wall"
    );
    assert_eq!(d.tags.is_some(), has_tags, "tag layer presence");

    let cells = d.width as usize * d.height as usize;
    let layer = |out: &mut Vec<u8>, l: &[u32]| {
        assert_eq!(l.len(), cells, "layer size");
        for &c in l {
            out.extend_from_slice(&c.to_le_bytes());
        }
    };
    if v < 4 {
        layer(&mut out, &d.walls[0]);
        layer(&mut out, &d.floors[0]);
        layer(&mut out, &d.orientations[0]);
        layer(&mut out, d.tags.as_ref().expect("v < 4 has tags"));
        layer(&mut out, &d.shadow);
    } else {
        for (w, o) in d.walls.iter().zip(&d.orientations) {
            layer(&mut out, w);
            layer(&mut out, o);
        }
        for f in &d.floors {
            layer(&mut out, f);
        }
        layer(&mut out, &d.shadow);
        if let Some(t) = &d.tags {
            layer(&mut out, t);
        }
    }

    if v >= 2 {
        u32_(&mut out, d.objects.len() as u32);
        for o in &d.objects {
            for x in [o.kind, o.id, o.x, o.y] {
                u32_(&mut out, x);
            }
            if v >= 6 {
                u32_(&mut out, o.flags);
            }
        }
    } else {
        assert!(d.objects.is_empty(), "v < 2 has no objects");
    }

    if v >= 12 && (d.tag_type == 1 || d.tag_type == 2) {
        if v >= 18 {
            out.extend_from_slice(&d.unknown_groups.unwrap_or_default());
        }
        u32_(&mut out, d.groups.len() as u32);
        for g in &d.groups {
            for x in [g.x, g.y, g.width, g.height] {
                u32_(&mut out, x);
            }
            if v >= 13 {
                u32_(&mut out, g.unknown);
            }
        }
    } else {
        assert!(d.groups.is_empty(), "no groups section in this file");
    }

    if v >= 14 {
        u32_(&mut out, d.paths.len() as u32);
        for p in &d.paths {
            for x in [p.points.len() as u32, p.x, p.y] {
                u32_(&mut out, x);
            }
            for q in &p.points {
                u32_(&mut out, q.x);
                u32_(&mut out, q.y);
                if v >= 15 {
                    u32_(&mut out, q.action);
                }
            }
        }
    } else {
        assert!(d.paths.is_empty(), "v < 14 has no paths");
    }
    out.extend_from_slice(&d.trailing);
    out
}

/// An empty `width × height` v18 preset: one wall layer (with its
/// orientations), one floor layer, the shadow, tag type 0 (no tag
/// layer, no groups), no objects or paths. All cells 0.
pub fn blank(width: u32, height: u32, act: u32) -> Ds1 {
    let cells = (width * height) as usize;
    Ds1 {
        version: 18,
        width,
        height,
        act,
        tag_type: 0,
        files: Vec::new(),
        unknown_header: None,
        walls: vec![vec![0; cells]],
        orientations: vec![vec![0; cells]],
        floors: vec![vec![0; cells]],
        shadow: vec![0; cells],
        tags: None,
        objects: Vec::new(),
        unknown_groups: None,
        groups: Vec::new(),
        groups_truncated: false,
        paths: Vec::new(),
        trailing: Vec::new(),
    }
}

fn u32_(out: &mut Vec<u8>, x: u32) {
    out.extend_from_slice(&x.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::ds1::{Ds1Group, Ds1Object, Ds1Path, Ds1PathPoint};

    fn numbered(n: usize, base: u32) -> Vec<u32> {
        (0..n as u32).map(|i| base + i * 0x0101).collect()
    }

    /// Every layer and record filled with distinct values.
    fn full(version: u32, tag_type: u32) -> Ds1 {
        let (w, h) = (3, 2);
        let cells = (w * h) as usize;
        let walls = if version >= 4 { 2 } else { 1 };
        let floors = if version >= 16 { 2 } else { 1 };
        let tag_type = if version >= 10 { tag_type } else { 0 };
        let has_tags = version < 4 || tag_type == 1 || tag_type == 2;
        Ds1 {
            version,
            width: w,
            height: h,
            act: if version >= 8 { 2 } else { 0 },
            tag_type,
            files: if version >= 3 {
                vec![b"Synth\\A.dt1".to_vec(), b"Synth\\B.dt1".to_vec()]
            } else {
                Vec::new()
            },
            unknown_header: (9..=13)
                .contains(&version)
                .then_some([1, 2, 3, 4, 5, 6, 7, 8]),
            walls: (0..walls).map(|i| numbered(cells, 0x10 + i)).collect(),
            orientations: (0..walls).map(|i| vec![i + 3; cells]).collect(),
            floors: (0..floors).map(|i| numbered(cells, 0x200 + i)).collect(),
            shadow: numbered(cells, 0x3000),
            tags: has_tags.then(|| numbered(cells, 0x40000)),
            objects: if version >= 2 {
                vec![
                    Ds1Object {
                        kind: 2,
                        id: 150,
                        x: 7,
                        y: 9,
                        flags: if version >= 6 { 1 } else { 0 },
                    },
                    Ds1Object {
                        kind: 1,
                        id: 4,
                        x: 1,
                        y: 2,
                        flags: 0,
                    },
                ]
            } else {
                Vec::new()
            },
            unknown_groups: (version >= 18 && has_tags).then_some([9, 8, 7, 6]),
            groups: if version >= 12 && has_tags {
                vec![Ds1Group {
                    x: 1,
                    y: 0,
                    width: 2,
                    height: 1,
                    unknown: if version >= 13 { 3 } else { 0 },
                }]
            } else {
                Vec::new()
            },
            groups_truncated: false,
            paths: if version >= 14 {
                vec![Ds1Path {
                    x: 7,
                    y: 9,
                    points: vec![
                        Ds1PathPoint {
                            x: 10,
                            y: 11,
                            action: if version >= 15 { 4 } else { 1 },
                        },
                        Ds1PathPoint {
                            x: 12,
                            y: 13,
                            action: 1,
                        },
                    ],
                }]
            } else {
                Vec::new()
            },
            trailing: Vec::new(),
        }
    }

    #[test]
    fn round_trips_every_version_from_7() {
        for v in 7..=18 {
            for tag_type in [0, 1, 2] {
                let d = full(v, tag_type);
                let back = Ds1::parse(&write(&d)).unwrap_or_else(|e| panic!("v{v}: {e}"));
                assert_eq!(back, d, "v{v} tag_type {tag_type}");
            }
        }
    }

    /// `v < 7`: the stored orientation 7 reads back as 0x05
    /// (`ORIENTATION_LOOKUP`, the reference vector); everything else as written.
    #[test]
    fn old_versions_map_orientations() {
        for v in 1..7 {
            let mut d = full(v, 0);
            for o in &mut d.orientations {
                o.fill(7);
            }
            let back = Ds1::parse(&write(&d)).unwrap_or_else(|e| panic!("v{v}: {e}"));
            let mut want = d.clone();
            for o in &mut want.orientations {
                o.fill(0x05);
            }
            assert_eq!(back, want, "v{v}");
        }
    }

    /// Spec test vector: v18, 2×2, one wall, one floor, tag type 0: the
    /// layers wall, orientation, floor, shadow and nothing else.
    #[test]
    fn minimal_v18_layout() {
        let d = blank(2, 2, 0);
        let bytes = write(&d);
        // version, w−1, h−1, act, tag_type, file count, walls, floors,
        // 4 layers of 4 cells, object count, path count (no groups
        // section with tag_type 0).
        assert_eq!(bytes.len(), 4 * (8 + 16 + 2));
        assert_eq!(&bytes[4..12], &[1, 0, 0, 0, 1, 0, 0, 0]);
        let back = Ds1::parse(&bytes).unwrap();
        assert_eq!(back.walls.len(), 1);
        assert_eq!(back.floors.len(), 1);
        assert!(back.tags.is_none());
        assert_eq!(back, d);
    }

    #[test]
    fn trailing_bytes_are_kept() {
        let mut d = full(18, 1);
        d.trailing = vec![0xAA, 0xBB];
        assert_eq!(Ds1::parse(&write(&d)).unwrap(), d);
    }
}
