//! Parallel headless batches. Each worker owns its loaded Session; report order
//! and artifact names follow the input list regardless of completion order.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Expand a directory recursively (sorted), or a UTF-8 list with one scenario
/// path per line, relative to the list. Blank lines and # comments are ignored.
pub fn files(input: &Path) -> Result<Vec<PathBuf>> {
    fn visit(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                visit(&path, out)?;
            } else if path.extension().is_some_and(|e| e == "scn") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if input.is_dir() {
        visit(input, &mut out)?;
        out.sort();
    } else {
        let base = input.parent().unwrap_or(Path::new("."));
        for line in std::fs::read_to_string(input)?.lines().map(str::trim) {
            if !line.is_empty() && !line.starts_with('#') {
                out.push(resolve_path(base, line));
            }
        }
    }
    if out.is_empty() {
        return Err(anyhow!("batch contains no scenarios"));
    }
    Ok(out)
}

/// Run all files, including duplicates. Invalid scenarios produce error rows
/// and repro.json artifacts, without discarding successful results.
pub fn run_batch(paths: &[PathBuf], jobs: usize, opts: &RunOpts) -> Result<Value> {
    run_batch_with_data(paths, jobs, opts, None)
}

/// Batch on a selected loaded install; each worker retains its own session.
pub fn run_batch_with_data(
    paths: &[PathBuf],
    jobs: usize,
    opts: &RunOpts,
    data: Option<GameData>,
) -> Result<Value> {
    if jobs == 0 || paths.is_empty() {
        return Err(anyhow!("batch needs scenarios and positive jobs"));
    }
    std::fs::create_dir_all(&opts.out)?;
    let wall = std::time::Instant::now();
    let next = AtomicUsize::new(0);
    let rows = Mutex::new(Vec::new());
    std::thread::scope(|scope| -> Result<()> {
        let mut workers = Vec::new();
        for _ in 0..jobs.min(paths.len()) {
            workers.push(scope.spawn(|| {
                // Load lazily so parse failures need no install.
                let mut session = data.clone().map(Session::from_data);
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(i) else { break };
                    let mut local = opts.clone();
                    local.file = path.display().to_string();
                    local.base_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
                    local.out = opts.out.join(format!("{:04}", i + 1));
                    let began = std::time::Instant::now();
                    let result = (|| -> Result<ReproReport, ReproError> {
                        let text = std::fs::read_to_string(path).map_err(|e| ReproError::Invalid(e.to_string()))?;
                        let scenario = Scenario::parse(&text).map_err(|e| ReproError::Invalid(e.to_string()))?;
                        validate(&scenario)?;
                        if session.is_none() { session = Some(Session::load(opts.game_dir.as_deref())?); }
                        session.as_mut().expect("loaded session").run(&scenario, &local)
                    })();
                    let (code, report) = match result {
                        Ok(r) => (r.exit, r.json),
                        Err(e) => {
                            let code = e.exit_code();
                            let report = json!({"format": FORMAT, "file": local.file, "exit": code,
                                "verdict": "error", "error": e.to_string(), "wall_ms": began.elapsed().as_millis() as u64});
                            std::fs::create_dir_all(&local.out)?;
                            std::fs::write(local.out.join("repro.json"), format!("{}\n", serde_json::to_string_pretty(&report)?))?;
                            (code, report)
                        }
                    };
                    let row = json!({"index": i + 1, "file": local.file, "out": local.out,
                        "scenario": report["scenario"], "exit": code, "verdict": report["verdict"],
                        "wall_ms": began.elapsed().as_millis() as u64, "error": report["error"]});
                    rows.lock().map_err(|_| anyhow!("batch result lock"))?.push(row);
                }
                Ok::<_, anyhow::Error>(())
            }));
        }
        for worker in workers {
            worker
                .join()
                .map_err(|_| anyhow!("batch worker panicked"))??;
        }
        Ok(())
    })?;
    let mut rows = rows
        .into_inner()
        .map_err(|_| anyhow!("batch result lock"))?;
    rows.sort_by_key(|r| r["index"].as_u64());
    let mut slowest = rows.clone();
    slowest.sort_by_key(|r| std::cmp::Reverse(r["wall_ms"].as_u64()));
    slowest.truncate(10);
    let passed = rows.iter().filter(|r| r["exit"] == exit::PASS).count();
    let code = rows
        .iter()
        .filter_map(|r| r["exit"].as_u64())
        .max()
        .unwrap_or(0);
    let report = json!({"format": "batch 1", "jobs": jobs.min(paths.len()), "exit": code,
        "totals": {"scenarios": rows.len(), "passed": passed, "failed": rows.len()-passed,
            "wall_ms": wall.elapsed().as_millis() as u64,
            "scenario_ms": rows.iter().filter_map(|r| r["wall_ms"].as_u64()).sum::<u64>()},
        "scenarios": rows, "slowest": slowest});
    std::fs::write(
        opts.out.join("batch.json"),
        format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    Ok(report)
}

