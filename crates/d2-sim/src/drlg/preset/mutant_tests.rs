//! Tests written to kill mutants `cargo mutants` left alive in
//! `drlg::preset`. Each asserts what the preset rules say; the survivors no
//! test can observe are listed in the mutation report.

use d2_data::tables::{Lvlprest, Record};

use super::data::{MonPresetRow, PresetDef, PresetTables};
use super::*;

// ---- data.rs ------------------------------------------------------------------

/// The lvlprest columns the DRLG reads, `File1..6`
/// NUL-trimmed, `PopPad` and `Files` signed.
#[test]
fn preset_def_from_record() {
    let mut b = vec![0u8; Lvlprest::SIZE];
    let put = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    for (k, at) in [0, 4, 8, 12, 16, 20, 24, 28, 40, 44, 48, 52, 56]
        .into_iter()
        .enumerate()
    {
        put(&mut b, at, 10 + k as u32);
    }
    put(&mut b, 60, (-4i32) as u32);
    put(&mut b, 64, 3);
    for (k, at) in [68, 128, 188, 248, 308, 368].into_iter().enumerate() {
        b[at..at + 3].copy_from_slice(format!("f{k}x").as_bytes()[..3].as_ref());
    }
    put(&mut b, 428, 0x55);
    let d = PresetDef::from_record(&Lvlprest::decode(&b));
    let want = PresetDef {
        def: 10,
        level_id: 11,
        populate: 12,
        logicals: 13,
        outdoors: 14,
        animate: 15,
        kill_edge: 16,
        fill_blanks: 17,
        size_x: 18,
        size_y: 19,
        automap: 20,
        scan: 21,
        pops: 22,
        pop_pad: -4,
        files: 3,
        file: [0, 1, 2, 3, 4, 5].map(|k| format!("f{k}x").into_bytes()),
        dt1_mask: 0x55,
        anim_speed: 0,
    };
    assert_eq!(d, want);
}

/// A monpreset record (as its place callback stores it)
/// holds the kind at +1 and the place index (u16) at +2.
#[test]
fn monpreset_row_from_bytes() {
    assert_eq!(
        MonPresetRow::from_record_bytes(&[9, 2, 0x34, 0x12]),
        MonPresetRow {
            kind: 2,
            place: 0x1234
        }
    );
}

/// `preset-tables.tsv` strict parse: door values (level, main, sub ≥ 0,
/// right 0 or 1, unit type ≥ 0) are checked, 0 is allowed, and errors
/// name the file line.
#[test]
fn preset_tables_door_ranges() {
    let header = "table\ta\tb\tc\td\te\tf\tg\th";
    let parse = |rows: &[&str]| PresetTables::from_tsv(&format!("{header}\n{}\n", rows.join("\n")));
    let t = parse(&[
        "door\t0\t0\t0\t0\t0\t5\t1\t-1",
        "door\t0\t1\t2\t1\t2\t6\t0\t0",
    ])
    .unwrap();
    assert_eq!(t.doors.len(), 1);
    assert_eq!(t.doors[0].1.len(), 2);
    for (k, bad) in [
        "door\t-1\t0\t0\t0\t0\t5\t1\t-1",
        "door\t0\t-1\t0\t0\t0\t5\t1\t-1",
        "door\t0\t0\t-1\t0\t0\t5\t1\t-1",
        "door\t0\t0\t0\t-1\t0\t5\t1\t-1",
        "door\t0\t0\t0\t2\t0\t5\t1\t-1",
        "door\t0\t0\t0\t0\t-1\t5\t1\t-1",
    ]
    .into_iter()
    .enumerate()
    {
        let ok = "door\t0\t0\t0\t0\t0\t5\t1\t-1";
        assert_eq!(
            parse(&[ok, ok, bad]),
            Err(PresetError::Tsv("line 4: door value out of range".into())),
            "case {k}"
        );
    }
}

// ---- ds1.rs ---------------------------------------------------------------------

use super::ds1::{Ds1File, Ds1Input, Ds1ObjectInput, ORIENTATION_REMAP};
use super::tests::{ds1, preset_data};

/// The DS1 parser at the version boundaries: act from v 8 (else 0),
/// tag type from v 10, the orientation remap below v 7, floors from v 4.
#[test]
fn ds1_version_boundaries() {
    let pd = preset_data();
    let file = |v: u32| {
        let mut i: Ds1Input = ds1(1, 1);
        i.version = v;
        i.act = 3;
        i.tag_type = 2;
        i.orientations[0][0] = 26;
        i.floors[0][0] = 7;
        Ds1File::from_input(&i, &pd).unwrap()
    };
    let (f7, f8) = (file(7), file(8));
    assert_eq!((f7.act, f8.act), (0, 3));
    let (f9, f10) = (file(9), file(10));
    assert_eq!((f9.tag_type, f10.tag_type), (0, 2));
    let (f6, f7) = (file(6), file(7));
    assert_eq!(f6.orientations[0][0], ORIENTATION_REMAP[26]);
    assert_eq!(f7.orientations[0][0], 26);
    let (f3, f4) = (file(3), file(4));
    assert!(f3.floors.is_empty());
    assert_eq!(f4.floors[0][0], 7);
}

