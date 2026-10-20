//! Mutation-testing kills for `fixup` and `fixup/*`: tests that fail on mutants
//! `cargo mutants` reported as missed. They exercise the fix-ups through the
//! per-table loaders in [`crate::fixup::apply`] and the helpers directly, on
//! synthetic vectors only (invented rows, keys and codes).

use d2_formats::animdata::{self, AnimData};

use crate::bin::{u32_at, BinSet, BinTable};
use crate::fixup::maps::{self, EquivKind};
use crate::fixup::qsort::qsort;
use crate::fixup::records;
use crate::fixup::{self, FixedSet, HC_INDICES};
use crate::schema::schema;
use crate::strings::StringTables;
use crate::txt::TxtTable;

// ================================================================ helpers

fn size(name: &str) -> usize {
    schema().table(name).unwrap().record_size
}

/// A `.bin` table of `records`, each padded to its schema record size.
fn table(name: &str, records: Vec<Vec<u8>>) -> BinTable {
    let size = size(name);
    BinTable {
        name: name.into(),
        source: "test".into(),
        count: records.len(),
        record_size: size,
        records: records
            .into_iter()
            .flat_map(|mut r| {
                r.resize(size, 0);
                r
            })
            .collect(),
    }
}

/// A zero record of `name`'s size with the given writes.
fn rec(name: &str, writes: &[(usize, &[u8])]) -> Vec<u8> {
    let mut r = vec![0; size(name)];
    for (o, b) in writes {
        r[*o..*o + b.len()].copy_from_slice(b);
    }
    r
}

fn rec_mut(t: &mut BinTable, k: usize) -> &mut [u8] {
    let size = t.record_size;
    &mut t.records[k * size..(k + 1) * size]
}

fn u16_at(r: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([r[o], r[o + 1]])
}

fn i16_at(r: &[u8], o: usize) -> i16 {
    u16_at(r, o) as i16
}

fn i32_at(r: &[u8], o: usize) -> i32 {
    u32_at(r, o) as i32
}

fn wide(r: &[u8], at: usize, units: usize) -> String {
    (0..units)
        .map(|k| u16_at(r, at + 2 * k))
        .take_while(|&u| u != 0)
        .map(|u| char::from(u as u8))
        .collect()
}

fn empty_txt() -> TxtTable {
    TxtTable {
        header: Vec::new(),
        records: Vec::new(),
        removed_lines: Vec::new(),
    }
}

fn no_anim() -> AnimData {
    AnimData {
        buckets: vec![Vec::new(); animdata::BUCKETS],
    }
}

/// AnimData with the given (name, speed) records.
fn anim(records: &[(&[u8], u32)]) -> AnimData {
    let mut a = no_anim();
    for (name, speed) in records {
        let mut r = animdata::DEFAULT_RECORD;
        r.name[..name.len()].copy_from_slice(name);
        r.speed = *speed;
        a.buckets[animdata::hash(name)].push(r);
    }
    a
}

/// Runs every fix-up on a set holding only `tables` (LoD, no strings).
fn fix_with(tables: Vec<BinTable>, anim: &AnimData) -> FixedSet {
    let set = BinSet {
        lod: true,
        rules: Default::default(),
        strings: StringTables::default(),
        tables,
        code: Default::default(),
        code_reports: Default::default(),
        hitclass: BinTable {
            name: "hitclass".into(),
            source: "test".into(),
            count: 0,
            record_size: 4,
            records: Vec::new(),
        },
        sounds: empty_txt(),
        soundenviron: empty_txt(),
    };
    fixup::apply(&set, anim).expect("fix-ups apply")
}

fn fix(tables: Vec<BinTable>) -> FixedSet {
    fix_with(tables, &no_anim())
}

/// monmode with the walk (2) and run (15) tokens.
fn monmode(rows: usize) -> BinTable {
    let mut t = table("monmode", vec![vec![]; rows]);
    if rows > 2 {
        rec_mut(&mut t, 2)[0x20..0x24].copy_from_slice(b"WL  ");
    }
    if rows > 15 {
        rec_mut(&mut t, 15)[0x20..0x24].copy_from_slice(b"RN  ");
    }
    t
}

