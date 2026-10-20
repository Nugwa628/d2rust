//! The native UI pass (`ui_pass`) and the compositor's scene output that
//! goes with it: buffer layouts, shader validity (naga), placement against
//! the mouse mapping, the sharp rule per item, blending, and the CPU
//! emulation against the GPU (ignored tests, local, need a GPU).

use d2_formats::palette::{Palette, Rgb};
use wgpu::naga;

use super::harness::{cases, prepare};
use super::pack::{self, AtlasFrames, LAYER_SCREEN, LAYER_UI, LAYER_WORLD};
use super::ui_pass::*;
use crate::scene::{self, BlendOp, DrawItem, FrameId, FrameImage, MapId, MapTable, Rect};
use crate::ui::frame::{FramePos, Presentation, ScaleMode};

fn module() -> naga::Module {
    naga::front::wgsl::parse_str(UI_SHADER).expect("WGSL parses")
}

#[test]
fn the_ui_shader_validates_with_naga() {
    let module = module();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .expect("WGSL validates");
    let entries: Vec<(&str, [u32; 3])> = module
        .entry_points
        .iter()
        .map(|e| (e.name.as_str(), e.workgroup_size))
        .collect();
    assert_eq!(
        entries,
        [("draw_ui", [super::WORKGROUP, super::WORKGROUP, 1])]
    );
}

#[test]
fn the_ui_shader_structs_and_bindings_match_packing() {
    let module = module();
    let mut layouter = naga::proc::Layouter::default();
    layouter.update(module.to_ctx()).expect("layout");
    let layout = |name: &str| {
        let (handle, ty) = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some(name))
            .expect("struct exists");
        let naga::TypeInner::Struct { members, .. } = &ty.inner else {
            panic!("{name} is a struct");
        };
        let offsets: Vec<(String, u32)> = members
            .iter()
            .map(|m| (m.name.clone().unwrap_or_default(), m.offset))
            .collect();
        (layouter[handle].size, offsets)
    };
    let owned = |v: &[(&str, u32)]| -> Vec<(String, u32)> {
        v.iter().map(|&(n, o)| (n.to_string(), o)).collect()
    };
    assert_eq!(
        layout("Item"),
        (
            UI_ITEM_SIZE as u32,
            owned(&[
                ("area", 0),
                ("texel", 16),
                ("src", 32),
                ("shade", 48),
                ("blend", 64),
                ("place", 80),
                ("rep", 96)
            ])
        )
    );
    assert_eq!(layout("Params").0, UI_PARAMS_SIZE as u32);
    let mut globals: Vec<(u32, String)> = module
        .global_variables
        .iter()
        .filter_map(|(_, g)| Some((g.binding.as_ref()?.binding, g.name.clone()?)))
        .collect();
    globals.sort();
    let names: Vec<(u32, &str)> = globals.iter().map(|(b, n)| (*b, n.as_str())).collect();
    assert_eq!(
        names,
        [
            (0, "params"),
            (1, "items"),
            (2, "bin_ranges"),
            (3, "bin_items"),
            (4, "maps"),
            (5, "atlas"),
            (6, "palette"),
            (7, "cover_out"),
            (8, "add_out"),
            (9, "rep_atlas"),
        ]
    );
}

#[test]
fn ui_items_round_trip_their_bytes() {
    let item = UiItem {
        area: [1, 2, 3, 4],
        texel: [5, 6],
        page: 7,
        shade_len: 2,
        src: [0, 1, 9, 10],
        shade: [11, 12, 0, 0],
        blend: 1,
        alpha: 192,
        source: SOURCE_REPLACEMENT,
        place: [-1.5, 2.0, 3.6, 2.4],
        rep: [13, 14, 2.5f32.to_bits(), REP_TINTED | 77],
    };
    assert_eq!(UiItem::from_le_bytes(&item.to_le_bytes()), item);
}

/// A palette with distinct colours: index `i` is `(i, 255 − i, i / 2)`.
fn palette() -> Palette {
    let mut colors = [Rgb { r: 0, g: 0, b: 0 }; 256];
    for (i, c) in colors.iter_mut().enumerate() {
        *c = Rgb {
            r: i as u8,
            g: 255 - i as u8,
            b: (i / 2) as u8,
        };
    }
    Palette { colors }
}

/// Synthetic UI: frames, the frame store's atlas and a frame-sized view.
struct Ui {
    frames: Vec<FrameImage>,
    atlas: AtlasFrames,
    maps: MapTable,
    view: Rect,
}

