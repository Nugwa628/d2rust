//! Checks D2RUST waypoint, quest, cube, NPC and vendor table views against
//! expected rows, mappings, recipe fields and store count ranges. The
//! player's MPQ set comes from `D2_GAME_DIR` and is loaded and fixed up
//! through `d2-data`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use d2_data::bin;
use d2_data::fixup::{self, FixedSet};
use d2_data::schema::schema;
use d2_data::tables::{decode_all, Leveldefs, Levels, Monstats, Objects, Record, Superuniques};
use d2_formats::mpq::ArchiveSet;
use d2_sim::rng::Seed;
use d2_sim::world::cube::{self, input_flags, kind, op_info, output_flags, OpScope, Recipe};
use d2_sim::world::npc::{parse_npc_table, HireRow, NpcControl, VENDORS_TSV};
use d2_sim::world::quests::QuestTables;
use d2_sim::world::vendors::{self, class, Column, ColumnEntry, GlobalLists, VendorTables};
use d2_sim::world::waypoints::{self, WaypointData, WaypointMap, NO_WAYPOINT};

const WAYPOINTS_TSV: &str = include_str!("../../../tables/world/waypoints.tsv");
const CUBE_OPS_TSV: &str = include_str!("../../../tables/world/cube-ops.tsv");

// ---- live tables ---------------------------------------------------------------------

fn fixed() -> &'static FixedSet {
    static F: OnceLock<FixedSet> = OnceLock::new();
    F.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let data = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live set loads");
        let anim = fixup::read_animdata(&set).expect("AnimData.d2");
        fixup::apply(&data, &anim).expect("fix-ups apply")
    })
}

fn rows<T: Record>() -> Vec<T> {
    let t = fixed().table(T::TABLE).expect("table loaded");
    decode_all(t).expect("table decodes")
}

fn item_index(code: &[u8; 4]) -> u16 {
    fixed()
        .item_codes
        .find(u32::from_le_bytes(*code))
        .unwrap_or_else(|| panic!("item {}", String::from_utf8_lossy(code))) as u16
}

/// Tab-separated rows after the header.
fn tsv(text: &str) -> Vec<Vec<&str>> {
    text.lines()
        .skip(1)
        .filter(|l| !l.is_empty())
        .map(|l| l.split('\t').collect())
        .collect()
}

// ---- waypoints -----------------------------------------------------------------------

