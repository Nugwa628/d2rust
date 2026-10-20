//! T-jump: every goal is a generated scenario. The sim is loaded once per
//! worker and each goal gets a fresh game, so the data load is not paid per
//! case. Boss and area goals are self-contained; the talk goals chain from
//! the previous talk goal's checkpoint, so a later conversation sees the
//! earlier quest state. A blocked goal leaves a standalone `.scn` behind.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::model::{GameMap, Skill};
use super::overrides::{Goal as OverrideGoal, Overrides};
use crate::app::repro::checkpoint::Checkpoint;
use crate::app::repro::{self, RunOpts};
use crate::scenario_script::Scenario;

/// The report's format word.
pub const FORMAT: &str = "game-map-t-jump 1";
/// The level a generated goal character starts in (the inline character's
/// default `char area`).
const START_TOWN: u32 = 1;
/// Ticks a boss goal's setup `poke goto` is given before its next step.
const SETUP_GOTO_TICKS: u32 = 240;
/// Ticks between other setup steps.
const SETUP_STEP_TICKS: u32 = 20;
/// Ticks from the walk to the boss to the goal's end, unless the boss
/// names its own.
const FIGHT_TICKS: u32 = 1470;
/// Ticks after the setup for the released boss to appear.
const SETUP_SPAWN_TICKS: u32 = 90;

/// One generated goal.
#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub name: String,
    pub kind: String,
    pub area: Option<u32>,
    pub class: Option<u32>,
    pub chain: Option<u32>,
    pub max_ticks: u32,
    pub text: String,
    /// Why the goal is expected to block: a pending goal that blocks is
    /// reported but does not fail the run.
    pub pending: Option<String>,
}

/// T-jump options.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub jobs: usize,
    pub filter: Option<String>,
    pub limit: Option<usize>,
    pub difficulty: String,
    pub out: PathBuf,
    pub game_dir: Option<PathBuf>,
}

fn header(
    name: &str,
    difficulty: &str,
    end: u32,
    class: u32,
    level: u32,
    skill: Option<u32>,
) -> String {
    // The generated character carries no items and the runner cannot build
    // `char item`; a class whose skill needs a weapon or ammunition can
    // never land a hit with it. It is a caster with a raised mana pool
    // instead, so every boss can actually be fought.
    let mut text = format!(
        "scenario 2\ntick_rate 60\nname {name}\nseed 7\ndifficulty {difficulty}\nend {end}\nchar class {class}\nchar level {level}\nchar stat 6 640000\nchar stat 7 640000\nchar stat 8 640000\nchar stat 9 640000\n"
    );
    if let Some(skill) = skill {
        text.push_str(&format!("char skill {skill} 20\nchar hand right {skill}\n"));
    }
    text
}

fn class_skills(map: &GameMap, class: u32) -> impl Iterator<Item = &Skill> {
    map.playables
        .classes
        .iter()
        .find(|c| c.id == class)
        .into_iter()
        .flat_map(|c| c.skills.iter())
        .filter_map(|id| map.playables.skills.iter().find(|s| s.id == *id))
}

fn first_skill(map: &GameMap, class: u32) -> Option<&Skill> {
    class_skills(map, class).find(|s| !s.passive && s.req_level <= 30)
}

/// The class and skill every goal's character uses. The character carries no
/// items, so it must be a caster. The Necromancer's Bone Spear deals magic
/// damage (few monsters are immune) and is aimed at a unit like the Sorceress
/// spells; the Sorceress' Frozen Orb was cold, and the cold-immune bosses
/// (Bone Ash, Coldworm, Snapchip, Frozenstein) never lost life. The map's
/// skill names come from a string table whose indices do not match the rows,
/// so the spells are chosen by row instead.
fn goal_character(map: &GameMap) -> (u32, Option<u32>) {
    if map.playables.classes.iter().any(|c| c.id == 2) {
        if let Some(skill) =
            class_skills(map, 2).find(|s| s.id == 84 && !s.passive && !s.aura && s.req_level <= 30)
        {
            return (2, Some(skill.id));
        }
    }
    const SPELLS: [u32; 7] = [64, 56, 59, 47, 53, 45, 36];
    if map.playables.classes.iter().any(|c| c.id == 1) {
        let skill = SPELLS
            .iter()
            .find_map(|id| {
                class_skills(map, 1)
                    .find(|s| s.id == *id && !s.passive && !s.aura && s.req_level <= 30)
            })
            .or_else(|| first_skill(map, 1));
        if let Some(skill) = skill {
            return (1, Some(skill.id));
        }
    }
    (0, first_skill(map, 0).map(|s| s.id))
}

