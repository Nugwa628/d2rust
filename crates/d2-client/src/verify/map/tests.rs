//! Tests for the `map` case kind on a synthetic layout (no game files): chunking,
//! the scene list against the `map::cpu` reference, the GPU seam statuses,
//! `--perturb` counts, and the index-0 background made visible. The
//! ignored test runs the real compute compositor.

use super::*;
use crate::map::layout::{DrawItem as MapItem, Source};
use crate::map::{TileImage, TileKey};
use crate::verify::{perturb_indices, synthetic_palette};

/// Tile images: a 40×30 image with a transparent diagonal stripe, a
/// 7×5 opaque block, a 1×1 dot, and an empty (0×0) image.
fn library() -> TileLibrary {
    let mut lib = TileLibrary::new();
    let striped = |w: u32, h: u32, base: u8| TileImage {
        x0: 0,
        y0: 0,
        width: w,
        height: h,
        pixels: (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if (x + y) % 9 == 0 {
                    0
                } else {
                    base.wrapping_add((x * 3 + y) as u8) | 1
                }
            })
            .collect(),
        roof_height: 0,
    };
    lib.images.push(striped(40, 30, 10));
    lib.images.push(TileImage {
        x0: 0,
        y0: 0,
        width: 7,
        height: 5,
        pixels: vec![200; 35],
        roof_height: 0,
    });
    lib.images.push(TileImage {
        x0: 0,
        y0: 0,
        width: 1,
        height: 1,
        pixels: vec![77],
        roof_height: 0,
    });
    lib.images.push(TileImage {
        x0: 0,
        y0: 0,
        width: 0,
        height: 0,
        pixels: Vec::new(),
        roof_height: 0,
    });
    lib
}

fn item(image: usize, x: i32, y: i32) -> MapItem {
    MapItem {
        image,
        x,
        y,
        key: TileKey {
            orientation: 0,
            main: 0,
            sub: 0,
        },
        cell: (x, y),
        source: Source::Sprite,
    }
}

/// Overlapping items across chunk borders, partly outside the view,
/// reused images, an empty image.
fn layout() -> Layout {
    Layout {
        items: vec![
            item(0, -15, -12),
            item(0, 20, 10),
            item(1, 28, 28),
            item(0, 50, 40),
            item(2, 31, 31),
            item(3, 5, 5),
            item(1, 95, 60),
            item(0, 70, -5),
            item(2, 0, 0),
        ],
        ..Layout::default()
    }
}

/// View 100×70 at (-6, -4): not a multiple of the 32 chunk side.
const VIEW: cpu::View = cpu::View {
    left: -6,
    top: -4,
    width: 100,
    height: 70,
};

/// The synthetic palette with index 0 black, as `map::cpu::to_rgba` paints
/// the background.
fn black_zero() -> Palette {
    let mut p = synthetic_palette();
    p.colors[0] = d2_formats::palette::Rgb { r: 0, g: 0, b: 0 };
    p
}

/// The CPU compositor as a GPU stand-in, optionally corrupting N indices
/// of its result before mapping them through the palette.
struct Echo {
    corrupt: usize,
}

impl GpuCompositor for Echo {
    fn compose(&mut self, job: &GpuJob<'_>) -> GpuOutcome {
        let mut indices =
            scene::compose_binned(job.items, job.bins, job.frames, job.maps, job.view).unwrap();
        perturb_indices(&mut indices, self.corrupt).unwrap();
        let rgba = scene::to_rgba(&indices, job.palette);
        GpuOutcome::Image { indices, rgba }
    }
}

struct Fixed(GpuOutcome);

impl GpuCompositor for Fixed {
    fn compose(&mut self, _job: &GpuJob<'_>) -> GpuOutcome {
        self.0.clone()
    }
}

fn run(palette: &Palette, perturb_n: usize, gpu: &mut dyn GpuCompositor) -> MapRun {
    let (layout, lib) = (layout(), library());
    let scene = MapScene {
        layout: &layout,
        lib: &lib,
        palette,
        view: VIEW,
    };
    compare_layout("t", scene, perturb_n, 32, gpu).unwrap()
}

