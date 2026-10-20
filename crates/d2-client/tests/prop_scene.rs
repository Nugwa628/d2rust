//! Property tests on the scene and the CPU reference compositor: random
//! frames (transparent pixels, empty frames, wrong pixel counts), random
//! draw items (positions and clips anywhere in i32, shade chains and
//! blend tables naming present or missing maps, reserved `flip_x`) over
//! random views. Properties: no panic or overflow; an error exactly when
//! an item is invalid (strict checks); the image equals a per-pixel
//! model written here; two runs give identical bytes; the
//! binned (GPU-model) composition equals the direct one; RGBA is
//! the palette lookup of every index; `order` is a stable sort by
//! key.

use crate::prop_support;

use d2_client::scene::{
    self, BlendOp, DrawItem, DrawKey, FrameId, FrameImage, ItemTag, MapId, MapTable, Rect,
    ShadeChain,
};
use d2_formats::palette::{Palette, Rgb};
use proptest::prelude::*;

use prop_support::{bounded, config};

fn coord() -> impl Strategy<Value = i32> {
    prop_oneof![
        8 => -40i32..80,
        1 => any::<i32>(),
        1 => prop_oneof![Just(i32::MIN), Just(i32::MAX), Just(i32::MAX - 3), Just(i32::MIN + 3)],
    ]
}

fn extent() -> impl Strategy<Value = u32> {
    prop_oneof![6 => 0u32..48, 1 => any::<u32>(), 1 => Just(u32::MAX)]
}

fn clip() -> impl Strategy<Value = Rect> {
    prop_oneof![
        1 => Just(Rect::FRAME),
        3 => (coord(), coord(), extent(), extent()).prop_map(|(x, y, w, h)| Rect::new(x, y, w, h)),
    ]
}

/// Views: small rects anywhere (pixel count bounded), or the frame.
fn view() -> impl Strategy<Value = Rect> {
    prop_oneof![
        1 => Just(Rect::FRAME),
        6 => (coord(), coord(), 0u32..64, 0u32..64).prop_map(|(x, y, w, h)| Rect::new(x, y, w, h)),
    ]
}

fn frame_image() -> impl Strategy<Value = FrameImage> {
    (0u32..12, 0u32..12, proptest::bool::weighted(0.04)).prop_flat_map(|(w, h, bad)| {
        let n = (w * h) as usize + usize::from(bad);
        proptest::collection::vec(prop_oneof![1 => Just(0u8), 2 => any::<u8>()], n).prop_map(
            move |pixels| FrameImage {
                width: w,
                height: h,
                pixels,
            },
        )
    })
}

/// Map table: a few PL2-like rows, then optionally one 256×256 blend
/// table.
fn maps() -> impl Strategy<Value = MapTable> {
    (
        proptest::collection::vec(any::<[u8; 32]>(), 0..4),
        any::<bool>(),
        any::<u8>(),
    )
        .prop_map(|(rows, table, salt)| {
            let mut m = MapTable::new();
            for r in rows {
                let mut row = [0u8; 256];
                for (i, b) in row.iter_mut().enumerate() {
                    *b = r[i % 32].wrapping_add(i as u8);
                }
                m.push(row);
            }
            if table {
                let mut t = Box::new([[0u8; 256]; 256]);
                for (s, row) in t.iter_mut().enumerate() {
                    for (d, b) in row.iter_mut().enumerate() {
                        *b = (s as u8).wrapping_mul(3) ^ (d as u8).wrapping_add(salt);
                    }
                }
                m.push_table(&t);
            }
            m
        })
}

fn map_id() -> impl Strategy<Value = MapId> {
    prop_oneof![8 => (0u32..6).prop_map(MapId), 1 => any::<u32>().prop_map(MapId)]
}