impl Ui {
    fn new(frames: Vec<FrameImage>, view: Rect) -> Self {
        let atlas = AtlasFrames::from_images(&frames, 4).unwrap();
        Ui {
            frames,
            atlas,
            maps: MapTable::new(),
            view,
        }
    }

    fn pack(&self, items: &[(DrawItem, UiBlend)], place: UiPlacement) -> UiPacked {
        self.pack_as(items, place, CelImage::Original)
    }

    fn pack_as(
        &self,
        items: &[(DrawItem, UiBlend)],
        place: UiPlacement,
        image: CelImage,
    ) -> UiPacked {
        let draws: Vec<UiDraw<'_>> = items
            .iter()
            .map(|(item, blend)| UiDraw {
                item,
                blend: *blend,
                image,
            })
            .collect();
        pack_ui(
            &draws,
            &self.frames,
            &self.atlas.slots,
            self.atlas.page_count(),
            self.maps.len() as u32,
            self.view,
            place,
        )
        .unwrap()
    }

    fn run(&self, items: &[(DrawItem, UiBlend)], place: UiPlacement) -> Vec<UiPixel> {
        emulate_ui(
            &self.pack(items, place),
            self.atlas.pages(),
            &self.maps,
            &palette(),
        )
    }
}

/// A replacement atlas of one page holding `pixels` (premultiplied RGBA
/// rows, `w` wide) at (`x`, `y`).
fn rep_atlas(x: u32, y: u32, w: u32, pixels: &[[u8; 4]]) -> std::sync::Arc<RepAtlas> {
    let mut page = RepAtlas::blank_page();
    for (i, px) in pixels.iter().enumerate() {
        let (px_x, px_y) = (x + i as u32 % w, y + i as u32 / w);
        let at = ((px_y * REP_PAGE + px_x) * 4) as usize;
        page[at..at + 4].copy_from_slice(px);
    }
    std::sync::Arc::new(RepAtlas {
        id: 1,
        pages: vec![page],
    })
}

/// A tinted replacement covers exactly the cel's output pixels, coloured
/// by the tint through the palette, and is placed in the packed item.
#[test]
fn a_tinted_replacement_covers_the_cel_in_its_colour() {
    let view = Rect::new(0, 0, 100, 100);
    let ui = Ui::new(vec![solid(4, 4, 9)], view);
    let place = place(100, 100, 200, 200);
    let image = CelImage::Replacement {
        page: 0,
        rect: [8, 16, 8, 8],
        scale: 2.0,
        tint: Some(50),
    };
    let items = [(at(0, 10, 10, view), UiBlend::Opaque)];
    let mut packed = ui.pack_as(&items, place, image);
    let it = packed.items[0];
    assert_eq!(it.source, SOURCE_REPLACEMENT);
    assert_eq!((it.texel, it.page), ([8, 16], 0));
    assert_eq!(it.rep, [8, 8, 2.0f32.to_bits(), REP_TINTED | 50]);
    assert_eq!(it.area, [20, 20, 28, 28]);
    packed.set_rep(Some(rep_atlas(8, 16, 8, &[[255; 4]; 64])));
    assert_eq!(packed.params.rep_pages, 1);
    let out = emulate_ui(&packed, ui.atlas.pages(), &ui.maps, &palette());
    let c = palette().colors[50];
    let want = [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0);
    for oy in 0..200u32 {
        for ox in 0..200u32 {
            let px = out[(oy * 200 + ox) as usize];
            if (20..28).contains(&ox) && (20..28).contains(&oy) {
                assert_eq!(px.cover, [1.0; 3], "({ox}, {oy})");
                for (got, want) in px.add.iter().zip(want) {
                    assert!((got - want).abs() < 1e-6, "({ox}, {oy})");
                }
            } else {
                assert_eq!(px, UiPixel::default(), "({ox}, {oy})");
            }
        }
    }
}

