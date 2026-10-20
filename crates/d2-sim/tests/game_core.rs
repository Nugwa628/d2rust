//! Checks D2RUST stat, unit, vitals and skill-level APIs against the
//! player's `.bin` tables in `D2_GAME_DIR`, loaded through `bin::load`,
//! `fixup::read_animdata` and `fixup::apply`. All tests are `#[ignore]`.
//!
//! Expected vectors and table constraints require a local run against the
//! player's data to confirm them; compatibility with every table set is
//! not established by these tests.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use d2_data::bin::{self, BinSet};
use d2_data::fixup::{self, FixedSet};
use d2_data::tables::{decode_all, Charstats, Itemstatcost, Monstats, Record, Skills};
use d2_formats::animdata::AnimData;
use d2_formats::mpq::ArchiveSet;
use d2_sim::combat::vitals::{
    init_player_stats, level_up, spend, stat as vstat, VitalsTables, VitalsUnits,
};
use d2_sim::rng::Seed;
use d2_sim::skills::special::SKILLCALC_CODES;
use d2_sim::skills::{
    elem_len, elem_max, elem_min, eval_skill, mana_cost, mana_cost_shifted, phys_max, phys_min,
    special, to_hit, SkillEntry, SkillTables, SkillUnits, LEVEL_CAP_114D, NO_CALC,
};
use d2_sim::stats::lists::{CallbackEvent, NoHost, RemoveCallback};
use d2_sim::stats::ops::op_row;
use d2_sim::stats::{key_stat, ListId, StatData, StatHost, StatLists, StatTable, StateTable};
use d2_sim::stats::{OpEntry, ValueCallback};
use d2_sim::tick::events::event;
use d2_sim::units::anim::{schedule, Events, Form};
use d2_sim::units::record::{UnitRecord, Units, ANIM_EVENTS};
use d2_sim::units::{UnitId, UnitType};
use d2_sim::wiring::interaction::{VitalsRest, VitalsView};

// ---- live tables ---------------------------------------------------------------------

struct Live {
    bin: BinSet,
    anim: AnimData,
    fixed: FixedSet,
}

fn live() -> &'static Live {
    static L: OnceLock<Live> = OnceLock::new();
    L.get_or_init(|| {
        let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
        let set = ArchiveSet::open_dir(dir).expect("archives open");
        let bin = bin::load(&set, bin::DEFAULT_LANGUAGE).expect("live .bin set loads");
        let anim = fixup::read_animdata(&set).expect("AnimData.d2 reads");
        let fixed = fixup::apply(&bin, &anim).expect("fix-ups apply");
        Live { bin, anim, fixed }
    })
}

fn table(name: &str) -> &'static d2_data::bin::BinTable {
    live()
        .fixed
        .table(name)
        .unwrap_or_else(|| panic!("table {name} in the live set"))
}

fn rows<T: Record>() -> Vec<T> {
    decode_all(table(T::TABLE)).unwrap_or_else(|e| panic!("{}: {e}", T::TABLE))
}

fn stat_data() -> Arc<StatData> {
    let states = StateTable::new(table("states"), &live().fixed.states).expect("states");
    Arc::new(
        StatData::new(
            table("itemstatcost"),
            table("charstats"),
            states,
            table("monstats"),
            table("skills"),
        )
        .expect("stat data"),
    )
}

fn vitals_tables() -> VitalsTables {
    VitalsTables::from_bin(&live().bin).expect("vitals tables")
}

const SORCERESS: u32 = 1;
const BARBARIAN: u32 = 4;

// ---- stats.md: itemstatcost columns --------------------------------------------------

fn stats_where(isc: &[Itemstatcost], f: impl Fn(&Itemstatcost) -> bool) -> Vec<u16> {
    (0..isc.len() as u16)
        .filter(|&s| f(&isc[usize::from(s)]))
        .collect()
}

/// Checks itemstatcost row counts, column values and flag-bit groups
/// against the expected table constraints.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn itemstatcost_columns_as_stated() {
    let t = table("itemstatcost");
    assert_eq!(t.count, 359, "itemstatcost count");
    let isc: Vec<Itemstatcost> = rows();
    let n = isc.len() as u16;

    let shifted = stats_where(&isc, |c| c.valshift != 0);
    assert_eq!(shifted, [6, 7, 8, 9, 10, 11, 216, 217], "ValShift ≠ 0");
    for s in shifted {
        assert_eq!(isc[usize::from(s)].valshift, 8, "ValShift of {s}");
    }
    assert_eq!(stats_where(&isc, |c| c.keepzero != 0), [8, 10], "keepzero");
    let fmin = stats_where(&isc, |c| c.fmin);
    assert_eq!(fmin, [0, 1, 2, 3, 7, 9, 11], "fMin");
    let minaccr: Vec<u32> = fmin.iter().map(|&s| isc[usize::from(s)].minaccr).collect();
    assert_eq!(minaccr, [1, 1, 1, 1, 1, 0, 0], "MinAccr of the fMin stats");
    assert_eq!(
        stats_where(&isc, |c| c.saved),
        (0..16).collect::<Vec<u16>>(),
        "Saved"
    );
    let cb = stats_where(&isc, |c| c.fcallback);
    assert_eq!(cb.len(), 35, "fCallback count: {cb:?}");
    assert_eq!(cb[..6], [7, 9, 11, 78, 81, 83], "fCallback, first six");
    assert_eq!(cb.last(), Some(&204), "fCallback, last");
    assert_eq!(
        stats_where(&isc, |c| c.damagerelated).len(),
        104,
        "damagerelated count"
    );
    // §3 "not read": MaxStat 6→7, 8→9, 10→11, 72→73; UpdateAnimRate 67–69.
    let maxstat: Vec<(u16, u16)> = (0..n)
        .filter(|&s| isc[usize::from(s)].maxstat < n)
        .map(|s| (s, isc[usize::from(s)].maxstat))
        .collect();
    assert_eq!(maxstat, [(6, 7), (8, 9), (10, 11), (72, 73)], "MaxStat");
    assert_eq!(
        stats_where(&isc, |c| c.updateanimrate),
        [67, 68, 69],
        "UpdateAnimRate"
    );
    // Flag bits of +0x04: 0–4, 9–12 from the file, 5–8 set at load.
    let bits = t
        .iter()
        .map(|r| u32::from_le_bytes([r[4], r[5], r[6], r[7]]))
        .fold(0u32, |a, f| a | f);
    let file_bits = [0, 1, 2, 3, 4, 9, 10, 11, 12]
        .iter()
        .fold(0u32, |a, b| a | 1 << b);
    assert_eq!(bits & !0x1E0, file_bits, "flag bits of +0x04: {bits:#x}");
}