fn item() -> impl Strategy<Value = DrawItem> {
    (
        prop_oneof![8 => 0u32..6, 1 => any::<u32>()],
        coord(),
        coord(),
        clip(),
        proptest::collection::vec(map_id(), 0..=4),
        prop_oneof![
            3 => Just(BlendOp::Opaque),
            1 => prop_oneof![
                (0u32..8).prop_map(MapId),
                any::<u32>().prop_map(MapId),
                Just(MapId(u32::MAX - 255)),
            ]
            .prop_map(BlendOp::IndexTable),
            1 => prop_oneof![
                (0u32..8).prop_map(MapId),
                any::<u32>().prop_map(MapId),
            ]
            .prop_map(BlendOp::IndexTableSrcRow),
        ],
        (0u32..=DrawKey::PASS_MAX, 0u32..4, 0u32..4, any::<u8>()),
        proptest::bool::weighted(0.03),
    )
        .prop_map(
            |(frame, x, y, clip, shade, blend, (p, ma, mi, sub), flip_x)| {
                let mut i = DrawItem::new(FrameId(frame), x, y);
                i.clip = clip;
                i.shade = ShadeChain::new(&shade).unwrap();
                i.blend = blend;
                i.key = DrawKey::new(p, ma, mi, sub).unwrap();
                i.flip_x = flip_x;
                i.tag = ItemTag::Ui(frame);
                i
            },
        )
}

fn palette() -> Palette {
    let mut p = Palette {
        colors: [Rgb::default(); 256],
    };
    for (i, c) in p.colors.iter_mut().enumerate() {
        let i = i as u8;
        *c = Rgb {
            r: i,
            g: i.wrapping_mul(7),
            b: !i,
        };
    }
    p
}

/// Whether the compositor must refuse `item` (strict checks).
fn invalid(item: &DrawItem, frames: &[FrameImage], maps: &MapTable) -> bool {
    let Some(f) = frames.get(item.frame.0 as usize) else {
        return true;
    };
    item.flip_x
        || f.pixels.len() as u64 != u64::from(f.width) * u64::from(f.height)
        || item
            .shade
            .maps()
            .iter()
            .any(|m| (m.0 as usize) >= maps.len())
        || matches!(item.blend.table(), Some(b) if u64::from(b.0) + 256 > maps.len() as u64)
}

fn inside(r: &Rect, x: i64, y: i64) -> bool {
    x >= i64::from(r.x)
        && x < i64::from(r.x) + i64::from(r.width)
        && y >= i64::from(r.y)
        && y < i64::from(r.y) + i64::from(r.height)
}

/// Per-pixel model of the composition with the shade chain (applied
/// to a non-zero source) and the blend (opaque, or `T[256 × d + s]`: row =
/// destination), framebuffer cleared to 0.
fn model(items: &[DrawItem], frames: &[FrameImage], maps: &MapTable, view: Rect) -> Vec<u8> {
    let mut out = Vec::with_capacity(view.width as usize * view.height as usize);
    for vy in 0..i64::from(view.height) {
        for vx in 0..i64::from(view.width) {
            let (sx, sy) = (i64::from(view.x) + vx, i64::from(view.y) + vy);
            let mut v = 0u8;
            for it in items {
                let f = &frames[it.frame.0 as usize];
                let img = Rect::new(it.x, it.y, f.width, f.height);
                if !inside(&img, sx, sy) || !inside(&it.clip, sx, sy) {
                    continue;
                }
                let (fx, fy) = (
                    (sx - i64::from(it.x)) as usize,
                    (sy - i64::from(it.y)) as usize,
                );
                let src = f.pixels[fy * f.width as usize + fx];
                if src == 0 {
                    continue;
                }
                let s = it
                    .shade
                    .maps()
                    .iter()
                    .fold(src, |i, m| maps.get(*m).unwrap()[usize::from(i)]);
                v = match it.blend {
                    BlendOp::Opaque => s,
                    BlendOp::IndexTable(b) => {
                        maps.get(MapId(b.0 + u32::from(v))).unwrap()[usize::from(s)]
                    }
                    BlendOp::IndexTableSrcRow(b) => {
                        maps.get(MapId(b.0 + u32::from(s))).unwrap()[usize::from(v)]
                    }
                };
            }
            out.push(v);
        }
    }
    out
}

