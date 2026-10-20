//! Game explorer G0/G1: reads any game on D2RUST into `game-map.json`,
//! applies the game's override file, writes `GAME.md` and runs the T-static
//! checks. Everything is read from the game's own tables; the override file
//! holds only what the data cannot say.

mod game_md;
mod goals;
mod model;
mod overrides;
mod reader;
mod static_checks;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use d2_sim::world::quests::QuestTables;

use super::single_player::{GameData, GameSource, Layers};

/// The map command's options.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub game_dir: Option<PathBuf>,
    pub native: Option<PathBuf>,
    pub mod_name: Option<String>,
    pub overrides: Option<PathBuf>,
    pub out: PathBuf,
    pub t_jump: bool,
    pub goals: Option<String>,
    pub limit: Option<usize>,
    pub jobs: usize,
    pub difficulty: String,
}

/// Parses `map [--game-dir DIR | --native DIR [--mod NAME]] [--overrides FILE] [--out DIR]`.
pub fn parse_args(args: &[String]) -> Result<Options> {
    let mut o = Options {
        out: "map-out".into(),
        jobs: 1,
        difficulty: "normal".into(),
        ..Options::default()
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if flag == "--t-jump" {
            o.t_jump = true;
            continue;
        }
        let value = it.next().with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--game-dir" => o.game_dir = Some(value.into()),
            "--native" => o.native = Some(value.into()),
            "--mod" => o.mod_name = Some(value.clone()),
            "--overrides" => o.overrides = Some(value.into()),
            "--out" => o.out = value.into(),
            "--goals" => o.goals = Some(value.clone()),
            "--limit" => o.limit = Some(value.parse().context("--limit must be a number")?),
            "--jobs" => o.jobs = value.parse().context("--jobs must be positive")?,
            "--difficulty" => {
                if !["normal", "nightmare", "hell"].contains(&value.as_str()) {
                    bail!("--difficulty must be normal, nightmare or hell");
                }
                o.difficulty = value.clone();
            }
            _ => bail!("unknown map option {flag}"),
        }
    }
    if o.native.is_some() && o.game_dir.is_some() {
        bail!("choose --game-dir or --native");
    }
    if o.mod_name.is_some() && o.native.is_none() {
        bail!("--mod NAME needs --native DIR");
    }
    if o.jobs == 0 {
        bail!("--jobs must be positive");
    }
    Ok(o)
}

fn source(o: &Options) -> Result<GameSource> {
    if let Some(dir) = &o.native {
        return Ok(GameSource::Native(
            dir.clone(),
            Layers::List(o.mod_name.iter().cloned().collect()),
        ));
    }
    let dir = o
        .game_dir
        .clone()
        .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from))
        .context("give --game-dir or set D2_GAME_DIR")?;
    Ok(GameSource::Archives(dir))
}

fn default_overrides(o: &Options) -> Option<PathBuf> {
    if o.overrides.is_some() {
        return o.overrides.clone();
    }
    let native = o.native.as_ref()?;
    let path = match &o.mod_name {
        Some(name) => native
            .join("mods")
            .join(name)
            .join("game-map.overrides.toml"),
        None => native.join("game-map.overrides.toml"),
    };
    path.is_file().then_some(path)
}

