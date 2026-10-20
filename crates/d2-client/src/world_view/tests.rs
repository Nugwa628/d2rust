//! Synthetic world → draw list (C4 key order, C7 slots), the CPU frame
//! hash (golden, with perturbations), the GPU packing checked
//! through the shader emulation, the UI binding, and a windowless Bevy
//! run. Every rule here is a test fixture ([`TestRules`]), not a
//! base-game rule.

use std::sync::{Arc, Mutex};

use bevy::prelude::{
    App, AssetApp, AssetPlugin, Assets, ButtonInput, Image, MinimalPlugins, MouseButton,
};
use d2_formats::cof::{Cof, CofLayer};
use d2_formats::palette::{Palette, Rgb};
use d2_proto::PROTOCOL_VERSION;

use super::*;
use crate::bridge::dispatch::{Dispatch, HandlerError, Message};
use crate::bridge::link::{LinkError, Pumped, SendQueue, Sent, ServerLink};
use crate::bridge::{Bridge, BridgeResource, UnitKey};
use crate::frames::{FramePart, FrameSet};
use crate::gpu_compositor::pack::emulate;
use crate::ui::{
    ClientIntent, ImageRef, ImageRequest, NoPanelRules, NoStrings, Panel, PanelId, Point,
    PointerButton, TextRequest, TextStyle, UiCtx, UiDraw, UiDrawSink, UiEvent, UiResponse, UiRoot,
    WidgetId,
};

const COF: &str = "data/global/chars/tst/tst.cof";

fn cof_path() -> CanonicalPath {
    CanonicalPath::new(COF).unwrap()
}

fn set_key(component: u8, dir: u8) -> FrameSetKey {
    FrameSetKey::new(
        format!("data/global/chars/tst/c{component}.dcc"),
        FramePart::Dir(dir),
    )
    .unwrap()
}

fn tile_key() -> FrameSetKey {
    FrameSetKey::new("data/global/tiles/tst.dt1", FramePart::Tile(0)).unwrap()
}

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

fn layer(component: u8) -> CofLayer {
    CofLayer {
        component,
        shadow: 0,
        selectable: 1,
        override_translucency: 0,
        new_translucency: 0,
        weapon_class: *b"hth\0",
    }
}

/// Two directions, one frame, two layers (components 0 and 1). Direction
/// 0 draws component 1 behind 0; direction 1 the reverse.
fn cof() -> Cof {
    Cof {
        layers_count: 2,
        frames: 1,
        directions: 2,
        version: 20,
        unknown: [0; 4],
        x_min: 0,
        x_max: 0,
        y_min: 0,
        y_max: 0,
        animation_rate: 256,
        layers: vec![layer(0), layer(1)],
        events: vec![0],
        event_padding: Vec::new(),
        draw_order: vec![1, 0, 0, 1],
    }
}

fn set(width: u32, height: u32, x_off: i32, y_off: i32, pixels: &[u8]) -> FrameSet {
    FrameSet {
        frames: vec![IndexFrame::new(width, height, x_off, y_off, pixels.to_vec()).unwrap()],
    }
}

/// The resident frame sets, in store insertion order.
fn sets() -> Vec<(FrameSetKey, FrameSet)> {
    vec![
        (set_key(0, 0), set(2, 2, 1, -2, &[0, 5, 7, 0])),
        (set_key(1, 0), set(3, 1, 0, 0, &[9, 9, 9])),
        (set_key(0, 1), set(2, 1, 0, 0, &[4, 4])),
        (set_key(1, 1), set(1, 1, 0, 0, &[6])),
        (tile_key(), set(4, 2, 0, 0, &[3; 8])),
    ]
}

/// Assets whose frame store holds `sets` in order.
fn assets_from(sets: Vec<(FrameSetKey, FrameSet)>) -> ViewAssets {
    let mut a = ViewAssets::new(palette());
    a.cofs.insert(cof_path(), cof());
    for (key, set) in sets {
        a.frames.insert(key, set).unwrap();
    }
    a
}

fn assets() -> ViewAssets {
    assets_from(sets())
}

/// [`assets`] without the set `key`.
fn assets_without(key: &FrameSetKey) -> ViewAssets {
    assets_from(sets().into_iter().filter(|(k, _)| k != key).collect())
}

fn world(keys: &[(u8, u32)]) -> ClientWorld {
    let mut w = ClientWorld::default();
    for &(unit_type, guid) in keys {
        let key = UnitKey { unit_type, guid };
        w.units.insert(key, ClientUnit::new(key));
    }
    w
}

/// Fixture rules: guid 9 is not drawn; COF direction = guid % 2; draw key
/// (2, guid, unit type); x = 10 × guid + x_off, y = 50 + y_off.
struct TestRules;