/// A monstats row: (BaseId, NextInClass, MonStatsEx), code `AA`.
fn monster(base: i16, next: i16, ex: i16) -> Vec<u8> {
    rec(
        "monstats",
        &[
            (0x02, &base.to_le_bytes()),
            (0x04, &next.to_le_bytes()),
            (0x10, b"aa  "),
            (0x18, &ex.to_le_bytes()),
        ],
    )
}

// ============================================ fixup::apply, per loader

// Each table's fix-up runs in its own loader. One small set per loader; the assertions are the rule's effect.

#[test]
fn apply_runs_itemtypes_and_montype() {
    let it = |code: &[u8; 4], e1: i16| rec("itemtypes", &[(0, code), (0x04, &e1.to_le_bytes())]);
    let mt = |e1: i16| rec("montype", &[(0x02, &e1.to_le_bytes())]);
    let f = fix(vec![
        table(
            "itemtypes",
            vec![it(b"aaa ", 0), it(b"bbb ", 0), it(b"ccc ", 1)],
        ),
        table("montype", vec![mt(0), mt(0), mt(1)]),
    ]);
    // itemtypes code link and equivalence matrix.
    assert_eq!(f.item_types.find(u32::from_le_bytes(*b"ccc ")), Some(2));
    assert_eq!(f.itemtypes_equiv.n, 3);
    let row =
        |m: &maps::EquivMatrix, i: usize| (0..m.n).filter(|&j| m.get(i, j)).collect::<Vec<_>>();
    assert_eq!(row(&f.itemtypes_equiv, 2), [0, 1, 2]);
    assert_eq!(f.montype_equiv.n, 3);
    assert_eq!(row(&f.montype_equiv, 2), [1, 2]);
}

// mask := (1 << stuff) − 1.
#[test]
fn apply_stat_mask_follows_stuff() {
    for (stuff, want_stuff, want_mask) in
        [(5i32, 5u32, 0x1Fu32), (1, 1, 1), (0, 6, 0x3F), (8, 8, 0xFF)]
    {
        let r0 = rec("itemstatcost", &[(0x140, &stuff.to_le_bytes())]);
        let f = fix(vec![table("itemstatcost", vec![r0])]);
        assert_eq!(
            (f.stat_stuff, f.stat_mask),
            (want_stuff, want_mask),
            "stuff {stuff}"
        );
    }
}

// Missiles and monumod clamps through their loaders.
#[test]
fn apply_clamps_missiles_and_monumod() {
    let m = |b: u8| rec("missiles", &[(0x183, &[b])]);
    let um = |k: u8| rec("monumod", &[(0x10, &[k])]);
    let f = fix(vec![
        table("missiles", vec![m(9), m(8), m(0xFF), m(3)]),
        table("monumod", (0..=255u8).chain([7, 9]).map(um).collect()),
    ]);
    let t = f.table("missiles").unwrap();
    let got: Vec<u8> = t.iter().map(|r| r[0x183]).collect();
    assert_eq!(got, [8, 8, 8, 3]);
    let t = f.table("monumod").unwrap();
    assert_eq!(t.count, 256);
    assert_eq!(t.records.len(), 256 * t.record_size);
    assert_eq!(t.record(255)[0x10], 255);
}

#[test]
fn apply_runs_states() {
    let f = fix(vec![table(
        "states",
        vec![vec![], rec("states", &[(0x10, &[1 << 4])])],
    )]);
    assert_eq!(f.states.pgsv, [1]);
    assert_eq!(f.states.words, 1);
}

