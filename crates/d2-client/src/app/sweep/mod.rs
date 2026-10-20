//! Content-derived scenarios, run through the warm repro runner and its rules.
mod anims;
mod areas;
mod content;
mod doors;
mod items;
mod npcs;
mod panels;
mod skills;

use super::{
    repro,
    single_player::{GameData, GameSource, Layers, LiveData},
};
use crate::scenario_script::Scenario;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

/// A standalone scenario and its content identity.
#[derive(Clone, Debug)]
pub struct Case {
    pub family: &'static str,
    pub name: String,
    pub text: String,
}
/// Generated cases and invalid-content skip counts, before filtering.
#[derive(Default)]
pub struct Generated {
    pub cases: Vec<Case>,
    pub skips: BTreeMap<String, usize>,
}
impl Generated {
    pub(super) fn skip(&mut self, reason: &str) {
        *self.skips.entry(reason.into()).or_default() += 1;
    }
    pub(super) fn add(
        &mut self,
        family: &'static str,
        name: String,
        header: &str,
        steps: &str,
        end: u32,
    ) -> Result<()> {
        let text = format!("scenario 2\ntick_rate 60\nname {family}-{name}\nseed 7\nend {end}\n{header}\n{steps}\nat {end} checkpoint\n");
        Scenario::parse(&text).map_err(|e| anyhow!("generator {family}/{name}: {e}"))?;
        self.cases.push(Case { family, name, text });
        Ok(())
    }
}
/// Sweep CLI options. A mod is an explicit single layer over a native base.
#[derive(Debug, Default)]
pub struct Options {
    pub family: String,
    pub filter: Option<String>,
    pub jobs: usize,
    pub out: PathBuf,
    pub game_dir: Option<PathBuf>,
    pub native: Option<PathBuf>,
    pub mod_name: Option<String>,
    pub save_failures: Option<PathBuf>,
}
/// Parse without loading data; invalid jobs, families and mod/source combinations fail early.
pub fn parse_args(args: &[String]) -> Result<Options> {
    let mut it = args.iter();
    let mut o = Options { family: it.next().context("sweep FAMILY [--filter TEXT] [--jobs N] [--out DIR] [--game-dir DIR | --native DIR [--mod NAME]] [--save-failures DIR]")?.clone(), jobs: 1, out: "sweep-out".into(), ..Options::default() };
    if ![
        "skills", "npcs", "areas", "items", "panels", "anims", "doors", "all",
    ]
    .contains(&o.family.as_str())
    {
        bail!("unknown sweep family {}", o.family);
    }
    while let Some(flag) = it.next() {
        let value = it.next().with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--filter" => o.filter = Some(value.clone()),
            "--jobs" => {
                o.jobs = value.parse().context("--jobs must be positive")?;
                if o.jobs == 0 {
                    bail!("--jobs must be positive");
                }
            }
            "--out" => o.out = value.into(),
            "--game-dir" => o.game_dir = Some(value.into()),
            "--native" => o.native = Some(value.into()),
            "--mod" => {
                if value.is_empty()
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    bail!("--mod needs a single mod folder name");
                }
                o.mod_name = Some(value.clone());
            }
            "--save-failures" => o.save_failures = Some(value.into()),
            _ => bail!("unknown sweep option {flag}"),
        }
    }
    if o.native.is_some() && o.game_dir.is_some() {
        bail!("choose --native or --game-dir");
    }
    if o.mod_name.is_some() && o.native.is_none() {
        bail!("--mod NAME requires --native DIR (converted base and mods)");
    }
    Ok(o)
}
/// Generate from this game's loaded, fixed-up tables.
pub fn generate(live: &LiveData, family: &str) -> Result<Generated> {
    let mut g = Generated::default();
    if family == "skills" || family == "all" {
        skills::generate(live, &mut g)?;
    }
    if family == "npcs" || family == "all" {
        npcs::generate(live, &mut g)?;
    }
    if family == "areas" || family == "all" {
        areas::generate(live, &mut g)?;
    }
    if family == "items" || family == "all" {
        items::generate(live, &mut g)?;
    }
    if family == "panels" || family == "all" {
        panels::generate(live, &mut g)?;
    }
    if family == "anims" || family == "all" {
        anims::generate(live, &mut g)?;
    }
    if family == "doors" || family == "all" {
        doors::generate(live, &mut g)?;
    }
    Ok(g)
}