impl ViewRules for TestRules {
    fn tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        Ok(vec![TileDraw {
            frame: ComponentFrame {
                set: tile_key(),
                index: 0,
            },
            x: 100,
            y: 100,
            clip: Rect::FRAME,
            shade: ShadeChain::EMPTY,
            blend: BlendOp::Opaque,
            key: DrawKey::new(0, 0, 0, 0).unwrap(),
            cell: (1, 2),
            light: None,
            alpha: None,
        }])
    }

    fn unit_pose(&self, _: &ClientWorld, u: &ClientUnit) -> Result<Option<UnitPose>, ViewError> {
        Ok((u.key.guid != 9).then(|| UnitPose {
            sub_frame: 0,
            cof: cof_path(),
            dir: (u.key.guid % 2) as usize,
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
            minor: u32::from(u.key.unit_type),
            clip: Rect::FRAME,
            tag: ItemTag::Unit(u.key.guid),
        })
    }

    fn component_frame(
        &self,
        _: &ClientUnit,
        pose: &UnitPose,
        req: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        Ok(ComponentFrame {
            set: set_key(req.slot.component, pose.dir as u8),
            index: req.frame,
        })
    }

    fn place(
        &self,
        u: &ClientUnit,
        _: &UnitPose,
        _: &ComponentRequest<'_>,
        image: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        Ok((10 * u.key.guid as i32 + image.x_off, 50 + image.y_off))
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

impl UiRules for TestRules {
    fn ui_image(&self, req: &ImageRequest, _: &ViewAssets) -> Result<UiSprite, ViewError> {
        Ok(UiSprite {
            frame: ComponentFrame {
                set: set_key(1, 1),
                index: req.image.frame as usize,
            },
            x: req.at.x,
            y: req.at.y,
            shade: ShadeChain::EMPTY,
            blend: BlendOp::Opaque,
            clip: None,
        })
    }

    fn ui_text(&self, _: &TextRequest, _: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        Err(ViewError::unresolved("UI text layout", "ui/text.md"))
    }

    fn ui_pass(&self) -> Result<u32, ViewError> {
        Ok(5)
    }
}

fn ui_image(x: i32, y: i32) -> UiDraw {
    UiDraw::Image(ImageRequest {
        image: ImageRef { file: 1, frame: 0 },
        at: Point::new(x, y),
        clip: crate::ui::Rect::new(0, 0, 800, 600),
        look: crate::ui::CelLook::PLAIN,
        call: crate::ui::draw::CelCall::Draw,
    })
}

/// (frame set, frame, x, y, key, tag) of each item, frame resolved
/// through the frame store.
type Row = (FrameSetKey, usize, i32, i32, (u32, u32, u32, u8), ItemTag);

fn rows(frame: &WorldFrame, assets: &ViewAssets) -> Vec<Row> {
    frame
        .items
        .iter()
        .map(|i| {
            let (k, n) = assets.frames.owner(i.frame).unwrap();
            let key = (i.key.pass(), i.key.major(), i.key.minor(), i.key.sub());
            (k.clone(), n, i.x, i.y, key, i.tag)
        })
        .collect()
}

fn scene_frame() -> (WorldFrame, ViewAssets) {
    let a = assets();
    let w = world(&[(0, 7), (1, 4), (1, 9)]);
    let f = build(&w, &[ui_image(300, 200)], &TestRules, &a).unwrap();
    (f, a)
}

#[test]
fn draw_list_is_ordered_by_key_with_cof_slots() {
    let (f, a) = scene_frame();
    assert_eq!(f.units_drawn, 2);
    assert_eq!(f.units_hidden, 1);
    let tile = ItemTag::Tile { x: 1, y: 2 };
    let expected: Vec<Row> = vec![
        (tile_key(), 0, 100, 100, (0, 0, 0, 0), tile),
        // Unit (1, 4): built second, sorted first (major 4 < 7). COF
        // direction 0: component 1 behind component 0.
        (set_key(1, 0), 0, 40, 50, (2, 4, 1, 0), ItemTag::Unit(4)),
        (set_key(0, 0), 0, 41, 48, (2, 4, 1, 1), ItemTag::Unit(4)),
        // Unit (0, 7), COF direction 1: component 0 behind 1.
        (set_key(0, 1), 0, 70, 50, (2, 7, 0, 0), ItemTag::Unit(7)),
        (set_key(1, 1), 0, 70, 50, (2, 7, 0, 1), ItemTag::Unit(7)),
        // The UI root's draws: major `UI_PANELS_MAJOR` (after the
        // automap's major 0).
        (set_key(1, 1), 0, 300, 200, (5, 1, 0, 0), ItemTag::Ui(0)),
    ];
    assert_eq!(rows(&f, &a), expected);
    // Ids are the frame store's (insertion order), not build order: the
    // tile, built first, has the last id; the UI image shares its frame's.
    let ids: Vec<u32> = f.items.iter().map(|i| i.frame.0).collect();
    assert_eq!(ids, vec![4, 1, 0, 2, 3, 3]);
}

#[test]
fn equal_keys_keep_build_order() {
    struct Same;
    impl ViewRules for Same {
        fn tiles(&self, w: &ClientWorld, a: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
            TestRules.tiles(w, a)
        }
        fn unit_pose(
            &self,
            w: &ClientWorld,
            u: &ClientUnit,
        ) -> Result<Option<UnitPose>, ViewError> {
            TestRules.unit_pose(w, u)
        }
        fn unit_params(
            &self,
            _: &ClientWorld,
            u: &ClientUnit,
            _: &UnitPose,
        ) -> Result<UnitParams, ViewError> {
            Ok(UnitParams {
                pass: 2,
                major: 1,
                minor: 1,
                clip: Rect::FRAME,
                tag: ItemTag::Unit(u.key.guid),
            })
        }
        fn component_frame(
            &self,
            u: &ClientUnit,
            p: &UnitPose,
            r: &ComponentRequest<'_>,
        ) -> Result<ComponentFrame, CompositeError> {
            TestRules.component_frame(u, p, r)
        }
        fn place(
            &self,
            u: &ClientUnit,
            p: &UnitPose,
            r: &ComponentRequest<'_>,
            i: &IndexFrame,
        ) -> Result<(i32, i32), CompositeError> {
            TestRules.place(u, p, r, i)
        }
        fn shade(
            &self,
            u: &ClientUnit,
            r: &ComponentRequest<'_>,
        ) -> Result<ShadeChain, CompositeError> {
            TestRules.shade(u, r)
        }
        fn blend(
            &self,
            u: &ClientUnit,
            r: &ComponentRequest<'_>,
        ) -> Result<BlendOp, CompositeError> {
            TestRules.blend(u, r)
        }
    }
    impl UiRules for Same {
        fn ui_image(&self, r: &ImageRequest, a: &ViewAssets) -> Result<UiSprite, ViewError> {
            TestRules.ui_image(r, a)
        }
        fn ui_text(&self, r: &TextRequest, a: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
            TestRules.ui_text(r, a)
        }
        fn ui_pass(&self) -> Result<u32, ViewError> {
            TestRules.ui_pass()
        }
    }
    let f = build(&world(&[(0, 7), (1, 4)]), &[], &Same, &assets()).unwrap();
    // Same pass/major/minor: `sub` (the slot) decides, and items with the
    // whole key equal keep build order, unit-key order (0, 7) before (1, 4).
    let tags: Vec<_> = f.items.iter().map(|i| (i.tag, i.key.sub())).collect();
    assert_eq!(
        tags,
        vec![
            (ItemTag::Tile { x: 1, y: 2 }, 0),
            (ItemTag::Unit(7), 0),
            (ItemTag::Unit(4), 0),
            (ItemTag::Unit(7), 1),
            (ItemTag::Unit(4), 1),
        ]
    );
}

#[test]
fn unspecified_rules_draw_nothing_and_refuse_ui() {
    let a = assets();
    let w = world(&[(0, 7), (1, 4)]);
    let f = build(&w, &[], &Unspecified, &a).unwrap();
    assert!(f.items.is_empty());
    assert_eq!((f.units_drawn, f.units_hidden), (0, 2));
    // An empty list composes to the cleared frame (index 0 everywhere).
    let rgba = compose_cpu(&f, &a).unwrap();
    let c = a.palette.colors[0];
    assert!(rgba.chunks(4).all(|p| p == [c.r, c.g, c.b, 255]));

    let e = build(&w, &[ui_image(0, 0)], &Unspecified, &a).unwrap_err();
    assert!(
        matches!(
            e,
            ViewError::Ui { index: 0, ref error }
                if matches!(**error, ViewError::Unresolved { spec: "ui/panels.md", .. })
        ),
        "{e}"
    );
}

#[test]
fn missing_assets_and_text_are_errors() {
    let w = world(&[(1, 4)]);
    let a = assets_without(&set_key(0, 0));
    match build(&w, &[], &TestRules, &a).unwrap_err() {
        ViewError::Unit { guid: 4, error, .. } => {
            assert!(error.to_string().contains("c0.dcc"), "{error}")
        }
        e => panic!("{e}"),
    }

    let a = assets_without(&tile_key());
    assert!(matches!(
        build(&w, &[], &TestRules, &a).unwrap_err(),
        ViewError::Tile { index: 0, .. }
    ));

    let mut a = assets();
    a.cofs.clear();
    assert!(matches!(
        build(&w, &[], &TestRules, &a).unwrap_err(),
        ViewError::CofMissing(p) if p == cof_path()
    ));

    let text = UiDraw::Text(TextRequest {
        text: vec![u16::from(b'a')],
        at: Point::new(0, 0),
        style: TextStyle::default(),
        opts: crate::ui::TextOpts::default(),
        clip: crate::ui::Rect::new(0, 0, 800, 600),
    });
    assert!(matches!(
        build(&w, &[ui_image(0, 0), text], &TestRules, &assets()).unwrap_err(),
        ViewError::Ui { index: 1, .. }
    ));
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// FNV-1a 64 of the CPU-composited RGBA frame of [`scene_frame`].
const GOLDEN: u64 = 0x0386_6d4b_e03b_3500;

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let o = (y * VIEW.width + x) as usize * 4;
    rgba[o..o + 4].try_into().unwrap()
}

fn color(a: &ViewAssets, i: u8) -> [u8; 4] {
    let c = a.palette.colors[usize::from(i)];
    [c.r, c.g, c.b, 255]
}

#[test]
fn cpu_frame_hash_is_golden() {
    let (f, a) = scene_frame();
    let rgba = compose_cpu(&f, &a).unwrap();
    assert_eq!(rgba.len(), 800 * 600 * 4);
    // Spot checks against the list: unit (1, 4) component 0 over 1.
    assert_eq!(pixel(&rgba, 42, 48), color(&a, 5));
    assert_eq!(pixel(&rgba, 41, 49), color(&a, 7));
    assert_eq!(pixel(&rgba, 40, 50), color(&a, 9));
    // Unit (0, 7): component 1 (index 6) over component 0 (index 4).
    assert_eq!(pixel(&rgba, 70, 50), color(&a, 6));
    assert_eq!(pixel(&rgba, 71, 50), color(&a, 4));
    assert_eq!(pixel(&rgba, 103, 101), color(&a, 3));
    assert_eq!(pixel(&rgba, 300, 200), color(&a, 6));
    assert_eq!(fnv1a(&rgba), GOLDEN, "hash {:#018x}", fnv1a(&rgba));
}

fn differing(a: &[u8], b: &[u8]) -> Vec<(u32, u32)> {
    a.chunks(4)
        .zip(b.chunks(4))
        .enumerate()
        .filter(|(_, (x, y))| x != y)
        .map(|(i, _)| (i as u32 % VIEW.width, i as u32 / VIEW.width))
        .collect()
}

// The hash check fails on exactly the perturbed input.
#[test]
fn frame_hash_catches_perturbations() {
    let (f, a) = scene_frame();
    let base = compose_cpu(&f, &a).unwrap();

    // One source pixel of one frame: exactly that screen pixel changes.
    let mut perturbed = sets();
    perturbed[0].1.frames[0].pixels[1] = 8;
    let a2 = assets_from(perturbed);
    let changed = compose_cpu(&f, &a2).unwrap();
    assert_eq!(differing(&base, &changed), vec![(42, 48)]);
    assert_ne!(fnv1a(&changed), fnv1a(&base));

    // One item (the 1×1 UI image) moved by one pixel: its old and new
    // pixel change, nothing else.
    let mut f2 = f.clone();
    f2.items[5].x += 1;
    let moved = compose_cpu(&f2, &a).unwrap();
    assert_eq!(differing(&base, &moved), vec![(300, 200), (301, 200)]);

    // Draw order: swapping two overlapping items changes exactly the
    // overlap (unit (0, 7) at (70, 50)).
    let mut f3 = f.clone();
    f3.items.swap(3, 4);
    let swapped = compose_cpu(&f3, &a).unwrap();
    assert_eq!(differing(&base, &swapped), vec![(70, 50)]);
}

#[test]
fn gpu_packing_emulates_to_the_cpu_image() {
    let (f, a) = scene_frame();
    let mut atlas = GpuAtlas::new(2).unwrap();
    atlas.ensure(&a.frames).unwrap();
    let packed = atlas.pack(&f, &a).unwrap();
    let gpu = emulate(&packed, atlas.atlas().pages()).unwrap();
    let cpu = scene::compose(&f.items, &a.frames, &a.maps, VIEW).unwrap();
    assert_eq!(gpu, cpu);
    // Frames go in once, in id order.
    assert_eq!(atlas.frames(), a.frames.len());
    let before = atlas.atlas().pages()[0].generation;
    atlas.ensure(&a.frames).unwrap();
    assert_eq!(atlas.atlas().pages()[0].generation, before);
    // A store that grew: only the new frames are packed, the old slots
    // stay; the frame still emulates to the CPU image.
    let old: Vec<_> = atlas.slots().to_vec();
    let mut grown = a.clone();
    let extra = FrameSetKey::new("data/global/tst/extra.dc6", FramePart::Dir(0)).unwrap();
    grown.frames.insert(extra, set(1, 1, 0, 0, &[2])).unwrap();
    atlas.ensure(&grown.frames).unwrap();
    assert_eq!(&atlas.slots()[..old.len()], &old[..]);
    assert_eq!(atlas.frames(), old.len() + 1);
    let packed = atlas.pack(&f, &grown).unwrap();
    assert_eq!(emulate(&packed, atlas.atlas().pages()).unwrap(), cpu);
    // An atlas ahead of its store (a rebuilt store) is refused.
    assert!(matches!(
        atlas.ensure(&a.frames),
        Err(ViewError::AtlasAhead { atlas: 6, store: 5 })
    ));
}

/// A session keeps every frame set it loaded (each level, act and
/// monster seen), and together they outgrow the atlas's pages. The atlas
/// places only the frames a draw list needs and starts over when they no
/// longer fit, so every draw list is still packed and composes to the CPU
/// image; placing the whole store at once is what does not fit.
#[test]
fn a_store_larger_than_the_atlas_keeps_every_draw_list_drawable() {
    let (f, a) = scene_frame();
    let mut grown = a.clone();
    // A big frame fills a whole page.
    let side = crate::frames::atlas::MAX_SIDE;
    let mut big = Vec::new();
    for k in 0..3u8 {
        let key =
            FrameSetKey::new(format!("data/global/tst/big{k}.dc6"), FramePart::Dir(0)).unwrap();
        let pixels = vec![k + 1; (side * side) as usize];
        grown
            .frames
            .insert(key.clone(), set(side, side, 0, 0, &pixels))
            .unwrap();
        big.push(grown.frames.id(&key, 0).unwrap());
    }
    // Two pages: the scene's frames and one big frame fit, two big frames
    // do not.
    let mut whole = GpuAtlas::new(2).unwrap();
    assert!(matches!(
        whole.ensure(&grown.frames),
        Err(ViewError::Atlas(AtlasError::Full { .. }))
    ));
    let mut atlas = GpuAtlas::new(2).unwrap();
    let mut versions = vec![atlas.version()];
    // Each "level" draws the scene and its own big frame, then the first
    // one is visited again.
    for &id in big.iter().chain(&big[..1]) {
        let mut frame = f.clone();
        let mut item = frame.items[0];
        item.frame = id;
        frame.items.push(item);
        atlas.ensure_for(&grown.frames, &frame.items).unwrap();
        versions.push(atlas.version());
        let packed = atlas.pack(&frame, &grown).unwrap();
        let cpu = scene::compose(&frame.items, &grown.frames, &grown.maps, VIEW).unwrap();
        assert_eq!(emulate(&packed, atlas.atlas().pages()).unwrap(), cpu);
        // Only the frames in view are placed.
        let placed = atlas.slots().iter().filter(|s| s.is_some()).count();
        assert!(placed <= f.items.len() + 1);
        assert!(atlas.atlas().pages().len() <= 2);
    }
    assert_eq!(atlas.restarts(), 3);
    // Every change of the pages is a new version for the render node.
    assert!(versions.windows(2).all(|v| v[0] < v[1]));
    // A draw list whose frames are all placed changes nothing.
    let version = atlas.version();
    let mut frame = f.clone();
    let mut item = frame.items[0];
    item.frame = big[0];
    frame.items.push(item);
    atlas.ensure_for(&grown.frames, &frame.items).unwrap();
    assert_eq!(atlas.version(), version);
}

#[test]
fn only_packed_pages_are_copied_for_the_node() {
    let (_, a) = scene_frame();
    let mut atlas = GpuAtlas::new(2).unwrap();
    atlas.ensure(&a.frames).unwrap();
    let first = atlas.share_pages(&[]);
    assert_eq!(first.len(), atlas.atlas().pages().len());
    // Nothing packed since: every page is shared.
    let same = atlas.share_pages(&first);
    assert!(same.iter().zip(&first).all(|(x, y)| Arc::ptr_eq(x, y)));
    // A new frame set: the page it went into is a fresh copy, the bytes
    // equal the atlas's.
    let mut grown = a.clone();
    let extra = FrameSetKey::new("data/global/tst/extra.dc6", FramePart::Dir(0)).unwrap();
    grown.frames.insert(extra, set(1, 1, 0, 0, &[2])).unwrap();
    atlas.ensure(&grown.frames).unwrap();
    let next = atlas.share_pages(&same);
    let copied = next
        .iter()
        .zip(&same)
        .filter(|(x, y)| !Arc::ptr_eq(x, y))
        .count();
    assert_eq!(copied, 1);
    for (page, real) in next.iter().zip(atlas.atlas().pages()) {
        assert_eq!(page.pixels, real.pixels);
    }
}

/// The frame of [`scene_frame`] composed onto `base` with `plan`: the
/// CPU reference of one frame of the cycle.
fn cycle_reference(f: &WorldFrame, a: &ViewAssets, base: &[u8], plan: scene::FramePlan) -> Vec<u8> {
    scene::compose_frame(&f.items, &a.frames, &a.maps, VIEW, base, plan).unwrap()
}

// The world view composes frames of the frame cycle: the framebuffer
// persists, BlankScreen clears rows 0..H − 47 before the draws, the
// post-draw clear blanks the frame and steps the counter.
#[test]
fn cpu_frames_run_the_frame_cycle() {
    let (f, a) = scene_frame();
    let pixels = (VIEW.width * VIEW.height) as usize;
    let all_5 = || FrameCycle::with_pixels(VIEW.width, VIEW.height, vec![5; pixels]).unwrap();
    let drawn = scene::compose(&f.items, &a.frames, &a.maps, VIEW).unwrap();

    // From an all-0 framebuffer the first frame is the single-frame image.
    let mut c = FrameCycle::new(VIEW.width, VIEW.height).unwrap();
    assert_eq!(
        compose_cycle_cpu(&mut c, true, &f, &a).unwrap(),
        compose_cpu(&f, &a).unwrap()
    );
    assert_eq!(c.pixels(), &drawn[..]);

    // BlankScreen 1 over a previous frame of 5: rows 553–599 keep 5 where
    // nothing draws; the presented image is the cycle's indices.
    let mut c = all_5();
    let plan = c.plan(true);
    assert_eq!(plan.clear_rows, 553);
    let want = cycle_reference(&f, &a, &vec![5; pixels], plan);
    let rgba = compose_cycle_cpu(&mut c, true, &f, &a).unwrap();
    assert_eq!(c.pixels(), &want[..]);
    assert_eq!(rgba, scene::to_rgba(&want, &a.palette));
    assert_eq!(c.pixels()[552 * 800 + 799], 0);
    assert_eq!(c.pixels()[553 * 800 + 799], 5);
    assert_eq!(c.pixels()[599 * 800], 5);
    assert_eq!(&c.pixels()[..553 * 800], &drawn[..553 * 800]);

    // BlankScreen 0: nothing cleared, the draws land on the 5s.
    let mut c = all_5();
    compose_cycle_cpu(&mut c, false, &f, &a).unwrap();
    assert_eq!(
        c.pixels(),
        &cycle_reference(&f, &a, &vec![5; pixels], scene::FramePlan::NONE)[..]
    );
    assert_eq!(c.pixels()[0], 5);

    // The next frame starts from the last one.
    let prev = c.pixels().to_vec();
    compose_cycle_cpu(&mut c, false, &f, &a).unwrap();
    assert_eq!(
        c.pixels(),
        &cycle_reference(&f, &a, &prev, scene::FramePlan::NONE)[..]
    );

    // Post-draw clear: that frame is all 0 and the counter steps down.
    c.set_post_clear(1);
    let rgba = compose_cycle_cpu(&mut c, true, &f, &a).unwrap();
    assert!(c.pixels().iter().all(|&i| i == 0));
    assert_eq!(rgba, scene::to_rgba(&vec![0; pixels], &a.palette));
    assert_eq!(c.post_clear(), 0);

    // A cycle of another size than the view is refused, unchanged.
    let mut small = FrameCycle::new(10, 48).unwrap();
    assert!(matches!(
        compose_cycle_cpu(&mut small, true, &f, &a),
        Err(ViewError::Scene(SceneError::BaseSize { .. }))
    ));
    assert_eq!(small, FrameCycle::new(10, 48).unwrap());
}

// The GPU path packs the same frame of the cycle: the previous frame as
// base and the plan's clears; its image (shader emulation) is the CPU
// cycle's, and committing it steps the cycle as the CPU path does.
#[test]
fn gpu_cycle_packing_emulates_to_the_cpu_cycle() {
    let (f, a) = scene_frame();
    let pixels = (VIEW.width * VIEW.height) as usize;
    let mut atlas = GpuAtlas::new(2).unwrap();
    atlas.ensure(&a.frames).unwrap();
    for (blank, post) in [(true, 0), (false, 0), (true, 1)] {
        let base: Vec<u8> = (0..pixels).map(|i| (i % 7) as u8).collect();
        let mut gpu = FrameCycle::with_pixels(VIEW.width, VIEW.height, base.clone()).unwrap();
        gpu.set_post_clear(post);
        let mut cpu = gpu.clone();
        let (packed, plan) = atlas.pack_cycle(&gpu, blank, &f, &a).unwrap();
        assert_eq!(plan, gpu.plan(blank));
        assert_eq!(packed.plan(), plan);
        let indices = emulate(&packed, atlas.atlas().pages()).unwrap();
        gpu.commit(plan, indices).unwrap();
        compose_cycle_cpu(&mut cpu, blank, &f, &a).unwrap();
        assert_eq!(gpu, cpu, "BlankScreen {blank}, counter {post}");
    }
}

#[test]
fn view_assets_present_the_pl2_palette() {
    let mut pl2 = vec![0xEE; 2048];
    pl2[..8].copy_from_slice(&[1, 2, 3, 0xFF, 0x10, 0x20, 0x30, 0]);
    let a = ViewAssets::from_pl2(&pl2).unwrap();
    assert_eq!(a.palette.colors[0], Rgb { r: 1, g: 2, b: 3 });
    assert_eq!(
        a.palette.colors[1],
        Rgb {
            r: 0x10,
            g: 0x20,
            b: 0x30
        }
    );
    assert_eq!(
        a.palette.colors[255],
        Rgb {
            r: 0xEE,
            g: 0xEE,
            b: 0xEE
        }
    );
    assert!(matches!(
        ViewAssets::from_pl2(&pl2[..1023]),
        Err(ViewError::Scene(SceneError::Pl2Size { len: 1023 }))
    ));
}

// Ids come from the store: another insertion order gives other ids for
// the same draws, and the same image.
#[test]
fn frame_ids_are_the_frame_stores() {
    let (f, a) = scene_frame();
    let mut reversed = sets();
    reversed.reverse();
    let b = assets_from(reversed);
    let w = world(&[(0, 7), (1, 4), (1, 9)]);
    let g = build(&w, &[ui_image(300, 200)], &TestRules, &b).unwrap();
    assert_eq!(rows(&g, &b), rows(&f, &a));
    for i in &g.items {
        let (key, n) = b.frames.owner(i.frame).unwrap();
        assert_eq!(b.frames.id(key, n).unwrap(), i.frame);
    }
    assert_ne!(g.items, f.items);
    assert_eq!(compose_cpu(&g, &b).unwrap(), compose_cpu(&f, &a).unwrap());
}

// --- UI binding and the Bevy edge ---------------------------------------

#[derive(Clone, Default)]
struct RecordingLink {
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    deliveries: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ServerLink for RecordingLink {
    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }
    fn tick_rate(&self) -> d2_sim::tick::TickRate {
        d2_sim::tick::TickRate::BASE
    }

    fn send(&mut self, _: SendQueue, msg: &[u8]) -> Result<Sent, LinkError> {
        self.sent.lock().unwrap().push(msg.to_vec());
        Ok(Sent::Queued)
    }

    fn pump(&mut self) -> Result<Pumped, LinkError> {
        Ok(Pumped { ticked: true })
    }

    fn receive(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut *self.deliveries.lock().unwrap())
    }
}

fn add_unit(world: &mut ClientWorld, msg: &Message<'_>) -> Result<(), HandlerError> {
    let key = msg.unit.ok_or(HandlerError::Invalid("no unit"))?;
    world.units.insert(key, ClientUnit::new(key));
    Ok(())
}

/// A bridge whose first frame adds units (0, 7), (1, 4), (1, 9) through a
/// synthetic 0x0E handler.
fn bridge_with_units() -> (Bridge<RecordingLink>, RecordingLink) {
    let link = RecordingLink::default();
    let mut chunk = Vec::new();
    for (t, g) in [(0u8, 7u32), (1, 4), (1, 9)] {
        let mut m = vec![0x0E, t];
        m.extend(g.to_le_bytes());
        m.resize(12, 0);
        chunk.extend(m);
    }
    link.deliveries.lock().unwrap().push(chunk);
    let mut d = Dispatch::empty();
    d.set(0x0E, "test", add_unit);
    (Bridge::with_dispatch(link.clone(), d).unwrap(), link)
}

/// Fixture camera feed: the player at client (1000, 2000), unit `guid`
/// at client (1000 + 10 × guid, 2000), one floor tile on cell (26, 18)
/// (screen (40, 40)), open mode 0, no shake.
struct TestFeed;

fn moving(px: i32, py: i32) -> crate::rules::UnitPosition {
    let (a, b) = (px + 2 * py, 2 * py - px);
    crate::rules::UnitPosition::Moving {
        x16: (a as u32) << 11,
        y16: (b as u32) << 11,
    }
}

impl crate::rules::ViewSource for TestFeed {
    fn unit_position(&self, u: &ClientUnit) -> Result<crate::rules::UnitPosition, String> {
        Ok(moving(1000 + 10 * u.key.guid as i32, 2000))
    }
    fn unit_offset(&self, _: &ClientUnit, _: &UnitPose) -> Result<(i32, i32), String> {
        Ok((0, 0))
    }
    fn map_tiles(
        &self,
        _: &ClientWorld,
        _: &ViewAssets,
    ) -> Result<Vec<crate::rules::MapTile>, ViewError> {
        Ok(vec![crate::rules::MapTile {
            cell: (26, 18),
            list: crate::rules::TileList::Floor,
            frame: ComponentFrame {
                set: tile_key(),
                index: 0,
            },
            blocks: Vec::new(),
            shade: ShadeChain::EMPTY,
            blend: BlendOp::Opaque,
            key: DrawKey::new(0, 0, 0, 0).unwrap(),
        }])
    }
}

impl ViewFeed for TestFeed {
    fn player(&self, _: &ClientWorld) -> Result<Option<crate::rules::UnitPosition>, ViewError> {
        Ok(Some(moving(1000, 2000)))
    }
    fn open_mode(&self, _: &ClientWorld) -> Result<crate::rules::OpenMode, ViewError> {
        Ok(crate::rules::OpenMode::NONE)
    }
    fn shake(&self, _: &ClientWorld) -> Result<Option<RunningShake>, ViewError> {
        Ok(None)
    }
    fn player_seed(&mut self, _: &ClientWorld) -> Result<&mut d2_sim::rng::Seed, ViewError> {
        unreachable!("no shake")
    }
    fn blank_screen(&self, _: &ClientWorld) -> Result<bool, ViewError> {
        Ok(true)
    }
}

/// C→S Walk to (3, 4).
const WALK: [u8; 5] = [0x01, 3, 0, 4, 0];

/// A panel at (290, 190, 100×100) drawing one image; a press sends Walk.
struct TestPanel;

impl Panel for TestPanel {
    fn id(&self) -> PanelId {
        PanelId(1)
    }
    fn rect(&self) -> crate::ui::Rect {
        crate::ui::Rect::new(290, 190, 100, 100)
    }
    fn draw(&self, ctx: &UiCtx, out: &mut dyn UiDrawSink) {
        // The panel sees the bridge's model read-only.
        assert!(ctx.tick >= 1 && ctx.world.units.len() == 3);
        out.push(ui_image(300, 200));
    }
    fn hit(&self, _: Point) -> Option<WidgetId> {
        Some(WidgetId(0))
    }
    fn event(&mut self, e: UiEvent, _: &UiCtx) -> UiResponse {
        match e {
            UiEvent::Press { .. } => UiResponse::Intent(ClientIntent(WALK.to_vec())),
            _ => UiResponse::Ignored,
        }
    }
}

fn ui_root() -> UiRoot {
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    root.add(Box::new(TestPanel)).unwrap();
    root.open(PanelId(1)).unwrap();
    root
}

const PRESS: UiEvent = UiEvent::Press {
    button: PointerButton::Left,
    at: Point::new(300, 200),
};

#[test]
fn ui_binding_routes_forwards_and_draws() {
    let (mut bridge, link) = bridge_with_units();
    bridge.frame().unwrap();
    let mut root = ui_root();
    let outside = UiEvent::CursorMoved(Point::new(10, 10));
    let mut queue = UiQueue(vec![outside, PRESS]);
    let f = ui_bind::run_ui(&mut root, &mut queue, &mut bridge, &NoStrings).unwrap();
    assert_eq!(f.unhandled, vec![outside]);
    assert_eq!(f.sent, 1);
    assert_eq!(f.draws, vec![ui_image(300, 200)]);
    assert!(queue.0.is_empty());
    assert_eq!(*link.sent.lock().unwrap(), vec![WALK.to_vec()]);

    let frame = build(bridge.world(), &f.draws, &TestRules, &assets()).unwrap();
    assert_eq!(frame, scene_frame().0);
}

#[test]
fn bevy_frame_presents_the_cpu_image() {
    let (bridge, link) = bridge_with_units();
    let boxed = Bridge::with_dispatch(Box::new(link.clone()) as _, {
        let mut d = Dispatch::empty();
        d.set(0x0E, "test", add_unit);
        d
    })
    .unwrap();
    drop(bridge);

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_plugins((crate::bridge::BridgePlugin, WorldViewPlugin { gpu: false }))
        .insert_resource(BridgeResource(boxed))
        .insert_resource(WorldViewState::new(
            assets(),
            Box::new(TestRules),
            Box::new(TestFeed),
        ));
    let mut ui = WorldViewUi::new(ui_root(), Box::new(NoStrings));
    ui.queue.0.push(PRESS);
    app.insert_non_send(ui);

    app.update();

    let state = app.world().resource::<WorldViewState>();
    let stats = state.last.unwrap();
    assert_eq!(
        (
            stats.bridge_frame,
            stats.items,
            stats.units_drawn,
            stats.units_hidden
        ),
        (1, 6, 2, 1)
    );
    assert_eq!((stats.ui_sent, stats.gpu), (1, false));
    assert_eq!(*link.sent.lock().unwrap(), vec![WALK.to_vec()]);

    // The app builds through the base-game placement (`OriginalView`,
    // camera from the feed), not the fixture's `place` / `tiles`.
    let a = assets();
    let w = world(&[(0, 7), (1, 4), (1, 9)]);
    let f = build_frame(&w, &[ui_image(300, 200)], &TestRules, &mut TestFeed, &a).unwrap();
    assert_eq!(f.items.len(), 6);
    let expected = compose_cpu(&f, &a).unwrap();
    let target = app.world().resource::<present::WorldViewTarget>();
    let image = app
        .world()
        .resource::<Assets<Image>>()
        .get(&target.image)
        .unwrap();
    assert_eq!(image.data.as_deref(), Some(&expected[..]));
    // The presented frame is the frame cycle's framebuffer:
    // the first frame starts from all 0.
    let cycle = &app.world().resource::<WorldViewState>().cycle;
    assert_eq!(
        cycle.pixels(),
        &scene::compose(&f.items, &a.frames, &a.maps, VIEW).unwrap()[..]
    );

    // Second frame: the same target image is overwritten, not re-created,
    // with the next frame of the cycle.
    let handle = target.image.clone();
    app.update();
    let target = app.world().resource::<present::WorldViewTarget>();
    assert_eq!(target.image, handle);
    let image = app
        .world()
        .resource::<Assets<Image>>()
        .get(&target.image)
        .unwrap();
    let cycle = &app.world().resource::<WorldViewState>().cycle;
    assert_eq!(
        image.data.as_deref(),
        Some(&scene::to_rgba(cycle.pixels(), &a.palette)[..])
    );
    assert_eq!(
        app.world()
            .resource::<WorldViewState>()
            .last
            .unwrap()
            .bridge_frame,
        2
    );
}

/// A compose job the render world never composes (it failed the job, or
/// keeps failing it) must not freeze the picture: live play waits a few
/// ticks for the job, as it does for a slow GPU, then gives it up and
/// hands the render world the next tick's job.
#[test]
fn a_job_the_render_world_never_composes_does_not_freeze_the_picture() {
    use super::node::ComposeJob;
    use super::present::{WorldViewGpu, NODE_STALL_TICKS};
    let (bridge, link) = bridge_with_units();
    let boxed = Bridge::with_dispatch(Box::new(link.clone()) as _, {
        let mut d = Dispatch::empty();
        d.set(0x0E, "test", add_unit);
        d
    })
    .unwrap();
    drop(bridge);
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>()
        .add_plugins((crate::bridge::BridgePlugin, WorldViewPlugin { gpu: false }))
        .insert_resource(BridgeResource(boxed))
        .insert_resource(WorldViewState::new(
            assets(),
            Box::new(TestRules),
            Box::new(TestFeed),
        ))
        // The main-world half of the GPU path, with no render world: no
        // job is ever composed.
        .insert_resource(WorldViewGpu::new().unwrap());
    app.insert_non_send(WorldViewUi::new(ui_root(), Box::new(NoStrings)));
    let job = |app: &App| app.world().get_resource::<ComposeJob>().map(|j| j.seq);
    app.update();
    assert_eq!(job(&app), Some(1));
    // While the job may still come back, the next tick waits for it.
    for _ in 0..NODE_STALL_TICKS - 1 {
        app.update();
        assert_eq!(job(&app), Some(1));
    }
    // Then it is given up and the picture moves on, again and again.
    app.update();
    assert_eq!(job(&app), Some(2));
    for _ in 0..NODE_STALL_TICKS {
        app.update();
    }
    assert_eq!(job(&app), Some(3));
    assert_eq!(app.world().resource::<WorldViewState>().skipped_frames, 0);
}

/// [`TestFeed`] with a frame light: every unit stands on sub-tile (10,
/// 20) at intensity 0x7F; component looks carry remap `MapId(3)`.
struct LitFeed {
    light: crate::rules::lighting::view::FrameLight,
}

impl crate::rules::lighting::view::LookFeed for LitFeed {
    fn light_subtile(&self, _: &ClientUnit) -> Result<(i32, i32), String> {
        Ok((10, 20))
    }
    fn look(
        &self,
        _: &ClientUnit,
        _: &ComponentRequest<'_>,
    ) -> Result<crate::rules::lighting::view::ComponentLook, String> {
        Ok(crate::rules::lighting::view::ComponentLook {
            ghostly: false,
            override_input: None,
            hovered: false,
            remap: Some(scene::MapId(3)),
            pal: None,
        })
    }
}

impl crate::rules::ViewSource for LitFeed {
    fn unit_position(&self, u: &ClientUnit) -> Result<crate::rules::UnitPosition, String> {
        TestFeed.unit_position(u)
    }
    fn unit_offset(&self, u: &ClientUnit, p: &UnitPose) -> Result<(i32, i32), String> {
        TestFeed.unit_offset(u, p)
    }
    fn map_tiles(
        &self,
        w: &ClientWorld,
        a: &ViewAssets,
    ) -> Result<Vec<crate::rules::MapTile>, ViewError> {
        TestFeed.map_tiles(w, a)
    }
}

impl ViewFeed for LitFeed {
    fn player(&self, w: &ClientWorld) -> Result<Option<crate::rules::UnitPosition>, ViewError> {
        TestFeed.player(w)
    }
    fn open_mode(&self, w: &ClientWorld) -> Result<crate::rules::OpenMode, ViewError> {
        TestFeed.open_mode(w)
    }
    fn shake(&self, _: &ClientWorld) -> Result<Option<RunningShake>, ViewError> {
        Ok(None)
    }
    fn player_seed(&mut self, _: &ClientWorld) -> Result<&mut d2_sim::rng::Seed, ViewError> {
        unreachable!("no shake")
    }
    fn blank_screen(&self, _: &ClientWorld) -> Result<bool, ViewError> {
        Ok(true)
    }
    fn light(&self, _: &ClientWorld) -> Result<Option<FeedLight<'_>>, ViewError> {
        Ok(Some(FeedLight {
            light: &self.light,
            look: self,
        }))
    }
}