fn area_goal(id: u32, difficulty: &str, class: u32, skill: Option<u32>) -> Goal {
    let name = format!("area-{id}");
    let end = 90;
    let mut text = header(&name, difficulty, end, class, 30, skill);
    text.push_str(&format!(
        "at 0 poke warp {id}\nby {end} check player.area == {id}\n"
    ));
    Goal {
        name,
        kind: "area".into(),
        area: Some(id),
        class: Some(class),
        chain: None,
        max_ticks: end,
        text,
        pending: None,
    }
}

/// The town of `area`'s act, when the goal character does not start in it:
/// a `from_town` boss goal enters its level from inside the act.
fn act_town(map: &GameMap, area: u32) -> Option<u32> {
    let act = map.world.areas.iter().find(|a| a.id == area)?.act;
    map.world
        .areas
        .iter()
        .find(|a| a.town && a.act == act)
        .map(|a| a.id)
        .filter(|&town| town != START_TOWN && town != area)
}

fn boss_goal(
    boss: &super::model::BossKill,
    town: Option<u32>,
    difficulty: &str,
    class: u32,
    skill: Option<u32>,
) -> Goal {
    let name = match boss.superunique {
        Some(row) => format!("boss-{}-su{row}", boss.area),
        None => format!("boss-{}-{}", boss.area, boss.class),
    };
    // The setup steps (a seal or an altar the boss is released by) run
    // before the walk to the boss; a goto gets the time a long walk takes,
    // since later pokes do not wait for it.
    let mut setup = String::new();
    let mut go = 30;
    for step in &boss.setup {
        setup.push_str(&format!("at {go} {step}\n"));
        go += if step.starts_with("poke goto") {
            SETUP_GOTO_TICKS
        } else {
            SETUP_STEP_TICKS
        };
    }
    if !boss.setup.is_empty() {
        go += SETUP_SPAWN_TICKS;
    }
    let end = go + boss.fight.unwrap_or(FIGHT_TICKS);
    let mut text = header(&name, difficulty, end, class, 30, skill);
    // A superunique is followed by its row: its minions and the level's
    // ordinary monsters often share its class. A boss named by class is the
    // only monster of that class.
    let (goto, seen, dead, near) = match boss.superunique {
        Some(row) => (
            format!("superunique {row}"),
            format!("superunique({row}).guid"),
            format!("superunique({row}).dead"),
            format!("superunique({row}).dist"),
        ),
        None => (
            format!("unit 1:{}", boss.class),
            format!("unit(monster, {}).guid", boss.class),
            format!("unit(monster, {}).mode == dead", boss.class),
            format!("unit(monster, {}).dist", boss.class),
        ),
    };
    // The character may not die to the boss it fights: the goal is the
    // boss's death, and a goal runner that dies first proves nothing. The
    // first walk leaves the join's town mode (`poke warp` does not): a
    // player still in a town mode cannot start a skill, so every cast was
    // refused before.
    // The level is entered from the act's town, as a player arrives: an act
    // change raises no level-change quest event, and some bosses only
    // appear after one (Nihlathak leaves Harrogath for his temple when a
    // player enters the temple's levels).
    text.push_str("at 0 dev god on\n");
    match town {
        Some(town) => text.push_str(&format!(
            "at 0 poke warp {town}\nat 2 poke warp {}\n",
            boss.area
        )),
        None => text.push_str(&format!("at 0 poke warp {}\n", boss.area)),
    }
    text.push_str("at 4 msg Walk x=@x+2 y=@y\n");
    text.push_str(&setup);
    text.push_str(&format!("at {go} poke goto {goto}\n"));
    // The goto walks room by room over several ticks and the input steps
    // do not wait for it: hold them until the boss exists and the walk has
    // landed next to it.
    text.push_str(&format!("at {} input wait-until {seen} max 900\n", go + 1));
    text.push_str(&format!(
        "at {} input wait-until {near} <= 8 max 900\n",
        go + 2
    ));
    // The right-skill-on-unit message (0x0D): the click path turns a scripted
    // unit click into a ground skill click (0x0C), which a unit-target skill
    // refuses, so no skill ever started. The message is the client's own
    // skill use, aimed by the superuniques row (or the boss's class).
    let action = match boss.superunique {
        Some(row) => format!("poke msg 0x0D 1 @su{row}"),
        None => format!("poke msg 0x0D 1 @1:{}", boss.class),
    };
    // A skill message during the previous cast is dropped (or restarts the
    // sequence), so a 7-tick cadence never landed a second hit. One message
    // per cast cycle (the bolt and orb casts run about 20 ticks at 0% faster
    // cast) keeps the boss under fire.
    for tick in (go + 90..end - 20).step_by(20) {
        text.push_str(&format!("at {tick} {action}\n"));
    }
    text.push_str(&format!("by {end} check {dead}\n"));
    Goal {
        name,
        kind: "boss".into(),
        area: Some(boss.area),
        class: Some(boss.class),
        chain: None,
        max_ticks: end,
        text,
        pending: boss.pending.clone(),
    }
}

