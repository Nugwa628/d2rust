//! Unit tests for the `.d2s` reader and writer in `d2s`.
//!
//! Every save is a synthetic one built in memory from known byte layouts; no save file
//! is read or committed.

use super::*;

/// The stock `itemstatcost` save columns (359 rows; ids
/// 0–15 saved, none with `CSvParam` / `CSvSigned`), plus test overrides.
struct Tables {
    extra: Vec<(u16, StatSave)>,
}

impl Tables {
    fn v114d() -> Self {
        Tables { extra: Vec::new() }
    }
}

impl SaveTables for Tables {
    fn stat_save(&self, id: u16) -> Option<StatSave> {
        if let Some(&(_, c)) = self.extra.iter().find(|(i, _)| *i == id) {
            return Some(c);
        }
        if id >= 359 {
            return None;
        }
        let bits = match id {
            0..=4 => 10,
            5 => 8,
            6..=11 => 21,
            12 => 7,
            13 => 32,
            14 | 15 => 25,
            _ => 0,
        };
        Some(StatSave {
            bits,
            param: 0,
            signed: false,
        })
    }

    /// Test items: `4A 4D`, a length byte (the whole entry), then filler.
    fn item_entry_len(&self, buf: &[u8]) -> Result<usize, String> {
        match buf {
            [0x4A, 0x4D, n, ..] if usize::from(*n) >= 3 => Ok(usize::from(*n)),
            _ => Err("not a test item".into()),
        }
    }
}

fn item(n: u8, fill: u8) -> ItemEntry {
    let mut bytes = vec![0x4A, 0x4D, n];
    bytes.resize(usize::from(n), fill);
    ItemEntry { bytes }
}

fn stats_bytes(e: &[StatEntry]) -> Vec<u8> {
    write_stats(e, &Tables::v114d())
}

fn st(id: u16, value: i32) -> StatEntry {
    StatEntry {
        id,
        layer: 0,
        value,
    }
}

/// A level-1 character with every section, items in all lists.
fn sample(expansion: bool) -> D2s {
    let mut h = Header::default();
    h.set_name(b"Tester").unwrap();
    h.class = 4;
    h.status = if expansion { status::EXPANSION } else { 0 };
    h.create_time = 0x6000_0000;
    h.save_time = 0x6000_1000;
    h.hotkeys[0] = Slot::encode(36, true, 0).unwrap();
    h.mouse[1] = Slot::encode(0, false, 0).unwrap();
    h.towns = [0x80, 0, 0];
    h.map_seed = 0x1234_5678;
    h.hireling = Hireling {
        flags: 0,
        seed: 77,
        name_index: 3,
        id: 1,
        experience: 1000,
        rest: [0; 16],
    };
    let mut quests = Quests::default();
    quests.records[0][0] = 1;
    let mut wp = Waypoints::default();
    wp.records[0][2] |= 0x02;
    let mut npcs = Npcs::default();
    npcs.b[0][0] = 0x06;
    let body = Body {
        quests,
        waypoints: wp,
        npcs,
        stats: Stats::Bits(vec![st(0, 30), st(6, 55 << 8), st(12, 1), st(13, 500)]),
        skills: {
            let mut s = vec![0u8; 30];
            s[0] = 1;
            s
        },
        items: vec![item(10, 0xAA), item(5, 0xBB)],
        corpses: vec![Corpse {
            unk: 0,
            x: 5000,
            y: 6000,
            items: vec![item(4, 0xCC)],
        }],
        hireling_items: expansion.then(|| Some(vec![item(6, 0xDD)])),
        golem: expansion.then(|| Golem {
            flag: 1,
            item: Some(item(7, 0xEE)),
        }),
        trailing: Vec::new(),
    };
    D2s {
        header: h,
        body: Some(body),
    }
}

fn opts(expansion: bool) -> ReadOptions {
    ReadOptions {
        expansion,
        game: None,
    }
}

fn code_of(file: &[u8], o: &ReadOptions) -> Option<u8> {
    read(file, o, &Tables::v114d()).unwrap_err().internal()
}

/// Re-stores size and checksum after a hand edit.
fn fix(mut b: Vec<u8>) -> Vec<u8> {
    finish(&mut b);
    b
}

// -------------------------------------------------------------- checksum

#[test]
fn checksum_vectors() {
    assert_eq!(checksum(&[1, 2, 3]), 0x0000_000B);
    assert_eq!(checksum(&[0xFF; 4]), 0x0000_0EF1);
    // s = 0x80000000 then byte 0: rotl gives 1.
    assert_eq!(0x8000_0000u32.rotate_left(1).wrapping_add(0), 1);
}

#[test]
fn stub_vector() {
    // A known new-character file: zero header, magic, version, size,
    // "Test", status 0x21, +0x29 0x10, +0x2A 0x1E, +0x2B 1.
    let mut b = vec![0u8; HEADER_SIZE];
    b[0..4].copy_from_slice(&[0x55, 0xAA, 0x55, 0xAA]);
    b[4] = 0x60;
    b[8..10].copy_from_slice(&[0x4F, 0x01]);
    b[0x14..0x18].copy_from_slice(b"Test");
    b[0x24] = 0x21;
    b[0x29] = 0x10;
    b[0x2A] = 0x1E;
    b[0x2B] = 0x01;
    assert_eq!(checksum(&b), 0xC737_3BCA);
    let mut f = b.clone();
    finish(&mut f);
    assert_eq!(&f[0x0C..0x10], &[0xCA, 0x3B, 0x37, 0xC7]);
    let s = read(&f, &opts(true), &Tables::v114d()).unwrap();
    assert!(s.body.is_none(), "new character");
    assert_eq!(s.header.name_bytes(), b"Test");
    // The model writes the same bytes back.
    assert_eq!(write(&s, &Tables::v114d()).unwrap(), f);
}

#[test]
fn new_stub_layout() {
    let s = D2s::new_stub(b"Nec", 2, status::HARDCORE, 1234).unwrap();
    let f = write(&s, &Tables::v114d()).unwrap();
    assert_eq!(f.len(), HEADER_SIZE);
    assert_eq!(u16_at(&f, 0x24), 0x0005);
    assert_eq!(&f[0x29..0x2C], &[0x10, 0x1E, 1]);
    assert_eq!(u32_at(&f, 0x2C), 1234);
    assert_eq!(u32_at(&f, 0x30), 1234);
    assert_eq!(&f[0x88..0x98], &STUB_COMPONENTS);
    assert_eq!(&f[0x98..0xA8], &[0xFF; 16]);
    assert!(f[0x34..0x88].iter().all(|&b| b == 0));
    assert!(f[0xA8..].iter().all(|&b| b == 0));
    // Class 5 / 6 get 0x20.
    let d = D2s::new_stub(b"Dru", 5, 0, 0).unwrap();
    assert_eq!(d.header.status, 0x21);
    assert!(D2s::new_stub(b"SixteenCharsLong", 0, 0, 0).is_none());
}

// -------------------------------------------------------- header checks

