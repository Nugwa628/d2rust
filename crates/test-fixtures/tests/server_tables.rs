//! The synthetic install feeds every table view the server builds a game
//! from: items, treasure, vendors, stats and states, skills, combat,
//! vitals, units, DRLG, population, waypoints and the cube. Each is built
//! from the loaded (and fixed-up) set exactly as a game-file run would,
//! and values written in the synthetic rows are read back through it.

use std::path::PathBuf;
use std::sync::OnceLock;

use d2_data::bin::BinSet;
use d2_data::fixup::{self, FixedSet};
use d2_data::tables::{decode_all, Record};
use d2_sim::combat::vitals::VitalsTables;
use d2_sim::combat::CombatTables;
use d2_sim::drlg::maze::MazeData;
use d2_sim::drlg::outdoor::OutdoorData;
use d2_sim::drlg::DrlgData;
use d2_sim::items::ItemTables;
use d2_sim::monsters::population::PopTables;
use d2_sim::skills::SkillTables;
use d2_sim::stats::{StatTable, StateTable};
use d2_sim::treasure::{item_list, TcSources, TreasureClasses};
use d2_sim::units::hooks::UnitData;
use d2_sim::world::cube::CubeData;
use d2_sim::world::vendors::VendorTables;
use d2_sim::world::waypoints::WaypointData;
use test_fixtures::content::{self, CLASSES, LEVELS};
use test_fixtures::{install, synth};

struct Loaded {
    bins: BinSet,
    fixed: FixedSet,
}

fn loaded() -> &'static Loaded {
    static L: OnceLock<Loaded> = OnceLock::new();
    L.get_or_init(|| {
        // One directory per test process (nextest runs each test in its own).
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("synthetic-server-{}", std::process::id()));
        let i = install::build(&dir, &synth::synthetic()).unwrap_or_else(|e| panic!("{e}"));
        let anim = fixup::read_animdata(&i.archives).unwrap();
        let fixed = fixup::apply(&i.loaded, &anim).unwrap();
        Loaded {
            bins: i.loaded,
            fixed,
        }
    })
}

fn typed<T: Record>() -> Vec<T> {
    decode_all(loaded().fixed.table(T::TABLE).unwrap()).unwrap()
}

#[test]
fn item_and_vendor_tables() {
    let f = &loaded().fixed;
    let items = ItemTables::from_fixed(f).unwrap();
    assert_eq!(items.items.len(), 6, "2 weapons + 2 armor + 2 misc");
    assert_eq!(items.n_suffix, 2);
    assert_eq!(items.n_prefix, 1);
    assert_eq!(items.n_rare_suffix, 1);
    assert_eq!(items.valshift.len(), content::STATS.len());
    let vendors = VendorTables::from_fixed(f).unwrap();
    assert_eq!(vendors.items.len(), 6);
    assert_eq!(vendors.itemtypes.len(), 8);
}

#[test]
fn treasure_classes() {
    let items = item_list(&typed(), &typed(), &typed());
    let tcs = TreasureClasses::build(&TcSources {
        treasureclassex: &typed(),
        itemtypes: &typed(),
        items: &items,
        equiv: &loaded().fixed.itemtypes_equiv,
        uniqueitems: &typed(),
        setitems: &typed(),
    })
    .unwrap();
    // TC 0, 32 automatic TCs for each of `weap` and `armo`, 3 rows.
    assert_eq!(tcs.len(), 1 + 64 + 3);
    assert!(tcs.notes.is_empty(), "{:?}", tcs.notes);
    // beast1's TreasureClass1 is "Synth Act 1", the second row: 1 + 64 + 1.
    let m: Vec<d2_data::tables::Monstats> = typed();
    assert_eq!(m[0].treasureclass1, 66);
}

#[test]
fn stats_skills_combat_vitals() {
    let l = loaded();
    let stats = StatTable::from_fixed(l.fixed.table("itemstatcost").unwrap()).unwrap();
    assert_eq!(stats.len(), content::STATS.len());
    StateTable::new(l.fixed.table("states").unwrap(), &l.fixed.states).unwrap();
    let skills = SkillTables::from_bin(&l.bins, 20).unwrap();
    assert_eq!(skills.skills.len(), 4);
    assert_eq!(skills.missile(1).map(|m| m.range), Some(30));
    assert!(!skills.skills_code.is_empty());
    let combat = CombatTables::from_bin(&l.bins).unwrap();
    assert_eq!(combat.difficultylevels.len(), 3);
    assert_eq!(combat.hitclass.len(), 4);
    assert_eq!(&combat.hitclass[2], b"blde");

    let v = VitalsTables::from_bin(&l.bins).unwrap();
    assert_eq!(v.charstats.len(), CLASSES.len());
    for class in 0..7 {
        assert_eq!(v.max_level(class), content::MAX_LEVEL);
        assert_eq!(v.threshold(class, 10), content::exp_for_level(11));
    }
    assert_eq!(v.level_from_exp(3, content::exp_for_level(12)), 12);
    assert_eq!(v.level_from_exp(3, content::exp_for_level(12) - 1), 11);
}

#[test]
fn world_tables() {
    let f = &loaded().fixed;
    let units = UnitData::new(f.table("monstats").unwrap(), f.table("monstats2").unwrap());
    units.unwrap();

    let drlg = DrlgData::from_tables(&typed(), &typed(), &typed(), &typed());
    assert_eq!(drlg.levels.len(), LEVELS.len());
    assert_eq!(drlg.levels[3].drlg_type, 1);
    assert_eq!(drlg.levels[1].drlg_type, 2);
    let _maze = MazeData::from_tables(&typed(), &typed());
    let _outdoor = OutdoorData::from_tables(&typed(), &typed(), &typed());

    let pop = PopTables::from_records(&typed(), &typed(), &typed(), &typed())
        .with_bins(f.table("monstats").unwrap(), f.table("monstats2").unwrap());
    assert_eq!(pop.superuniques.len(), content::SUPERUNIQUES);
    assert_eq!(pop.monstats.len(), 3);

    let wp = WaypointData::new(&typed(), &typed());
    assert_eq!(wp.objects.len(), 3);
    assert_eq!(wp.objects[0].operate_fn, 23);

    let cube = CubeData::new(Vec::new(), &typed(), &typed(), &typed(), &typed(), &typed());
    assert_eq!(cube.max_level, content::MAX_LEVEL as i32);
    assert_eq!(cube.items.len(), 6);
}