fn talk_goal(
    map: &GameMap,
    goal: &OverrideGoal,
    difficulty: &str,
    class: u32,
    skill: Option<u32>,
) -> Result<Option<Goal>> {
    let (Some(area), Some(npc), Some(chain)) = (goal.area, goal.npc, goal.chain) else {
        bail!("goal {}: talk needs area, npc and chain", goal.name);
    };
    let quest = map.objectives.quests.iter().find(|q| q.chain == chain);
    let Some(slot) = quest.and_then(|q| q.flag).map(|f| f as u8) else {
        return Ok(None);
    };
    let end = goal.max_ticks.unwrap_or(260);
    let mut text = header(&goal.name, difficulty, end, class, 30, skill);
    // Spawn the NPC next to the player: a town's NPCs are streamed with their
    // rooms, and some rooms are never active from the town entrance the warp
    // uses, so a `goto unit` alone cannot find those NPCs (Akara, Atma). The
    // goal exercises the conversation and the quest line, not the placement.
    // The interaction is the client's C→S entity message, not a screen click:
    // it works while the client is still loading an act and it is the path the
    // NPC menu tests use. The first message opens the quest line; Space
    // advances the rest of it.
    text.push_str(&format!(
        "at 0 poke warp {area}\nat 5 spawn {npc} @x+3 @y normal\nat 10 poke goto unit 1:{npc}\nat 20 msg InteractWithEntity type=1 id=@1:{npc}\n"
    ));
    // The 0x31 message is the quest's own line for this NPC (the state-0
    // entry in the data); it is what advances the quest record. The Space
    // steps only page through the dialog the interact opened.
    let message = quest
        .and_then(|q| q.steps.iter().min_by_key(|s| s.state))
        .and_then(|step| {
            step.npc_messages
                .iter()
                .find(|m| m.npc == npc)
                .map(|m| m.message)
                .or_else(|| step.npc_messages.first().map(|m| m.message))
        });
    if let Some(message) = message {
        text.push_str(&format!("at 24 poke talk @1:{npc} quest:{message}\n"));
    }
    for tick in (28..end.saturating_sub(4)).step_by(4).take(8) {
        text.push_str(&format!(
            "at {tick} key-if 32 client.dialog_lines != null\n"
        ));
    }
    text.push_str(&format!("by {end} check quest_word({slot}) changes\n"));
    Ok(Some(Goal {
        name: goal.name.clone(),
        kind: "talk".into(),
        area: Some(area),
        class: Some(npc),
        chain: Some(chain),
        max_ticks: end,
        text,
        pending: goal.pending.clone(),
    }))
}