fn step_signature(step: &Value) -> String {
    let kind = step["kind"].as_str().unwrap_or("step");
    let text = step["text"].as_str().unwrap_or("");
    if kind == "invariant" {
        return text.into();
    }
    let text = if kind == "check" {
        text.split_once(" check ").map_or(text, |(_, c)| c)
    } else {
        text
    };
    format!("{kind}:{text}")
}

/// Stable failure signatures; the same signature may name many content cases.
pub fn failures(report: &Value) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for v in report["violations"].as_array().into_iter().flatten() {
        if let Some(sig) = v["signature"].as_str() {
            found.insert(sig.into(), v["detail"].as_str().unwrap_or("").into());
        }
    }
    for step in report["steps"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["ok"] == false)
    {
        let kind = step["kind"].as_str().unwrap_or("step");
        if kind == "invariant" {
            continue;
        }
        found.insert(
            step_signature(step),
            step["result"].as_str().unwrap_or("").into(),
        );
    }
    if found.is_empty() && report["exit"] != 0 {
        found.insert(
            "runner:error".into(),
            report["error"]
                .as_str()
                .or(report["run_error"].as_str())
                .unwrap_or("run failed")
                .into(),
        );
    }
    found
}
/// The run-time skip of a case whose `goto` searched every room of its
/// level without finding the target: this seed's map does not place it
/// (content the case cannot reach), not a game failure.
const NOT_ON_MAP: &str = "target-not-on-map";

fn target_not_on_map(report: &Value) -> bool {
    report["steps"].as_array().into_iter().flatten().any(|s| {
        s["kind"] == "poke"
            && s["text"].as_str().is_some_and(|t| t.starts_with("goto "))
            && s["result"]
                .as_str()
                .is_some_and(|r| r.contains(d2_sim::poke::GOTO_NONE_IN_LEVEL))
    })
}