/// `waypoints.md` Constants: `waypoints.tsv` is the expected result of
/// deriving §1 and §7 rule 4 from the live tables.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn waypoint_map_matches_tsv() {
    let map = WaypointMap::new(&rows::<Levels>());
    let expected: Vec<(u8, u32, u8, bool, u8)> = tsv(WAYPOINTS_TSV)
        .iter()
        .map(|c| {
            (
                c[0].parse().unwrap(),
                c[1].parse().unwrap(),
                c[2].parse().unwrap(),
                c[3] == "1",
                c[4].parse().unwrap(),
            )
        })
        .collect();
    assert_eq!(map.rows(), expected);
    // §1 rule 2: 39 indexes 0..38, each on exactly one level.
    assert_eq!(expected.len(), 39);
    for wp in 0..39u8 {
        let on: Vec<u32> = (0..map.level_count())
            .filter(|&l| map.index_of_level(l) == Some(wp))
            .collect();
        assert_eq!(on.len(), 1, "levels with index {wp}: {on:?}");
        assert_eq!(map.level_of_index(wp.into()), Some(on[0]));
    }
    assert!((0..map.level_count())
        .filter_map(|l| map.index_of_level(l))
        .all(|wp| wp < 39));
    for (acts, first, last) in [(0, 0, 8), (1, 9, 17), (2, 18, 26), (3, 27, 29), (4, 30, 38)] {
        for wp in first..=last {
            let l = map.level_of_index(wp).unwrap();
            assert_eq!(map.act(l), Some(acts), "act of index {wp}");
        }
    }
    assert_eq!(map.level_of_index(10), Some(48));
    assert_eq!(map.level_of_index(11), Some(42));
    // §1 rule 1: a missing record or an index ≥ 255 is none.
    assert_eq!(map.index_of_level(map.level_count()), None);
    assert_eq!(map.level_of_index(u32::from(NO_WAYPOINT)), None);
    // §1 rule 4: towns and their indexes.
    for (level, wp) in [(1, 0), (40, 9), (75, 18), (103, 27), (109, 30)] {
        assert!(waypoints::is_town(level));
        assert_eq!(map.index_of_level(level), Some(wp), "town {level}");
    }
    // §7 rule 4: waypoint levels with tile code 13; 133–136 have none.
    let code13: Vec<u32> = map
        .rows()
        .iter()
        .filter(|r| r.4 == 13)
        .map(|r| r.1)
        .collect();
    let mut want = vec![1, 40, 74, 46, 75, 103, 109];
    want.sort_unstable();
    let mut got = code13;
    got.sort_unstable();
    assert_eq!(got, want);
    for l in 133..=136 {
        assert_eq!(map.index_of_level(l), None, "level {l}");
    }
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn waypoint_objects() {
    let objects = rows::<Objects>();
    let data = WaypointData::new(&rows::<Levels>(), &objects);
    let classes = data.waypoint_classes();
    assert_eq!(
        classes,
        [119, 145, 156, 157, 237, 238, 288, 323, 324, 398, 402, 429, 494, 496, 511, 539]
    );
    for &c in &classes {
        let o = &objects[usize::from(c)];
        assert_eq!((o.mode0, o.mode1, o.mode2), (1, 1, 1), "modes of {c}");
        let long = [494, 496, 511, 539].contains(&c);
        // The fixed-up set holds `FrameCnt` × 256 (`data/fixups.md` §13
        // rule 2; `waypoints.md` Inputs: "already × 256"); §5 r1 states the
        // `objects.txt` value 15 / 20, read back as `FrameCnt1 >> 8` (§5.1 r2).
        let frames: u32 = if long { 20 } else { 15 };
        assert_eq!(o.framecnt1, frames << 8, "FrameCnt1 of {c}");
        assert_eq!(o.framedelta1, 200, "FrameDelta1 of {c}");
        let unsynced = [429, 494, 496, 511, 539].contains(&c);
        assert_eq!(o.sync, u8::from(!unsynced), "Sync of {c}");
    }
}

/// `waypoints.md` §7 rule 6 (measured): every waypoint level with
/// `Position` ≠ 0 gets tile code 13.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn waypoint_position_levels_use_code_13() {
    let map = WaypointMap::new(&rows::<Levels>());
    let defs = rows::<Leveldefs>();
    for (wp, level, _, _, code) in map.rows() {
        if defs[level as usize].position != 0 {
            assert_eq!(code, 13, "index {wp}, level {level}");
        }
    }
}

// ---- quests --------------------------------------------------------------------------

/// `quests.md` §4.6 (`levels.txt` `Quest`), §4.4 rule 4 (superunique
/// hcIdx values), §7.1 (every NPC id of `quest-messages.tsv` is a monstats
/// row) and the monstats / superuniques counts of `preset.md` §5.3.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn quest_tables_against_live_data() {
    let levels = rows::<Levels>();
    let quest: Vec<(usize, u8)> = levels
        .iter()
        .enumerate()
        .filter(|(_, l)| l.quest != 0)
        .map(|(i, l)| (i, l.quest))
        .collect();
    assert_eq!(quest, [(8, 1), (74, 11)]);
    let monstats = rows::<Monstats>();
    let superuniques = rows::<Superuniques>();
    assert_eq!((monstats.len(), superuniques.len()), (734, 66));
    let hc: Vec<u32> = superuniques.iter().map(|s| s.hcidx).collect();
    for idx in [26, 27, 29, 36, 37, 38, 39, 42, 43, 44, 45, 60] {
        assert!(hc.contains(&idx), "hcIdx {idx}");
    }
    let t = QuestTables::load().expect("quest TSVs parse");
    assert_eq!(t.rows.len(), 41);
    for m in &t.messages {
        assert!(usize::from(m.npc) < monstats.len(), "npc {}", m.npc);
    }
}

