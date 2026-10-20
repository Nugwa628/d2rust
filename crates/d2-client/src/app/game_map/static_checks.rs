//! T-static: the map checks that run from the data alone, on every PR.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::model::*;

/// The report's format word.
pub const FORMAT: &str = "game-map-static 1";

/// One rule's result.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleResult {
    pub rule: String,
    pub checked: usize,
    pub findings: Vec<Finding>,
    pub omitted: usize,
}

/// One T-static finding (Unverified until a person confirms it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub detail: String,
}

/// The whole report.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub format: String,
    pub game: String,
    pub rules: Vec<RuleResult>,
    pub exit: u8,
}

const MAX_FINDINGS: usize = 100;

struct Checks {
    rules: Vec<RuleResult>,
}

impl Checks {
    fn rule(&mut self, rule: &str, checked: usize, findings: Vec<Finding>) {
        let omitted = findings.len().saturating_sub(MAX_FINDINGS);
        self.rules.push(RuleResult {
            rule: rule.into(),
            checked,
            findings: findings.into_iter().take(MAX_FINDINGS).collect(),
            omitted,
        });
    }
}

fn find(id: impl Into<String>, detail: impl Into<String>, out: &mut Vec<Finding>) {
    let id = id.into();
    if !out.iter().any(|f| f.id == id) {
        out.push(Finding {
            id,
            detail: detail.into(),
        });
    }
}

fn reachability(map: &GameMap) -> Vec<Finding> {
    let exempt: BTreeSet<u32> = map.world.unreachable.iter().map(|u| u.area).collect();
    let mut adjacency: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for link in &map.world.links {
        adjacency.entry(link.from).or_default().insert(link.to);
        adjacency.entry(link.to).or_default().insert(link.from);
    }
    let mut acts: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for area in &map.world.areas {
        acts.entry(area.act).or_default().push(area.id);
    }
    let mut findings = Vec::new();
    for (act, areas) in acts {
        let towns: Vec<u32> = map
            .world
            .areas
            .iter()
            .filter(|a| a.act == act && a.town)
            .map(|a| a.id)
            .collect();
        let starts = if towns.is_empty() { areas } else { towns };
        let mut seen: BTreeSet<u32> = starts.iter().copied().collect();
        let mut queue: VecDeque<u32> = starts.iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            for &to in adjacency.get(&id).into_iter().flatten() {
                if seen.insert(to) {
                    queue.push_back(to);
                }
            }
        }
        for area in map.world.areas.iter().filter(|a| a.act == act) {
            if exempt.contains(&area.id) {
                continue;
            }
            if !seen.contains(&area.id) {
                find(
                    format!("area-{}", area.id),
                    format!(
                        "{} (area {}, act {}) has no link path from an act start",
                        area.name, area.id, act
                    ),
                    &mut findings,
                );
            }
        }
    }
    findings
}

fn link_references(map: &GameMap) -> Vec<Finding> {
    let ids: BTreeSet<u32> = map.world.areas.iter().map(|a| a.id).collect();
    let mut findings = Vec::new();
    for link in &map.world.links {
        if !ids.contains(&link.from) || !ids.contains(&link.to) {
            find(
                format!("link-{}-{}", link.from, link.to),
                format!("link {} -> {} names a missing area", link.from, link.to),
                &mut findings,
            );
        }
        if link.from == link.to {
            find(
                format!("self-link-{}", link.from),
                format!("area {} links to itself", link.from),
                &mut findings,
            );
        }
    }
    findings
}

fn quest_steps(map: &GameMap) -> Vec<Finding> {
    let mut findings = Vec::new();
    for quest in &map.objectives.quests {
        // A quest with no message table completes through a scripted event;
        // only a quest that names one must have an NPC step.
        if quest.message_table.is_some() && quest.steps.is_empty() {
            find(
                format!("quest-{}", quest.chain),
                format!("{} has no NPC message step", quest.name),
                &mut findings,
            );
        }
        for step in &quest.steps {
            if step.messages.is_empty() && step.npcs.is_empty() {
                find(
                    format!("quest-{}-state-{}", quest.chain, step.state),
                    format!("{} state {} has no message or NPC", quest.name, step.state),
                    &mut findings,
                );
            }
        }
    }
    findings
}

/// Item codes are four bytes, space or NUL padded; an override may write the
/// padding, the map's code text does not.
fn code_key(code: &str) -> &str {
    code.trim_end_matches([' ', '\0'])
}

fn rewards(map: &GameMap) -> Vec<Finding> {
    let codes: BTreeSet<&str> = map
        .playables
        .items
        .iter()
        .map(|i| code_key(i.code.as_str()))
        .collect();
    let mut findings = Vec::new();
    for quest in &map.objectives.quests {
        for reward in &quest.rewards {
            if reward.kind == "item" && reward.item.is_none() {
                find(
                    format!("reward-{}-{}", quest.chain, reward.name),
                    format!("{} names an item reward with no code", reward.name),
                    &mut findings,
                );
            }
            if let Some(code) = &reward.item {
                if !codes.contains(code_key(code)) {
                    find(
                        format!("reward-{}-{}", quest.chain, reward.name),
                        format!("{} names missing item {code:?}", reward.name),
                        &mut findings,
                    );
                }
            }
        }
    }
    findings
}