/// `stats.md` §6.3, Constants, Test vectors "Real data": the op of
/// every stat, the op targets, entries(7), deps(12), A53; every op in the
/// data has its `stat-ops.tsv` row.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn itemstatcost_ops_as_stated() {
    let st = StatTable::from_fixed(table("itemstatcost")).expect("stat table");
    let n = st.len() as u16;
    let info = |s: u16| st.get(s).expect("valid stat");
    let mut by_op: BTreeMap<u8, Vec<u16>> = BTreeMap::new();
    for s in 0..n {
        if info(s).op != 0 {
            by_op.entry(info(s).op).or_default().push(s);
        }
    }
    let per_level: Vec<u16> = [216, 217].into_iter().chain(220..=250).collect();
    let by_time: Vec<u16> = [268, 270, 271, 272].into_iter().chain(274..=303).collect();
    let expected: BTreeMap<u8, Vec<u16>> = BTreeMap::from([
        (1, vec![162, 163]),
        (2, per_level.clone()),
        (4, vec![214, 218]),
        (5, vec![215, 219]),
        (6, by_time),
        (7, vec![269, 273]),
        (8, vec![1]),
        (9, vec![3]),
        (11, vec![76, 77]),
        (13, vec![16, 17, 18, 75, 94]),
    ]);
    assert_eq!(by_op, expected, "stats by op (§6.3)");
    let histogram: Vec<(u8, usize)> = by_op.iter().map(|(&o, v)| (o, v.len())).collect();
    assert_eq!(
        histogram,
        [
            (1, 2),
            (2, 33),
            (4, 2),
            (5, 2),
            (6, 34),
            (7, 2),
            (8, 1),
            (9, 1),
            (11, 2),
            (13, 5)
        ],
        "ops used (Constants)"
    );
    // Ops 2, 4, 5 read level (12) as their op base (§6.3).
    for &s in per_level.iter().chain(&[214, 215, 218, 219]) {
        assert_eq!(info(s).op_base, 12, "op base of {s}");
    }
    assert_eq!(
        (0..n).filter(|&s| info(s).a52).count(),
        42,
        "op targets (A52)"
    );
    let entries = |t: u16| -> Vec<(u16, u16, u8, u8)> {
        info(t)
            .entries
            .iter()
            .map(|e| (e.base, e.source, e.op, e.param))
            .collect()
    };
    assert_eq!(
        entries(7),
        [
            (0xFFFF, 3, 9, 0),
            (0xFFFF, 76, 11, 0),
            (12, 216, 2, 3),
            (0xFFFF, 270, 6, 0)
        ],
        "entries(7)"
    );
    // §6.3 targets: op 8 → maxmana, op 9 → maxhp and maxstamina, op 11
    // 76 → maxhp and 77 → maxmana, op 1 (162, 163) → maxstamina.
    let has = |t: u16, source: u16, op: u8| {
        info(t)
            .entries
            .iter()
            .any(|e| e.source == source && e.op == op)
    };
    assert!(has(9, 1, 8), "op 8: energy → maxmana");
    assert!(has(7, 3, 9) && has(11, 3, 9), "op 9: vitality → 7, 11");
    assert!(has(7, 76, 11) && has(9, 77, 11), "op 11: 76 → 7, 77 → 9");
    assert!(has(11, 162, 1) && has(11, 163, 1), "op 1: 162, 163 → 11");
    assert_eq!(info(12).deps, (214..=250).collect::<Vec<u16>>(), "deps(12)");
    assert_eq!(
        (0..n).filter(|&s| info(s).a53).collect::<Vec<u16>>(),
        [214, 215, 218, 219],
        "A53"
    );
    // `stat-ops.tsv`: every op the data uses (and every op an entry
    // carries) has its row; ops 3, 10, 12 are in the table but unused.
    let tsv_ops: Vec<u8> = include_str!("../../../tables/sim/stat-ops.tsv")
        .lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('\t').next().unwrap().parse().expect("op number"))
        .collect();
    assert_eq!(tsv_ops, (1..=13).collect::<Vec<u8>>(), "stat-ops.tsv rows");
    for s in 0..n {
        for e in &info(s).entries {
            assert!(tsv_ops.contains(&e.op), "entry op {} of {s}", e.op);
            assert_eq!(op_row(e.op).map(|r| r.op), Some(e.op), "op_row({})", e.op);
        }
    }
    for unused in [3, 10, 12] {
        assert!(!by_op.contains_key(&unused), "op {unused} used");
    }
}

