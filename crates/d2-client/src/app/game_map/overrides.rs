//! `game-map.overrides.toml`: the short hand-written file for what the data
//! cannot say. The base game's file is embedded from `tables/world`; a mod
//! keeps its own next to its files.

use toml_edit::{DocumentMut, Item, Table};

use super::model::ActTransition;

/// The base game's overrides, shipped in this repo.
pub const BASE: &str = include_str!("../../../../../tables/world/game-map.overrides.toml");

/// A link gate the data does not name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    pub from: u32,
    pub to: u32,
    pub kind: String,
}

/// One hand-written world edge the tables cannot name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkOverride {
    pub from: u32,
    pub to: u32,
    pub kind: String,
    pub reason: String,
}

/// A hand-written chain of world edges: each consecutive pair is a link.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathOverride {
    pub areas: Vec<u32>,
    pub kind: String,
    pub reason: String,
}

/// An area the shipped game has no entrance to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unreachable {
    pub area: u32,
    pub reason: String,
}

/// A reward named for a quest chain.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reward {
    pub chain: u32,
    pub kind: String,
    pub name: String,
    pub item: Option<String>,
}

/// A level boss: the level and either a `superuniques` row or a monster
/// class. The tables do not say where a boss stands (the level maps do).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Boss {
    pub area: u32,
    pub superunique: Option<u32>,
    pub monster: Option<u32>,
    pub setup: Vec<String>,
    pub from_town: bool,
    pub fight: Option<u32>,
    pub pending: Option<String>,
}

/// A hand-written goal for the T-jump runner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Goal {
    pub name: String,
    pub kind: String,
    pub act: Option<u32>,
    pub area: Option<u32>,
    pub class: Option<u32>,
    pub npc: Option<u32>,
    pub chain: Option<u32>,
    pub state: Option<u32>,
    pub skill: Option<u32>,
    pub gear: Vec<String>,
    pub max_ticks: Option<u32>,
    /// Why the goal is expected to block; a pending goal that blocks does
    /// not fail the run.
    pub pending: Option<String>,
}

/// The parsed override file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    pub game: Option<String>,
    pub final_objective: Option<String>,
    pub towns: Vec<u32>,
    pub transitions: Vec<ActTransition>,
    pub gates: Vec<Gate>,
    pub links: Vec<LinkOverride>,
    pub paths: Vec<PathOverride>,
    pub unreachable: Vec<Unreachable>,
    pub rewards: Vec<Reward>,
    pub prerequisites: Vec<(u32, Vec<u32>)>,
    pub bosses: Vec<Boss>,
    pub goals: Vec<Goal>,
}

fn int(item: Option<&Item>, key: &str) -> Result<Option<u32>, String> {
    match item {
        None => Ok(None),
        Some(i) => i
            .as_integer()
            .and_then(|v| u32::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| format!("{key}: a non-negative integer")),
    }
}

fn text(item: Option<&Item>, key: &str) -> Result<Option<String>, String> {
    match item {
        None => Ok(None),
        Some(i) => i
            .as_str()
            .map(|s| Some(s.to_owned()))
            .ok_or_else(|| format!("{key}: a string")),
    }
}

fn texts(table: &Table, key: &str) -> Result<Vec<String>, String> {
    let Some(item) = table.get(key) else {
        return Ok(Vec::new());
    };
    let array = item
        .as_array()
        .ok_or_else(|| format!("{key}: an array of strings"))?;
    array
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key}: an array of strings"))
        })
        .collect()
}

fn ints(table: &Table, key: &str) -> Result<Vec<u32>, String> {
    let Some(item) = table.get(key) else {
        return Ok(Vec::new());
    };
    let array = item
        .as_array()
        .ok_or_else(|| format!("{key}: an array of integers"))?;
    array
        .iter()
        .map(|v| {
            v.as_integer()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| format!("{key}: an array of integers"))
        })
        .collect()
}

fn tables<'a>(doc: &'a DocumentMut, key: &str) -> Result<Vec<&'a Table>, String> {
    let Some(item) = doc.get(key) else {
        return Ok(Vec::new());
    };
    let tables = item
        .as_array_of_tables()
        .ok_or_else(|| format!("{key}: an array of tables"))?;
    Ok(tables.iter().collect())
}

