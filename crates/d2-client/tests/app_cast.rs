//! A right-hand skill cast on the play host, headless, on the user's
//! install (`real_rig`; q-fixture-migrate): C→S 0x0C (the right skill at
//! a point) → the server's skill use → mana → the missile in the
//! server's store → the cast mode at the client. The skill is the
//! install's Fire Bolt, found by name in `skills.txt`; the mana it costs
//! is its own row's (Fire Bolt L1).

use crate::app_support;
use crate::real_rig;

use d2_data::bin::TableFiles;
use d2_sim::units::UnitType;
use real_rig::{skill_row, Rig};

/// The install's skill id of `name` (`skills.txt`).
fn skill(name: &str) -> usize {
    let d = app_support::live();
    let (_, bytes) = d
        .archives
        .read_excel("skills.txt")
        .expect("skills.txt reads")
        .expect("the install has skills.txt");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().expect("header").split('\t').collect();
    let col = |n: &str| {
        head.iter()
            .position(|h| *h == n)
            .unwrap_or_else(|| panic!("{n}"))
    };
    let (c_name, c_id) = (col("skill"), col("Id"));
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        if f.get(c_name) == Some(&name) {
            return f[c_id].parse().expect("Id");
        }
    }
    panic!("no skills.txt row called {name}");
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn a_right_skill_at_a_point_costs_mana_and_creates_the_missile() {
    let fire_bolt = skill("Fire Bolt");
    let row = skill_row(fire_bolt);
    let mut r = Rig::new("sorceress", &[fire_bolt]);
    r.leave_town();
    r.strengthen();
    r.select_right(fire_bolt);
    // Regeneration at rest, from half mana: the cast's fall is the cost
    // less what a frame regenerates.
    r.with(|sim, p| {
        sim.events
            .action
            .with(&mut sim.game, |_, v| v.set_base(p, 8, 50 << 8));
    });
    let (mut regen, mut prev) = (0, r.mana());
    for _ in 0..10 {
        r.step(1);
        let m = r.mana();
        regen = regen.max(m - prev);
        prev = m;
    }
    let (px, py) = r.pos();
    let mut msg = vec![0x0C];
    msg.extend(((px + 6) as u16).to_le_bytes());
    msg.extend((py as u16).to_le_bytes());
    let mark = r.s2c_mark();
    let fate = r.send_for_fate(&msg);
    assert!(fate.contains("Dispatched(Done)"), "{fate}");
    let (mut most, mut drop, mut modes) = (0, 0, Vec::new());
    for _ in 0..60 {
        r.step(1);
        most = most.max(r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Missile).len()));
        let m = r.mana();
        drop = drop.max(prev - m);
        prev = m;
        modes.push(r.mode());
    }
    let errors = r.errors();
    // Fire Bolt L1 pays `mana << manashift`.
    let cost = i32::from(row.mana) << row.manashift;
    assert!(
        (cost - regen..=cost).contains(&drop),
        "mana spent: drop {drop}, cost {cost}, regen {regen}; errors: {errors}"
    );
    assert!(most > 0, "the server made the missile; errors: {errors}");
    assert!(r.s2c_contains_since(mark, 0x4D), "{errors}");
    // The cast mode is the server player's. The client's own player
    // enters it at its click, not from a server message: the game's
    // server sends its own client nothing, and this rig sends the 0x0C
    // bytes without a click.
    assert!(
        modes.contains(&u32::from(row.anim)),
        "the server player's cast mode: {modes:?}; {errors}"
    );
}

