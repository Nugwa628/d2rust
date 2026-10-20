// Spec: specs/client/render-pipeline.md (A9, A10, Test vectors)
// Spec: specs/render/composition.md (Test vectors)
// Spec: specs/render/shading.md (§4), specs/render/blend-modes.md (§2, §6)
//! CPU = GPU comparison on synthetic cases (repo only, no game data): the
//! spec's test vectors, an off-center view, two atlas pages and a stress
//! list; the frame-cycle and pixel-write vectors of `composition.md` (a
//! persistent base, the BlankScreen and post-draw clears, `L[P[s]]` and
//! `T[256 × d + P[s]]`) and every blend op with an asymmetric table; the
//! per-pixel light gradients of DT1 blocks (`shading.md` §4) and the
//! transposed translucent-wall read (`blend-modes.md` §2, §6). C6's verify
//! runner can call [`compare`] with its own cases; the
//! `gpu_compare` example and an ignored test run [`cases`].
//!
//! `--perturb N` (M08): N bytes of the CPU reference are changed before the
//! diff ([`perturb`]); the report must then show exactly N differing bytes.

use d2_formats::palette::{Palette, Rgb};

use super::pack::{pack, AtlasFrames};
use super::{Gpu, GpuError};
use crate::scene::{
    self, order, BlendOp, DrawItem, DrawKey, FrameCycle, FrameId, FrameImage, FramePlan,
    GradientKind, LightGradient, MapId, MapTable, Rect, ShadeChain,
};

/// Atlas pages a case may use.
pub const MAX_PAGES: u32 = 4;

/// One synthetic comparison case: everything the compositor reads.
#[derive(Debug, Clone)]
pub struct Case {
    pub name: &'static str,
    pub frames: Vec<FrameImage>,
    pub maps: MapTable,
    /// In draw order ([`scene::order`] already applied).
    pub items: Vec<DrawItem>,
    pub view: Rect,
    pub palette: Palette,
    /// The previous frame (view sized); `None`: all index 0.
    pub base: Option<Vec<u8>>,
    /// The frame's clears (`composition.md` §3).
    pub plan: FramePlan,
}

impl Case {
    /// The start framebuffer of the frame.
    pub fn base(&self) -> Vec<u8> {
        self.base
            .clone()
            .unwrap_or_else(|| vec![0; self.view.width as usize * self.view.height as usize])
    }

    /// The CPU reference of the case (`scene::compose_frame`).
    pub fn reference(&self) -> Result<Vec<u8>, scene::SceneError> {
        scene::compose_frame(
            &self.items,
            &self.frames,
            &self.maps,
            self.view,
            &self.base(),
            self.plan,
        )
    }
}

/// Byte differences between two equal-length buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Diff {
    /// Differing bytes (index buffer) or differing pixels (RGBA).
    pub differing: u64,
    /// First differing pixel `(x, y)` in the view, with both values.
    pub first: Option<(u32, u32, u32, u32)>,
}

/// The result of one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub name: String,
    pub pixels: usize,
    pub items: usize,
    pub pages: u32,
    /// Bytes of the CPU reference changed on purpose.
    pub perturbed: usize,
    /// Index framebuffer, per byte.
    pub indices: Diff,
    /// RGBA image, per pixel.
    pub rgba: Diff,
}

impl Report {
    /// Both images agree byte for byte.
    pub fn is_match(&self) -> bool {
        self.indices.differing == 0 && self.rgba.differing == 0
    }
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "case {}: {} pixels, {} items, {} pages: {} differing bytes (indices), {} differing pixels (rgba)",
            self.name, self.pixels, self.items, self.pages, self.indices.differing, self.rgba.differing
        )?;
        if self.perturbed > 0 {
            write!(f, ", {} perturbed", self.perturbed)?;
        }
        if let Some((x, y, cpu, gpu)) = self.indices.first {
            write!(f, "; first at ({x},{y}): cpu {cpu} gpu {gpu}")?;
        }
        Ok(())
    }
}