/// Parses an override file.
pub fn parse(input: &str) -> Result<Overrides, String> {
    let doc = input
        .parse::<DocumentMut>()
        .map_err(|e| format!("overrides: {e}"))?;
    let mut o = Overrides {
        game: text(doc.get("game"), "game")?,
        final_objective: text(doc.get("final_objective"), "final_objective")?,
        towns: ints(doc.as_table(), "towns")?,
        ..Overrides::default()
    };
    for t in tables(&doc, "act_transition")? {
        let from =
            int(t.get("from"), "act_transition.from")?.ok_or("act_transition.from: required")?;
        let to = int(t.get("to"), "act_transition.to")?.ok_or("act_transition.to: required")?;
        o.transitions.push(ActTransition {
            from,
            to,
            quest: text(t.get("quest"), "act_transition.quest")?,
            area: int(t.get("area"), "act_transition.area")?,
        });
    }
    for t in tables(&doc, "gate")? {
        o.gates.push(Gate {
            from: int(t.get("from"), "gate.from")?.ok_or("gate.from: required")?,
            to: int(t.get("to"), "gate.to")?.ok_or("gate.to: required")?,
            kind: text(t.get("kind"), "gate.kind")?.ok_or("gate.kind: required")?,
        });
    }
    for t in tables(&doc, "link")? {
        o.links.push(LinkOverride {
            from: int(t.get("from"), "link.from")?.ok_or("link.from: required")?,
            to: int(t.get("to"), "link.to")?.ok_or("link.to: required")?,
            kind: text(t.get("kind"), "link.kind")?.unwrap_or_else(|| "override".into()),
            reason: text(t.get("reason"), "link.reason")?.ok_or("link.reason: required")?,
        });
    }
    for t in tables(&doc, "path")? {
        let areas = ints(t, "areas")?;
        if areas.len() < 2 {
            return Err("path.areas: at least two area ids".into());
        }
        o.paths.push(PathOverride {
            areas,
            kind: text(t.get("kind"), "path.kind")?.unwrap_or_else(|| "path".into()),
            reason: text(t.get("reason"), "path.reason")?.ok_or("path.reason: required")?,
        });
    }
    for t in tables(&doc, "unreachable")? {
        o.unreachable.push(Unreachable {
            area: int(t.get("area"), "unreachable.area")?.ok_or("unreachable.area: required")?,
            reason: text(t.get("reason"), "unreachable.reason")?
                .ok_or("unreachable.reason: required")?,
        });
    }
    for t in tables(&doc, "reward")? {
        o.rewards.push(Reward {
            chain: int(t.get("chain"), "reward.chain")?.ok_or("reward.chain: required")?,
            kind: text(t.get("kind"), "reward.kind")?.ok_or("reward.kind: required")?,
            name: text(t.get("name"), "reward.name")?.ok_or("reward.name: required")?,
            item: text(t.get("item"), "reward.item")?,
        });
    }
    for t in tables(&doc, "quest")? {
        let Some(chain) = int(t.get("chain"), "quest.chain")? else {
            continue;
        };
        let before = ints(t, "prerequisites")?;
        if !before.is_empty() {
            o.prerequisites.push((chain, before));
        }
    }
    for t in tables(&doc, "boss")? {
        let boss = Boss {
            area: int(t.get("area"), "boss.area")?.ok_or("boss.area: required")?,
            superunique: int(t.get("superunique"), "boss.superunique")?,
            monster: int(t.get("monster"), "boss.monster")?,
            setup: texts(t, "setup")?,
            from_town: t
                .get("from_town")
                .map(|v| v.as_bool().ok_or("boss.from_town: true or false"))
                .transpose()?
                .unwrap_or(false),
            fight: int(t.get("fight"), "boss.fight")?,
            pending: text(t.get("pending"), "boss.pending")?,
        };
        if boss.superunique.is_some() == boss.monster.is_some() {
            return Err(format!(
                "boss in area {}: one of superunique or monster",
                boss.area
            ));
        }
        o.bosses.push(boss);
    }
    for t in tables(&doc, "goal")? {
        o.goals.push(Goal {
            name: text(t.get("name"), "goal.name")?.ok_or("goal.name: required")?,
            kind: text(t.get("kind"), "goal.kind")?.ok_or("goal.kind: required")?,
            act: int(t.get("act"), "goal.act")?,
            area: int(t.get("area"), "goal.area")?,
            class: int(t.get("class"), "goal.class")?,
            npc: int(t.get("npc"), "goal.npc")?,
            chain: int(t.get("chain"), "goal.chain")?,
            state: int(t.get("state"), "goal.state")?,
            skill: int(t.get("skill"), "goal.skill")?,
            gear: texts(t, "gear")?,
            max_ticks: int(t.get("max_ticks"), "goal.max_ticks")?,
            pending: text(t.get("pending"), "goal.pending")?,
        });
    }
    Ok(o)
}

/// Loads a file, or the embedded base file.
pub fn load(path: Option<&std::path::Path>) -> Result<(Overrides, Option<String>), String> {
    match path {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("overrides {}: {e}", path.display()))?;
            Ok((parse(&text)?, Some(path.display().to_string())))
        }
        None => Ok((parse(BASE).map_err(|e| format!("built-in {e}"))?, None)),
    }
}
