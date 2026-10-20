//! `d2s-tool` on the synthetic install (`test_fixtures`: made-up tables,
//! no Blizzard data), in CI. Saves are built in memory (or in the test's
//! temp dir for the command line) and never committed.

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_formats::d2s::{self, status, D2s, Stats, Waypoints};
use d2_proto::item_bits::{save_entry_len, Location};
use d2_server::adapters::item_bits::TablesLookup;
use d2s_tool::items::ItemSpec;
use d2s_tool::save::{apply, new_save, Edits, QuestSpec, WpSpec};
use d2s_tool::tables::Tables;
use d2s_tool::{read_options, round_trip, RoundTrip};
use test_fixtures::{install, synth};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("d2s-tool-{}", std::process::id()))
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let i = install::build(&dir(), &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
        let anim = d2_data::fixup::read_animdata(&i.archives).unwrap_or_else(|e| panic!("{e}"));
        let mut t = Tables::from_loaded(&i.loaded, &anim).unwrap_or_else(|e| panic!("{e}"));
        // The synthetic itemstatcost gives every stat 10 save bits (32
        // for experience): too narrow for life in 1/256 points. Stats
        // 0–15 get the widths the stock tables use, so the
        // creation values fit (the tool refuses a value that would clamp).
        for (id, bits) in [10, 10, 10, 10, 10, 8, 21, 21, 21, 21, 21, 21, 7, 32, 25, 25]
            .into_iter()
            .enumerate()
        {
            t.isc_save[id].bits = bits;
        }
        t
    })
}

/// A class whose skill list is as long as the largest (the skills
/// section frames).
fn class() -> u8 {
    let t = tables();
    (0..7u8)
        .find(|&c| t.class_skills(c).len() == t.max_skill_count())
        .expect("a class with the largest skill list")
}

fn edits() -> Edits {
    Edits {
        name: Some("Tester".to_owned()),
        class: Some(class()),
        level: Some(5),
        expansion: true,
        hardcore: Some(true),
        gold: Some(1234),
        time: Some(0x1234_5678),
        ..Edits::default()
    }
}

fn write(s: &D2s) -> Vec<u8> {
    d2s::write(s, tables()).unwrap_or_else(|e| panic!("{e}"))
}

fn read(b: &[u8]) -> D2s {
    d2s::read(b, &read_options(b, None), tables()).unwrap_or_else(|e| panic!("{e}"))
}

fn stat(s: &D2s, id: u16) -> Option<i32> {
    match &s.body.as_ref().expect("body").stats {
        Stats::Bits(v) => v.iter().find(|e| e.id == id).map(|e| e.value),
        Stats::Mask { .. } => panic!("mask layout"),
    }
}

/// A new save reads back in section order and rewrites byte for byte;
/// the size and checksum the writer stores are what the loader checks.
#[test]
fn new_save_round_trips() {
    let mut e = edits();
    e.items = vec!["sb1".parse().unwrap(), "ar1@4,0".parse().unwrap()];
    let save = new_save(&e, tables()).unwrap();
    let bytes = write(&save);
    assert_eq!(&bytes[0..4], &[0x55, 0xAA, 0x55, 0xAA]);
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
        bytes.len()
    );
    let back = read(&bytes);
    assert_eq!(write(&back), bytes);
    assert!(matches!(
        round_trip(&bytes, &read_options(&bytes, None), tables()).unwrap(),
        RoundTrip::Same(_)
    ));
    // The model read back is the model written, apart from +0x08 / +0x0C.
    let mut h = back.header.clone();
    h.file_size = 0;
    h.checksum = 0;
    assert_eq!(h, save.header);
    assert_eq!(back.body, save.body);
}