#[test]
fn header_check_errors() {
    let good = write(&D2s::new_stub(b"Test", 0, 0, 0).unwrap(), &Tables::v114d()).unwrap();
    // One byte changed after the checksum → 6 → result 14.
    let mut bad = good.clone();
    bad[0x40] ^= 1;
    let e = read(&bad, &opts(false), &Tables::v114d()).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(6), Some(14)));
    // Size 336 with the checksum recomputed → 5 → result 14.
    let mut sz = good.clone();
    sz[8..12].copy_from_slice(&336u32.to_le_bytes());
    sz[0x0C..0x10].fill(0);
    let c = checksum(&sz);
    sz[0x0C..0x10].copy_from_slice(&c.to_le_bytes());
    let e = read(&sz, &opts(false), &Tables::v114d()).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(5), Some(14)));
    // Version 0x61 → 7 → result 1.
    let mut v = good.clone();
    v[4] = 0x61;
    let e = read(&fix(v), &opts(false), &Tables::v114d()).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(7), Some(1)));
}

#[test]
fn dispatch_errors() {
    let t = Tables::v114d();
    assert_eq!(
        read(&[0x55, 0xAA], &opts(false), &t),
        Err(D2sError::NotASave)
    );
    assert_eq!(D2sError::NotASave.result(), Some(9));
    let mut b = vec![0u8; 16];
    b[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    b[4] = 0x5B;
    assert_eq!(read(&b, &opts(false), &t).unwrap_err().result(), Some(9)); // legacy: shorter than 0x82
    b[4] = 0x60;
    assert_eq!(code_of(&b, &opts(false)), Some(4));
}

#[test]
fn file_cut_at_0x2000() {
    let s = sample(true);
    let mut f = write(&s, &Tables::v114d()).unwrap();
    // Pad the model past the buffer by hand: the cut copy fails the checksum.
    f.resize(MAX_FILE + 10, 0);
    finish(&mut f);
    assert_eq!(code_of(&f, &opts(true)), Some(6));
}

#[test]
fn class_and_new_flag() {
    let t = Tables::v114d();
    let mut s = D2s::new_stub(b"Test", 7, 0, 0).unwrap();
    // Class 7 passes the header check.
    assert!(read(&write(&s, &t).unwrap(), &opts(false), &t).is_ok());
    s.header.class = 8;
    let f = fix(s.header.to_bytes().to_vec());
    assert_eq!(code_of(&f, &opts(false)), Some(4));
    // New flag with more data → 2 → result 9.
    let mut f = write(&D2s::new_stub(b"Test", 0, 0, 0).unwrap(), &t).unwrap();
    f.push(0);
    let f = fix(f);
    let e = read(&f, &opts(false), &t).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(2), Some(9)));
}

#[test]
fn stale_new_flag_repair() {
    let t = Tables::v114d();
    // A full save written with the creation stub's "new character" flag
    // still set: the strict reader refuses it.
    let mut s = sample(true);
    s.header.status |= status::NEW;
    let stale = write(&s, &t).unwrap();
    let e = read(&stale, &opts(true), &t).unwrap_err();
    assert_eq!(e.internal(), Some(2));
    // The repair clears only the flag (and the checksum) and the body reads.
    let fixed = clear_stale_new_flag(&stale).expect("repairs");
    assert_eq!(fixed.len(), stale.len());
    assert_eq!(u16_at(&fixed, 0x24), u16_at(&stale, 0x24) & !status::NEW);
    let diff: Vec<usize> = (0..fixed.len()).filter(|&i| fixed[i] != stale[i]).collect();
    assert!(diff.iter().all(|&i| i == 0x24 || (0x0C..0x10).contains(&i)));
    let got = read(&fixed, &opts(true), &t).unwrap();
    s.header.status &= !status::NEW;
    assert_eq!(got.header.status, s.header.status);
    assert_eq!(write(&got, &t).unwrap(), fixed);
    // Not repaired: the bare stub (valid as is), a save without the flag,
    // a damaged file (checksum), and a non-save.
    let stub = write(&D2s::new_stub(b"Test", 0, 0, 0).unwrap(), &t).unwrap();
    assert_eq!(clear_stale_new_flag(&stub), None);
    assert_eq!(clear_stale_new_flag(&fixed), None);
    let mut bad = stale.clone();
    bad[0x40] ^= 1;
    assert_eq!(clear_stale_new_flag(&bad), None);
    assert_eq!(clear_stale_new_flag(&[0x55, 0xAA]), None);
}

#[test]
fn context_checks() {
    let mut h = Header::default();
    h.set_name(b"Hero").unwrap();
    let game = |name: &[u8], exp: bool, hc: bool, d: u8| GameContext {
        client_name: name.to_vec(),
        expansion: exp,
        hardcore: hc,
        difficulty: d,
    };
    let c = |h: &Header, g: GameContext| check_header(h, &g).err().and_then(|e| e.internal());
    assert_eq!(c(&h, game(b"hERO", false, false, 0)), None);
    assert_eq!(c(&h, game(b"", false, false, 0)), Some(3));
    assert_eq!(c(&h, game(b"Other", false, false, 0)), Some(7));
    assert_eq!(c(&h, game(b"Hero", true, false, 0)), Some(9));
    h.status = status::EXPANSION;
    assert_eq!(c(&h, game(b"Hero", false, false, 0)), Some(8));
    h.status = status::EXPANSION | status::HARDCORE | status::DEAD;
    assert_eq!(c(&h, game(b"Hero", true, true, 0)), Some(10));
    h.status = status::EXPANSION | status::HARDCORE;
    assert_eq!(c(&h, game(b"Hero", true, false, 0)), Some(11));
    h.status = status::EXPANSION;
    assert_eq!(c(&h, game(b"Hero", true, true, 0)), Some(12));
    // Progression thresholds: expansion 5 / 10, classic 4 / 8.
    h.status = status::EXPANSION | (4 << 8);
    assert_eq!(c(&h, game(b"Hero", true, false, 1)), Some(13));
    h.status = status::EXPANSION | (5 << 8);
    assert_eq!(c(&h, game(b"Hero", true, false, 1)), None);
    assert_eq!(c(&h, game(b"Hero", true, false, 2)), Some(14));
    h.status = 4 << 8;
    assert_eq!(c(&h, game(b"Hero", false, false, 1)), None);
    h.status = 7 << 8;
    assert_eq!(c(&h, game(b"Hero", false, false, 2)), Some(14));
    h.status = 8 << 8;
    assert_eq!(c(&h, game(b"Hero", false, false, 2)), None);
    // Run from `read`: the name check precedes the class check.
    let mut s = D2s::new_stub(b"Hero", 0, 0, 0).unwrap();
    s.header.class = 9;
    let f = fix(s.header.to_bytes().to_vec());
    let o = ReadOptions {
        expansion: false,
        game: Some(game(b"Other", false, false, 0)),
    };
    assert_eq!(code_of(&f, &o), Some(7));
}

#[test]
fn header_field_offsets() {
    let s = sample(true);
    let f = write(&s, &Tables::v114d()).unwrap();
    assert_eq!(&f[0x14..0x1A], b"Tester");
    assert_eq!(f[0x28], 4);
    assert_eq!(u32_at(&f, 0x34), 0xFFFF_FFFF);
    assert_eq!(&f[0x38..0x3C], &[0x24, 0x80, 0, 0]);
    assert_eq!(f[0xA8], 0x80);
    assert_eq!(u32_at(&f, 0xAB), 0x1234_5678);
    assert_eq!(u32_at(&f, 0xB3), 77);
    assert_eq!(u16_at(&f, 0xB7), 3);
    assert_eq!(u16_at(&f, 0xB9), 1);
    assert_eq!(u32_at(&f, 0xBB), 1000);
    assert!(s.header.hireling.is_present());
    assert!(!Hireling::default().is_present());
}