/// Runs `case` on the CPU reference and on `gpu` and diffs both images,
/// after changing `perturb` bytes of the CPU reference.
pub fn compare(gpu: &Gpu, case: &Case, perturb_count: usize) -> Result<Report, GpuError> {
    let mut cpu = case.reference()?;
    let perturbed = perturb(&mut cpu, perturb_count);
    let cpu_rgba = scene::to_rgba(&cpu, &case.palette);
    let (atlas, packed) = prepare(case)?;
    let (indices, rgba) = gpu.compose_rgba(&packed, atlas.pages(), &case.palette)?;
    Ok(Report {
        name: case.name.to_string(),
        pixels: cpu.len(),
        items: case.items.len(),
        pages: atlas.page_count(),
        perturbed,
        indices: diff(&cpu, &indices, case.view.width, 1),
        rgba: diff(&cpu_rgba, &rgba, case.view.width, 4),
    })
}

/// The atlas and packed buffers of `case` (everything up to dispatch).
pub fn prepare(case: &Case) -> Result<(AtlasFrames, super::Packed), GpuError> {
    let atlas = AtlasFrames::from_images(&case.frames, MAX_PAGES)?;
    let bins = scene::bin(&case.items, &case.frames, &case.maps, case.view)?;
    let packed = pack(
        &case.items,
        &bins,
        &case.frames,
        &atlas.slots,
        &case.maps,
        atlas.page_count(),
    )?
    .with_frame(&case.base(), case.plan)?;
    Ok((atlas, packed))
}

/// Compares pixels of `bpp` bytes. A length mismatch counts every pixel of
/// the longer buffer past the shorter one as differing.
pub fn diff(cpu: &[u8], gpu: &[u8], width: u32, bpp: usize) -> Diff {
    let mut d = Diff::default();
    let pixels = cpu.len().max(gpu.len()).div_ceil(bpp);
    for p in 0..pixels {
        let a = cpu.get(p * bpp..(p + 1) * bpp);
        let b = gpu.get(p * bpp..(p + 1) * bpp);
        if a == b {
            continue;
        }
        d.differing += 1;
        if d.first.is_none() {
            let value = |s: Option<&[u8]>| {
                s.map_or(u32::MAX, |s| {
                    s.iter().fold(0u32, |v, &b| (v << 8) | u32::from(b))
                })
            };
            let width = width.max(1) as usize;
            d.first = Some(((p % width) as u32, (p / width) as u32, value(a), value(b)));
        }
    }
    d
}

/// Changes `n` bytes of `buf` (all of them if `n` is larger), evenly
/// spaced from byte 0, each XOR 0xFF so every one differs. Returns how many
/// bytes were changed.
pub fn perturb(buf: &mut [u8], n: usize) -> usize {
    let n = n.min(buf.len());
    if n == 0 {
        return 0;
    }
    let step = buf.len() / n;
    for i in 0..n {
        buf[i * step] ^= 0xFF;
    }
    n
}

/// A palette whose entries are all distinct, so an index change always
/// changes the RGBA pixel.
pub fn distinct_palette() -> Palette {
    let mut p = Palette {
        colors: [Rgb::default(); 256],
    };
    for (i, c) in p.colors.iter_mut().enumerate() {
        let i = i as u8;
        *c = Rgb {
            r: i,
            g: 255 - i,
            b: i ^ 0x5a,
        };
    }
    p
}

/// An identity map row with some entries replaced.
pub fn map_with(pairs: &[(u8, u8)]) -> [u8; 256] {
    let mut row = [0u8; 256];
    for (i, v) in row.iter_mut().enumerate() {
        *v = i as u8;
    }
    for &(from, to) in pairs {
        row[usize::from(from)] = to;
    }
    row
}

fn solid(width: u32, height: u32, index: u8) -> FrameImage {
    FrameImage {
        width,
        height,
        pixels: vec![index; (width * height) as usize],
    }
}