// A feed that states the frame's light gets every unit component's shade
// and blend from `LitRules` (cel ops of the unit's light-map cell);
// tiles and UI keep the rules' answers.
#[test]
fn build_frame_lights_units_through_the_feeds_light() {
    use crate::rules::blend::cel_ops;
    use crate::rules::lighting::view::FrameLight;
    use crate::rules::lighting::LightMap;
    use crate::rules::shading::ShadeTables;
    let mut maps = scene::MapTable::new();
    let tables = ShadeTables::push(&mut maps, &crate::rules::lighting::view_tests::pl2());
    let mut map = LightMap::new((10, 20));
    let (gx, gy) = (10 - map.origin.0, 20 - map.origin.1);
    map.cell_mut(gx, gy).unwrap().i = 0x7F;
    let mut feed = LitFeed {
        light: FrameLight { tables, map },
    };
    let a = assets();
    let w = world(&[(0, 7), (1, 4), (1, 9)]);
    let lit = build_frame(&w, &[ui_image(300, 200)], &TestRules, &mut feed, &a).unwrap();
    let plain = build_frame(&w, &[ui_image(300, 200)], &TestRules, &mut TestFeed, &a).unwrap();
    assert_eq!(lit.items.len(), plain.items.len());
    let (shade, blend) = cel_ops(&tables, 5, Some(scene::MapId(3)), 0x7F);
    let mut units = 0;
    for (l, p) in lit.items.iter().zip(&plain.items) {
        if matches!(l.tag, ItemTag::Unit(_)) {
            units += 1;
            assert_eq!((l.shade, l.blend), (shade, blend));
            assert_eq!((l.x, l.y, l.frame), (p.x, p.y, p.frame));
        } else {
            assert_eq!(l, p);
        }
    }
    assert!(units > 0);
}