/// Level, experience, gold and the creation values appear in the stats
/// section, ascending, life in 1/256 points; stats without `CSvBits`
/// (tohit, nextexp, …) are not written.
#[test]
fn stats_level_and_gold_in_stats_section() {
    let t = tables();
    let s = read(&write(&new_save(&edits(), t).unwrap()));
    let v = t.vitals.as_ref().unwrap();
    let c = class();
    let cs = v.charstats(i32::from(c)).unwrap();
    assert_eq!(stat(&s, 12), Some(5));
    assert_eq!(s.header.level, 5);
    assert_eq!(stat(&s, 13), Some(v.threshold(i32::from(c), 4) as i32));
    assert_eq!(stat(&s, 14), Some(1234));
    assert_eq!(stat(&s, 0), Some(i32::from(cs.str)));
    // Creation life and level-ups: (vit + hpadd) << 8, plus LifePerLevel × 4 << 6.
    let maxhp =
        ((i32::from(cs.vit) + i32::from(cs.hpadd)) << 8) + ((i32::from(cs.lifeperlevel) * 4) << 6);
    assert_eq!(stat(&s, 7), Some(maxhp));
    assert_eq!(stat(&s, 6), Some(maxhp));
    // statpts: StatPerLevel × 4; newskills 4.
    assert_eq!(stat(&s, 4), Some(i32::from(cs.statperlevel as i8) * 4));
    assert_eq!(stat(&s, 5), Some(4));
    let Stats::Bits(e) = &s.body.as_ref().unwrap().stats else {
        unreachable!()
    };
    assert!(e
        .windows(2)
        .all(|w| (w[0].id, w[0].layer) < (w[1].id, w[1].layer)));
    assert!(e
        .iter()
        .all(|x| x.value != 0 && t.isc_save[usize::from(x.id)].bits != 0));
    assert_eq!(stat(&s, 30), None);
}

/// Gold above level × 10,000 is refused (the loader would zero it).
#[test]
fn gold_over_the_carry_limit_is_refused() {
    let mut e = edits();
    e.gold = Some(50_001);
    assert!(new_save(&e, tables()).is_err());
    e.gold = Some(50_000);
    assert!(new_save(&e, tables()).is_ok());
}

/// Skill bytes: one per class-list entry, `--all-skills` then `--skill`.
#[test]
fn skill_bytes() {
    let t = tables();
    let n = t.max_skill_count();
    let mut e = edits();
    e.all_skills = Some(3);
    e.skills = vec![(0, 7)];
    let s = read(&write(&new_save(&e, t).unwrap()));
    assert_eq!(usize::from(s.header.skill_count), n);
    let mut want = vec![3u8; n];
    want[0] = 7;
    assert_eq!(s.body.as_ref().unwrap().skills, want);
    e.skills = vec![(n, 1)];
    assert!(new_save(&e, t).is_err(), "index past the class list");
}

/// Status bits: expansion 0x20, hardcore 0x04, progression bits 8–12 at
/// their thresholds; `jf` / `kf` only for expansion.
#[test]
fn status_bits() {
    let t = tables();
    let c = class();
    let mut e = edits();
    e.unlocked = Some(2);
    let s = read(&write(&new_save(&e, t).unwrap()));
    assert_eq!(
        s.header.status,
        status::EXPANSION | status::HARDCORE | (10 << 8)
    );
    let b = s.body.as_ref().unwrap();
    assert_eq!(b.hireling_items, Some(None));
    assert_eq!(b.golem.as_ref().map(|g| g.flag), Some(0));
    let bytes = write(&s);
    assert_eq!(&bytes[bytes.len() - 5..], &[0x6A, 0x66, 0x6B, 0x66, 0x00]);
    // The loader's checks pass for a Hell game of the same kind.
    let game = d2s::GameContext {
        client_name: b"tester".to_vec(),
        expansion: true,
        hardcore: true,
        difficulty: 2,
    };
    d2s::check_header(&s.header, &game).unwrap();

    if !matches!(c, 5 | 6) {
        let e = Edits {
            expansion: false,
            hardcore: None,
            unlocked: Some(1),
            ..edits()
        };
        let s = read(&write(&new_save(&e, t).unwrap()));
        assert_eq!(s.header.status, 4 << 8);
        let b = s.body.as_ref().unwrap();
        assert_eq!((b.hireling_items.clone(), b.golem.clone()), (None, None));
        let bytes = write(&s);
        assert_eq!(
            &bytes[bytes.len() - 4..],
            &[0x4A, 0x4D, 0x00, 0x00],
            "ends with the corpse list"
        );
    }
}

