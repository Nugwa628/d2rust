//! Case kind `scene`: a captured reference frame ([`capture`]) becomes a pass/fail.
//!
//! `check = "compare"`: per selected frame, the PNG is first checked against its record (size,
//! `index_sha256`, `palette_sha256`), then a [`SceneSource`] turns the recorded state into
//! compositor inputs. The CPU reference ([`scene::compose`]) must equal the capture's index bytes
//! exactly, the scene palette must equal the capture's 768 palette bytes, and the GPU half
//! ([`GpuCompositor`]) must equal both. The report gives the differing byte count and the first
//! difference. `--perturb N` corrupts N pixels of each capture after its integrity check, so
//! every compared frame must fail with exactly N: a deliberate perturbation that proves the
//! comparison can fail.
//!
//! `check = "stability"`: every PNG is re-hashed against its record, frames with
//! `clear_counter > 0` must be all index 0, and frames with equal state keys must have equal
//! index hashes, with at least two keys seen at least twice. That proves the capture point and
//! the state key are complete: equal recorded state, equal frame. `--perturb N` flips one byte in
//! each of the first N saved frames, so exactly N re-hashes differ.
//!
//! The source of the D2RUST scene is the seam [`SceneSource`]. The wired one is
//! [`scene_source::WorldScene`]: the recorded camera is checked against the camera rules (view
//! rectangle, `shiftX`, tile and unit origin; a difference fails the frame), then the recorded
//! world goes through `world_view::build` and the view rules (`rules::OriginalView`). What the
//! recording does not hold (units, map, UI, assets) is a seam ([`SceneOutcome::Seam`],
//! [`scene_source::RECORDER_GAP`]): such frames report [`Status::SceneNotWired`], meaning the
//! capture's own checks and the camera check ran but the pixel comparison did not. Never a pass.

use std::path::{Path, PathBuf};

use super::capture::{self, Captured, Frame, Image, Raw};
use super::case::{CaptureCheck, RawRef, SceneCase};
use super::{
    compare, compare_indices, perturb_indices, Built, CaseReport, GpuCompositor, GpuJob,
    GpuOutcome, Status,
};
use crate::scene::{self, Rect};

pub mod scene_source;

#[cfg(test)]
mod scene_tests;

/// The repository root (case paths are relative to it).
pub fn repo_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .map_or_else(|| manifest.join("..").join(".."), Path::to_path_buf)
}

/// Default directory of the capture cases (`d2-client verify --cases`).
pub fn capture_case_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capture-cases")
}

/// One frame to render: the record, its state and the initial framebuffer (`previous`).
///
/// The initial framebuffer is the captured frame `seq − 1` of a recording made with `--every 1`,
/// i.e. the previous in-game draw: a frame only overwrites the pixels it draws, so every other
/// pixel keeps the previous frame's content. `None` otherwise: the frame cannot be composed.
pub struct SceneJob<'a> {
    pub case: &'a str,
    pub frame: &'a Frame,
    pub captured: &'a Captured,
    pub previous: Option<&'a Image>,
}

/// What a scene source returned.
pub enum SceneOutcome {
    /// Compositor inputs for the frame; `view` must be the whole frame.
    Built(Box<Built>),
    /// No source of the D2RUST scene is wired into this build.
    NotWired,
    /// The source stops at a seam: the recording lacks what the pipeline
    /// needs (the message names it). Reported as not wired, never a pass.
    Seam(String),
    /// The recorded state disagrees with a rule before any pixel is
    /// composed (one line per difference): fails the frame.
    Differs(Vec<String>),
    /// The source could not build the frame (a rule without an answer, a
    /// missing asset): fails the case.
    Error(String),
}