/// A replacement is sampled bilinearly in its own pixels, transparent past
/// its edges, and draws nothing without its atlas.
#[test]
fn a_replacement_is_sampled_bilinearly_and_needs_its_atlas() {
    let view = Rect::new(0, 0, 100, 100);
    let ui = Ui::new(vec![solid(1, 1, 9)], view);
    let place = place(100, 100, 400, 400);
    let image = CelImage::Replacement {
        page: 0,
        rect: [0, 0, 2, 2],
        scale: 2.0,
        tint: None,
    };
    let items = [(at(0, 0, 0, view), UiBlend::Opaque)];
    let mut packed = ui.pack_as(&items, place, image);
    let none = emulate_ui(&packed, ui.atlas.pages(), &ui.maps, &palette());
    assert!(none.iter().all(|px| *px == UiPixel::default()));
    let (o, w) = ([0; 4], [255; 4]);
    packed.set_rep(Some(rep_atlas(0, 0, 2, &[o, w, o, w])));
    let out = emulate_ui(&packed, ui.atlas.pages(), &ui.maps, &palette());
    let row: Vec<f32> = (0..5)
        .map(|x| out[(2 * 400 + x) as usize].cover[0])
        .collect();
    let want = [0.0, 0.25, 0.75, 0.75, 0.0];
    for (got, want) in row.iter().zip(want) {
        assert!((got - want).abs() < 1e-5, "{row:?}");
    }
    assert_eq!(out[2 * 400 + 2].add, out[2 * 400 + 2].cover, "white");
}

/// A replacement is clipped where its cel is: only the cel's drawable
/// part (in frame pixels) shows.
#[test]
fn a_replacement_is_clipped_like_its_cel() {
    let view = Rect::new(0, 0, 100, 100);
    let ui = Ui::new(vec![solid(4, 4, 9)], view);
    let place = place(100, 100, 300, 300);
    let image = CelImage::Replacement {
        page: 0,
        rect: [0, 0, 12, 12],
        scale: 3.0,
        tint: None,
    };
    let mut item = at(0, 10, 10, view);
    item.clip = Rect::new(0, 0, 12, 100);
    let mut packed = ui.pack_as(&[(item, UiBlend::Opaque)], place, image);
    packed.set_rep(Some(rep_atlas(0, 0, 12, &[[255; 4]; 144])));
    let out = emulate_ui(&packed, ui.atlas.pages(), &ui.maps, &palette());
    for ox in 27..45u32 {
        let px = out[(35 * 300 + ox) as usize];
        let shown = (30..36).contains(&ox);
        assert_eq!(px.cover, [if shown { 1.0 } else { 0.0 }; 3], "column {ox}");
    }
}

fn solid(w: u32, h: u32, index: u8) -> FrameImage {
    FrameImage {
        width: w,
        height: h,
        pixels: vec![index; (w * h) as usize],
    }
}

fn at(frame: u32, x: i32, y: i32, view: Rect) -> DrawItem {
    let mut d = DrawItem::new(FrameId(frame), x, y);
    d.clip = view;
    d
}

fn place(frame_w: u32, frame_h: u32, out_w: u32, out_h: u32) -> UiPlacement {
    UiPlacement {
        frame_w,
        frame_h,
        out_w,
        out_h,
    }
}

/// The output pixels an item covers are exactly those the mouse maps into
/// its frame pixels, at 1080p, 1440p and 2160p.
#[test]
fn an_item_covers_the_output_pixels_the_mouse_maps_into_it() {
    let view = Rect::new(0, 0, 1068, 600);
    for (w, h) in [(1920, 1080), (2560, 1440), (3840, 2160)] {
        let p = Presentation::with_mode(w, h, 1068, 600, ScaleMode::Fit).unwrap();
        let place = place(1068, 600, p.image_w, p.image_h);
        let ui = Ui::new(vec![solid(37, 11, 9)], view);
        for (x, y) in [(0, 0), (134, 553), (517, 300), (1031, 589)] {
            let packed = ui.pack(&[(at(0, x, y, view), UiBlend::Opaque)], place);
            let [x0, y0, x1, y1] = packed.items[0].area;
            for ox in x0.saturating_sub(2)..(x1 + 2).min(p.image_w) {
                let inside_out = (x0..x1).contains(&ox);
                let FramePos::Inside(f) = p.to_frame(
                    i64::from(ox) + i64::from(p.left),
                    i64::from(y0) + i64::from(p.top),
                ) else {
                    panic!("inside the image");
                };
                let inside_frame = (x..x + 37).contains(&f.x);
                assert_eq!(inside_out, inside_frame, "{w}x{h} item at {x}: column {ox}");
            }
            for oy in y0.saturating_sub(2)..(y1 + 2).min(p.image_h) {
                let FramePos::Inside(f) = p.to_frame(
                    i64::from(x0) + i64::from(p.left),
                    i64::from(oy) + i64::from(p.top),
                ) else {
                    panic!("inside the image");
                };
                assert_eq!((y0..y1).contains(&oy), (y..y + 11).contains(&f.y));
            }
        }
    }
}

