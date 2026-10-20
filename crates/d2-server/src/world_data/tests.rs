//! Tests of the `world_data` providers: archive-name rules, DS1 / DT1
//! conversion to the DRLG seam types, the world file loader and the
//! hireling tables.
//!
//! Synthetic DS1 / DT1 bytes run through the providers in CI; the Act I
//! levels generated from the live tables through
//! `wiring::worldgen::levels::WorldTypes` live in the `game` submodule
//! (`#[ignore]`, `D2_GAME_DIR`).

mod game;

use std::collections::BTreeMap;

use d2_sim::drlg::outdoor::{SubDefs, SubFiles, SubRow};
use d2_sim::drlg::preset::{Ds1File, Ds1Source, PresetData, PresetDef, PresetTables};
use d2_sim::drlg::tiles::{ACT_EDGE_TILE, FIXED_LIBRARY};
use d2_sim::drlg::TileSource;
use d2_sim::world::hirelings::HirelingTables;

use super::*;

// ---- synthetic files ------------------------------------------------------------

/// A DS1 to write: version ≥ 14 layout.
struct Ds1Spec {
    version: u32,
    /// Stored width and height (cells per row = width + 1).
    w: u32,
    h: u32,
    act: u32,
    tag_type: u32,
    walls: u32,
    floors: u32,
    /// (type, id, x, y, flags).
    objects: Vec<[u32; 5]>,
    /// (x, y, w, h, unknown).
    groups: Vec<[u32; 5]>,
    /// (x, y, points (x, y, action)).
    paths: Vec<(u32, u32, Vec<[u32; 3]>)>,
}

impl Ds1Spec {
    fn new(version: u32, w: u32, h: u32) -> Self {
        Self {
            version,
            w,
            h,
            act: 0,
            tag_type: 0,
            walls: 1,
            floors: 1,
            objects: Vec::new(),
            groups: Vec::new(),
            paths: Vec::new(),
        }
    }

    fn cells(&self) -> u32 {
        (self.w + 1) * (self.h + 1)
    }

    /// Layer values: layer `n` (stream order) cell `i` = `n · 1000 + i`.
    fn bytes(&self) -> Vec<u8> {
        let v = self.version;
        let mut d = Vec::new();
        let mut put = |x: u32| d.extend_from_slice(&x.to_le_bytes());
        put(v);
        put(self.w);
        put(self.h);
        if v >= 8 {
            put(self.act);
        }
        if v >= 10 {
            put(self.tag_type);
        }
        if v >= 3 {
            put(0); // no tile-file names
        }
        if (9..=13).contains(&v) {
            put(0); // unknown header, 8 bytes
            put(0);
        }
        let mut layer = 0;
        let mut put_layer = |put: &mut dyn FnMut(u32)| {
            for i in 0..self.cells() {
                put(layer * 1000 + i);
            }
            layer += 1;
        };
        if v < 4 {
            for _ in 0..5 {
                put_layer(&mut put);
            }
        } else {
            put(self.walls);
            if v >= 16 {
                put(self.floors);
            }
            for _ in 0..self.walls {
                put_layer(&mut put); // wall
                put_layer(&mut put); // orientation
            }
            for _ in 0..if v >= 16 { self.floors } else { 1 } {
                put_layer(&mut put);
            }
            put_layer(&mut put); // shadow
            if self.tag_type == 1 || self.tag_type == 2 {
                put_layer(&mut put);
            }
        }
        put(self.objects.len() as u32);
        for o in &self.objects {
            for (k, &x) in o.iter().enumerate() {
                if k < 4 || v >= 6 {
                    put(x);
                }
            }
        }
        if v >= 12 && (self.tag_type == 1 || self.tag_type == 2) {
            if v >= 18 {
                put(0);
            }
            put(self.groups.len() as u32);
            for g in &self.groups {
                for (k, &x) in g.iter().enumerate() {
                    if k < 4 || v >= 13 {
                        put(x);
                    }
                }
            }
        }
        if v >= 14 {
            put(self.paths.len() as u32);
            for (x, y, pts) in &self.paths {
                put(pts.len() as u32);
                put(*x);
                put(*y);
                for p in pts {
                    put(p[0]);
                    put(p[1]);
                    if v >= 15 {
                        put(p[2]);
                    }
                }
            }
        }
        d
    }