/// The seam from a recorded per-frame state to the D2RUST scene.
pub trait SceneSource {
    fn scene(&mut self, job: &SceneJob<'_>) -> SceneOutcome;

    /// Report lines of the last [`SceneSource::scene`] call (checks that
    /// passed), taken once.
    fn take_notes(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// The scene source until the world view is wired to recorded states.
pub struct SceneNotWired;

impl SceneSource for SceneNotWired {
    fn scene(&mut self, _job: &SceneJob<'_>) -> SceneOutcome {
        SceneOutcome::NotWired
    }
}

/// Resolves the case's raw file and image directory under `root`.
pub fn resolve(root: &Path, case: &SceneCase) -> Result<(PathBuf, PathBuf), String> {
    let raw = match &case.raw {
        RawRef::Path(p) => root.join(p),
        RawRef::Latest => {
            let dir = root.join("traces").join("raw");
            let entries =
                std::fs::read_dir(&dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
            let mut newest: Option<PathBuf> = None;
            for entry in entries {
                let path = entry.map_err(|e| e.to_string())?.path();
                let is_frames = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("-frames.jsonl"));
                if is_frames && newest.as_ref().is_none_or(|n| path > *n) {
                    newest = Some(path);
                }
            }
            newest.ok_or_else(|| format!("no *-frames.jsonl in {}", dir.display()))?
        }
    };
    let images = match &case.images {
        Some(p) => root.join(p),
        None => {
            let stamp = raw
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix("-frames.jsonl"))
                .ok_or_else(|| {
                    format!(
                        "{}: not named <stamp>-frames.jsonl; give `images`",
                        raw.display()
                    )
                })?;
            root.join("game").join("captures").join(stamp)
        }
    };
    Ok((raw, images))
}

/// Lines printed per kind of per-frame finding before the rest is counted.
const SHOWN: usize = 10;

/// Runs a scene case with paths under `root`.
pub fn run_capture(
    name: &str,
    case: &SceneCase,
    root: &Path,
    perturb_n: usize,
    source: &mut dyn SceneSource,
    gpu: &mut dyn GpuCompositor,
) -> CaseReport {
    let mut report = CaseReport {
        name: name.to_owned(),
        kind: "scene",
        lines: Vec::new(),
        status: Status::Pass,
    };
    let loaded = resolve(root, case).and_then(|(raw_path, images)| {
        let raw = capture::read_raw(&raw_path).map_err(|e| e.to_string())?;
        Ok((raw_path, images, raw))
    });
    let (raw_path, images, raw) = match loaded {
        Ok(l) => l,
        Err(e) => {
            report.status = Status::Error(e);
            return report;
        }
    };
    let refused = raw.frames.iter().filter(|f| f.captured().is_none()).count();
    report.lines.push(format!(
        "raw {} ({}): {} frames ({refused} refused), {} ticks; {}; Game.exe {}",
        raw_path.display(),
        match raw.format {
            capture::RawFormat::Raw1 => capture::FORMAT_1,
            capture::RawFormat::Raw2 => capture::FORMAT_2,
        },
        raw.frames.len(),
        raw.ticks.len(),
        raw.header.tool,
        raw.header.game_exe_sha256
    ));
    for note in &raw.footer.notes {
        report.lines.push(format!("note: {note}"));
    }
    // Frames are numbered by the recorder, not by game tick: two frames on one tick mean the game
    // paused or skipped a tick. Worth flagging, not an error.
    let repeated = capture::repeated_ticks(&raw.frames);
    if !repeated.is_empty() {
        report.lines.push(format!(
            "flag: {} ticks carry more than one frame",
            repeated.len()
        ));
        for (tick, seqs) in repeated.iter().take(SHOWN) {
            report.lines.push(format!("  tick {tick}: frames {seqs:?}"));
        }
    }
    if perturb_n > 0 {
        report.lines.push(format!(
            "debug: --perturb {perturb_n}; verify must FAIL with exactly {perturb_n}"
        ));
    }
    report.status = match case.check {
        CaptureCheck::Stability => stability(&raw, &images, perturb_n, &mut report.lines),
        CaptureCheck::Compare => compare_frames(
            name,
            case,
            &raw,
            &images,
            perturb_n,
            source,
            gpu,
            &mut report.lines,
        ),
    };
    report
}

fn stability(raw: &Raw, images: &Path, perturb_n: usize, lines: &mut Vec<String>) -> Status {
    let mut hashed = Vec::new();
    let (mut saved, mut rehash_bad, mut zero_bad) = (0usize, Vec::new(), Vec::new());
    for f in &raw.frames {
        let Some(c) = f.captured() else { continue };
        let mut hash = c.index_sha256.clone();
        if c.image.is_some() {
            let (_, mut image) = match capture::read_image(images, c) {
                Ok(i) => i,
                Err(e) => return Status::Error(e),
            };
            if (i64::from(image.width), i64::from(image.height)) != (f.w.into(), f.h.into()) {
                return Status::Error(format!(
                    "frame {}: PNG is {}x{}, the record {}x{}",
                    f.seq, image.width, image.height, f.w, f.h
                ));
            }
            saved += 1;
            // The deliberate perturbation: one byte of each of the first N saved frames.
            if saved <= perturb_n {
                image.indices[0] ^= 0x80;
            }
            hash = capture::sha256_hex(&image.indices);
            if hash != c.index_sha256 {
                rehash_bad.push(f.seq);
            }
            if c.state.clear_counter > 0 && image.indices.iter().any(|&b| b != 0) {
                zero_bad.push(f.seq);
            }
        }
        hashed.push((f.seq, c, hash));
    }
    if perturb_n > saved {
        return Status::Error(format!(
            "--perturb {perturb_n} exceeds the {saved} saved frames"
        ));
    }
    let captured = hashed.len();
    let st = capture::stability(hashed);
    lines.push(format!("{captured} captured frames, {saved} with PNG"));
    lines.push(format!(
        "re-hash: {} of {saved} frames differ from their record",
        rehash_bad.len()
    ));
    for q in rehash_bad.iter().take(SHOWN) {
        lines.push(format!(
            "  frame {q}: PNG index hash differs from index_sha256"
        ));
    }
    if !zero_bad.is_empty() {
        lines.push(format!(
            "clear: {} frames with clear_counter > 0 are not all index 0, frames {:?}",
            zero_bad.len(),
            &zero_bad[..zero_bad.len().min(SHOWN)]
        ));
    }
    lines.push(format!("stability: {st}"));
    for g in st.differing().take(SHOWN) {
        lines.push(format!(
            "  key {:?}: {} frames, {} different images, frames {:?}",
            g.key,
            g.seqs.len(),
            g.hashes.len(),
            &g.seqs[..g.seqs.len().min(SHOWN)]
        ));
    }
    let mut why = Vec::new();
    if !rehash_bad.is_empty() {
        why.push(format!("{} frames re-hash differently", rehash_bad.len()));
    }
    if !zero_bad.is_empty() {
        why.push(format!("{} cleared frames are not all 0", zero_bad.len()));
    }
    if st.differing().next().is_some() {
        why.push(format!(
            "{} state groups with differing frames (the capture point or the state key misses an input)",
            st.differing().count()
        ));
    }
    if st.repeated() < 2 {
        why.push(format!(
            "{} state keys seen at least twice, stability needs 2",
            st.repeated()
        ));
    }
    if why.is_empty() {
        Status::Pass
    } else {
        Status::Fail(why.join("; "))
    }
}

/// The verdict of one compared frame, worst first.
fn worse(a: Status, b: Status) -> Status {
    fn rank(s: &Status) -> u8 {
        match s {
            Status::Error(_) => 5,
            Status::Fail(_) => 4,
            Status::SceneNotWired => 3,
            Status::NoAdapter(_) => 2,
            Status::GpuNotWired => 1,
            Status::Pass => 0,
        }
    }
    if rank(&b) > rank(&a) {
        b
    } else {
        a
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_frames(
    name: &str,
    case: &SceneCase,
    raw: &Raw,
    images: &Path,
    perturb_n: usize,
    source: &mut dyn SceneSource,
    gpu: &mut dyn GpuCompositor,
    lines: &mut Vec<String>,
) -> Status {
    let captured: Vec<(&Frame, &Captured)> = raw
        .frames
        .iter()
        .filter_map(|f| f.captured().map(|c| (f, c)))
        .collect();
    // Default: every captured frame but the first, whose uncleared rows
    // come from frames drawn before the recording started.
    let selected: Vec<usize> = if case.seqs.is_empty() {
        (1..captured.len()).collect()
    } else {
        let mut sel = Vec::new();
        for q in &case.seqs {
            match captured.iter().position(|(f, _)| f.seq == *q) {
                Some(i) => sel.push(i),
                None => return Status::Error(format!("frame {q} is not a captured frame")),
            }
        }
        sel
    };
    // The initial framebuffer is frame seq − 1, which is the previous draw only when every
    // in-game draw was captured.
    let every_draw = raw.capture.as_ref().is_some_and(|m| m.every_draw());
    if !every_draw {
        lines.push("initial framebuffer: none (the recording is not --every 1)".into());
    }
    if selected.is_empty() {
        return Status::Error("no frame to compare (a capture needs at least two)".into());
    }
    lines.push(format!("{} frames selected", selected.len()));
    let mut status = Status::Pass;
    let (mut passed, mut failed, mut not_wired, mut shown) = (0usize, 0usize, 0usize, 0usize);
    let mut previous: Option<(u32, Image)> = None;
    for (i, &k) in selected.iter().enumerate() {
        let (frame, c) = captured[k];
        let image = match capture::read_image(images, c).and_then(|(_, img)| {
            img.check(frame, c)
                .map_err(|e| format!("frame {}: {e}", frame.seq))?;
            Ok(img)
        }) {
            Ok(img) => img,
            Err(e) => return Status::Error(e),
        };
        // Frame seq − 1, when it was captured: from the run before when
        // consecutive, else read on demand.
        let want = frame.seq.checked_sub(1).filter(|_| every_draw);
        let prev = match (previous.take(), want) {
            (Some((q, img)), Some(p)) if q == p => Some(img),
            (_, Some(p)) => match captured.iter().find(|(pf, _)| pf.seq == p) {
                Some((pf, pc)) => match capture::read_image(images, pc).and_then(|(_, img)| {
                    img.check(pf, pc).map_err(|e| format!("frame {p}: {e}"))?;
                    Ok(img)
                }) {
                    Ok(img) => Some(img),
                    Err(e) => return Status::Error(e),
                },
                None => None,
            },
            (_, None) => None,
        };
        let job = SceneJob {
            case: name,
            frame,
            captured: c,
            previous: prev.as_ref(),
        };
        let mut expected = image.indices.clone();
        if let Err(e) = perturb_indices(&mut expected, perturb_n) {
            return Status::Error(e);
        }
        let verdict = compare_one(&job, &image, &expected, source, gpu);
        let (frame_status, frame_lines) = verdict;
        match &frame_status {
            Status::Pass | Status::GpuNotWired | Status::NoAdapter(_) => passed += 1,
            Status::SceneNotWired => not_wired += 1,
            Status::Fail(_) | Status::Error(_) => failed += 1,
        }
        let show = !frame_lines.is_empty()
            && (matches!(frame_status, Status::Fail(_) | Status::Error(_)) || i == 0);
        if show && shown < SHOWN {
            shown += 1;
            lines.push(format!(
                "frame {} (draw {}, tick {}, raw line {})",
                frame.seq,
                c.draw,
                frame.tick.map_or("none".into(), |t| t.to_string()),
                frame.line
            ));
            lines.extend(frame_lines.into_iter().map(|l| format!("  {l}")));
        }
        status = worse(status, frame_status);
        previous = Some((frame.seq, image));
    }
    lines.push(format!(
        "frames: {passed} match, {failed} differ or fail, {not_wired} scene not wired"
    ));
    match status {
        Status::Fail(_) => Status::Fail(format!("{failed} of {} frames", selected.len())),
        s => s,
    }
}

/// One frame: build, CPU compare, palette compare, GPU compare.
fn compare_one(
    job: &SceneJob<'_>,
    image: &Image,
    expected: &[u8],
    source: &mut dyn SceneSource,
    gpu: &mut dyn GpuCompositor,
) -> (Status, Vec<String>) {
    let mut lines = Vec::new();
    let outcome = source.scene(job);
    lines.extend(source.take_notes());
    let built = match outcome {
        SceneOutcome::Built(b) => b,
        SceneOutcome::NotWired => return (Status::SceneNotWired, lines),
        SceneOutcome::Seam(s) => {
            lines.push(format!("scene: seam: {s}"));
            return (Status::SceneNotWired, lines);
        }
        SceneOutcome::Differs(d) => {
            let n = d.len();
            lines.extend(d);
            return (
                Status::Fail(format!("{n} recorded camera values differ")),
                lines,
            );
        }
        SceneOutcome::Error(e) => return (Status::Error(format!("scene: {e}")), lines),
    };
    let view = Rect::new(0, 0, image.width, image.height);
    if built.view != view {
        return (
            Status::Error(format!(
                "scene view {}x{} at {},{}; the capture is the whole {}x{} frame",
                built.view.width,
                built.view.height,
                built.view.x,
                built.view.y,
                view.width,
                view.height
            )),
            lines,
        );
    }
    let reference = match scene::compose(&built.items, &built.frames, &built.maps, view) {
        Ok(r) => r,
        Err(e) => return (Status::Error(format!("CPU compose: {e}")), lines),
    };
    let cpu = match compare_indices(&reference, expected, view) {
        Ok(m) => m,
        Err(e) => return (Status::Error(e), lines),
    };
    lines.push(format!("{} items; CPU: {cpu}", built.items.len()));
    let ours = capture::palette_bytes(&built.palette);
    let pal_diff: Vec<usize> = (0..256)
        .filter(|&i| ours[3 * i..3 * i + 3] != image.palette[3 * i..3 * i + 3])
        .collect();
    lines.push(match pal_diff.first() {
        None => "palette: 0 of 256 entries differ".to_owned(),
        Some(&i) => format!(
            "palette: {} of 256 entries differ, first index {i}: expected {:?}, got {:?}",
            pal_diff.len(),
            &image.palette[3 * i..3 * i + 3],
            &ours[3 * i..3 * i + 3]
        ),
    });
    let cpu_failed = cpu.mismatched > 0 || !pal_diff.is_empty();
    let bins = match scene::bin(&built.items, &built.frames, &built.maps, view) {
        Ok(b) => b,
        Err(e) => return (Status::Error(format!("bin: {e}")), lines),
    };
    let gpu_job = GpuJob {
        case: job.case,
        items: &built.items,
        bins: &bins,
        frames: &built.frames,
        maps: &built.maps,
        palette: &built.palette,
        view,
    };
    let gpu_status = match gpu.compose(&gpu_job) {
        GpuOutcome::NotWired => Status::GpuNotWired,
        GpuOutcome::NoAdapter(e) => {
            lines.push(format!("GPU: no adapter: {e}"));
            Status::NoAdapter(e)
        }
        GpuOutcome::Error(e) => Status::Error(format!("GPU: {e}")),
        GpuOutcome::Image { indices, rgba } => {
            // The capture's RGBA: its indices through its own palette, alpha 255, the same
            // palette lookup presentation does.
            let mut want = Vec::with_capacity(expected.len() * 4);
            for &i in expected {
                let p = &image.palette[3 * usize::from(i)..3 * usize::from(i) + 3];
                want.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
            match compare_indices(&indices, expected, view)
                .and_then(|i| Ok((i, compare(&rgba, &want, view)?)))
            {
                Err(e) => Status::Error(format!("GPU: {e}")),
                Ok((i, m)) => {
                    lines.push(format!("GPU indices: {i}"));
                    lines.push(format!("GPU: {m}"));
                    if i.mismatched == 0 && m.mismatched == 0 {
                        Status::Pass
                    } else {
                        Status::Fail("GPU half".into())
                    }
                }
            }
        }
    };
    let status = match (cpu_failed, gpu_status) {
        (_, Status::Error(e)) => Status::Error(e),
        (true, Status::Fail(_)) => Status::Fail("CPU and GPU halves".into()),
        (true, _) => Status::Fail("CPU half".into()),
        (false, s) => s,
    };
    (status, lines)
}