// ------------------------------------------------------------ hotkeys

#[test]
fn hotkey_vectors() {
    let s = Slot::encode(36, true, 0).unwrap();
    assert_eq!((s.code.to_le_bytes(), s.item), ([0x24, 0x80], 0));
    assert_eq!(s.decode(), (36, true, -1));
    let n = Slot::encode(-1, false, 0).unwrap();
    assert_eq!(n, Slot::NONE);
    assert_eq!(n.decode(), (-1, false, -1));
    assert!(Slot::encode(0x8000, false, 0).is_none());
    assert_eq!(
        Slot {
            code: 0x0005,
            item: 3
        }
        .decode(),
        (5, false, 3)
    );
}

// -------------------------------------------------------------- stats

#[test]
fn stats_vectors() {
    assert_eq!(
        stats_bytes(&[st(0, 30), st(6, 14080), st(12, 1)]),
        [0x67, 0x66, 0x00, 0x3C, 0x30, 0x00, 0x70, 0x03, 0x18, 0x04, 0xFE, 0x03]
    );
    assert_eq!(stats_bytes(&[]), [0x67, 0x66, 0xFF, 0x01]);
    assert_eq!(
        stats_bytes(&[st(14, 40_000_000)]),
        [0x67, 0x66, 0x0E, 0xFE, 0xFF, 0xFF, 0xFF, 0x07]
    );
    // CSvBits 0 and ids past the table are not written.
    assert_eq!(
        stats_bytes(&[st(16, 5), st(400, 1)]),
        [0x67, 0x66, 0xFF, 0x01]
    );
}

#[test]
fn stat_clamps() {
    let u = StatSave {
        bits: 7,
        param: 0,
        signed: false,
    };
    assert_eq!(clamp_stat(-5, u), 0);
    assert_eq!(clamp_stat(200, u), 127);
    assert_eq!(clamp_stat(50, u), 50);
    let s = StatSave { signed: true, ..u };
    assert_eq!(clamp_stat(-100, s), 0x40); // −64 in 7 bits
    assert_eq!(clamp_stat(100, s), 63);
    assert_eq!(clamp_stat(-1, s), 0x7F);
    let w = StatSave { bits: 32, ..u };
    assert_eq!(clamp_stat(-1, w), 0xFFFF_FFFF);
}

#[test]
fn stats_reader_columns() {
    // A signed stat with a param: the layer is read sign-extended, the
    // value sign-extended.
    let t = Tables {
        extra: vec![(
            20,
            StatSave {
                bits: 6,
                param: 4,
                signed: true,
            },
        )],
    };
    let mut s = sample(false);
    s.body.as_mut().unwrap().stats = Stats::Bits(vec![StatEntry {
        id: 20,
        layer: 0xFFFF,
        value: -3,
    }]);
    let f = write(&s, &t).unwrap();
    let back = read(&f, &opts(false), &t).unwrap();
    assert_eq!(
        back.body.unwrap().stats,
        Stats::Bits(vec![StatEntry {
            id: 20,
            layer: 0xFFFF,
            value: -3
        }])
    );
    // Stat 16 (CSvBits 0) on the wire → 18 → result 4: write it with an
    // override table, read with the stock one.
    let t16 = Tables {
        extra: vec![(
            16,
            StatSave {
                bits: 5,
                param: 0,
                signed: false,
            },
        )],
    };
    let mut s = sample(false);
    s.body.as_mut().unwrap().stats = Stats::Bits(vec![st(16, 1)]);
    let f = write(&s, &t16).unwrap();
    let e = read(&f, &opts(false), &Tables::v114d()).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(18), Some(4)));
}

#[test]
fn stats_without_terminator_rejected() {
    // Edge case 2: the game never ends; D2RUST rejects with 18. Build a file
    // ending inside the stats bit stream.
    let s = sample(false);
    let f = write(&s, &Tables::v114d()).unwrap();
    let cut = fix(f[..STATS_OFFSET + 4].to_vec());
    assert_eq!(code_of(&cut, &opts(false)), Some(18));
}

#[test]
fn legacy_mask_stats() {
    let mut s = sample(false);
    s.header.version = 0x5E;
    s.body.as_mut().unwrap().stats = Stats::Mask {
        mask: vec![0x01, 0x10],
        values: vec![(0, 30), (12, 1)],
    };
    let f = write(&s, &Tables::v114d()).unwrap();
    assert_eq!(
        &f[STATS_OFFSET..STATS_OFFSET + 12],
        &[0x67, 0x66, 0x01, 0x10, 0x1E, 0, 0, 0, 0x01, 0, 0, 0]
    );
    let back = read(&f, &opts(false), &Tables::v114d()).unwrap();
    assert_eq!(back.body, s.body);
    assert_eq!(write(&back, &Tables::v114d()).unwrap(), f);
    assert_eq!(
        back.body.unwrap().stats.entries(),
        vec![st(0, 30), st(12, 1)]
    );
}

// ------------------------------------------------------------- skills

#[test]
fn skills_section() {
    let mut s = sample(false);
    s.header.class = 0;
    s.body.as_mut().unwrap().skills = vec![0; 30];
    let f = write(&s, &Tables::v114d()).unwrap();
    let at = STATS_OFFSET + stats_bytes(&[st(0, 30), st(6, 55 << 8), st(12, 1), st(13, 500)]).len();
    let mut want = vec![0x69, 0x66];
    want.extend([0u8; 30]);
    assert_eq!(&f[at..at + 32], &want[..]);
    // Writer error 2 for class 7.
    s.header.class = 7;
    assert_eq!(write(&s, &Tables::v114d()), Err(WriteError::Class(7)));
    // Bad marker and a short section → 19 → result 5.
    s.header.class = 0;
    let mut f = write(&s, &Tables::v114d()).unwrap();
    f[at] = 0;
    let e = read(&fix(f), &opts(false), &Tables::v114d()).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(19), Some(5)));
    let f = write(&s, &Tables::v114d()).unwrap();
    assert_eq!(code_of(&fix(f[..at + 10].to_vec()), &opts(false)), Some(19));
}

// ------------------------------------------------------- fixed sections