/// Items written by `write_save` parse back through `save_entry_len`
/// as inventory items (mode 0, body location 0, page 0 → page + 1 = 1)
/// at the cells asked for or found.
#[test]
fn items_parse_back() {
    let t = tables();
    let mut e = edits();
    e.items = vec![
        "sb1@0,0".parse().unwrap(),
        "ar1@2,1".parse().unwrap(),
        "pt1".parse().unwrap(),
    ];
    let s = read(&write(&new_save(&e, t).unwrap()));
    let items = &s.body.as_ref().unwrap().items;
    assert_eq!(items.len(), 3);
    let mut seen = Vec::new();
    for it in items {
        let d = save_entry_len(&it.bytes, &TablesLookup(&t.items)).unwrap();
        assert_eq!(d.len, it.bytes.len());
        assert_eq!(d.item.mode, 0);
        assert!(d.item.save);
        assert_eq!(
            d.item.version, 101,
            "expansion item format (generation.md §1.2)"
        );
        assert_eq!(d.item.ilvl, 5);
        let Some(Location::Slot { body, x, y, page1 }) = d.item.location else {
            panic!("no slot location");
        };
        assert_eq!((body, page1), (0, 1));
        seen.push((d.item.code, x, y));
    }
    assert_eq!(seen[0], (*b"sb1 ", 0, 0));
    assert_eq!(seen[1], (*b"ar1 ", 2, 1));
    assert_eq!(seen[2].0, *b"pt1 ");
    // A taken cell is refused.
    e.items = vec!["sb1@0,0".parse().unwrap(), "pt1@0,1".parse().unwrap()];
    assert!(new_save(&e, t).is_err());
    assert!("sb1:2".parse::<ItemSpec>().is_err(), "trade page");
}

/// Quest act transitions set their bits; `all` is not settled (refused);
/// waypoint indices set their bits, index 0 always known.
#[test]
fn quests_and_waypoints() {
    let t = tables();
    let mut e = edits();
    e.quests = Some("acts=2,1:3.0".parse().unwrap());
    e.waypoints = Some("3,2:5".parse().unwrap());
    let s = read(&write(&new_save(&e, t).unwrap()));
    let b = s.body.as_ref().unwrap();
    let bit = |d: usize, q: usize, k: u8| {
        let n = 16 * q + usize::from(k);
        b.quests.records[d][n >> 3] & (1 << (n & 7)) != 0
    };
    for d in 0..3 {
        assert!(bit(d, 7, 0) && bit(d, 10, 0) && bit(d, 15, 0));
        assert!(!bit(d, 23, 0));
        assert_eq!(bit(d, 3, 0), d == 1);
    }
    let wp = |d: usize, n: u8| {
        let (by, m) = Waypoints::bit(n).unwrap();
        b.waypoints.records[d][by] & m != 0
    };
    for d in 0..3 {
        assert!(wp(d, 0) && wp(d, 3));
        assert_eq!(wp(d, 5), d == 2);
    }
    e.quests = Some(QuestSpec::All);
    assert!(new_save(&e, t).is_err());
    e.quests = None;
    e.waypoints = Some(WpSpec::All);
    let s = new_save(&e, t).unwrap();
    for &n in &t.waypoint_indices {
        let (by, m) = Waypoints::bit(n).unwrap();
        assert!(s.body.as_ref().unwrap().waypoints.records[1][by] & m != 0);
    }
}