/// `fixups.md` §2 step 3, rebuilt from the typed op columns of every
/// stat: A51, A52, A53, deps and entries of the fixed-up records; and
/// `stats.md` edge case 5: the op graph has no cycle.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn op_tables_rebuild_from_the_columns() {
    let isc: Vec<Itemstatcost> = rows();
    let st = StatTable::from_fixed(table("itemstatcost")).expect("stat table");
    let n = isc.len();
    let mut a51 = vec![false; n];
    let mut a52 = vec![false; n];
    let mut a53 = vec![false; n];
    let mut deps: Vec<Vec<u16>> = vec![Vec::new(); n];
    let mut entries: Vec<Vec<OpEntry>> = vec![Vec::new(); n];
    for (i, c) in isc.iter().enumerate() {
        if c.op == 0 || c.op > 13 {
            continue;
        }
        let base = usize::from(c.op_base);
        if base < n {
            a51[base] = true;
            if deps[base].len() < 64 {
                deps[base].push(i as u16);
            }
            if c.op == 4 || c.op == 5 {
                a53[i] = true;
            }
        }
        for s in [c.op_stat1, c.op_stat2, c.op_stat3] {
            let s = usize::from(s);
            if s >= n {
                break;
            }
            a51[i] = true;
            if entries[s].len() < 16 {
                entries[s].push(OpEntry {
                    base: c.op_base,
                    source: i as u16,
                    op: c.op,
                    param: c.op_param,
                });
                a52[s] = true;
            }
        }
    }
    for s in 0..n {
        let got = st.get(s as u16).unwrap();
        assert_eq!(
            (got.a51, got.a52, got.a53),
            (a51[s], a52[s], a53[s]),
            "A51/A52/A53 of {s}"
        );
        assert_eq!(got.deps, deps[s], "deps({s})");
        assert_eq!(got.entries, entries[s], "entries({s})");
    }
    // Edge case 5: no cycle (source → target edges, self-loops included).
    fn visit(s: usize, entries: &[Vec<OpEntry>], mark: &mut [u8]) {
        match mark[s] {
            1 => panic!("op graph cycle through stat {s}"),
            2 => return,
            _ => {}
        }
        mark[s] = 1;
        // Edges source → target: the targets are the stats whose entries
        // name s as source.
        for (t, es) in entries.iter().enumerate() {
            if es.iter().any(|e| usize::from(e.source) == s) {
                visit(t, entries, mark);
            }
        }
        mark[s] = 2;
    }
    let mut mark = vec![0u8; n];
    for s in 0..n {
        visit(s, &entries, &mut mark);
    }
}

// ---- vitals.md: charstats, experience, creation, level-up, points --------------------

/// `vitals.md` Constants: the charstats columns of the 7 classes and the
/// experience table facts; `levels.md` OQ2 / §1 step 3: the skill-level
/// cap is experience row 0 of class 0.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn charstats_and_experience_as_stated() {
    let cs: Vec<Charstats> = rows();
    #[rustfmt::skip]
    let expected: [[u8; 13]; 7] = [
        // str dex int vit stamina hpadd L/Lvl S/Lvl M/Lvl L/Vit S/Vit M/Mag StatPerLevel
        [20, 25, 15, 20, 84, 30, 8, 4, 6, 12, 4, 6, 5], // Amazon
        [10, 25, 35, 10, 74, 30, 4, 4, 8, 8, 4, 8, 5],  // Sorceress
        [15, 25, 25, 15, 79, 30, 6, 4, 8, 8, 4, 8, 5],  // Necromancer
        [25, 20, 15, 25, 89, 30, 8, 4, 6, 12, 4, 6, 5], // Paladin
        [30, 20, 10, 25, 92, 30, 8, 4, 4, 16, 4, 4, 5], // Barbarian
        [15, 20, 20, 25, 84, 30, 6, 4, 8, 8, 4, 8, 5],  // Druid
        [20, 20, 25, 20, 95, 30, 8, 5, 6, 12, 5, 7, 5], // Assassin
    ];
    assert!(cs.len() >= 7, "charstats rows: {}", cs.len());
    for (class, want) in expected.iter().enumerate() {
        let c = &cs[class];
        let got = [
            c.str,
            c.dex,
            c.int,
            c.vit,
            c.stamina,
            c.hpadd,
            c.lifeperlevel,
            c.staminaperlevel,
            c.manaperlevel,
            c.lifepervitality,
            c.staminapervitality,
            c.manapermagic,
            c.statperlevel,
        ];
        assert_eq!(&got, want, "charstats class {class}");
    }
    let vt = vitals_tables();
    for class in 0..7 {
        assert_eq!(vt.max_level(class), 99, "MaxLvl of class {class}");
        assert_eq!(vt.threshold(class, 1), 500, "threshold({class}, 1)");
        assert_eq!(
            vt.threshold(class, 99),
            3_837_739_017,
            "threshold({class}, 99)"
        );
    }
    assert_eq!(vt.level_from_exp(0, 499), 1);
    assert_eq!(vt.level_from_exp(0, 500), 2);
    assert_eq!(vt.level_from_exp(0, u32::MAX), 99, "edge case 5");
    assert_eq!(vt.max_level(0) as i32, LEVEL_CAP_114D);
}

