//! Tests for the scene module: the CPU reference compositor, shade chains, blend ops, draw order,
//! screen bins and the persistent-framebuffer frame cycle.
//!
//! They use small hand-made frames and tables with known results, check that binned composition
//! equals plain composition, and check the strict-input rules: every invalid input is reported
//! as exactly one error naming the item, and changing N visible source pixels changes exactly N
//! output pixels.

use d2_formats::palette::{Palette, Rgb};

use super::*;

const W: u32 = 32;
const H: u32 = 24;
const VIEW: Rect = Rect::new(0, 0, W, H);

/// A palette whose every entry is distinct, so RGBA reveals the index.
fn palette() -> Palette {
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

fn rgb_at(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 3] {
    let o = (y * width + x) as usize * 4;
    assert_eq!(rgba[o + 3], 255);
    [rgba[o], rgba[o + 1], rgba[o + 2]]
}

fn color(p: &Palette, i: u8) -> [u8; 3] {
    let c = p.colors[usize::from(i)];
    [c.r, c.g, c.b]
}

fn frames() -> Vec<FrameImage> {
    vec![
        // 0: a 2×2 frame `[0,5;7,0]`.
        FrameImage {
            width: 2,
            height: 2,
            pixels: vec![0, 5, 7, 0],
        },
        // 1: solid 3×3 of index 1.
        FrameImage {
            width: 3,
            height: 3,
            pixels: vec![1; 9],
        },
        // 2: solid 3×3 of index 2.
        FrameImage {
            width: 3,
            height: 3,
            pixels: vec![2; 9],
        },
    ]
}

fn map_with(pairs: &[(u8, u8)]) -> [u8; 256] {
    let mut row = [0u8; 256];
    for (i, v) in row.iter_mut().enumerate() {
        *v = i as u8;
    }
    for &(from, to) in pairs {
        row[usize::from(from)] = to;
    }
    row
}

fn key(major: u32) -> DrawKey {
    DrawKey::new(2, major, 0, 0).unwrap()
}

#[test]
fn opaque_frame_vector() {
    let pal = palette();
    let items = [DrawItem::new(FrameId(0), 10, 10)];
    let rgba = compose_rgba(&items, &frames(), &MapTable::new(), &pal, VIEW).unwrap();
    assert_eq!(rgba.len(), (W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let want = match (x, y) {
                (11, 10) => 5,
                (10, 11) => 7,
                _ => 0, // background
            };
            assert_eq!(rgb_at(&rgba, W, x, y), color(&pal, want), "({x},{y})");
        }
    }
}

#[test]
fn shade_chain_one_map() {
    let pal = palette();
    let mut maps = MapTable::new();
    let m = maps.push(map_with(&[(5, 9)]));
    let mut item = DrawItem::new(FrameId(0), 10, 10);
    item.shade = ShadeChain::new(&[m]).unwrap();
    let rgba = compose_rgba(&[item], &frames(), &maps, &pal, VIEW).unwrap();
    assert_eq!(rgb_at(&rgba, W, 11, 10), color(&pal, 9));
    assert_eq!(rgb_at(&rgba, W, 10, 11), color(&pal, 7));
}

#[test]
fn shade_chain_order() {
    let pal = palette();
    let mut maps = MapTable::new();
    let m1 = maps.push(map_with(&[(5, 9)]));
    let m2 = maps.push(map_with(&[(9, 3)]));
    let mut item = DrawItem::new(FrameId(0), 10, 10);
    item.shade = ShadeChain::new(&[m1, m2]).unwrap();
    let rgba = compose_rgba(&[item], &frames(), &maps, &pal, VIEW).unwrap();
    assert_eq!(rgb_at(&rgba, W, 11, 10), color(&pal, 3));
    // Reversed order: m2 leaves 5 alone, then m1 gives 9.
    item.shade = ShadeChain::new(&[m2, m1]).unwrap();
    let rgba = compose_rgba(&[item], &frames(), &maps, &pal, VIEW).unwrap();
    assert_eq!(rgb_at(&rgba, W, 11, 10), color(&pal, 9));
}

#[test]
fn zero_is_tested_before_the_chain() {
    let mut maps = MapTable::new();
    // m[0] = 4 would paint transparent pixels if 0 went through the chain;
    // m[7] = 0 shows the neutral rule for a mapped 0 (drawn as index 0).
    let m = maps.push(map_with(&[(0, 4), (7, 0)]));
    let mut under = DrawItem::new(FrameId(1), 10, 10);
    under.key = key(0);
    let mut item = DrawItem::new(FrameId(0), 10, 10);
    item.key = key(1);
    item.shade = ShadeChain::new(&[m]).unwrap();
    let out = compose(&[under, item], &frames(), &maps, VIEW).unwrap();
    let at = |x: u32, y: u32| out[(y * W + x) as usize];
    assert_eq!(at(10, 10), 1); // transparent: underlying item shows
    assert_eq!(at(11, 11), 1);
    assert_eq!(at(11, 10), 5);
    // A drawn pixel mapped to 0 is written as opaque index 0.
    assert_eq!(at(10, 11), 0);
}

#[test]
fn chain_of_four_and_too_long() {
    let mut maps = MapTable::new();
    let ms: Vec<MapId> = (0..4u8)
        .map(|k| maps.push(map_with(&[(5 + k, 6 + k)])))
        .collect();
    let mut item = DrawItem::new(FrameId(0), 0, 0);
    item.shade = ShadeChain::new(&ms).unwrap();
    assert_eq!(item.shade.maps(), &ms[..]);
    let out = compose(&[item], &frames(), &maps, VIEW).unwrap();
    assert_eq!(out[1], 9);
    assert_eq!(
        ShadeChain::new(&[MapId(0); 5]),
        Err(SceneError::ShadeChainTooLong { len: 5 })
    );
}

#[test]
fn higher_key_draws_on_top() {
    // Built in the wrong order; `order` puts k1 < k2 first.
    let mut top = DrawItem::new(FrameId(2), 1, 1);
    top.key = key(7);
    let mut bottom = DrawItem::new(FrameId(1), 0, 0);
    bottom.key = key(3);
    let mut items = vec![top, bottom];
    order(&mut items);
    assert_eq!(items[0].key, key(3));
    let out = compose(&items, &frames(), &MapTable::new(), VIEW).unwrap();
    let at = |x: u32, y: u32| out[(y * W + x) as usize];
    assert_eq!(at(0, 0), 1);
    assert_eq!(at(1, 1), 2); // overlap: the second visible
    assert_eq!(at(2, 2), 2);
    assert_eq!(at(3, 3), 2);
}

#[test]
fn equal_keys_keep_build_order() {
    let mut items: Vec<DrawItem> = (0..6)
        .map(|n| {
            let mut it = DrawItem::new(FrameId(1 + n % 2), 0, 0);
            it.key = key(if n == 3 { 0 } else { 5 });
            it.tag = ItemTag::Ui(n);
            it
        })
        .collect();
    order(&mut items);
    let tags: Vec<ItemTag> = items.iter().map(|i| i.tag).collect();
    let want = [3, 0, 1, 2, 4, 5].map(ItemTag::Ui);
    assert_eq!(tags, want);
    // Last of the equal keys is n = 5 (frame 2).
    let out = compose(&items, &frames(), &MapTable::new(), VIEW).unwrap();
    assert_eq!(out[0], 2);
}

#[test]
fn key_layout() {
    let k = DrawKey::new(15, DrawKey::MAJOR_MAX, DrawKey::MINOR_MAX, 255).unwrap();
    assert_eq!(k.0, u64::MAX);
    let k = DrawKey::new(3, 0x0abc_def1, 0x12_3456, 0x78).unwrap();
    assert_eq!(k.0, 0x3abc_def1_1234_5678);
    assert_eq!(
        (k.pass(), k.major(), k.minor(), k.sub()),
        (3, 0x0abc_def1, 0x12_3456, 0x78)
    );
    // Field significance: pass > major > minor > sub.
    let a = DrawKey::new(1, 0, 0, 0).unwrap();
    let b = DrawKey::new(0, DrawKey::MAJOR_MAX, DrawKey::MINOR_MAX, 255).unwrap();
    assert!(a > b);
    let a = DrawKey::new(0, 1, 0, 0).unwrap();
    let b = DrawKey::new(0, 0, DrawKey::MINOR_MAX, 255).unwrap();
    assert!(a > b);
    assert!(DrawKey::new(0, 0, 1, 0).unwrap() > DrawKey::new(0, 0, 0, 255).unwrap());
}

/// Synthetic 256×256 blend table with distinct-looking entries.
fn blend_table() -> Box<[[u8; 256]; 256]> {
    let mut t = Box::new([[0u8; 256]; 256]);
    for (s, row) in t.iter_mut().enumerate() {
        for (d, v) in row.iter_mut().enumerate() {
            *v = (s * 7 + d * 13 + 1) as u8;
        }
    }
    t
}

#[test]
fn index_table_every_src_and_dest() {
    let table = blend_table();
    let mut maps = MapTable::new();
    maps.push(map_with(&[])); // rows before the table: base is not 0
    let base = maps.push_table(&table);
    assert_eq!(base, MapId(1));
    // Destination: row y holds index y (row 0 is transparent → clear 0).
    // Source: column x holds index x (column 0 transparent → dest kept).
    let dest = FrameImage {
        width: 256,
        height: 256,
        pixels: (0..256 * 256).map(|p| (p / 256) as u8).collect(),
    };
    let src = FrameImage {
        width: 256,
        height: 256,
        pixels: (0..256 * 256).map(|p| (p % 256) as u8).collect(),
    };
    let frames = vec![dest, src];
    let mut over = DrawItem::new(FrameId(1), 0, 0);
    over.blend = BlendOp::IndexTable(base);
    let view = Rect::new(0, 0, 256, 256);
    let mut items = [DrawItem::new(FrameId(0), 0, 0), over];
    items[0].clip = view;
    items[1].clip = view;
    let out = compose(&items, &frames, &maps, view).unwrap();
    for d in 0..256usize {
        for s in 0..256usize {
            // Row = destination, column = source.
            let want = if s == 0 { d as u8 } else { table[d][s] };
            assert_eq!(out[d * 256 + s], want, "src {s} dest {d}");
        }
    }
}

/// `IndexTableSrcRow`: the same table read transposed, `dest =
/// table[src][dest]` (the lit translucent wall drawer).
#[test]
fn index_table_src_row_every_src_and_dest() {
    let table = blend_table();
    let mut maps = MapTable::new();
    maps.push(map_with(&[]));
    let base = maps.push_table(&table);
    let dest = FrameImage {
        width: 256,
        height: 256,
        pixels: (0..256 * 256).map(|p| (p / 256) as u8).collect(),
    };
    let src = FrameImage {
        width: 256,
        height: 256,
        pixels: (0..256 * 256).map(|p| (p % 256) as u8).collect(),
    };
    let mut over = DrawItem::new(FrameId(1), 0, 0);
    over.blend = BlendOp::IndexTableSrcRow(base);
    let view = Rect::new(0, 0, 256, 256);
    let items = [DrawItem::new(FrameId(0), 0, 0), over];
    let out = compose(&items, &vec![dest, src], &maps, view).unwrap();
    for d in 0..256usize {
        for s in 0..256usize {
            let want = if s == 0 { d as u8 } else { table[s][d] };
            assert_eq!(out[d * 256 + s], want, "src {s} dest {d}");
        }
    }
    // A table past the map table is refused like `IndexTable`.
    let mut bad = DrawItem::new(FrameId(1), 0, 0);
    bad.blend = BlendOp::IndexTableSrcRow(MapId(2));
    assert!(compose(
        &[bad],
        &vec![FrameImage {
            width: 1,
            height: 1,
            pixels: vec![1]
        }],
        &maps,
        view
    )
    .is_err());
}

#[test]
fn chain_applies_before_blend() {
    let table = blend_table();
    let mut maps = MapTable::new();
    let m = maps.push(map_with(&[(2, 40)]));
    let base = maps.push_table(&table);
    let mut over = DrawItem::new(FrameId(2), 0, 0);
    over.shade = ShadeChain::new(&[m]).unwrap();
    over.blend = BlendOp::IndexTable(base);
    let items = [DrawItem::new(FrameId(1), 0, 0), over];
    let out = compose(&items, &frames(), &maps, VIEW).unwrap();
    assert_eq!(out[0], table[1][40]);
    // Where nothing was below, dest is the cleared 0.
    let out = compose(&items[1..], &frames(), &maps, VIEW).unwrap();
    assert_eq!(out[0], table[0][40]);
}

#[test]
fn clip_rect_bounds_every_pixel() {
    let big = FrameImage {
        width: 20,
        height: 20,
        pixels: vec![6; 400],
    };
    let mut item = DrawItem::new(FrameId(0), 2, 3);
    item.clip = Rect::new(5, 4, 7, 9);
    let out = compose(&[item], &vec![big], &MapTable::new(), VIEW).unwrap();
    for y in 0..H as i64 {
        for x in 0..W as i64 {
            let inside = item.clip.contains(x, y);
            assert_eq!(out[(y * W as i64 + x) as usize], if inside { 6 } else { 0 });
        }
    }
}

#[test]
fn view_origin_and_edges() {
    // View offset: output pixel 0 is screen (view.x, view.y).
    let view = Rect::new(9, 9, 4, 3);
    let mut item = DrawItem::new(FrameId(0), 10, 10);
    item.clip = Rect::new(-100, -100, 1000, 1000);
    let out = compose(&[item], &frames(), &MapTable::new(), view).unwrap();
    assert_eq!(out, [0, 0, 0, 0, 0, 0, 5, 0, 0, 7, 0, 0]);
    // Partly and fully off-screen items, extreme coordinates: no panic.
    let mut items = vec![
        DrawItem::new(FrameId(1), -2, -2),
        DrawItem::new(FrameId(1), i32::MAX, i32::MAX),
        DrawItem::new(FrameId(1), i32::MIN, 0),
        DrawItem::new(FrameId(1), W as i32 - 1, H as i32 - 1),
    ];
    for it in &mut items {
        it.clip = Rect::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX);
    }
    let out = compose(&items, &frames(), &MapTable::new(), VIEW).unwrap();
    let ones: Vec<usize> = (0..out.len()).filter(|&i| out[i] == 1).collect();
    assert_eq!(ones, [0, (H * W - 1) as usize]);
    // Empty view.
    let empty = Rect::new(0, 0, 0, 5);
    assert!(compose(&items, &frames(), &MapTable::new(), empty)
        .unwrap()
        .is_empty());
}

