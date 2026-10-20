//! The wired scene source on a synthetic 800 × 600 capture generated here
//! (never game pixels): the recorded camera checked against the camera rules,
//! a recorded world built through `world_view::build` and
//! `rules::OriginalView`, compared byte for byte with an expected frame
//! painted by hand from the placement rules (not from the pipeline), on the
//! CPU half and the GPU half; `--perturb N` reports exactly N; today's
//! recordings stop at the recorder seam.

use std::path::{Path, PathBuf};

use d2_formats::cof::{Cof, CofLayer};
use serde_json::{json, Value};

use super::scene_source::{
    camera_differences, recorded_camera, CaptureWorld, NotRecorded, RecordedScene, WorldAnswer,
    WorldScene, NO_DRAW_LOG, NO_INITIAL_FRAME, RECORDER_GAP,
};
use super::{run_capture, SceneJob};
use crate::assets::path::CanonicalPath;
use crate::bridge::world::ClientWorld;
use crate::bridge::{ClientUnit, UnitKey};
use crate::composite::{ComponentFrame, ComponentRequest, CompositeError, UnitParams};
use crate::frames::{FrameAnchor, FramePart, FrameSet, FrameSetKey, IndexFrame};
use crate::rules::{BlockRect, Camera, MapTile, TileList, UnitPosition, ViewSource};
use crate::scene::{self, BlendOp, DrawKey, ItemTag, Rect, ShadeChain};
use crate::ui::{ImageRequest, TextRequest};
use crate::verify::capture;
use crate::verify::case::{CaptureCheck, RawRef, SceneCase};
use crate::verify::{
    gpu, synthetic_palette, CaseReport, GpuCompositor, GpuJob, GpuOutcome, Status,
};
use crate::world_view::{TileDraw, UiRules, UiSprite, UnitPose, ViewAssets, ViewError, ViewRules};

const W: i32 = 800;
const H: i32 = 600;
const COF: &str = "data/global/tst/tst.cof";
const PLAYER: u32 = 1;
const OBJECT: u32 = 2;