/// `set` edits a written save: level, stats, an added item next to the
/// existing ones; the result rewrites byte for byte.
#[test]
fn set_edits_a_save() {
    let t = tables();
    let mut e = edits();
    e.items = vec!["ar1@0,0".parse().unwrap()];
    let mut s = read(&write(&new_save(&e, t).unwrap()));
    let edit = Edits {
        level: Some(9),
        stats: vec![(0, 99)],
        gold: Some(9),
        items: vec!["ar1".parse().unwrap()],
        ..Edits::default()
    };
    apply(&mut s, &edit, t).unwrap();
    let bytes = write(&s);
    let s = read(&bytes);
    assert_eq!(write(&s), bytes);
    assert_eq!(
        (stat(&s, 12), stat(&s, 0), stat(&s, 14)),
        (Some(9), Some(99), Some(9))
    );
    assert_eq!(s.header.level, 9);
    let items = &s.body.as_ref().unwrap().items;
    assert_eq!(items.len(), 2);
    let loc = |i: usize| {
        save_entry_len(&items[i].bytes, &TablesLookup(&t.items))
            .unwrap()
            .item
            .location
    };
    assert_ne!(loc(0), loc(1), "the new item avoids the taken cells");
}

/// The command line: new, check, dump, set and new-stub on files in the
/// test's temp dir, tables from `--game-dir`.
#[test]
fn command_line() {
    let t = tables();
    let _ = t;
    let d = dir();
    let game = d.to_str().unwrap().to_owned();
    let f = d.join("cli.d2s");
    let g = d.join("cli2.d2s");
    let stub = d.join("stub.d2s");
    let run = |a: &[&str]| -> (i32, String) {
        let args: Vec<String> = a.iter().map(|s| s.to_string()).collect();
        let mut out = Vec::new();
        let code = d2s_tool::cli::run(&args, &mut out).unwrap_or_else(|e| panic!("{a:?}: {e:#}"));
        (code, String::from_utf8(out).unwrap())
    };
    let c = class().to_string();
    let (code, _) = run(&[
        "new",
        "--game-dir",
        &game,
        "--name",
        "Cli",
        "--class",
        &c,
        "--level",
        "3",
        "--expansion",
        "--stat",
        "0=40",
        "--item",
        "sb1",
        "--time",
        "7",
        // The unpatched synthetic itemstatcost saves 10 bits for these.
        "--stat",
        "6=1000",
        "--stat",
        "7=1000",
        "--stat",
        "8=1000",
        "--stat",
        "9=1000",
        "--stat",
        "10=1000",
        "--stat",
        "11=1000",
        "-o",
        f.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let (code, out) = run(&["check", "--game-dir", &game, f.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("OK"));
    let (_, out) = run(&["dump", "--game-dir", &game, f.to_str().unwrap()]);
    assert!(out.contains("name \"Cli\""), "{out}");
    assert!(out.contains("stat   0 layer 0 = 40"), "{out}");
    assert!(out.contains("sb1"), "{out}");
    let (code, _) = run(&[
        "set",
        "--game-dir",
        &game,
        f.to_str().unwrap(),
        "--gold",
        "100",
        "-o",
        g.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let (code, _) = run(&["check", "--game-dir", &game, g.to_str().unwrap()]);
    assert_eq!(code, 0);
    let (code, _) = run(&[
        "new-stub",
        "--name",
        "Test",
        "--class",
        "ama",
        "--time",
        "0",
        "-o",
        stub.to_str().unwrap(),
    ]);
    assert_eq!(code, 0);
    let b = std::fs::read(&stub).unwrap();
    assert_eq!(b.len(), 335);
    let mut z = b.clone();
    z[0x0C..0x10].fill(0);
    assert_eq!(
        d2s::checksum(&z),
        u32::from_le_bytes(b[0x0C..0x10].try_into().unwrap())
    );
    // A stub needs no tables.
    let (code, out) = run(&["check", stub.to_str().unwrap()]);
    assert_eq!(code, 0, "{out}");
    // A corrupted file: the check reports it.
    let mut bad = std::fs::read(&f).unwrap();
    let n = bad.len();
    bad[n - 1] ^= 1;
    std::fs::write(&g, &bad).unwrap();
    let args: Vec<String> = ["check", "--game-dir", &game, g.to_str().unwrap()]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert!(
        d2s_tool::cli::run(&args, &mut Vec::new()).is_err(),
        "checksum mismatch"
    );
}

/// A generated save is what the game writes after loading it (C66): no
/// item carries flag 0x2000, the appearance bytes are 32 × 0xFF (no
/// equipped item).
#[test]
fn new_save_is_the_game_resave_form() {
    let t = tables();
    let mut e = edits();
    e.items = vec!["sb1".parse().unwrap(), "pt1".parse().unwrap()];
    let s = read(&write(&new_save(&e, t).unwrap()));
    assert_eq!(s.header.to_bytes()[0x88..0xA8], [0xFF; 32]);
    for it in &s.body.as_ref().unwrap().items {
        let f = it.record_flags(0).expect("JM");
        assert_eq!(f & d2s::ITEM_FLAG_INSTORE, 0, "flags {f:#x}");
        assert_eq!(f & 0x80_0000, 0x80_0000, "the writer's forced bit");
    }
}

/// `set` writes the game's re-save form: 0x2000 cleared on every item
/// record (player list, corpse, hireling list), the appearance bytes
/// reset when nothing is equipped.
#[test]
fn resave_clears_instore_and_resets_appearance() {
    use d2s_tool::save::resave;
    let t = tables();
    let mut e = edits();
    e.items = vec!["sb1".parse().unwrap()];
    let mut s = new_save(&e, t).unwrap();
    {
        // A file as an older writer left it: 0x2000 on the items, the
        // stub's component bytes.
        let b = s.body.as_mut().unwrap();
        let mut it = b.items[0].clone();
        let f = it.record_flags(0).unwrap();
        it.set_record_flags(0, f | d2s::ITEM_FLAG_INSTORE).unwrap();
        b.items[0] = it.clone();
        b.corpses.push(d2s::Corpse {
            unk: 1,
            x: 0,
            y: 0,
            items: vec![it.clone()],
        });
        b.hireling_items = Some(Some(vec![it]));
        // The list is read only for a present hireling.
        s.header.hireling.seed = 1;
        s.header.components = d2s::STUB_COMPONENTS;
    }
    let notes = resave(&mut s, t).unwrap();
    assert!(notes.is_empty());
    let b = s.body.as_ref().unwrap();
    let hire = &b.hireling_items.as_ref().unwrap().as_ref().unwrap()[0];
    for it in [&b.items[0], &b.corpses[0].items[0], hire] {
        assert_eq!(it.record_flags(0).unwrap() & d2s::ITEM_FLAG_INSTORE, 0);
    }
    assert_eq!(s.header.to_bytes()[0x88..0xA8], [0xFF; 32]);
    // It still reads back and rewrites to itself.
    let f = write(&s);
    assert!(matches!(
        round_trip(&f, &read_options(&f, None), t).unwrap(),
        RoundTrip::Same(_)
    ));
}

/// `lv=ID` sets the waypoint of a level's row; a level without one, or
/// past the rows, is refused.
#[test]
fn waypoints_by_level() {
    let t = tables();
    let (lv, n) = t
        .level_waypoint
        .iter()
        .enumerate()
        .find(|&(i, &n)| i > 0 && n != 255 && n != 0)
        .map(|(i, &n)| (i, n))
        .expect("a level with a waypoint");
    let mut e = edits();
    e.waypoints = Some(format!("1:lv={lv}").parse().unwrap());
    let s = read(&write(&new_save(&e, t).unwrap()));
    let b = s.body.as_ref().unwrap();
    let (by, m) = Waypoints::bit(n).unwrap();
    for d in 0..3 {
        assert_eq!(
            b.waypoints.records[d][by] & m != 0,
            d == 1,
            "level {lv} index {n}"
        );
    }
    if let Some(none) = t.level_waypoint.iter().position(|&n| n == 255) {
        e.waypoints = Some(format!("lv={none}").parse().unwrap());
        assert!(new_save(&e, t).is_err());
    }
    e.waypoints = Some(format!("lv={}", t.level_waypoint.len()).parse().unwrap());
    assert!(new_save(&e, t).is_err());
    assert!("lv=x".parse::<WpSpec>().is_err());
}