/// A mixed scene: shading, blending, clipping, overlap, off-screen parts.
fn scene() -> (Vec<DrawItem>, Vec<FrameImage>, MapTable) {
    let mut maps = MapTable::new();
    let m = maps.push(map_with(&[(3, 200), (4, 0)]));
    let base = maps.push_table(&blend_table());
    let mut frames = Vec::new();
    for n in 0..5u32 {
        let (w, h) = (17 + n * 9, 13 + n * 7);
        frames.push(FrameImage {
            width: w,
            height: h,
            pixels: (0..w * h).map(|p| ((p * (n + 3)) % 7) as u8).collect(),
        });
    }
    let mut items = Vec::new();
    for n in 0..12i32 {
        let mut it = DrawItem::new(FrameId((n % 5) as u32), n * 11 - 15, n * 7 - 9);
        it.key = DrawKey::new(1, (n % 4) as u32, 0, n as u8).unwrap();
        if n % 3 == 0 {
            it.shade = ShadeChain::new(&[m]).unwrap();
        }
        if n % 4 == 1 {
            it.blend = BlendOp::IndexTable(base);
        }
        if n == 5 {
            it.clip = Rect::new(40, 20, 30, 25);
        }
        items.push(it);
    }
    order(&mut items);
    (items, frames, maps)
}