// The pettype append through the skills loader: p ≥ pettype count is skipped,
// the pettype table is found by name among the earlier tables.
#[test]
fn apply_runs_skills_and_pet_append() {
    let skill = |class: u8, pet: u8| {
        rec(
            "skills",
            &[
                (0x0C, &[class]),
                (0x94, &(-1i16).to_le_bytes()),
                (0xBE, &[pet]),
            ],
        )
    };
    let f = fix(vec![
        table("states", vec![]),
        table("pettype", vec![vec![], vec![]]),
        table(
            "skills",
            vec![
                skill(0, 1),
                skill(0xFF, 2),
                skill(1, 0xFF),
                skill(0, 1),
                skill(0, 9),
            ],
        ),
    ]);
    assert_eq!(f.skill_lists.counts[..2], [3, 1]);
    let pet = f.table("pettype").unwrap();
    assert_eq!(u32_at(pet.record(0), 0xBC), 0);
    let p1 = pet.record(1);
    assert_eq!(
        (u32_at(p1, 0xBC), u16_at(p1, 0xC0), u16_at(p1, 0xC2)),
        (2, 0, 3)
    );
    // The earlier non-pettype table is untouched.
    assert!(f.table("states").unwrap().records.is_empty());
}

#[test]
fn apply_runs_charstats() {
    let f = fix(vec![table(
        "charstats",
        vec![rec("charstats", &[(0x20, b"ama")])],
    )]);
    let r = f.table("charstats").unwrap().record(0);
    assert_eq!(wide(r, 0, 16), "ama -not xlated ");
}

/// Items: weapons, armor, misc of one record each, (code, version, level).
fn items() -> Vec<BinTable> {
    let item = |name: &str, code: &[u8; 4], version: u16, level: u8| {
        table(
            name,
            vec![rec(
                name,
                &[
                    (0x80, code),
                    (0xF6, &version.to_le_bytes()),
                    (0xFD, &[level]),
                ],
            )],
        )
    };
    vec![
        item("weapons", b"aaa ", 1, 9),
        item("armor", b"bbb ", 0, 4),
        item("misc", b"ccc ", 0, 2),
    ]
}

// The version-0 list through the items loaders; the item code map is built over weapons, armor, misc in that order.
#[test]
fn apply_builds_item_lists_after_misc() {
    let f = fix(items());
    assert_eq!(f.version0_items, [1, 2, 0]);
    for (k, code) in [b"aaa ", b"bbb ", b"ccc "].iter().enumerate() {
        assert_eq!(
            f.item_codes.find(u32::from_le_bytes(**code)),
            Some(k as u32)
        );
    }
}

#[test]
fn apply_runs_gems_and_gamble() {
    let mut t = items();
    t.push(table(
        "gems",
        vec![rec("gems", &[(0x28, &(-1i32).to_le_bytes())])],
    ));
    t.push(table(
        "gamble",
        vec![
            rec("gamble", &[(0, b"ccc ")]),
            rec("gamble", &[(0, b"bbb ")]),
        ],
    ));
    let f = fix(t);
    assert_eq!(i32_at(f.table("weapons").unwrap().record(0), 0xF0), -1);
    assert_eq!(f.gamble.index, Some(vec![2, 1]));
    assert_eq!(&f.gamble.thresholds[..5], [2, 0, 1, 1, 2]);
    let g = f.table("gamble").unwrap().record(1);
    assert_eq!((u32_at(g, 4), u32_at(g, 8)), (4, 1));
}

#[test]
fn apply_runs_monseq_monpreset() {
    let seq = |s: i16| rec("monseq", &[(0, &s.to_le_bytes())]);
    let f = fix(vec![
        table("monseq", vec![seq(0), seq(1), seq(1)]),
        table("monpreset", vec![vec![1], vec![2], vec![2]]),
    ]);
    let got: Vec<_> = f
        .monseq
        .iter()
        .map(|e| (e.first, e.count, e.count2))
        .collect();
    assert_eq!(got, [(Some(0), 1, 1), (Some(1), 2, 2)]);
    assert_eq!(&f.monpreset.first[..2], [Some(0), Some(1)]);
    assert_eq!(&f.monpreset.count[..2], [1, 2]);
}

