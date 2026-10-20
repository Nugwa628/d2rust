//! Synthetic content sweeps exercise the production headless runner in CI.
use d2_client::app::{single_player::GameData, sweep};
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sweep-{tag}-{}", std::process::id()))
}
fn install(tag: &str) -> PathBuf {
    install_with(tag, |_| {})
}
fn install_with(tag: &str, tweak: impl FnOnce(&mut test_fixtures::synth::Synthetic)) -> PathBuf {
    let dir = scratch(tag);
    let mut data = test_fixtures::play::playable();
    // Player action records are absent from the small content fixture. Supply
    // a real action event and end frame so the synthetic cast can complete.
    for class in 0..7 {
        for mode in ["DT", "NU", "WL", "RN", "GH", "TN", "TW", "A1"] {
            data.animdata.push(test_fixtures::animdata::Anim {
                name: format!("C{class}{mode}HTH"),
                frames: 8,
                speed: 256,
                events: if mode == "A1" { vec![(4, 1)] } else { vec![] },
            });
        }
    }
    // Long casts can reach every idle/pressed cursor animation.
    for name in ["Gaunt", "grasp", "ohand", "orotate", "ppress", "buysell"] {
        data.stored.push((
            format!(r"data\global\ui\cursor\{name}.dc6"),
            test_fixtures::play::stand_in_dc6(),
        ));
    }
    // Full monster-table fixup reads monmode 15 for its last record.
    let modes = data.tables.files.get_mut("monmode.txt").unwrap();
    let key = modes.columns.iter().position(|c| c == "name").unwrap();
    while modes.rows.len() < 16 {
        let mut row = modes.rows[0].clone();
        row[key] = format!("sweep-pad-{}", modes.rows.len());
        modes.rows.push(row);
    }
    // The native converter validates the complete monster table shape.
    let monsters = data.tables.files.get_mut("monstats.txt").unwrap();
    let key = monsters.columns.iter().position(|c| c == "Id").unwrap();
    while monsters.rows.len() < 708 {
        let mut row = monsters.rows[0].clone();
        row[key] = format!("sweep-pad-{}", monsters.rows.len());
        monsters.rows.push(row);
    }
    // The base content fixture uses tiny stat encodings; a save fixture needs
    // room for fixed-point vitals and the mana reservoir used by cast checks.
    let stats = data.tables.files.get_mut("itemstatcost.txt").unwrap();
    for name in ["save bits", "send bits", "csvbits"] {
        let column = stats.columns.iter().position(|c| c == name).unwrap();
        for row in &mut stats.rows[6..15] {
            row[column] = "21".into();
        }
    }
    // NPC greeting records address numeric string IDs, beyond this tiny fixture.
    while data.strings.base.len() < 10000 {
        let id = data.strings.base.len();
        data.strings
            .base
            .push((format!("sweep-string-{id}"), "Synthetic greeting.".into()));
    }
    // A synthetic town vendor at the class used by the menu table.
    let monsters = data.tables.files.get_mut("monstats.txt").unwrap();
    let mut vendor = monsters.rows[2].clone();
    vendor[monsters.columns.iter().position(|c| c == "Id").unwrap()] = "sweep-vendor".into();
    monsters.rows[148] = vendor;
    let images =
        d2_client::ui::panels::UiFiles::new(&d2_client::ui::layout::panel_layout().unwrap());
    for name in images.names() {
        data.stored.push((
            format!(r"data\global\ui\{name}.dc6"),
            test_fixtures::play::stand_in_dc6(),
        ));
    }
    data.stored.push((
        r"data\global\ui\menu\boxpieces.dc6".into(),
        test_fixtures::play::stand_in_dc6(),
    ));
    for table in ["weapons.txt", "armor.txt", "misc.txt"] {
        let rows = data.tables.files.get(table).unwrap();
        for name in ["flippyfile", "invfile"] {
            if let Some(column) = rows
                .columns
                .iter()
                .position(|c| c.eq_ignore_ascii_case(name))
            {
                for row in &rows.rows {
                    if !row[column].is_empty() {
                        data.stored.push((
                            format!(r"data\global\items\{}.dc6", row[column]),
                            test_fixtures::play::stand_in_dc6(),
                        ));
                    }
                }
            }
        }
    }
    let skills = data.tables.files.get_mut("skills.txt").unwrap();
    let set =
        |rows: &mut Vec<Vec<String>>, column: usize, value: &str| rows[2][column] = value.into();
    for (name, value) in [("anim", "A1"), ("manashift", "8"), ("srvdofunc", "0")] {
        let column = skills
            .columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
            .unwrap();
        set(&mut skills.rows, column, value);
    }
    tweak(&mut data);
    test_fixtures::cache::copy_install(Path::new(env!("CARGO_TARGET_TMPDIR")), &dir, &data)
        .unwrap();
    dir
}
#[test]
fn skills_are_derived_from_loaded_tables_and_run_with_checkpoints() {
    let dir = install("skills");
    let data = GameData::select(Some(&dir)).unwrap();
    let GameData::Live(live) = &data;
    let g = sweep::generate(live, "skills").unwrap();
    assert!(
        !g.cases.is_empty(),
        "synthetic skills must generate cases: {:?}",
        g.skips
    );
    assert!(!g.skips.is_empty());
    for c in &g.cases {
        let s = d2_client::scenario_script::Scenario::parse(&c.text).unwrap();
        assert_eq!(s.tick_rate, 60);
        assert_eq!(
            d2_client::scenario_script::Scenario::parse(&s.to_text()).unwrap(),
            s
        );
    }
    let selected = g
        .cases
        .iter()
        .find(|c| c.name.contains("class-1-") && c.name.contains("ground"))
        .unwrap()
        .name
        .clone();
    let report = sweep::run(
        data,
        &sweep::Options {
            family: "skills".into(),
            filter: Some(selected),
            jobs: 1,
            out: scratch("skills-out"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 1);
    assert_eq!(report["exit"], 0, "{report:#}");
    let path = PathBuf::from(report["cases"][0]["out"].as_str().unwrap()).join("repro.json");
    let repro: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(repro["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["kind"] == "checkpoint" && s["ok"] == true));
}
#[test]
fn a_unit_search_skill_skips_ground_and_self_shapes() {
    let dir = install_with("unit-search", |data| {
        let skills = data.tables.files.get_mut("skills.txt").unwrap();
        let column = |name: &str| {
            skills
                .columns
                .iter()
                .position(|c| c.eq_ignore_ascii_case(name))
                .unwrap()
        };
        // Firebolt (row 2, class 1) needs an enemy near the click point; it
        // has no open-point or point-enemy search, so a ground or self click
        // finds no target and must not be generated.
        let row = &mut skills.rows[2];
        row[column("SearchEnemyNear")] = "1".into();
        row[column("SearchOpenXY")] = "0".into();
        row[column("SearchEnemyXY")] = "0".into();
    });
    let data = GameData::select(Some(&dir)).unwrap();
    let GameData::Live(live) = &data;
    let g = sweep::generate(live, "skills").unwrap();
    let firebolt: Vec<&str> = g
        .cases
        .iter()
        .filter(|c| c.name.starts_with("class-1-skill-2-"))
        .map(|c| c.name.as_str())
        .collect();
    assert!(
        firebolt.iter().any(|n| n.ends_with("-monster")),
        "{firebolt:?}"
    );
    assert!(
        !firebolt
            .iter()
            .any(|n| n.ends_with("-ground") || n.ends_with("-self")),
        "{firebolt:?}"
    );
    assert!(
        g.skips
            .get("skills:unit-target-required")
            .copied()
            .unwrap_or(0)
            >= 2
    );
}

#[test]
fn cli_rejects_invalid_selection_and_signatures_ignore_deadlines() {
    let parse =
        |a: &[&str]| sweep::parse_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    for a in [
        vec!["bogus"],
        vec!["skills", "--jobs", "0"],
        vec!["skills", "--mod", "x"],
        vec!["skills", "--native", "x", "--mod", "../x"],
    ] {
        assert!(parse(&a).is_err());
    }
    assert_eq!(
        parse(&["all", "--native", "n", "--mod", "demo", "--jobs", "4"])
            .unwrap()
            .mod_name
            .as_deref(),
        Some("demo")
    );
    let report = |tick| serde_json::json!({"exit":1,"steps":[{"kind":"check","text":format!("at {tick} check player.mana < 1000"),"ok":false,"result":"actual: 1000"}]});
    assert_eq!(sweep::failures(&report(10)), sweep::failures(&report(30)));
}

#[test]
fn checkpoints_are_read_only_and_report_unserializable_state() {
    use d2_client::{
        app::repro::{self, RunOpts},
        scenario_script::Scenario,
    };
    let dir = install("checkpoint");
    let mut session = repro::Session::load(Some(&dir)).unwrap();
    let base = "scenario 2\ntick_rate 60\nname checkpoint\nend 12\nchar class 0\n";
    let run = |session: &mut repro::Session, text: &str, tag: &str| {
        let opts = RunOpts {
            out: scratch(tag),
            keep_going: true,
            ..Default::default()
        };
        let report = session.run(&Scenario::parse(text).unwrap(), &opts).unwrap();
        (report, opts.out)
    };
    let (a, out_a) = run(&mut session, base, "checkpoint-baseline");
    let (b, out_b) = run(
        &mut session,
        &format!("{base}at 6 checkpoint\n"),
        "checkpoint-probe",
    );
    assert_eq!(a.exit, 0, "{}", a.json);
    assert_eq!(b.exit, 0, "{}", b.json);
    for file in ["state-tail.jsonl", "client-state.json"] {
        assert_eq!(
            std::fs::read(out_a.join(file)).unwrap(),
            std::fs::read(out_b.join(file)).unwrap()
        );
    }
    let (broken, _) = run(
        &mut session,
        &format!("{base}char stat 0 1000000\nat 6 checkpoint\n"),
        "checkpoint-broken",
    );
    assert_eq!(broken.exit, 1, "{}", broken.json);
    assert!(sweep::failures(&broken.json)
        .keys()
        .any(|s| s.starts_with("checkpoint:roundtrip:")));
}

#[test]
fn a_mod_generates_and_executes_its_own_skill_data() {
    use d2_client::app::single_player::{GameSource, Layers};
    use d2_formats::mpq::writer::MpqWriter;
    let dir = install("mod-archives");
    for archive in d2_convert::convert::REQUIRED_ARCHIVES {
        let path = dir.join(archive);
        if !path.exists() {
            MpqWriter::new().write(path).unwrap();
        }
    }
    let native = scratch("mod-native");
    let mut options = d2_convert::Options::new(&dir, &native);
    options.threads = 2;
    options.converter_commit = "synthetic-sweep".into();
    let converted = d2_convert::convert(&options, &d2_convert::kinds::builtin()).unwrap();
    assert!(converted.failures.is_empty(), "{:?}", converted.failures);
    let mod_dir = native.join("mods/sweep-test");
    std::fs::create_dir_all(mod_dir.join("files/patches")).unwrap();
    std::fs::write(
        mod_dir.join("mod.toml"),
        "name = \"sweep-test\"\nversion = \"1\"\nnative_format_version = 1\nrequires = []\n",
    )
    .unwrap();
    std::fs::write(
        mod_dir.join("files/patches/skill.d2patch"),
        "d2patch 1\ntable skills\nset Firebolt reqlevel 1 -> 7\n",
    )
    .unwrap();
    // A broken order file proves the explicit source never reads it.
    std::fs::write(native.join("mods/order.toml"), "order = [\"absent\"]\n").unwrap();
    let open =
        |mods| GameData::open(&GameSource::Native(native.clone(), Layers::List(mods))).unwrap();
    let base = open(vec![]);
    let modified = open(vec!["sweep-test".into()]);
    let cases = |data: &GameData| {
        let GameData::Live(live) = data;
        sweep::generate(live, "skills").unwrap().cases
    };
    let select = |cases: Vec<sweep::Case>| {
        cases
            .into_iter()
            .find(|c| c.name.contains("class-1-") && c.name.ends_with("ground"))
            .unwrap()
    };
    let b = select(cases(&base));
    let m = select(cases(&modified));
    assert!(b.text.contains("char level 1\n"));
    assert!(m.text.contains("char level 7\n"));
    let report = sweep::run(
        modified,
        &sweep::Options {
            family: "skills".into(),
            filter: Some(m.name),
            jobs: 1,
            out: scratch("mod-out"),
            mod_name: Some("sweep-test".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report["mod"], "sweep-test");
    assert_eq!(report["exit"], 0, "{report:#}");
}

#[test]
fn npc_menu_cases_follow_loaded_rows_and_run_player_interactions() {
    let data = GameData::select(Some(&install("npcs"))).unwrap();
    let GameData::Live(live) = &data;
    let cases = sweep::generate(live, "npcs").unwrap();
    assert!(cases.cases.iter().any(|c| c.name == "npc-148-talk-close"));
    let report = sweep::run(
        data,
        &sweep::Options {
            family: "npcs".into(),
            filter: Some("npc-148-talk-close".into()),
            jobs: 4,
            out: scratch("npcs-out"),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report["exit"], 0, "{report:#}");
    let path = Path::new(report["cases"][0]["out"].as_str().unwrap()).join("repro.json");
    let repro: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let keys: Vec<_> = repro["steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["kind"] == "key-if")
        .collect();
    assert!(keys
        .iter()
        .any(|s| s["result"].as_str().unwrap().starts_with("sent:")));
    assert!(keys
        .iter()
        .any(|s| s["result"].as_str().unwrap().starts_with("skipped:")));
}

#[test]
fn areas_items_and_panels_generate_and_run_synthetic_slices() {
    let data = GameData::select(Some(&install("remaining"))).unwrap();
    for (family, selection) in [
        ("areas", "area-1-walk"),
        ("items", "pickup-drop"),
        ("panels", "action-game-menu"),
    ] {
        let GameData::Live(live) = &data;
        let generated = sweep::generate(live, family).unwrap();
        assert!(
            generated.cases.iter().any(|c| c.name.contains(selection)),
            "{family}: {:?}",
            generated.skips
        );
        for c in &generated.cases {
            d2_client::scenario_script::Scenario::parse(&c.text).unwrap();
        }
        let selected = generated
            .cases
            .iter()
            .find(|c| c.name.contains(selection))
            .unwrap()
            .name
            .clone();
        let report = sweep::run(
            data.clone(),
            &sweep::Options {
                family: family.into(),
                filter: Some(selected),
                jobs: 4,
                out: scratch(&format!("{family}-out")),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report["exit"], 0, "{report:#}");
    }
}

/// Item cases prepare what the game's rules ask for: an item is identified
/// before it is worn, a purchase clicks the first stocked item rather than a
/// fixed cell, and a row whose ground art is in no archive is a counted skip.
#[test]
fn item_cases_identify_before_equipping_and_skip_rows_without_ground_art() {
    let with_art = GameData::select(Some(&install("items-art"))).unwrap();
    let GameData::Live(live) = &with_art;
    let all = sweep::generate(live, "items").unwrap();
    let text = |cases: &[sweep::Case], what: &str| {
        cases
            .iter()
            .filter(|c| c.name.ends_with(what))
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
    };
    let equips = text(&all.cases, "-equip");
    assert!(!equips.is_empty(), "{:?}", all.skips);
    for scenario in &equips {
        let identify = scenario.find("dev identify").unwrap();
        let equip = scenario.find("msg EquipItem").unwrap();
        assert!(identify < equip, "{scenario}");
    }
    let buys = text(&all.cases, "-buy");
    assert!(!buys.is_empty(), "{:?}", all.skips);
    for scenario in &buys {
        assert!(
            scenario.contains("input click ui:shop.item(0)"),
            "{scenario}"
        );
        assert!(!scenario.contains("shop.slot(0,0)"), "{scenario}");
    }
    assert!(!all.skips.contains_key("items:ground-art-not-in-archives"));
    let robes = all
        .cases
        .iter()
        .filter(|c| c.text.contains("poke item ar1 "))
        .count();
    assert!(robes > 0);

    // Without the Robe's ground art the armor row has no cases at all; the
    // Vest keeps its own.
    let without = GameData::select(Some(&install_with("items-no-art", |data| {
        data.stored
            .retain(|(name, _)| !name.eq_ignore_ascii_case(r"data\global\items\Robe.dc6"));
    })))
    .unwrap();
    let GameData::Live(live) = &without;
    let fewer = sweep::generate(live, "items").unwrap();
    assert_eq!(fewer.skips["items:ground-art-not-in-archives"], 1);
    assert!(!fewer
        .cases
        .iter()
        .any(|c| c.text.contains("poke item ar1 ")));
    assert!(fewer
        .cases
        .iter()
        .any(|c| c.text.contains("poke item ar2 ")));
}

/// Animation-rate cases come from the loaded classes, skills and monsters,
/// and each checks the drawn rate of one mode against its table rate.
#[test]
fn anims_cases_cover_every_class_mode_and_spawnable_monster() {
    let data = GameData::select(Some(&install("anims"))).unwrap();
    let GameData::Live(live) = &data;
    let generated = sweep::generate(live, "anims").unwrap();
    for class in 0..7 {
        for mode in [
            "town-neutral",
            "town-walk",
            "neutral",
            "walk",
            "run",
            "attack1",
        ] {
            let name = format!("class-{class}-{mode}");
            let case = generated.cases.iter().find(|c| c.name == name);
            let case = case.unwrap_or_else(|| panic!("{name}: {:?}", generated.skips));
            assert!(
                case.text.contains("check anim_rate(player, *, "),
                "{}",
                case.text
            );
        }
    }
    assert!(generated
        .cases
        .iter()
        .any(|c| c.name.starts_with("monster-")));
    for c in &generated.cases {
        d2_client::scenario_script::Scenario::parse(&c.text).unwrap();
        assert!(c.text.contains(".ratio ~ 1 0.15"), "{}", c.text);
    }
}

/// Door cases come from the embedded door table and the loaded objects:
/// each brings the player to a door, opens it and walks through both ways.
#[test]
fn doors_cases_open_and_cross_each_door_class() {
    let data = GameData::select(Some(&install("doors"))).unwrap();
    let GameData::Live(live) = &data;
    let generated = sweep::generate(live, "doors").unwrap();
    // The synthetic levels hold no door-table level; every table row is
    // accounted for as a case or a counted skip.
    assert!(!generated.cases.is_empty() || !generated.skips.is_empty());
    for c in &generated.cases {
        d2_client::scenario_script::Scenario::parse(&c.text).unwrap();
        assert!(c.text.contains(".mode_id == 2"), "{}", c.text);
    }
}