// ---- cube ----------------------------------------------------------------------------

fn recipes() -> Vec<Recipe> {
    cube::recipes(fixed().table("cubemain").expect("cubemain loaded")).expect("cubemain decodes")
}

/// Σ max(quantity, 1) over the input slots holding `item` (any-item path).
fn input_count(r: &Recipe, item: u16) -> u32 {
    r.inputs
        .iter()
        .filter(|s| s.flags & input_flags::USEANY != 0 && s.item == item)
        .map(|s| u32::from(s.quantity.max(1)))
        .sum()
}

/// `cube.md` Test vectors, "Live-data facts": every record parsed, its
/// fields and output kinds as measured, and its op listed in
/// `cube-ops.tsv` with the scope the code gives it.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn cubemain_live_facts() {
    let all = recipes();
    assert_eq!(all.len(), 151);
    let idx = |f: &dyn Fn(&Recipe) -> bool| -> Vec<usize> {
        (0..all.len()).filter(|&i| f(&all[i])).collect()
    };
    assert_eq!(idx(&|r| r.enabled == 0), (142..=146).collect::<Vec<_>>());
    let ladder: Vec<usize> = (104..=122).chain([131, 132]).collect();
    assert_eq!(idx(&|r| r.ladder != 0), ladder);
    assert_eq!(idx(&|r| r.version == 100).len(), 96);
    assert_eq!(idx(&|r| r.op == 28), [0, 1, 2, 148, 149]);
    assert_eq!(idx(&|r| r.op != 28 && r.op != 0), Vec::<usize>::new());
    for (i, r) in all.iter().enumerate() {
        assert_eq!(
            (r.min_diff, r.class, r.param, r.value),
            (0, 0xFF, 0, 0),
            "record {i}"
        );
        let n: u32 = r
            .inputs
            .iter()
            .filter(|s| s.flags != 0)
            .map(|s| u32::from(s.quantity.max(1)))
            .sum();
        assert_eq!(u32::from(r.numinputs), n, "numinputs of record {i}");
        for b in &r.outputs[1..] {
            assert_eq!((b.kind, b.flags), (0, 0), "outputs b/c of record {i}");
        }
    }
    // Output kinds of slot a.
    let mut kinds: BTreeMap<u8, usize> = BTreeMap::new();
    for r in &all {
        *kinds.entry(r.outputs[0].kind).or_default() += 1;
    }
    let want: BTreeMap<u8, usize> = [
        (kind::ITEMCODE, 78),
        (kind::USETYPE, 48),
        (kind::USEITEM, 21),
        (kind::ITEMTYPE, 1),
        (kind::COW_PORTAL, 1),
        (kind::PANDEMONIUM, 1),
        (kind::PANDEMONIUM_FINALE, 1),
    ]
    .into();
    assert_eq!(kinds, want);
    // Output flags (records carrying each flag in slot a).
    for (flag, n) in [
        (output_flags::MOD, 8),
        (output_flags::EXC, 4),
        (output_flags::ELI, 4),
        (output_flags::REP, 4),
        (output_flags::RCH, 2),
        (output_flags::SOCK, 1),
        (output_flags::UNS, 1),
    ] {
        assert_eq!(
            idx(&|r| r.outputs[0].flags & flag != 0).len(),
            n,
            "flag {flag:#x}"
        );
    }
    // 133 mods, all chance 0.
    let mods: Vec<_> = all
        .iter()
        .flat_map(|r| r.outputs.iter().flat_map(|o| o.mods.iter()))
        .filter(|m| m.property >= 0)
        .collect();
    assert_eq!(mods.len(), 133);
    assert!(mods.iter().all(|m| m.chance == 0));
    // Level modes of slot a.
    let mut modes: BTreeMap<(bool, bool, bool), usize> = BTreeMap::new();
    for r in &all {
        let o = &r.outputs[0];
        *modes
            .entry((o.lvl != 0, o.plvl != 0, o.ilvl != 0))
            .or_default() += 1;
    }
    let want: BTreeMap<(bool, bool, bool), usize> = [
        ((false, false, false), 99),
        ((false, true, true), 38),
        ((true, false, true), 6),
        ((true, false, false), 5),
        ((false, true, false), 2),
        ((false, false, true), 1),
    ]
    .into();
    assert_eq!(modes, want);
    // Every op is a cube-ops.tsv row with the code's scope.
    let ops: BTreeMap<u8, &str> = tsv(CUBE_OPS_TSV)
        .iter()
        .map(|c| (c[0].parse().unwrap(), c[1]))
        .collect();
    for (i, r) in all.iter().enumerate() {
        let row = ops
            .get(&r.op.min(29))
            .unwrap_or_else(|| panic!("op {} of record {i}", r.op));
        let scope = match op_info(r.op).0 {
            OpScope::Recipe => "recipe",
            OpScope::Input0 => "input0",
            OpScope::Inputs => "inputs",
        };
        assert_eq!(scope, *row, "scope of op {} (record {i})", r.op);
    }
}