// The monstats and monequip fix-ups through the monstats and monequip loaders.
#[test]
fn apply_runs_monstats_and_monequip() {
    let equip = |m: i16| {
        rec(
            "monequip",
            &[(0, &m.to_le_bytes()), (0x08, b"    "), (0x14, &[3])],
        )
    };
    let f = fix(vec![
        table("monstats2", vec![]),
        monmode(16),
        table("monstats", vec![monster(0, 1, -1), monster(0, -1, -1)]),
        table("monequip", vec![equip(1)]),
    ]);
    let t = f.table("monstats").unwrap();
    let r0 = t.record(0);
    let r1 = t.record(1);
    // Chains: 0 → 1 → stop.
    assert_eq!((r0[0x4A], r0[0x4B], r1[0x4A], r1[0x4B]), (2, 0, 2, 1));
    // Walk: AnimData default 256; run: walk / 2.
    assert_eq!((u16_at(r0, 0x36), u16_at(r0, 0x38)), (256, 128));
    // monequip: row 0 names monster 1; monster 0 has none.
    assert_eq!((i16_at(r0, 0x2A), i16_at(r1, 0x2A)), (-1, 0));
}

// Superunique map: hcIdx 0–65, first row wins, values ≥ 66 ignored.
#[test]
fn apply_superunique_hc_map() {
    let su = |hc: u32| rec("superuniques", &[(0x08, &hc.to_le_bytes())]);
    let f = fix(vec![table(
        "superuniques",
        vec![su(3), su(3), su(66), su(65), su(1000), su(0)],
    )]);
    let mut want = [None; HC_INDICES];
    want[3] = Some(0);
    want[65] = Some(3);
    want[0] = Some(5);
    assert_eq!(f.superunique_hc, want);
}

#[test]
fn apply_runs_levels_leveldefs() {
    let lv = rec(
        "levels",
        &[(0x36, &5i16.to_le_bytes()), (0x38, &(-1i16).to_le_bytes())],
    );
    let def = |p: u32| rec("leveldefs", &[(0x8C, &p.to_le_bytes())]);
    let f = fix(vec![
        table("levels", vec![lv]),
        table("leveldefs", vec![def(1), def(0), def(7)]),
    ]);
    let r = f.table("levels").unwrap().record(0);
    assert_eq!((r[0x33], r[0x34], r[0x35]), (1, 25, 25));
    assert_eq!(f.portals, [0, 2]);
}

#[test]
fn apply_runs_tile_tables_automap_objects() {
    let ty = |t: i32| rec("lvlsub", &[(0, &t.to_le_bytes()), (0x04, b"s/t")]);
    let am = rec("automap", &[(0, b"1 Town"), (0x10, b"wl")]);
    let f = fix(vec![
        table("lvltypes", vec![rec("lvltypes", &[(0, b"a/b")])]),
        table("lvlprest", vec![rec("lvlprest", &[(0x44, b"c/d")])]),
        table("lvlsub", vec![ty(0), ty(2)]),
        table("automap", vec![am]),
        table(
            "objects",
            vec![rec("objects", &[(0xD8, &3u32.to_le_bytes())])],
        ),
    ]);
    let path = |t: &str, k: usize, at: usize| {
        crate::bin::cstr(f.table(t).unwrap().record(k), at..at + 60).to_vec()
    };
    assert_eq!(path("lvltypes", 0, 0), b"DATA\\GLOBAL\\TILES\\a\\b");
    assert_eq!(path("lvlprest", 0, 0x44), b"DATA\\GLOBAL\\TILES\\c\\d");
    assert_eq!(path("lvlsub", 1, 4), b"DATA\\GLOBAL\\TILES\\s\\t");
    assert_eq!(f.lvlsub_types, [0, 0, 1]);
    assert_eq!(f.automap.records.len(), 1);
    let a = &f.automap.records[0];
    assert_eq!((i32_at(a, 0), i32_at(a, 4)), (1, 1));
    assert_eq!(u32_at(f.table("objects").unwrap().record(0), 0xD8), 0x300);
}