fn gpu_counts(run: &MapRun) -> (usize, usize) {
    match &run.gpu {
        GpuResult::Compared { indices, rgba, .. } => (indices.mismatched, rgba.mismatched),
        other => panic!("{other:?}"),
    }
}

#[test]
fn chunks_cover_the_view_once() {
    let view = view_rect(VIEW);
    let rects = chunks(view, 32);
    // 100 = 32+32+32+4, 70 = 32+32+6.
    assert_eq!(rects.len(), 12);
    assert_eq!(rects[0], Rect::new(-6, -4, 32, 32));
    assert_eq!(rects[3], Rect::new(90, -4, 4, 32));
    assert_eq!(rects[11], Rect::new(90, 60, 4, 6));
    let mut seen = vec![0u8; 100 * 70];
    for r in &rects {
        for y in 0..r.height as i32 {
            for x in 0..r.width as i32 {
                seen[((r.y + y + 4) * 100 + r.x + x + 6) as usize] += 1;
            }
        }
    }
    assert!(seen.iter().all(|&n| n == 1));
    assert_eq!(chunks(view, 1024), [view]);
}

#[test]
fn chunk_keeps_touching_items_in_layout_order() {
    let (layout, lib) = (layout(), library());
    let c = chunk(&layout, &lib, view_rect(VIEW), Rect::new(26, 28, 32, 32));
    // Items 1 (image 0 at 20,10), 2 (image 1), 3 (image 0 at 50,40),
    // 4 (image 2); not 0, 5 (empty), 6, 7, 8.
    let got: Vec<_> = c.items.iter().map(|i| (i.frame, i.x, i.y)).collect();
    assert_eq!(
        got,
        [
            (FrameId(0), 20, 10),
            (FrameId(1), 28, 28),
            (FrameId(0), 50, 40),
            (FrameId(2), 31, 31),
        ]
    );
    // Images in first-use order, each once.
    let sizes: Vec<_> = c.frames.iter().map(|f| (f.width, f.height)).collect();
    assert_eq!(sizes, [(40, 30), (7, 5), (1, 1)]);
    assert!(c.items.iter().all(|i| i.clip == view_rect(VIEW)));
}

// The chunked scene list reproduces the map reference.
#[test]
fn scene_list_matches_the_map_reference() {
    let r = run(&black_zero(), 0, &mut Echo { corrupt: 0 });
    assert_eq!((r.items, r.chunks), (9, 12));
    assert_eq!(r.binned.mismatched, 0);
    assert_eq!(r.binned.bytes, 100 * 70);
    assert_eq!(gpu_counts(&r), (0, 0));
    let (lines, status) = r.verdict();
    assert_eq!(status, Status::Pass, "{lines:?}");
    // The stitched GPU image is the reference.
    match &r.gpu {
        GpuResult::Compared { image, .. } => assert_eq!(image, &r.reference),
        other => panic!("{other:?}"),
    }
    // The reference is not trivial: background and several indices show.
    let indexed = cpu::render_indexed(&layout(), &library(), VIEW);
    assert!(indexed.contains(&0) && indexed.contains(&200) && indexed.contains(&77));
}

// `--perturb N` reports exactly N on every comparison.
#[test]
fn perturb_reports_exactly_n_on_every_comparison() {
    for n in [1, 7, 500] {
        let r = run(&black_zero(), n, &mut Echo { corrupt: 0 });
        assert_eq!(r.binned.mismatched, n);
        assert_eq!(gpu_counts(&r), (n, n));
        assert_eq!(r.verdict().1, Status::Fail("CPU and GPU halves".into()));
    }
    // Exactly N whatever the palette: here every index is black, so an
    // index change alone would not show in RGBA.
    let flat = Palette {
        colors: [d2_formats::palette::Rgb { r: 0, g: 0, b: 0 }; 256],
    };
    let r = run(&flat, 7, &mut Echo { corrupt: 0 });
    assert_eq!((r.binned.mismatched, gpu_counts(&r)), (7, (7, 7)));
    // Above the pixel count is an error, not fewer.
    let (layout, lib) = (layout(), library());
    let scene = MapScene {
        layout: &layout,
        lib: &lib,
        palette: &flat,
        view: VIEW,
    };
    assert!(compare_layout("t", scene, 7001, 32, &mut Echo { corrupt: 0 }).is_err());
}