/// The vitals' calls no spec provides: counted only.
#[derive(Default)]
struct Rest {
    level_ups: u32,
}

impl VitalsRest for Rest {
    fn refresh(&mut self, _: UnitId) {}
    fn level_up_notify(&mut self, _: UnitId) {
        self.level_ups += 1;
    }
    fn level_up_event(&mut self, _: UnitId) {}
}

/// One player on the live stat data: its unit record and extended list
/// (server callback, `stat-lists.md` §4.3).
struct Player {
    units: Units,
    lists: StatLists,
    host: NoHost,
    rest: Rest,
}

const PU: UnitId = UnitId(1);

impl Player {
    fn new(data: &Arc<StatData>, class: u32) -> Self {
        let mut units = Units::new();
        units.insert(PU, UnitRecord::new(UnitType::Player, class, 1));
        let mut lists = StatLists::new(data.clone());
        let mut host = NoHost;
        lists.alloc_extended(
            &mut host,
            PU,
            UnitType::Player,
            1,
            class,
            0,
            Some(ValueCallback::Server),
        );
        Self {
            units,
            lists,
            host,
            rest: Rest::default(),
        }
    }

    fn view(&mut self) -> VitalsView<'_, NoHost, Rest> {
        VitalsView {
            units: &self.units,
            stats: &mut self.lists,
            hooks: &mut self.host,
            rest: &mut self.rest,
        }
    }
}

/// `vitals.md` §1 for all 7 classes on the live charstats and
/// experience, through the real stat lists (`VitalsView`); the
/// Sorceress and Barbarian vectors.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn player_creation_every_class() {
    let data = stat_data();
    let vt = vitals_tables();
    for class in 0..7u32 {
        let c = vt.charstats(class as i32).expect("charstats row").clone();
        let mut p = Player::new(&data, class);
        let mut v = p.view();
        init_player_stats(&mut v, &vt, PU, 0);
        let life = (i32::from(c.vit) + i32::from(c.hpadd)) << 8;
        let mana = i32::from(c.int) << 8;
        let stamina = i32::from(c.stamina) << 8;
        for (s, want) in [
            (vstat::STRENGTH, i32::from(c.str)),
            (vstat::ENERGY, i32::from(c.int)),
            (vstat::DEXTERITY, i32::from(c.dex)),
            (vstat::VITALITY, i32::from(c.vit)),
            (vstat::TOHIT, 0),
            (vstat::TOBLOCK, 0),
            (vstat::HITPOINTS, life),
            (vstat::MAXHP, life),
            (vstat::MANA, mana),
            (vstat::MAXMANA, mana),
            (vstat::STAMINA, stamina),
            (vstat::MAXSTAMINA, stamina),
            (vstat::LEVEL, 1),
            (vstat::NEXTEXP, 500),
            (vstat::ATTACKRATE, 100),
            (vstat::VELOCITYPERCENT, 100),
            (vstat::OTHER_ANIMRATE, 100),
        ] {
            assert_eq!(v.base_stat(PU, s), want, "class {class} stat {s}");
        }
        assert_eq!(
            (v.max_life(PU), v.max_mana(PU), v.max_stamina(PU)),
            (life, mana, stamina),
            "class {class} maxima"
        );
        let created = (v.stat(PU, 6), v.stat(PU, 8), v.stat(PU, 10));
        match class {
            SORCERESS => assert_eq!(created, (10240, 8960, 18944), "Sorceress created"),
            BARBARIAN => assert_eq!(created, (14080, 2560, 23552), "Barbarian created"),
            _ => {}
        }
    }
}