// ===================================================== records (record fix-ups)

// itemstatcost ops: op 13 is valid, a base equal to n is no base, op-base
// slots fill in order, op 4/5 alone set +0x53, entries fill in order,
// and the flag bits for 7 / 9 / 11 as source or target.
#[test]
fn stat_ops_slots_entries_and_flags() {
    let n = 12;
    let mut t = table("itemstatcost", vec![vec![]; n]);
    let none = 0xFFFFu16;
    // (stat, op, param, base, op stats)
    let stats: [(usize, u8, u8, u16, [u16; 3]); 6] = [
        (1, 13, 0, 0, [none; 3]),
        (2, 5, 0, 0, [none; 3]),
        (3, 2, 0, n as u16, [none; 3]),
        (5, 1, 0, none, [11, none, none]),
        (7, 3, 4, none, [4, 6, none]),
        (9, 1, 0, none, [4, none, none]),
    ];
    for &(i, op, param, base, ops) in &stats {
        let r = rec_mut(&mut t, i);
        r[0x54] = op;
        r[0x55] = param;
        r[0x56..0x58].copy_from_slice(&base.to_le_bytes());
        for (k, s) in ops.iter().enumerate() {
            r[0x58 + 2 * k..0x5A + 2 * k].copy_from_slice(&s.to_le_bytes());
        }
    }
    records::stat_ops(&mut t);
    let r = |k: usize| t.record(k);
    // Stat 0 is the base of stats 1 and 2, in that order.
    assert_eq!(
        (r(0)[0x51], u16_at(r(0), 0x5E), u16_at(r(0), 0x60)),
        (1, 1, 2)
    );
    assert_eq!(u16_at(r(0), 0x62), 0xFFFF);
    assert_eq!((r(1)[0x54], r(1)[0x53]), (13, 0));
    assert_eq!(r(2)[0x53], 1);
    // Base n: nothing.
    assert_eq!((r(3)[0x51], r(3)[0x53]), (0, 0));
    // Flags: 5 → 11 sets bits 8 and 5; 7 (twice) bits 6 and 5; 9 bits 7, 5.
    assert_eq!(u32_at(r(5), 0x04), 0x120);
    assert_eq!(u32_at(r(7), 0x04), 0x60);
    assert_eq!(u32_at(r(9), 0x04), 0xA0);
    // Stat 4's table: entry 0 from stat 7, entry 1 from stat 9.
    assert_eq!(
        &r(4)[0xDE..0xEA],
        [0xFF, 0xFF, 7, 0, 3, 4, 0xFF, 0xFF, 9, 0, 1, 0]
    );
    assert_eq!(r(4)[0x52], 1);
}

// Set attachment: an item of set 1 attaches to set 1.
#[test]
fn set_attachment_to_a_later_set() {
    let mut sets = table(
        "sets",
        vec![
            rec("sets", &[(0x04, &7u16.to_le_bytes())]),
            rec("sets", &[(0x04, &100u16.to_le_bytes())]),
        ],
    );
    let item = |s: i16| rec("setitems", &[(0x2C, &s.to_le_bytes())]);
    let mut items = table("setitems", vec![item(1), item(2), item(1)]);
    records::attach_set_items(&mut items, &mut sets).unwrap();
    let s1 = sets.record(1);
    assert_eq!(
        (i32_at(s1, 0x0C), u32_at(s1, 0x110), u32_at(s1, 0x114)),
        (2, 0, 2)
    );
    assert_eq!(i32_at(sets.record(0), 0x0C), 0);
    let it = |k: usize| (u16_at(items.record(k), 0x22), u16_at(items.record(k), 0x2E));
    assert_eq!([it(0), it(1), it(2)], [(100, 0), (0, 0), (100, 1)]);
}