#[test]
fn frame_spanning_four_bins() {
    // A 4×4 frame across the corner of bins (0,0), (1,0), (0,1), (1,1).
    let f = FrameImage {
        width: 4,
        height: 4,
        pixels: (1..=16).collect(),
    };
    let frames = vec![f];
    let view = Rect::new(0, 0, 64, 64);
    let items = [DrawItem::new(FrameId(0), 30, 30)];
    let maps = MapTable::new();
    let bins = bin(&items, &frames, &maps, view).unwrap();
    assert_eq!((bins.cols(), bins.rows()), (2, 2));
    for (c, r) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        assert_eq!(bins.list(c, r), &[0]);
    }
    let plain = compose(&items, &frames, &maps, view).unwrap();
    assert_eq!(
        compose_binned(&items, &bins, &frames, &maps, view).unwrap(),
        plain
    );
}

#[test]
fn binned_equals_plain() {
    let (items, frames, maps) = scene();
    // Partial last bins and an offset view.
    for view in [Rect::new(0, 0, 100, 70), Rect::new(-7, 5, 33, 65), VIEW] {
        let bins = bin(&items, &frames, &maps, view).unwrap();
        assert_eq!(bins.cols(), view.width.div_ceil(BIN_SIZE));
        assert_eq!(
            bins.rect(bins.cols() - 1, 0).width,
            (view.width - 1) % BIN_SIZE + 1
        );
        let plain = compose(&items, &frames, &maps, view).unwrap();
        assert!(plain.iter().any(|&p| p != 0));
        assert_eq!(
            compose_binned(&items, &bins, &frames, &maps, view).unwrap(),
            plain
        );
        // Lists are in draw order.
        for r in 0..bins.rows() {
            for c in 0..bins.cols() {
                assert!(bins.list(c, r).windows(2).all(|w| w[0] < w[1]));
            }
        }
    }
}