/// Builds and writes the map, the overrides summary and the T-static report.
pub fn run(o: &Options) -> Result<static_checks::Report> {
    let source = source(o)?;
    let label = match &source {
        GameSource::Archives(dir) => dir.display().to_string(),
        GameSource::Native(dir, _) => dir.display().to_string(),
    };
    let load_started = Instant::now();
    let data = GameData::open(&source)?;
    let load_ms = load_started.elapsed().as_millis();
    let GameData::Live(live) = &data;
    let quests = QuestTables::load().context("quests table")?;
    let (over, overrides_path) =
        overrides::load(default_overrides(o).as_deref()).map_err(anyhow::Error::msg)?;

    let started = Instant::now();
    let map = reader::build(&reader::Inputs {
        bins: &live.tables.bins,
        quests: &quests,
        overrides: &over,
        source: label,
        mod_name: o.mod_name.clone(),
        overrides_path,
    })
    .map_err(anyhow::Error::msg)?;
    let build_ms = started.elapsed().as_millis();

    let started = Instant::now();
    let report = static_checks::run(&map);
    let static_ms = started.elapsed().as_millis();

    std::fs::create_dir_all(&o.out)?;
    std::fs::write(
        o.out.join("game-map.json"),
        format!("{}\n", serde_json::to_string_pretty(&map)?),
    )?;
    std::fs::write(
        o.out.join("static.json"),
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    std::fs::write(o.out.join("GAME.md"), game_md::write(&map, &report))?;
    println!(
        "map: {} areas, {} quests, {} classes, {} items, {} recipes, {} runewords",
        map.summary.areas,
        map.summary.quests,
        map.summary.classes,
        map.summary.items,
        map.summary.cube_recipes,
        map.summary.runewords
    );
    println!(
        "map: data load {load_ms} ms, build {build_ms} ms, t-static {static_ms} ms, exit {}",
        report.exit
    );
    for rule in &report.rules {
        println!(
            "t-static {}: {} checked, {} findings{}",
            rule.rule,
            rule.checked,
            rule.findings.len() + rule.omitted,
            if rule.omitted > 0 {
                format!(" ({} shown)", rule.findings.len())
            } else {
                String::new()
            }
        );
    }
    println!("map: wrote {}", o.out.display());
    let mut report = report;
    if o.t_jump {
        let game_dir = o
            .game_dir
            .clone()
            .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from));
        let t_jump = goals::run(
            &map,
            &over,
            &goals::Options {
                jobs: o.jobs,
                filter: o.goals.clone(),
                limit: o.limit,
                difficulty: o.difficulty.clone(),
                out: o.out.clone(),
                game_dir,
            },
        )?;
        // A blocked goal fails the run unless it is marked pending.
        report.exit = report.exit.max(goals::exit_code(&t_jump));
    }
    Ok(report)
}