// monstats COF name: weapon class from monstats2 when MonStatsEx is
// in 0 … count − 1, else `hth`.
#[test]
fn speed_weapon_class_range() {
    let m2 = table(
        "monstats2",
        vec![rec("monstats2", &[(0x10, b"1hs ")]), vec![]],
    );
    let a = anim(&[(b"AAWL1HS", 77), (b"AAWLHTH", 55)]);
    let mut t = table(
        "monstats",
        vec![monster(0, -1, 0), monster(1, -1, 2), monster(2, -1, -1)],
    );
    records::monstats_speeds(&mut t, &m2, &monmode(16), &a).unwrap();
    let walk: Vec<u16> = t.iter().map(|r| u16_at(r, 0x36)).collect();
    assert_eq!(walk, [77, 55, 55]);
}

// A missing monmode row 2 is a FixupError.
#[test]
fn speed_missing_monmode_row_is_an_error() {
    let mut t = table("monstats", vec![monster(0, -1, -1)]);
    let m2 = table("monstats2", vec![]);
    assert!(records::monstats_speeds(&mut t, &m2, &monmode(2), &no_anim()).is_err());
}

// monstats BaseId repair: the repair writes row r's own BaseId.
#[test]
fn base_repair_writes_its_own_row() {
    let mut t = table("monstats", vec![monster(0, -1, -1), monster(9, -1, -1)]);
    let m2 = table("monstats2", vec![]);
    records::monstats_speeds(&mut t, &m2, &monmode(16), &no_anim()).unwrap();
    assert_eq!((i16_at(t.record(0), 2), i16_at(t.record(1), 2)), (0, 1));
}

// monstats run base: rows < 410 halve the walk, row 410 looks up the run.
#[test]
fn run_base_switches_at_row_410() {
    let mut t = table("monstats", (0..411).map(|r| monster(r, -1, -1)).collect());
    let m2 = table("monstats2", vec![]);
    let a = anim(&[(b"AAWLHTH", 100), (b"AARNHTH", 70)]);
    records::monstats_speeds(&mut t, &m2, &monmode(16), &a).unwrap();
    assert_eq!(u16_at(t.record(409), 0x38), 50);
    assert_eq!(u16_at(t.record(410), 0x38), 70);
}

// monequip: m ≥ monstats count leaves the row as it is.
#[test]
fn monequip_monster_past_the_end() {
    let mut ms = table("monstats", vec![vec![], vec![]]);
    let row = rec(
        "monequip",
        &[
            (0, &2i16.to_le_bytes()),
            (0x08, b"zzz "),
            (0x14, &[12, 0, 3]),
        ],
    );
    let mut me = table("monequip", vec![row.clone()]);
    records::link_monequip(&mut me, &mut ms, &crate::compile::CodeLinker::default());
    assert_eq!(me.record(0), &row[..]);
    assert!(ms.iter().all(|r| i16_at(r, 0x2A) == -1));
}

fn path_at(r: &[u8], at: usize) -> &[u8] {
    crate::bin::cstr(r, at..at + 60)
}

const FIXED: &[u8] = b"DATA\\GLOBAL\\TILES\\a\\b";

// Tile path fields and the `lod` gate.
#[test]
fn tile_path_fields_and_gate() {
    // lvltypes: File 1–32 at 0x3C·k, every row.
    let fields: Vec<usize> = (0..32).map(|k| 0x3C * k).collect();
    let w: Vec<(usize, &[u8])> = fields.iter().map(|&o| (o, &b"a/b"[..])).collect();
    let mut t = table("lvltypes", vec![rec("lvltypes", &w)]);
    records::tile_paths(&mut t, false).unwrap();
    for &o in &fields {
        assert_eq!(path_at(t.record(0), o), FIXED, "lvltypes +{o:#x}");
    }
    // lvlprest: File1–6 at 0x44 + 0x3C·k; gated by lod or Expansion = 0.
    let fields: Vec<usize> = (0..6).map(|k| 0x44 + 0x3C * k).collect();
    let row = |exp: u32| {
        let mut w: Vec<(usize, &[u8])> = fields.iter().map(|&o| (o, &b"a/b"[..])).collect();
        let e = exp.to_le_bytes();
        w.push((0x20, &e));
        rec("lvlprest", &w)
    };
    for lod in [false, true] {
        let mut t = table("lvlprest", vec![row(0), row(1)]);
        records::tile_paths(&mut t, lod).unwrap();
        for &o in &fields {
            assert_eq!(path_at(t.record(0), o), FIXED, "lvlprest +{o:#x}");
            let want: &[u8] = if lod { FIXED } else { b"a/b" };
            assert_eq!(path_at(t.record(1), o), want, "lvlprest lod {lod}");
        }
    }
    // lvlsub: File at +0x04, every row.
    let mut t = table("lvlsub", vec![rec("lvlsub", &[(0x04, b"a/b")])]);
    records::tile_paths(&mut t, false).unwrap();
    assert_eq!(path_at(t.record(0), 4), FIXED);
}