    fn parse(&self) -> Ds1 {
        Ds1::parse(&self.bytes()).expect("synthetic DS1 parses")
    }
}

/// A DT1 of block-less tiles: (orientation, main, sub, rarity, material,
/// first sub-tile flag).
fn dt1_bytes(tiles: &[[u32; 6]]) -> Vec<u8> {
    let mut d = Vec::new();
    let put = |d: &mut Vec<u8>, x: u32| d.extend_from_slice(&x.to_le_bytes());
    put(&mut d, 7);
    put(&mut d, 6);
    d.extend_from_slice(&[0; 260]);
    put(&mut d, tiles.len() as u32);
    put(&mut d, 276);
    for t in tiles {
        put(&mut d, 0); // light direction
        d.extend_from_slice(&0u16.to_le_bytes()); // roof height
        d.extend_from_slice(&(t[4] as u16).to_le_bytes());
        for x in [80, 160, 0] {
            put(&mut d, x); // height, width, unknown height
        }
        for &x in &t[..4] {
            put(&mut d, x);
        }
        put(&mut d, 0); // unknown colour
        let mut flags = [0u8; 25];
        flags[0] = t[5] as u8;
        flags[24] = 0x80;
        d.extend_from_slice(&flags);
        d.extend_from_slice(&[0; 7]);
        for _ in 0..3 {
            put(&mut d, 0); // blocks offset, length, count
        }
        d.extend_from_slice(&[0; 4]);
        put(&mut d, 0); // unknown 0x58, cache index
        put(&mut d, 0);
    }
    d
}

/// Preset view: monstats count 10, one lvlprest row naming `files`.
fn preset_data(files: [&[u8]; 6]) -> PresetData {
    PresetData {
        defs: vec![PresetDef {
            files: 1,
            file: files.map(<[u8]>::to_vec),
            ..PresetDef::default()
        }],
        monpreset_acts: Default::default(),
        monpreset: Vec::new(),
        monstats_count: 10,
        superuniques_count: 0,
        hdm_item: -1,
        tables: PresetTables::spec().unwrap(),
    }
}

fn sub_row(file: &[u8]) -> SubRow {
    SubRow {
        type_: 0,
        file: file.to_vec(),
        check_all: 0,
        bord_type: 0,
        dt1_mask: 0,
        grid_size: 0,
        prob: [0; 5],
        trials: [0; 5],
        max: [0; 5],
    }
}

fn outdoor_data(files: &[&[u8]]) -> OutdoorData {
    OutdoorData {
        levels: vec![SubDefs {
            sub_type: -1,
            sub_theme: -1,
            sub_waypoint: -1,
            sub_shrine: -1,
        }],
        presets: Vec::new(),
        subs: files.iter().map(|f| sub_row(f)).collect(),
    }
}

// ---- paths -------------------------------------------------------------------------

// Table tile paths get the tile prefix and `\` separators; fixed paths stay as is.
#[test]
fn archive_names_follow_the_tile_path_fix() {
    assert_eq!(
        archive_name(b"Act1/Town/Floor.dt1"),
        b"DATA\\GLOBAL\\TILES\\Act1\\Town\\Floor.dt1"
    );
    assert_eq!(
        archive_name(b"Act1\\Outdoors\\Bridge.ds1"),
        b"DATA\\GLOBAL\\TILES\\Act1\\Outdoors\\Bridge.ds1"
    );
    // 0/1-character strings are unchanged (the `0` placeholders).
    assert_eq!(archive_name(b"0"), b"0");
    assert_eq!(archive_name(b""), b"");
    assert!(!names_file(b"0"));
    // Already fixed (a fixed-up table, the fixed library): as is.
    let fixed = b"DATA\\GLOBAL\\TILES\\Act1\\Town\\Floor.dt1";
    assert_eq!(archive_name(fixed), fixed);
    assert_eq!(archive_name(FIXED_LIBRARY[0]), FIXED_LIBRARY[0]);
}

// ---- DS1 → Ds1Input --------------------------------------------------------------