#[test]
fn fixed_section_layout_and_errors() {
    let s = sample(false);
    let f = write(&s, &Tables::v114d()).unwrap();
    assert_eq!(
        &f[QUEST_OFFSET..QUEST_OFFSET + 10],
        &[0x57, 0x6F, 0x6F, 0x21, 6, 0, 0, 0, 0x2A, 0x01]
    );
    assert_eq!(f[QUEST_OFFSET + 10], 1);
    assert_eq!(
        &f[WAYPOINT_OFFSET..WAYPOINT_OFFSET + 11],
        &[0x57, 0x53, 1, 0, 0, 0, 0x50, 0, 0x02, 0x01, 0x03]
    );
    assert_eq!(&f[NPC_OFFSET..NPC_OFFSET + 4], &[0x01, 0x77, 0x34, 0]);
    assert_eq!(f[NPC_OFFSET + 0x1C], 0x06);
    assert_eq!(&f[STATS_OFFSET..STATS_OFFSET + 2], &[0x67, 0x66]);
    // Each marker broken → its code.
    for (at, code, result) in [
        (QUEST_OFFSET, 15, 2),
        (QUEST_OFFSET + 4, 15, 2),
        (WAYPOINT_OFFSET, 16, 3),
        (NPC_OFFSET, 17, 11),
        (STATS_OFFSET, 18, 4),
    ] {
        let mut b = f.clone();
        b[at] ^= 0x40;
        let e = read(&fix(b), &opts(false), &Tables::v114d()).unwrap_err();
        assert_eq!(
            (e.internal(), e.result()),
            (Some(code), Some(result)),
            "at {at:#x}"
        );
    }
    // The quest size, waypoint u32 / size / pad bytes and the NPC size are
    // not checked.
    let mut b = f.clone();
    b[QUEST_OFFSET + 8] = 0;
    b[WAYPOINT_OFFSET + 2] = 9;
    b[WAYPOINT_OFFSET + 6] = 0;
    b[WAYPOINT_OFFSET + 8 + 16] = 7;
    b[NPC_OFFSET + 2] = 0;
    assert!(read(&fix(b), &opts(false), &Tables::v114d()).is_ok());
}

#[test]
fn waypoint_record_magic() {
    let f = write(&sample(false), &Tables::v114d()).unwrap();
    for (magic, ok) in [
        ([0x02, 0x01], true),
        ([0x01, 0x01], true),
        ([0, 0], true),
        ([0x03, 0x01], false),
    ] {
        let mut b = f.clone();
        b[WAYPOINT_OFFSET + 8 + 24..WAYPOINT_OFFSET + 8 + 26].copy_from_slice(&magic);
        let r = read(&fix(b), &opts(false), &Tables::v114d());
        assert_eq!(r.is_ok(), ok, "{magic:?}");
        if !ok {
            assert_eq!(r.unwrap_err().internal(), Some(16));
        }
    }
    assert_eq!(Waypoints::bit(0), Some((2, 1)));
    assert_eq!(Waypoints::bit(9), Some((3, 2)));
    assert_eq!(Waypoints::bit(0x70), None);
}

#[test]
fn npc_bits() {
    for (class, bit) in [
        (147, 1),
        (148, 2),
        (150, 3),
        (155, 4),
        (154, 5),
        (265, 6),
        (175, 7),
        (178, 10),
        (202, 11),
        (200, 12),
        (210, 13),
        (201, 14),
        (198, 15),
        (199, 16),
        (244, 17),
        (246, 19),
        (251, 20),
        (257, 26),
        (264, 27),
        (297, 28),
        (511, 29),
        (515, 33),
        (520, 34),
        (-1, 0),
        (149, 0),
        (0, 0),
    ] {
        assert_eq!(npc_bit(class), bit, "class {class}");
    }
}

// ---------------------------------------------------------------- items

#[test]
fn item_section_bytes() {
    let mut s = sample(true);
    {
        let b = s.body.as_mut().unwrap();
        b.items.clear();
        b.corpses.clear();
        b.hireling_items = Some(None);
        b.golem = Some(Golem::default());
    }
    // No hireling in the header (`6A 66` without a list).
    s.header.hireling = Hireling::default();
    let f = write(&s, &Tables::v114d()).unwrap();
    // Empty player list, no corpse, no hireling list, no golem.
    assert!(f.ends_with(&[0x4A, 0x4D, 0, 0, 0x4A, 0x4D, 0, 0, 0x6A, 0x66, 0x6B, 0x66, 0x00]));
    // With items: counts and entries in order.
    let f = write(&sample(true), &Tables::v114d()).unwrap();
    let mut want = vec![0x4A, 0x4D, 2, 0];
    want.extend(item(10, 0xAA).bytes);
    want.extend(item(5, 0xBB).bytes);
    want.extend([0x4A, 0x4D, 1, 0, 0, 0, 0, 0]);
    want.extend(5000u32.to_le_bytes());
    want.extend(6000u32.to_le_bytes());
    want.extend([0x4A, 0x4D, 1, 0]);
    want.extend(item(4, 0xCC).bytes);
    want.extend([0x6A, 0x66, 0x4A, 0x4D, 1, 0]);
    want.extend(item(6, 0xDD).bytes);
    want.extend([0x6B, 0x66, 1]);
    want.extend(item(7, 0xEE).bytes);
    assert!(f.ends_with(&want));
}

#[test]
fn round_trip_both_games() {
    let t = Tables::v114d();
    for exp in [false, true] {
        let s = sample(exp);
        let f = write(&s, &t).unwrap();
        assert_eq!(u32_at(&f, 8) as usize, f.len());
        let back = read(&f, &opts(exp), &t).unwrap();
        let mut want = s.clone();
        want.header.file_size = f.len() as u32;
        want.header.checksum = u32_at(&f, 0x0C);
        assert_eq!(back, want);
        assert_eq!(write(&back, &t).unwrap(), f);
    }
    // A classic game does not read jf / kf: they stay as trailing bytes.
    let f = write(&sample(true), &t).unwrap();
    let back = read(&f, &opts(false), &t).unwrap();
    let b = back.body.as_ref().unwrap();
    assert_eq!(b.hireling_items, None);
    assert_eq!(&b.trailing[..2], &[0x6A, 0x66]);
    assert_eq!(write(&back, &t).unwrap(), f);
}

#[test]
fn player_list_errors() {
    let t = Tables::v114d();
    let f = write(&sample(false), &t).unwrap();
    let at = f.windows(4).position(|w| w == [0x4A, 0x4D, 2, 0]).unwrap();
    let mut b = f.clone();
    b[at] = 0;
    let e = read(&fix(b), &opts(false), &t).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(20), Some(7)));
    // An entry that does not decode.
    let mut b = f.clone();
    b[at + 4] = 0;
    assert_eq!(code_of(&fix(b), &opts(false)), Some(20));
    // An entry longer than the file.
    let mut b = f.clone();
    b[at + 6] = 0xFF;
    assert_eq!(code_of(&fix(b), &opts(false)), Some(20));
}

#[test]
fn corpse_rules() {
    let t = Tables::v114d();
    let mut s = sample(false);
    s.body.as_mut().unwrap().corpses[0].unk = 0xDEAD_BEEF;
    let f = write(&s, &t).unwrap();
    // The first u32 is skipped by the loader but kept by the model.
    assert_eq!(
        read(&f, &opts(false), &t).unwrap().body.unwrap().corpses[0].unk,
        0xDEAD_BEEF
    );
    // Corpse count 2 → 21 → result 8.
    let at = f.windows(4).position(|w| w == [0x4A, 0x4D, 1, 0]).unwrap();
    let mut b = f.clone();
    b[at + 2] = 2;
    let e = read(&fix(b), &opts(false), &t).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(21), Some(8)));
    // Marker and the corpse item list.
    let mut b = f.clone();
    b[at + 1] = 0;
    assert_eq!(code_of(&fix(b), &opts(false)), Some(21));
    let mut b = f.clone();
    b[at + 16] = 0;
    assert_eq!(code_of(&fix(b), &opts(false)), Some(21));
}