fn tmp(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("d2rust-scene-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Moving-unit 16.16 coordinates whose client position is `(px, py)`
/// (a − b = 2 px, a + b = 4 py).
fn fixed(px: i32, py: i32) -> [u32; 2] {
    let (a, b) = (px + 2 * py, 2 * py - px);
    [(a as u32) << 11, (b as u32) << 11]
}

fn unit_key(guid: u32) -> FrameSetKey {
    FrameSetKey::new(format!("data/global/tst/u{guid}.dcc"), FramePart::Dir(0)).unwrap()
}

fn tile_key(name: &str) -> FrameSetKey {
    FrameSetKey::new(format!("data/global/tiles/{name}.dt1"), FramePart::Tile(0)).unwrap()
}

fn one(frame: IndexFrame) -> FrameSet {
    FrameSet {
        frames: vec![frame],
    }
}

fn filled(w: u32, h: u32, x: i32, y: i32, v: u8) -> IndexFrame {
    IndexFrame::new(w, h, x, y, vec![v; (w * h) as usize]).unwrap()
}

fn cof() -> Cof {
    Cof {
        layers_count: 1,
        frames: 1,
        directions: 1,
        version: 20,
        unknown: [0; 4],
        x_min: 0,
        x_max: 0,
        y_min: 0,
        y_max: 0,
        animation_rate: 256,
        layers: vec![CofLayer {
            component: 0,
            shadow: 0,
            selectable: 1,
            override_translucency: 0,
            new_translucency: 0,
            weapon_class: *b"hth\0",
        }],
        events: vec![0],
        event_padding: Vec::new(),
        draw_order: vec![0],
    }
}

/// The hooks owned by other specs, answered by fixture (not rules): every
/// unit shows the one-layer COF and its own frame set, key (2, guid).
struct Fixture;

impl ViewRules for Fixture {
    fn tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        unreachable!("OriginalView answers tiles")
    }
    fn unit_pose(&self, _: &ClientWorld, _: &ClientUnit) -> Result<Option<UnitPose>, ViewError> {
        Ok(Some(UnitPose {
            sub_frame: 0,
            cof: CanonicalPath::new(COF).unwrap(),
            dir: 0,
            frame: 0,
            dir64: 0,
        }))
    }
    fn unit_params(
        &self,
        _: &ClientWorld,
        u: &ClientUnit,
        _: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        Ok(UnitParams {
            pass: 2,
            major: u.key.guid,
            minor: 0,
            clip: Rect::new(0, 0, 1, 1),
            tag: ItemTag::Unit(u.key.guid),
        })
    }
    fn component_frame(
        &self,
        u: &ClientUnit,
        _: &UnitPose,
        _: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        Ok(ComponentFrame {
            set: unit_key(u.key.guid),
            index: 0,
        })
    }
    fn place(
        &self,
        _: &ClientUnit,
        _: &UnitPose,
        _: &ComponentRequest<'_>,
        _: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        unreachable!("OriginalView answers place")
    }
    fn shade(
        &self,
        _: &ClientUnit,
        _: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError> {
        Ok(ShadeChain::EMPTY)
    }
    fn blend(&self, _: &ClientUnit, _: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        Ok(BlendOp::Opaque)
    }
}

impl UiRules for Fixture {
    fn ui_image(&self, _: &ImageRequest, _: &ViewAssets) -> Result<UiSprite, ViewError> {
        unreachable!("no UI")
    }
    fn ui_text(&self, _: &TextRequest, _: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        unreachable!("no UI")
    }
    fn ui_pass(&self) -> Result<u32, ViewError> {
        Ok(9)
    }
}

/// Unit positions and map tiles of one recorded frame: the player at its
/// recorded path position, the object at static subtile (163, 93), a
/// floor and a wall on cell (26, 18).
struct Positions {
    player: UnitPosition,
}

impl ViewSource for Positions {
    fn unit_position(&self, unit: &ClientUnit) -> Result<UnitPosition, String> {
        match unit.key.guid {
            PLAYER => Ok(self.player),
            OBJECT => Ok(UnitPosition::Static { sx: 163, sy: 93 }),
            g => Err(format!("no unit {g}")),
        }
    }
    fn unit_offset(&self, _: &ClientUnit, _: &UnitPose) -> Result<(i32, i32), String> {
        Ok((0, 0))
    }
    fn map_tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<MapTile>, ViewError> {
        let tile = |list, name: &str, blocks| MapTile {
            cell: (26, 18),
            list,
            frame: ComponentFrame {
                set: tile_key(name),
                index: 0,
            },
            blocks,
            shade: ShadeChain::EMPTY,
            blend: BlendOp::Opaque,
            key: DrawKey::default(),
        };
        let wall = BlockRect {
            x: 64,
            y: -64,
            width: 2,
            height: 2,
        };
        Ok(vec![
            tile(TileList::Floor, "floor", Vec::new()),
            tile(TileList::Wall, "wall", vec![wall]),
        ])
    }
}

/// A recording that holds the world (what [`RECORDER_GAP`] asks for), as
/// a test fixture.
struct SyntheticWorld {
    world: ClientWorld,
    assets: ViewAssets,
    positions: Option<Positions>,
}

impl SyntheticWorld {
    fn new() -> Self {
        let mut world = ClientWorld::default();
        for (unit_type, guid) in [(0, PLAYER), (2, OBJECT)] {
            let key = UnitKey { unit_type, guid };
            world.units.insert(key, ClientUnit::new(key));
        }
        let mut assets = ViewAssets::new(synthetic_palette());
        assets.cofs.insert(CanonicalPath::new(COF).unwrap(), cof());
        let mut add = |k, s| assets.frames.insert(k, s).unwrap();
        add(unit_key(PLAYER), one(filled(8, 20, -4, -20, 5)));
        add(
            unit_key(OBJECT),
            one(filled(3, 2, 5, 10, 6).with_anchor(FrameAnchor::Bottom)),
        );
        add(tile_key("floor"), one(filled(4, 2, 80, 0, 3)));
        add(tile_key("wall"), one(filled(2, 2, 64, -64, 4)));
        SyntheticWorld {
            world,
            assets,
            positions: None,
        }
    }
}

impl CaptureWorld for SyntheticWorld {
    fn scene(&mut self, job: &SceneJob<'_>, _: &Camera) -> WorldAnswer<'_> {
        let Some([x16, y16]) = job.captured.state.player.as_ref().and_then(|p| p.fixed) else {
            return WorldAnswer::Error("no player".into());
        };
        let source: &dyn ViewSource = self.positions.insert(Positions {
            player: UnitPosition::Moving { x16, y16 },
        });
        WorldAnswer::Scene(RecordedScene {
            world: &self.world,
            ui: &[],
            rules: &Fixture,
            source,
            assets: &self.assets,
        })
    }
}

fn paint(buf: &mut [u8], x: i32, y: i32, w: i32, h: i32, v: u8) {
    for yy in y..y + h {
        for xx in x..x + w {
            buf[(yy * W + xx) as usize] = v;
        }
    }
}

/// The frame D2RUST must show with the player at client (1000 + dx,
/// 2000), from the camera and sprite placement rules,
/// written out by hand: floor handed (40, 40), wall image at (24, 56),
/// player DCC box at (396, 272), object DC6 bottom row 350.
fn expected(dx: i32) -> Vec<u8> {
    let mut want = vec![0u8; (W * H) as usize];
    paint(&mut want, 40 - dx, 40, 4, 2, 3);
    paint(&mut want, 24 - dx, 56, 2, 2, 4);
    paint(&mut want, 396, 272, 8, 20, 5);
    paint(&mut want, 525 - dx, 349, 3, 2, 6);
    want
}

fn png(pixels: &[u8], palette: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, W as u32, H as u32);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(palette.to_vec());
    let mut w = enc.write_header().unwrap();
    w.write_image_data(pixels).unwrap();
    w.finish().unwrap();
    out
}

/// One recorded frame: draw counter, player x offset, and the recorded
/// camera values (at open mode 0, no shake), which a test
/// may change.
#[derive(Clone)]
struct Shot {
    draw: u32,
    dx: i32,
    record: Value,
}

fn shot(draw: u32, dx: i32) -> Shot {
    let px = 1000 + dx;
    let record = json!({
        "k": "frame", "f": 200 + draw, "video_type": 1, "w": W, "h": H,
        "draw": draw, "open_mode": 0, "shift_x": 0,
        "unit_origin": [px - 400, 1716], "tile_origin": [px - 400, 1720],
        "view_rect": [0, 0, 800, 560], "shake": [0, 0, 0],
        "clear_counter": 0, "res_mode": 2,
        "player": {"type": 0, "mode": 2, "cur": 0, "fixed": fixed(px, 2000), "client": [px, 2000]},
        "image": format!("frame-{draw:07}.png"),
    });
    Shot { draw, dx, record }
}

/// Writes the raw file and the PNGs (each the hand-painted frame).
fn record(dir: &Path, shots: &[Shot]) -> SceneCase {
    let images = dir.join("captures");
    std::fs::create_dir_all(&images).unwrap();
    let palette = capture::palette_bytes(&synthetic_palette());
    let mut lines = vec![
        json!({"k": "header", "format": "frames-raw-1", "tool": "test", "date": "2026-10-06",
               "game_exe_sha256": "0".repeat(64), "args": ["-w", "-ns"]}),
        json!({"k": "game", "g": "0x1", "frame_before": 0}),
    ];
    for s in shots {
        let pixels = expected(s.dx);
        std::fs::write(
            images.join(format!("frame-{:07}.png", s.draw)),
            png(&pixels, &palette),
        )
        .unwrap();
        lines.push(json!({"k": "tick", "f": s.record["f"]}));
        let mut rec = s.record.clone();
        rec["index_sha256"] = json!(capture::sha256_hex(&pixels));
        rec["palette_sha256"] = json!(capture::sha256_hex(&palette));
        lines.push(rec);
    }
    lines.push(json!({"k": "footer", "ticks": shots.len(),
                      "counts": {"game": 1, "tick": shots.len(), "frame": shots.len()},
                      "notes": []}));
    let raw = dir.join("20261006-000000-frames.jsonl");
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&raw, text).unwrap();
    SceneCase {
        raw: RawRef::Path(raw.display().to_string()),
        images: Some(images.display().to_string()),
        check: CaptureCheck::Compare,
        seqs: Vec::new(),
    }
}

/// A walk: the player one pixel right per draw, then standing.
fn walk() -> Vec<Shot> {
    vec![shot(10, 0), shot(11, 1), shot(12, 2), shot(13, 2)]
}

/// The GPU's contract (the CPU reference), so the GPU branch runs without
/// an adapter.
struct CpuAsGpu;

impl GpuCompositor for CpuAsGpu {
    fn compose(&mut self, job: &GpuJob<'_>) -> GpuOutcome {
        let indices = scene::compose(job.items, job.frames, job.maps, job.view).unwrap();
        let rgba = scene::to_rgba(&indices, job.palette);
        GpuOutcome::Image { indices, rgba }
    }
}

fn run(
    c: &SceneCase,
    n: usize,
    world: impl CaptureWorld,
    gpu: &mut dyn GpuCompositor,
) -> CaseReport {
    run_capture(
        "scene-test",
        c,
        Path::new("/"),
        n,
        &mut WorldScene::new(world),
        gpu,
    )
}

fn count(r: &CaseReport, text: &str) -> usize {
    r.lines.iter().filter(|l| l.contains(text)).count()
}

#[test]
fn recorded_world_through_the_original_view_equals_the_capture() {
    let dir = tmp("pass");
    let c = record(&dir, &walk());
    let r = run(&c, 0, SyntheticWorld::new(), &mut CpuAsGpu);
    assert_eq!(r.status, Status::Pass, "{:#?}", r.lines);
    assert!(
        r.lines.iter().any(|l| l == "3 frames selected"),
        "{:#?}",
        r.lines
    );
    assert_eq!(count(&r, "frames: 3 match, 0 differ or fail"), 1);
    assert_eq!(count(&r, "equal camera.md §1, §3"), 1, "{:#?}", r.lines);
    assert_eq!(count(&r, "CPU: 0 of 480000"), 1, "{:#?}", r.lines);
    assert_eq!(count(&r, "GPU indices: 0 of 480000"), 1);
    assert_eq!(count(&r, "palette: 0 of 256"), 1);
}

// The deliberate perturbation: `--perturb N` → every compared frame fails with exactly N on the
// CPU half and both GPU comparisons.
#[test]
fn perturbation_reports_exactly_n() {
    let dir = tmp("perturb");
    let c = record(&dir, &walk());
    for n in [1usize, 7, 64] {
        let r = run(&c, n, SyntheticWorld::new(), &mut CpuAsGpu);
        assert_eq!(r.status, Status::Fail("3 of 3 frames".into()), "{n}");
        assert_eq!(
            count(&r, &format!("CPU: {n} of 480000 bytes differ")),
            3,
            "{:#?}",
            r.lines
        );
        assert_eq!(
            count(&r, &format!("GPU indices: {n} of 480000 bytes differ")),
            3
        );
        assert_eq!(count(&r, &format!("GPU: {n} of 480000 pixels differ")), 3);
    }
}

// A one-pixel perturbation on the pipeline: one recorded player pixel off moves every
// non-player draw, and the camera check catches the recorded origins
// before any pixel is compared.
#[test]
fn recorded_camera_differences_fail_the_frame() {
    let dir = tmp("camera");
    let mut shots = walk();
    shots[2].record["tile_origin"] = json!([603, 1720]);
    shots[3].record["view_rect"] = json!([0, 0, 800, 561]);
    let c = record(&dir, &shots);
    let r = run(&c, 0, SyntheticWorld::new(), &mut CpuAsGpu);
    assert_eq!(
        r.status,
        Status::Fail("2 of 3 frames".into()),
        "{:#?}",
        r.lines
    );
    assert_eq!(
        count(
            &r,
            "tile origin (camera.md §3): recorded [603, 1720], rule [602, 1720]"
        ),
        1,
        "{:#?}",
        r.lines
    );
    assert_eq!(
        count(&r, "view rect (camera.md §1): recorded [0, 0, 800, 561]"),
        1
    );

    // A pixel the recorded player path disagrees with: draw 11 recorded
    // at x 1001 but painted for 1002 (the pipeline is not fooled).
    let dir = tmp("moved");
    let mut shots = walk();
    shots[1].dx = 2;
    let c = record(&dir, &shots);
    let r = run(&c, 0, SyntheticWorld::new(), &mut CpuAsGpu);
    assert_eq!(
        r.status,
        Status::Fail("1 of 3 frames".into()),
        "{:#?}",
        r.lines
    );
    // Floor, wall, object: each loses one column and gains one (2 + 2 + 2
    // rows × 2 sides).
    assert_eq!(
        count(&r, "CPU: 12 of 480000 bytes differ"),
        1,
        "{:#?}",
        r.lines
    );
}

#[test]
fn recorded_camera_reads_mode_size_and_shake() {
    let rec = |v: Value| {
        let text = format!(
            "{}\n{}\n{}\n",
            json!({"k": "header", "format": "frames-raw-1", "tool": "t", "date": "d",
                   "game_exe_sha256": "x", "args": []}),
            v,
            json!({"k": "footer", "ticks": 0, "counts": {"frame": 1}, "notes": []})
        );
        let raw = capture::parse_raw(&text).unwrap();
        let f = raw.frames[0].clone();
        let c = f.captured().unwrap().clone();
        (f, c)
    };
    let mut v = shot(1, 0).record;
    v["index_sha256"] = json!("0".repeat(64));
    v["palette_sha256"] = json!("0".repeat(64));
    // Mode 1 with a shake (−3, 2): view (−200, 0, 600, 560), origins
    // shifted by the offsets.
    v["open_mode"] = json!(1);
    v["shift_x"] = json!(-200);
    v["view_rect"] = json!([-200, 0, 600, 560]);
    v["shake"] = json!([4, -3, 2]);
    v["tile_origin"] = json!([597, 1722]);
    v["unit_origin"] = json!([597, 1718]);
    let (f, c) = rec(v.clone());
    let cam = recorded_camera(f.w, f.h, &c.state).unwrap();
    assert_eq!(camera_differences(&cam, &c.state), Vec::<String>::new());
    // The player is drawn at (200, 292) in mode 1.
    assert_eq!(
        cam.unit_draw(crate::rules::ClientPos { x: 1000, y: 2000 }, (0, 0)),
        (200 + 3, 292 - 2)
    );

    let mut bad = v.clone();
    bad["open_mode"] = json!(4);
    let (f, c) = rec(bad);
    assert!(recorded_camera(f.w, f.h, &c.state)
        .unwrap_err()
        .contains("open mode 4"));
    let mut bad = v.clone();
    bad["w"] = json!(640);
    bad["h"] = json!(480);
    let (f, c) = rec(bad);
    assert!(recorded_camera(f.w, f.h, &c.state)
        .unwrap_err()
        .contains("800x600"));
    let mut bad = v;
    bad.as_object_mut().unwrap().remove("player");
    let (f, c) = rec(bad);
    assert!(recorded_camera(f.w, f.h, &c.state)
        .unwrap_err()
        .contains("no player"));
}

// Today's recordings: the camera check runs, then the frame stops at the
// recorder seam (never a pass). A frames-raw-1 frame has no draw log.
#[test]
fn todays_recordings_stop_at_the_recorder_seam() {
    let dir = tmp("seam");
    let c = record(&dir, &walk());
    let r = run(&c, 0, NotRecorded, &mut CpuAsGpu);
    assert_eq!(r.status, Status::SceneNotWired, "{:#?}", r.lines);
    assert_eq!(
        count(&r, "frames: 0 match, 0 differ or fail, 3 scene not wired"),
        1
    );
    assert_eq!(count(&r, "equal camera.md §1, §3"), 1, "{:#?}", r.lines);
    assert_eq!(count(&r, &format!("scene: seam: {NO_DRAW_LOG}")), 1);
}

// A frame is composed only with its draw log and the
// initial framebuffer (frame seq − 1 of an --every 1 recording); the
// seam names the first missing input.
#[test]
fn a_frame_needs_its_draw_log_and_initial_framebuffer() {
    let mut frame = shot(10, 0).record;
    for (k, v) in [
        ("seq", json!(1)),
        ("client_update", json!(5)),
        ("level", json!({"level_id": 1, "act": 0})),
        (
            "light",
            json!({"quality": 0, "draw_rate": 25, "opt_a": 0, "opt_b": 0, "render_kind": 1}),
        ),
        ("light_key", json!(0)),
        (
            "weather",
            json!({"rain": 0, "snow": 0, "lightning": 0, "flash": 0, "update": 0}),
        ),
        ("index_sha256", json!("0".repeat(64))),
        ("palette_sha256", json!("0".repeat(64))),
    ] {
        frame[k] = v;
    }
    let mut logged = frame.clone();
    logged["draws"] = json!([{"op": "CelDraw", "at": 1, "a": [0, 1, 2]}]);
    let parse = |f: &Value| {
        let text = format!(
            "{}\n{}\n{}\n{}\n",
            json!({"k": "header", "format": "frames-raw-2", "tool": "t", "date": "d",
                   "game_exe_sha256": "x", "args": []}),
            json!({"k": "capture", "images": null, "every": 1, "draws_every": 1,
                   "state_key": []}),
            f,
            json!({"k": "footer", "ticks": 0, "counts": {"capture": 1, "frame": 1}, "notes": []})
        );
        capture::parse_raw(&text).unwrap().frames.remove(0)
    };
    let previous = capture::Image {
        width: 1,
        height: 1,
        indices: vec![0],
        palette: vec![0; 768],
    };
    let bare = parse(&frame);
    let logged = parse(&logged);
    let entry = logged.captured().unwrap().draws.as_ref().unwrap();
    assert_eq!(entry[0]["op"], "CelDraw");
    let camera = recorded_camera(W, H, &bare.captured().unwrap().state).unwrap();
    let gap = |f: &capture::Frame, previous: Option<&capture::Image>| {
        let job = SceneJob {
            case: "t",
            frame: f,
            captured: f.captured().unwrap(),
            previous,
        };
        match NotRecorded.scene(&job, &camera) {
            WorldAnswer::Seam(s) => s,
            _ => panic!("NotRecorded answers a seam"),
        }
    };
    assert_eq!(gap(&bare, Some(&previous)), NO_DRAW_LOG);
    assert_eq!(gap(&logged, None), NO_INITIAL_FRAME);
    assert_eq!(gap(&logged, Some(&previous)), RECORDER_GAP);
    // An entry without its op is refused by the reader.
    let mut bad = frame;
    bad["draws"] = json!([{"at": 1}]);
    let text = format!(
        "{}\n{}\n{}\n",
        json!({"k": "header", "format": "frames-raw-2", "tool": "t", "date": "d",
               "game_exe_sha256": "x", "args": []}),
        json!({"k": "capture", "images": null, "every": 1, "draws_every": 1, "state_key": []}),
        bad
    );
    let e = capture::parse_raw(&text).unwrap_err().to_string();
    assert!(e.contains("draws[0].op: expected a string"), "{e}");
}

/// The real GPU half (llvmpipe in the cloud): byte-identical to the
/// capture, and `--perturb 7` exactly 7 on every comparison.
#[test]
#[ignore = "needs a GPU adapter (a software one such as lavapipe will do)"]
fn gpu_half_matches_the_synthetic_capture() {
    let mut gpu = gpu::Wgpu::new();
    let line = gpu.open();
    println!("{line}");
    assert!(line.starts_with("adapter: "), "{line}");
    let dir = tmp("gpu");
    let c = record(&dir, &walk());
    let r = run(&c, 0, SyntheticWorld::new(), &mut gpu);
    println!("{:#?}", r.lines);
    assert_eq!(r.status, Status::Pass, "{:#?}", r.lines);
    assert_eq!(count(&r, "GPU: 0 of 480000 pixels differ"), 1);
    let r = run(&c, 7, SyntheticWorld::new(), &mut gpu);
    assert_eq!(
        r.status,
        Status::Fail("3 of 3 frames".into()),
        "{:#?}",
        r.lines
    );
    for prefix in ["CPU: 7 of", "GPU indices: 7 of", "GPU: 7 of"] {
        assert_eq!(count(&r, prefix), 3, "{prefix} {:#?}", r.lines);
    }
}