// The DS1 conversion keeps the stored sizes, layers and records.
#[test]
fn ds1_input_keeps_stored_sizes_layers_and_records() {
    let mut s = Ds1Spec::new(18, 3, 2);
    s.act = 2;
    s.walls = 2;
    s.objects = vec![[1, 4, 7, 9, 0], [2, 5, 3, 4, 1]];
    s.paths = vec![(7, 9, vec![[10, 11, 2], [12, 13, 3]])];
    let d = s.parse();
    assert_eq!((d.width, d.height), (4, 3), "d2_formats adds 1");
    let i = ds1_input(b"x.ds1", &d).unwrap();
    assert_eq!((i.version, i.width, i.height, i.act), (18, 3, 2, 2));
    let cells = 12;
    assert_eq!(i.walls.len(), 2);
    assert_eq!(i.orientations.len(), 2);
    assert_eq!(i.floors.len(), 1);
    // Stream order: wall 0, orientation 0, wall 1, orientation 1, floor,
    // shadow.
    assert_eq!(i.walls[1][5], 2005);
    assert_eq!(i.orientations[0][cells - 1], 1000 + cells as u32 - 1);
    assert_eq!(i.orientations[1][0], 3000);
    assert_eq!(i.floors[0][3], 4003);
    assert_eq!(i.shadow[0], 5000);
    assert_eq!(i.objects.len(), 2);
    assert_eq!(
        (i.objects[1].kind, i.objects[1].id, i.objects[1].flags),
        (2, 5, 1)
    );
    assert_eq!(i.paths[0].points, vec![(10, 11, 2), (12, 13, 3)]);
    // The preset parser keeps the stored sizes and builds its unit list.
    let f = Ds1File::from_input(&i, &preset_data(Default::default())).unwrap();
    assert_eq!((f.width, f.height, f.stride()), (3, 2, 4));
    assert_eq!(f.units.len(), 2);
    let monster = f.units.iter().find(|u| u.unit_type == 1).unwrap();
    assert_eq!((monster.class, monster.x, monster.y), (4, 7, 9));
    assert_eq!(monster.path.as_ref().unwrap().len(), 2);
}

#[test]
fn ds1_before_version_7_is_refused() {
    // A parsed v6 file (d2_formats has already remapped its orientations).
    let mut d = Ds1Spec::new(18, 1, 1).parse();
    d.version = 6;
    match ds1_input(b"old.ds1", &d) {
        Err(WorldDataError::OldOrientations { version: 6, .. }) => {}
        other => panic!("{other:?}"),
    }
}

// ---- DS1 → SubFile ---------------------------------------------------------------

// The lvlsub conversion builds grids, groups and units from the parsed DS1.
#[test]
fn sub_file_has_the_file_grids_groups_and_units() {
    let mut s = Ds1Spec::new(18, 4, 1);
    s.tag_type = 2;
    s.walls = 2;
    s.groups = vec![[0, 0, 1, 1, 3], [2, 0, 2, 1, 0]];
    s.objects = vec![[1, 2, 6, 3, 0]];
    let d = s.parse();
    let f = sub_file(b"sub.ds1", &d, &preset_data(Default::default())).unwrap();
    assert_eq!(f.method, 2);
    assert_eq!(
        f.groups,
        vec![
            SubGroup {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
                variants: 3
            },
            SubGroup {
                x: 2,
                y: 0,
                w: 2,
                h: 1,
                variants: 0
            },
        ]
    );
    // Grids are (W + 1) × (H + 1) = 5 × 2.
    assert_eq!((f.walls[0].width, f.walls[0].height), (5, 2));
    assert_eq!(f.walls.len(), 2);
    assert_eq!(f.tile_types.len(), 2);
    // Cell (1, 1) = index 6. Wall layer 1 is read ORed with 1 << 18.
    assert_eq!(f.wall_at(0, 1, 1), 6);
    assert_eq!(f.wall_at(1, 1, 1), (2006) | (1 << 18));
    assert_eq!(f.tile_type_at(1, 1, 1), 3006);
    assert_eq!(f.floor_at(1, 1), 4006);
    assert_eq!(f.shadow_at(1, 1), 5006);
    assert_eq!(f.units.len(), 1);
    assert_eq!(
        (
            f.units[0].unit_type,
            f.units[0].class,
            f.units[0].x,
            f.units[0].y
        ),
        (1, 2, 6, 3)
    );
}