/// Missile casts keep the server caster at its starting position through
/// the skill start and the full cast animation, for every player class.
#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn missile_skills_cast_on_a_distant_monster_in_place() {
    print_unit_cast_changes();
    // Cover every player class with a missile skill, including both
    // channeled skills whose click dispatch changes in the live table.
    for (class, name, weapon) in [
        ("amazon", "Fire Arrow", Some(*b"sbw ")),
        ("sorceress", "Fire Ball", None),
        ("sorceress", "Inferno", None),
        ("necromancer", "Teeth", None),
        ("paladin", "Holy Bolt", None),
        ("barbarian", "Double Throw", Some(*b"tax ")),
        ("druid", "Firestorm", None),
        ("druid", "Arctic Blast", None),
        ("assassin", "Fire Trauma", None),
    ] {
        let skill = real_rig::skill_named(name);
        let row = skill_row(skill);
        let mut r = Rig::new(class, &[skill]);
        r.leave_town();
        r.strengthen();
        if let Some(weapon) = weapon {
            // Meet the throwing axes' Dexterity requirement so the
            // barbarian exercises a usable missile skill.
            r.with(|sim, p| {
                sim.events.action.with(&mut sim.game, |_, v| {
                    v.set_base(p, d2_sim::stats::stat::DEXTERITY, 100);
                });
            });
            for hand in [4, 5] {
                if r.worn(hand).is_some() {
                    r.take_off(hand);
                }
            }
            r.wear(weapon, 4);
            r.wear(if class == "amazon" { *b"aqv " } else { weapon }, 5);
        }
        r.select_right(skill);
        let target = r.spawn_monster(10);
        let before = r.pos();
        r.right_click_unit(target);
        // Inferno and Arctic Blast spawn their missile on the held do, not
        // on the skill start: the first click opens the channel and the
        // held repeats send the do. A player holds the button for these.
        let channel = row.srvdofunc == 19;
        let mut modes = vec![r.mode()];
        let mut most_missiles = 0;
        for frame in 0..60 {
            if channel && frame < 20 {
                r.right_click_unit_held(target);
            }
            r.step(1);
            modes.push(r.mode());
            most_missiles = most_missiles
                .max(r.with(|sim, _| sim.game.lists.units_of_type(UnitType::Missile).len()));
            assert_eq!(r.pos(), before, "{class} {name}, frame {frame}");
        }
        assert!(
            modes.contains(&u32::from(row.anim)),
            "{class} {name} must actually cast: {modes:?}; {}",
            r.errors()
        );
        assert_eq!(r.used_skill(), Some(skill as i32), "{class} {name}");
        assert!(most_missiles > 0, "{class} {name} must emit a missile");
    }
}

/// Reproducible audit of the live table rows affected by the unit-cast
/// policy. Print only the decision fields, never an extracted game table.
fn print_unit_cast_changes() {
    let skills = real_rig::ExcelTable::load("skills.txt");
    let mut changed = Vec::new();
    for row in &skills.rows {
        let Ok(id) = skills.cell(row, "Id").parse::<usize>() else {
            continue;
        };
        let class = skills.cell(row, "charclass");
        let player = !class.is_empty() || id <= 5 || (217..=220).contains(&id);
        let range = skills.cell(row, "range");
        let dofunc = skills.cell(row, "srvdofunc");
        if player
            && skills.cell(row, "InGame") == "1"
            && skills.cell(row, "passive") != "1"
            && (range == "loc" || (dofunc == "19" && range != "h2h"))
        {
            changed.push(id);
            let compiled = skill_row(id);
            assert_eq!(compiled.srvdofunc, 19);
            assert_eq!(compiled.range, if range == "rng" { 2 } else { 0 });
            println!(
                "CLICK_CHANGE|{id}|{}|{class}|{range}|{dofunc}|targetitem={}",
                skills.cell(row, "skill"),
                skills.cell(row, "TargetItem")
            );
        }
    }
    assert_eq!(changed, [41, 230], "review the affected player-skill list");
    let missiles = real_rig::ExcelTable::load("missiles.txt");
    for name in ["Inferno", "Arctic Blast"] {
        let row = skills.row(name);
        let missile = skills.cell(row, "srvmissilea");
        let m = missiles.row(missile);
        let compiled = skill_row(real_rig::skill_named(name));
        assert_eq!((compiled.srvstfunc, compiled.srvdofunc), (11, 19));
        assert!(missiles.cell(m, "Range").parse::<i32>().unwrap() > 0);
        assert!(missiles.cell(m, "Vel").parse::<i32>().unwrap() > 0);
        println!(
            "CHANNEL|{name}|range={}|srvdofunc={}|missile={missile}|calc1={}|Param1={}|Param2={}",
            skills.cell(row, "range"),
            skills.cell(row, "srvdofunc"),
            skills.cell(row, "calc1"),
            skills.cell(row, "Param1"),
            skills.cell(row, "Param2")
        );
        for column in ["Range", "LevRange", "Vel", "MaxVel", "Param1", "Param2"] {
            println!(
                "CHANNEL_MISSILE|{name}|{column}={}",
                missiles.cell(m, column)
            );
        }
    }
}