pub(super) fn main(args: &CliArgs) -> u8 {
    let result = (|| -> Result<Value> {
        let paths = files(args.batch.as_deref().expect("batch argument"))?;
        let opts = RunOpts {
            out: args.out.clone().unwrap_or_else(|| "repro-out/batch".into()),
            game_dir: args.game_dir.clone(),
            keep_going: args.keep_going,
            queries: args
                .query
                .as_deref()
                .map(query::parse_names)
                .transpose()
                .map_err(|e| anyhow!(e))?
                .unwrap_or_default(),
            query_at: args.at.clone(),
            ..RunOpts::default()
        };
        run_batch_with_data(&paths, args.jobs.max(1), &opts, selected_data(args)?)
    })();
    match result {
        Ok(report) => {
            for row in report["scenarios"].as_array().into_iter().flatten() {
                println!(
                    "{}\t{}\t{} ms\t{}",
                    row["index"], row["verdict"], row["wall_ms"], row["file"]
                );
            }
            println!("batch: {}", report["totals"]);
            report["exit"].as_u64().unwrap_or(1) as u8
        }
        Err(e) => {
            eprintln!("batch: {e:#}");
            exit::INVALID
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_cli_requires_positive_jobs_and_one_source() {
        let parse = |a: &[&str]| parse_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(parse(&["--batch", "list", "--jobs", "4"]).unwrap().jobs, 4);
        for a in [
            vec!["--batch", "x", "--jobs", "0"],
            vec!["x", "--batch", "y"],
            vec!["x", "--jobs", "2"],
        ] {
            assert!(parse(&a).is_err());
        }
    }
    #[test]
    fn lists_keep_duplicates_and_invalid_cases_leave_reports() {
        let root = std::env::temp_dir().join(format!("batch-unit-{}", std::process::id()));
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("nested/z.scn"), "invalid").unwrap();
        std::fs::write(root.join("a.scn"), "invalid").unwrap();
        std::fs::write(
            root.join("list.txt"),
            "# cases\nnested/z.scn\n\na.scn\nnested/z.scn\n",
        )
        .unwrap();
        let listed = files(&root.join("list.txt")).unwrap();
        assert_eq!(
            listed,
            [
                root.join("nested/z.scn"),
                root.join("a.scn"),
                root.join("nested/z.scn")
            ]
        );
        assert_eq!(
            files(&root).unwrap(),
            [root.join("a.scn"), root.join("nested/z.scn")]
        );
        let report = run_batch(
            &listed,
            2,
            &RunOpts {
                out: root.join("out"),
                ..RunOpts::default()
            },
        )
        .unwrap();
        assert_eq!(report["exit"], exit::INVALID);
        assert_eq!(report["totals"]["failed"], 3);
        assert_eq!(report["scenarios"][0]["index"], 1);
        assert!(root.join("out/0003/repro.json").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