/// A synthetic 256×256 blend table: every entry depends on both indices.
fn blend_table(seed: u32) -> Box<[[u8; 256]; 256]> {
    let mut t = Box::new([[0u8; 256]; 256]);
    for (s, row) in t.iter_mut().enumerate() {
        for (d, v) in row.iter_mut().enumerate() {
            *v = ((s as u32 * 31 + d as u32 * 7 + seed * 13) % 256) as u8;
        }
    }
    t
}

fn case(
    name: &'static str,
    frames: Vec<FrameImage>,
    maps: MapTable,
    mut items: Vec<DrawItem>,
    view: Rect,
) -> Case {
    order(&mut items);
    Case {
        name,
        frames,
        maps,
        items,
        view,
        palette: distinct_palette(),
        base: None,
        plan: FramePlan::NONE,
    }
}

/// The 16:9 play frame (1068 × 600): a full-frame column ramp (every
/// pixel non-zero, each column its own index) under a sprite that crosses
/// the right edge and the last, partial bin column (1068 = 33 × 32 + 12).
pub fn wide_case() -> Case {
    let view = Rect::new(0, 0, 1068, 600);
    let ramp = FrameImage {
        width: 1068,
        height: 600,
        pixels: (0..1068u32 * 600)
            .map(|i| (i % 1068 % 250 + 1) as u8)
            .collect(),
    };
    let mut items = vec![
        DrawItem::new(FrameId(0), 0, 0),
        DrawItem::new(FrameId(1), 1050, 290),
    ];
    for it in &mut items {
        it.clip = view;
    }
    case(
        "wide-frame",
        vec![ramp, solid(40, 20, 251)],
        MapTable::new(),
        items,
        view,
    )
}

fn key(major: u32) -> DrawKey {
    DrawKey::new(2, major, 0, 0).expect("small key")
}

/// Deterministic generator for the stress case (not game randomness).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// All synthetic cases, in a fixed order.
pub fn cases() -> Vec<Case> {
    const SMALL: Rect = Rect::new(0, 0, 32, 24);
    let spec_frame = || FrameImage {
        width: 2,
        height: 2,
        pixels: vec![0, 5, 7, 0],
    };
    let mut out = Vec::new();

    // §Test vectors 1–3: opaque frame, chain of one, chain order.
    out.push(case(
        "vector-opaque",
        vec![spec_frame()],
        MapTable::new(),
        vec![DrawItem::new(FrameId(0), 10, 10)],
        SMALL,
    ));
    let mut maps = MapTable::new();
    let m = maps.push(map_with(&[(5, 9)]));
    let mut item = DrawItem::new(FrameId(0), 10, 10);
    item.shade = ShadeChain::new(&[m]).expect("chain");
    out.push(case(
        "vector-chain1",
        vec![spec_frame()],
        maps,
        vec![item],
        SMALL,
    ));
    let mut maps = MapTable::new();
    let m1 = maps.push(map_with(&[(5, 9)]));
    let m2 = maps.push(map_with(&[(9, 3)]));
    let mut item = DrawItem::new(FrameId(0), 10, 10);
    item.shade = ShadeChain::new(&[m1, m2]).expect("chain");
    out.push(case(
        "vector-chain2",
        vec![spec_frame()],
        maps,
        vec![item],
        SMALL,
    ));

    // §Test vectors 4–5: key order, then equal keys in build order.
    let mut a = DrawItem::new(FrameId(0), 4, 4);
    a.key = key(2);
    let mut b = DrawItem::new(FrameId(1), 5, 5);
    b.key = key(1);
    out.push(case(
        "vector-key-order",
        vec![solid(3, 3, 1), solid(3, 3, 2)],
        MapTable::new(),
        vec![a, b],
        SMALL,
    ));
    out.push(case(
        "vector-equal-keys",
        vec![solid(3, 3, 1), solid(3, 3, 2)],
        MapTable::new(),
        vec![
            DrawItem::new(FrameId(1), 5, 5),
            DrawItem::new(FrameId(0), 4, 4),
        ],
        SMALL,
    ));

    // §Test vectors 6: IndexTable over an opaque background.
    let mut maps = MapTable::new();
    let base = maps.push_table(&blend_table(1));
    let mut top = DrawItem::new(FrameId(1), 6, 6);
    top.blend = BlendOp::IndexTable(base);
    out.push(case(
        "vector-index-table",
        vec![solid(20, 16, 40), spec_frame_grid()],
        maps,
        vec![DrawItem::new(FrameId(0), 2, 2), top],
        SMALL,
    ));

    // §Test vectors 7: clip.
    let mut item = DrawItem::new(FrameId(0), 3, 3);
    item.clip = Rect::new(5, 4, 6, 3);
    out.push(case(
        "vector-clip",
        vec![solid(12, 12, 77)],
        MapTable::new(),
        vec![item],
        SMALL,
    ));

    // §Test vectors 8: a frame spanning four bins, full frame (partial
    // last bin row: 600 = 18 × 32 + 24).
    out.push(case(
        "vector-four-bins",
        vec![spec_frame_grid()],
        MapTable::new(),
        vec![DrawItem::new(FrameId(0), 20, 20)],
        Rect::FRAME,
    ));

    // Empty list: the clear value everywhere.
    out.push(case(
        "empty",
        Vec::new(),
        MapTable::new(),
        Vec::new(),
        Rect::FRAME,
    ));

    // Two atlas pages: the second frame cannot share page 0.
    let big = |seed: u32| FrameImage {
        width: 1500,
        height: 1500,
        pixels: (0..1500u32 * 1500)
            .map(|i| ((i * 7 + seed + i / 1500) % 251) as u8)
            .collect(),
    };
    out.push(case(
        "two-pages",
        vec![big(1), big(2)],
        MapTable::new(),
        vec![
            DrawItem::new(FrameId(0), -700, -800),
            DrawItem::new(FrameId(1), 300, 250),
        ],
        Rect::FRAME,
    ));

    out.push(stress("offset-view", 7, 120, Rect::new(-37, 13, 333, 250)));
    out.push(stress("stress", 1, 400, Rect::FRAME));
    out.extend(composition_cases());
    out
}

