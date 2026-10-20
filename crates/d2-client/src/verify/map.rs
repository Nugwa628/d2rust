//! Case kind `map`: compare a real map composed on the CPU with the same map composed by the
//! compute compositor. CPU half: the `map::cpu` reference of the DS1 layout. GPU half: the same
//! layout as a scene draw list through [`GpuCompositor`], like every other case.
//!
//! The layout becomes scene items one to one, in layout order: item `k` draws its tile image at
//! its `(x, y)`, opaque, unshaded, clipped to the view, key 0 (all keys equal, so the stable
//! order is the layout order, which is the map's draw order). Scene and `map::cpu` agree on that
//! list by construction (index 0 transparent, later items on top); the "CPU binned" line proves
//! it on every run.
//!
//! The view is composed in chunks of at most [`CHUNK`]² pixels (a whole map is up to
//! [`MAX_TEXTURE_SIDE`]² pixels, above the compositor's storage limits); each chunk is one
//! [`GpuJob`] holding only the items and tile images that touch it. Counts are summed over
//! chunks, and the first mismatch is the first in chunk order (chunks row by row).
//!
//! Comparisons, each byte for byte against the reference of the whole view: CPU binned compose
//! of the scene list (indices), GPU indices, GPU RGBA. The RGBA reference is
//! `map::cpu::to_rgba` (index 0 = opaque black); the compositor maps every index through the
//! palette (`scene::to_rgba`). They differ wherever the background shows exactly when
//! `palette[0]` is not black. Index 0 is black in every act palette, and a drawn pixel mapped
//! to 0 is opaque index 0, so with an act palette the two agree; a mismatch there names a
//! palette that is not an act's.
//!
//! `--perturb N` flips the index top bit of N reference pixels and the red top bit of the same N
//! RGBA pixels (the [`super::perturb`] spacing), so every comparison reports exactly N whatever
//! the palette.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use d2_formats::mpq::ArchiveSet;
use d2_formats::palette::Palette;

use super::case::MapCase;
use super::{
    compare, compare_indices, perturb, perturb_indices, ByteMismatch, CaseReport, GpuCompositor,
    GpuJob, GpuOutcome, Mismatch, Status,
};
use crate::map::{self, cpu, Layout, TileLibrary};
use crate::scene::{self, DrawItem, FrameId, FrameImage, ItemTag, MapTable, Rect};

/// Largest side of the whole-map view (the reference image).
pub const MAX_TEXTURE_SIDE: u32 = 8192;

/// Largest chunk side composed in one GPU job.
pub const CHUNK: u32 = 1024;

/// The game archives in `$D2_GAME_DIR`.
pub fn archives() -> Result<ArchiveSet> {
    let dir = std::env::var("D2_GAME_DIR").context("set D2_GAME_DIR to the game folder")?;
    ArchiveSet::open_dir(&dir).with_context(|| format!("opening archives in {dir}"))
}

/// File name without folders and extension (`townN1`).
pub fn file_stem(ds1: &str) -> String {
    ds1.rsplit(['\\', '/'])
        .next()
        .and_then(|f| f.split('.').next())
        .unwrap_or("map")
        .to_owned()
}

/// The whole-map view, rounded up to even sizes (pixel alignment) and
/// clamped to [`MAX_TEXTURE_SIDE`] around the map center.
pub fn full_view(bounds: map::Bounds) -> cpu::View {
    let side = |lo: i32, hi: i32| -> (i32, u32) {
        let len = ((hi - lo) as u32).div_ceil(2) * 2;
        if len <= MAX_TEXTURE_SIDE {
            (lo, len)
        } else {
            let center = (lo + hi) / 2;
            (center - MAX_TEXTURE_SIDE as i32 / 2, MAX_TEXTURE_SIDE)
        }
    };
    let (left, width) = side(bounds.x0, bounds.x1);
    let (top, height) = side(bounds.y0, bounds.y1);
    cpu::View {
        left,
        top,
        width,
        height,
    }
}

/// The view as a scene rectangle (screen coordinates).
pub fn view_rect(view: cpu::View) -> Rect {
    Rect::new(view.left, view.top, view.width, view.height)
}

/// The chunks of `view`, row by row, each at most `side`² pixels.
pub fn chunks(view: Rect, side: u32) -> Vec<Rect> {
    assert!(side > 0, "chunk side must be positive");
    let mut out = Vec::new();
    let mut y = 0;
    while y < view.height {
        let h = side.min(view.height - y);
        let mut x = 0;
        while x < view.width {
            let w = side.min(view.width - x);
            out.push(Rect::new(view.x + x as i32, view.y + y as i32, w, h));
            x += w;
        }
        y += h;
    }
    out
}

