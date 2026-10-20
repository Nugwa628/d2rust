//! JSON verdict for reference-free frame, sequence and optional T9 layout checks,
//! plus the flicker, hover-highlight and text-scale probes (`img_diff::probe`).
use anyhow::{bail, Context, Result};
use img_diff::check::{frame_rules, sequence_rules, ui_rules, Frame};
use img_diff::probe::{flicker_rules, highlight_rules, text_scale_rules};
use img_diff::Rect;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

fn load(path: &Path) -> Result<image::RgbaImage> {
    Ok(image::open(path)
        .with_context(|| path.display().to_string())?
        .to_rgba8())
}
fn parse_rect(value: &str, flag: &str) -> Result<Rect> {
    let n = value
        .split(',')
        .map(str::parse)
        .collect::<Result<Vec<u32>, _>>()
        .with_context(|| format!("{flag} takes X,Y,W,H"))?;
    let [x, y, w, h] = n[..] else {
        bail!("{flag} takes X,Y,W,H")
    };
    Ok(Rect { x, y, w, h })
}
fn run() -> Result<Value> {
    let mut args = std::env::args().skip(1);
    let source=PathBuf::from(args.next().context("usage: img-check PNG|DIR|MANIFEST.json [--ui FILE] [--tol N] [--jump N] [--missing R,G,B] [--flicker MIN_PIXELS [--flicker-delta N]] [--highlight BASE.png... [--min-pixels N] [--ignore X,Y,W,H]...] [--text-base BASE.png --world X,Y,W,H] [--out FILE]")?);
    let (mut ui, mut out) = (None, None);
    let (mut flicker, mut flicker_delta, mut min_pixels) = (None::<usize>, 16u8, 50usize);
    let (mut highlight, mut text_base, mut world) =
        (Vec::<PathBuf>::new(), None::<PathBuf>, None::<Rect>);
    let mut ignore = Vec::new();
    let (mut tol, mut jump, mut missing) = (0u8, 8.0f64, [255u8, 0, 255]);
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .with_context(|| format!("{arg} needs a value"))?;
        match arg.as_str() {
            "--ui" => ui = Some(PathBuf::from(value)),
            "--out" => out = Some(PathBuf::from(value)),
            "--tol" => tol = value.parse()?,
            "--flicker" => flicker = Some(value.parse().context("--flicker takes a pixel count")?),
            "--flicker-delta" => {
                flicker_delta = value.parse().context("--flicker-delta takes 0-255")?
            }
            "--highlight" => highlight.push(PathBuf::from(value)),
            "--ignore" => ignore.push(parse_rect(&value, "--ignore")?),
            "--min-pixels" => min_pixels = value.parse().context("--min-pixels takes a count")?,
            "--text-base" => text_base = Some(PathBuf::from(value)),
            "--world" => world = Some(parse_rect(&value, "--world")?),
            "--jump" => {
                jump = value.parse()?;
                if !jump.is_finite() || jump < 0.0 {
                    bail!("invalid jump");
                }
            }
            "--missing" => {
                missing = value
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<Vec<u8>, _>>()?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("expected R,G,B"))?
            }
            _ => bail!("unknown option {arg}"),
        }
    }
    let mut frames = Vec::new();
    let mut failures = Vec::new();
    let sequence = source.extension().is_some_and(|e| e == "json");
    if sequence {
        let manifest: Value = serde_json::from_slice(&fs::read(&source)?)?;
        for f in manifest["frames"]
            .as_array()
            .context("manifest needs frames array")?
        {
            let name = f["path"].as_str().context("frame needs path")?.to_string();
            frames.push(Frame {
                image: load(&source.parent().unwrap_or(Path::new(".")).join(&name))?,
                name,
                tick: f["tick"].as_u64().context("frame needs tick")?,
                phase: f["phase"].as_u64().context("frame needs phase (ms)")?,
            });
        }
    } else {
        let mut paths = if source.is_dir() {
            fs::read_dir(&source)?
                .map(|e| e.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            vec![source]
        };
        paths.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")));
        paths.sort();
        for path in paths {
            frames.push(Frame {
                name: path.display().to_string(),
                tick: 0,
                phase: 0,
                image: load(&path)?,
            });
        }
    }
    if frames.is_empty() {
        bail!("no frames");
    }
    for f in &frames {
        for mut v in frame_rules(&f.image, missing) {
            v["frame"] = json!(f.name);
            failures.push(v);
        }
    }
    if sequence {
        failures.extend(sequence_rules(&frames, tol, jump)?);
    }
    let mut metrics = serde_json::Map::new();
    if let Some(min) = flicker {
        // Display order: the manifest's order, or sorted file names.
        failures.extend(flicker_rules(&frames, tol, flicker_delta, min)?);
    }
    if !highlight.is_empty() {
        let bases = highlight
            .iter()
            .map(|p| load(p))
            .collect::<Result<Vec<_>>>()?;
        let (m, v) = highlight_rules(&bases, &frames[0].image, min_pixels, &ignore)?;
        metrics.insert("highlight".into(), m);
        failures.extend(v);
    }
    match (text_base, world) {
        (Some(base), Some(world)) => {
            let (m, v) = text_scale_rules(&load(&base)?, &frames[0].image, world)?;
            metrics.insert("text_scale".into(), m);
            failures.extend(v);
        }
        (None, None) => {}
        _ => bail!("--text-base and --world go together"),
    }
    if let Some(ui) = ui {
        failures.extend(ui_rules(
            &serde_json::from_slice(&fs::read(ui)?)?,
            frames[0].image.width(),
            frames[0].image.height(),
        )?);
    }
    let mut verdict = json!({"format":"img-check 1","pass":failures.is_empty(),"frames":frames.len(),"violations":failures});
    if !metrics.is_empty() {
        verdict["metrics"] = Value::Object(metrics);
    }
    if let Some(out) = out {
        fs::write(out, serde_json::to_string_pretty(&verdict)? + "\n")?;
    }
    Ok(verdict)
}
fn main() -> ExitCode {
    match run() {
        Ok(v) => {
            println!("{v}");
            ExitCode::from(u8::from(v["pass"] != true))
        }
        Err(e) => {
            eprintln!("img-check: {e:#}");
            ExitCode::from(2)
        }
    }
}