/// `composition.md` Test vectors and every blend op, as frames of the
/// frame cycle on the 800 × 600 framebuffer.
fn composition_cases() -> Vec<Case> {
    let mut out = Vec::new();
    let all_5 = vec![5u8; (scene::FRAME_WIDTH * scene::FRAME_HEIGHT) as usize];
    let cycle = |blank: bool, post_clear: u32| {
        let mut c = FrameCycle::with_pixels(scene::FRAME_WIDTH, scene::FRAME_HEIGHT, all_5.clone())
            .expect("800 × 600");
        c.set_post_clear(post_clear);
        c.plan(blank)
    };
    // §3 vectors: all 5, nothing drawn; BlankScreen 1, BlankScreen 0, the
    // post-draw counter at 1. A drawn item across the cleared edge (rows
    // 550..556) shows what the clear does under draws.
    let edge = || vec![DrawItem::new(FrameId(0), 390, 550)];
    for (name, blank, post_clear) in [
        ("frame-blank-screen", true, 0),
        ("frame-no-clear", false, 0),
        ("frame-post-clear", true, 1),
    ] {
        let mut c = case(
            name,
            vec![solid(20, 6, 9)],
            MapTable::new(),
            edge(),
            Rect::FRAME,
        );
        c.base = Some(all_5.clone());
        c.plan = cycle(blank, post_clear);
        out.push(c);
    }

    // §5 vectors: s = 7, P[7] = 9, L[9] = 3, no T → 3; s = 7, P[7] = 9,
    // T, d = 200 → T[256 × 200 + 9]. One pixel each over a base of 200.
    let mut maps = MapTable::new();
    let p = maps.push(map_with(&[(7, 9)]));
    let l = maps.push(map_with(&[(9, 3)]));
    let t = maps.push_table(&blend_table(4));
    let one = FrameImage {
        width: 1,
        height: 1,
        pixels: vec![7],
    };
    let mut pl = DrawItem::new(FrameId(0), 10, 10);
    pl.shade = ShadeChain::new(&[p, l]).expect("chain");
    let mut pt = DrawItem::new(FrameId(0), 11, 10);
    pt.shade = ShadeChain::new(&[p]).expect("chain");
    pt.blend = BlendOp::IndexTable(t);
    let mut c = case(
        "pixel-write",
        vec![one],
        maps,
        vec![pl, pt],
        Rect::new(0, 0, 32, 32),
    );
    c.base = Some(vec![200; 32 * 32]);
    out.push(c);

    // Every blend op over every (dest, src) pair: dest row y = y, source
    // column x = x (column 0 transparent), an asymmetric table, so a
    // swapped row / column shows.
    let mut maps = MapTable::new();
    let t = maps.push_table(&blend_table(5));
    let dest = FrameImage {
        width: 256,
        height: 256,
        pixels: (0..256 * 256u32).map(|p| (p / 256) as u8).collect(),
    };
    let src = FrameImage {
        width: 256,
        height: 256,
        pixels: (0..256 * 256u32).map(|p| (p % 256) as u8).collect(),
    };
    let mut table = DrawItem::new(FrameId(1), 0, 0);
    table.blend = BlendOp::IndexTable(t);
    let opaque = DrawItem::new(FrameId(1), 0, 256);
    let mut c = case(
        "blend-ops",
        vec![dest, src],
        maps,
        vec![
            DrawItem::new(FrameId(0), 0, 0),
            DrawItem::new(FrameId(0), 0, 256),
            table,
            opaque,
        ],
        Rect::new(0, 0, 256, 512),
    );
    c.base = Some((0..256 * 512u32).map(|i| (i * 13 % 251) as u8).collect());
    out.push(c);

    out.extend(shading_cases());
    out.push(gdi_case());

    // The stress list as one frame of a running cycle: a random previous
    // frame, BlankScreen clear, draws over both the cleared and the kept
    // rows.
    let mut c = stress("frame-stress", 3, 300, Rect::FRAME);
    let mut rng = Lcg(99);
    c.base = Some(
        (0..scene::FRAME_WIDTH * scene::FRAME_HEIGHT)
            .map(|_| rng.below(256) as u8)
            .collect(),
    );
    c.plan = cycle(true, 0);
    out.push(c);
    out
}