fn item_references(map: &GameMap) -> Vec<Finding> {
    let codes: BTreeSet<&str> = map
        .playables
        .items
        .iter()
        .map(|i| code_key(i.code.as_str()))
        .collect();
    let skills: BTreeSet<u32> = map.playables.skills.iter().map(|s| s.id).collect();
    let types: BTreeSet<&str> = map
        .playables
        .items
        .iter()
        .flat_map(|i| {
            i.types
                .iter()
                .map(String::as_str)
                .chain(std::iter::once(i.item_type.as_str()))
        })
        .filter(|t| !t.is_empty())
        .collect();
    let mut findings = Vec::new();
    for item in &map.playables.items {
        if item.code.is_empty() {
            find(
                format!("item-{}", item.name),
                format!("item {} has an empty code", item.name),
                &mut findings,
            );
        }
        if item.item_type.is_empty() {
            find(
                format!("item-type-{}", item.name),
                format!("item {} has no type", item.name),
                &mut findings,
            );
        }
    }
    for skill in &map.playables.skills {
        for req in &skill.requires {
            if !skills.contains(req) {
                find(
                    format!("skill-{}-req-{req}", skill.id),
                    format!("skill {} requires missing skill {req}", skill.name),
                    &mut findings,
                );
            }
        }
    }
    for recipe in &map.playables.cube_recipes {
        if !recipe.enabled {
            continue;
        }
        for (i, input) in recipe.inputs.iter().enumerate() {
            if input.kind == "item" && input.code.is_none() {
                find(
                    format!("recipe-{}-input-{i}", recipe.index),
                    format!("recipe {} input {i} names a missing item", recipe.index),
                    &mut findings,
                );
            }
            if input.kind == "type"
                && input
                    .item_type
                    .as_ref()
                    .is_none_or(|t| !types.contains(t.as_str()))
            {
                find(
                    format!("recipe-{}-input-{i}", recipe.index),
                    format!("recipe {} input {i} names a missing type", recipe.index),
                    &mut findings,
                );
            }
        }
        for (i, output) in recipe.outputs.iter().enumerate() {
            if output.kind == "item" && output.code.is_none() {
                find(
                    format!("recipe-{}-output-{i}", recipe.index),
                    format!("recipe {} output {i} names a missing item", recipe.index),
                    &mut findings,
                );
            }
            if output.kind == "type"
                && output
                    .item_type
                    .as_ref()
                    .is_none_or(|t| !types.contains(t.as_str()))
            {
                find(
                    format!("recipe-{}-output-{i}", recipe.index),
                    format!("recipe {} output {i} names a missing type", recipe.index),
                    &mut findings,
                );
            }
        }
    }
    for runeword in &map.playables.runewords {
        if runeword.complete && runeword.runes.len() < 2 {
            find(
                format!("runeword-{}", runeword.name),
                format!("runeword {} has fewer than two runes", runeword.name),
                &mut findings,
            );
        }
        for rune in &runeword.runes {
            if !codes.contains(code_key(rune)) {
                find(
                    format!("runeword-{}-{rune}", runeword.name),
                    format!("runeword {} names missing rune {rune:?}", runeword.name),
                    &mut findings,
                );
            }
        }
        for item_type in &runeword.item_types {
            if !types.contains(item_type.as_str()) {
                find(
                    format!("runeword-type-{}-{item_type}", runeword.name),
                    format!(
                        "runeword {} names missing type {item_type:?}",
                        runeword.name
                    ),
                    &mut findings,
                );
            }
        }
    }
    findings
}

fn class_skills(map: &GameMap) -> Vec<Finding> {
    let mut findings = Vec::new();
    for class in &map.playables.classes {
        let usable = class.skills.iter().any(|id| {
            map.playables
                .skills
                .iter()
                .find(|s| s.id == *id)
                .is_some_and(|s| s.req_level <= 30)
        });
        if !usable {
            find(
                format!("class-{}", class.id),
                format!("{} has no skill with requirement 30 or lower", class.name),
                &mut findings,
            );
        }
    }
    findings
}

fn world_references(map: &GameMap) -> Vec<Finding> {
    let ids: BTreeSet<u32> = map.world.areas.iter().map(|a| a.id).collect();
    let mut findings = Vec::new();
    for waypoint in &map.world.waypoints {
        if !ids.contains(&waypoint.level) {
            find(
                format!("waypoint-{}", waypoint.level),
                format!("waypoint {} names a missing area", waypoint.level),
                &mut findings,
            );
        }
    }
    for boss in &map.objectives.boss_kills {
        if !ids.contains(&boss.area) {
            find(
                format!("boss-{}-{}", boss.area, boss.class),
                format!("{} names a missing area {}", boss.name, boss.area),
                &mut findings,
            );
        }
    }
    findings
}

/// Runs every T-static rule.
pub fn run(map: &GameMap) -> Report {
    let mut checks = Checks { rules: Vec::new() };
    checks.rule("area-reachable", map.world.areas.len(), reachability(map));
    checks.rule("link-target", map.world.links.len(), link_references(map));
    checks.rule("quest-step", map.objectives.quests.len(), quest_steps(map));
    checks.rule(
        "reward-exists",
        map.objectives.quests.iter().map(|q| q.rewards.len()).sum(),
        rewards(map),
    );
    checks.rule(
        "item-references",
        map.playables.items.len()
            + map.playables.cube_recipes.len()
            + map.playables.runewords.len(),
        item_references(map),
    );
    checks.rule(
        "class-skills",
        map.playables.classes.len(),
        class_skills(map),
    );
    checks.rule(
        "world-references",
        map.world.waypoints.len() + map.objectives.boss_kills.len(),
        world_references(map),
    );
    let failed = checks.rules.iter().any(|r| !r.findings.is_empty());
    Report {
        format: FORMAT.into(),
        game: map.game.source.clone(),
        rules: checks.rules,
        exit: u8::from(failed),
    }
}

/// The first finding of a rule, for tests and short output.
#[cfg(test)]
pub fn first_failure<'a>(report: &'a Report, rule: &str) -> Option<&'a Finding> {
    report
        .rules
        .iter()
        .find(|r| r.rule == rule)
        .and_then(|r| r.findings.first())
}