/// One chunk's compositor inputs: the layout items touching `view`, in
/// layout order, and their tile images (`FrameId(n)` = `frames[n]`, in
/// first-use order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The draw items touching the chunk, in layout order.
    pub items: Vec<DrawItem>,
    /// The tile images those items use.
    pub frames: Vec<FrameImage>,
    /// The chunk's screen rectangle.
    pub view: Rect,
}

/// The scene list of `layout` restricted to the chunk `view` of the whole
/// map view `clip` (every item is clipped to `clip`, as `map::cpu` clips
/// to its view).
pub fn chunk(layout: &Layout, lib: &TileLibrary, clip: Rect, view: Rect) -> Chunk {
    let mut ids: BTreeMap<usize, FrameId> = BTreeMap::new();
    let mut frames = Vec::new();
    let mut items = Vec::new();
    for it in &layout.items {
        let img = &lib.images[it.image];
        let image = Rect::new(it.x, it.y, img.width, img.height);
        let touches = image
            .intersect(&clip)
            .and_then(|r| r.intersect(&view))
            .is_some();
        if !touches {
            continue;
        }
        let id = *ids.entry(it.image).or_insert_with(|| {
            frames.push(FrameImage {
                width: img.width,
                height: img.height,
                pixels: img.pixels.clone(),
            });
            FrameId(frames.len() as u32 - 1)
        });
        let mut item = DrawItem::new(id, it.x, it.y);
        item.clip = clip;
        item.tag = ItemTag::Tile {
            x: it.cell.0,
            y: it.cell.1,
        };
        items.push(item);
    }
    Chunk {
        items,
        frames,
        view,
    }
}

/// The `rect` part of a row-major image of `view` (`bpp` bytes per pixel).
fn crop(image: &[u8], view: Rect, rect: Rect, bpp: usize) -> Vec<u8> {
    let (dx, dy) = ((rect.x - view.x) as usize, (rect.y - view.y) as usize);
    let stride = view.width as usize * bpp;
    let mut out = Vec::with_capacity(rect.width as usize * rect.height as usize * bpp);
    for row in dy..dy + rect.height as usize {
        let start = row * stride + dx * bpp;
        out.extend_from_slice(&image[start..start + rect.width as usize * bpp]);
    }
    out
}

/// Writes the `rect` part of `view`'s image from `part`.
fn paste(image: &mut [u8], view: Rect, rect: Rect, part: &[u8], bpp: usize) {
    let (dx, dy) = ((rect.x - view.x) as usize, (rect.y - view.y) as usize);
    let stride = view.width as usize * bpp;
    let row_len = rect.width as usize * bpp;
    for (i, src) in part.chunks_exact(row_len).enumerate() {
        let start = (dy + i) * stride + dx * bpp;
        image[start..start + row_len].copy_from_slice(src);
    }
}

fn add_bytes(total: &mut ByteMismatch, m: ByteMismatch) {
    total.bytes += m.bytes;
    total.mismatched += m.mismatched;
    total.first = total.first.or(m.first);
}

fn add_pixels(total: &mut Mismatch, m: Mismatch) {
    total.pixels += m.pixels;
    total.mismatched += m.mismatched;
    total.first = total.first.or(m.first);
}

/// What the GPU half returned over all chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuResult {
    /// Every chunk composed: the sums and the stitched RGBA image.
    Compared {
        indices: ByteMismatch,
        rgba: Mismatch,
        image: Vec<u8>,
    },
    /// The first chunk that did not return an image decided it: no GPU compositor is wired.
    NotWired,
    /// No usable adapter.
    NoAdapter(String),
    /// The GPU half failed.
    Error(String),
}

/// The result of a map comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRun {
    /// Number of layout items in the scene list.
    pub items: usize,
    /// Number of chunks composed.
    pub chunks: usize,
    /// The scene list composed binned on the CPU, against the reference
    /// indices.
    pub binned: ByteMismatch,
    /// What the GPU half returned.
    pub gpu: GpuResult,
    /// The (perturbed) RGBA reference of the whole view.
    pub reference: Vec<u8>,
}