/// The live records of `cube.md` Test vectors V1–V23, as far as the record
/// itself states them.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn cubemain_vector_records() {
    let all = recipes();
    let r = |i: usize| &all[i];
    // V1: 3 × gcv → gfv, no level fields.
    assert_eq!(r(23).numinputs, 3);
    assert_eq!(input_count(r(23), item_index(b"gcv ")), 3);
    let o = &r(23).outputs[0];
    assert_eq!((o.kind, o.item), (kind::ITEMCODE, item_index(b"gfv ")));
    assert_eq!((o.lvl, o.plvl, o.ilvl), (0, 0, 0));
    // V3: 2 × aqv → cqv.
    assert_eq!(r(21).numinputs, 2);
    assert_eq!(input_count(r(21), item_index(b"aqv ")), 2);
    assert_eq!(r(21).outputs[0].item, item_index(b"cqv "));
    // V5, V8: slot 0 three magic rings; plvl 75.
    let s = &r(13).inputs[0];
    assert_eq!((s.item, s.quality, s.quantity), (item_index(b"rin "), 4, 3));
    assert_eq!(r(13).outputs[0].plvl, 75);
    // V6: `fhl,mag,upg`, version 100. `fhl` is an item code, not an item
    // type: `data/callbacks.md` §3 input parse step 4.3 (item code →
    // 0x0001 USEANY, item = item index), plus `upg` 0x0080.
    let s = &r(64).inputs[0];
    assert_eq!(s.flags, input_flags::USEANY | input_flags::UPG);
    assert_eq!((s.item, s.quality), (item_index(b"fhl "), 4));
    assert_eq!(r(64).version, 100);
    // V7, V9, V10, V11: level fields.
    let lv = |i: usize| {
        let o = &r(i).outputs[0];
        (o.lvl, o.plvl, o.ilvl)
    };
    assert_eq!(lv(15), (30, 0, 60));
    assert_eq!(lv(61), (0, 40, 40));
    assert_eq!(lv(62), (0, 66, 66));
    assert_eq!(lv(60), (0, 0, 100));
    // V17, V18: ladder and disabled.
    assert_ne!(r(104).ladder, 0);
    assert_eq!(r(142).enabled, 0);
    // V19: op 28, output hst; V20: op 28; V21/V22: the Cow portal.
    assert_eq!((r(0).op, r(0).outputs[0].item), (28, item_index(b"hst ")));
    assert_eq!(r(148).op, 28);
    assert_eq!(r(2).outputs[0].kind, kind::COW_PORTAL);
    assert_eq!(input_count(r(2), item_index(b"leg ")), 1);
    assert_eq!(input_count(r(2), item_index(b"tbk ")), 1);
    // V23: record 137 takes `r09`.
    assert_eq!(input_count(r(137), item_index(b"r09 ")), 1);
}