#[test]
fn compose_is_deterministic() {
    let (items, frames, maps) = scene();
    let a = compose_rgba(&items, &frames, &maps, &palette(), Rect::FRAME).unwrap();
    let b = compose_rgba(&items, &frames, &maps, &palette(), Rect::FRAME).unwrap();
    assert_eq!(a.len(), (FRAME_WIDTH * FRAME_HEIGHT * 4) as usize);
    assert_eq!(a, b);
}

/// The image reflects exactly the input changed. Perturbing N visible opaque source pixels
/// changes exactly N output pixels, in plain and binned composition alike (what verify's
/// `--perturb N` relies on).
#[test]
fn perturbed_pixels_change_exactly_n_outputs() {
    let view = Rect::new(0, 0, 64, 48);
    let solid = FrameImage {
        width: 40,
        height: 30,
        pixels: vec![9; 1200],
    };
    let items = [DrawItem::new(FrameId(0), 10, 6)];
    let maps = MapTable::new();
    let base = compose(&items, &vec![solid.clone()], &maps, view).unwrap();
    for n in [1usize, 5, 37] {
        let mut changed = solid.clone();
        for k in 0..n {
            changed.pixels[k * 31 % 1200] = 10; // stays non-zero
        }
        let frames = vec![changed];
        let out = compose(&items, &frames, &maps, view).unwrap();
        let diff = base.iter().zip(&out).filter(|(a, b)| a != b).count();
        assert_eq!(diff, n);
        let bins = bin(&items, &frames, &maps, view).unwrap();
        assert_eq!(
            compose_binned(&items, &bins, &frames, &maps, view).unwrap(),
            out
        );
    }
}