impl MapRun {
    /// The report lines and verdict (same rules as a synthetic case: a
    /// failed half fails the case; GPU errors win).
    pub fn verdict(&self) -> (Vec<String>, Status) {
        let mut lines = vec![format!("CPU binned: {}", self.binned)];
        let cpu_failed = self.binned.mismatched > 0;
        let gpu = match &self.gpu {
            GpuResult::NotWired => Status::GpuNotWired,
            GpuResult::NoAdapter(e) => {
                lines.push(format!("GPU: no adapter: {e}"));
                Status::NoAdapter(e.clone())
            }
            GpuResult::Error(e) => Status::Error(format!("GPU: {e}")),
            GpuResult::Compared { indices, rgba, .. } => {
                lines.push(format!("GPU indices: {indices}"));
                lines.push(format!("GPU: {rgba}"));
                if indices.mismatched == 0 && rgba.mismatched == 0 {
                    Status::Pass
                } else {
                    Status::Fail("GPU half".into())
                }
            }
        };
        let status = match (cpu_failed, gpu) {
            (_, Status::Error(e)) => Status::Error(e),
            (true, Status::Fail(_)) => Status::Fail("CPU and GPU halves".into()),
            (true, _) => Status::Fail("CPU half".into()),
            (false, s) => s,
        };
        (lines, status)
    }
}

/// A map to compare: the layout, its tile images, palette and view.
#[derive(Debug, Clone, Copy)]
pub struct MapScene<'a> {
    /// The DS1 layout.
    pub layout: &'a Layout,
    /// The tile images.
    pub lib: &'a TileLibrary,
    /// The palette.
    pub palette: &'a Palette,
    /// The whole-map view.
    pub view: cpu::View,
}

/// Compares the layout's CPU reference with the scene list composed
/// binned on the CPU and through `gpu`, chunk by chunk (`side`² pixels at
/// most). `perturb_n` corrupts the reference first (module docs). Errors:
/// a view with an odd side, a perturb count above the pixel count, a
/// scene error.
pub fn compare_layout(
    name: &str,
    map: MapScene<'_>,
    perturb_n: usize,
    side: u32,
    gpu: &mut dyn GpuCompositor,
) -> Result<MapRun, String> {
    let MapScene {
        layout,
        lib,
        palette,
        view,
    } = map;
    if !(view.width.is_multiple_of(2) && view.height.is_multiple_of(2)) {
        return Err("--view width and height must be even (pixel alignment)".into());
    }
    let whole = view_rect(view);
    let clean = cpu::render_indexed(layout, lib, view);
    let mut reference = cpu::to_rgba(&clean, palette);
    let mut indices = clean;
    perturb_indices(&mut indices, perturb_n)?;
    perturb(&mut reference, perturb_n)?;

    let rects = chunks(whole, side);
    let zero_bytes = ByteMismatch {
        bytes: 0,
        mismatched: 0,
        first: None,
    };
    let mut binned = zero_bytes;
    let mut gpu_indices = zero_bytes;
    let mut gpu_rgba = Mismatch {
        pixels: 0,
        mismatched: 0,
        first: None,
    };
    let mut image = vec![0u8; reference.len()];
    let mut stopped: Option<GpuResult> = None;
    let maps = MapTable::new();
    for rect in &rects {
        let c = chunk(layout, lib, whole, *rect);
        let bins = scene::bin(&c.items, &c.frames, &maps, c.view).map_err(|e| e.to_string())?;
        let want_idx = crop(&indices, whole, c.view, 1);
        let want_rgba = crop(&reference, whole, c.view, 4);
        let got = scene::compose_binned(&c.items, &bins, &c.frames, &maps, c.view)
            .map_err(|e| e.to_string())?;
        add_bytes(&mut binned, compare_indices(&got, &want_idx, c.view)?);
        if stopped.is_some() {
            continue;
        }
        let job = GpuJob {
            case: name,
            items: &c.items,
            bins: &bins,
            frames: &c.frames,
            maps: &maps,
            palette,
            view: c.view,
        };
        match gpu.compose(&job) {
            GpuOutcome::Image { indices: i, rgba } => {
                let compared = compare_indices(&i, &want_idx, c.view)
                    .and_then(|i| Ok((i, compare(&rgba, &want_rgba, c.view)?)));
                match compared {
                    Ok((i, m)) => {
                        add_bytes(&mut gpu_indices, i);
                        add_pixels(&mut gpu_rgba, m);
                        paste(&mut image, whole, c.view, &rgba, 4);
                    }
                    Err(e) => stopped = Some(GpuResult::Error(e)),
                }
            }
            GpuOutcome::NotWired => stopped = Some(GpuResult::NotWired),
            GpuOutcome::NoAdapter(e) => stopped = Some(GpuResult::NoAdapter(e)),
            GpuOutcome::Error(e) => stopped = Some(GpuResult::Error(e)),
        }
    }
    let gpu = stopped.unwrap_or(GpuResult::Compared {
        indices: gpu_indices,
        rgba: gpu_rgba,
        image,
    });
    Ok(MapRun {
        items: layout.items.len(),
        chunks: rects.len(),
        binned,
        gpu,
        reference,
    })
}