/// Headless command entry point.
pub fn main(args: &[String]) -> u8 {
    let result = parse_args(args).and_then(|o| run(&o));
    match result {
        Ok(report) => report.exit,
        Err(e) => {
            eprintln!("map: {e:#}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_data::bin::{BinSet, BinTable, DataRules};
    use d2_data::strings::StringTables;
    use d2_data::tables::{
        Armor, Charstats, Itemtypes, Leveldefs, Levels, Misc, Monstats, Objects, Record, Runes,
        Skills, Superuniques, Weapons,
    };
    use d2_data::txt::TxtTable;

    fn put_u8(r: &mut [u8], at: usize, v: u8) {
        r[at] = v;
    }

    fn put_u16(r: &mut [u8], at: usize, v: u16) {
        r[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn put_u32(r: &mut [u8], at: usize, v: u32) {
        r[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn put_str(r: &mut [u8], at: usize, text: &str) {
        let b = text.as_bytes();
        r[at..at + b.len()].copy_from_slice(b);
    }

    fn table<T: Record>(records: Vec<Vec<u8>>) -> BinTable {
        assert!(records.iter().all(|r| r.len() == T::SIZE));
        BinTable {
            name: T::TABLE.into(),
            source: "synthetic".into(),
            count: records.len(),
            record_size: T::SIZE,
            records: records.concat(),
        }
    }

    fn empty_table(name: &str) -> BinTable {
        BinTable {
            name: name.into(),
            source: "synthetic".into(),
            count: 0,
            record_size: 1,
            records: Vec::new(),
        }
    }

    fn txt() -> TxtTable {
        TxtTable {
            header: Vec::new(),
            records: Vec::new(),
            removed_lines: Vec::new(),
        }
    }

    struct Tiny {
        bins: BinSet,
        quests: QuestTables,
    }

    fn tiny() -> Tiny {
        let mut levels = Vec::new();
        for _ in 0..3 {
            let mut row = vec![0u8; Levels::SIZE];
            for offset in (154..=202).step_by(2) {
                put_u16(&mut row, offset, 0xFFFF);
            }
            // The `Waypoint` column is an index; 255 is "none".
            put_u8(&mut row, 228, u8::MAX);
            levels.push(row);
        }
        put_u8(&mut levels[1], 0, 1);
        put_u8(&mut levels[1], 3, 0);
        put_u8(&mut levels[1], 228, 1);
        put_str(&mut levels[1], 245, "Tiny Town");
        put_u8(&mut levels[2], 0, 2);
        put_u8(&mut levels[2], 3, 0);
        put_u16(&mut levels[2], 154, 0);
        put_str(&mut levels[2], 245, "Tiny Cave");
        let mut defs = Vec::new();
        for _ in 0..3 {
            defs.push(vec![0u8; Leveldefs::SIZE]);
        }
        put_u32(&mut defs[1], 12, 20);
        put_u32(&mut defs[1], 24, 20);
        put_u32(&mut defs[1], 72, 2);
        put_u32(&mut defs[2], 12, 30);
        put_u32(&mut defs[2], 24, 30);
        put_u32(&mut defs[2], 72, 1);

        let mut monsters = Vec::new();
        for _ in 0..3 {
            monsters.push(vec![0u8; Monstats::SIZE]);
        }
        put_u16(&mut monsters[0], 0, 148);
        put_u16(&mut monsters[0], 6, 0);
        put_str(&mut monsters[0], 16, "npc ");
        put_u8(&mut monsters[0], 13, 0x01);
        put_u8(&mut monsters[0], 15, 0x02);
        put_u16(&mut monsters[1], 0, 1);
        put_u16(&mut monsters[1], 6, 0);
        put_u8(&mut monsters[1], 12, 0x40);
        put_u8(&mut monsters[1], 15, 0x02);

        let mut supers = vec![vec![0u8; Superuniques::SIZE]];
        put_u16(&mut supers[0], 2, 0);
        put_u32(&mut supers[0], 4, 1);
        put_u32(&mut supers[0], 8, 0);

        let mut classes = vec![vec![0u8; Charstats::SIZE]];
        put_str(&mut classes[0], 32, "Amazon");

        let mut skills = vec![vec![0u8; Skills::SIZE]];
        put_u16(&mut skills[0], 0, 0);
        put_u8(&mut skills[0], 12, 0);
        put_u16(&mut skills[0], 300, 20);
        put_u16(&mut skills[0], 372, 1);
        put_u16(&mut skills[0], 394, 5);

        let mut types = Vec::new();
        for code in ["swor", "misc"] {
            let mut r = vec![0u8; Itemtypes::SIZE];
            put_str(&mut r, 0, code);
            types.push(r);
        }

        let mut weapons = vec![vec![0u8; Weapons::SIZE]];
        put_str(&mut weapons[0], 128, "ssd ");
        put_u16(&mut weapons[0], 286, 0);
        put_u8(&mut weapons[0], 312, 2);
        put_u8(&mut weapons[0], 319, 1);

        let mut miscs = vec![vec![0u8; Misc::SIZE]];
        put_str(&mut miscs[0], 128, "hp1 ");
        put_u16(&mut miscs[0], 286, 1);

        let mut cube = [vec![0u8; 328]];
        put_u8(&mut cube[0], 0, 1);
        put_u8(&mut cube[0], 16, 1);
        put_u16(&mut cube[0], 20, 0x0002);
        put_u16(&mut cube[0], 22, 0);
        put_u16(&mut cube[0], 78, 1);
        put_u8(&mut cube[0], 84, 0xFC);

        let mut runes = vec![vec![0u8; Runes::SIZE]];
        put_str(&mut runes[0], 0, "Tiny Word");
        put_u8(&mut runes[0], 128, 1);
        put_u32(&mut runes[0], 152, 0);
        put_u32(&mut runes[0], 156, 1);

        let mut objects = vec![vec![0u8; Objects::SIZE]];
        put_str(&mut objects[0], 0, "Waypoint");
        put_str(&mut objects[0], 192, "wp");
        put_u8(&mut objects[0], 435, 2);

        let mut bins = BinSet {
            lod: true,
            rules: DataRules::Original,
            strings: StringTables::default(),
            tables: vec![
                table::<Levels>(levels),
                table::<Leveldefs>(defs),
                table::<Monstats>(monsters),
                table::<Superuniques>(supers),
                table::<Charstats>(classes),
                table::<Skills>(skills),
                table::<Itemtypes>(types),
                table::<Weapons>(weapons),
                table::<Armor>(Vec::new()),
                table::<Misc>(miscs),
                BinTable {
                    name: "cubemain".into(),
                    source: "synthetic".into(),
                    count: cube.len(),
                    record_size: 328,
                    records: cube.concat(),
                },
                table::<Runes>(runes),
                table::<Objects>(objects),
            ],
            code: Default::default(),
            code_reports: Default::default(),
            hitclass: empty_table("hitclass"),
            sounds: txt(),
            soundenviron: txt(),
        };
        bins.tables.sort_by(|a, b| a.name.cmp(&b.name));

        let quests = QuestTables::parse(
            "index\tchain\tflag\tact\tversion\tno_set_state\tinit\tfilter\tflag2\tinit_no\tseq_id\tcallbacks\tstatus_fn\tactive_fn\tseq_fn\tmsgs\tname\tspec\n\
             1\t1\t1\t0\t100\t0\t0x1\t1\t-\t4\t2\t0:1\t0\t0\t0\t0x10\tA1Q1 Tiny Quest\tspecified\n",
            "table\tentries\tchain\tstate\tslot\tnpc\tstring\tmenu\n\
             0x10\t1\t1\t0\t0\t148\t100\t0\n",
        )
        .unwrap();
        Tiny { bins, quests }
    }

    fn map(tiny: &Tiny) -> model::GameMap {
        reader::build(&reader::Inputs {
            bins: &tiny.bins,
            quests: &tiny.quests,
            overrides: &overrides::parse(
                "towns = [1]\nfinal_objective = \"Win\"\n[[boss]]\narea = 2\nsuperunique = 0\n",
            )
            .unwrap(),
            source: "synthetic".into(),
            mod_name: None,
            overrides_path: None,
        })
        .unwrap()
    }

    #[test]
    fn a_tiny_game_reads_and_passes_t_static() {
        let tiny = tiny();
        let map = map(&tiny);
        assert_eq!(map.summary.areas, 2);
        assert_eq!(map.summary.links, 1);
        assert_eq!(map.summary.waypoints, 1);
        assert_eq!(map.summary.towns, 1);
        assert_eq!(map.summary.quests, 1);
        assert_eq!(map.summary.boss_kills, 1);
        assert_eq!(map.summary.classes, 1);
        assert_eq!(map.summary.items, 2);
        assert_eq!(map.summary.cube_recipes, 1);
        assert_eq!(map.summary.runewords, 1);
        assert_eq!(
            map.playables.cube_recipes[0].inputs[0].code.as_deref(),
            Some("ssd")
        );
        assert_eq!(
            map.playables.cube_recipes[0].outputs[0].code.as_deref(),
            Some("hp1")
        );
        assert_eq!(map.objectives.quests[0].steps[0].npcs, vec![148]);
        let report = static_checks::run(&map);
        for rule in &report.rules {
            assert!(
                rule.findings.is_empty(),
                "{}: {:?}",
                rule.rule,
                rule.findings
            );
        }
        assert_eq!(report.exit, 0);
        assert!(game_md::write(&map, &report).contains("Tiny Town"));
    }

    #[test]
    fn a_levels_unique_monster_columns_are_not_bosses() {
        // Tiny Cave's `umon1` is 0, the hcIdx of superunique row 0; only the
        // overrides' `[[boss]]` list makes it a boss.
        let tiny = tiny();
        let map = reader::build(&reader::Inputs {
            bins: &tiny.bins,
            quests: &tiny.quests,
            overrides: &overrides::parse("towns = [1]\n").unwrap(),
            source: "synthetic".into(),
            mod_name: None,
            overrides_path: None,
        })
        .unwrap();
        assert!(map.objectives.boss_kills.is_empty());
        let named = self::map(&tiny);
        let boss = &named.objectives.boss_kills[0];
        assert_eq!((boss.area, boss.class, boss.superunique), (2, 1, Some(0)));
    }

    #[test]
    fn a_boss_names_one_superunique_row_or_one_monster() {
        assert!(overrides::parse("[[boss]]\narea = 2\n").is_err());
        assert!(overrides::parse("[[boss]]\narea = 2\nsuperunique = 0\nmonster = 1\n").is_err());
        let o = overrides::parse("[[boss]]\narea = 2\nmonster = 1\npending = \"never spawns\"\n")
            .unwrap();
        assert_eq!(o.bosses[0].monster, Some(1));
        assert_eq!(o.bosses[0].pending.as_deref(), Some("never spawns"));
        let tiny = tiny();
        let bad = reader::build(&reader::Inputs {
            bins: &tiny.bins,
            quests: &tiny.quests,
            overrides: &overrides::parse("[[boss]]\narea = 2\nsuperunique = 5\n").unwrap(),
            source: "synthetic".into(),
            mod_name: None,
            overrides_path: None,
        });
        assert!(bad.unwrap_err().contains("no superuniques row 5"));
    }

    #[test]
    fn every_rule_can_fire() {
        let tiny = tiny();
        let mut map = map(&tiny);
        map.world.links.clear();
        map.playables.classes[0].skills.clear();
        map.playables.cube_recipes[0].inputs[0].code = None;
        map.playables.runewords[0].runes.clear();
        map.objectives.quests[0].steps.clear();
        map.objectives.quests[0].rewards.push(model::Reward {
            kind: "item".into(),
            name: "missing".into(),
            item: Some("nope".into()),
        });
        map.world.waypoints.push(model::Waypoint {
            level: 99,
            act: 1,
            town: false,
            name: "missing".into(),
        });
        let report = static_checks::run(&map);
        assert_eq!(report.exit, 1);
        for rule in [
            "area-reachable",
            "quest-step",
            "reward-exists",
            "item-references",
            "class-skills",
            "world-references",
        ] {
            assert!(
                static_checks::first_failure(&report, rule).is_some(),
                "{rule} did not fire"
            );
        }
    }

    #[test]
    fn overrides_parse_towns_goals_and_rewards() {
        let o = overrides::parse(
            "game = \"x\"\ntowns = [1, 2]\n[[reward]]\nchain = 1\nkind = \"item\"\nname = \"r\"\nitem = \"rin \"\n[[goal]]\nname = \"g\"\nkind = \"talk\"\narea = 1\nnpc = 148\nchain = 1\n",
        )
        .unwrap();
        assert_eq!(o.towns, vec![1, 2]);
        assert_eq!(o.rewards[0].item.as_deref(), Some("rin "));
        assert_eq!(o.goals[0].npc, Some(148));
    }

    #[test]
    fn the_base_overrides_parse() {
        let o = overrides::parse(overrides::BASE).unwrap();
        assert_eq!(o.towns.len(), 5);
        assert_eq!(o.transitions.len(), 4);
        assert_eq!(o.paths.len(), 5);
        assert_eq!(o.links.len(), 14);
        assert!(o.unreachable.is_empty());
        assert_eq!(o.goals.len(), 7);
        assert_eq!(o.bosses.len(), 62);
    }

    #[test]
    #[ignore = "needs the base game's files in D2_GAME_DIR"]
    fn the_base_game_builds_and_passes_t_static() {
        let o = Options {
            game_dir: std::env::var_os("D2_GAME_DIR").map(PathBuf::from),
            out: std::env::temp_dir().join("d2rust-game-map-test"),
            ..Options::default()
        };
        if o.game_dir.is_none() {
            return;
        }
        let report = run(&o).unwrap();
        let map: model::GameMap =
            serde_json::from_slice(&std::fs::read(o.out.join("game-map.json")).unwrap()).unwrap();
        assert!(map.summary.areas > 100);
        assert!(map.summary.quests > 20);
        assert!(map.summary.classes >= 7);
        eprintln!(
            "base game map: {} areas, {} quests, {} items; static exit {}",
            map.summary.areas, map.summary.quests, map.summary.items, report.exit
        );
        for rule in &report.rules {
            for finding in &rule.findings {
                eprintln!("{}: {}: {}", rule.rule, finding.id, finding.detail);
            }
        }
    }
}