// objects frame counts.
#[test]
fn objects_all_frame_counts() {
    let w: Vec<[u8; 4]> = (1..=8u32)
        .map(|k| (k | 0x0100_0000).to_le_bytes())
        .collect();
    let writes: Vec<(usize, &[u8])> = w
        .iter()
        .enumerate()
        .map(|(k, b)| (0xD8 + 4 * k, &b[..]))
        .collect();
    let mut t = table("objects", vec![rec("objects", &writes)]);
    records::objects(&mut t, &StringTables::default()).unwrap();
    for k in 0..8 {
        assert_eq!(u32_at(t.record(0), 0xD8 + 4 * k), (k as u32 + 1) << 8);
    }
}

// =================================================== maps (runtime maps)

// Equivalence reader: out-of-range i or j → 0; row 0 holds column 0 only.
#[test]
fn equiv_bounds_and_row_zero() {
    let t = table("itemtypes", vec![vec![]; 32]);
    let m = maps::equiv_matrix(&t, EquivKind::ItemTypes).unwrap();
    assert!(m.get(1, 0) && m.get(0, 0));
    assert!(!m.get(0, 32));
    assert!(!m.get(32, 0));
    // Row 0 is not walked even when its equiv1 names another row.
    let t = table(
        "itemtypes",
        vec![
            rec("itemtypes", &[(0x04, &1i16.to_le_bytes())]),
            vec![],
            vec![],
        ],
    );
    let m = maps::equiv_matrix(&t, EquivKind::ItemTypes).unwrap();
    assert_eq!((0..3).filter(|&j| m.get(0, j)).collect::<Vec<_>>(), [0]);
    let t = table(
        "montype",
        vec![
            rec("montype", &[(0x02, &1i16.to_le_bytes())]),
            vec![],
            vec![],
        ],
    );
    let m = maps::equiv_matrix(&t, EquivKind::MonType).unwrap();
    assert!((0..3).all(|j| !m.get(0, j)));
}

// Both monseq counts count the rows.
#[test]
fn monseq_both_counts() {
    let seq = |s: i16| rec("monseq", &[(0, &s.to_le_bytes())]);
    let e = maps::monseq(&table("monseq", vec![seq(0), seq(0), seq(0)])).unwrap();
    assert_eq!((e[0].count, e[0].count2), (3, 3));
}

// hireling: only Id < 256 is entered.
#[test]
fn hireling_id_256_is_ignored() {
    let row = |id: i32| rec("hireling", &[(4, &id.to_le_bytes())]);
    let h = maps::hireling_first(&table("hireling", vec![row(256), row(255)])).unwrap();
    assert_eq!(h[0][255], 1);
    assert!(h[0][..255].iter().all(|&v| v == -1));
    assert!(h[1].iter().all(|&v| v == -1));
}

// leveldefs: every level with a portal, in order.
#[test]
fn portals_in_order() {
    let def = |p: u32| rec("leveldefs", &[(0x8C, &p.to_le_bytes())]);
    let t = table("leveldefs", vec![def(2), def(0), def(0), def(1)]);
    assert_eq!(maps::portals(&t), [0, 3]);
}

// ==================================================== qsort