#[test]
fn expansion_sections() {
    let t = Tables::v114d();
    let f = write(&sample(true), &t).unwrap();
    let jf = f.windows(2).rposition(|w| w == [0x6A, 0x66]).unwrap();
    let kf = f.windows(2).rposition(|w| w == [0x6B, 0x66]).unwrap();
    // Missing jf and kf (fewer than 2 bytes left) → absent, no error.
    let cut = fix(f[..jf].to_vec());
    let s = read(&cut, &opts(true), &t).unwrap();
    assert_eq!(
        (
            s.body.as_ref().unwrap().hireling_items.clone(),
            s.body.unwrap().golem
        ),
        (None, None)
    );
    let cut = fix(f[..kf + 1].to_vec());
    let s = read(&cut, &opts(true), &t).unwrap();
    assert!(s.body.as_ref().unwrap().golem.is_none());
    assert_eq!(s.body.unwrap().trailing, vec![0x6B]);
    // Bad markers.
    let mut b = f.clone();
    b[jf] = 0;
    let e = read(&fix(b), &opts(true), &t).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(22), Some(10)));
    let mut b = f.clone();
    b[kf] = 0;
    let e = read(&fix(b), &opts(true), &t).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(23), Some(10)));
    // No hireling in the header → no list is read after jf.
    let mut s = sample(true);
    s.header.hireling = Hireling::default();
    s.body.as_mut().unwrap().hireling_items = Some(None);
    let f2 = write(&s, &t).unwrap();
    let back = read(&f2, &opts(true), &t).unwrap();
    assert_eq!(back.body.as_ref().unwrap().hireling_items, Some(None));
    assert_eq!(back.body.unwrap().golem.unwrap().item, Some(item(7, 0xEE)));
    // g = 0: no item, the cursor moves past g.
    let mut s = sample(true);
    s.body.as_mut().unwrap().golem = Some(Golem::default());
    let f3 = write(&s, &t).unwrap();
    let back = read(&f3, &opts(true), &t).unwrap();
    assert_eq!(back.body.as_ref().unwrap().golem, Some(Golem::default()));
    assert!(back.body.unwrap().trailing.is_empty());
    // Golem item that does not decode → 23.
    let mut b = f.clone();
    b[kf + 3] = 0;
    assert_eq!(code_of(&fix(b), &opts(true)), Some(23));
}

#[test]
fn writer_overflow() {
    let mut s = sample(false);
    s.body.as_mut().unwrap().items = (0..40).map(|_| item(250, 0)).collect();
    assert!(matches!(
        write(&s, &Tables::v114d()),
        Err(WriteError::Overflow(_))
    ));
}

#[test]
fn result_table() {
    let want = [
        0, 14, 9, 14, 9, 14, 14, 1, 24, 23, 21, 20, 19, 17, 18, 2, 3, 11, 4, 5, 7, 8, 10, 10, 14,
        26, 25,
    ];
    for (i, &r) in want.iter().enumerate() {
        assert_eq!(load_result(i as u8), r);
    }
    assert_eq!(load_result(27), 1);
}

// The checksum catches a changed byte in every section.
#[test]
fn checksum_catches_every_byte() {
    let f = write(&sample(true), &Tables::v114d()).unwrap();
    for at in (0..f.len()).filter(|&a| !(0x0C..0x10).contains(&a)) {
        let mut b = f.clone();
        b[at] ^= 0x01;
        assert!(
            read(&b, &opts(true), &Tables::v114d()).is_err(),
            "byte {at:#x}"
        );
    }
}

// ------------------------------------------------- load order and edge cases

/// The stock tables, but the loader finds no hireling row (a missing-row case).
struct NoHirelingRow(Tables);

impl SaveTables for NoHirelingRow {
    fn stat_save(&self, id: u16) -> Option<StatSave> {
        self.0.stat_save(id)
    }
    fn item_entry_len(&self, buf: &[u8]) -> Result<usize, String> {
        self.0.item_entry_len(buf)
    }
    fn hireling_restored(&self, _: &Hireling) -> bool {
        false
    }
}

/// Offset of the `if` marker in a `sample` file.
fn skills_at() -> usize {
    STATS_OFFSET + stats_bytes(&[st(0, 30), st(6, 55 << 8), st(12, 1), st(13, 500)]).len()
}

#[test]
fn header_check_order_and_unread_fields() {
    let t = Tables::v114d();
    let good = write(&D2s::new_stub(b"Test", 0, 0, 0).unwrap(), &t).unwrap();
    // The header length before the checksum: a short buffer with a wrong checksum → 4.
    let mut short = good[..0x100].to_vec();
    short[0x0C] ^= 1;
    assert_eq!(code_of(&short, &opts(false)), Some(4));
    // Checksum before version: bad checksum and version 0x61 → 6.
    let mut b = good.clone();
    b[4] = 0x61;
    assert_eq!(code_of(&b, &opts(false)), Some(6));
    // Size before version: size 336 (checksum recomputed) and 0x61 → 5.
    let mut b = good.clone();
    b[4] = 0x61;
    b[8..12].copy_from_slice(&336u32.to_le_bytes());
    b[0x0C..0x10].fill(0);
    let c = checksum(&b);
    b[0x0C..0x10].copy_from_slice(&c.to_le_bytes());
    assert_eq!(code_of(&b, &opts(false)), Some(5));
    // Version before the name: 0x61 and a wrong client name → 7 (version).
    let other = ReadOptions {
        expansion: false,
        game: Some(GameContext {
            client_name: b"Other".to_vec(),
            ..GameContext::default()
        }),
    };
    let mut b = good.clone();
    b[4] = 0x61;
    let e = read(&fix(b), &other, &t).unwrap_err();
    assert_eq!(e, fail(7, "version", 0x04));
    // The status checks before the class: expansion character in a
    // classic game with class 9 → 8.
    let mut b = good.clone();
    b[0x24] |= 0x20;
    b[0x28] = 9;
    let ctx = ReadOptions {
        expansion: false,
        game: Some(GameContext {
            client_name: b"Test".to_vec(),
            ..GameContext::default()
        }),
    };
    assert_eq!(code_of(&fix(b), &ctx), Some(8));
    // Not read on load: +0x29, +0x2B, +0x30, +0x34, +0x88..+0xA7, +0xD0..
    let f = write(&sample(false), &t).unwrap();
    let mut b = f.clone();
    b[0x29] = 0;
    b[0x2B] = 99;
    b[0x30..0x38].fill(0x5A);
    b[0x88..0xA8].fill(0x33);
    b[0xD0..HEADER_SIZE].fill(0x77);
    let ctx = ReadOptions {
        expansion: false,
        game: Some(GameContext {
            client_name: b"tester".to_vec(),
            ..GameContext::default()
        }),
    };
    let back = read(&fix(b), &ctx, &t).unwrap();
    assert_eq!(back.body, read(&f, &ctx, &t).unwrap().body);
}

