// Spec: specs/data/schema.md, specs/data/fixups.md, specs/items/generation.md §1, specs/data/calc-expressions.md §3
//! Property tests (METHODS M07) of the `d2-sim` table readers: every
//! projection of a loaded or fixed-up table set (`from_bin`,
//! `from_fixed`, `from_table`, `from_tables`, `from_records`, the record
//! helpers) on sets of arbitrary records, then their lookups with any
//! caller index; and the formula evaluator on arbitrary code bytes. Any
//! input gives a value or a refusal, never a panic, a debug overflow or a
//! hang.
//!
//! Default case counts keep `cargo test` fast; set `PROPTEST_CASES` to
//! hunt harder.

#[path = "../../d2-data/tests/prop_common/mod.rs"]
mod prop_common;

use std::collections::BTreeMap;

use proptest::prelude::*;

use d2_data::bin::{BinSet, BinTable, CodeFile};
use d2_data::fixup::{self, FixedSet};
use d2_data::schema::{schema, CalcBuffer};
use d2_data::strings::StringTables;
use d2_data::tables::{decode_all, Record};
use d2_data::txt::TxtTable;
use d2_formats::animdata::AnimData;

use d2_sim::combat::vitals::VitalsTables;
use d2_sim::combat::CombatTables;
use d2_sim::drlg::data::DrlgData;
use d2_sim::drlg::maze::MazeData;
use d2_sim::drlg::outdoor::OutdoorData;
use d2_sim::items::ItemTables;
use d2_sim::monsters::init::{component_counts, monstats_extra};
use d2_sim::monsters::population::data::{chain_lengths, composits};
use d2_sim::monsters::population::PopTables;
use d2_sim::skills::calc::{eval, CalcContext};
use d2_sim::skills::SkillTables;
use d2_sim::stats::{StatTable, StateTable};
use d2_sim::units::hooks::UnitData;
use d2_sim::world::cube::recipes;
use d2_sim::world::npc::HireRow;
use d2_sim::world::vendors::VendorTables;

use prop_common::{bounded, config, Gen};

fn empty_txt() -> TxtTable {
    TxtTable {
        header: vec![b"Sound".to_vec()],
        records: Vec::new(),
        removed_lines: Vec::new(),
    }
}

/// Every runtime table with 0–`max` random records (`nasty`: any bytes;
/// else small values that pass the load-time refusals), and random code
/// buffers.
fn random_set(g: &mut Gen, max: usize, nasty: bool) -> BinSet {
    let tables = schema()
        .runtime()
        .map(|def| {
            let count = g.below(max + 1);
            BinTable {
                name: def.name.clone(),
                source: "prop".into(),
                count,
                record_size: def.record_size,
                records: (0..count)
                    .flat_map(|_| g.record_in(def.record_size, nasty))
                    .collect(),
            }
        })
        .collect();
    let code = CalcBuffer::ALL
        .iter()
        .map(|&buffer| {
            let n = g.below(64);
            let bytes = (0..n).map(|_| g.next() as u8).collect();
            let file = CodeFile {
                buffer,
                source: "prop".into(),
                bytes,
            };
            (buffer, file)
        })
        .collect();
    BinSet {
        lod: g.one_in(2),
        rules: Default::default(),
        strings: StringTables::default(),
        tables,
        code,
        code_reports: BTreeMap::new(),
        hitclass: BinTable {
            name: "hitclass".into(),
            source: "prop".into(),
            count: 0,
            record_size: 4,
            records: Vec::new(),
        },
        sounds: empty_txt(),
        soundenviron: empty_txt(),
    }
}

/// Tables whose fix-ups refuse most random rows; emptied so the readers
/// get a fixed-up set (`d2-data` `prop_data.rs` has the same list).
const REFUSING: &[&str] = &[
    "gamble",
    "gems",
    "monseq",
    "monpreset",
    "automap",
    "monstats",
    "charstats",
    "setitems",
    "uniqueitems",
];

fn table<'a>(tables: &'a [BinTable], name: &str) -> &'a BinTable {
    tables
        .iter()
        .find(|t| t.name == name)
        .expect("runtime table")
}

fn typed<T: Record>(tables: &[BinTable]) -> Vec<T> {
    decode_all::<T>(table(tables, T::TABLE)).expect("schema size")
}

/// Caller indices: small, at the edges, negative, huge.
const INDICES: [i64; 9] = [0, 1, 2, 7, 255, -1, -2, i32::MAX as i64, u32::MAX as i64];