/// `vitals.md` Test vectors: level 1 → 10 for the Sorceress and the
/// Barbarian (§3, §4.1), +10 vitality (Barbarian) and +10 energy
/// (Sorceress) spent (§2).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn level_up_and_stat_point_vectors() {
    let data = stat_data();
    let vt = vitals_tables();
    for (class, maxima) in [
        (SORCERESS, (12544, 13568, 21248)),
        (BARBARIAN, (18688, 4864, 25856)),
    ] {
        let mut p = Player::new(&data, class);
        let mut v = p.view();
        init_player_stats(&mut v, &vt, PU, 0);
        // Experience of level 10 (row 10 = threshold(class, 9)).
        v.set_base_stat(PU, vstat::EXPERIENCE, vt.threshold(class as i32, 9) as i32);
        assert_eq!(level_up(&mut v, &vt, PU), 9, "class {class} d");
        assert_eq!(v.base_stat(PU, vstat::LEVEL), 10);
        assert_eq!(
            v.base_stat(PU, vstat::NEXTEXP),
            vt.threshold(class as i32, 10) as i32
        );
        assert_eq!(
            (v.max_life(PU), v.max_mana(PU), v.max_stamina(PU)),
            maxima,
            "class {class} maxima at level 10"
        );
        assert_eq!(
            (v.stat(PU, 6), v.stat(PU, 8), v.stat(PU, 10)),
            maxima,
            "class {class}: life, mana, stamina refilled"
        );
        assert_eq!(v.base_stat(PU, vstat::STATPTS), 45, "statpts +45");
        assert_eq!(v.base_stat(PU, vstat::NEWSKILLS), 9, "newskills +9");
        assert_eq!(p.rest.level_ups, 1);
    }

    // Barbarian, +10 vitality: max life +10240, max stamina +2560.
    let mut p = Player::new(&data, BARBARIAN);
    let mut v = p.view();
    init_player_stats(&mut v, &vt, PU, 0);
    let (life, stamina) = (v.max_life(PU), v.max_stamina(PU));
    v.set_base_stat(PU, vstat::STATPTS, 10);
    for k in 0..10 {
        assert!(spend(&mut v, &vt, PU, 3), "vitality point {k}");
    }
    assert!(!spend(&mut v, &vt, PU, 3), "no point left");
    assert_eq!(v.max_life(PU), life + 10240);
    assert_eq!(v.max_stamina(PU), stamina + 2560);
    assert_eq!(v.base_stat(PU, vstat::STATPTS), 0);

    // Sorceress, +10 energy: max mana +5120.
    let mut p = Player::new(&data, SORCERESS);
    let mut v = p.view();
    init_player_stats(&mut v, &vt, PU, 0);
    let mana = v.max_mana(PU);
    v.set_base_stat(PU, vstat::STATPTS, 10);
    for _ in 0..10 {
        assert!(spend(&mut v, &vt, PU, 1));
    }
    assert_eq!(v.max_mana(PU), mana + 5120);
}

// ---- stat-lists.md on live records ---------------------------------------------------

/// Logs callbacks; frees state lists like the synthetic vector's host.
#[derive(Default)]
struct Log {
    callbacks: Vec<(u16, i32, i32)>,
    removed: Vec<(UnitId, u32)>,
}

impl StatHost for Log {
    fn on_callback(&mut self, _: &StatLists, ev: &CallbackEvent) {
        self.callbacks.push((key_stat(ev.key), ev.old, ev.new));
    }
    fn list_removed(
        &mut self,
        lists: &mut StatLists,
        unit: UnitId,
        state: u32,
        _: ListId,
        _: RemoveCallback,
    ) {
        lists.toggle_state(unit, state, false);
        self.removed.push((unit, state));
    }
}

fn full(lists: &StatLists, l: ListId) -> Vec<(u16, i32)> {
    lists
        .full_entries(l)
        .into_iter()
        .map(|(k, v)| (key_stat(k), v))
        .collect()
}

/// `stat-lists.md` Test vectors, steps 1–4 and 6, on the live
/// itemstatcost and states tables with the live Barbarian record (its
/// LifePerVitality 16 and StaminaPerVitality 4 are the vector's class,
/// `vitals.md` Constants). Same expected values as the synthetic run.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn stat_list_vector_on_live_records() {
    let mut log = Log::default();
    let mut lists = StatLists::new(stat_data());
    let p = lists.alloc_extended(
        &mut log,
        PU,
        UnitType::Player,
        1,
        BARBARIAN,
        0,
        Some(ValueCallback::Server),
    );
    for (s, v) in [(0, 30), (12, 10), (3, 25), (7, 12800), (6, 12800)] {
        lists.set(&mut log, p, s, v, 0, None);
    }
    assert_eq!(
        full(&lists, p),
        [(0, 30), (3, 25), (6, 12800), (7, 12800), (12, 10)]
    );
    assert_eq!(log.callbacks, [(7, 0, 12800)]);
    let mods: Vec<u16> = lists.mods(p).into_iter().map(key_stat).collect();
    assert_eq!(mods, [0, 3, 7, 12]);

    log.callbacks.clear();
    let item = UnitId(2);
    let i = lists.alloc_extended(&mut log, item, UnitType::Item, 7, 0, 0, None);
    for (s, v) in [(3, 10), (216, 8), (19, 5)] {
        lists.set(&mut log, i, s, v, 0, None);
    }
    lists.attach(&mut log, PU, i, true);
    assert_eq!(log.callbacks, [(7, 12800, 23050), (11, 0, 2560)]);
    assert_eq!(lists.base(p, 6, 0), 23050);
    assert_eq!(
        full(&lists, p),
        [
            (0, 30),
            (3, 35),
            (6, 23050),
            (7, 23050),
            (11, 2560),
            (12, 10),
            (19, 5)
        ]
    );

    log.callbacks.clear();
    lists.detach(&mut log, i);
    assert_eq!(log.callbacks, [(7, 23050, 12800), (11, 2560, 0)]);
    assert_eq!(lists.base(p, 6, 0), 12800);
    assert_eq!(
        full(&lists, p),
        [(0, 30), (3, 25), (6, 12800), (7, 12800), (12, 10)]
    );

    let s = lists.alloc(0, 0, 0, 1);
    lists.set_state(s, 30);
    lists.set_expire(s, 20);
    lists.set(&mut log, s, 0, 5, 0, None);
    lists.set_remove_callback(s, Some(RemoveCallback(1)));
    lists.attach(&mut log, PU, s, true);
    lists.toggle_state(PU, 30, true);
    assert_eq!(lists.total(p, 0, 0), 35);
    assert!(lists.has_state(PU, 30));
    lists.expire_lists(&mut log, PU, 19).unwrap();
    assert!(lists.is_live(s));
    lists.expire_lists(&mut log, PU, 20).unwrap();
    assert!(!lists.is_live(s));
    assert_eq!(lists.total(p, 0, 0), 30);
    assert_eq!(log.removed, [(PU, 30)]);
    assert!(!lists.has_state(PU, 30));
}