/// Each invalid input is reported as exactly that error, with the index of the item that carries
/// it, by both compose paths and `bin`.
#[test]
fn strict_inputs_report_exactly_the_perturbation() {
    let mut maps = MapTable::new();
    let m = maps.push(map_with(&[]));
    let base = maps.push_table(&blend_table()); // rows 1..=256
    let mut frames = frames();
    let good = || {
        let mut it = DrawItem::new(FrameId(0), 1, 1);
        it.shade = ShadeChain::new(&[m]).unwrap();
        it.blend = BlendOp::IndexTable(base);
        it
    };
    let clean = vec![good(), good(), good()];
    compose(&clean, &frames, &maps, VIEW).unwrap();

    let check = |items: &[DrawItem], frames: &Vec<FrameImage>, want: SceneError| {
        let want = SceneError::Item {
            index: 1,
            error: Box::new(want),
        };
        assert_eq!(compose(items, frames, &maps, VIEW), Err(want.clone()));
        assert_eq!(bin(items, frames, &maps, VIEW), Err(want.clone()));
        let bins = bin(&clean, frames, &maps, VIEW);
        if let Ok(bins) = bins {
            let got = compose_binned(items, &bins, frames, &maps, VIEW);
            assert_eq!(got, Err(want));
        }
    };
    let perturb = |f: &dyn Fn(&mut DrawItem)| {
        let mut items = clean.clone();
        f(&mut items[1]);
        items
    };
    check(
        &perturb(&|it| it.frame = FrameId(3)),
        &frames,
        SceneError::FrameMissing(FrameId(3)),
    );
    check(&perturb(&|it| it.flip_x = true), &frames, SceneError::FlipX);
    check(
        &perturb(&|it| it.shade = ShadeChain::new(&[m, MapId(257)]).unwrap()),
        &frames,
        SceneError::MapMissing(MapId(257)),
    );
    check(
        &perturb(&|it| it.blend = BlendOp::IndexTable(MapId(2))),
        &frames,
        SceneError::BlendTable(MapId(2)),
    );
    check(
        &perturb(&|it| it.blend = BlendOp::IndexTable(MapId(u32::MAX))),
        &frames,
        SceneError::BlendTable(MapId(u32::MAX)),
    );
    // Off-screen items are validated too.
    check(
        &perturb(&|it| {
            it.x = -1000;
            it.frame = FrameId(9);
        }),
        &frames,
        SceneError::FrameMissing(FrameId(9)),
    );
    // A frame whose pixel count disagrees with its size.
    frames[0].pixels.pop();
    let want = SceneError::Item {
        index: 0,
        error: Box::new(SceneError::FrameSize {
            width: 2,
            height: 2,
            len: 3,
        }),
    };
    assert_eq!(compose(&clean, &frames, &maps, VIEW), Err(want));
}

#[test]
fn key_fields_out_of_range_are_errors() {
    assert!(DrawKey::new(15, DrawKey::MAJOR_MAX, DrawKey::MINOR_MAX, 255).is_ok());
    for (args, field, value, max) in [
        ((16, 0, 0), "pass", 16, 15),
        ((0, 1 << 28, 0), "major", 1 << 28, DrawKey::MAJOR_MAX),
        ((0, 0, 1 << 24), "minor", 1 << 24, DrawKey::MINOR_MAX),
    ] {
        assert_eq!(
            DrawKey::new(args.0, args.1, args.2, 0),
            Err(SceneError::KeyField { field, value, max })
        );
    }
}

#[test]
fn bins_must_match_the_composed_list() {
    let (items, frames, maps) = scene();
    let bins = bin(&items, &frames, &maps, VIEW).unwrap();
    let other = Rect::new(0, 0, W, H + 1);
    assert_eq!(
        compose_binned(&items, &bins, &frames, &maps, other),
        Err(SceneError::BinsView {
            built: VIEW,
            view: other
        })
    );
    assert_eq!(
        compose_binned(&items[1..], &bins, &frames, &maps, VIEW),
        Err(SceneError::BinsItems {
            built: items.len(),
            items: items.len() - 1
        })
    );
}