/// The readers that take one table or typed records, on `tables`.
fn table_readers(tables: &[BinTable]) {
    use d2_data::tables::*;
    if let Ok(t) = StatTable::from_fixed(table(tables, "itemstatcost")) {
        let _ = t;
    }
    let _ = HireRow::from_table(table(tables, "hireling"));
    let _ = recipes(table(tables, "cubemain"));
    let _ = UnitData::new(table(tables, "monstats"), table(tables, "monstats2"));
    let _ = monstats_extra(table(tables, "monstats"));
    let _ = component_counts(table(tables, "monstats2"));
    let _ = chain_lengths(table(tables, "monstats"));
    let _ = composits(table(tables, "monstats2"));
    let pop = PopTables::from_records(
        &typed::<Levels>(tables),
        &typed::<Monstats>(tables),
        &typed::<Monstats2>(tables),
        &typed::<Superuniques>(tables),
    )
    .with_bins(table(tables, "monstats"), table(tables, "monstats2"));
    for i in INDICES {
        let _ = pop.mon(i as i32);
    }
    let leveldefs = typed::<Leveldefs>(tables);
    let lvlprest = typed::<Lvlprest>(tables);
    let drlg = DrlgData::from_tables(
        &leveldefs,
        &typed::<Lvlwarp>(tables),
        &typed::<Lvltypes>(tables),
        &typed::<Objects>(tables),
    );
    for i in INDICES {
        let _ = drlg.level(i as u32);
        let _ = drlg.lvlwarp_row(i as i32, i as u8);
        let _ = drlg.lvltype_file(i as u32, i as usize);
        let _ = drlg.is_waypoint_object(i as u32);
    }
    let _ = MazeData::from_tables(&typed::<Lvlmaze>(tables), &lvlprest);
    let _ = OutdoorData::from_tables(&leveldefs, &lvlprest, &typed::<Lvlsub>(tables));
    for p in &lvlprest {
        let _ = d2_sim::drlg::preset::PresetDef::from_record(p);
    }
}

/// The `from_bin` readers, the fix-ups, then the `from_fixed` readers.
fn all_readers(seed: u64, max: usize) {
    bounded(move || {
        let mut g = Gen::new(seed);
        let nasty = g.one_in(3);
        let mut set = random_set(&mut g, max, nasty);
        if let Ok(t) = SkillTables::from_bin(&set, g.below(200) as i32 - 50) {
            for i in INDICES {
                let _ = t.skill(i as i32);
                let _ = t.missile(i as i32);
            }
            for s in t.skills.iter().take(4) {
                let _ = t.skilldesc_of(s);
            }
        }
        let _ = CombatTables::from_bin(&set);
        let _ = VitalsTables::from_bin(&set);
        table_readers(&set.tables);

        if !g.one_in(3) {
            for t in &mut set.tables {
                if REFUSING.contains(&t.name.as_str()) {
                    t.count = 0;
                    t.records.clear();
                }
            }
        }
        let anim = AnimData::parse(&[0u8; 256 * 4]).expect("256 empty buckets");
        if let Ok(f) = fixup::apply(&set, &anim) {
            fixed_readers(&f);
            table_readers(&f.tables);
        }
    });
}

/// The readers of a fixed-up set and their lookups.
fn fixed_readers(f: &FixedSet) {
    if let Ok(t) = ItemTables::from_fixed(f) {
        for i in INDICES {
            let (u, s) = (i as usize, i as i16);
            let _ = t.item(u);
            let _ = t.itemtype(s);
            let _ = t.itype_of(u);
            let _ = t.is_type(u, s);
            let _ = t.stat_valid(i as u16);
            let _ = t.class_skills(u);
            for j in INDICES {
                let _ = t.is_type(j as usize, s);
            }
        }
        let _ = t.first_auto();
    }
    if let Ok(t) = VendorTables::from_fixed(f) {
        for i in INDICES {
            let _ = t.item(i as usize);
            let _ = t.type_of(i as usize);
            let _ = t.is_type(i as usize, i as u16);
            let _ = t.npc_row(i as u16);
            let code = (i as u32).to_le_bytes();
            let _ = t.find_code(code);
            let _ = t.valid_code(code);
        }
    }
    if let Some(states) = f.table("states") {
        let _ = StateTable::new(states, &f.states);
    }
}

/// A formula context with `count` functions of the given arities.
struct Ctx {
    arities: Vec<u8>,
    calls: usize,
}

impl CalcContext for Ctx {
    fn function_count(&self) -> u8 {
        self.arities.len() as u8
    }
    fn arity(&self, index: u8) -> u8 {
        self.arities[usize::from(index)]
    }
    fn param(&mut self, c: i32) -> i32 {
        c.wrapping_mul(3)
    }
    fn call(&mut self, index: u8, args: &[i32]) -> i32 {
        assert_eq!(args.len(), usize::from(self.arities[usize::from(index)]));
        self.calls += 1;
        args.iter()
            .fold(i32::from(index), |a, &b| a.wrapping_add(b))
    }
}

proptest! {
    #![proptest_config(config(48))]

    #[test]
    fn readers_on_random_sets(seed in any::<u64>(), max in 0usize..8) {
        all_readers(seed, max);
    }
}

proptest! {
    #![proptest_config(config(256))]

    /// `calc-expressions.md` §3: any code bytes and offset evaluate to a
    /// value; functions are called with their arity.
    #[test]
    fn calc_eval_any_code(
        code in prop::collection::vec(
            prop_oneof![0u8..0x18, any::<u8>()],
            0..200,
        ),
        offset in prop_oneof![0u32..200, Just(u32::MAX), any::<u32>()],
        arities in prop::collection::vec(0u8..6, 0..8),
    ) {
        let mut ctx = Ctx { arities, calls: 0 };
        let _ = eval(&code, offset, &mut ctx);
        prop_assert!(ctx.calls <= code.len());
    }
}

/// The empty set is accepted by every reader.
#[test]
fn empty_set_reads() {
    let mut g = Gen::new(0);
    let set = random_set(&mut g, 0, false);
    let anim = AnimData::parse(&[0u8; 256 * 4]).expect("256 empty buckets");
    let f = fixup::apply(&set, &anim).expect("empty set fixes up");
    assert!(ItemTables::from_fixed(&f).is_ok());
    assert!(VendorTables::from_fixed(&f).is_ok());
    table_readers(&f.tables);
}