/// `stat-lists.md` §7.2 rule 2 on every live monstats row: setting a
/// monster's max life sets stat 74 to ((new >> 8) · DamageRegen) >> 4
/// when `DamageRegen` ≠ 0, and leaves it 0 otherwise.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn monster_damage_regen_every_class() {
    let data = stat_data();
    let monstats: Vec<Monstats> = rows();
    assert_eq!(data.damage_regen.len(), monstats.len());
    let mut lists = StatLists::new(data);
    let mut host = NoHost;
    let new = 100 << 8;
    let mut with_regen = 0;
    for (class, m) in monstats.iter().enumerate() {
        let u = UnitId(1000 + class as u32);
        let l = lists.alloc_extended(
            &mut host,
            u,
            UnitType::Monster,
            class as u32,
            class as u32,
            0,
            Some(ValueCallback::Server),
        );
        lists.set(&mut host, l, 7, new, 0, None);
        let want = if m.damageregen != 0 {
            with_regen += 1;
            ((new >> 8).wrapping_mul(m.damageregen as i32)) >> 4
        } else {
            0
        };
        assert_eq!(lists.total(l, 74, 0), want, "monstats row {class}");
        lists.free_unit_list(&mut host, u);
    }
    println!(
        "{} monstats rows, {with_regen} with DamageRegen",
        monstats.len()
    );
}

// ---- units.md §4.2 on every AnimData record -------------------------------------------

/// `units.md` §4.2 main form (bonus 0, f = 100) on every live AnimData
/// record with its own speed as s and frames · 256 as F: the U4 shape
/// (action events in frame order, a1 ∈ 1–4, numbered a2, exactly one
/// event 1, last) and the end frame f + max(⌈F / s⌉, 2); with s ≤ 256
/// every frame index below min(frames, 144) is read.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn anim_schedule_every_record() {
    const F0: i32 = 100;
    let (mut checked, mut skipped) = (0, 0);
    for rec in live().anim.buckets.iter().flatten() {
        let Ok(s) = i16::try_from(rec.speed).map(i32::from) else {
            skipped += 1;
            continue;
        };
        let frames = rec.frames as i32;
        let fc = frames.wrapping_mul(256);
        let events: &[u8; ANIM_EVENTS] = &rec.events;
        let ev = Events::Record {
            byte_0f: (rec.speed >> 24) as u8,
            events,
        };
        let name = String::from_utf8_lossy(&rec.name).into_owned();
        let sched = schedule(Form::Main { bonus: 0 }, F0, s, fc, 0, ev)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .expect("main form always schedules");
        assert!(!sched.cancels, "{name}");
        let (last, actions) = sched.events.split_last().expect("event 1");
        assert_eq!(last.event, event::END_ANIM, "{name}: last is event 1");
        assert_eq!((last.a1, last.a2), (0, 0), "{name}");
        if s == 0 {
            assert_eq!(last.expire, F0 + 1, "{name}: speed 0");
            assert!(actions.is_empty(), "{name}");
            checked += 1;
            continue;
        }
        assert_eq!(sched.frame, Some(F0 * 256), "{name}: +0x44");
        let end = F0 + ((fc + s - 1) / s).max(2);
        assert_eq!(last.expire, end, "{name}: end frame");
        let mut prev = F0 + 1;
        let mut k = 0;
        for a in actions {
            assert_eq!(a.event, event::MODE_CHANGE, "{name}: one event 1");
            assert!(a.expire >= prev && a.expire < end, "{name}: order");
            prev = a.expire;
            match a.a1 {
                1 | 2 | 4 => {
                    assert_eq!(a.a2, k, "{name}: numbering");
                    k += 1;
                }
                3 => assert_eq!(a.a2, 0, "{name}: event 3 unnumbered"),
                other => panic!("{name}: a1 {other}"),
            }
        }
        if s <= 256 {
            let read = (frames.max(0) as usize).min(ANIM_EVENTS);
            let want = rec.events[..read]
                .iter()
                .filter(|&&e| (1..=4).contains(&e))
                .count();
            assert_eq!(actions.len(), want, "{name}: events of frames < {read}");
        }
        checked += 1;
    }
    println!("{checked} AnimData records scheduled, {skipped} with a speed past i16");
    assert!(checked > 0);
}

// ---- skills/levels.md on the live skills table ----------------------------------------

/// A unit with no stats, skills or items: the formula context of a unit
/// that exists (its seed for `rand`), every read 0.
struct Bare {
    seed: Seed,
}