/// Light gradients and the transposed table read: wall and RLE floor
/// blocks clipped out of a larger tile image (some blocks partly off the
/// view), translucent walls over them, and a random list of both.
fn shading_cases() -> Vec<Case> {
    let mut out = Vec::new();
    let mut rng = Lcg(0x5EED);
    let mut maps = MapTable::new();
    let mut light = [[0u8; 256]; 32];
    for (k, row) in light.iter_mut().enumerate() {
        for (i, v) in row.iter_mut().enumerate() {
            *v = ((i * (k + 1) / 32 + k * 3) % 256) as u8;
        }
    }
    let light0 = maps.push(light[0]);
    for row in &light[1..] {
        maps.push(*row);
    }
    let a0 = maps.push_table(&blend_table(6));
    let a2 = maps.push_table(&blend_table(7));
    let tile = FrameImage {
        width: 160,
        height: 96,
        pixels: (0..160 * 96u32)
            .map(|i| {
                if i % 13 == 5 {
                    0
                } else {
                    (i * 7 % 255 + 1) as u8
                }
            })
            .collect(),
    };
    let view = Rect::new(-20, -10, 200, 140);
    let mut items = vec![DrawItem::new(FrameId(1), view.x, view.y)];
    // Opaque gradient walls, then RLE floor blocks, then translucent walls
    // read transposed, each block a clip of the tile image at (-10, -6).
    let (tx, ty) = (-10, -6);
    for (n, (kind, op)) in [
        (GradientKind::Wall, None),
        (GradientKind::RleFloor, None),
        (GradientKind::Wall, Some(a0)),
        (GradientKind::Wall, Some(a2)),
    ]
    .into_iter()
    .enumerate()
    {
        for bx in 0..5 {
            let rows = kind.rows() as i32;
            let (x, y) = (tx + 32 * bx, ty + 24 * n as i32);
            let mut it = DrawItem::new(FrameId(0), tx, ty);
            it.clip = Rect::new(x, y, 32, rows as u32);
            let corners = [0, 1, 2, 3].map(|_| rng.below(256) as u8);
            it.shade = ShadeChain::new(&[])
                .expect("chain")
                .with_gradient(LightGradient {
                    kind,
                    x,
                    y,
                    corners,
                    light0,
                });
            if let Some(t) = op {
                it.blend = BlendOp::IndexTableSrcRow(t);
            }
            items.push(it);
        }
    }
    let ground = FrameImage {
        width: 200,
        height: 140,
        pixels: (0..200 * 140u32).map(|i| (i * 31 % 256) as u8).collect(),
    };
    out.push(case(
        "shading-blocks",
        vec![tile.clone(), ground],
        maps.clone(),
        items,
        view,
    ));

    // Random gradients, chains and ops, both table orientations.
    let mut items = Vec::new();
    let remap = maps.push(blend_table(8)[3]);
    for _ in 0..250 {
        let kind = if rng.below(2) == 0 {
            GradientKind::Wall
        } else {
            GradientKind::RleFloor
        };
        let x = -40 + rng.below(320) as i32;
        let y = -40 + rng.below(240) as i32;
        let (bx, by) = (x + rng.below(40) as i32, y + rng.below(30) as i32);
        let mut it = DrawItem::new(FrameId(0), x, y);
        // A clip inside the block, sometimes smaller than it.
        let (cx, cy) = (rng.below(8) as i32, rng.below(4) as i32);
        it.clip = Rect::new(bx + cx, by + cy, 32 - cx as u32, kind.rows() - cy as u32);
        let chain = if rng.below(3) == 0 {
            vec![remap]
        } else {
            Vec::new()
        };
        it.shade = ShadeChain::new(&chain).expect("chain");
        if rng.below(4) != 0 {
            it.shade = it.shade.with_gradient(LightGradient {
                kind,
                x: bx,
                y: by,
                corners: [0, 1, 2, 3].map(|_| rng.below(256) as u8),
                light0,
            });
        }
        it.blend = match rng.below(3) {
            0 => BlendOp::Opaque,
            1 => BlendOp::IndexTable([a0, a2][rng.below(2) as usize]),
            _ => BlendOp::IndexTableSrcRow([a0, a2][rng.below(2) as usize]),
        };
        items.push(it);
    }
    let mut c = case(
        "shading-stress",
        vec![tile],
        maps,
        items,
        Rect::new(0, 0, 256, 160),
    );
    c.base = Some((0..256 * 160u32).map(|_| rng.below(256) as u8).collect());
    out.push(c);
    out
}