/// Generates the goal list: area loads and boss kills from the map, then the
/// hand-written story goals from the overrides.
pub fn generate(map: &GameMap, overrides: &Overrides, difficulty: &str) -> Result<Vec<Goal>> {
    let (class, skill) = goal_character(map);
    let mut goals = Vec::new();
    for area in &map.world.areas {
        goals.push(area_goal(area.id, difficulty, class, skill));
    }
    for boss in &map.objectives.boss_kills {
        goals.push(boss_goal(
            boss,
            act_town(map, boss.area).filter(|_| boss.from_town),
            difficulty,
            class,
            skill,
        ));
    }
    for goal in &overrides.goals {
        match goal.kind.as_str() {
            "talk" => {
                if let Some(g) = talk_goal(map, goal, difficulty, class, skill)? {
                    goals.push(g);
                }
            }
            "area" => {
                let Some(area) = goal.area else {
                    bail!("goal {}: area needs area", goal.name);
                };
                let mut g = area_goal(area, difficulty, class, skill);
                g.name = goal.name.clone();
                g.pending = goal.pending.clone();
                goals.push(g);
            }
            "boss" => {
                let (Some(area), Some(boss_class)) = (goal.area, goal.class) else {
                    bail!("goal {}: boss needs area and class", goal.name);
                };
                let mut g = boss_goal(
                    &super::model::BossKill {
                        name: goal.name.clone(),
                        class: boss_class,
                        area,
                        act: 1,
                        superunique: None,
                        setup: Vec::new(),
                        from_town: false,
                        fight: None,
                        pending: goal.pending.clone(),
                    },
                    None,
                    difficulty,
                    class,
                    skill,
                );
                g.name = goal.name.clone();
                goals.push(g);
            }
            other => bail!("goal {}: unknown kind {other}", goal.name),
        }
    }
    for goal in &mut goals {
        Scenario::parse(&goal.text)
            .map_err(|e| anyhow::anyhow!("generated goal {}: {e}", goal.name))?;
    }
    Ok(goals)
}

fn run_lane(goals: &[Goal], lane: usize, o: &Options, game_dir: &Path) -> Result<Vec<Value>> {
    let scenarios = o.out.join("t-jump/scenarios");
    let runs = o.out.join("t-jump/runs");
    let checkpoints = o.out.join("t-jump/checkpoints");
    std::fs::create_dir_all(&scenarios)?;
    std::fs::create_dir_all(&runs)?;
    std::fs::create_dir_all(&checkpoints)?;
    let mut session = repro::Session::load(Some(game_dir))?;
    let mut checkpoint: Option<Checkpoint> = None;
    let mut rows = Vec::new();
    for (i, goal) in goals.iter().enumerate() {
        let index = lane + i * o.jobs.max(1);
        let next = checkpoints.join(format!("{lane:02}-{i:04}.json"));
        let scenario_path = scenarios.join(format!("{}.scn", goal.name));
        std::fs::write(&scenario_path, &goal.text)?;
        let scenario =
            Scenario::parse(&goal.text).map_err(|e| anyhow::anyhow!("goal {}: {e}", goal.name))?;
        // Only the talk goals chain: each sees the earlier conversations'
        // quest state. Boss and area goals are self-contained scenarios: a
        // chain would replay the earlier goals' steps, and their
        // `wait-until` pauses (one input script for the whole run) shift the
        // later goals' clicks past the boundary, where a new goal has
        // already warped away.
        let run_scenario = match (&checkpoint, goal.kind.as_str()) {
            (Some(cp), "talk") => cp.chain(&scenario)?,
            _ => scenario.clone(),
        };
        let opts = RunOpts {
            out: runs.join(format!("{lane:02}-{i:04}")),
            game_dir: Some(game_dir.to_path_buf()),
            base_dir: scenarios.clone(),
            keep_going: true,
            file: scenario_path.display().to_string(),
            capture_state: true,
            ..RunOpts::default()
        };
        let began = Instant::now();
        let (exit, failure, report_path) = match session.run(&run_scenario, &opts) {
            Ok(r) => (
                r.exit,
                r.json["first_failure"].clone(),
                opts.out.join("repro.json").display().to_string(),
            ),
            Err(e) => (
                repro::exit::FAIL,
                json!({"error": e.to_string()}),
                String::new(),
            ),
        };
        let wall_ms = began.elapsed().as_millis() as u64;
        let passed = exit == repro::exit::PASS;
        let checkpointed = passed && goal.kind == "talk";
        if checkpointed {
            let saved = Checkpoint::capture(&opts.out, &run_scenario, run_scenario.end)
                .and_then(|cp| cp.write(&next).map(|()| cp));
            match saved {
                Ok(cp) => checkpoint = Some(cp),
                Err(e) => {
                    rows.push(json!({
                        "index": index,
                        "lane": lane,
                        "name": goal.name,
                        "kind": goal.kind,
                        "area": goal.area,
                        "class": goal.class,
                        "exit": repro::exit::FAIL,
                        "wall_ms": wall_ms,
                        "checkpoint": null,
                        "first_failure": {"error": format!("checkpoint: {e}")},
                        "scenario": scenario_path.display().to_string(),
                        "pending": goal.pending,
                    }));
                    continue;
                }
            }
        } else if !passed {
            let blocks = o.out.join("t-jump/blocks");
            std::fs::create_dir_all(&blocks)?;
            std::fs::write(blocks.join(format!("{}.scn", goal.name)), &goal.text)?;
            std::fs::write(
                blocks.join(format!("{}.json", goal.name)),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&json!({
                        "name": goal.name,
                        "kind": goal.kind,
                        "area": goal.area,
                        "class": goal.class,
                        "first_failure": failure,
                        "run": report_path,
                    }))?
                ),
            )?;
        }
        rows.push(json!({
            "index": index,
            "lane": lane,
            "name": goal.name,
            "kind": goal.kind,
            "area": goal.area,
            "class": goal.class,
            "exit": exit,
            "wall_ms": wall_ms,
            "checkpoint": checkpointed.then(|| next.display().to_string()),
            "first_failure": failure,
            "scenario": scenario_path.display().to_string(),
            "pending": goal.pending,
        }));
    }
    Ok(rows)
}

