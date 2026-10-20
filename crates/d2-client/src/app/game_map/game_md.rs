//! `GAME.md`: the readable summary of a map for developers and AI sessions.
//! It is generated from `game-map.json` and never ships to players.

use std::fmt::Write;

use super::model::*;
use super::static_checks::Report;

/// The file's first line: this file is generated, not edited.
pub const HEADER: &str = "<!-- generated from game-map.json by D2RUST-Game map; do not edit -->";

/// Writes the summary.
pub fn write(map: &GameMap, checks: &Report) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{HEADER}");
    let _ = writeln!(out, "# {}", map.game.game_title());
    let _ = writeln!(out);
    let _ = writeln!(out, "{}", map.game.source_line());
    let _ = writeln!(out);
    let _ = writeln!(out, "## Summary");
    let s = &map.summary;
    let _ = writeln!(
        out,
        "{} acts, {} areas ({} towns), {} links, {} waypoints.",
        s.acts, s.areas, s.towns, s.links, s.waypoints
    );
    let _ = writeln!(
        out,
        "{} quests, {} boss kills, {} classes, {} skills, {} items, {} NPCs, {} objects.",
        s.quests, s.boss_kills, s.classes, s.skills, s.items, s.npcs, s.objects
    );
    let _ = writeln!(
        out,
        "{} cube recipes, {} runewords.",
        s.cube_recipes, s.runewords
    );
    if let Some(endgame) = &map.objectives.endgame {
        let _ = writeln!(out, "Final objective: **{}**.", endgame.name);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "## Acts, towns and waypoints");
    for act in 1..=s.acts {
        let areas: Vec<&Area> = map.world.areas.iter().filter(|a| a.act == act).collect();
        if areas.is_empty() {
            continue;
        }
        let towns: Vec<&str> = areas
            .iter()
            .filter(|a| a.town)
            .map(|a| a.name.as_str())
            .collect();
        let _ = writeln!(
            out,
            "- **Act {act}**: {} areas; town(s): {}.",
            areas.len(),
            if towns.is_empty() {
                "none named".to_owned()
            } else {
                towns.join(", ")
            }
        );
    }
    let _ = writeln!(out, "\nWaypoints:");
    for w in &map.world.waypoints {
        let _ = writeln!(
            out,
            "- {} (area {}, act {}){}",
            w.name,
            w.level,
            w.act,
            if w.town { ", town" } else { "" }
        );
    }
    let _ = writeln!(out, "\n## Quests");
    for q in &map.objectives.quests {
        let _ = writeln!(
            out,
            "- **{}** (chain {}, act {}, flag {})",
            q.name,
            q.chain,
            q.act,
            q.flag.map_or_else(|| "-".into(), |f| f.to_string())
        );
        for step in &q.steps {
            let _ = writeln!(out, "  - state {}: {}", step.state, step.text);
        }
        for reward in &q.rewards {
            let _ = writeln!(
                out,
                "  - reward: {} ({}){}",
                reward.name,
                reward.kind,
                reward
                    .item
                    .as_ref()
                    .map_or_else(String::new, |i| format!(" item {i}"))
            );
        }
        if !q.prerequisites.is_empty() {
            let _ = writeln!(out, "  - after chain(s): {:?}", q.prerequisites);
        }
    }
    let _ = writeln!(out, "\n## Bosses");
    for boss in &map.objectives.boss_kills {
        let _ = writeln!(
            out,
            "- {} (class {}, area {}, act {}){}",
            boss.name,
            boss.class,
            boss.area,
            boss.act,
            match boss.superunique {
                Some(row) => format!(", superunique row {row}"),
                None => String::new(),
            }
        );
        if let Some(why) = &boss.pending {
            let _ = writeln!(out, "  - pending: {why}");
        }
    }
    let _ = writeln!(out, "\n## Classes and skills");
    for class in &map.playables.classes {
        let usable = class.skills.len();
        let _ = writeln!(out, "- **{}**: {} skills.", class.name, usable);
        let names: Vec<&str> = class
            .skills
            .iter()
            .filter_map(|id| map.playables.skills.iter().find(|s| s.id == *id))
            .filter(|s| s.req_level <= 30)
            .map(|s| s.name.as_str())
            .take(12)
            .collect();
        if !names.is_empty() {
            let _ = writeln!(out, "  - usable by 30: {}", names.join(", "));
        }
    }
    let _ = writeln!(out, "\n## NPCs and services");
    for npc in &map.playables.npcs {
        let _ = writeln!(
            out,
            "- {} (class {}, act {}): {}",
            npc.name,
            npc.class,
            npc.act,
            if npc.services.is_empty() {
                "no service".to_owned()
            } else {
                npc.services.join(", ")
            }
        );
    }
    let _ = writeln!(out, "\n## Items and crafting");
    let mut kinds: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for item in &map.playables.items {
        *kinds.entry(item.kind.as_str()).or_default() += 1;
    }
    let _ = writeln!(
        out,
        "Items: {}.",
        kinds
            .iter()
            .map(|(k, n)| format!("{n} {k}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let useable = map
        .playables
        .cube_recipes
        .iter()
        .filter(|r| r.enabled)
        .count();
    let _ = writeln!(
        out,
        "Cube recipes: {} ({} enabled).",
        map.playables.cube_recipes.len(),
        useable
    );
    let _ = writeln!(out, "Runewords: {}.", map.playables.runewords.len());
    let _ = writeln!(out, "\n## T-static");
    let _ = writeln!(out, "{} from `game-map.json`.", checks.format);
    for rule in &checks.rules {
        let _ = writeln!(
            out,
            "- {}: {} checked, {} failed{}.",
            rule.rule,
            rule.checked,
            rule.findings.len() + rule.omitted,
            if rule.omitted > 0 {
                format!(" ({} shown)", rule.findings.len())
            } else {
                String::new()
            }
        );
        for finding in &rule.findings {
            let _ = writeln!(out, "  - {}: {}", finding.id, finding.detail);
        }
    }
    out
}

impl GameInfo {
    fn game_title(&self) -> String {
        self.mod_name
            .clone()
            .unwrap_or_else(|| "D2RUST game map".to_owned())
    }

    fn source_line(&self) -> String {
        format!(
            "Generated from {}{}{}.",
            self.source,
            self.overrides
                .as_ref()
                .map_or_else(String::new, |p| format!("; overrides {p}")),
            self.final_objective
                .as_ref()
                .map_or_else(String::new, |f| format!("; final objective {f}"))
        )
    }
}