impl SkillUnits for Bare {
    type Unit = u32;
    type Item = u32;
    fn unit_type(&self, _: u32) -> UnitType {
        UnitType::Player
    }
    fn class_id(&self, _: u32) -> i32 {
        0
    }
    fn stat(&self, _: u32, _: u16, _: u16) -> i32 {
        0
    }
    fn item_stat(&self, _: u32, _: u16, _: u16) -> i32 {
        0
    }
    fn base_stat(&self, _: u32, _: u16, _: u16) -> i32 {
        0
    }
    fn formula_stat(&self, _: u32, _: u16, _: i32) -> i32 {
        0
    }
    fn stat_entries(&self, _: u32, _: u16, _: usize) -> Vec<(u16, i32)> {
        Vec::new()
    }
    fn has_state(&self, _: u32, _: u16) -> bool {
        false
    }
    fn state_stat(&self, _: u32, _: u16, _: u16) -> Option<i32> {
        None
    }
    fn seed(&mut self, _: u32) -> &mut Seed {
        &mut self.seed
    }
    fn skill_list(&self, _: u32) -> Vec<SkillEntry> {
        Vec::new()
    }
    fn used_skill(&self, _: u32) -> Option<SkillEntry> {
        None
    }
    fn current_weapon(&self, _: u32) -> Option<u32> {
        None
    }
    fn weapon(&self, _: u32) -> Option<u32> {
        None
    }
    fn item_at(&self, _: u32, _: u8) -> Option<u32> {
        None
    }
    fn item_is(&self, _: u32, _: i32) -> bool {
        false
    }
    fn itype_is(&self, _: i32, _: i32) -> bool {
        false
    }
    fn wield_type(&self, _: u32) -> i32 {
        0
    }
    fn item_damage(&self, _: u32, _: bool) -> i32 {
        0
    }
    fn str_dex_bonus(&self, _: u32) -> (i32, i32) {
        (0, 0)
    }
    fn item_flag_throw(&self, _: u32) -> bool {
        false
    }
    fn missile_level(&self, _: u32) -> i32 {
        0
    }
}

fn skill_tables() -> SkillTables {
    SkillTables::from_bin(&live().bin, LEVEL_CAP_114D).expect("skill tables")
}

/// Every formula field of a skills record.
fn calc_fields(r: &Skills) -> [u32; 32] {
    [
        r.prgcalc1,
        r.prgcalc2,
        r.prgcalc3,
        r.auralencalc,
        r.aurarangecalc,
        r.aurastatcalc1,
        r.aurastatcalc2,
        r.aurastatcalc3,
        r.aurastatcalc4,
        r.aurastatcalc5,
        r.aurastatcalc6,
        r.passivecalc1,
        r.passivecalc2,
        r.passivecalc3,
        r.passivecalc4,
        r.passivecalc5,
        r.petmax,
        r.sumsk1calc,
        r.sumsk2calc,
        r.sumsk3calc,
        r.sumsk4calc,
        r.sumsk5calc,
        r.cltcalc1,
        r.cltcalc2,
        r.cltcalc3,
        r.skpoints,
        r.calc1,
        r.calc2,
        r.calc3,
        r.calc4,
        r.tohitcalc,
        r.dmgsympercalc,
    ]
}

const LEVELS: [i32; 9] = [1, 2, 5, 10, 20, 25, 30, 60, 99];

/// `levels.md` §2–§5 on every live skill: every formula field and the
/// level-dependent values (elemental and physical damage, length, mana,
/// to-hit) evaluate at the sampled levels from 1 to 99, without and with
/// a unit, and give
/// the same value when evaluated again (no hidden state; `rand` draws
/// only from the unit's seed, which is reset per evaluation).
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn every_skill_level_calc_evaluates() {
    let t = skill_tables();
    let mut evaluated = 0usize;
    for (i, rec) in t.skills.iter().enumerate() {
        let skill = i as i32;
        let mut fields: Vec<u32> = calc_fields(rec).to_vec();
        fields.extend([rec.edmgsympercalc, rec.elensympercalc]);
        for lvl in LEVELS {
            let run = |unit: Option<u32>| {
                let mut w = Bare {
                    seed: Seed::init_low(1),
                };
                let mut out: Vec<i32> = fields
                    .iter()
                    .filter(|&&f| f != NO_CALC)
                    .map(|&f| eval_skill(&mut w, &t, unit, f, skill, lvl))
                    .collect();
                out.extend([
                    elem_min(&mut w, &t, unit, skill, lvl, false),
                    elem_max(&mut w, &t, unit, skill, lvl, false),
                    elem_min(&mut w, &t, unit, skill, lvl, true),
                    elem_max(&mut w, &t, unit, skill, lvl, true),
                    elem_len(&mut w, &t, unit, skill, lvl),
                    phys_min(&mut w, &t, unit, skill, lvl, false),
                    phys_max(&mut w, &t, unit, skill, lvl, false),
                    mana_cost(rec, lvl),
                    mana_cost_shifted(&t, skill, lvl),
                    to_hit(&mut w, &t, unit, skill, lvl),
                ]);
                out
            };
            for unit in [None, Some(0)] {
                let a = run(unit);
                let b = run(unit);
                assert_eq!(a, b, "skill {skill} level {lvl} unit {unit:?}");
                evaluated += a.len();
            }
        }
    }
    println!("{} skills, {evaluated} values evaluated", t.skills.len());
}

fn s32(v: u32) -> i32 {
    v as i32
}

