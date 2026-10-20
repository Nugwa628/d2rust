//! The preset rules' test vectors on synthetic inputs.

use std::collections::BTreeMap;

use d2_data::fixup::maps::ActRanges;

use super::data::PRESET_TABLES_TSV;
use super::ds1::ORIENTATION_REMAP;
use super::*;
use crate::drlg::{room_flags, Drlg, LevelDef, NoLevelTypes, SpawnTile};
use crate::rng::Seed;

const START: u32 = 4014346869;
const M: u32 = 734;
const S: u32 = 66;

#[derive(Default)]
pub(super) struct Files(pub(super) BTreeMap<Vec<u8>, Ds1Input>);

impl Ds1Source for Files {
    fn ds1(&self, path: &[u8]) -> Option<&Ds1Input> {
        self.0.get(path)
    }
}

fn drlg_data() -> DrlgData {
    let mut d = DrlgData {
        levels: vec![LevelDef::default(); 150],
        object_subclass: vec![0; 600],
        ..DrlgData::default()
    };
    for l in &mut d.levels {
        l.warp = [-1; 8];
    }
    d.object_subclass[119] = 0x40; // a waypoint object class
    d
}

pub(super) fn preset_data() -> PresetData {
    let defs = (0..302)
        .map(|i| PresetDef {
            def: i,
            populate: 1,
            ..PresetDef::default()
        })
        .collect();
    PresetData {
        defs,
        monpreset_acts: ActRanges::default(),
        monpreset: Vec::new(),
        monstats_count: M,
        superuniques_count: S,
        hdm_item: 77,
        tables: PresetTables::spec().unwrap(),
    }
}

/// A v18 DS1 of `w × h` tiles: one wall layer, one floor layer, all 0.
pub(super) fn ds1(w: u32, h: u32) -> Ds1Input {
    let n = ((w + 1) * (h + 1)) as usize;
    Ds1Input {
        version: 18,
        width: w,
        height: h,
        walls: vec![vec![0; n]],
        orientations: vec![vec![0; n]],
        floors: vec![vec![0; n]],
        shadow: vec![0; n],
        ..Ds1Input::default()
    }
}

pub(super) fn cell(main: u32, sub: u32) -> u32 {
    (main << 20) | (sub << 8)
}

fn obj(kind: u32, id: u32, x: u32, y: u32) -> Ds1ObjectInput {
    Ds1ObjectInput {
        kind,
        id,
        x,
        y,
        flags: 0,
    }
}

pub(super) struct World {
    pub(super) dd: DrlgData,
    pub(super) pd: PresetData,
    pub(super) files: Files,
    pub(super) cache: Ds1Cache,
    pub(super) drlg: Drlg,
    pub(super) p: Presets,
}

impl World {
    pub(super) fn new() -> Self {
        let dd = drlg_data();
        let mut drlg = Drlg::create(0, 1, 0, 0, false, &dd, &mut NoLevelTypes).unwrap();
        drlg.start_seed = START;
        Self {
            dd,
            pd: preset_data(),
            files: Files::default(),
            cache: Ds1Cache::default(),
            drlg,
            p: Presets::default(),
        }
    }

    /// Makes level `id` a DrlgType 2 level of `w × h` claimed by `def`.
    pub(super) fn level(&mut self, id: u32, def: u32, w: i32, h: i32) -> LevelIdx {
        let l = &mut self.dd.levels[id as usize];
        l.drlg_type = 2;
        l.size = [(w, h); 3];
        self.pd.defs[def as usize].level_id = id;
        self.drlg
            .get_or_alloc_level(&self.dd, &mut NoLevelTypes, id)
            .unwrap()
    }

    pub(super) fn file(&mut self, def: u32, path: &[u8], f: Ds1Input) {
        let d = &mut self.pd.defs[def as usize];
        d.files = 1;
        d.file[0] = path.to_vec();
        self.files.0.insert(path.to_vec(), f);
    }