/// Panel pieces side by side meet without a gap or a blended seam at a
/// fractional scale: every output pixel of the two is fully covered.
#[test]
fn side_by_side_pieces_meet_without_a_seam_at_3_6x() {
    let view = Rect::new(0, 0, 1068, 600);
    let ui = Ui::new(vec![solid(13, 7, 40), solid(13, 7, 41)], view);
    let place = place(1068, 600, 3845, 2160);
    for x in [0, 1, 2, 3, 4, 101] {
        let items = [
            (at(0, x, 5, view), UiBlend::Opaque),
            (at(1, x + 13, 5, view), UiBlend::Opaque),
        ];
        let packed = ui.pack(&items, place);
        assert_eq!(packed.items[0].area[2], packed.items[1].area[0], "abut");
        let out = ui.run(&items, place);
        let [x0, y0, _, y1] = packed.items[0].area;
        let x1 = packed.items[1].area[2];
        for oy in y0..y1 {
            for ox in x0..x1 {
                let px = out[(oy * place.out_w + ox) as usize];
                assert_eq!(px.cover, [1.0; 3], "({ox}, {oy}) at x {x}");
            }
        }
    }
}

/// At a whole-number scale every output pixel is the nearest frame pixel:
/// opaque art replaces the picture with exactly its palette colour.
#[test]
fn a_whole_number_scale_draws_nearest_pixels() {
    let view = Rect::new(0, 0, 40, 30);
    let image = FrameImage {
        width: 3,
        height: 2,
        pixels: vec![10, 0, 30, 40, 50, 60],
    };
    let ui = Ui::new(vec![image.clone()], view);
    let place = place(40, 30, 120, 90);
    let out = ui.run(&[(at(0, 4, 5, view), UiBlend::Opaque)], place);
    let pal = palette();
    for oy in 0..90u32 {
        for ox in 0..120u32 {
            let (fx, fy) = (ox / 3, oy / 3);
            let px = out[(oy * 120 + ox) as usize];
            let index = (4..7)
                .contains(&fx)
                .then_some(())
                .and((5..7).contains(&fy).then_some(()))
                .map(|_| image.pixels[((fy - 5) * 3 + fx - 4) as usize])
                .unwrap_or(0);
            if index == 0 {
                assert_eq!(px, UiPixel::default(), "({ox}, {oy}) is untouched");
            } else {
                let c = pal.colors[usize::from(index)];
                let (cover, add) = px.bytes();
                assert_eq!(cover, [255; 4], "({ox}, {oy})");
                assert_eq!(add, [c.r, c.g, c.b, 255], "({ox}, {oy})");
            }
        }
    }
}

/// Inside an item every frame pixel is a solid block at 3.6x; only the
/// one output pixel where two meet is a blend of both.
#[test]
fn inside_an_item_pixels_are_solid_blocks_with_one_blended_edge() {
    let view = Rect::new(0, 0, 100, 10);
    let image = FrameImage {
        width: 4,
        height: 1,
        pixels: vec![200, 100, 200, 100],
    };
    let ui = Ui::new(vec![image], view);
    let place = place(100, 10, 360, 36);
    let out = ui.run(&[(at(0, 10, 3, view), UiBlend::Opaque)], place);
    let row = 3 * 36 / 10 + 1;
    let reds: Vec<f32> = (36..50)
        .map(|x| out[(row * 360 + x) as usize].add[0])
        .collect();
    let solid = |v: f32| (v - 200.0 / 255.0).abs() < 1e-4 || (v - 100.0 / 255.0).abs() < 1e-4;
    let blended = reds.iter().filter(|&&v| !solid(v)).count();
    // Four pixels of 3.6: three seams inside, at most one blended output
    // pixel each.
    assert!(blended <= 3, "{reds:?}");
    assert!(reds.iter().filter(|&&v| solid(v)).count() >= 11, "{reds:?}");
}

#[test]
fn blends_keep_add_and_scale_the_picture_under_them() {
    let view = Rect::new(0, 0, 20, 20);
    let ui = Ui::new(vec![solid(4, 4, 128)], view);
    let place = place(20, 20, 40, 40);
    let c = palette().colors[128];
    let colour = [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0);
    let pixel =
        |blend: UiBlend| ui.run(&[(at(0, 2, 2, view), blend)], place)[(5 * 40 + 5) as usize];
    let under = [0.5, 0.25, 0.3];
    let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5);
    // Opaque: the art.
    assert!(near(pixel(UiBlend::Opaque).apply(under), colour));
    // A quarter alpha: three quarters of the picture, a quarter of the art.
    let quarter: [f32; 3] = std::array::from_fn(|i| under[i] * 0.75 + colour[i] * 0.25);
    assert!(near(pixel(UiBlend::Alpha(64)).apply(under), quarter));
    let added: [f32; 3] = std::array::from_fn(|i| under[i] + colour[i]);
    assert!(near(pixel(UiBlend::Add).apply(under), added));
    let scaled: [f32; 3] = std::array::from_fn(|i| under[i] * colour[i]);
    assert!(near(pixel(UiBlend::Mul).apply(under), scaled));
    // Outside the art: untouched.
    let out = ui.run(&[(at(0, 2, 2, view), UiBlend::Opaque)], place);
    assert_eq!(out[0], UiPixel::default());
}