#[test]
fn hireling_block_layout() {
    let t = Tables::v114d();
    let mut s = sample(false);
    s.header.hireling = Hireling {
        flags: Hireling::DEAD,
        seed: 0x0102_0304,
        name_index: 0x0506,
        id: 0x0708,
        experience: 0x090A_0B0C,
        rest: [0; 16],
    };
    let f = write(&s, &t).unwrap();
    assert_eq!(
        &f[0xAF..0xCF],
        &[
            0x00, 0x00, 0x01, 0x00, // flags 0x10000
            0x04, 0x03, 0x02, 0x01, // seed
            0x06, 0x05, // name index
            0x08, 0x07, // Id
            0x0C, 0x0B, 0x0A, 0x09, // experience
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // +0xBF: 16 zero bytes
        ]
    );
    let back = read(&f, &opts(false), &t).unwrap();
    assert_eq!(back.header.hireling, s.header.hireling);
}

#[test]
fn quest_section_layout() {
    let t = Tables::v114d();
    let mut s = sample(false);
    {
        let q = &mut s.body.as_mut().unwrap().quests;
        q.records[0][95] = 0x11;
        q.records[1][0] = 0x22;
        q.records[1][95] = 0x33;
        q.records[2][0] = 0x44;
        q.records[2][95] = 0x55;
    }
    let f = write(&s, &t).unwrap();
    let q = &f[QUEST_OFFSET..QUEST_OFFSET + QUEST_SIZE];
    assert_eq!(u32_at(q, 0), 0x216F_6F57);
    assert_eq!(u32_at(q, 4), 6);
    assert_eq!(u16_at(q, 8), 0x012A);
    // Record d at +10 + 96d.
    assert_eq!((q[10], q[10 + 95]), (1, 0x11));
    assert_eq!((q[106], q[106 + 95]), (0x22, 0x33));
    assert_eq!((q[202], q[202 + 95]), (0x44, 0x55));
    // 10 + 3 × 96 = 298: the waypoint section follows.
    assert_eq!(QUEST_OFFSET + 10 + 3 * 96, WAYPOINT_OFFSET);
    assert_eq!(&f[WAYPOINT_OFFSET..WAYPOINT_OFFSET + 2], b"WS");
    let back = read(&f, &opts(false), &t).unwrap();
    assert_eq!(back.body.unwrap().quests, s.body.unwrap().quests);
}

#[test]
fn stats_layout_by_version_and_table() {
    let t = Tables::v114d();
    // Version 0x5F takes the bit-field layout, like 0x60.
    let mut s = sample(false);
    s.header.version = 0x5F;
    let f = write(&s, &t).unwrap();
    assert_eq!(
        &f[STATS_OFFSET..STATS_OFFSET + 4],
        &stats_bytes(&[st(0, 30), st(6, 55 << 8), st(12, 1), st(13, 500)])[..4]
    );
    let back = read(&f, &opts(false), &t).unwrap();
    assert_eq!(back.body, s.body);
    // The widths come from the loaded table: strength patched to 12 bits.
    let patched = Tables {
        extra: vec![(
            0,
            StatSave {
                bits: 12,
                param: 0,
                signed: false,
            },
        )],
    };
    // 9 bits id 0, 12 bits 30, 9 bits 0x1FF = 30 bits.
    assert_eq!(
        write_stats(&[st(0, 30)], &patched),
        [0x67, 0x66, 0x00, 0x3C, 0xE0, 0x3F]
    );
    assert_eq!(
        write_stats(&[st(0, 30)], &t),
        [0x67, 0x66, 0x00, 0x3C, 0xF8, 0x0F]
    );
    let mut s = sample(false);
    s.body.as_mut().unwrap().stats = Stats::Bits(vec![st(0, 3000)]);
    let f = write(&s, &patched).unwrap();
    let back = read(&f, &opts(false), &patched).unwrap();
    assert_eq!(back.body.unwrap().stats, Stats::Bits(vec![st(0, 3000)]));
    // The same bytes misread with the stock widths do not give 3000.
    assert_ne!(
        read(&f, &opts(false), &t)
            .ok()
            .and_then(|d| d.body)
            .map(|b| b.stats),
        Some(Stats::Bits(vec![st(0, 3000)]))
    );
}

#[test]
fn class_7_has_no_skill_list() {
    let t = Tables::v114d();
    let mut s = sample(false);
    s.header.class = 0;
    let mut f = write(&s, &t).unwrap();
    f[0x28] = 7;
    // The header check passes and the file loads ...
    let back = read(&fix(f), &opts(false), &t).unwrap();
    assert_eq!(back.header.class, 7);
    // ... but there is no class list to write the skills from.
    assert_eq!(write(&back, &t), Err(WriteError::Class(7)));
}

#[test]
fn item_that_does_not_fit_fails() {
    let t = Tables::v114d();
    let mut s = sample(false);
    s.body.as_mut().unwrap().items.clear();
    let base = write(&s, &t).unwrap().len();
    // Fill the player list up to exactly 0x2000 bytes.
    let mut left = MAX_FILE - base;
    let mut items = Vec::new();
    while left > 0 {
        let mut n = left.min(250);
        if (1..3).contains(&(left - n)) {
            n -= 3;
        }
        items.push(item(n as u8, 0x11));
        left -= n;
    }
    s.body.as_mut().unwrap().items = items;
    let f = write(&s, &t).unwrap();
    assert_eq!(f.len(), MAX_FILE);
    assert!(read(&f, &opts(false), &t).is_ok());
    // One more byte in the last item no longer fits.
    let last = s.body.as_mut().unwrap().items.last_mut().unwrap();
    let n = last.bytes.len() as u8 + 1;
    *last = item(n, 0x11);
    assert_eq!(write(&s, &t), Err(WriteError::Overflow(MAX_FILE + 1)));
}

#[test]
fn load_sequence_after_stats() {
    let t = Tables::v114d();
    let f = write(&sample(true), &t).unwrap();
    let skills = skills_at();
    let items = skills + 32;
    let corpse = items + 4 + 10 + 5;
    let jf = f.windows(2).rposition(|w| w == [0x6A, 0x66]).unwrap();
    let kf = f.windows(2).rposition(|w| w == [0x6B, 0x66]).unwrap();
    assert_eq!(&f[skills..skills + 2], b"if");
    assert_eq!(&f[items..items + 4], &[0x4A, 0x4D, 2, 0]);
    assert_eq!(&f[corpse..corpse + 4], &[0x4A, 0x4D, 1, 0]);
    // Two sections broken: the earlier one in the sequence reports.
    for (a, b, code) in [
        (skills, items, 19),
        (items, corpse, 20),
        (corpse, jf, 21),
        (jf, kf, 22),
    ] {
        let mut x = f.clone();
        x[a] ^= 0x40;
        x[b] ^= 0x40;
        assert_eq!(code_of(&fix(x), &opts(true)), Some(code), "{a:#x}/{b:#x}");
    }
    // The hireling from the header decides whether the jf list is read
    // before the golem.
    let mut s = sample(true);
    s.header.hireling = Hireling::default();
    s.body.as_mut().unwrap().hireling_items = Some(None);
    let back = read(&write(&s, &t).unwrap(), &opts(true), &t).unwrap();
    assert_eq!(back.body.unwrap().golem.unwrap().flag, 1);
}