#[test]
fn outputs_reach_the_ui_and_the_sound_requests_in_order() {
    use crate::audio::driver::SoundRequest;
    use crate::bridge::BridgePlugin;
    use crate::ui::layout::Screen;
    use crate::ui::original::{OriginalUi, UiConfig};
    let link = RecordingLink::default();
    // Object 13, a 0x5D whose UI row is sound 7, a 0x2C on the object,
    // then the stash (0x77 0x10).
    let chunk: Vec<u8> = [
        "51 02 0d 00 00 00 25 00 14 12 c0 11 02 00",
        "5d 08 02 00 00 00",
        "2c 02 0d 00 00 00 0d 00",
        "77 10",
    ]
    .iter()
    .flat_map(|m| {
        m.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
    })
    .collect();
    link.deliveries.lock().unwrap().push(chunk);
    let bridge = Bridge::new(Box::new(link) as _).unwrap();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .add_plugins((BridgePlugin, WorldViewPlugin { gpu: false }))
        .insert_resource(BridgeResource(bridge));
    let mut root = UiRoot::new(Box::new(NoPanelRules));
    let original = OriginalUi::new(
        UiConfig {
            screen: Screen::R800,
            expansion_installed: false,
        },
        None,
    )
    .unwrap();
    original.install(&mut root).unwrap();
    let mut ui = WorldViewUi::new(root, Box::new(NoStrings));
    ui.original = Some(original);
    app.insert_non_send(ui);
    app.update();
    assert_eq!(
        app.world().resource::<UiSounds>().0,
        [
            SoundRequest::Ui(7),
            SoundRequest::Server {
                unit: UnitKey::new(2, 13),
                class: 37,
                at: Some((0x1214, 0x11C0)),
                event: 13
            }
        ]
    );
    let ui = app.world().non_send::<WorldViewUi>();
    assert!(ui.original.as_ref().unwrap().is_open(25), "the stash");
    assert!(app
        .world()
        .resource::<crate::bridge::FrameOutputs>()
        .0
        .is_empty());
}