// ---- npc -----------------------------------------------------------------------------

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn npc_records_from_live_monstats() {
    let ms = rows::<Monstats>();
    let hire = HireRow::from_table(fixed().table("hireling").unwrap()).unwrap();
    let mut seed = Seed::init_low(1);
    let c = NpcControl::new(&ms, hire, true, 0, &mut seed).unwrap();
    let interact: Vec<u16> = (0..ms.len() as u16)
        .filter(|&i| ms[usize::from(i)].interact)
        .collect();
    assert_eq!(interact.len(), 47);
    let classes: Vec<u16> = c.records.iter().map(|r| r.class).collect();
    assert_eq!(classes, interact);
    let table = parse_npc_table(VENDORS_TSV).unwrap();
    assert_eq!(table.len(), 43);
    for t in &table {
        let r = c
            .record(t.class)
            .unwrap_or_else(|| panic!("{} has a record", t.name));
        assert_eq!(
            (r.act, r.trader, r.byte6),
            (t.act, t.trader, t.byte6),
            "{}",
            t.name
        );
    }
    for unlisted in [527, 537, 538, 539] {
        assert!(table.iter().all(|t| t.class != unlisted));
        let r = c
            .record(unlisted)
            .unwrap_or_else(|| panic!("{unlisted} has a record"));
        assert_eq!((r.act, r.trader), (0, 0), "class {unlisted}");
    }
}

// ---- vendors -------------------------------------------------------------------------

fn vendor_tables() -> VendorTables {
    VendorTables::from_fixed(fixed()).expect("vendor tables")
}

/// `vendors.md` §1 rule 1 order of the 17 columns.
const VENDORS: [&str; 17] = [
    "Akara", "Gheed", "Charsi", "Fara", "Lysander", "Drognan", "Hratli", "Alkor", "Ormus", "Elzix",
    "Asheara", "Cain", "Halbu", "Jamella", "Malah", "Larzuk", "Drehya",
];

/// Column `i` read straight from the item records through the
/// `<Vendor>Min…MagicLvl` columns of `fields.tsv` (§1 rules 1–2).
fn raw_column(i: usize) -> Column {
    let mut c = Column::default();
    for table in ["weapons", "armor", "misc"] {
        let def = schema().table(table).unwrap();
        let at = |col: &str| {
            def.field(col)
                .unwrap_or_else(|| panic!("{table}.{col}"))
                .offset as usize
        };
        // The item tables spell Hratli's columns `Hralti…` (`fields.tsv`
        // weapons / armor / misc seq 127–131 at 332 + …); `vendors.md` §1 r1
        // names the NPC.
        let v = if VENDORS[i] == "Hratli" {
            "Hralti"
        } else {
            VENDORS[i]
        };
        let (min, max) = (at(&format!("{v}Min")), at(&format!("{v}Max")));
        let (mmin, mmax) = (at(&format!("{v}MagicMin")), at(&format!("{v}MagicMax")));
        let mlvl = at(&format!("{v}MagicLvl"));
        assert_eq!(min, 326 + i, "{table}.{v}Min");
        let (code, spawnable, perm) = (at("code"), at("spawnable"), at("PermStoreItem"));
        for r in fixed().table(table).unwrap().iter() {
            if r[spawnable] == 0 || (r[max] == 0 && r[mmax] == 0) {
                continue;
            }
            let code: [u8; 4] = r[code..code + 4].try_into().unwrap();
            if r[perm] != 0 {
                c.perm.push(code);
            } else {
                c.items.push(ColumnEntry {
                    min: r[min],
                    max: r[max],
                    magic_min: r[mmin],
                    magic_max: r[mmax],
                    code,
                    magic_lvl: r[mlvl],
                });
            }
        }
    }
    c
}