    pub(super) fn run<T>(
        &mut self,
        f: impl FnOnce(&mut Presets, &mut Drlg, &mut PresetCtx<'_>) -> T,
    ) -> T {
        let mut ctx = PresetCtx {
            drlg: &self.dd,
            data: &self.pd,
            source: &self.files,
            cache: &mut self.cache,
        };
        f(&mut self.p, &mut self.drlg, &mut ctx)
    }

    pub(super) fn init(&mut self, l: LevelIdx) {
        self.run(|p, d, c| p.init_level(d, c, l)).unwrap();
    }

    pub(super) fn generate(&mut self, l: LevelIdx) -> Result<Option<DrlgRoomId>, PresetError> {
        let id = self.drlg.level(l).id;
        self.drlg.level_mut(l).seed = Seed::init_low(START.wrapping_add(id));
        self.run(|p, d, c| p.generate(d, c, l))
    }
}

// ---- tables ---------------------------------------------------------------

#[test]
fn tables_tsv_counts_and_lookups() {
    let t = PresetTables::spec().unwrap();
    let n: usize = t
        .objpreset
        .iter()
        .map(|a| a.iter().filter(|&&c| c != 0).count())
        .sum();
    assert_eq!(n, 580);
    let counts: Vec<u32> = (0..5).map(|a| t.objpreset_count(a)).collect();
    assert_eq!(counts, [113, 135, 116, 66, 150]);
    assert_eq!(t.objpreset_count(5), 0);
    assert_eq!(t.objpreset[0][0], 12);
    assert_eq!(t.doors.len(), 37);
    assert_eq!(t.doors.iter().map(|(_, r)| r.len()).sum::<usize>(), 121);
    let r = t.door(28, 7, 0, true).unwrap();
    assert_eq!((r.unit_type, r.class, r.dx, r.dy), (2, 14, 5, 0));
    assert_eq!(t.door(28, 7, 0, false).unwrap().class, 13);
    assert!(t.door(28, 7, 1, false).is_none());
    assert!(t.door(1, 7, 0, false).is_none());
}

#[test]
fn tables_tsv_check_catches_perturbations() {
    let good = PRESET_TABLES_TSV;
    let bad = [
        good.replacen("table\t", "tabel\t", 1),
        good.replacen("objpreset\t0\t1\t37", "objpreset\t0\t0\t37", 1),
        good.replacen("objpreset\t0\t1\t37", "objpreset\t5\t1\t37", 1),
        good.replacen("objpreset\t0\t1\t37\t", "objpreset\t0\t1\t37\t1", 1),
        good.replacen("door\t28\t7\t0\t1", "door\t28\t7\t0\t2", 1),
        good.replacen("door\t28\t7\t0\t1", "door\t28\tx\t0\t1", 1),
        format!("{good}door\t28\t1\t1\t1\t2\t1\t0\t0\n"),
        format!("{good}other\t0\t0\t0\t\t\t\t\t\n"),
    ];
    for (i, b) in bad.iter().enumerate() {
        assert!(PresetTables::from_tsv(b).is_err(), "perturbation {i}");
    }
    // A changed value parses and changes exactly that entry.
    let t = PresetTables::from_tsv(&good.replacen("objpreset\t0\t1\t37", "objpreset\t0\t1\t38", 1))
        .unwrap();
    assert_eq!(t.objpreset[0][1], 38);
    let mut want = PresetTables::spec().unwrap();
    want.objpreset[0][1] = 38;
    assert_eq!(t, want);
}

// ---- file choice -------------------------------------------------

#[test]
fn row_lookup_by_level_and_index() {
    let mut pd = preset_data();
    pd.defs[5].level_id = 9;
    pd.defs[7].level_id = 9;
    assert_eq!(pd.def_for_level(9), Some(5));
    assert_eq!(pd.def_for_level(10), None);
    assert_eq!(pd.def(301).unwrap().def, 301);
    assert_eq!(pd.def(302), Err(PresetError::UnknownDef(302)));
}

#[test]
fn level_init_file_draws() {
    let mut w = World::new();
    // Level 26: Files 1 → one roll(1) → 0.
    let l26 = w.level(26, 165, 8, 8);
    w.pd.defs[165].files = 1;
    w.init(l26);
    assert_eq!(w.p.info(l26).unwrap().direction, 0);
    let mut s = Seed::new(4014346895, 666);
    s.step();
    assert_eq!(w.drlg.level(l26).seed, s);
    // Level 27: Files 3, lo' 2260552554 → 0.
    let l27 = w.level(27, 166, 8, 8);
    w.pd.defs[166].files = 3;
    w.init(l27);
    let mut s = Seed::new(4014346896, 666);
    assert_eq!(s.step(), 2260552554);
    assert_eq!(w.p.info(l27).unwrap().direction, 0);
    // Levels 90 / 124 / 94 (derived vectors).
    for (id, def, files, lo, want) in [
        (90u32, 200u32, 6, 3449482213u32, 1),
        (124, 201, 4, 4227474959, 3),
        (94, 202, 3, 2025139961, 2),
    ] {
        let l = w.level(id, def, 8, 8);
        w.pd.defs[def as usize].files = files;
        w.init(l);
        assert_eq!(w.drlg.level(l).seed.lo, lo, "level {id}");
        assert_eq!(w.p.info(l).unwrap().direction, want, "level {id}");
    }
    // Level 1: Files 0 → −1, no draw.
    let l1 = w.level(1, 1, 8, 8);
    w.init(l1);
    assert_eq!(w.p.info(l1).unwrap().direction, -1);
    assert_eq!(w.drlg.level(l1).seed, Seed::init_low(START + 1));
    // No row → fatal.
    let l2 = w
        .drlg
        .get_or_alloc_level(&w.dd, &mut NoLevelTypes, 2)
        .unwrap();
    assert_eq!(
        w.run(|p, d, c| p.init_level(d, c, l2)),
        Err(PresetError::NoPresetForLevel(2))
    );
}

#[test]
fn town_generation_35_rooms_row_major() {
    let mut w = World::new();
    let l = w.level(1, 1, 56, 40);
    w.pd.defs[1].scan = 1;
    w.file(1, b"town.ds1", ds1(56, 40));
    w.pd.defs[1].files = 0;
    w.init(l);
    w.p.set_direction(l, 0).unwrap(); // the act layout's town orientation
    let last = w.generate(l).unwrap().unwrap();
    let rooms = w.drlg.level_rooms(l);
    assert_eq!(rooms.len(), 35);
    // Head insert: list order is reverse allocation; `last` is the head.
    assert_eq!(rooms[0], last);
    let mut alloc: Vec<_> = rooms.clone();
    alloc.reverse();
    for (k, &r) in alloc.iter().enumerate() {
        let k = k as i32;
        assert_eq!(
            w.drlg.room(r).rect,
            TileRect::new(8 * (k % 7), 8 * (k / 7), 8, 8)
        );
    }
    // Files 0: no file draw; the first draw is room allocation.
    let mut s = Seed::init_low(START + 1);
    let first = s.step();
    assert_eq!(first, 2928842600);
    assert_eq!(w.drlg.room(alloc[0]).seed.lo, {
        let mut r = Seed::init_low(first);
        r.step();
        r.lo
    });
    for _ in 1..35 {
        s.step();
    }
    assert_eq!(w.drlg.level(l).seed, s);
    let m = w.p.level_maps(l)[0];
    assert_eq!(w.p.map(m).unwrap().picked_file, 0);
    assert_eq!(w.p.info(l).unwrap().map, Some(m));
    let f = w.drlg.room(alloc[0]).flags;
    assert_eq!(f & room_flags::NO_POPULATION, 0);
}

#[test]
fn map_alloc_redraws_and_syncs_direction() {
    let mut w = World::new();
    let l = w.level(90, 200, 16, 8);
    w.file(200, b"a.ds1", ds1(16, 8));
    w.pd.defs[200].files = 6;
    for i in 1..6 {
        w.pd.defs[200].file[i] = b"a.ds1".to_vec();
    }
    w.init(l);
    assert_eq!(w.p.info(l).unwrap().direction, 1);
    w.generate(l).unwrap();
    let m = w.p.level_maps(l)[0];
    assert_eq!(w.p.map(m).unwrap().picked_file, 1);
    // Direction overwritten (act layout) → the map's file follows it.
    let l2 = w.level(91, 201, 8, 8);
    w.file(201, b"b.ds1", ds1(8, 8));
    w.pd.defs[201].files = 6;
    for i in 1..6 {
        w.pd.defs[201].file[i] = b"b.ds1".to_vec();
    }
    w.init(l2);
    w.p.set_direction(l2, 5).unwrap();
    w.generate(l2).unwrap();
    let m2 = w.p.level_maps(l2)[0];
    assert_eq!(w.p.map(m2).unwrap().picked_file, 5);
    // SizeX / SizeY override the rectangle only when both are set.
    w.pd.defs[10].size_x = 24;
    w.pd.defs[10].size_y = 16;
    w.pd.defs[11].size_x = 24;
    let (a, b) = w.run(|p, d, c| {
        let a = p.alloc_map(d, c, l, 10, TileRect::new(3, 4, 8, 8)).unwrap();
        let b = p.alloc_map(d, c, l, 11, TileRect::new(3, 4, 8, 8)).unwrap();
        (a, b)
    });
    assert_eq!(w.p.map(a).unwrap().rect, TileRect::new(3, 4, 24, 16));
    assert_eq!(w.p.map(b).unwrap().rect, TileRect::new(3, 4, 8, 8));
    // Head insert on the level's map list.
    assert_eq!(&w.p.level_maps(l)[..2], &[b, a]);
    assert!(w.p.map(a).unwrap().hardcoded_pending);
}

#[test]
fn room_split_84_and_8() {
    let mut w = World::new();
    let l = w.level(124, 201, 84, 84);
    w.file(201, b"n.ds1", ds1(84, 84));
    w.init(l);
    w.generate(l).unwrap();
    let rooms = w.drlg.level_rooms(l);
    assert_eq!(rooms.len(), 121);
    let head = w.drlg.room(rooms[0]).rect;
    assert_eq!(head, TileRect::new(80, 80, 4, 4));
    let l20 = w.level(20, 202, 8, 8);
    w.file(202, b"t.ds1", ds1(8, 8));
    w.init(l20);
    w.generate(l20).unwrap();
    assert_eq!(w.drlg.room_count(l20), 1);
    // Single-room mode: one room over the map, preset room flag 1.
    let m = w.run(|p, d, c| p.alloc_map(d, c, l20, 202, TileRect::new(0, 0, 20, 12)));
    let r = w
        .run(|p, d, c| p.build_area(d, c, l20, m.unwrap(), 0x4, true))
        .unwrap()
        .unwrap();
    assert_eq!(w.drlg.room(r).rect, TileRect::new(0, 0, 20, 12));
    assert!(w.p.room(r).unwrap().single);
    assert_eq!(w.drlg.room(r).flags & 0x4, 0x4);
}

#[test]
fn reset_releases_maps_and_ds1() {
    let mut w = World::new();
    let l = w.level(20, 202, 8, 8);
    w.file(202, b"t.ds1", ds1(8, 8));
    w.pd.defs[202].scan = 1;
    w.init(l);
    w.generate(l).unwrap();
    assert_eq!(w.cache.refs(b"t.ds1"), 1);
    let r = w.drlg.level_rooms(l)[0];
    w.run(|p, _, c| p.reset_level(c, l, true));
    assert_eq!(w.cache.refs(b"t.ds1"), 0);
    assert!(w.p.level_maps(l).is_empty());
    assert!(w.p.room(r).is_err());
    assert_eq!(w.p.info(l).unwrap().map, None);
    w.run(|p, _, c| p.reset_level(c, l, false));
    assert!(w.p.info(l).is_none());
}

// ---- DS1 loading ---------------------------------------------------------

#[test]
fn cache_shares_by_exact_path() {
    let pd = preset_data();
    let mut files = Files::default();
    files.0.insert(b"A.ds1".to_vec(), ds1(8, 8));
    let mut c = Ds1Cache::default();
    let a = c.load(b"A.ds1", &files, &pd).unwrap();
    let b = c.load(b"A.ds1", &files, &pd).unwrap();
    assert_eq!(a, b);
    assert_eq!(c.refs(b"A.ds1"), 2);
    assert!(matches!(
        c.load(b"a.ds1", &files, &pd),
        Err(PresetError::MissingDs1(_))
    ));
    c.release(a);
    assert_eq!(c.refs(b"A.ds1"), 1);
    c.file_mut(a).walls[0][0] = 5;
    c.release(b);
    assert_eq!(c.refs(b"A.ds1"), 0);
    // Reloaded fresh after the last release.
    let a = c.load(b"A.ds1", &files, &pd).unwrap();
    assert_eq!(c.file(a).walls[0][0], 0);
}

#[test]
fn parser_header_rules() {
    let pd = preset_data();
    let mut f = ds1(2, 2);
    f.act = 7;
    f.tag_type = 2;
    let p = Ds1File::from_input(&f, &pd).unwrap();
    assert_eq!((p.act, p.tag_type), (4, 2));
    f.act = -3;
    assert_eq!(Ds1File::from_input(&f, &pd).unwrap().act, -3);
    f.version = 9;
    f.act = 3;
    let p = Ds1File::from_input(&f, &pd).unwrap();
    assert_eq!((p.act, p.tag_type), (3, 0));
    f.version = 7;
    assert_eq!(Ds1File::from_input(&f, &pd).unwrap().act, 0);
    // v < 7: orientation remap through the 42-entry table.
    f.version = 6;
    f.orientations[0][0] = 26;
    f.orientations[0][1] = 41;
    f.orientations[0][2] = 5;
    let p = Ds1File::from_input(&f, &pd).unwrap();
    assert_eq!(&p.orientations[0][..3], &[0, 20, 3]);
    assert_eq!(ORIENTATION_REMAP[29], 5);
    f.orientations[0][0] = 42;
    assert!(Ds1File::from_input(&f, &pd).is_err());
    // v < 4: floor count 0.
    f.orientations[0][0] = 0;
    f.version = 3;
    assert!(Ds1File::from_input(&f, &pd).unwrap().floors.is_empty());
    // Strict input: a short layer is an error.
    f.shadow.pop();
    assert!(Ds1File::from_input(&f, &pd).is_err());
}

#[test]
fn object_conversion() {
    let mut pd = preset_data();
    pd.monpreset_acts.first[4] = Some(10);
    pd.monpreset_acts.count[4] = 4;
    pd.monpreset = vec![MonPresetRow::default(); 20];
    pd.monpreset[10] = MonPresetRow {
        kind: 1,
        place: 537,
    };
    pd.monpreset[11] = MonPresetRow { kind: 0, place: 33 };
    pd.monpreset[12] = MonPresetRow { kind: 2, place: 5 };
    pd.monpreset[13] = MonPresetRow { kind: 3, place: 5 };
    let conv = |v: u32, act: i32, o: Ds1ObjectInput| {
        let mut f = ds1(1, 1);
        f.version = v;
        f.act = act;
        f.objects = vec![o];
        Ds1File::from_input(&f, &pd).map(|p| p.units)
    };
    let one = |v, act, o| conv(v, act, o).unwrap().pop();
    // Monpreset kind 1 place 537 in act 4 → object 476, mode 0.
    let u = one(18, 4, obj(1, 0, 3, 4)).unwrap();
    assert_eq!((u.unit_type, u.class, u.mode, u.x, u.y), (2, 476, 0, 3, 4));
    assert_eq!(
        one(18, 4, obj(1, 1, 0, 0)).unwrap().class,
        (33 + S + M) as i32
    );
    assert_eq!(one(18, 4, obj(1, 2, 0, 0)).unwrap().class, (5 + M) as i32);
    assert!(one(18, 4, obj(1, 3, 0, 0)).is_none()); // other kind → −1
                                                    // Beyond the range: id as stored; act conversions.
    let u = one(18, 4, obj(1, 514, 0, 0)).unwrap();
    assert_eq!((u.unit_type, u.class, u.mode), (2, 461, 0));
    assert_eq!(one(18, 2, obj(1, 297, 0, 0)).unwrap().class, 382);
    assert_eq!(one(18, 2, obj(1, 366, 0, 0)).unwrap().class, 404);
    let u = one(18, 1, obj(1, 297, 0, 0)).unwrap();
    assert_eq!((u.unit_type, u.class, u.mode), (1, 297, 1));
    // Monsters dropped for v ≤ 4.
    assert!(one(4, 0, obj(1, 5, 0, 0)).is_none());
    // Objects: table (v ≥ 6), id − 150, v ≤ 5 as stored, 573 dropped.
    assert_eq!(one(18, 0, obj(2, 1, 0, 0)).unwrap().class, 37);
    assert_eq!(one(18, 0, obj(2, 149, 0, 0)).unwrap().class, 0);
    assert_eq!(one(18, 0, obj(2, 731, 0, 0)).unwrap().class, 581);
    assert_eq!(one(5, 0, obj(2, 1, 0, 0)).unwrap().class, 1);
    assert!(one(5, 0, obj(2, 573, 0, 0)).is_none());
    // Items: hdm for v ≥ 5 (mode 3), stored for v ≤ 4, id ≥ 1 reads
    // beyond the table.
    let u = one(18, 0, obj(4, 0, 0, 0)).unwrap();
    assert_eq!((u.class, u.mode), (77, 3));
    assert_eq!(one(4, 0, obj(4, 9, 0, 0)).unwrap().class, 9);
    assert_eq!(
        conv(18, 0, obj(4, 1, 0, 0)),
        Err(PresetError::ItemCodeBeyondTable(1))
    );
    // Other types: mode 0, id as stored; v < 2 has no objects.
    let u = one(18, 0, obj(3, 9, 0, 0)).unwrap();
    assert_eq!((u.unit_type, u.class, u.mode), (3, 9, 0));
    assert!(one(1, 0, obj(2, 1, 0, 0)).is_none());
    // Flags kept; reverse file order.
    let mut f = ds1(1, 1);
    let mut a = obj(2, 1, 1, 1);
    a.flags = 1;
    f.objects = vec![a, obj(2, 2, 2, 2)];
    let p = Ds1File::from_input(&f, &pd).unwrap();
    assert_eq!((p.units[0].x, p.units[1].x, p.units[1].flags), (2, 1, 1));
}

#[test]
fn paths_attach_last_wins() {
    let pd = preset_data();
    let mut f = ds1(1, 1);
    f.version = 14;
    f.objects = vec![obj(1, 5, 10, 10)];
    f.paths = vec![
        Ds1PathInput {
            x: 10,
            y: 10,
            points: vec![(1, 1, 7)],
        },
        Ds1PathInput {
            x: 10,
            y: 10,
            points: vec![(2, 2, 7), (3, 3, 9)],
        },
        Ds1PathInput {
            x: 99,
            y: 10,
            points: vec![(4, 4, 7)],
        },
    ];
    let p = Ds1File::from_input(&f, &pd).unwrap();
    let path = p.units[0].path.as_ref().unwrap();
    assert_eq!(
        path,
        &vec![
            PathPoint {
                action: 1,
                x: 2,
                y: 2
            },
            PathPoint {
                action: 1,
                x: 3,
                y: 3
            }
        ]
    );
    f.version = 15;
    let p = Ds1File::from_input(&f, &pd).unwrap();
    assert_eq!(p.units[0].path.as_ref().unwrap()[1].action, 9);
    f.version = 13;
    assert!(Ds1File::from_input(&f, &pd).unwrap().units[0]
        .path
        .is_none());
}

// ---- scan ----------------------------------------------------------------

/// A level `id` with a scanned 32×16 DS1 at map origin (40, 24).
fn scanned(f: Ds1Input, scan: u32, pops: u32) -> (World, LevelIdx) {
    let mut w = World::new();
    let l = w.level(33, 210, f.width as i32, f.height as i32);
    w.drlg.level_mut(l).rect = TileRect::new(40, 24, f.width as i32, f.height as i32);
    w.pd.defs[210].scan = scan;
    w.pd.defs[210].pops = pops;
    w.file(210, b"s.ds1", f);
    w.init(l);
    w.generate(l).unwrap();
    (w, l)
}

fn set(f: &mut Ds1Input, x: u32, y: u32, o: u32, v: u32) {
    let i = (y * (f.width + 1) + x) as usize;
    f.orientations[0][i] = o;
    f.walls[0][i] = v;
}

fn room_at(w: &World, l: LevelIdx, x: i32, y: i32) -> DrlgRoomId {
    *w.drlg
        .level_rooms(l)
        .iter()
        .find(|&&r| w.drlg.room(r).rect.contains(x, y))
        .unwrap()
}

#[test]
fn scan_tile_info_in_order() {
    let mut f = ds1(32, 16);
    set(&mut f, 26, 7, 10, cell(30, 0));
    set(&mut f, 28, 7, 11, cell(32, 3));
    set(&mut f, 30, 14, 10, cell(33, 0));
    set(&mut f, 5, 5, 10, cell(31, 2));
    set(&mut f, 6, 5, 9, cell(30, 9)); // orientation 9: not scanned
    set(&mut f, 32, 5, 10, cell(30, 9)); // edge column: not scanned
    let (w, l) = scanned(f, 1, 0);
    let (x, y) = (40, 24);
    assert_eq!(
        w.drlg.level(l).spawn_tiles,
        vec![
            SpawnTile {
                x: x + 5,
                y: y + 5,
                index: 7
            },
            SpawnTile {
                x: x + 26,
                y: y + 7,
                index: 0
            },
            SpawnTile {
                x: x + 28,
                y: y + 7,
                index: 10
            },
            SpawnTile {
                x: x + 30,
                y: y + 14,
                index: 11
            },
        ]
    );
}

#[test]
fn pops_rectangles() {
    let mut f = ds1(32, 16);
    set(&mut f, 2, 2, 10, cell(8, 8));
    set(&mut f, 6, 11, 10, cell(8, 8));
    set(&mut f, 11, 6, 11, cell(9, 9));
    set(&mut f, 20, 3, 10, cell(29, 1));
    set(&mut f, 21, 3, 10, cell(30, 1)); // tile info, but Scan = 0
    let (w, l) = scanned(f, 0, 3);
    let m = w.p.level_maps(l)[0];
    let pops = &w.p.map(m).unwrap().pops;
    let got: Vec<_> = pops.iter().map(|p| (p.group, p.sub, p.rect)).collect();
    assert_eq!(
        got,
        vec![
            (1, 8, TileRect::new(42, 26, 5, 10)),
            // Seen once: from the DS1 origin (reproduced bug).
            (6, 1, TileRect::new(40, 24, 21, 4)),
            (1, 9, TileRect::new(40, 24, 12, 7)),
        ]
    );
    assert!(w.drlg.level(l).spawn_tiles.is_empty());
}

#[test]
fn scan_room_flags() {
    let mut f = ds1(32, 16);
    set(&mut f, 9, 1, 10, cell(3, 0)); // warp style 3 → room (8,0) 0x80
    set(&mut f, 17, 1, 10, cell(2, 5)); // sub 5, no bit 31 → nothing
    set(&mut f, 25, 1, 11, cell(1, 5) | 0x8000_0000); // bit 31 → 0x20
    set(&mut f, 2, 9, 10, cell(8, 0)); // style 8: not a warp
    f.objects = vec![obj(2, 119 + 150, 45, 3), obj(2, 118 + 150, 5, 45)];
    let mut w = World::new();
    w.dd.levels[33].vis = [0, 5, 0, 0, 6, 0, 0, 0];
    w.dd.levels[33].warp = [-1, 2, -1, -1, -1, -1, -1, -1];
    w.dd.levels[5].warp = [-1; 8];
    let l = w.level(33, 210, 32, 16);
    w.pd.defs[210].scan = 1;
    w.pd.defs[210].outdoors = 1;
    w.pd.defs[210].populate = 0;
    w.pd.defs[210].dt1_mask = 0x3;
    w.file(210, b"s.ds1", f);
    w.init(l);
    w.generate(l).unwrap();
    // Vis slot 4 set with warp −1 → 0x100; slot 1 has a warp.
    let base = room_flags::WARP_0 << 4 | room_flags::NO_LOS_DRAW | room_flags::NO_POPULATION;
    let flags = |x, y| w.drlg.room(room_at(&w, l, x, y)).flags;
    assert_eq!(flags(0, 0), base);
    assert_eq!(flags(8, 0), base | 0x80 | room_flags::ANY_WAYPOINT);
    assert_eq!(flags(16, 0), base);
    assert_eq!(flags(24, 0), base | 0x20);
    assert_eq!(flags(0, 8), base);
    assert_eq!(w.drlg.room(room_at(&w, l, 0, 0)).dt1_mask, 3);
}

#[test]
fn size_mismatch_fatal_only_at_build() {
    let mut w = World::new();
    let l = w.level(20, 202, 8, 8);
    w.file(202, b"t.ds1", ds1(9, 8));
    w.pd.defs[202].scan = 1;
    w.init(l);
    assert!(matches!(
        w.generate(l),
        Err(PresetError::SizeMismatch { .. })
    ));
    // Lazy load (Scan 0, Pops 0) does not check the size.
    let l2 = w.level(21, 203, 8, 8);
    w.file(203, b"u.ds1", ds1(9, 8));
    w.init(l2);
    w.generate(l2).unwrap();
    let r = w.drlg.level_rooms(l2)[0];
    w.run(|p, d, c| p.add_preset_units(d, c, r)).unwrap();
}

// ---- unit filter -----------------------------------------------------

fn filter_one(kind: u32, class: i32, seed: Seed) -> (bool, Seed) {
    let mut w = World::new();
    let l = w.level(20, 202, 8, 8);
    w.pd.defs[202].scan = 1;
    let mut f = ds1(8, 8);
    // DS1 id producing `class` (objects: id − 150; monsters: as stored).
    let id = if kind == 2 { class + 150 } else { class };
    f.objects = vec![obj(kind, id as u32, 1, 1)];
    w.file(202, b"t.ds1", f);
    w.init(l);
    let m = w
        .run(|p, d, c| p.alloc_map(d, c, l, 202, TileRect::new(0, 0, 8, 8)))
        .unwrap();
    let mut ctx = PresetCtx {
        drlg: &w.dd,
        data: &w.pd,
        source: &w.files,
        cache: &mut w.cache,
    };
    let mut s = seed;
    w.p.load_and_filter(&mut ctx, m, &mut s).unwrap();
    (!w.p.map(m).unwrap().units.is_empty(), s)
}

#[test]
fn unit_filter_draws() {
    let ms = (M + S) as i32;
    // lo' = 9 / 10 for monster 371: kept / skipped.
    assert!(filter_one(1, 371, Seed::new(0, 9)).0);
    assert!(!filter_one(1, 371, Seed::new(0, 10)).0);
    for c in [204, 205, 372] {
        assert!(filter_one(1, c, Seed::new(0, 3)).0);
        assert!(!filter_one(1, c, Seed::new(0, 4)).0);
    }
    assert!(filter_one(1, ms + 33, Seed::new(0, 1)).0);
    assert!(!filter_one(1, ms + 33, Seed::new(0, 4)).0);
    assert!(filter_one(1, ms + 34, Seed::new(0, 1)).0);
    assert!(!filter_one(1, ms + 34, Seed::new(0, 2)).0);
    assert!(filter_one(1, ms + 35, Seed::new(0, 4)).0);
    assert!(!filter_one(1, ms + 35, Seed::new(0, 5)).0);
    for c in [196, 261] {
        assert!(filter_one(2, c, Seed::new(0, 2)).0);
        assert!(!filter_one(2, c, Seed::new(0, 3)).0);
    }
    // Object 581: lo' & 3 = 0 → skipped, else kept.
    assert!(!filter_one(2, 581, Seed::new(0, 8)).0);
    assert!(filter_one(2, 581, Seed::new(0, 9)).0);
    // No draw for anything else (superuniques included).
    let (kept, s) = filter_one(1, M as i32 + 3, Seed::new(0, 9));
    assert!(kept);
    assert_eq!(s, Seed::new(0, 9));
    let (_, s) = filter_one(1, 371, Seed::new(0, 9));
    assert_eq!(s, {
        let mut t = Seed::new(0, 9);
        t.step();
        t
    });
}

#[test]
fn list_orders_file_map_room() {
    let mut w = World::new();
    let l = w.level(20, 202, 8, 8);
    w.drlg.level_mut(l).rect = TileRect::new(2, 3, 8, 8);
    w.pd.defs[202].scan = 1;
    let mut f = ds1(8, 8);
    // A, B, C in file order; one with a path.
    f.objects = vec![obj(2, 151, 1, 1), obj(2, 152, 2, 2), obj(2, 153, 3, 3)];
    f.paths = vec![Ds1PathInput {
        x: 1,
        y: 1,
        points: vec![(4, 4, 1)],
    }];
    w.file(202, b"t.ds1", f);
    w.init(l);
    w.generate(l).unwrap();
    let m = w.p.level_maps(l)[0];
    let classes = |u: &[PresetUnit]| u.iter().map(|u| u.class).collect::<Vec<_>>();
    let map = w.p.map(m).unwrap();
    assert_eq!(classes(&map.units), [1, 2, 3]);
    // Shifted by the map origin (sub-tiles), path points too.
    assert_eq!((map.units[0].x, map.units[0].y), (11, 16));
    assert_eq!(
        map.units[0].path.as_ref().unwrap()[0],
        PathPoint {
            action: 1,
            x: 14,
            y: 19
        }
    );
    let r = w.drlg.level_rooms(l)[0];
    w.run(|p, d, c| p.room_grids(d, c, r)).unwrap();
    assert_eq!(classes(w.p.room_units(r)), [3, 2, 1]);
    let u = &w.p.room_units(r)[2];
    // Room-relative position, absolute path.
    assert_eq!((u.x, u.y), (1, 1));
    assert_eq!(
        u.path.as_ref().unwrap()[0],
        PathPoint {
            action: 1,
            x: 14,
            y: 19
        }
    );
    assert!(w.p.map(m).unwrap().units.is_empty());
    let seam = w.p.preset_units(r);
    assert_eq!((seam[0].unit_type, seam[0].class, seam[0].x), (2, 3, 3));
}

// ---- first activation ------------------------------------------------

#[test]
fn lazy_load_filters_on_room_seed() {
    let mut w = World::new();
    let l = w.level(20, 202, 16, 8);
    let mut f = ds1(16, 8);
    f.objects = vec![obj(1, 371, 1, 1)];
    w.file(202, b"t.ds1", f);
    w.init(l);
    w.generate(l).unwrap();
    let m = w.p.level_maps(l)[0];
    assert_eq!(w.p.map(m).unwrap().ds1, None);
    let level_seed = w.drlg.level(l).seed;
    let r = w.drlg.level_rooms(l)[0];
    let room_seed = w.drlg.room(r).seed;
    w.run(|p, d, c| p.add_preset_units(d, c, r)).unwrap();
    let mut s = room_seed;
    let kept = s.step().is_multiple_of(3);
    assert_eq!(w.drlg.room(r).seed, s);
    assert_eq!(w.drlg.level(l).seed, level_seed);
    assert_eq!(w.p.map(m).unwrap().units.len(), usize::from(kept));
    assert_ne!(w.drlg.room(r).flags & room_flags::PRESET_UNITS_ADDED, 0);
    assert!(!w.p.map(m).unwrap().hardcoded_pending);
    // Second room: DS1 already loaded, no draw.
    let r2 = w.drlg.level_rooms(l)[1];
    let s2 = w.drlg.room(r2).seed;
    w.run(|p, d, c| p.add_preset_units(d, c, r2)).unwrap();
    assert_eq!(w.drlg.room(r2).seed, s2);
    assert_eq!(w.cache.refs(b"t.ds1"), 1);
}

#[test]
fn navi_unit() {
    let mut w = World::new();
    w.dd.levels[2].drlg_type = 3;
    let l = w
        .drlg
        .get_or_alloc_level(&w.dd, &mut NoLevelTypes, 2)
        .unwrap();
    for i in 0..4 {
        w.pd.defs[5].file[i] = b"b.ds1".to_vec();
    }
    w.files.0.insert(b"b.ds1".to_vec(), ds1(9, 7));
    let m = w
        .run(|p, d, c| p.alloc_map(d, c, l, 5, TileRect::new(10, 20, 9, 7)))
        .unwrap();
    // roll(Files) never gives 3 here; outdoor code sets it.
    w.p.map_mut(m).unwrap().picked_file = 3;
    let r = w
        .run(|p, d, c| p.build_area(d, c, l, m, 0, true))
        .unwrap()
        .unwrap();
    w.run(|p, d, c| p.add_preset_units(d, c, r)).unwrap();
    let u = &w.p.map(m).unwrap().units;
    assert_eq!(u.len(), 1);
    assert_eq!(
        (
            u[0].unit_type,
            u[0].class,
            u[0].mode,
            u[0].x,
            u[0].y,
            u[0].flags
        ),
        (1, 266, 1, (10 + 4) * 5, (20 + 3) * 5, 0)
    );
    // Picked file ≠ 3: nothing.
    let m2 = w
        .run(|p, d, c| p.alloc_map(d, c, l, 5, TileRect::new(10, 20, 9, 7)))
        .unwrap();
    w.p.map_mut(m2).unwrap().picked_file = 2;
    let r2 = w
        .run(|p, d, c| p.build_area(d, c, l, m2, 0, true))
        .unwrap()
        .unwrap();
    w.run(|p, d, c| p.add_preset_units(d, c, r2)).unwrap();
    assert!(w.p.map(m2).unwrap().units.is_empty());
}

#[test]
fn river_objects_client_only() {
    let build = |def: u32, client: bool| {
        let mut w = World::new();
        let l = w.level(20, def, 12, 8);
        if client {
            w.drlg.level_mut(l).flags |= 0x10;
        }
        let mut f = ds1(12, 8);
        // Row 0, column 2: style 2 sub 24 (river column).
        f.floors[0][2] = cell(2, 24);
        // Column 2, row 1: style 4 sub 8 → skip three rows.
        f.floors[0][13 + 2] = cell(4, 8);
        w.file(def, b"r.ds1", f);
        w.init(l);
        w.generate(l).unwrap();
        let rooms = w.drlg.level_rooms(l);
        let r = *rooms.last().unwrap();
        w.run(|p, d, c| p.add_preset_units(d, c, r)).unwrap();
        let m = w.p.level_maps(l)[0];
        w.p.map(m).unwrap().units.clone()
    };
    assert!(build(26, false).is_empty());
    let u = build(26, true);
    // sounds(0 + 2 + 1): y = 0 only (< 8·5); strip(2): rows 0, 1, 5, 6, 7.
    let mut want: Vec<(i32, i32, i32)> = vec![(65, 15, 0)];
    for row in [0, 1, 5, 6, 7] {
        let x0 = 2 * 5 - 5;
        for (k, c) in [40, 41, 41, 41, 42].into_iter().enumerate() {
            want.push((c, x0 + 5 * k as i32, row * 5));
        }
    }
    want.reverse();
    let got: Vec<_> = u.iter().map(|u| (u.class, u.x, u.y)).collect();
    assert_eq!(got, want);
    assert!(u
        .iter()
        .all(|u| u.flags == 1 && u.mode == 0 && u.unit_type == 2));
    // Def 27: sounds(map x), strip(−1) over every row (G column 0).
    let u = build(27, true);
    assert_eq!(u.len(), 1 + 8 * 5);
    assert_eq!((u.last().unwrap().class, u.last().unwrap().x), (65, 0));
    assert_eq!(u[0].x, -5 - 5 + 20);
}

// ---- grids, switches ---------------------------------------------

#[test]
fn room_grids_or_bits_into_shared_ds1() {
    let mut w = World::new();
    let l = w.level(20, 1, 16, 8);
    let mut f = ds1(16, 8);
    f.walls.push(vec![0; 17 * 9]);
    f.orientations.push(vec![0; 17 * 9]);
    f.floors.push(vec![0; 17 * 9]);
    f.walls[0][3] = cell(10, 23); // tombstone candidate? level 20: no
    w.file(1, b"t.ds1", f);
    w.pd.defs[1].kill_edge = 1;
    w.pd.defs[1].fill_blanks = 1;
    w.pd.defs[1].animate = 1;
    w.init(l);
    w.p.set_direction(l, 0).unwrap();
    w.generate(l).unwrap();
    let left = room_at(&w, l, 0, 0);
    let right = room_at(&w, l, 8, 0);
    w.run(|p, d, c| p.add_preset_units(d, c, left)).unwrap();
    let g = w.run(|p, d, c| p.room_grids(d, c, left)).unwrap();
    // Passes: floor 0 (FillBlanks), floor 1, wall 0, wall 1, shadow,
    // extra zero grid (Def 1).
    assert_eq!(g.passes.len(), 6);
    assert!(g.passes[0].fill_blanks && !g.passes[1].fill_blanks);
    assert!(g.passes[2].orientation.is_some() && g.passes[4].orientation.is_none());
    assert!(g.passes[5].cells.cells.iter().all(|&c| c == 0));
    assert_eq!((g.passes[0].cells.width, g.passes[0].cells.height), (9, 9));
    // Left room: right edge is not the map's.
    assert!(!g.kill_edge_x && g.kill_edge_y && g.animate);
    assert_eq!(g.anim_speed, 0);
    let wall0 = &g.passes[2].cells;
    assert_eq!(wall0.get(0, 0), 0x84);
    assert_eq!(wall0.get(3, 0), cell(10, 23) | 0x84);
    assert_eq!(wall0.get(4, 4), 0);
    assert_eq!(g.passes[3].cells.get(4, 4), 1 << 18);
    assert_eq!(g.passes[3].cells.get(0, 0), 1 << 18);
    assert_eq!(g.passes[1].cells.get(4, 4), 1 << 18);
    assert_eq!(g.passes[1].cells.get(8, 4), (1 << 18) | 0x84);
    assert_eq!(g.passes[4].cells.get(8, 8), 0x84);
    assert_eq!(g.passes[0].cells.get(4, 4), 0);
    w.run(|p, d, c| p.add_preset_units(d, c, right)).unwrap();
    let g2 = w.run(|p, d, c| p.room_grids(d, c, right)).unwrap();
    assert!(g2.kill_edge_x && g2.kill_edge_y);
    assert_eq!(w.p.tombstones(left), None);
    // The ORs live in the shared DS1: a single-room map of the same file
    // sees the column x = 8 edge bits inside its own grid.
    let m = w
        .run(|p, d, c| p.alloc_map(d, c, l, 1, TileRect::new(0, 0, 16, 8)))
        .unwrap();
    let r = w
        .run(|p, d, c| p.build_area(d, c, l, m, 0, true))
        .unwrap()
        .unwrap();
    w.run(|p, d, c| p.add_preset_units(d, c, r)).unwrap();
    assert_eq!(w.cache.refs(b"t.ds1"), 2);
    let g3 = w.run(|p, d, c| p.room_grids(d, c, r)).unwrap();
    assert_eq!(g3.passes[2].cells.get(8, 4), 0x84);
    assert_eq!(g3.passes[2].cells.get(12, 4), 0);
}

#[test]
fn tombstones_level_17() {
    let mut w = World::new();
    let l = w.level(17, 120, 8, 8);
    w.drlg.level_mut(l).rect = TileRect::new(4, 2, 8, 8);
    let mut f = ds1(8, 8);
    for (x, sub) in [(0u32, 23u32), (1, 27), (2, 28), (3, 25), (4, 24)] {
        f.walls[0][x as usize] = cell(10, sub);
    }
    for x in 0..4 {
        f.walls[0][9 + x] = cell(10, 26);
    }
    f.walls[0][8] = cell(10, 23); // column 8 = room w: not scanned
    w.file(120, b"g.ds1", f);
    w.init(l);
    w.generate(l).unwrap();
    let r = w.drlg.level_rooms(l)[0];
    w.run(|p, d, c| p.add_preset_units(d, c, r)).unwrap();
    w.run(|p, d, c| p.room_grids(d, c, r)).unwrap();
    let t = w.p.tombstones(r).unwrap();
    let at = |x: i32, y: i32| ((4 + x) * 5 + 2, (2 + y) * 5 + 2);
    assert_eq!(
        t,
        &[at(0, 0), at(1, 0), at(3, 0), at(4, 0), at(0, 1), at(1, 1)]
    );
}

#[test]
fn unit_transfer_bounds() {
    let mut w = World::new();
    let l = w.level(20, 202, 16, 16);
    w.pd.defs[202].scan = 1;
    let mut f = ds1(16, 16);
    f.objects = vec![
        obj(2, 151, 79, 39),
        obj(2, 152, 80, 39),
        obj(2, 153, 200, 200),
    ];
    w.file(202, b"t.ds1", f);
    w.init(l);
    w.generate(l).unwrap();
    let r = room_at(&w, l, 8, 0);
    assert_eq!(w.drlg.room(r).rect, TileRect::new(8, 0, 8, 8));
    w.run(|p, d, c| p.room_grids(d, c, r)).unwrap();
    let u = w.p.room_units(r);
    assert_eq!(u.len(), 1);
    assert_eq!((u[0].class, u[0].x, u[0].y), (1, 39, 39));
    for r in w.drlg.level_rooms(l) {
        w.run(|p, d, c| p.room_grids(d, c, r)).unwrap();
    }
    // The unit outside every room stays on the map list.
    let m = w.p.level_maps(l)[0];
    let left: Vec<_> = w.p.map(m).unwrap().units.iter().map(|u| u.class).collect();
    assert_eq!(left, [2, 3]);
}

// ---- doors ----------------------------------------------------------

#[test]
fn door_units() {
    let mut w = World::new();
    w.pd.monstats_count = 433;
    let l = w.level(28, 230, 16, 16);
    w.drlg.level_mut(l).rect = TileRect::new(100, 50, 16, 16);
    w.file(230, b"d.ds1", ds1(16, 16));
    w.init(l);
    w.generate(l).unwrap();
    let r = room_at(&w, l, 108, 58);
    let ctx_door = |w: &mut World, wx, wy, c, o| {
        w.run(|p, d, ctx| p.door_unit(d, ctx, r, wx, wy, c, o))
            .unwrap()
    };
    // Level 28, main 7 sub 0 right → object 14 at +(5, 0).
    assert_eq!(
        ctx_door(&mut w, 109, 59, cell(7, 0), 9),
        DoorOutcome::Placed
    );
    let u = &w.p.room_units(r)[0];
    assert_eq!(
        (u.unit_type, u.class, u.mode, u.x, u.y, u.flags),
        (2, 14, 0, 10, 5, 0)
    );
    // Left door → 13 at +(0, 5).
    assert_eq!(
        ctx_door(&mut w, 109, 59, cell(7, 0), 8),
        DoorOutcome::Placed
    );
    assert_eq!(
        (w.p.room_units(r)[0].class, w.p.room_units(r)[0].y),
        (13, 10)
    );
    // Right door at the room's right column: x = 40 + 5 → outside.
    assert_eq!(
        ctx_door(&mut w, 116, 59, cell(7, 0), 9),
        DoorOutcome::Outside
    );
    assert_eq!(ctx_door(&mut w, 109, 59, cell(7, 1), 9), DoorOutcome::NoRow);
    // Objects 91/92: roll(3) on the room seed.
    let t = PresetTables::spec().unwrap();
    let (lvl, row) = t
        .doors
        .iter()
        .find_map(|(lv, rows)| rows.iter().find(|r| r.class == 91).map(|r| (*lv, *r)))
        .unwrap();
    let mut w2 = World::new();
    let l2 = w2.level(lvl, 231, 16, 16);
    w2.file(231, b"e.ds1", ds1(16, 16));
    w2.init(l2);
    w2.generate(l2).unwrap();
    let r2 = room_at(&w2, l2, 0, 0);
    let mut s = w2.drlg.room(r2).seed;
    let want = if s.roll(3) == 0 {
        DoorOutcome::Rolled0
    } else {
        DoorOutcome::Placed
    };
    let c = cell(row.main, row.sub);
    let o = if row.right { 9 } else { 8 };
    let (wx, wy) = (2 - row.dx.min(0) / 5, 2 - row.dy.min(0) / 5);
    let got = w2
        .run(|p, d, ctx| p.door_unit(d, ctx, r2, wx, wy, c, o))
        .unwrap();
    assert_eq!(got, want);
    assert_eq!(w2.drlg.room(r2).seed, s);
    // Monster rows: class range-checked against monstats.
    let (mlvl, mrow) = t
        .doors
        .iter()
        .find_map(|(lv, rows)| rows.iter().find(|r| r.class == 435).map(|r| (*lv, *r)))
        .unwrap();
    let mut w3 = World::new();
    w3.pd.monstats_count = 435;
    let l3 = w3.level(mlvl, 232, 16, 16);
    w3.file(232, b"f.ds1", ds1(16, 16));
    w3.init(l3);
    w3.generate(l3).unwrap();
    let r3 = room_at(&w3, l3, 0, 0);
    let (wx, wy) = (2 - mrow.dx.min(0) / 5, 2 - mrow.dy.min(0) / 5);
    let c = cell(mrow.main, mrow.sub);
    let o = if mrow.right { 9 } else { 8 };
    assert_eq!(
        w3.run(|p, d, ctx| p.door_unit(d, ctx, r3, wx, wy, c, o))
            .unwrap(),
        DoorOutcome::Placed
    );
    let u = &w3.p.room_units(r3)[0];
    assert_eq!((u.unit_type, u.class, u.mode), (1, -1, 1));
}

#[test]
fn door_record_flag_outcomes() {
    // Flag 0x20 after a unit is added and when roll(3) gave 0 (the draw
    // is spent); not when no row matches or the position is outside.
    assert!(DoorOutcome::Placed.sets_record_flag());
    assert!(DoorOutcome::Rolled0.sets_record_flag());
    assert!(!DoorOutcome::NoRow.sets_record_flag());
    assert!(!DoorOutcome::Outside.sets_record_flag());
}