/// Items compose in order: an opaque panel under a half-alpha tooltip
/// shows the tooltip over the panel, not over the world.
#[test]
fn items_compose_in_draw_order() {
    let view = Rect::new(0, 0, 20, 20);
    let ui = Ui::new(vec![solid(8, 8, 200), solid(4, 4, 0x10)], view);
    let place = place(20, 20, 40, 40);
    let items = [
        (at(0, 0, 0, view), UiBlend::Opaque),
        (at(1, 2, 2, view), UiBlend::Alpha(128)),
    ];
    let px = ui.run(&items, place)[(5 * 40 + 5) as usize];
    let pal = palette();
    let (p, t) = (pal.colors[200], pal.colors[0x10]);
    let want = [(p.r, t.r), (p.g, t.g), (p.b, t.b)]
        .map(|(a, b)| (f32::from(a) * 0.5 + f32::from(b) * 0.5) / 255.0);
    let got = px.apply([0.9, 0.1, 0.3]);
    assert!(
        got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-5),
        "{got:?} {want:?}"
    );
}

#[test]
fn draw_modes_become_ui_blends() {
    let kinds = |b: MapId| match b.0 {
        10 => Some(TableKind::Alpha(64)),
        20 => Some(TableKind::Add),
        30 => Some(TableKind::Mul),
        40 => Some(TableKind::Max),
        _ => None,
    };
    assert_eq!(ui_blend(BlendOp::Opaque, kinds), UiBlend::Opaque);
    assert_eq!(
        ui_blend(BlendOp::IndexTable(MapId(10)), kinds),
        UiBlend::Alpha(64)
    );
    // Read transposed: the picture's share and the art's swap.
    assert_eq!(
        ui_blend(BlendOp::IndexTableSrcRow(MapId(10)), kinds),
        UiBlend::Alpha(192)
    );
    assert_eq!(
        ui_blend(BlendOp::IndexTable(MapId(20)), kinds),
        UiBlend::Add
    );
    assert_eq!(
        ui_blend(BlendOp::IndexTable(MapId(30)), kinds),
        UiBlend::Mul
    );
    assert_eq!(
        ui_blend(BlendOp::IndexTable(MapId(40)), kinds),
        UiBlend::Add
    );
    assert_eq!(
        ui_blend(BlendOp::IndexTable(MapId(99)), kinds),
        UiBlend::Alpha(128)
    );
}

/// An alpha table built for a known share is measured back as that share.
#[test]
fn an_alpha_table_measures_its_share() {
    // A grey ramp palette, so every blend has an exact nearest entry.
    let mut colors = [Rgb { r: 0, g: 0, b: 0 }; 256];
    for (i, c) in colors.iter_mut().enumerate() {
        *c = Rgb {
            r: i as u8,
            g: i as u8,
            b: i as u8,
        };
    }
    let pal = Palette { colors };
    for share in [64u16, 128, 192] {
        let a = f64::from(share) / 256.0;
        let rows: Vec<[u8; 256]> = (0..256)
            .map(|d| std::array::from_fn(|s| (d as f64 * (1.0 - a) + s as f64 * a).round() as u8))
            .collect();
        let got = fit_alpha(&rows, &pal);
        assert!(got.abs_diff(share) <= 1, "{share}: {got}");
    }
}