/// GDI lines and rectangles (`blend-modes.md` §8) as built by
/// `rules::blend`: opaque color lines (color 0 included) clipped to the
/// surface, and rectangles of every per-mode value `k` (0: color, 1:
/// `T[d]` through chain `[Z]` and the transposed read, 2: `T[256·d +
/// color]`) over a varied base.
fn gdi_case() -> Case {
    use crate::rules::blend;
    use crate::rules::camera::FrameSize;
    use crate::rules::shading::ShadeTables;
    use d2_formats::palette::Pl2;

    let row = |seed: u32| -> Vec<[u8; 256]> { blend_table(seed).to_vec() };
    let ident = map_with(&[]);
    let pl2 = Pl2 {
        base_palette: distinct_palette(),
        light_levels: vec![ident; 32],
        inventory_variations: vec![ident; 16],
        selected_unit_shift: ident,
        alpha_blend: vec![row(11), row(12), row(13)],
        additive_blend: row(14),
        multiplicative_blend: row(15),
        hue_variations: vec![ident; 111],
        red_tones: ident,
        green_tones: ident,
        blue_tones: ident,
        unknown_variations: vec![ident; 14],
        max_component_blend: row(16),
        darkened_shift: ident,
        text_colors: Vec::new(),
        text_color_shifts: Vec::new(),
    };
    let mut maps = MapTable::new();
    let t = ShadeTables::push(&mut maps, &pl2);
    let size = FrameSize {
        width: 96,
        height: 64,
    };
    let mut draws = Vec::new();
    for (n, mode) in [0u8, 1, 2, 3, 4, 5, 6, 7, 9].into_iter().enumerate() {
        let c = maps.push(blend::color_row(n as u8 * 29));
        let x = (n as i32 % 3) * 34 - 6;
        let y = (n as i32 / 3) * 22 - 3;
        draws.extend(blend::gdi_rectangle(&t, size, c, x, y, x + 30, y + 20, mode).expect("rect"));
    }
    for (n, (x0, y0, x1, y1)) in [
        (-10, 5, 120, 40),
        (50, -20, 40, 90),
        (3, 60, 90, 2),
        (95, 63, 95, 63),
        (0, 0, 0, 0),
        (70, 10, -5, 13),
    ]
    .into_iter()
    .enumerate()
    {
        let c = maps.push(blend::color_row(n as u8 * 41));
        draws.extend(blend::gdi_line(size, c, x0, y0, x1, y1).expect("line"));
    }
    let frames: Vec<FrameImage> = draws.iter().map(|d| d.image.clone()).collect();
    let items: Vec<DrawItem> = draws
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let mut it = d.item(FrameId(i as u32));
            it.key = key(i as u32);
            it
        })
        .collect();
    let mut c = case("gdi-lines-rects", frames, maps, items, size.rect());
    c.base = Some((0..96 * 64u32).map(|i| (i * 37 % 253 + 1) as u8).collect());
    c
}