// The method of a lvlsub file is its DS1 tag type after parsing.
#[test]
fn sub_file_method_is_the_tag_type_after_the_parser() {
    // v < 10 has no stored tag type: method 0 (the DS1 parser's rule).
    let d = Ds1Spec::new(9, 1, 1).parse();
    let f = sub_file(b"s.ds1", &d, &preset_data(Default::default())).unwrap();
    assert_eq!(f.method, 0);
    assert!(f.groups.is_empty());
}

// ---- DT1 → TileInfo --------------------------------------------------------------

#[test]
fn tile_info_takes_the_header_fields() {
    let dt1 = Dt1::parse(&dt1_bytes(&[[3, 5, 7, 2, 0x40, 0x09], [0, 1, 0, 0, 0, 0]])).unwrap();
    let t: Vec<TileInfo> = dt1.tiles.iter().map(tile_info).collect();
    assert_eq!(t.len(), 2);
    assert_eq!(
        (
            t[0].orientation,
            t[0].main,
            t[0].sub,
            t[0].rarity,
            t[0].material
        ),
        (3, 5, 7, 2, 0x40)
    );
    assert_eq!(t[0].subtile_flags[0], 0x09);
    assert_eq!(t[0].subtile_flags[24], 0x80);
    assert_eq!(t[1].main, 1);
}

// ---- loading -----------------------------------------------------------------------

fn floor_dt1() -> Vec<u8> {
    dt1_bytes(&[[0, 0, 0, 1, 0, 0]])
}

/// Files by archive name, and the names asked for.
struct Disk {
    files: BTreeMap<String, Vec<u8>>,
    asked: Vec<String>,
}

impl Disk {
    fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        self.asked.push(name.to_owned());
        self.files.get(name).cloned().ok_or_else(|| "absent".into())
    }
}

fn new_disk() -> Disk {
    let mut files = BTreeMap::new();
    let mut sub = Ds1Spec::new(18, 1, 1);
    sub.tag_type = 1;
    sub.groups = vec![[0, 0, 1, 1, 1]];
    files.insert(
        "DATA\\GLOBAL\\TILES\\A\\p.ds1".to_owned(),
        Ds1Spec::new(18, 8, 8).bytes(),
    );
    files.insert("DATA\\GLOBAL\\TILES\\A\\s.ds1".to_owned(), sub.bytes());
    files.insert("DATA\\GLOBAL\\TILES\\A\\f.dt1".to_owned(), floor_dt1());
    for p in FIXED_LIBRARY
        .into_iter()
        .chain(ACT_EDGE_TILE.iter().flatten().map(|&(p, _)| p))
    {
        files.insert(String::from_utf8(p.to_vec()).unwrap(), floor_dt1());
    }
    Disk {
        files,
        asked: Vec::new(),
    }
}

fn drlg_data(lvltype_files: &[&[u8]]) -> DrlgData {
    DrlgData {
        lvltypes: vec![lvltype_files.iter().map(|f| f.to_vec()).collect()],
        ..DrlgData::default()
    }
}

#[test]
fn world_files_load_every_named_file_once_by_its_table_string() {
    let mut disk = new_disk();
    let pd = preset_data([b"A/p.ds1", b"0", b"A/p.ds1", b"", b"", b""]);
    let od = outdoor_data(&[b"A/s.ds1", b"A/s.ds1"]);
    let dd = drlg_data(&[b"A/f.dt1", b"0", b"A/f.dt1"]);
    let w = WorldFiles::load(&dd, &pd, &od, |n| disk.read(n)).unwrap();
    // Keyed by the strings the sim asks with.
    assert_eq!(w.ds1.ds1(b"A/p.ds1").unwrap().width, 8);
    assert!(w.ds1.ds1(b"0").is_none());
    assert_eq!(w.sub_file(b"A/s.ds1").unwrap().groups.len(), 1);
    assert_eq!(w.dt1.dt1(b"A/f.dt1").unwrap().len(), 1);
    for p in FIXED_LIBRARY {
        assert_eq!(w.dt1.dt1(p).unwrap()[0].rarity, 1);
    }
    // The acts' base libraries (acts I–III).
    for &(p, _) in ACT_EDGE_TILE.iter().flatten() {
        assert_eq!(w.dt1.dt1(p).unwrap().len(), 1);
    }
    // Each file read once, placeholders never.
    let mut asked = disk.asked.clone();
    asked.sort();
    asked.dedup();
    assert_eq!(asked.len(), disk.asked.len());
    assert_eq!(disk.asked.len(), 3 + FIXED_LIBRARY.len() + 3);
    // An install without the base libraries still loads (the client asks
    // for them only in a level that draws edge floors).
    let mut disk = new_disk();
    for &(p, _) in ACT_EDGE_TILE.iter().flatten() {
        disk.files.remove(&String::from_utf8(p.to_vec()).unwrap());
    }
    let w = WorldFiles::load(&dd, &pd, &od, |n| disk.read(n)).unwrap();
    assert!(w.dt1.dt1(ACT_EDGE_TILE[0].unwrap().0).is_none());
}