#[test]
fn rgba_is_a_plain_palette_lookup() {
    let pal = palette();
    let rgba = to_rgba(&[0, 2, 255], &pal);
    assert_eq!(
        rgba,
        [
            0,
            255,
            0x5a,
            255,
            2,
            253,
            2 ^ 0x5a,
            255,
            255,
            0,
            255 ^ 0x5a,
            255
        ]
    );
}

// --- Frame cycle and pixel writes ----------------------------------------

const FW: u32 = 800;
const FH: u32 = 600;

fn rows_of(pixels: &[u8], width: u32) -> Vec<(u32, u32, u8)> {
    // (first row, last row, value) runs of whole uniform rows.
    let mut runs: Vec<(u32, u32, u8)> = Vec::new();
    for (y, row) in pixels.chunks(width as usize).enumerate() {
        let v = row[0];
        assert!(row.iter().all(|&p| p == v), "row {y} is uniform");
        match runs.last_mut() {
            Some(r) if r.2 == v => r.1 = y as u32,
            _ => runs.push((y as u32, y as u32, v)),
        }
    }
    runs
}

/// Frame-cycle vectors: framebuffer all 5, nothing drawn, 800 × 600.
#[test]
fn frame_cycle_clears() {
    let all_5 = || FrameCycle::with_pixels(FW, FH, vec![5; (FW * FH) as usize]).unwrap();
    let nothing = |c: &mut FrameCycle, blank| {
        c.compose(blank, &[], &Vec::<FrameImage>::new(), &MapTable::new())
            .unwrap()
            .to_vec()
    };
    // BlankScreen 1: rows 0–552 = 0, rows 553–599 = 5.
    let mut c = all_5();
    assert_eq!(c.plan(true).clear_rows, 553);
    assert_eq!(
        rows_of(&nothing(&mut c, true), FW),
        [(0, 552, 0), (553, 599, 5)]
    );
    // BlankScreen 0: all 5.
    let mut c = all_5();
    assert_eq!(rows_of(&nothing(&mut c, false), FW), [(0, 599, 5)]);
    // Counter 1: all 0 and the counter becomes 0; the next frame keeps
    // the persistent framebuffer again.
    let mut c = all_5();
    c.set_post_clear(1);
    assert_eq!(rows_of(&nothing(&mut c, false), FW), [(0, 599, 0)]);
    assert_eq!(c.post_clear(), 0);
    assert!(!c.plan(true).clear_after);
}

/// Drawn pixels survive in the uncleared bottom 47 rows into the next
/// frame; the post-draw clear blacks out a frame that was drawn.
#[test]
fn framebuffer_persists_between_frames() {
    let frames = vec![FrameImage {
        width: 4,
        height: 4,
        pixels: vec![9; 16],
    }];
    let maps = MapTable::new();
    let mut c = FrameCycle::new(FW, FH).unwrap();
    // A fresh framebuffer is all 0 (a single-frame case starts there).
    assert!(c.pixels().iter().all(|&p| p == 0));
    let top = DrawItem::new(FrameId(0), 10, 10);
    let bottom = DrawItem::new(FrameId(0), 10, 580);
    c.compose(true, &[top, bottom], &frames, &maps).unwrap();
    // Next frame, nothing drawn: the top square is cleared, the bottom one
    // (rows 580..584 ≥ 553) is still there.
    let out = c.compose(true, &[], &frames, &maps).unwrap();
    let at = |x: u32, y: u32| out[(y * FW + x) as usize];
    assert_eq!((at(10, 10), at(13, 583), at(14, 583)), (0, 9, 0));
    // Pixel (x, y) is byte y × W + x.
    assert_eq!(c.pixels()[(580 * FW + 10) as usize], 9);
    // The post-draw clear: drawn, then all 0.
    c.set_post_clear(1);
    let out = c.compose(false, &[top], &frames, &maps).unwrap();
    assert!(out.iter().all(|&p| p == 0));
}