#[test]
fn section_order_and_missing_fixed_sections() {
    let t = Tables::v114d();
    let f = write(&sample(false), &t).unwrap();
    let cut = |from: usize, len: usize| {
        let mut b = f[..from].to_vec();
        b.extend_from_slice(&f[from + len..]);
        fix(b)
    };
    // A missing fixed section: the next section sits at its offset.
    assert_eq!(
        code_of(&cut(QUEST_OFFSET, QUEST_SIZE), &opts(false)),
        Some(15)
    );
    assert_eq!(
        code_of(&cut(WAYPOINT_OFFSET, WAYPOINT_SIZE), &opts(false)),
        Some(16)
    );
    assert_eq!(code_of(&cut(NPC_OFFSET, NPC_SIZE), &opts(false)), Some(17));
    // Waypoint and NPC sections swapped: fails on the waypoint marker.
    let mut b = f[..WAYPOINT_OFFSET].to_vec();
    b.extend_from_slice(&f[NPC_OFFSET..NPC_OFFSET + NPC_SIZE]);
    b.extend_from_slice(&f[WAYPOINT_OFFSET..NPC_OFFSET]);
    b.extend_from_slice(&f[NPC_OFFSET + NPC_SIZE..]);
    assert_eq!(code_of(&fix(b), &opts(false)), Some(16));
    // Skills before stats: fails on the stats marker.
    let skills = skills_at();
    let mut b = f[..STATS_OFFSET].to_vec();
    b.extend_from_slice(&f[skills..skills + 32]);
    b.extend_from_slice(&f[STATS_OFFSET..skills]);
    b.extend_from_slice(&f[skills + 32..]);
    assert_eq!(code_of(&fix(b), &opts(false)), Some(18));
}

#[test]
fn mask_stats_bound_counts_whole_last_byte() {
    let t = Tables::v114d();
    let mk = |mask: Vec<u8>| {
        let mut s = sample(false);
        s.header.version = 0x5E;
        s.header.stat_count = 9; // m = 2 mask bytes, bits 9..15 unused
        s.header.skill_count = 0;
        let b = s.body.as_mut().unwrap();
        b.stats = Stats::Mask {
            mask,
            values: vec![(0, 30), (8, 5)],
        };
        b.skills.clear();
        b.items.clear();
        b.corpses.clear();
        s
    };
    // Bit 9 (≥ k) set: counted by the bound (2 + 2 + 12 ≤ 22 bytes left),
    // but no value is consumed for it; the next section follows two values.
    let s = mk(vec![0x01, 0x03]);
    let f = write(&s, &t).unwrap();
    assert_eq!(f.len() - STATS_OFFSET, 2 + 2 + 8 + 2 + 4 + 4);
    let back = read(&f, &opts(false), &t).unwrap();
    assert_eq!(back.body, s.body);
    // Bits 9..15 all set: the bound needs 2 + 2 + 4 × 9 = 40 bytes but 22
    // are left → 18, although only two values would be read.
    let f = write(&mk(vec![0x01, 0xFF]), &t).unwrap();
    assert_eq!(code_of(&f, &opts(false)), Some(18));
}

#[test]
fn csvparam_layer_sign_extended() {
    let t = Tables {
        extra: vec![(
            20,
            StatSave {
                bits: 6,
                param: 4,
                signed: false,
            },
        )],
    };
    for (layer, want) in [(0x0008, 0xFFF8), (0x0007, 0x0007), (0x000F, 0xFFFF)] {
        let mut s = sample(false);
        s.body.as_mut().unwrap().stats = Stats::Bits(vec![StatEntry {
            id: 20,
            layer,
            value: 5,
        }]);
        let f = write(&s, &t).unwrap();
        let back = read(&f, &opts(false), &t).unwrap();
        assert_eq!(
            back.body.unwrap().stats,
            Stats::Bits(vec![StatEntry {
                id: 20,
                layer: want,
                value: 5
            }]),
            "layer {layer:#x}"
        );
    }
}

#[test]
fn missing_hireling_row_fails_on_kf() {
    let f = write(&sample(true), &Tables::v114d()).unwrap();
    assert!(read(&f, &opts(true), &Tables::v114d()).is_ok());
    // The header has a hireling, but its row is missing: the jf list is not
    // consumed and the kf check sees its `JM` → 23.
    let e = read(&f, &opts(true), &NoHirelingRow(Tables::v114d())).unwrap_err();
    assert_eq!((e.internal(), e.result()), (Some(23), Some(10)));
}

// ------------------------------------------------- item and appearance details

#[test]
fn hireling_without_items_writes_an_empty_list() {
    // Expansion, hireling present with no items, no golem →
    // `6A 66 4A 4D 00 00 6B 66 00`.
    let mut s = sample(true);
    {
        let b = s.body.as_mut().unwrap();
        b.hireling_items = Some(Some(Vec::new()));
        b.golem = Some(Golem::default());
    }
    let t = Tables::v114d();
    let f = write(&s, &t).unwrap();
    assert!(f.ends_with(&[0x6A, 0x66, 0x4A, 0x4D, 0x00, 0x00, 0x6B, 0x66, 0x00]));
    let back = read(
        &f,
        &ReadOptions {
            expansion: true,
            game: None,
        },
        &t,
    )
    .unwrap();
    assert_eq!(back.body.unwrap().hireling_items, Some(Some(Vec::new())));
}

#[test]
fn item_flags_after_a_load() {
    // Known flags 0x00A02010 (compact) and 0x00802010 (full), loaded
    // and saved again → 0x00A00010 and 0x00800010. The writer clears
    // 0x80000 and sets 0x800000.
    let save = |f: u32| (f & !ITEM_FLAG_LOADED) | 0x80_0000;
    assert_eq!(item_flags_on_load(0x00A0_2010), 0x00A8_0010);
    assert_eq!(save(item_flags_on_load(0x00A0_2010)), 0x00A0_0010);
    assert_eq!(save(item_flags_on_load(0x0080_2010)), 0x0080_0010);
    // The decode drops a stored 0x80000 and the alt-code bit.
    assert_eq!(item_flags_on_load(0x0288_2010), 0x0088_0010);
    // Flags without 0x2000 only gain 0x80000.
    assert_eq!(item_flags_on_load(0x0080_0010), 0x0088_0010);
}

#[test]
fn record_flags_sit_after_the_marker() {
    let mut e = ItemEntry {
        bytes: vec![0x4A, 0x4D, 0x10, 0x20, 0xA0, 0x00, 0x65, 0x00],
    };
    assert_eq!(e.record_flags(0), Some(0x00A0_2010));
    e.set_record_flags(0, 0x00A0_0010).unwrap();
    assert_eq!(e.bytes, [0x4A, 0x4D, 0x10, 0x00, 0xA0, 0x00, 0x65, 0x00]);
    // No marker, or too short: none.
    assert_eq!(e.record_flags(1), None);
    assert_eq!(e.set_record_flags(4, 0), None);
    assert_eq!(e.bytes[4..], [0xA0, 0x00, 0x65, 0x00]);
}

#[test]
fn appearance_bytes_reset_to_ff() {
    // +0x88..+0x97 = the stub's bytes, no equipped item,
    // loaded and saved → 32 × FF.
    let mut s = D2s::new_stub(b"Stub", 2, 0, 1).unwrap();
    assert_eq!(s.header.components, STUB_COMPONENTS);
    s.header.colours[3] = 7;
    s.header.reset_appearance();
    let b = s.header.to_bytes();
    assert_eq!(b[0x88..0xA8], [0xFF; 32]);
}