/// Runs the goals and writes `t-jump.json`.
pub fn run(map: &GameMap, overrides: &Overrides, o: &Options) -> Result<Value> {
    let game_dir = o
        .game_dir
        .clone()
        .context("t-jump needs --game-dir or D2_GAME_DIR")?;
    let mut goals = generate(map, overrides, &o.difficulty)?;
    if let Some(filter) = &o.filter {
        goals.retain(|g| format!("{}/{}", g.kind, g.name).contains(filter));
    }
    if let Some(limit) = o.limit {
        goals.truncate(limit);
    }
    let jobs = o.jobs.max(1).min(goals.len().max(1));
    let began = Instant::now();
    let mut rows: Vec<Value> = Vec::new();
    if goals.is_empty() {
        let report = json!({
            "format": FORMAT,
            "difficulty": o.difficulty,
            "jobs": jobs,
            "totals": {"goals": 0, "passed": 0, "blocked": 0, "pending": 0, "failed": 0, "wall_ms": 0, "goal_ms": 0},
            "goals": [],
        });
        write_report(o, &report)?;
        return Ok(report);
    }
    std::thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        for lane in 0..jobs {
            let lane_goals: Vec<Goal> = goals
                .iter()
                .enumerate()
                .filter(|(i, _)| i % jobs == lane)
                .map(|(_, g)| g.clone())
                .collect();
            let o = o.clone();
            let game_dir = game_dir.clone();
            handles.push(scope.spawn(move || run_lane(&lane_goals, lane, &o, &game_dir)));
        }
        for handle in handles {
            let lane_rows = handle
                .join()
                .map_err(|_| anyhow::anyhow!("t-jump worker panicked"))??;
            rows.extend(lane_rows);
        }
        Ok(())
    })?;
    rows.sort_by_key(|r| r["index"].as_u64());
    let (passed, pending, failed) = tally(&rows);
    let goal_ms: u64 = rows.iter().filter_map(|r| r["wall_ms"].as_u64()).sum();
    let report = json!({
        "format": FORMAT,
        "difficulty": o.difficulty,
        "jobs": jobs,
        "totals": {
            "goals": rows.len(),
            "passed": passed,
            "blocked": rows.len() - passed,
            "pending": pending,
            "failed": failed,
            "wall_ms": began.elapsed().as_millis() as u64,
            "goal_ms": goal_ms,
        },
        "goals": rows,
    });
    write_report(o, &report)?;
    Ok(report)
}

/// (passed, blocked but marked pending, failed) of the goal rows. A blocked
/// goal is a failure unless the goal is marked pending with a reason.
fn tally(rows: &[Value]) -> (usize, usize, usize) {
    let passed = rows.iter().filter(|r| r["exit"] == 0).count();
    let pending = rows
        .iter()
        .filter(|r| r["exit"] != 0 && r["pending"].is_string())
        .count();
    (passed, pending, rows.len() - passed - pending)
}

/// The exit code of a T-jump report: 1 when a goal failed (blocked and not
/// marked pending), else 0.
pub fn exit_code(report: &Value) -> u8 {
    u8::from(report["totals"]["failed"].as_u64().unwrap_or(0) > 0)
}