/// A frame composed over the GPU's own copy of the frame before (live
/// play, `world_view::node`) is counted with `pass`: its post-draw clear
/// counts down once and the pixels stay until the next commit; a later
/// commit counts its own frame. Each frame's plan is counted once, so the
/// plans match those of a cycle that committed every frame.
#[test]
fn frame_cycle_pass_counts_a_frame_without_its_pixels() {
    let mut every = FrameCycle::with_pixels(4, 48, vec![3; 4 * 48]).unwrap();
    let mut passed = every.clone();
    every.set_post_clear(2);
    passed.set_post_clear(2);
    for frame in 0..4u8 {
        let plan = every.plan(false);
        assert_eq!(passed.plan(false), plan, "frame {frame}");
        every.commit(plan, vec![frame; 4 * 48]).unwrap();
        if frame % 2 == 0 {
            passed.pass(plan).unwrap();
            assert!(passed.pixels().iter().all(|&p| p != frame));
        } else {
            passed.commit(plan, vec![frame; 4 * 48]).unwrap();
            assert_eq!(passed.pixels(), every.pixels());
        }
        assert_eq!(passed.post_clear(), every.post_clear());
    }
    assert_eq!(every.post_clear(), 0);
    // A plan this cycle would not make is refused, and counts nothing.
    let late = FramePlan {
        clear_rows: 0,
        clear_after: true,
    };
    assert_eq!(passed.pass(late), Err(SceneError::FramePlan(late)));
    assert_eq!(passed.post_clear(), 0);
}

/// The frame cycle is strict: no change on error, foreign plans and sizes
/// are rejected, and the clear needs more than 47 rows.
#[test]
fn frame_cycle_strict_inputs() {
    assert_eq!(
        FrameCycle::new(10, 47),
        Err(SceneError::FramebufferHeight { height: 47 })
    );
    assert!(FrameCycle::new(10, 48).is_ok());
    assert!(matches!(
        FrameCycle::with_pixels(10, 48, vec![0; 479]),
        Err(SceneError::BaseSize { .. })
    ));
    let mut c = FrameCycle::with_pixels(4, 48, vec![3; 4 * 48]).unwrap();
    let mut flipped = DrawItem::new(FrameId(0), 0, 0);
    flipped.flip_x = true;
    assert!(c
        .compose(true, &[flipped], &frames(), &MapTable::new())
        .is_err());
    assert!(c.pixels().iter().all(|&p| p == 3));
    // A plan this cycle would not make.
    let odd = FramePlan {
        clear_rows: 2,
        clear_after: false,
    };
    assert_eq!(
        c.commit(odd, vec![0; 4 * 48]),
        Err(SceneError::FramePlan(odd))
    );
    let late = FramePlan {
        clear_rows: 0,
        clear_after: true,
    };
    assert_eq!(
        c.commit(late, vec![0; 4 * 48]),
        Err(SceneError::FramePlan(late))
    );
    assert!(matches!(
        c.commit(FramePlan::NONE, vec![0; 3]),
        Err(SceneError::BaseSize { .. })
    ));
    // compose_frame: base size and rows inside the view.
    let view = Rect::new(0, 0, 4, 2);
    let none: Vec<FrameImage> = Vec::new();
    assert!(matches!(
        compose_frame(&[], &none, &MapTable::new(), view, &[0; 7], FramePlan::NONE),
        Err(SceneError::BaseSize { .. })
    ));
    let three = FramePlan {
        clear_rows: 3,
        clear_after: false,
    };
    assert_eq!(
        compose_frame(&[], &none, &MapTable::new(), view, &[0; 8], three),
        Err(SceneError::FramePlan(three))
    );
}

/// Pixel-write vectors: `L[P[s]]` without `T`; `T[256 × d + P[s]]` with `T`.
#[test]
fn pixel_write_vectors() {
    let mut maps = MapTable::new();
    let p = maps.push(map_with(&[(7, 9)]));
    let l = maps.push(map_with(&[(9, 3)]));
    // T[d][s] = distinct for every pair.
    let mut table = Box::new([[0u8; 256]; 256]);
    for (d, row) in table.iter_mut().enumerate() {
        for (s, v) in row.iter_mut().enumerate() {
            *v = (d * 3 + s * 5 + 11) as u8;
        }
    }
    let t = maps.push_table(&table);
    let frames = vec![FrameImage {
        width: 1,
        height: 1,
        pixels: vec![7],
    }];
    let mut pl = DrawItem::new(FrameId(0), 0, 0);
    pl.shade = ShadeChain::new(&[p, l]).unwrap();
    let mut pt = DrawItem::new(FrameId(0), 1, 0);
    pt.shade = ShadeChain::new(&[p]).unwrap();
    pt.blend = BlendOp::IndexTable(t);
    let view = Rect::new(0, 0, 2, 1);
    let out = compose_frame(
        &[pl, pt],
        &frames,
        &maps,
        view,
        &[200, 200],
        FramePlan::NONE,
    )
    .unwrap();
    assert_eq!(out[0], 3);
    // The flat PL2 layout: byte 256 × 200 + 9 of the table.
    let flat: Vec<u8> = table.iter().flatten().copied().collect();
    assert_eq!(out[1], flat[256 * 200 + 9]);
    assert_ne!(flat[256 * 200 + 9], flat[256 * 9 + 200]);
    // Binned: the same.
    let bins = bin(&[pl, pt], &frames, &maps, view).unwrap();
    let binned = compose_binned_frame(
        &[pl, pt],
        &bins,
        &frames,
        &maps,
        view,
        &[200, 200],
        FramePlan::NONE,
    )
    .unwrap();
    assert_eq!(binned, out);
}