/// An object id ≥ 150 (v ≥ 6) becomes id − 150; the
/// monster → object conversions apply to monster records only.
#[test]
fn ds1_object_ids_and_conversions() {
    let pd = preset_data();
    let mut i: Ds1Input = ds1(1, 1);
    i.version = 18;
    i.act = 2;
    let obj = |kind, id| Ds1ObjectInput {
        kind,
        id,
        x: 1,
        y: 2,
        flags: 0,
    };
    // Object 150 → class 0; object 447 → class 297 (no conversion).
    i.objects = vec![obj(2, 150), obj(2, 447)];
    let f = Ds1File::from_input(&i, &pd).unwrap();
    // Head insertion: reverse file order.
    let got: Vec<_> = f
        .units
        .iter()
        .map(|u| (u.unit_type, u.class, u.mode))
        .collect();
    assert_eq!(got, [(2, 297, 0), (2, 0, 0)]);
}

// ---- map.rs ---------------------------------------------------------------------

use super::tests::World;

/// In multi-room mode each room's link is the
/// map's link-grid value of its cell, the grid being (w/8 + 1) wide.
#[test]
fn build_area_links_by_cell() {
    let mut w = World::new();
    let l = w.level(2, 5, 16, 16);
    w.file(5, b"a.ds1", ds1(16, 16));
    w.init(l);
    let rooms = w
        .run(|p, d, c| {
            let rect = d.level(l).rect;
            let m = p.alloc_map(d, c, l, 5, rect)?;
            p.map_mut(m)?.link_grid = Some((1..=9).collect());
            p.build_area(d, c, l, m, 0, false)?;
            Ok::<_, PresetError>(d.level_rooms(l))
        })
        .unwrap();
    let mut links: Vec<(i32, i32, u32)> = rooms
        .iter()
        .map(|&r| {
            let rect = w.drlg.room(r).rect;
            (rect.x, rect.y, w.p.room(r).unwrap().link)
        })
        .collect();
    links.sort();
    assert_eq!(links, [(0, 0, 1), (0, 8, 4), (8, 0, 2), (8, 8, 5)]);
}

/// With direction −1 (`Files` = 0 at allocation, no
/// draw), the direction takes the map's picked file (0 here).
#[test]
fn generate_direction_from_the_picked_file() {
    let mut w = World::new();
    let l = w.level(2, 5, 8, 8);
    w.file(5, b"a.ds1", ds1(8, 8));
    w.pd.defs[5].files = 0;
    w.init(l);
    assert_eq!(w.p.info(l).unwrap().direction, -1);
    w.generate(l).unwrap();
    let info = w.p.info(l).unwrap();
    assert_eq!(info.direction, 0);
    let m = info.map.unwrap();
    assert_eq!(w.p.map(m).unwrap().picked_file, 0);
}

/// With `Scan`, each file unit of type 2 with class
/// < 573 and objects `SubClass` bit 0x40 flags its cell (x/5/8, y/5/8)
/// with 0x30000.
#[test]
fn build_area_waypoint_cells() {
    use crate::drlg::room_flags;
    let mut w = World::new();
    w.dd.object_subclass[573] = 0x40;
    let l = w.level(2, 5, 16, 16);
    let mut f = ds1(16, 16);
    let obj = |id, x, y| Ds1ObjectInput {
        kind: 2,
        id,
        x,
        y,
        flags: 0,
    };
    // Class 119 (id 269) at sub-tiles (45, 50) → cell (1, 1); class 573
    // (id 723) at (5, 5) → cell (0, 0) not flagged.
    f.objects = vec![obj(269, 45, 50), obj(723, 5, 5)];
    w.file(5, b"a.ds1", f);
    w.pd.defs[5].scan = 1;
    w.init(l);
    w.generate(l).unwrap();
    let mut flagged: Vec<(i32, i32, bool)> = w
        .drlg
        .level_rooms(l)
        .iter()
        .map(|&r| {
            let room = w.drlg.room(r);
            (
                room.rect.x,
                room.rect.y,
                room.flags & room_flags::ANY_WAYPOINT == room_flags::ANY_WAYPOINT,
            )
        })
        .collect();
    flagged.sort();
    assert_eq!(
        flagged,
        [(0, 0, false), (0, 8, false), (8, 0, false), (8, 8, true)]
    );
}

// ---- map.rs: first activation and room.rs -------------------------

