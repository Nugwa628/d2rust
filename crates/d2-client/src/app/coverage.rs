//! Shared coverage envelope for repro, explore and sweep reports.
//! Counters describe events; no counter is part of a simulation decision.

use super::autoplay_host::Host;
use crate::world_view::WorldViewUi;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Versioned shared report field. Sweep writers can merge these envelopes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub format: String,
    pub counters: BTreeMap<String, BTreeMap<String, u64>>,
    /// Numeric row universe, with no table text or game assets.
    #[serde(default)]
    pub tables: BTreeMap<String, Vec<String>>,
}

impl Default for Coverage {
    fn default() -> Self {
        Self {
            format: "coverage 1".into(),
            counters: BTreeMap::new(),
            tables: BTreeMap::new(),
        }
    }
}

impl Coverage {
    pub fn merge(&mut self, other: &Self) {
        self.format = "coverage 1".into();
        for (category, rows) in &other.tables {
            let ids = self.tables.entry(category.clone()).or_default();
            ids.extend(rows.iter().cloned());
            ids.sort();
            ids.dedup();
        }
        for (category, rows) in &other.counters {
            for (id, count) in rows {
                let total = self
                    .counters
                    .entry(category.clone())
                    .or_default()
                    .entry(id.clone())
                    .or_default();
                *total = total.saturating_add(*count);
            }
        }
    }
}

fn report_rows(
    counters: &d2_sim::debug::coverage::Counters,
) -> BTreeMap<String, BTreeMap<String, u64>> {
    let mut rows: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for (&(category, id), &count) in &counters.rows {
        let id = if matches!(category, "server_packets_received" | "server_packets_sent") {
            format!("{id:02X}")
        } else {
            id.to_string()
        };
        rows.entry(category.into()).or_default().insert(id, count);
    }
    rows
}

/// Read the server-owned counts and the client's UI and transport counters.
pub fn snapshot(host: &mut Host) -> Result<Coverage> {
    let (counters, tables) = host
        .server()
        .lock()
        .map_err(|_| anyhow!("server lock"))?
        .with(|link| {
            let sim = &link.host().game;
            let t = &sim.events.action.sys.hooks.tables;
            let mut tables: BTreeMap<String, Vec<String>> = [
                ("skills_cast", t.skills.skills.len()),
                ("missiles_spawned", t.missiles.len()),
                ("monsters_spawned", t.combat.monstats.len()),
                ("monsters_killed", t.combat.monstats.len()),
                ("areas_loaded", t.levels.len()),
            ]
            .into_iter()
            .map(|(name, len)| (name.into(), (0..len).map(|id| id.to_string()).collect()))
            .collect();
            tables.insert(
                "npc_menus_opened".into(),
                t.combat
                    .monstats
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.npc)
                    .map(|(id, _)| id.to_string())
                    .collect(),
            );
            let mut rows = report_rows(&sim.game.coverage);
            rows.extend(report_rows(&link.host().coverage));
            (rows, tables)
        })
        .map_err(|e| anyhow!("{e:?}"))?;
    let mut result = Coverage {
        format: "coverage 1".into(),
        counters,
        tables,
    };
    let ui = host.app_mut().world().non_send::<WorldViewUi>();
    result.merge(&Coverage {
        format: "coverage 1".into(),
        counters: report_rows(&ui.root.coverage),
        tables: Default::default(),
    });
    let state = host.state()?;
    for (field, category) in [("sent", "packets_sent"), ("received", "packets_received")] {
        if let Some(rows) = state["client"][field].as_object() {
            for (id, count) in rows {
                result
                    .counters
                    .entry(category.into())
                    .or_default()
                    .insert(id.clone(), count.as_u64().unwrap_or(0));
            }
        }
    }
    // C→S item intent IDs, counted at the transport boundary, not inferred
    // from final inventory snapshots. Rejected requests remain exercised actions.
    if let Some(sent) = result.counters.get("packets_sent").cloned() {
        for (id, count) in sent {
            if u8::from_str_radix(&id, 16).is_ok_and(|n| matches!(n, 0x16..=0x2A | 0x32..=0x35)) {
                result
                    .counters
                    .entry("item_actions".into())
                    .or_default()
                    .insert(id, count);
            }
        }
    }
    Ok(result)
}
// TODO(Q3): add `coverage: Coverage` to sweep.json and merge each case's
// repro coverage. The sweep writer is not present on this main revision.

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_events_keep_report_key_formats() {
        let mut counters = d2_sim::debug::coverage::Counters {
            enabled: true,
            ..Default::default()
        };
        counters.hit("server_packets_received", || 0x0C);
        counters.hit("skills_cast", || 47);
        let rows = report_rows(&counters);
        assert_eq!(rows["server_packets_received"]["0C"], 1);
        assert_eq!(rows["skills_cast"]["47"], 1);
    }

    #[test]
    fn merge_sums_events_and_unions_table_rows() {
        let input = |count, rows| Coverage {
            format: "coverage 1".into(),
            counters: BTreeMap::from([(
                "skills_cast".into(),
                BTreeMap::from([("7".into(), count)]),
            )]),
            tables: BTreeMap::from([("skills_cast".into(), rows)]),
        };
        let mut total = Coverage::default();
        total.merge(&input(2, vec!["7".into()]));
        total.merge(&input(3, vec!["7".into(), "9".into()]));
        assert_eq!(total.counters["skills_cast"]["7"], 5);
        assert_eq!(total.tables["skills_cast"], ["7", "9"]);
        assert_eq!(
            serde_json::from_value::<Coverage>(serde_json::to_value(&total).unwrap()).unwrap(),
            total
        );
    }
}