/// The compositor's scene output: the frame without the UI items, and the
/// presented RGBA and screen layer follow it only with the native UI flag.
#[test]
fn the_scene_is_the_frame_without_its_ui_items() {
    for case in cases() {
        let (atlas, mut packed) = prepare(&case).unwrap();
        let layer = |i: usize| match i % 3 {
            0 => LAYER_WORLD,
            1 => LAYER_SCREEN,
            _ => LAYER_UI,
        };
        packed.set_layers(layer);
        let reference = case.reference().unwrap();
        let px = pack::emulate_layers(&packed, atlas.pages()).unwrap();
        let index: Vec<u8> = px.iter().map(|p| p.index).collect();
        assert_eq!(index, reference, "{}: the frame keeps its UI", case.name);
        let mut scene_case = case.clone();
        scene_case.items = case
            .items
            .iter()
            .enumerate()
            .filter(|&(i, _)| layer(i) != LAYER_UI)
            .map(|(_, it)| *it)
            .collect();
        let scene: Vec<u8> = px.iter().map(|p| p.scene).collect();
        assert_eq!(
            scene,
            scene_case.reference().unwrap(),
            "{}: scene",
            case.name
        );
        // Without the flag the presented image is the frame, as before.
        for p in &px {
            assert_eq!(p.shown(false), p.index);
            assert_eq!(p.shown(true), p.scene);
            assert_eq!(p.on_screen_layer(false), p.screen);
            assert_eq!(p.on_screen_layer(true), p.scene_screen);
        }
        // The world layer leaves out both screen and UI items, as before.
        let mut world_case = case.clone();
        world_case.items = case
            .items
            .iter()
            .enumerate()
            .filter(|&(i, _)| layer(i) == LAYER_WORLD)
            .map(|(_, it)| *it)
            .collect();
        let world: Vec<u8> = px.iter().map(|p| p.world).collect();
        assert_eq!(
            world,
            world_case.reference().unwrap(),
            "{}: world",
            case.name
        );
    }
}

/// UI items tagged as UI behave exactly as screen items when the native
/// UI pass is off: the layer outputs are unchanged.
#[test]
fn ui_items_are_screen_items_without_the_native_pass() {
    for case in cases() {
        let (atlas, mut packed) = prepare(&case).unwrap();
        packed.set_screen_layer(|i| i % 2 == 1);
        let screen = pack::emulate_layers(&packed, atlas.pages()).unwrap();
        packed.set_layers(|i| if i % 2 == 1 { LAYER_UI } else { LAYER_WORLD });
        let ui = pack::emulate_layers(&packed, atlas.pages()).unwrap();
        for (a, b) in screen.iter().zip(&ui) {
            assert_eq!((a.index, a.world, a.screen), (b.index, b.world, b.screen));
        }
    }
}

fn gpu() -> super::Gpu {
    let (gpu, info) = super::Gpu::headless().expect("a GPU adapter");
    eprintln!("adapter: {} ({:?})", info.name, info.backend);
    gpu
}

/// Runs the pass on the GPU into two storage textures and reads them back
/// (RGBA8 rows, tightly packed).
fn gpu_run(
    gpu: &super::Gpu,
    packed: &UiPacked,
    pages: &[crate::frames::atlas::AtlasPage],
    maps: &MapTable,
    pal: &Palette,
) -> (Vec<u8>, Vec<u8>, std::time::Duration) {
    let device = gpu.device();
    let (w, h) = (packed.params.width, packed.params.height);
    let atlas = gpu.upload_atlas(pages);
    // The frame inputs of an empty frame with this map table.
    let frame_items: Vec<DrawItem> = Vec::new();
    let frames: Vec<FrameImage> = Vec::new();
    let view = Rect::new(0, 0, 1, 1);
    let bins = scene::bin(&frame_items, &frames, maps, view).unwrap();
    let slots: Vec<crate::frames::AtlasSlot> = Vec::new();
    let frame = pack::pack(
        &frame_items,
        &bins,
        &frames,
        &slots,
        maps,
        pages.len() as u32,
    )
    .unwrap();
    let inputs = gpu.frame_inputs(&frame, Some(pal));
    let texture = || {
        device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let (cover, add) = (texture(), texture());
    let pass = UiPass::new(device);
    let run = || {
        let mut encoder = device.create_command_encoder(&Default::default());
        pass.encode(
            device,
            &mut encoder,
            packed,
            &atlas,
            inputs.maps(),
            inputs.palette().unwrap(),
            &cover.create_view(&Default::default()),
            &add.create_view(&Default::default()),
        )
        .unwrap();
        let start = std::time::Instant::now();
        gpu.queue().submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        start.elapsed()
    };
    run();
    let took = (0..20).map(|_| run()).min().unwrap();
    let read = |t: &wgpu::Texture| {
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            t.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            t.size(),
        );
        gpu.queue().submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = buffer.get_mapped_range(..).to_vec();
        bytes
            .chunks(row as usize)
            .flat_map(|r| r[..(w * 4) as usize].to_vec())
            .collect::<Vec<u8>>()
    };
    (read(&cover), read(&add), took)
}