/// Execute a generated selection using shared immutable data and fresh games.
pub fn run(data: GameData, options: &Options) -> Result<Value> {
    if options.jobs == 0 {
        bail!("sweep needs positive jobs");
    }
    let began = Instant::now();
    let GameData::Live(live) = &data;
    let mut generated = generate(live, &options.family)?;
    let before = generated.cases.len();
    generated.cases.retain(|c| {
        options
            .filter
            .as_ref()
            .is_none_or(|f| format!("{}/{}", c.family, c.name).contains(f))
    });
    std::fs::create_dir_all(options.out.join("scenarios"))?;
    let mut paths = Vec::new();
    for c in &generated.cases {
        let path = options
            .out
            .join("scenarios")
            .join(format!("{}-{}.scn", c.family, c.name));
        std::fs::write(&path, &c.text)?;
        paths.push(path);
    }
    let opts = repro::RunOpts {
        out: options.out.join("runs"),
        keep_going: true,
        ..Default::default()
    };
    let batch = if paths.is_empty() {
        let empty = json!({"format":"batch 1","jobs":0,"scenarios":[],"exit":0,"totals":{"scenarios":0,"passed":0,"failed":0,"wall_ms":0,"scenario_ms":0},"slowest":[]});
        std::fs::create_dir_all(&opts.out)?;
        std::fs::write(
            opts.out.join("batch.json"),
            format!("{}\n", serde_json::to_string_pretty(&empty)?),
        )?;
        empty
    } else {
        repro::batch::run_batch_with_data(&paths, options.jobs, &opts, Some(data))?
    };
    let mut groups: BTreeMap<String, Value> = BTreeMap::new();
    let mut rows = Vec::new();
    let mut not_on_map = 0u64;
    for (i, c) in generated.cases.iter().enumerate() {
        let row = &batch["scenarios"][i];
        let report: Value = serde_json::from_slice(&std::fs::read(
            opts.out.join(format!("{:04}/repro.json", i + 1)),
        )?)?;
        // Only a plain scenario failure can be a skip; a crash, timeout or
        // invalid run keeps its exit code and stays a failure.
        if row["exit"] == repro::exit::FAIL && target_not_on_map(&report) {
            *generated
                .skips
                .entry(format!("{}:{NOT_ON_MAP}", c.family))
                .or_default() += 1;
            not_on_map += 1;
            rows.push(json!({"family":c.family,"case":c.name,"exit":row["exit"],"wall_ms":row["wall_ms"],"out":row["out"],"skipped":NOT_ON_MAP,"first_violation":null,"failures":{}}));
            continue;
        }
        let failures = failures(&report);
        for (sig, detail) in &failures {
            let group = groups
                .entry(sig.clone())
                .or_insert_with(|| json!({"status":"Unverified", "detail":detail,"cases":[]}));
            group["cases"]
                .as_array_mut()
                .expect("cases array")
                .push(json!(format!("{}/{}", c.family, c.name)));
        }
        if !failures.is_empty() {
            if let Some(dir) = &options.save_failures {
                std::fs::create_dir_all(dir)?;
                std::fs::write(dir.join(format!("{}-{}.scn", c.family, c.name)), &c.text)?;
            }
        }
        rows.push(json!({"family":c.family,"case":c.name,"exit":row["exit"],"wall_ms":row["wall_ms"],"out":row["out"],"first_violation":report["first_failure"].as_object().map(|_| step_signature(&report["first_failure"])).or_else(|| failures.keys().next().cloned()),"failures":failures}));
    }
    // Skipped cases leave the failed count. The exit is 0 only when every
    // case that did not pass is a skip; otherwise the batch's own exit.
    let mut totals = batch["totals"].clone();
    let failed = totals["failed"]
        .as_u64()
        .unwrap_or(0)
        .saturating_sub(not_on_map);
    totals["failed"] = json!(failed);
    totals["skipped"] = json!(not_on_map);
    let real_failure = rows
        .iter()
        .any(|r| r["skipped"].is_null() && r["exit"] != 0);
    let exit = if real_failure {
        batch["exit"].clone()
    } else {
        json!(0)
    };
    let report = json!({"format":"sweep 1","family":options.family,"mod":options.mod_name,"tick_rate":60,"jobs":options.jobs,"exit":exit,"skips":generated.skips,"filtered":before-generated.cases.len(),"totals":totals,"wall_ms":began.elapsed().as_millis() as u64,"cases":rows,"failures":groups});
    std::fs::write(
        options.out.join("sweep.json"),
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    Ok(report)
}
/// Headless command entry point.
pub fn main(args: &[String]) -> u8 {
    let result = (|| -> Result<Value> {
        let o = parse_args(args)?;
        let source = if let Some(dir) = &o.native {
            GameSource::Native(
                dir.clone(),
                Layers::List(o.mod_name.iter().cloned().collect()),
            )
        } else {
            GameSource::Archives(
                o.game_dir
                    .clone()
                    .or_else(|| std::env::var_os("D2_GAME_DIR").map(PathBuf::from))
                    .context("give --game-dir or set D2_GAME_DIR")?,
            )
        };
        run(GameData::open(&source)?, &o)
    })();
    match result {
        Ok(r) => {
            println!("family\tcase\tverdict\tfirst violation");
            for c in r["cases"].as_array().into_iter().flatten() {
                println!(
                    "{}\t{}\t{}\t{}",
                    c["family"].as_str().unwrap_or(""),
                    c["case"].as_str().unwrap_or(""),
                    if c["skipped"].is_string() {
                        "skip"
                    } else if c["exit"] == 0 {
                        "pass"
                    } else {
                        "fail"
                    },
                    c["first_violation"].as_str().unwrap_or("")
                );
            }
            println!(
                "sweep: {} skips={} wall_ms={}",
                r["totals"], r["skips"], r["wall_ms"]
            );
            r["exit"].as_u64().unwrap_or(1) as u8
        }
        Err(e) => {
            eprintln!("sweep: {e:#}");
            repro::exit::INVALID
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a `goto` that searched its whole level is a skip; a goto that
    /// failed otherwise, or any other failed step, stays a failure.
    #[test]
    fn only_a_goto_that_searched_its_whole_level_is_a_skip() {
        let step = |kind: &str, text: &str, result: &str| json!({"steps":[{"kind":kind,"text":text,"ok":false,"result":result}]});
        assert!(target_not_on_map(&step(
            "poke",
            "goto preset 26 2:23",
            "failed (none in level 26)"
        )));
        assert!(!target_not_on_map(&step(
            "poke",
            "goto preset 26 2:23",
            "failed"
        )));
        assert!(!target_not_on_map(&step(
            "poke",
            "warp 26",
            "failed (none in level 26)"
        )));
        assert!(!target_not_on_map(&step(
            "check",
            "near(object, 23).mode_id == 2",
            "never held"
        )));
    }
}