#[test]
fn world_files_a_missing_named_file_is_an_error() {
    let mut disk = new_disk();
    let pd = preset_data([b"A/none.ds1", b"", b"", b"", b"", b""]);
    let err =
        WorldFiles::load(&drlg_data(&[]), &pd, &outdoor_data(&[]), |n| disk.read(n)).unwrap_err();
    match err {
        WorldDataError::Read { path, .. } => {
            assert_eq!(path, "DATA\\GLOBAL\\TILES\\A\\none.ds1")
        }
        other => panic!("{other:?}"),
    }
    // A broken DT1 is a parse error.
    let mut disk = new_disk();
    disk.files
        .insert("DATA\\GLOBAL\\TILES\\A\\f.dt1".into(), vec![7, 0, 0]);
    let err = WorldFiles::load(
        &drlg_data(&[b"A/f.dt1"]),
        &preset_data(Default::default()),
        &outdoor_data(&[]),
        |n| disk.read(n),
    )
    .unwrap_err();
    assert!(matches!(err, WorldDataError::Parse { .. }), "{err:?}");
}

// ---- hireling tables ------------------------------------------------------------

fn bin(
    name: &str,
    size: usize,
    count: usize,
    fill: impl Fn(usize, &mut [u8]),
) -> d2_data::bin::BinTable {
    let mut records = vec![0u8; size * count];
    for (i, r) in records.chunks_mut(size).enumerate() {
        fill(i, r);
    }
    d2_data::bin::BinTable {
        name: name.into(),
        source: "patch_d2.mpq".into(),
        count,
        record_size: size,
        records,
    }
}

// The hireling tables load from `hireling`, `pettype` and `experience`.
#[test]
fn hireling_tables_load_from_the_three_tables() {
    use d2_data::tables::{Experience, Hireling, Pettype, Record};
    // One hireling row; pettype row 7 with warp (bit 0 at +4) and
    // basemax 1 (u16 +10); experience MaxLvl 2 (Amazon of row 0) and
    // ExpRatio (+28) 1024 (the MaxLvl row) / 1100 / 1000 / 900 (levels
    // 0–2).
    let hireling = bin(Hireling::TABLE, Hireling::SIZE, 1, |_, r| {
        r[0..2].copy_from_slice(&100u16.to_le_bytes());
    });
    let pettype = bin(Pettype::TABLE, Pettype::SIZE, 8, |i, r| {
        if i == 7 {
            r[4] = 1;
            r[10..12].copy_from_slice(&1u16.to_le_bytes());
        }
    });
    let experience = bin(Experience::TABLE, Experience::SIZE, 4, |i, r| {
        if i == 0 {
            r[0..4].copy_from_slice(&2u32.to_le_bytes());
        }
        let ratio = [1024u32, 1100, 1000, 900][i];
        r[28..32].copy_from_slice(&ratio.to_le_bytes());
    });
    let all = [hireling, pettype, experience];
    let find = |n: &str| all.iter().find(|t| t.name == n);
    let t = tables::hireling_tables_by(find).expect("loads");
    assert_eq!(t.max_level, 2);
    assert_eq!(t.pet_flags, HirelingTables::WARP);
    assert_eq!(t.pet_basemax, 1);
    assert_eq!(t.rows.rows.len(), 1);
    assert_eq!(t.exp_ratios.ratio(2), 900);
    assert_eq!(t.exp_ratios.ratio(0), 1024);
    // A missing table is an error naming it.
    let two = &all[..2];
    let e = tables::hireling_tables_by(|n| two.iter().find(|t| t.name == n)).unwrap_err();
    assert!(e.to_string().contains("experience"), "{e}");
}