#[test]
fn gpu_corruption_fails_the_gpu_half_only() {
    // Two corrupted indices per chunk, 12 chunks.
    let r = run(&black_zero(), 0, &mut Echo { corrupt: 2 });
    assert_eq!(r.binned.mismatched, 0);
    assert_eq!(gpu_counts(&r), (24, 24));
    let (lines, status) = r.verdict();
    assert_eq!(status, Status::Fail("GPU half".into()));
    assert!(lines
        .iter()
        .any(|l| l.starts_with("GPU indices: 24 of 7000")));
}

/// `map::cpu::to_rgba` paints index 0 black; the compositor maps it
/// through the palette. With `palette[0]` not black the RGBA comparison
/// fails on exactly the background pixels while the indices agree: the index-0 background
/// difference shows instead of being hidden.
#[test]
fn index_zero_question_is_visible() {
    let r = run(&synthetic_palette(), 0, &mut Echo { corrupt: 0 });
    let background = cpu::render_indexed(&layout(), &library(), VIEW)
        .iter()
        .filter(|&&i| i == 0)
        .count();
    assert!(background > 0);
    assert_eq!(r.binned.mismatched, 0);
    assert_eq!(gpu_counts(&r), (0, background));
    assert_eq!(r.verdict().1, Status::Fail("GPU half".into()));
}

#[test]
fn gpu_seam_statuses() {
    let p = black_zero();
    let r = run(&p, 0, &mut Fixed(GpuOutcome::NotWired));
    assert_eq!(r.gpu, GpuResult::NotWired);
    assert_eq!(r.verdict().1, Status::GpuNotWired);
    let r = run(&p, 0, &mut Fixed(GpuOutcome::NoAdapter("none".into())));
    assert_eq!(r.verdict().1, Status::NoAdapter("none".into()));
    let r = run(&p, 0, &mut Fixed(GpuOutcome::Error("lost".into())));
    assert_eq!(r.verdict().1, Status::Error("GPU: lost".into()));
    // A wrong-sized image is an error, not a count.
    let bad = GpuOutcome::Image {
        indices: vec![0; 3],
        rgba: vec![0; 12],
    };
    assert!(matches!(
        run(&p, 0, &mut Fixed(bad)).verdict().1,
        Status::Error(_)
    ));
    // The CPU half still runs over every chunk.
    let r = run(&p, 5, &mut Fixed(GpuOutcome::NotWired));
    assert_eq!(r.binned.mismatched, 5);
    assert_eq!(r.verdict().1, Status::Fail("CPU half".into()));
}

#[test]
fn odd_view_is_an_error() {
    let (layout, lib) = (layout(), library());
    let scene = MapScene {
        layout: &layout,
        lib: &lib,
        palette: &black_zero(),
        view: cpu::View { width: 99, ..VIEW },
    };
    assert!(compare_layout("t", scene, 0, 32, &mut Echo { corrupt: 0 }).is_err());
}

/// The real compute compositor (`verify::gpu::Wgpu`) on the synthetic
/// map, chunked: 0 differing, then `--perturb 7` reports exactly 7 on
/// every comparison. Run only on Mesa llvmpipe so far, not on a real GPU:
/// `cargo test -p d2-client --lib verify::map::tests::gpu_map -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU adapter (a software one such as lavapipe will do)"]
fn gpu_map_matches_cpu_reference() {
    let mut gpu = crate::verify::gpu::Wgpu::new();
    let info = gpu.open();
    println!("{info}");
    assert!(info.starts_with("adapter: "), "{info}");
    let p = black_zero();
    for side in [32, 64, 1024] {
        let (layout, lib) = (layout(), library());
        let scene = MapScene {
            layout: &layout,
            lib: &lib,
            palette: &p,
            view: VIEW,
        };
        let r = compare_layout("gpu", scene, 0, side, &mut gpu).unwrap();
        let (lines, status) = r.verdict();
        println!("side {side}: {lines:?}");
        assert_eq!(status, Status::Pass, "{lines:?}");
        let r = compare_layout("gpu", scene, 7, side, &mut gpu).unwrap();
        assert_eq!(r.binned.mismatched, 7);
        assert_eq!(gpu_counts(&r), (7, 7));
    }
}