/// Pixel writes with all three tables: `T[256 × d + L[s]]`, the remap dropped; each other
/// combination as in `PixelTables::ops`.
#[test]
fn lit_blend_drops_the_remap() {
    let mut maps = MapTable::new();
    let p = maps.push(map_with(&[(7, 9)]));
    let l = maps.push(map_with(&[(7, 4), (9, 3)]));
    let mut table = Box::new([[0u8; 256]; 256]);
    for (d, row) in table.iter_mut().enumerate() {
        for (s, v) in row.iter_mut().enumerate() {
            *v = (d * 3 + s * 5 + 11) as u8;
        }
    }
    let t = maps.push_table(&table);
    let frames = vec![FrameImage {
        width: 1,
        height: 1,
        pixels: vec![7],
    }];
    let cases = [
        (PixelTables::default(), 7),
        (
            PixelTables {
                remap: Some(p),
                light: Some(l),
                blend: None,
            },
            3,
        ),
        (
            PixelTables {
                remap: Some(p),
                light: None,
                blend: Some(t),
            },
            table[200][9],
        ),
        (
            PixelTables {
                remap: Some(p),
                light: Some(l),
                blend: Some(t),
            },
            table[200][4],
        ),
        (
            PixelTables {
                remap: None,
                light: Some(l),
                blend: Some(t),
            },
            table[200][4],
        ),
    ];
    let items: Vec<DrawItem> = cases
        .iter()
        .enumerate()
        .map(|(i, (tables, _))| {
            let mut item = DrawItem::new(FrameId(0), i as i32, 0);
            (item.shade, item.blend) = tables.ops();
            item
        })
        .collect();
    let view = Rect::new(0, 0, cases.len() as u32, 1);
    let out = compose_frame(&items, &frames, &maps, view, &[200; 5], FramePlan::NONE).unwrap();
    let want: Vec<u8> = cases.iter().map(|&(_, v)| v).collect();
    assert_eq!(out, want);
    // With the remap applied the lit blend would read column L[P[s]] = 3.
    assert_ne!(table[200][4], table[200][3]);
    assert_eq!(
        PixelTables {
            remap: Some(p),
            light: Some(l),
            blend: Some(t),
        }
        .ops(),
        (ShadeChain::new(&[l]).unwrap(), BlendOp::IndexTable(t))
    );
}

/// Palette vector: PL2 bytes `01 02 03 xx 10 20 30 xx` → index 0 (1, 2, 3),
/// index 1 (0x10, 0x20, 0x30); RGBA alpha 255, index 0 included.
#[test]
fn present_palette_from_pl2() {
    let mut pl2 = vec![0xEEu8; PL2_PALETTE_BYTES + 100];
    pl2[..8].copy_from_slice(&[1, 2, 3, 0xAA, 0x10, 0x20, 0x30, 0xBB]);
    pl2[1020..1024].copy_from_slice(&[7, 8, 9, 0xCC]);
    let p = present_palette(&pl2).unwrap();
    let rgb = |i: usize| (p.colors[i].r, p.colors[i].g, p.colors[i].b);
    assert_eq!(rgb(0), (1, 2, 3));
    assert_eq!(rgb(1), (0x10, 0x20, 0x30));
    assert_eq!(rgb(255), (7, 8, 9));
    assert_eq!(to_rgba(&[0, 1], &p), [1, 2, 3, 255, 0x10, 0x20, 0x30, 255]);
    assert_eq!(
        present_palette(&pl2[..1023]),
        Err(SceneError::Pl2Size { len: 1023 })
    );
}

/// The binned model equals the reference on a whole running frame: random
/// previous frame, BlankScreen clear, the mixed scene drawn across it.
#[test]
fn binned_frame_matches_reference() {
    let (items, frames, maps) = scene();
    let base: Vec<u8> = (0..W * H).map(|i| (i * 37 % 256) as u8).collect();
    for plan in [
        FramePlan::NONE,
        FramePlan {
            clear_rows: 5,
            clear_after: false,
        },
        FramePlan {
            clear_rows: H,
            clear_after: true,
        },
    ] {
        let reference = compose_frame(&items, &frames, &maps, VIEW, &base, plan).unwrap();
        let bins = bin(&items, &frames, &maps, VIEW).unwrap();
        let binned =
            compose_binned_frame(&items, &bins, &frames, &maps, VIEW, &base, plan).unwrap();
        assert_eq!(binned, reference, "{plan:?}");
    }
}