/// `vendors.md` Test vectors, game-file test: the 17 column lists from
/// the live item tables; Charsi's examples; column 11 (Cain) never built.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn vendor_columns_from_live_items() {
    let t = vendor_tables();
    assert_eq!(VENDORS.len(), vendors::COLUMNS);
    for (i, name) in VENDORS.iter().enumerate() {
        assert_eq!(Column::build(&t, i), raw_column(i), "column {i} ({name})");
    }
    let charsi = Column::build(&t, 2);
    assert!(charsi.perm.contains(b"aqv ") && charsi.perm.contains(b"cqv "));
    let axe = charsi
        .items
        .iter()
        .find(|e| &e.code == b"axe ")
        .expect("axe");
    assert_eq!(
        (
            axe.min,
            axe.max,
            axe.magic_min,
            axe.magic_max,
            axe.magic_lvl
        ),
        (1, 1, 1, 1, 1)
    );
    let globals = GlobalLists::build(&t);
    assert_eq!(globals.columns[11], Column::default());
}

/// One expected store entry: item code and either separate normal/magic
/// counts or a combined total.
enum Run {
    Total([u8; 4], u32),
    Split([u8; 4], u32, u32),
}

/// Checks an expected store vector against column `i` at item level `ilvl`
/// (`vendors.md` §3): per eligible entry in list order, n_norm in
/// [Min, Max], n_mag in [MagicMin, MagicMax] when the item can be magic;
/// an entry absent from the vector must allow zero items. The permanent
/// code list must match exactly.
fn check_store(t: &VendorTables, i: usize, ilvl: u8, runs: &[Run], perm: &[&[u8; 4]]) {
    let col = Column::build(t, i);
    let mut runs = runs.iter().peekable();
    for e in &col.items {
        let rec = t.item(t.find_code(e.code).unwrap()).unwrap();
        if rec.level > ilvl {
            continue;
        }
        let magic = rec.bitfield1 & 1 != 0 && e.magic_lvl <= ilvl;
        let norm = (u32::from(e.min), u32::from(e.max.max(e.min)));
        let mag = if magic {
            (
                u32::from(e.magic_min),
                u32::from(e.magic_max.max(e.magic_min)),
            )
        } else {
            (0, 0)
        };
        let name = String::from_utf8_lossy(&e.code).into_owned();
        let code = match runs.peek() {
            Some(Run::Total(c, _)) | Some(Run::Split(c, _, _)) => *c,
            None => [0; 4],
        };
        if code != e.code {
            assert_eq!(
                norm.0 + mag.0,
                0,
                "{name} yields at least one item but is absent"
            );
            continue;
        }
        match runs.next().unwrap() {
            Run::Total(_, n) => {
                assert!((norm.0 + mag.0..=norm.1 + mag.1).contains(n), "{name}: {n}");
            }
            Run::Split(_, n, m) => {
                assert!((norm.0..=norm.1).contains(n), "{name} normal: {n}");
                assert!((mag.0..=mag.1).contains(m), "{name} magic: {m}");
            }
        }
    }
    assert!(runs.next().is_none(), "recorded entries not in column {i}");
    let want: Vec<[u8; 4]> = perm.iter().map(|c| **c).collect();
    assert_eq!(col.perm, want, "permanent list of column {i}");
}

