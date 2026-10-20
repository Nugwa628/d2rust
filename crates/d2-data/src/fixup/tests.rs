//! Tests for the post-load fix-ups, runtime maps, wide text helpers and the C runtime
//! `qsort`, run on small synthetic tables (invented rows, keys and codes).

use d2_formats::animdata::{self, AnimData};

use super::maps::{self, EquivKind};
use super::qsort::qsort;
use super::records::{self, SPEED_CAP};
use super::text::{copy_wide, fix_path, wide_text};
use super::*;

fn table(name: &str, size: usize, records: Vec<Vec<u8>>) -> BinTable {
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

fn rec(size: usize, writes: &[(usize, &[u8])]) -> Vec<u8> {
    let mut r = vec![0; size];
    for (o, b) in writes {
        r[*o..*o + b.len()].copy_from_slice(b);
    }
    r
}

fn u16s(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

fn wide(r: &[u8], at: usize, units: usize) -> String {
    (0..units)
        .map(|k| get_u16(r, at + 2 * k))
        .take_while(|&u| u != 0)
        .map(|u| char::from(u as u8))
        .collect()
}

// ------------------------------------------------------------- record fix-ups

#[test]
fn pet_skills_cap_at_15() {
    let mut pet = table("pettype", 224, vec![vec![]; 2]);
    let skills = table(
        "skills",
        572,
        (0..20)
            .map(|s| {
                let p = match s {
                    3 => 1,
                    4 => 0xFF,
                    _ => 0,
                };
                rec(572, &[(0xBE, &[p])])
            })
            .collect(),
    );
    append_pet_skills(&skills, &mut pet);
    let p0 = pet.record(0);
    assert_eq!(u32_at(p0, 0xBC), 15);
    let listed: Vec<u16> = (0..15).map(|k| get_u16(p0, 0xC0 + 2 * k)).collect();
    assert_eq!(listed, [0, 1, 2, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    let p1 = pet.record(1);
    assert_eq!((u32_at(p1, 0xBC), get_u16(p1, 0xC0)), (1, 3));
}

fn isc(n: usize, stats: &[(usize, u8, u8, u16, [u16; 3])]) -> BinTable {
    let mut t = table("itemstatcost", 0x144, vec![vec![]; n]);
    for &(i, op, param, base, ops) in stats {
        let r = super::rec(&mut t, i);
        r[0x54] = op;
        r[0x55] = param;
        set_u16(r, 0x56, base);
        for (k, s) in ops.iter().enumerate() {
            set_u16(r, 0x58 + 2 * k, *s);
        }
    }
    t
}

#[test]
fn stat_ops_base_and_targets() {
    let mut t = isc(3, &[(1, 2, 3, 0, [2, 0xFFFF, 0])]);
    records::stat_ops(&mut t);
    let r0 = t.record(0);
    assert_eq!(
        (r0[0x51], get_u16(r0, 0x5E), get_u16(r0, 0x60)),
        (1, 1, 0xFFFF)
    );
    assert_eq!((r0[0x52], r0[0xE2]), (0, 0)); // op stat 3 (stat 0) not read
    assert_eq!(t.record(1)[0x51], 1);
    let r2 = t.record(2);
    assert_eq!(&r2[0xDE..0xE4], [0, 0, 1, 0, 2, 3]);
    assert_eq!(r2[0x52], 1);
    assert_eq!(u32_at(t.record(1), 0x04), 0);
}

#[test]
fn stat_ops_bad_op_is_cleared() {
    let mut t = isc(3, &[(1, 14, 3, 0, [2, 0xFFFF, 0])]);
    records::stat_ops(&mut t);
    assert_eq!(t.record(1)[0x54], 0);
    for r in t.iter() {
        assert_eq!((r[0x51], r[0x52], r[0x53]), (0, 0, 0));
        assert!(r[0x5E..0xDE].iter().all(|&b| b == 0xFF));
        assert!(r[0xDE..0x13E].iter().all(|&b| b == 0));
    }
}

#[test]
fn stat_ops_flags_for_maxhp() {
    let mut t = isc(8, &[(5, 1, 0, 0xFFFF, [7, 0xFFFF, 0xFFFF])]);
    records::stat_ops(&mut t);
    assert_eq!(u32_at(t.record(5), 0x04), 0x60);
    assert_eq!(&t.record(7)[0xDE..0xE4], [0xFF, 0xFF, 5, 0, 1, 0]);
}

#[test]
fn stat_ops_op4_sets_053() {
    let mut t = isc(3, &[(2, 4, 0, 1, [0xFFFF; 3])]);
    records::stat_ops(&mut t);
    assert_eq!(t.record(2)[0x53], 1);
    assert_eq!(get_u16(t.record(1), 0x5E), 2);
}

#[test]
fn set_attachment() {
    let mut sets = table("sets", 0x128, vec![rec(0x128, &[(0x04, &u16s(100))])]);
    let mut items = table(
        "setitems",
        0x1B8,
        (0..10)
            .map(|k| {
                let s: i16 = match k {
                    8 => -1,
                    9 => 1,
                    _ => 0,
                };
                rec(0x1B8, &[(0x2C, &s.to_le_bytes())])
            })
            .collect(),
    );
    records::attach_set_items(&mut items, &mut sets).unwrap();
    let s = sets.record(0);
    assert_eq!(u32_at(s, 0x0C), 6);
    let slots: Vec<u32> = (0..6).map(|c| u32_at(s, 0x110 + 4 * c)).collect();
    assert_eq!(slots, [0, 1, 2, 3, 4, 5]);
    for k in 0..6 {
        let r = items.record(k);
        assert_eq!((get_u16(r, 0x2E), get_u16(r, 0x22)), (k as u16, 100));
    }
    for k in 6..10 {
        let r = items.record(k);
        assert_eq!((get_u16(r, 0x2E), get_u16(r, 0x22)), (0, 0));
    }
}

#[test]
fn gem_offsets() {
    let item = |k: u32| rec(424, &[(0xF0, &k.to_le_bytes())]);
    let mut items = vec![
        table("weapons", 424, (0..3).map(|_| item(7)).collect()),
        table("armor", 424, (0..3).map(|_| item(7)).collect()),
        table("misc", 424, vec![]),
    ];
    let mut g = table(
        "gems",
        0xC0,
        vec![
            rec(0xC0, &[(0x28, &(-1i32).to_le_bytes())]),
            rec(0xC0, &[(0x28, &4i32.to_le_bytes())]),
        ],
    );
    records::gems(&mut g, &mut items, &StringTables::default()).unwrap();
    let off = |t: usize, k: usize| i32_at(items[t].record(k), 0xF0);
    assert_eq!([off(0, 0), off(0, 1), off(0, 2)], [-1, -1, 7]);
    assert_eq!([off(1, 0), off(1, 1), off(1, 2)], [7, 1, 7]);
    assert_eq!(get_u16(g.record(0), 0x2C), 0);
    let mut bad = table(
        "gems",
        0xC0,
        vec![rec(0xC0, &[(0x28, &6i32.to_le_bytes())])],
    );
    assert!(records::gems(&mut bad, &mut items, &StringTables::default()).is_err());
}

fn monstats(rows: &[(i16, i16)]) -> BinTable {
    table(
        "monstats",
        0x1A8,
        rows.iter()
            .map(|(b, n)| rec(0x1A8, &[(0x02, &b.to_le_bytes()), (0x04, &n.to_le_bytes())]))
            .collect(),
    )
}

#[test]
fn monstats_chain_positions() {
    for last in [-1, 2] {
        let mut t = monstats(&[(0, 1), (0, 2), (0, last)]);
        records::monstats_chains(&mut t).unwrap();
        let got: Vec<(u8, u8)> = t.iter().map(|r| (r[0x4A], r[0x4B])).collect();
        assert_eq!(got, [(3, 0), (3, 1), (3, 2)]);
    }
    let mut t = monstats(&[(0, 1), (0, 0)]);
    records::monstats_chains(&mut t).unwrap();
    let got: Vec<(u8, u8)> = t.iter().map(|r| (r[0x4A], r[0x4B])).collect();
    assert_eq!(got, [(0, 254), (0, 255)]);
    assert!(records::monstats_chains(&mut monstats(&[(5, -1)])).is_err());
    assert!(records::monstats_chains(&mut monstats(&[(0, 9)])).is_err());
}

/// AnimData with the given (name, speed) records.
fn anim(records: &[(&[u8], u32)]) -> AnimData {
    let mut a = AnimData {
        buckets: vec![Vec::new(); animdata::BUCKETS],
    };
    for (name, speed) in records {
        let mut r = animdata::DEFAULT_RECORD;
        r.name[..name.len()].copy_from_slice(name);
        r.speed = *speed;
        a.buckets[animdata::hash(name)].push(r);
    }
    a
}

fn monmode() -> BinTable {
    let mut t = table("monmode", 0x34, vec![vec![]; 16]);
    super::rec(&mut t, 2)[0x20..0x24].copy_from_slice(b"WL  ");
    super::rec(&mut t, 15)[0x20..0x24].copy_from_slice(b"RN  ");
    t
}

/// Two monstats rows of class 0 (`AA`), row 1 based on row 0 (or as
/// given): (BaseId, Velocity, Run, compiled +0x36).
fn speeds(rows: &[(i16, i16, i16, i16)], walk: u32) -> BinTable {
    let mut t = table(
        "monstats",
        0x1A8,
        rows.iter()
            .map(|(b, v, run, w)| {
                rec(
                    0x1A8,
                    &[
                        (0x02, &b.to_le_bytes()),
                        (0x04, &(-1i16).to_le_bytes()),
                        (0x10, b"aa  "),
                        (0x18, &(-1i16).to_le_bytes()),
                        (0x32, &v.to_le_bytes()),
                        (0x34, &run.to_le_bytes()),
                        (0x36, &w.to_le_bytes()),
                    ],
                )
            })
            .collect(),
    );
    let a = anim(&[(b"AAWLHTH", walk)]);
    let m2 = table("monstats2", 0x134, vec![]);
    records::monstats_speeds(&mut t, &m2, &monmode(), &a).unwrap();
    t
}

#[test]
fn monstats_walk_scaling() {
    let walk = |t: &BinTable, r: usize| get_u16(t.record(r), 0x36);
    let t = speeds(&[(0, 2, 0, 0), (0, 3, 0, 0)], 101);
    assert_eq!((walk(&t, 0), walk(&t, 1)), (101, 151));
    let t = speeds(&[(0, 2, 0, 0), (0, -1, 0, 0)], 100);
    assert_eq!(walk(&t, 1), SPEED_CAP as u16);
    let t = speeds(&[(0, 0, 0, 0), (0, 5, 0, 0)], 100);
    assert_eq!(walk(&t, 1), 100);
    // Row 0 run: walk / 2; row 1 run: scaled by Run.
    let t = speeds(&[(0, 0, 2, 0), (0, 0, 4, 0)], 100);
    let run = |r: usize| get_u16(t.record(r), 0x38);
    assert_eq!((run(0), run(1)), (50, 100));
}

#[test]
fn monstats_run_base_from_later_row() {
    let t = speeds(&[(1, 0, 0, 0), (1, 0, 0, -3)], 100);
    assert_eq!(get_u16(t.record(0), 0x38), SPEED_CAP as u16);
}

#[test]
fn monstats_out_of_range_base_is_repaired_in_pass_b() {
    let t = speeds(&[(7, 0, 0, 0)], 90);
    assert_eq!(
        (i16_at(t.record(0), 0x02), get_u16(t.record(0), 0x36)),
        (0, 90)
    );
}

#[test]
fn monstats_missing_name_gives_default() {
    let t = speeds(&[(0, 0, 0, 0)], 90);
    let a = anim(&[]);
    let mut t2 = t.clone();
    records::monstats_speeds(&mut t2, &table("monstats2", 0x134, vec![]), &monmode(), &a).unwrap();
    assert_eq!(get_u16(t2.record(0), 0x36), 256);
}

#[test]
fn monequip_links_and_clears() {
    let mut items = CodeLinker::default();
    items.add(u32::from_le_bytes(*b"hax "));
    let mut ms = table("monstats", 0x1A8, vec![vec![]; 3]);
    let row = |m: i16, codes: [&[u8; 4]; 3], locs: [u8; 3]| {
        rec(
            28,
            &[
                (0, &m.to_le_bytes()),
                (0x08, codes[0]),
                (0x0C, codes[1]),
                (0x10, codes[2]),
                (0x14, &locs),
            ],
        )
    };
    let mut me = table(
        "monequip",
        28,
        vec![
            row(2, [b"hax ", b"xyz ", b"    "], [4, 3, 3]),
            row(2, [b"hax ", b"hax ", b"hax "], [11, 0, 0xFF]),
            row(0, [b"    "; 3], [0; 3]),
            row(-1, [b"zzz "; 3], [12; 3]),
        ],
    );
    records::link_monequip(&mut me, &mut ms, &items);
    let at = |n| i16_at(ms.record(n), 0x2A);
    assert_eq!([at(0), at(1), at(2)], [2, -1, 0]);
    assert_eq!(&me.record(0)[0x14..0x17], [4, 0, 3]);
    assert_eq!(&me.record(1)[0x14..0x17], [0, 0, 0]);
    assert_eq!(&me.record(3)[0x14..0x17], [12, 12, 12]);
}

#[test]
fn monumod_clamp() {
    let mut t = table("monumod", 32, vec![vec![]; 300]);
    records::clamp_monumod(&mut t);
    assert_eq!((t.count, t.records.len()), (256, 256 * 32));
}

#[test]
fn level_monster_counts() {
    let mut r = rec(0x220, &[]);
    for (k, v) in [5i16, -1, 7, -1].iter().enumerate() {
        r[0x36 + 2 * k..0x38 + 2 * k].copy_from_slice(&v.to_le_bytes());
    }
    r[0x9A..0x9A + 50].fill(0x01);
    let mut t = table("levels", 0x220, vec![r]);
    records::levels(&mut t, &StringTables::default()).unwrap();
    assert_eq!(&t.record(0)[0x33..0x36], [1, 25, 25]);
}

#[test]
fn level_names_miss_text_is_cut() {
    let mut r = rec(
        0x220,
        &[(0xF5, b"abc"), (0x11D, b"To The Pandemonium Run 1")],
    );
    r[0x16E..0x20E].fill(0x55);
    let mut t = table("levels", 0x220, vec![r]);
    records::levels(&mut t, &StringTables::default()).unwrap();
    let r = t.record(0);
    assert_eq!(wide(r, 0x16E, 40), "abc -not xlated call ken ");
    assert_eq!(
        wide(r, 0x1BE, 40),
        "To The Pandemonium Run 1 -not xlated ca"
    );
    assert_eq!((get_u16(r, 0x1BC), get_u16(r, 0x20C)), (0, 0));
}

#[test]
fn wide_text_vectors() {
    let s = StringTables::default();
    let t = wide_text(&s, "t", b"abc").unwrap();
    let mut r = vec![0u8; 80];
    copy_wide(&mut r, 0, &t, 40);
    assert_eq!(wide(&r, 0, 40), "abc -not xlated call ken ");
    assert!(wide_text(&s, "t", b"").unwrap().is_empty());
    assert!(wide_text(&s, "t", &[0xE9]).is_err());
    // A text of L or more units fills all L, no terminator.
    let mut r = vec![0u8; 8];
    copy_wide(&mut r, 0, &[0x41; 5], 4);
    assert_eq!(r, [0x41, 0, 0x41, 0, 0x41, 0, 0x41, 0]);
}

#[test]
fn tile_path_vectors() {
    let fixed = |s: &[u8]| {
        let mut r = vec![0xAAu8; 60];
        r[..s.len()].copy_from_slice(s);
        r[s.len()] = 0;
        fix_path(&mut r, 0, "lvlsub").map(|()| r)
    };
    let r = fixed(b"a/b").unwrap();
    assert_eq!(cstr(&r, 0..60), b"DATA\\GLOBAL\\TILES\\a\\b");
    assert_eq!(r[22], 0xAA); // after the new NUL: old bytes
    assert_eq!(cstr(&fixed(b"0").unwrap(), 0..60), b"0");
    assert_eq!(cstr(&fixed(b"").unwrap(), 0..60), b"");
    assert!(fixed(&[b'x'; 41]).is_ok());
    assert!(fixed(&[b'x'; 42]).is_err());
}

#[test]
fn objects_frames_and_names() {
    let mut t = table(
        "objects",
        0x1C0,
        vec![rec(
            0x1C0,
            &[(0x00, b"stma"), (0xD8, &0x0100_0001u32.to_le_bytes())],
        )],
    );
    records::objects(&mut t, &StringTables::default()).unwrap();
    let r = t.record(0);
    assert_eq!(u32_at(r, 0xD8), 0x100);
    assert_eq!(wide(r, 0x40, 64), "stma -not xlated call ken ");
}

// ------------------------------------------------------ runtime maps

fn sorted(keys: &[u8]) -> Vec<usize> {
    let mut v: Vec<(u8, usize)> = keys.iter().copied().zip(0..).collect();
    qsort(&mut v, |a, b| a.0.cmp(&b.0));
    v.into_iter().map(|(_, i)| i).collect()
}

#[test]
fn qsort_vectors() {
    assert_eq!(sorted(&[0; 8]), [1, 2, 3, 4, 5, 6, 7, 0]);
    assert_eq!(sorted(&[0; 10]), (0..10).collect::<Vec<_>>());
    assert_eq!(
        sorted(&[2, 1, 2, 1, 2, 1, 2, 1, 2, 1, 2, 1]),
        [11, 1, 7, 3, 5, 9, 2, 8, 6, 4, 10, 0]
    );
    // Sorted order for distinct keys.
    let keys: Vec<u8> = (0..200u32).map(|k| (k * 73 % 211) as u8).collect();
    let got: Vec<u8> = sorted(&keys).into_iter().map(|i| keys[i]).collect();
    let mut want = keys.clone();
    want.sort();
    assert_eq!(got, want);
}

fn equiv_table(name: &str, size: usize, links: &[(i16, i16)]) -> BinTable {
    let (o1, o2) = if name == "itemtypes" { (4, 6) } else { (2, 4) };
    table(
        name,
        size,
        links
            .iter()
            .map(|(e1, e2)| rec(size, &[(o1, &e1.to_le_bytes()), (o2, &e2.to_le_bytes())]))
            .collect(),
    )
}

fn row(m: &maps::EquivMatrix, i: usize) -> Vec<usize> {
    (0..m.n).filter(|&j| m.get(i, j)).collect()
}

#[test]
fn equivalence_vectors() {
    let t = equiv_table("itemtypes", 0xE4, &[(0, 0), (0, 0), (1, 0), (2, 0)]);
    let m = maps::equiv_matrix(&t, EquivKind::ItemTypes).unwrap();
    assert_eq!(
        (m.words, row(&m, 3), row(&m, 2), row(&m, 0)),
        (1, vec![0, 1, 2, 3], vec![0, 1, 2], vec![0])
    );
    let t = equiv_table("itemtypes", 0xE4, &[(0, 0), (0, 0), (1, 0), (0, 1)]);
    let m = maps::equiv_matrix(&t, EquivKind::ItemTypes).unwrap();
    assert_eq!(row(&m, 3), [0, 3]);
    let t = equiv_table("itemtypes", 0xE4, &[(0, 0), (0, 0), (1, 0), (7, 0)]);
    let m = maps::equiv_matrix(&t, EquivKind::ItemTypes).unwrap();
    assert_eq!(row(&m, 3), [0, 3]);
    let t = equiv_table("montype", 0x0C, &[(0, 0), (0, 0), (1, 0), (2, 0)]);
    let m = maps::equiv_matrix(&t, EquivKind::MonType).unwrap();
    assert_eq!((row(&m, 3), row(&m, 0)), (vec![1, 2, 3], vec![]));
    // e2 is followed (and the walk tries it after e1's branch).
    let t = equiv_table("montype", 0x0C, &[(0, 0), (0, 0), (0, 0), (2, 1)]);
    let m = maps::equiv_matrix(&t, EquivKind::MonType).unwrap();
    assert_eq!(row(&m, 3), [1, 2, 3]);
    let t = equiv_table("montype", 0x0C, &[(0, 0), (1, -2), (0, 0)]);
    assert!(maps::equiv_matrix(&t, EquivKind::MonType).is_err());
}

#[test]
fn desc_list_sorts_by_signed_priority() {
    let mut t = table("itemstatcost", 0x144, vec![vec![]; 4]);
    for (i, (prio, func)) in [(5i16, 1u8), (-1, 1), (5, 0), (0, 2)].iter().enumerate() {
        let r = super::rec(&mut t, i);
        r[0x34..0x36].copy_from_slice(&prio.to_le_bytes());
        r[0x36] = *func;
    }
    assert_eq!(maps::desc_list(&t), [1, 3, 0]);
}

#[test]
fn state_maps() {
    let mut t = table("states", 0x3C, vec![vec![]; 34]);
    super::rec(&mut t, 33)[0x10] = 1 << 4; // pgsv
    super::rec(&mut t, 2)[0x11] = 1 << 3; // bit 11 curse
    super::rec(&mut t, 5)[0x14] = 1 << 7; // bit 39
    super::rec(&mut t, 6)[0x2A] = 3;
    let m = maps::states(&t);
    assert_eq!((m.words, m.bitsets.len()), (2, 80));
    assert_eq!(m.bitsets[4 * 2 + 1], 1 << 1);
    assert_eq!(m.bitsets[11 * 2], 1 << 2);
    assert_eq!(m.bitsets[39 * 2], 1 << 5);
    assert_eq!((m.pgsv, m.curse, m.itemtype), (vec![33], vec![2], vec![6]));
    assert!(m.disguise.is_empty() && m.active.is_empty());
}

#[test]
fn skill_class_lists() {
    let classes = [0u8, 0xFF, 2, 0, 7, 2, 0];
    let mut t = table("skills", 572, vec![vec![]; classes.len()]);
    for (s, &c) in classes.iter().enumerate() {
        let r = super::rec(&mut t, s);
        r[0x0C] = c;
        let p: i16 = if s == 4 { 0 } else { -1 };
        r[0x94..0x96].copy_from_slice(&p.to_le_bytes());
    }
    let l = maps::skill_lists(&t);
    assert_eq!(l.counts, [3, 0, 2, 0, 0, 0, 0]);
    assert_eq!(l.max, 3);
    assert_eq!(&l.lists[..9], [0, 3, 6, 0, 0, 0, 2, 5, 0]);
    assert_eq!(l.lists.len(), 21);
    assert_eq!(l.passives, [4]);
}

#[test]
fn version0_list() {
    let w = table(
        "weapons",
        424,
        vec![rec(424, &[(0xF6, &u16s(100))]), vec![]],
    );
    let a = table("armor", 424, vec![vec![], rec(424, &[(0xF6, &u16s(1))])]);
    assert_eq!(maps::version0_items(&[&w, &a]), [1, 2, 0, 0]);
}

#[test]
fn gamble_vectors() {
    let items = table(
        "misc",
        424,
        [3u8, 1, 2]
            .iter()
            .map(|&l| rec(424, &[(0xFD, &[l])]))
            .collect(),
    );
    let mut codes = CodeLinker::default();
    for c in [b"aaa ", b"bbb ", b"ccc "] {
        codes.add(u32::from_le_bytes(*c));
    }
    let mut t = table(
        "gamble",
        12,
        [b"aaa ", b"bbb ", b"ccc "]
            .iter()
            .map(|c| rec(12, &[(0, *c)]))
            .collect(),
    );
    let g = maps::gamble(&mut t, &codes, &[&items]).unwrap();
    assert_eq!(g.index, Some(vec![1, 2, 0]));
    assert_eq!(&g.thresholds[..4], [2, 1, 2, 3]);
    assert!(g.thresholds[3..].iter().all(|&v| v == 3));
    assert_eq!((u32_at(t.record(0), 4), u32_at(t.record(0), 8)), (3, 0));
    let mut empty = table("gamble", 12, vec![]);
    let g = maps::gamble(&mut empty, &codes, &[&items]).unwrap();
    assert_eq!((g.index, g.thresholds), (None, [0; 100]));
    let mut bad = table("gamble", 12, vec![rec(12, &[(0, b"zzz ")])]);
    assert!(maps::gamble(&mut bad, &codes, &[&items]).is_err());
}

#[test]
fn monseq_index() {
    let t = table(
        "monseq",
        6,
        [0i16, 1, 1, 2]
            .iter()
            .map(|s| rec(6, &[(0, &s.to_le_bytes())]))
            .collect(),
    );
    let e = maps::monseq(&t).unwrap();
    let got: Vec<(Option<u32>, u32)> = e.iter().map(|x| (x.first, x.count)).collect();
    assert_eq!(got, [(Some(0), 1), (Some(1), 2), (Some(3), 1)]);
    let t = table(
        "monseq",
        6,
        [0i16, 3, 1]
            .iter()
            .map(|s| rec(6, &[(0, &s.to_le_bytes())]))
            .collect(),
    );
    assert!(maps::monseq(&t).is_err());
}

#[test]
fn monpreset_acts() {
    let t = table(
        "monpreset",
        4,
        [1u8, 1, 3].iter().map(|&a| vec![a]).collect(),
    );
    let a = maps::monpreset(&t).unwrap();
    assert_eq!(&a.first[..3], [Some(0), Some(2), Some(2)]);
    assert_eq!(&a.count[..3], [2, 0, 1]);
    let t = table("monpreset", 4, vec![vec![7]]);
    assert!(maps::monpreset(&t).is_err());
}

#[test]
fn hireling_tables() {
    let row = |v: u16, id: i32| rec(0x118, &[(0, &u16s(v)), (4, &id.to_le_bytes())]);
    let t = table(
        "hireling",
        0x118,
        vec![row(0, 0), row(0, 0), row(100, 0), row(0, 300), row(0, 1)],
    );
    let h = maps::hireling_first(&t).unwrap();
    assert_eq!((h[0][0], h[0][1], h[1][0], h[1][1]), (0, 4, 2, -1));
    assert!(maps::hireling_first(&table("hireling", 0x118, vec![row(0, -1)])).is_err());
}

#[test]
fn leveldefs_and_lvlsub() {
    let t = table(
        "leveldefs",
        0x9C,
        vec![vec![], rec(0x9C, &[(0x8C, &[1])]), vec![]],
    );
    assert_eq!(maps::portals(&t), [1]);
    let t = table(
        "lvlsub",
        0x15C,
        [0i32, 2, 2, 5]
            .iter()
            .map(|v| rec(0x15C, &[(0, &v.to_le_bytes())]))
            .collect(),
    );
    assert_eq!(maps::lvlsub_types(&t).unwrap(), [0, 0, 1, 0, 0, 3]);
    let t = table("lvlsub", 0x15C, vec![vec![]]);
    assert!(maps::lvlsub_types(&t).unwrap().is_empty());
}

#[test]
fn automap_conversion() {
    let row = |level: &[u8], tile: &[u8], cels: [i32; 4]| {
        let mut r = rec(0x2C, &[(0, level), (0x10, tile), (0x18, &[1, 2, 3])]);
        for (k, c) in cels.iter().enumerate() {
            r[0x1C + 4 * k..0x20 + 4 * k].copy_from_slice(&c.to_le_bytes());
        }
        r
    };
    let t = table(
        "automap",
        0x2C,
        vec![
            row(b"1 Town", b"fl", [0, 1, -1, 5]),
            row(b"1 Town", b"wl", [0, 1, 2, 3]),
            row(b"0abc", b"0", [9, -1, -1, -1]),
            row(b"1 Town", b"fi", [4, -1, -1, -1]),
        ],
    );
    let a = maps::automap(&t).unwrap();
    let r0 = &a.records[0];
    assert_eq!(i32_at(r0, 0), 1);
    assert_eq!(i32_at(r0, 4), 0);
    assert_eq!(&r0[8..12], [1, 2, 3, 0]);
    assert_eq!(i32_at(r0, 0x1C), 2);
    assert_eq!(i32_at(&a.records[1], 0x1C), 4);
    assert_eq!(i32_at(&a.records[2], 0), 0);
    assert_eq!(i32_at(&a.records[3], 4), 19);
    assert_eq!(a.ranges[1], (3, 4)); // the later run overwrites
    assert_eq!(a.ranges[0], (2, 3));
    assert_eq!(a.ranges[2], (-1, -1));
    let bad = table("automap", 0x2C, vec![row(b"6 Town", b"fl", [0; 4])]);
    assert!(maps::automap(&bad).is_err());
}