/// A 40×40 frame with transparent holes and every index 0..=255.
fn spec_frame_grid() -> FrameImage {
    FrameImage {
        width: 40,
        height: 40,
        pixels: (0..1600u32)
            .map(|i| if i % 11 == 3 { 0 } else { (i % 256) as u8 })
            .collect(),
    }
}

/// Random frames, chains, blends, clips and keys, some items off screen,
/// one empty frame.
pub fn stress(name: &'static str, seed: u64, count: u32, view: Rect) -> Case {
    let mut rng = Lcg(seed);
    let mut frames = vec![FrameImage {
        width: 0,
        height: 0,
        pixels: Vec::new(),
    }];
    for _ in 0..24 {
        let (w, h) = (1 + rng.below(120), 1 + rng.below(90));
        let pixels = (0..w * h)
            .map(|_| {
                if rng.below(10) < 3 {
                    0
                } else {
                    rng.below(256) as u8
                }
            })
            .collect();
        frames.push(FrameImage {
            width: w,
            height: h,
            pixels,
        });
    }
    let mut maps = MapTable::new();
    let mut shade_maps = Vec::new();
    for _ in 0..8 {
        let mut row = [0u8; 256];
        for v in row.iter_mut() {
            *v = rng.below(256) as u8;
        }
        shade_maps.push(maps.push(row));
    }
    let tables = [
        maps.push_table(&blend_table(2)),
        maps.push_table(&blend_table(3)),
    ];
    let mut items = Vec::new();
    for _ in 0..count {
        let frame = FrameId(rng.below(frames.len() as u32));
        let x = view.x - 150 + rng.below(view.width + 300) as i32;
        let y = view.y - 120 + rng.below(view.height + 240) as i32;
        let mut item = DrawItem::new(frame, x, y);
        let chain: Vec<MapId> = (0..rng.below(5))
            .map(|_| shade_maps[rng.below(8) as usize])
            .collect();
        item.shade = ShadeChain::new(&chain).expect("≤ 4 maps");
        if rng.below(4) == 0 {
            item.blend = BlendOp::IndexTable(tables[rng.below(2) as usize]);
        }
        if rng.below(3) == 0 {
            item.clip = Rect::new(
                view.x + rng.below(view.width) as i32 - 20,
                view.y + rng.below(view.height) as i32 - 20,
                rng.below(300),
                rng.below(200),
            );
        }
        item.key = DrawKey::new(rng.below(3), rng.below(40), rng.below(4), 0).expect("key");
        items.push(item);
    }
    case(name, frames, maps, items, view)
}