/// The C runtime `qsort` written out step by step, as the test oracle.
fn reference_qsort(v: &mut [(u8, usize)]) {
    let n = v.len() as isize;
    if n < 2 {
        return;
    }
    let cmp = |v: &[(u8, usize)], a: isize, b: isize| v[a as usize].0.cmp(&v[b as usize].0) as i32;
    let sw = |v: &mut [(u8, usize)], a: isize, b: isize| v.swap(a as usize, b as usize);
    let mut stack = Vec::new();
    let (mut lo, mut hi) = (0isize, n - 1);
    let mut step = 1;
    let (mut mid, mut l, mut h) = (0, 0, 0);
    loop {
        match step {
            1 => {
                let size = hi - lo + 1;
                if size <= 8 {
                    let mut top = hi;
                    while top > lo {
                        let mut m = lo;
                        for p in lo + 1..=top {
                            if cmp(v, p, m) > 0 {
                                m = p;
                            }
                        }
                        sw(v, m, top);
                        top -= 1;
                    }
                    step = 6;
                } else {
                    mid = lo + size / 2;
                    step = 2;
                }
            }
            2 => {
                if cmp(v, lo, mid) > 0 {
                    sw(v, lo, mid);
                }
                if cmp(v, lo, hi) > 0 {
                    sw(v, lo, hi);
                }
                if cmp(v, mid, hi) > 0 {
                    sw(v, mid, hi);
                }
                l = lo;
                h = hi;
                step = 3;
            }
            3 => {
                if mid > l {
                    l += 1;
                    while l < mid && cmp(v, l, mid) <= 0 {
                        l += 1;
                    }
                }
                if mid <= l {
                    l += 1;
                    while l <= hi && cmp(v, l, mid) <= 0 {
                        l += 1;
                    }
                }
                h -= 1;
                while h > mid && cmp(v, h, mid) > 0 {
                    h -= 1;
                }
                if h < l {
                    step = 4;
                } else {
                    sw(v, l, h);
                    if mid == h {
                        mid = l;
                    }
                }
            }
            4 => {
                h += 1;
                if mid < h {
                    h -= 1;
                    while h > mid && cmp(v, h, mid) == 0 {
                        h -= 1;
                    }
                }
                if mid >= h {
                    h -= 1;
                    while h > lo && cmp(v, h, mid) == 0 {
                        h -= 1;
                    }
                }
                step = 5;
            }
            5 => {
                step = 6;
                if h - lo >= hi - l {
                    if lo < h {
                        stack.push((lo, h));
                    }
                    if l < hi {
                        lo = l;
                        step = 1;
                    }
                } else {
                    if l < hi {
                        stack.push((l, hi));
                    }
                    if lo < h {
                        hi = h;
                        step = 1;
                    }
                }
            }
            _ => match stack.pop() {
                None => return,
                Some((a, b)) => {
                    (lo, hi) = (a, b);
                    step = 1;
                }
            },
        }
    }
}

// The production `qsort` agrees with the oracle above.
#[test]
fn qsort_matches_the_reference_steps() {
    // Two elements out of order are swapped.
    let mut two = [(1u8, 0usize), (0, 1)];
    qsort(&mut two, |a, b| a.0.cmp(&b.0));
    assert_eq!(two, [(0, 1), (1, 0)]);
    // Many lengths and key ranges (ties dominate at small ranges).
    let mut seed = 0x1234_5678u32;
    for len in 0..70 {
        for range in [1u32, 2, 3, 5, 17, 256] {
            for _ in 0..6 {
                let keys: Vec<(u8, usize)> = (0..len)
                    .map(|i| {
                        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                        (((seed >> 16) % range) as u8, i)
                    })
                    .collect();
                let mut got = keys.clone();
                qsort(&mut got, |a, b| a.0.cmp(&b.0));
                let mut want = keys.clone();
                reference_qsort(&mut want);
                assert_eq!(got, want, "keys {keys:?}");
                assert!(want.windows(2).all(|w| w[0].0 <= w[1].0));
            }
        }
    }
}