/// A synthetic HUD-like list: a panel of pieces, translucent boxes and many
/// small glyphs, over a 1068 x 600 frame.
fn busy_ui() -> (Ui, Vec<(DrawItem, UiBlend)>) {
    let view = Rect::new(0, 0, 1068, 600);
    let mut frames = vec![solid(256, 48, 30), solid(120, 80, 0x20)];
    for g in 0..40u8 {
        let mut glyph = solid(7, 10, 0);
        for (i, p) in glyph.pixels.iter_mut().enumerate() {
            if (i + usize::from(g)) % 3 != 0 {
                *p = 100 + g;
            }
        }
        frames.push(glyph);
    }
    let ui = Ui::new(frames, view);
    let mut items = Vec::new();
    for x in 0..5 {
        items.push((at(0, 22 + 256 * x, 552, view), UiBlend::Opaque));
    }
    for k in 0..6 {
        items.push((
            at(1, 40 + 170 * k, 200 + 13 * k, view),
            UiBlend::Alpha(64 + 32 * (k as u16 % 4)),
        ));
    }
    for line in 0..20 {
        for c in 0..60 {
            let g = 2 + ((line * 7 + c) % 40) as u32;
            let blend = if c % 11 == 0 {
                UiBlend::Add
            } else {
                UiBlend::Opaque
            };
            items.push((at(g, 30 + 8 * c, 20 + 12 * line, view), blend));
        }
    }
    (ui, items)
}

/// Local, needs a GPU: the pass's output equals the CPU emulation within
/// one unit of 255 per channel, at 1.8x and 3.6x.
/// `cargo test -p d2-client --lib gpu_compositor::ui_pass_tests::gpu_ui_pass_matches_emulation -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU"]
fn gpu_ui_pass_matches_emulation() {
    let gpu = gpu();
    let (ui, items) = busy_ui();
    for (w, h) in [(1922, 1080), (3845, 2160)] {
        let place = place(1068, 600, w, h);
        let packed = ui.pack(&items, place);
        let want = emulate_ui(&packed, ui.atlas.pages(), &ui.maps, &palette());
        let (cover, add, _) = gpu_run(&gpu, &packed, ui.atlas.pages(), &ui.maps, &palette());
        let mut worst = 0u8;
        for (i, p) in want.iter().enumerate() {
            let (c, a) = p.bytes();
            for ch in 0..4 {
                worst = worst
                    .max(c[ch].abs_diff(cover[4 * i + ch]))
                    .max(a[ch].abs_diff(add[4 * i + ch]));
            }
        }
        assert!(worst <= 1, "{w}x{h}: off by {worst}");
    }
}

/// Local, needs a GPU: the pass's cost at 2160p for a busy UI.
/// `cargo test --release -p d2-client --lib gpu_compositor::ui_pass_tests::gpu_ui_pass_cost_2160p -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU"]
fn gpu_ui_pass_cost_2160p() {
    let gpu = gpu();
    let (ui, items) = busy_ui();
    let place = place(1068, 600, 3845, 2160);
    let packed = ui.pack(&items, place);
    let (_, _, took) = gpu_run(&gpu, &packed, ui.atlas.pages(), &ui.maps, &palette());
    println!(
        "native UI pass at 3845x2160, {} items: {:.3} ms per tick frame (best of 20)",
        packed.items.len(),
        took.as_secs_f64() * 1000.0
    );
}