fn run(frames: Vec<FrameImage>, maps: MapTable, mut items: Vec<DrawItem>, view: Rect) {
    // Stable sort by key.
    let mut want = items.clone();
    want.sort_by_key(|i| i.key);
    scene::order(&mut items);
    assert_eq!(items, want);

    let a = scene::compose(&items, &frames, &maps, view);
    let b = scene::compose(&items, &frames, &maps, view);
    assert_eq!(a, b, "two runs differ");
    // A view whose pixels pass the i32 screen range is refused first.
    let end = i64::from(i32::MAX) + 1;
    let bad_view = i64::from(view.x) + i64::from(view.width) > end
        || i64::from(view.y) + i64::from(view.height) > end;
    let bad = bad_view || items.iter().any(|i| invalid(i, &frames, &maps));
    let binned = scene::bin(&items, &frames, &maps, view)
        .and_then(|bins| scene::compose_binned(&items, &bins, &frames, &maps, view));
    match a {
        Ok(img) => {
            assert!(!bad, "an invalid item was composed");
            assert_eq!(img.len(), view.width as usize * view.height as usize);
            assert_eq!(img, model(&items, &frames, &maps, view));
            assert_eq!(binned.as_ref(), Ok(&img), "binned differs from direct");
            let p = palette();
            let rgba = scene::to_rgba(&img, &p);
            assert_eq!(rgba.len(), img.len() * 4);
            for (i, px) in img.iter().zip(rgba.chunks(4)) {
                let c = p.colors[usize::from(*i)];
                assert_eq!(px, [c.r, c.g, c.b, 255]);
            }
            let again = scene::compose_rgba(&items, &frames, &maps, &p, view).unwrap();
            assert_eq!(again, rgba);
        }
        Err(e) => {
            assert!(bad, "valid list refused: {e}");
            assert_eq!(binned.err(), Some(e));
        }
    }
}

proptest! {
    #![proptest_config(config(256))]

    /// Random scenes over random views.
    #[test]
    fn random_scenes(
        frames in proptest::collection::vec(frame_image(), 0..6),
        maps in maps(),
        items in proptest::collection::vec(item(), 0..10),
        view in view(),
    ) {
        bounded(move || run(frames, maps, items, view));
    }

    /// Valid scenes only (every frame id, map and table present), so the
    /// pixel paths run on most cases.
    #[test]
    fn valid_scenes(
        frames in proptest::collection::vec(frame_image(), 1..6),
        maps in maps(),
        items in proptest::collection::vec(item(), 0..10),
        view in view(),
    ) {
        let frames: Vec<FrameImage> = frames
            .into_iter()
            .map(|mut f| {
                f.pixels.truncate((f.width * f.height) as usize);
                f
            })
            .collect();
        let n = frames.len() as u32;
        let rows = maps.len() as u32;
        let items: Vec<DrawItem> = items
            .into_iter()
            .map(|mut i| {
                i.frame = FrameId(i.frame.0 % n);
                i.flip_x = false;
                let shade: Vec<MapId> = if rows == 0 {
                    Vec::new()
                } else {
                    i.shade.maps().iter().map(|m| MapId(m.0 % rows)).collect()
                };
                i.shade = ShadeChain::new(&shade).unwrap();
                i.blend = match i.blend {
                    BlendOp::IndexTable(b) if rows >= 256 => {
                        BlendOp::IndexTable(MapId(b.0 % (rows - 255)))
                    }
                    BlendOp::IndexTableSrcRow(b) if rows >= 256 => {
                        BlendOp::IndexTableSrcRow(MapId(b.0 % (rows - 255)))
                    }
                    _ => BlendOp::Opaque,
                };
                i
            })
            .collect();
        bounded(move || run(frames, maps, items, view));
    }
}

/// Minimized failure (fixed at the root): a view whose pixels pass
/// `i32::MAX` overflowed in `compose_binned` (bin rects wrapped to
/// negative x); such views are refused by every entry point.
#[test]
fn regress_view_past_i32() {
    let view = Rect::new(i32::MAX - 3, 0, 40, 1);
    let frames = vec![FrameImage {
        width: 1,
        height: 1,
        pixels: vec![1],
    }];
    let maps = MapTable::new();
    let mut item = DrawItem::new(FrameId(0), i32::MAX - 1, 0);
    item.clip = Rect::new(0, 0, u32::MAX, 1);
    let items = [item];
    let want = scene::SceneError::View(view);
    assert_eq!(
        scene::compose(&items, &frames, &maps, view),
        Err(want.clone())
    );
    assert_eq!(scene::bin(&items, &frames, &maps, view).err(), Some(want));
    // The last representable column is still a valid view.
    let edge = Rect::new(i32::MAX - 3, 0, 4, 1);
    let img = scene::compose(&items, &frames, &maps, edge).unwrap();
    assert_eq!(img, [0, 0, 1, 0]);
    let bins = scene::bin(&items, &frames, &maps, edge).unwrap();
    assert_eq!(
        scene::compose_binned(&items, &bins, &frames, &maps, edge).unwrap(),
        img
    );
}