fn write_report(o: &Options, report: &Value) -> Result<()> {
    std::fs::create_dir_all(o.out.join("t-jump"))?;
    std::fs::write(
        o.out.join("t-jump/t-jump.json"),
        format!("{}\n", serde_json::to_string_pretty(report)?),
    )?;
    println!(
        "t-jump: {}/{} passed, {} blocked ({} pending, {} failed), {} ms total ({} ms of goals)",
        report["totals"]["passed"],
        report["totals"]["goals"],
        report["totals"]["blocked"],
        report["totals"]["pending"],
        report["totals"]["failed"],
        report["totals"]["wall_ms"],
        report["totals"]["goal_ms"]
    );
    for row in report["goals"].as_array().into_iter().flatten() {
        let verdict = match (row["exit"] == 0, row["pending"].as_str()) {
            (true, _) => "pass".to_owned(),
            (false, Some(why)) => format!("pending ({why})"),
            (false, None) => "block".to_owned(),
        };
        println!(
            "{}\t{verdict}\t{} ms\t{}",
            row["name"].as_str().unwrap_or(""),
            row["wall_ms"].as_u64().unwrap_or(0),
            row["first_failure"]
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::game_map::model::{Area, BossKill, Class, NpcMessage, Quest, QuestStep, Skill};
    use crate::app::game_map::overrides::Goal as OverrideGoal;

    fn map() -> GameMap {
        let mut map = GameMap::default();
        map.world.areas.push(Area {
            id: 1,
            name: "Town".into(),
            act: 1,
            town: true,
            waypoint: true,
            ..Area::default()
        });
        map.world.areas.push(Area {
            id: 2,
            name: "Cave".into(),
            act: 1,
            ..Area::default()
        });
        map.objectives.quests.push(Quest {
            chain: 1,
            flag: Some(1),
            name: "Tiny".into(),
            steps: vec![QuestStep {
                state: 0,
                npcs: vec![148],
                messages: vec![100],
                npc_messages: vec![NpcMessage {
                    npc: 148,
                    message: 100,
                }],
                ..QuestStep::default()
            }],
            ..Quest::default()
        });
        map.objectives.boss_kills.push(BossKill {
            name: "Tiny Boss".into(),
            class: 9,
            area: 2,
            act: 1,
            superunique: Some(0),
            setup: Vec::new(),
            from_town: false,
            fight: None,
            pending: None,
        });
        map.playables.classes.push(Class {
            id: 0,
            name: "Amazon".into(),
            skills: vec![0],
        });
        map.playables.skills.push(Skill {
            id: 0,
            class: 0,
            req_level: 1,
            ..Skill::default()
        });
        map
    }

    fn overrides() -> Overrides {
        Overrides {
            goals: vec![OverrideGoal {
                name: "talk".into(),
                kind: "talk".into(),
                act: Some(1),
                area: Some(1),
                npc: Some(148),
                chain: Some(1),
                max_ticks: Some(120),
                ..OverrideGoal::default()
            }],
            ..Overrides::default()
        }
    }

    fn char_header(text: &str) -> Vec<&str> {
        text.lines().filter(|l| l.starts_with("char ")).collect()
    }

    #[test]
    fn goals_generate_from_the_map_and_parse() {
        let goals = generate(&map(), &overrides(), "normal").unwrap();
        assert_eq!(goals.len(), 4, "{goals:?}");
        assert_eq!(goals[0].kind, "area");
        assert!(goals.iter().any(|g| g.kind == "boss"));
        assert!(goals.iter().any(|g| g.kind == "talk"));
        let first = char_header(&goals[0].text);
        assert!(!first.is_empty());
        for goal in &goals {
            Scenario::parse(&goal.text).unwrap();
            assert_eq!(char_header(&goal.text), first, "{}", goal.name);
        }
    }

    #[test]
    fn a_superunique_boss_goal_follows_its_row_and_waits_for_the_walk() {
        let goals = generate(&map(), &overrides(), "normal").unwrap();
        let boss = goals.iter().find(|g| g.kind == "boss").unwrap();
        assert_eq!(boss.name, "boss-2-su0");
        assert!(boss.text.contains("at 0 dev god on\n"));
        assert!(boss.text.contains("at 4 msg Walk x=@x+2 y=@y\n"));
        assert!(boss.text.contains("at 30 poke goto superunique 0\n"));
        assert!(boss
            .text
            .contains("at 31 input wait-until superunique(0).guid max 900\n"));
        assert!(boss
            .text
            .contains("at 32 input wait-until superunique(0).dist <= 8 max 900\n"));
        assert!(boss.text.contains("at 120 poke msg 0x0D 1 @su0\n"));
        assert!(boss.text.contains("check superunique(0).dead\n"));
        // A corpse stays a unit of its class: a count can never reach 0.
        assert!(!boss.text.contains("count(monster"));
        // Every attack comes after the walk has had time to land.
        for line in boss.text.lines().filter(|l| l.contains("poke msg 0x0D")) {
            let tick: u32 = line
                .strip_prefix("at ")
                .unwrap()
                .split(' ')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert!(tick > 100, "{line}");
        }
    }

    #[test]
    fn a_from_town_boss_is_entered_from_its_acts_town() {
        let mut map = map();
        for (id, town) in [(40, true), (41, false)] {
            map.world.areas.push(Area {
                id,
                name: format!("Act 2 level {id}"),
                act: 2,
                town,
                ..Area::default()
            });
        }
        map.objectives.boss_kills[0].area = 41;
        map.objectives.boss_kills[0].from_town = true;
        let boss_text = |map: &GameMap| {
            let goals = generate(map, &overrides(), "normal").unwrap();
            goals.into_iter().find(|g| g.kind == "boss").unwrap().text
        };
        // An act change raises no level-change event: the warp into the
        // boss level starts in the act's town.
        assert!(boss_text(&map).contains("at 0 poke warp 40\nat 2 poke warp 41\n"));
        // Without `from_town` the warp goes straight into the level.
        map.objectives.boss_kills[0].from_town = false;
        assert!(boss_text(&map).contains("at 0 poke warp 41\nat 4 msg Walk"));
        // A level of the start town's act is warped to directly.
        let mut start = super::tests::map();
        start.objectives.boss_kills[0].from_town = true;
        assert!(boss_text(&start).contains("at 0 poke warp 2\nat 4 msg Walk"));
    }

    #[test]
    fn a_boss_released_by_a_setup_is_walked_to_after_it() {
        let mut map = map();
        map.objectives.boss_kills[0].setup =
            vec!["poke goto unit 2:396".into(), "poke operate @2:396".into()];
        let goals = generate(&map, &overrides(), "normal").unwrap();
        let boss = goals.iter().find(|g| g.kind == "boss").unwrap();
        Scenario::parse(&boss.text).unwrap();
        // The goto gets a long walk's time, the operate a short step, then
        // the released boss its time to appear.
        assert!(boss.text.contains("at 30 poke goto unit 2:396\n"));
        assert!(boss.text.contains("at 270 poke operate @2:396\n"));
        assert!(boss.text.contains("at 380 poke goto superunique 0\n"));
        assert!(boss
            .text
            .contains("at 381 input wait-until superunique(0).guid max 900\n"));
        assert!(boss.text.contains("at 470 poke msg 0x0D 1 @su0\n"));
        assert!(!boss.text.contains("at 120 poke msg"));
        // The fight keeps its length after the setup.
        assert!(boss.text.contains("by 1850 check superunique(0).dead\n"));
    }

    #[test]
    fn a_boss_with_its_own_fight_length_ends_after_it() {
        let mut map = map();
        let goals = generate(&map, &overrides(), "normal").unwrap();
        let boss = goals.iter().find(|g| g.kind == "boss").unwrap();
        assert!(boss.text.contains("by 1500 check superunique(0).dead\n"));
        map.objectives.boss_kills[0].fight = Some(2100);
        let goals = generate(&map, &overrides(), "normal").unwrap();
        let boss = goals.iter().find(|g| g.kind == "boss").unwrap();
        assert_eq!(boss.max_ticks, 2130);
        assert!(boss.text.contains("at 2100 poke msg 0x0D 1 @su0\n"));
        assert!(boss.text.contains("by 2130 check superunique(0).dead\n"));
    }

    #[test]
    fn a_boss_named_by_class_follows_its_class() {
        let mut map = map();
        map.objectives.boss_kills[0].superunique = None;
        let goals = generate(&map, &overrides(), "normal").unwrap();
        let boss = goals.iter().find(|g| g.kind == "boss").unwrap();
        assert_eq!(boss.name, "boss-2-9");
        assert!(boss.text.contains("poke goto unit 1:9\n"));
        assert!(boss
            .text
            .contains("at 31 input wait-until unit(monster, 9).guid max 900\n"));
        assert!(boss
            .text
            .contains("at 32 input wait-until unit(monster, 9).dist <= 8 max 900\n"));
        assert!(boss.text.contains("at 120 poke msg 0x0D 1 @1:9\n"));
        assert!(boss.text.contains("check unit(monster, 9).mode == dead\n"));
    }

    fn row(name: &str, exit: u8, pending: Option<&str>) -> Value {
        json!({"name": name, "exit": exit, "pending": pending})
    }

    fn report(rows: Vec<Value>) -> Value {
        let (passed, pending, failed) = tally(&rows);
        json!({"totals": {"passed": passed, "pending": pending, "failed": failed}})
    }

    #[test]
    fn a_blocked_goal_fails_the_run_unless_marked_pending() {
        // Every goal passed: exit 0.
        assert_eq!(exit_code(&report(vec![row("a", 0, None)])), 0);
        // One blocked goal: exit 1, even when every other goal passed.
        let blocked = report(vec![row("a", 0, None), row("b", 1, None)]);
        assert_eq!(blocked["totals"]["failed"], 1);
        assert_eq!(exit_code(&blocked), 1);
        // All blocked (the old 0/146 run): exit 1.
        assert_eq!(
            exit_code(&report(vec![row("a", 1, None), row("b", 3, None)])),
            1
        );
        // A blocked goal marked pending with a reason is reported, not failed.
        let pending = report(vec![
            row("a", 0, None),
            row("b", 1, Some("boss never spawns")),
        ]);
        assert_eq!(pending["totals"]["pending"], 1);
        assert_eq!(exit_code(&pending), 0);
        // A pending goal that passes counts as passed.
        let fixed = report(vec![row("b", 0, Some("boss never spawns"))]);
        assert_eq!(fixed["totals"]["passed"], 1);
        assert_eq!(exit_code(&fixed), 0);
    }

    #[test]
    fn a_talk_goal_uses_the_npc_entity_message_and_dialog_advance() {
        let goals = generate(&map(), &overrides(), "normal").unwrap();
        let talk = goals.iter().find(|g| g.kind == "talk").unwrap();
        assert!(talk.text.contains("spawn 148 @x+3 @y normal"));
        assert!(talk
            .text
            .contains("msg InteractWithEntity type=1 id=@1:148"));
        assert!(talk.text.contains("poke talk @1:148 quest:100"));
        assert!(talk.text.contains("key-if 32 client.dialog_lines != null"));
        assert!(talk.text.contains("check quest_word(1) changes"));
        assert!(!talk.text.contains("rclickunit"));
    }

    #[test]
    fn a_chained_goal_shifts_its_steps_past_the_checkpoint() {
        let goals = generate(&map(), &overrides(), "normal").unwrap();
        let tail = Scenario::parse(&goals[0].text).unwrap();
        let script = "scenario 2\ntick_rate 60\nname cp\nseed 7\ndifficulty normal\nend 30\nchar class 0\nchar level 30\nchar stat 6 640000\nchar stat 7 640000\nchar stat 8 640000\nchar stat 9 640000\nchar skill 0 20\nchar hand right 0\nat 0 poke warp 1\n";
        let cp = Checkpoint {
            at: 30,
            script: script.into(),
            state: String::new(),
        };
        let chained = cp.chain(&tail).unwrap();
        assert_eq!(chained.end, 30 + tail.end);
        assert!(chained.steps.iter().filter(|s| s.tick >= 30).count() > tail.steps.len() - 1);
    }

    #[test]
    fn a_tail_with_another_seed_is_refused() {
        let goals = generate(&map(), &overrides(), "normal").unwrap();
        let mut tail = Scenario::parse(&goals[0].text).unwrap();
        tail.seed = 8;
        let script = "scenario 2\ntick_rate 60\nname cp\nseed 7\ndifficulty normal\nend 30\nchar class 0\nchar level 30\nchar stat 6 640000\nchar stat 7 640000\nchar stat 8 640000\nchar stat 9 640000\nchar skill 0 20\nchar hand right 0\nat 0 poke warp 1\n";
        let cp = Checkpoint {
            at: 30,
            script: script.into(),
            state: String::new(),
        };
        assert!(cp.chain(&tail).is_err());
    }
}