#[test]
fn event_text_codes_that_end_in_a_screen_message_request_sound_6() {
    use super::present::event_text_adds_line;
    // The local player's own join (code 2) adds the empty line.
    assert!(event_text_adds_line(2));
    assert!((0..=5).all(event_text_adds_line));
    assert!(event_text_adds_line(0xD));
    // 0x12 adds its line after the shake.
    assert!(event_text_adds_line(0x12));
    // 7 (other functions), 0xC, 0xE and above 0x12 add none.
    assert!(![7, 0xC, 0xE, 0x13, 0xFF]
        .into_iter()
        .any(event_text_adds_line));
}

#[test]
fn ui_overlay_calls_reach_the_effect_layer_at_their_output() {
    use crate::bridge::output::Output;
    use crate::bridge::world::MONSTER;
    use crate::ui::layout::Screen;
    use crate::ui::original::msg_ui::OverlayCall;
    use crate::ui::original::{OriginalUi, UiConfig};
    use present::{deliver_with, EffectIn};

    let mut bridge = Bridge::new(RecordingLink::default()).unwrap();
    let config = UiConfig {
        screen: Screen::R800,
        expansion_installed: true,
    };
    let mut ui = OriginalUi::new(config, None).unwrap();
    let k = UnitKey::new(MONSTER, 7);
    // An effect output, then 0x8A (the UI's overlay 72), then another
    // effect output: the base game creates the three overlays in that order.
    let overlay = |id| Output::UnitOverlay {
        unit: k,
        overlay: id,
        mode: 3,
        sound: 0,
    };
    let list = [
        overlay(10),
        Output::NpcInteract {
            unit: k,
            present: true,
            class: 148,
            mdata_3c: Some(-1),
            blocker_open: false,
        },
        overlay(11),
    ];
    let mut seen = Vec::new();
    let mut effects = |e: EffectIn<'_>| match e {
        EffectIn::Output(Output::UnitOverlay { overlay, .. }) => seen.push(*overlay),
        EffectIn::Output(_) => {}
        EffectIn::Overlays(calls) => {
            assert_eq!(
                calls,
                [OverlayCall {
                    unit: k,
                    id: 72,
                    on: true
                }]
            );
            seen.push(72);
        }
    };
    deliver_with(&mut bridge, &list, Some(&mut ui), false, Some(&mut effects)).unwrap();
    assert_eq!(seen, [10, 72, 11]);
    assert!(ui.take_overlay_calls().is_empty());
}

/// The pre-pass colour of the unit marker:
/// blue for an `interact` class, else red / yellow / green by life
/// percent (stat 6 × 100 / stat 7: below 33, below 66, else); max life 0
/// is green.
#[test]
fn the_hit_flash_colour_follows_life_and_interact() {
    let key = UnitKey::new(1, 7);
    let at = |life: i32, max: i32, interact: bool| {
        let mut u = ClientUnit::new(key);
        u.stats.insert(6, life);
        u.stats.insert(7, max);
        u.hit_flash_interact = interact;
        hit_flash_rgb(&u)
    };
    let (red, yellow, green) = ((0xE0, 0, 0), (0xE0, 0xE0, 0), (0, 0xC0, 0));
    assert_eq!(at(0x8000, 0x8000, false), green);
    assert_eq!(at(0x5480, 0x8000, false), green); // 66 %
    assert_eq!(at(0x5400, 0x8000, false), yellow); // 65 %
    assert_eq!(at(0x2A40, 0x8000, false), yellow); // 33 %
    assert_eq!(at(0x2A00, 0x8000, false), red); // 32 %
    assert_eq!(at(0x100, 0, false), green);
    assert_eq!(at(0x100, 0x8000, true), (0, 0, 0xC0));
}
