// Spec: specs/client/render-pipeline.md (A9, A10)
//! Headless CPU = GPU comparison of the compute compositor on the
//! synthetic cases (repo only, needs a GPU).
//!
//!   cargo run -p d2-client --example gpu_compare -- [--case NAME] [--perturb N]
//!
//! Prints one line per case. Exit status 0 only if every case has 0
//! differing bytes. `--perturb N` changes N bytes of each CPU reference
//! first (M08): every case must then FAIL with exactly N differing bytes.

use anyhow::{bail, Context, Result};
use d2_client::gpu_compositor::{harness, Gpu};

fn main() -> Result<()> {
    let mut perturb = 0usize;
    let mut only: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--perturb" => perturb = value()?.parse().context("--perturb")?,
            "--case" => only = Some(value()?),
            _ => bail!("usage: gpu_compare [--case NAME] [--perturb N]"),
        }
    }
    let cases: Vec<_> = harness::cases()
        .into_iter()
        .filter(|c| only.as_deref().is_none_or(|n| n == c.name))
        .collect();
    if cases.is_empty() {
        bail!("no case named {:?}", only.unwrap_or_default());
    }
    let (gpu, info) = Gpu::headless()?;
    println!(
        "adapter: {} ({:?}, {:?}, driver {})",
        info.name, info.backend, info.device_type, info.driver
    );
    if perturb > 0 {
        println!("debug: {perturb} bytes of each CPU reference corrupted; every case must FAIL with exactly {perturb}");
    }
    let mut failed = 0;
    for case in &cases {
        let report = harness::compare(&gpu, case, perturb)?;
        println!(
            "{} {report}",
            if report.is_match() { "PASS" } else { "FAIL" }
        );
        if !report.is_match() {
            failed += 1;
        }
    }
    if failed > 0 {
        bail!("{failed} of {} cases differ", cases.len());
    }
    println!("all {} cases: 0 differing bytes", cases.len());
    Ok(())
}