#[test]
fn field_a_setter_sets_the_first_matching_bit() {
    // Confirmed on a save: Kashya (class 150, bit 3) in Normal → A of
    // difficulty 0 is `08 00 …`, B unchanged.
    let mut n = Npcs::default();
    n.set_intro_a(0, 150).unwrap();
    assert_eq!(n.a[0], [0x08, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(n.b, [[0; 8]; 3]);
    // A range pair: class 177 → bit 9 (byte 1, 0x02); only that bit.
    n.set_intro_a(1, 177).unwrap();
    assert_eq!(n.a[1], [0, 0x02, 0, 0, 0, 0, 0, 0]);
    // No pair matches → bit 0 only.
    n.set_intro_a(2, 999).unwrap();
    assert_eq!(n.a[2], [0x01, 0, 0, 0, 0, 0, 0, 0]);
    // Class 520 → bit 34 (byte 4, 0x04).
    n.set_intro_a(2, 520).unwrap();
    assert_eq!(n.a[2], [0x01, 0, 0, 0, 0x04, 0, 0, 0]);
    assert_eq!(n.set_intro_a(3, 150), None);
}

#[test]
fn hireling_items_written_by_the_writer_round_trip_through_the_loader() {
    let t = Tables::v114d();
    let entries = vec![item(9, 0x11), item(6, 0x33)];
    let mut s = sample(true);
    s.body
        .as_mut()
        .unwrap()
        .set_hireling_items(true, Some(entries.clone()));
    let f = write(&s, &t).unwrap();
    let at = f
        .windows(4)
        .position(|w| w == [0x6A, 0x66, 0x4A, 0x4D])
        .expect("the marker, then the hireling's list");
    assert_eq!(&f[at + 4..at + 6], &[2, 0]);
    let back = read(&f, &opts(true), &t).unwrap();
    assert_eq!(
        back.body.unwrap().hireling_items,
        Some(Some(entries.clone()))
    );

    // No hireling node: the marker alone, straight before `kf`.
    let mut s = sample(true);
    s.header.hireling = Hireling::default();
    s.body.as_mut().unwrap().set_hireling_items(true, None);
    let f = write(&s, &t).unwrap();
    assert!(f.windows(4).any(|w| w == [0x6A, 0x66, 0x6B, 0x66]));
    let back = read(&f, &opts(true), &t).unwrap();
    assert_eq!(back.body.unwrap().hireling_items, Some(None));

    // Classic game: no hireling item section at all.
    let mut s = sample(false);
    s.body
        .as_mut()
        .unwrap()
        .set_hireling_items(false, Some(entries));
    assert_eq!(s.body.as_ref().unwrap().hireling_items, None);
    let f = write(&s, &t).unwrap();
    assert!(!f.windows(2).any(|w| w == [0x6A, 0x66]));
}

// A save D2RUST writes must load in the game, so a model the loader could not
// frame is refused instead of written.
#[test]
fn writer_refuses_models_the_loader_cannot_frame() {
    let t = Tables::v114d();
    let refused = |s: &D2s| matches!(write(s, &t), Err(WriteError::Model(_)));
    assert!(!refused(&sample(true)));
    // Skills: the loader reads header +0x2A bytes.
    let mut s = sample(true);
    s.body.as_mut().unwrap().skills.pop();
    assert!(refused(&s));
    // Corpse: n >= 2 is error 21.
    let mut s = sample(true);
    let c = s.body.as_ref().unwrap().corpses[0].clone();
    s.body.as_mut().unwrap().corpses.push(c);
    assert!(refused(&s));
    // Golem: g and the item go together (g = 1 without an item reads the
    // next section as the item; an item with g = 0 is read as a marker).
    let mut s = sample(true);
    s.body.as_mut().unwrap().golem = Some(Golem {
        flag: 1,
        item: None,
    });
    assert!(refused(&s));
    let mut s = sample(true);
    s.body.as_mut().unwrap().golem = Some(Golem {
        flag: 0,
        item: Some(item(7, 0)),
    });
    assert!(refused(&s));
    // jf: a hireling in the header needs the list, none in the header
    // must not have one (the loader reads the list iff it restores it).
    let mut s = sample(true);
    s.body.as_mut().unwrap().hireling_items = Some(None);
    assert!(refused(&s));
    let mut s = sample(true);
    s.header.hireling = Hireling::default();
    assert!(refused(&s));
    // Absent jf is fine for either header (the loader tolerates it).
    let mut s = sample(true);
    s.body.as_mut().unwrap().hireling_items = None;
    assert!(!refused(&s));
}

#[test]
fn swap_pairs_and_switch_byte_sit_at_their_offsets() {
    let mut h = Header {
        weapon_switch: 1,
        ..Header::default()
    };
    h.mouse[0] = Slot {
        code: 0x0024,
        item: 0,
    };
    h.mouse[2] = Slot {
        code: 0x0024,
        item: 0,
    };
    let b = h.to_bytes();
    assert_eq!(&b[0x10..0x14], &[1, 0, 0, 0]);
    assert_eq!(&b[0x78..0x7C], &[0x24, 0, 0, 0]);
    assert_eq!(&b[0x80..0x84], &[0x24, 0, 0, 0]);
    assert_eq!(&b[0x84..0x88], &[0, 0, 0, 0]);
}

#[test]
fn skills_are_known_when_a_later_section_fails() {
    let t = Tables::v114d();
    let good = write(&sample(true), &t).unwrap();
    let want = read(&good, &opts(true), &t).unwrap().body.unwrap().skills;
    // The player item list's marker broken (internal 20): the skills read
    // before it are still reported.
    let at = (0..good.len() - 1)
        .find(|&i| &good[i..i + 2] == b"JM")
        .unwrap();
    let mut bad = good.clone();
    bad[at] = 0;
    let bad = fix(bad);
    assert_eq!(code_of(&bad, &opts(true)), Some(20));
    assert_eq!(skills_before_failure(&bad, &t), Some(want));
    // A failure before the skills (quests, internal 15) reports none.
    let mut early = good;
    early[0x14F] = 0;
    let early = fix(early);
    assert_eq!(code_of(&early, &opts(true)), Some(15));
    assert_eq!(skills_before_failure(&early, &t), None);
}

/// The unlocked-difficulty mask: Nightmare at progression 4 (classic) / 5
/// (expansion), Hell at 8 / 10; Normal always.
#[test]
fn difficulty_mask_follows_progression() {
    use super::status::{self, difficulties, difficulty_open};
    let st = |p: u16, exp: bool| (p << 8) | if exp { status::EXPANSION } else { 0 };
    let cases = [
        (0, false, 0b001),
        (3, false, 0b001),
        (4, false, 0b011),
        (7, false, 0b011),
        (8, false, 0b111),
        (0, true, 0b001),
        (4, true, 0b001),
        (5, true, 0b011),
        (9, true, 0b011),
        (10, true, 0b111),
        (15, true, 0b111),
    ];
    for (p, exp, want) in cases {
        assert_eq!(
            difficulties(st(p, exp)),
            want,
            "progression {p} expansion {exp}"
        );
    }
    // Other status bits do not change the mask.
    assert_eq!(
        difficulties(st(5, true) | status::HARDCORE | status::DEAD | status::LADDER),
        0b011
    );
    assert!(difficulty_open(0b011, 1));
    assert!(!difficulty_open(0b011, 2));
    assert!(!difficulty_open(0b111, 3));
}