/// Writes `cpu.png` (the reference), `gpu.png` and, when they differ,
/// `diff.png` (red where pixels differ, the reference darkened elsewhere).
fn write_images(out: &Path, view: cpu::View, run: &MapRun) -> Result<()> {
    let (w, h) = (view.width, view.height);
    std::fs::create_dir_all(out)?;
    let save = |file: &str, rgba: &[u8]| {
        image::save_buffer(out.join(file), rgba, w, h, image::ExtendedColorType::Rgba8)
            .with_context(|| format!("writing {}", out.join(file).display()))
    };
    save("cpu.png", &run.reference)?;
    if let GpuResult::Compared { image, .. } = &run.gpu {
        save("gpu.png", image)?;
        if image != &run.reference {
            let mut diff = Vec::with_capacity(image.len());
            let a = image.as_chunks::<4>().0;
            let e = run.reference.as_chunks::<4>().0;
            for (a, e) in a.iter().zip(e) {
                if a == e {
                    diff.extend_from_slice(&[e[0] / 3, e[1] / 3, e[2] / 3, 255]);
                } else {
                    diff.extend_from_slice(&[255, 0, 0, 255]);
                }
            }
            save("diff.png", &diff)?;
        }
    }
    Ok(())
}

/// Runs a map case with its own headless compositor
/// ([`super::gpu::Wgpu`]); see [`run_with`].
pub fn run(name: &str, case: &MapCase, out: Option<PathBuf>, perturb_n: usize) -> CaseReport {
    run_with(name, case, out, perturb_n, &mut super::gpu::Wgpu::new())
}

/// Runs a map case through `gpu`; prints the verify lines as it goes. `out` defaults to `game/renders/verify-<ds1 stem>` (gitignored:
/// the images show game graphics).
pub fn run_with(
    name: &str,
    case: &MapCase,
    out: Option<PathBuf>,
    perturb_n: usize,
    gpu: &mut dyn GpuCompositor,
) -> CaseReport {
    let mut report = CaseReport {
        name: name.to_owned(),
        kind: "map",
        lines: Vec::new(),
        status: Status::Pass,
    };
    match run_inner(name, case, out, perturb_n, gpu) {
        Ok((lines, status)) => {
            report.lines = lines;
            report.status = status;
        }
        Err(e) => report.status = Status::Error(format!("{e:#}")),
    }
    report
}

fn run_inner(
    name: &str,
    case: &MapCase,
    out: Option<PathBuf>,
    perturb_n: usize,
    gpu: &mut dyn GpuCompositor,
) -> Result<(Vec<String>, Status)> {
    let archives = archives()?;
    let mut loaded = map::load(&archives, &case.ds1)?;
    let mut layout = map::build(&loaded.ds1, &loaded.library, case.wall_base);
    let (dcc, dc6) = map::load_sprites(&archives)?;
    map::add_sprites(&mut layout, &mut loaded.library, &loaded.ds1, &dcc, &dc6);
    let bounds = layout.bounds.context("map has nothing to draw")?;
    let view = match case.view {
        Some((left, top, width, height)) => cpu::View {
            left,
            top,
            width,
            height,
        },
        None => full_view(bounds),
    };
    println!(
        "verify {}: {} draw items, view {}x{} at {},{}",
        case.ds1,
        layout.items.len(),
        view.width,
        view.height,
        view.left,
        view.top
    );
    if perturb_n > 0 {
        println!("debug: corrupted {perturb_n} reference pixels; verify must FAIL");
    }
    let scene = MapScene {
        layout: &layout,
        lib: &loaded.library,
        palette: &loaded.palette,
        view,
    };
    let run = compare_layout(name, scene, perturb_n, CHUNK, gpu).map_err(anyhow::Error::msg)?;
    let out_dir = out
        .unwrap_or_else(|| PathBuf::from(format!("game/renders/verify-{}", file_stem(&case.ds1))));
    write_images(&out_dir, view, &run)?;
    println!("images in {}", out_dir.display());
    let (mut lines, status) = run.verdict();
    lines.insert(
        0,
        format!("{} chunks of at most {CHUNK}x{CHUNK}", run.chunks),
    );
    if status == Status::Pass {
        println!("PASS: GPU render matches the CPU reference exactly");
    }
    Ok((lines, status))
}

#[cfg(test)]
mod tests;