/// Local, needs a GPU: the cost of presenting a display frame with the
/// native UI at 2160p (`world_view/native_ui.wgsl`'s work: the world and
/// screen layers sharp-sampled, the two UI images read per pixel, the UI
/// applied in gamma), for comparison with the plain layer present of
/// `tests::gpu_present_cost_2160p`.
/// `cargo test --release -p d2-client --lib gpu_compositor::ui_pass_tests::gpu_native_present_cost_2160p -- --ignored --nocapture`
#[test]
#[ignore = "needs a GPU"]
fn gpu_native_present_cost_2160p() {
    const SHADER: &str = r"
@group(0) @binding(0) var world: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var screen: texture_2d<f32>;
@group(0) @binding(3) var cover: texture_2d<f32>;
@group(0) @binding(4) var add: texture_2d<f32>;

struct Out { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs(@builtin(vertex_index) i: u32) -> Out {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: Out;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

fn sharp(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(t));
    let texel = uv * size;
    let base = floor(texel);
    let d = fract(texel) - 0.5;
    let solid = 0.5 - 0.5 / 4.0;
    let f = (d - clamp(d, vec2<f32>(-solid), vec2<f32>(solid))) * 4.0 + 0.5;
    return textureSampleLevel(t, smp, (base + f) / size, 0.0);
}

fn to_gamma(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@fragment
fn fs(o: Out) -> @location(0) vec4<f32> {
    let w = sharp(world, o.uv + vec2<f32>(0.001, 0.0));
    let s = sharp(screen, o.uv);
    let picture = s.rgb + w.rgb * (1.0 - s.a);
    let size = vec2<i32>(textureDimensions(cover));
    let at = clamp(vec2<i32>(floor(o.uv * vec2<f32>(size))), vec2<i32>(0), size - 1);
    let c = textureLoad(cover, at, 0).rgb;
    let a = textureLoad(add, at, 0).rgb;
    let out = to_gamma(picture) * (1.0 - c) + a;
    return vec4<f32>(to_linear(clamp(out, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}
";
    let (w, h) = (3840u32, 2160u32);
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapters = bevy::tasks::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    for adapter in adapters {
        let info = adapter.get_info();
        if info.device_type == wgpu::DeviceType::Cpu {
            continue;
        }
        let Ok((device, queue)) =
            bevy::tasks::block_on(adapter.request_device(&Default::default()))
        else {
            continue;
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("native present cost"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("native present cost"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let texture = |width, height, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let bind = wgpu::TextureUsages::TEXTURE_BINDING;
        let target = texture(w, h, format, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let target_view = target.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let views: Vec<wgpu::TextureView> = [
            texture(1068, 600, format, bind),
            texture(1068, 600, format, bind),
            texture(w, h, wgpu::TextureFormat::Rgba8Unorm, bind),
            texture(w, h, wgpu::TextureFormat::Rgba8Unorm, bind),
        ]
        .iter()
        .map(|t| t.create_view(&Default::default()))
        .collect();
        let entry = |binding, resource| wgpu::BindGroupEntry { binding, resource };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                entry(0, wgpu::BindingResource::TextureView(&views[0])),
                entry(1, wgpu::BindingResource::Sampler(&sampler)),
                entry(2, wgpu::BindingResource::TextureView(&views[1])),
                entry(3, wgpu::BindingResource::TextureView(&views[2])),
                entry(4, wgpu::BindingResource::TextureView(&views[3])),
            ],
        });
        let runs = 200;
        let (mut total, mut min) = (std::time::Duration::ZERO, std::time::Duration::MAX);
        for n in 0..runs + 10 {
            let start = std::time::Instant::now();
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
            queue.submit([encoder.finish()]);
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll");
            let took = start.elapsed();
            if n >= 10 {
                total += took;
                min = min.min(took);
            }
        }
        println!(
            "{} ({:?}, {:?}): 2160p native UI present {:.3} ms (min {:.3} ms) per display frame",
            info.name,
            info.backend,
            info.device_type,
            total.as_secs_f64() * 1000.0 / f64::from(runs),
            min.as_secs_f64() * 1000.0,
        );
    }
}
#[test]
fn enhanced_glyph_indices_keep_every_palette_colour_and_colour_code() {
    let view = Rect::new(0, 0, 256, 1);
    let mut ui = Ui::new(
        vec![FrameImage {
            width: 256,
            height: 1,
            pixels: (0..=255).collect(),
        }],
        view,
    );
    let a = ui.maps.push(std::array::from_fn(|i| (255 - i) as u8));
    let b = ui
        .maps
        .push(std::array::from_fn(|i| (i as u8).wrapping_add(37)));
    let mut draw = at(0, 0, 0, view);
    draw.shade = scene::ShadeChain::new(&[a, b]).unwrap();
    let items = [(draw, UiBlend::Opaque)];
    let placement = place(256, 1, 512, 2);
    let bitmap = ui.run(&items, placement);
    let mut enhanced = ui.pack_as(
        &items,
        placement,
        CelImage::IndexedGlyph {
            page: 0,
            rect: [0, 0, 512, 2],
            scale: 2.0,
        },
    );
    let pixels: Vec<[u8; 4]> = (0..1024)
        .map(|i| {
            let index = ((i % 512) / 2) as u8;
            [index, 0, 0, if index == 0 { 0 } else { 255 }]
        })
        .collect();
    enhanced.set_rep(Some(rep_atlas(0, 0, 512, &pixels)));
    let enhanced = emulate_ui(&enhanced, ui.atlas.pages(), &ui.maps, &palette());
    assert_eq!(
        enhanced, bitmap,
        "all 256 indices must retain both colour maps exactly"
    );
}