/// Groups adjacent equal item codes into total-count store entries.
fn runs_of(codes: &str) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for c in codes.split(' ') {
        let mut code = [b' '; 4];
        code[..c.len()].copy_from_slice(c.as_bytes());
        match out.last_mut() {
            Some(Run::Total(last, n)) if *last == code => *n += 1,
            _ => out.push(Run::Total(code, 1)),
        }
    }
    out
}

/// The Charsi and Akara store vectors fit the loaded vendor columns
/// at item level 6, computed for character level 1 in Normal act 0.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn recorded_stores_fit_live_columns() {
    let t = vendor_tables();
    let ilvl = vendors::store_level(1, 0, 0);
    assert_eq!(ilvl, 6);
    let ilvl = ilvl as u8;
    let charsi = "hax hax lax lax spc ssd scm scm dgr dgr tkf tkf jav jav spr spr bar bar \
                  sbw sbw hbw hbw ktr ktr cap cap skp qui qui lea hla buc buc sml sml lgl lgl \
                  lbt lbt lbl lbl";
    let col = vendors::column_of(class::CHARSI).unwrap();
    check_store(&t, col, ilvl, &runs_of(charsi), &[b"aqv ", b"cqv "]);
    let ktr = t.item(t.find_code(*b"ktr ").unwrap()).unwrap();
    assert!(ktr.version >= 100, "ktr is an expansion item");
    let akara = [
        Run::Split(*b"wnd ", 5, 8),
        Run::Split(*b"scp ", 3, 3),
        Run::Split(*b"sst ", 6, 8),
    ];
    let perm = [
        b"vps ", b"yps ", b"wms ", b"tbk ", b"ibk ", b"tsc ", b"isc ", b"key ", b"hp1 ", b"mp1 ",
    ];
    let col = vendors::column_of(class::AKARA).unwrap();
    check_store(&t, col, ilvl, &akara, &perm);
}

/// `vendors.md` §9.3 (live `npc.txt`) and the `difficultylevels` gamble
/// odds of Constants.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn npc_txt_multipliers() {
    use class::*;
    let t = vendor_tables();
    let row = |c: u16| *t.npc_row(c).unwrap_or_else(|| panic!("npc.txt row {c}"));
    let gheed = row(GHEED);
    assert_eq!((gheed.sell, gheed.buy, gheed.rep), (1088, 512, 128));
    assert_eq!(gheed.quests[0], (4, 922, 1024, 1024));
    assert_eq!(gheed.max_buy, [5000, 30000, 35000]);
    let groups: [(&[u16], i32, u32, i32, bool); 6] = [
        (&[CHARSI], 960, 4, 5000, false),
        (&[AKARA], 1024, 4, 5000, false),
        (&[LYSANDER, DROGNAN, ELZIX, FARA], 1024, 9, 10000, false),
        (&[HRATLI, ALKOR, ORMUS, ASHEARA], 1024, 17, 15000, false),
        (&[JAMELLA, HALBU], 1024, 41, 20000, false),
        (&[MALAH, DREHYA, LARZUK, NIHLATHAK], 2048, 41, 25000, true),
    ];
    for (classes, sell, slot_a, max_buy, quest_b) in groups {
        for &c in classes {
            let r = row(c);
            assert_eq!((r.sell, r.buy, r.rep), (sell, 512, 128), "class {c}");
            assert_eq!(r.quests[0].0, slot_a, "quest A slot of {c}");
            assert_eq!(r.max_buy[0], max_buy, "max buy of {c}");
            if quest_b {
                assert_eq!(r.quests[1], (35, 512, 1024, 1024), "quest B of {c}");
            } else {
                assert_eq!(r.quests[1].0, 0, "no quest B for {c}");
            }
        }
    }
    assert_eq!(gheed.quests[1].0, 0);
    assert_eq!(t.difficulty.len(), 3);
    for d in &t.difficulty {
        assert_eq!(
            (d.rare, d.set, d.unique, d.uber, d.ultra),
            (10000, 100, 50, 90, 33)
        );
    }
}