/// `levels.md` Constants: row count and the column counts of the live
/// `skills.txt`.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn skills_table_facts() {
    let t = skill_tables();
    let s = &t.skills;
    assert_eq!(s.len(), 357, "skills rows");
    let count = |f: &dyn Fn(&Skills) -> bool| s.iter().filter(|r| f(r)).count();
    assert_eq!(
        count(&|r| r.edmgsympercalc != NO_CALC),
        64,
        "EDmgSymPerCalc"
    );
    assert_eq!(count(&|r| r.dmgsympercalc != NO_CALC), 6, "DmgSymPerCalc");
    assert_eq!(count(&|r| r.elensympercalc != NO_CALC), 4, "ELenSymPerCalc");
    assert_eq!(count(&|r| r.tohitcalc != NO_CALC), 3, "ToHitCalc");
    assert_eq!(count(&|r| r.srcdam != 0), 63, "SrcDam");
    assert_eq!(count(&|r| r.skpoints != NO_CALC), 0, "skpoints");
    assert_eq!(count(&|r| r.hitshift == 8), 316, "HitShift 8");
    assert_eq!(count(&|r| r.hitshift == 7), 20, "HitShift 7");
    let negative: Vec<usize> = (0..s.len())
        .filter(|&i| (s[i].lvlmana as i16) < 0)
        .collect();
    assert_eq!(negative.len(), 7, "negative lvlmana: {negative:?}");
    const TELEPORT: usize = 54;
    assert!(
        negative.contains(&TELEPORT),
        "Teleport has negative lvlmana"
    );
}

/// `levels.md` Test vectors not covered by `skills::tests::real_skill_vectors`:
/// dm12 / dm56 / ln34 / ln12 special values, the linear to-hit and the
/// Teleport mana past zero. Each skill is identified by its id and then
/// checked to hold the parameters the vector names.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn special_value_vectors() {
    let t = skill_tables();
    let code = |c: &str| {
        SKILLCALC_CODES
            .iter()
            .position(|&x| x == c)
            .unwrap_or_else(|| panic!("code {c}")) as u8
    };
    let params = |skill: usize| {
        let r = &t.skills[skill];
        [
            r.param1, r.param2, r.param3, r.param4, r.param5, r.param6, r.param7, r.param8,
        ]
        .map(s32)
    };
    let mut w = Bare {
        seed: Seed::init_low(1),
    };
    let mut sv =
        |c: &str, skill: usize, lvl: i32| special(&mut w, &t, None, code(c), skill as i32, lvl);

    const CRITICAL_STRIKE: usize = 9;
    const DODGE: usize = 13;
    const AMPLIFY_DAMAGE: usize = 66;
    const LOWER_RESIST: usize = 91;
    const SACRIFICE: usize = 96;
    const BASH: usize = 126;
    const TELEPORT: usize = 54;

    assert_eq!(params(CRITICAL_STRIKE)[..2], [5, 80]);
    assert_eq!(
        [1, 2, 5, 10, 20, 30, 99].map(|l| sv("dm12", CRITICAL_STRIKE, l)),
        [16, 25, 42, 56, 68, 73, 80],
        "dm12 Critical Strike"
    );
    assert_eq!(params(DODGE)[..2], [10, 65]);
    assert_eq!(
        [1, 20].map(|l| sv("dm12", DODGE, l)),
        [18, 56],
        "dm12 Dodge"
    );
    assert_eq!(params(LOWER_RESIST)[4..6], [25, 70]);
    assert_eq!(
        [1, 20].map(|l| sv("dm56", LOWER_RESIST, l)),
        [31, 62],
        "dm56 Lower Resist"
    );
    assert_eq!(params(AMPLIFY_DAMAGE)[2..4], [200, 75]);
    assert_eq!(
        [1, 10].map(|l| sv("ln34", AMPLIFY_DAMAGE, l)),
        [200, 875],
        "ln34 Amplify Damage"
    );
    assert_eq!(params(BASH)[..2], [50, 5]);
    assert_eq!([1, 20].map(|l| sv("ln12", BASH, l)), [50, 145], "ln12 Bash");

    let mut w2 = Bare {
        seed: Seed::init_low(1),
    };
    let sac = &t.skills[SACRIFICE];
    assert_eq!((s32(sac.tohit), s32(sac.levtohit)), (20, 7));
    assert_eq!(sac.tohitcalc, NO_CALC);
    assert_eq!(
        [1, 20].map(|l| to_hit(&mut w2, &t, None, SACRIFICE as i32, l)),
        [20, 153],
        "Sacrifice to-hit"
    );

    let tele = &t.skills[TELEPORT];
    assert_eq!(
        [25, 30].map(|l| mana_cost(tele, l)),
        [0, -1280],
        "Teleport usmc"
    );
    assert_eq!(
        mana_cost_shifted(&t, TELEPORT as i32, 25),
        0,
        "Teleport L25 shifted"
    );
    assert_eq!(sv("mana", TELEPORT, 30), -5, "Teleport L30 `mana`");
}

/// The states table's curse flag (11) selects exactly the listed states,
/// including shrine states 128-137. This checks table membership only;
/// timed-state replacement is not exercised here.
#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn shrine_states_are_in_the_curse_group() {
    let states = StateTable::new(table("states"), &live().fixed.states).expect("states");
    let curse: Vec<u32> = (0..states.count() as u32)
        .filter(|&s| states.has_flag(s, 11))
        .collect();
    assert_eq!(
        curse,
        [
            9, 19, 23, 27, 55, 56, 57, 58, 59, 60, 61, 89, 113, 114, 128, 129, 130, 131, 132, 133,
            134, 135, 136, 137, 141, 156
        ]
    );
}