/// Level 2 with one 8 × 8 room of map def `def`, the DS1 `f`, the map's
/// picked file `file` and the hardcoded units pending.
fn one_preset_room(def: u32, file: i32, f: Ds1Input) -> (World, DrlgRoomId, MapId) {
    let mut w = World::new();
    let l = w.level(2, def, 8, 8);
    w.file(def, b"a.ds1", f);
    // Every File column names the same DS1 (the picked file is set below).
    w.pd.defs[def as usize].file = std::array::from_fn(|_| b"a.ds1".to_vec());
    w.init(l);
    w.generate(l).unwrap();
    let r = w.drlg.level_rooms(l)[0];
    let m = w.p.room(r).unwrap().map;
    let map = w.p.map_mut(m).unwrap();
    map.picked_file = file;
    map.hardcoded_pending = true;
    (w, r, m)
}

/// Blood Moor wild border maps (d 4–7) with picked
/// file 3 add navi (class 266, or −1 when monstats has ≤ 266 rows) at
/// ((map x + w/2)·5, (map y + h/2)·5), mode 1.
#[test]
fn navi_unit_on_blood_moor_border() {
    for (rows, class) in [(734, 266), (266, -1)] {
        let (mut w, r, m) = one_preset_room(5, 3, ds1(8, 8));
        w.pd.monstats_count = rows;
        w.run(|p, d, c| p.add_preset_units(d, c, r)).unwrap();
        let u = &w.p.map(m).unwrap().units[0];
        assert_eq!(
            (u.unit_type, u.class, u.mode, u.x, u.y),
            (1, class, 1, 20, 20)
        );
    }
}

/// River objects, from the rule: d ≠ 27 walks row 0
/// of floor layer 0 for style 2 / sub 24 cells (c += 4 after one), each
/// giving sounds(map x + c + 1) and strip(c); strips skip 3 rows after a
/// style-4 cell with sub in {0, 4, 8, 16, 29, 39}.
#[test]
fn river_objects_by_the_rules() {
    use crate::drlg::room_flags;
    let style_cell = |style: u32, sub: u32| (style << 20) | (sub << 8);
    let mut f = ds1(8, 8);
    let fl = &mut f.floors[0];
    // Row 0: style 2 / sub 24 at c = 1, 3 (skipped by c += 4) and 6.
    for c in [1, 3, 6] {
        fl[c] = style_cell(2, 24);
    }
    // Column 1, row 2: style 4 sub 8 → skip 3 rows; column 6 row 1: sub 5
    // (no skip).
    fl[2 * 9 + 1] = style_cell(4, 8);
    fl[9 + 6] = style_cell(4, 5);
    let (mut w, r, m) = one_preset_room(26, 0, f.clone());
    w.drlg.room_mut(r).flags |= room_flags::AUTOMAP_REVEAL;
    let rect = w.p.map(m).unwrap().rect;
    w.run(|p, d, c| p.add_preset_units(d, c, r)).unwrap();
    // The model.
    let g = |c: i32, row: i32| f.floors[0][(row * 9 + c) as usize];
    let mut out: Vec<(i32, i32, i32)> = Vec::new();
    let sounds = |out: &mut Vec<(i32, i32, i32)>, cx: i32| {
        let mut sy = rect.y * 5;
        while sy < (rect.y + 8) * 5 {
            out.push((65, cx * 5, sy));
            sy += 40;
        }
    };
    let strip = |out: &mut Vec<(i32, i32, i32)>, c: i32| {
        let mut row = 0;
        while row < rect.h {
            let x0 = (rect.x + c) * 5 - 5;
            let y = (rect.y + row) * 5;
            for (k, class) in [40, 41, 41, 41, 42].into_iter().enumerate() {
                out.push((class, x0 + 5 * k as i32, y));
            }
            let v = g(c.max(0), row);
            if (v >> 20) & 0x3F == 4 && [0, 4, 8, 16, 29, 39].contains(&((v >> 8) & 0xFF)) {
                row += 3;
            }
            row += 1;
        }
    };
    let mut c = 0;
    while c < rect.w {
        let v = g(c, 0);
        if (v >> 20) & 0x3F == 2 && (v >> 8) & 0xFF == 24 {
            sounds(&mut out, rect.x + c + 1);
            strip(&mut out, c);
            c += 4;
        }
        c += 1;
    }
    out.reverse();
    let got: Vec<(i32, i32, i32)> =
        w.p.map(m)
            .unwrap()
            .units
            .iter()
            .filter(|u| u.flags == 1)
            .map(|u| (u.class, u.x, u.y))
            .collect();
    assert_eq!(got, out);
    assert!(out.len() > 40);
}

/// The room's grids are (w + 1) × (h + 1) from the
/// DS1 sub-rectangle; border cells of wall grid 0 and the floor grid get
/// 0x84.
#[test]
fn room_grid_size_and_borders() {
    let (mut w, r, _) = one_preset_room(9, 0, ds1(8, 8));
    let grids = w.run(|p, d, c| {
        p.add_preset_units(d, c, r)?;
        p.room_grids(d, c, r)
    });
    let g = grids.unwrap();
    for pass in &g.passes {
        assert_eq!((pass.cells.width, pass.cells.height), (9, 9));
        assert_eq!(pass.cells.get(8, 8) & 0x84, 0x84);
        assert_eq!(pass.cells.get(4, 4) & 0x84, 0);
    }
}
