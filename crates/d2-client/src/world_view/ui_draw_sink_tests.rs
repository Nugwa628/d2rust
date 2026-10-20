//! The UI draw sink's rectangles, draw modes and remaps through the world
//! view (`ui_items`, `PanelArtRules`, the CPU reference compositor), on
//! synthetic PL2 tables (repo only; the act 1 values are
//! `tests/game_ui_draw_sink.rs`, ignored). The tables are asymmetric so
//! a swapped row / column or a wrong table shows.

use std::sync::{Arc, RwLock};

use d2_formats::palette::{Palette, Pl2, Rgb};

use super::*;
use crate::assets::path::MemorySource;
use crate::bridge::world::ClientWorld;
use crate::rules::camera::FrameSize;
use crate::rules::shading::ShadeTables;
use crate::scene::{self, FramePlan};
use crate::ui::panels::{PanelTables, UiFiles};
use crate::ui::{CelLook, ImageRef, Point, RectRequest, Remap, FRAME};
use crate::world_view::panel_art::{PanelArtLoader, PanelArtRules};
use crate::world_view::{build, Unspecified};

fn map(f: impl Fn(u32) -> u32) -> [u8; 256] {
    let mut m = [0u8; 256];
    for (i, v) in m.iter_mut().enumerate() {
        *v = (f(i as u32) % 256) as u8;
    }
    m
}

/// An asymmetric 256×256 table, `[row][column]`.
fn table(k: u32) -> Vec<[u8; 256]> {
    (0..256u32)
        .map(|d| map(|s| d * (3 + k) + s * (5 + 2 * k) + k + 1))
        .collect()
}

/// A synthetic PL2; light map `k` sends every index to `50 + k`.
fn pl2() -> Pl2 {
    Pl2 {
        base_palette: Palette {
            colors: [Rgb::default(); 256],
        },
        light_levels: (0..32).map(|k| map(move |_| 50 + k)).collect(),
        inventory_variations: (0..16).map(|k| map(move |i| i + k)).collect(),
        selected_unit_shift: map(|i| i),
        alpha_blend: (0..3).map(table).collect(),
        additive_blend: table(3),
        multiplicative_blend: table(4),
        hue_variations: (0..111).map(|k| map(move |i| i + k + 1)).collect(),
        red_tones: map(|i| i),
        green_tones: map(|i| i),
        blue_tones: map(|i| i),
        unknown_variations: (0..14).map(|k| map(move |i| i ^ k)).collect(),
        max_component_blend: table(5),
        darkened_shift: map(|i| i / 2),
        text_colors: Vec::new(),
        text_color_shifts: Vec::new(),
    }
}

/// Text-colour map `k` sends `i` to `(i + 7k) mod 256`.
fn text_map(k: u32) -> [u8; 256] {
    map(move |i| i + 7 * k)
}

struct Setup {
    assets: ViewAssets,
    pl2: Pl2,
    files: UiFiles,
    colors: TextColors,
}

fn setup(shades: bool) -> Setup {
    let p = pl2();
    let mut assets = ViewAssets::new(Palette {
        colors: [Rgb::default(); 256],
    });
    if shades {
        assets.shades = Some(ShadeTables::push(&mut assets.maps, &p));
    }
    let mut maps = [MapId(0); TEXT_COLORS - 1];
    for (k, m) in maps.iter_mut().enumerate() {
        *m = assets.maps.push(text_map(k as u32 + 1));
    }
    let mut files = PanelTables::load().unwrap().files;
    files.add(ICON);
    files.add(HORADRIC);
    Setup {
        assets,
        pl2: p,
        files,
        colors: TextColors { maps },
    }
}

fn rules(s: &Setup) -> PanelArtRules<Unspecified> {
    PanelArtRules {
        rules: Unspecified,
        files: s.files.clone(),
        text: Some(Arc::new(RwLock::new(Some(s.colors)))),
    }
}

/// A DC6 of one direction, one 2 × 1 frame of pixels `a`, `b`.
fn dc6(a: u8, b: u8) -> Vec<u8> {
    let rows = [2u8, a, b, 0x80];
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&1u32.to_le_bytes());
    d.extend_from_slice(&1u32.to_le_bytes());
    let at = d.len() as u32 + 4;
    d.extend_from_slice(&at.to_le_bytes());
    for v in [0u32, 2, 1, 0, 0, 0, 0, rows.len() as u32] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&rows);
    d.extend_from_slice(&[0xEE; 3]);
    d
}

/// The indices of `draws` composed over a frame of index `under`
/// (resident through the panel loader first).
fn compose(s: &mut Setup, src: MemorySource, draws: &[UiDraw], under: u8) -> Vec<u8> {
    let loader = PanelArtLoader::new(Arc::new(src), s.files.clone());
    loader.ensure(draws, &mut s.assets).unwrap();
    let r = rules(s);
    let frame = build(&ClientWorld::default(), draws, &r, &s.assets).unwrap();
    let view = scene::Rect::FRAME;
    let base = vec![under; (view.width * view.height) as usize];
    scene::compose_frame(
        &frame.items,
        &s.assets.frames,
        &s.assets.maps,
        view,
        &base,
        FramePlan::NONE,
    )
    .unwrap()
}

fn at(px: &[u8], x: i32, y: i32) -> u8 {
    px[(y * FRAME_WIDTH as i32 + x) as usize]
}

const FRAME_WIDTH: u32 = scene::FRAME_WIDTH;

fn rect(x0: i32, y0: i32, x1: i32, y1: i32, color: u8, mode: u8) -> UiDraw {
    UiDraw::Rect(RectRequest {
        x0,
        y0,
        x1,
        y1,
        color,
        mode,
    })
}

#[test]
fn rectangles_write_by_the_blend_kind_of_their_mode() {
    let mut s = setup(true);
    let (d, c) = (40u8, 9u8);
    let a2 = &s.pl2.alpha_blend[2];
    let a0 = &s.pl2.alpha_blend[0];
    let add = s.pl2.additive_blend.clone();
    let mx = s.pl2.max_component_blend.clone();
    let (a2, a0) = (a2.clone(), a0.clone());
    let draws = [
        rect(10, 10, 12, 12, c, 5), // k 0: d' = color
        rect(20, 10, 22, 12, c, 0), // k 2: A2[256·d + color]
        rect(30, 10, 32, 12, c, 2), // k 2: A0[256·d + color]
        rect(40, 10, 42, 12, c, 3), // k 1: ADD[d] (row 0, column d)
        rect(50, 10, 52, 12, c, 6), // k 1: MAX[d]
        rect(60, 10, 62, 12, c, 7), // k 0 (mode 7)
        rect(70, 10, 72, 12, c, 9), // other: k 0
    ];
    let px = compose(&mut s, MemorySource::default(), &draws, d);
    let (d, c) = (usize::from(d), usize::from(c));
    assert_eq!(at(&px, 10, 10), c as u8);
    assert_eq!(at(&px, 20, 10), a2[d][c]);
    assert_eq!(at(&px, 30, 10), a0[d][c]);
    assert_eq!(at(&px, 40, 10), add[0][d]);
    assert_eq!(at(&px, 50, 10), mx[0][d]);
    assert_eq!(at(&px, 60, 10), c as u8);
    assert_eq!(at(&px, 70, 10), c as u8);
    // The transposed or swapped reads would differ on these tables.
    assert_ne!(a2[d][c], a2[c][d]);
    assert_ne!(add[0][d], add[d][0]);
    // Columns x0 … x1 − 1, rows y0 … y1 − 1.
    assert_eq!(at(&px, 11, 11), c as u8);
    assert_eq!(at(&px, 12, 10), d as u8);
    assert_eq!(at(&px, 10, 12), d as u8);
    assert_eq!(at(&px, 9, 10), d as u8);
}

#[test]
fn rectangles_clamp_to_the_surface_and_refuse_reversed_edges() {
    let s = setup(true);
    let r = |x0, y0, x1, y1| RectRequest {
        x0,
        y0,
        x1,
        y1,
        color: 1,
        mode: 5,
    };
    let mut a = s.assets;
    ensure_rects(
        &[UiDraw::Rect(r(-5, -5, 10_000, 10_000))],
        FrameSize::LOW,
        &mut a,
    )
    .unwrap();
    // Clamped to [0, W − 1] × [0, H − 1]: the last column and row are
    // never reached (639 × 479 pixels at 640 × 480).
    let sp = rect_sprite(&r(-5, -5, 10_000, 10_000), FrameSize::LOW, &a)
        .unwrap()
        .unwrap();
    assert_eq!((sp.x, sp.y), (0, 0));
    assert_eq!(sp.frame.set, rect_key(639, 479));
    let sp = rect_sprite(&r(-5, -5, 10_000, 10_000), FrameSize::D2RUST, &a)
        .unwrap()
        .unwrap();
    assert_eq!(sp.frame.set, rect_key(799, 599));
    // x0 = x1 (after the clamp) or y0 = y1: nothing, before any y test.
    assert_eq!(
        rect_sprite(&r(5, 9, 5, 3), FrameSize::D2RUST, &a).unwrap(),
        None
    );
    assert_eq!(
        rect_sprite(&r(900, 0, 1000, 5), FrameSize::D2RUST, &a).unwrap(),
        None
    );
    assert_eq!(
        rect_sprite(&r(0, 4, 5, 4), FrameSize::D2RUST, &a).unwrap(),
        None
    );
    // y1 < y0: fatal 0x32; x1 < x0: the base game faults.
    assert!(rect_sprite(&r(0, 9, 5, 3), FrameSize::D2RUST, &a).is_err());
    assert!(rect_sprite(&r(9, 0, 3, 5), FrameSize::D2RUST, &a).is_err());
    assert!(ensure_rects(&[UiDraw::Rect(r(0, 9, 5, 3))], FrameSize::D2RUST, &mut a).is_err());
}

#[test]
fn the_ui_rectangle_primitive_is_x_y_w_h_in_mode_0() {
    // The rectangle with (x, y, 29, 29, color, 0) = DrawRectangle(x, y, x + 29,
    // y + 29, color, 0): pixels x … x + 28, y … y + 28, A2[256·d + c].
    let r = RectRequest::sized(100, 200, 29, 29, 4, 0);
    assert_eq!(
        (r.x0, r.y0, r.x1, r.y1, r.color, r.mode),
        (100, 200, 129, 229, 4, 0)
    );
    let mut s = setup(true);
    let a2 = s.pl2.alpha_blend[2].clone();
    let px = compose(&mut s, MemorySource::default(), &[UiDraw::Rect(r)], 17);
    assert_eq!(at(&px, 100, 200), a2[17][4]);
    assert_eq!(at(&px, 128, 228), a2[17][4]);
    assert_eq!(at(&px, 129, 228), 17);
    assert_eq!(at(&px, 128, 229), 17);
}

#[test]
fn a_blended_rectangle_without_the_act_tables_is_an_error() {
    let mut s = setup(false);
    let opaque = [rect(0, 0, 2, 2, 3, 5)];
    let px = compose(&mut s, MemorySource::default(), &opaque, 1);
    assert_eq!(at(&px, 1, 1), 3);
    let blended = [rect(0, 0, 2, 2, 3, 0)];
    let loader = PanelArtLoader::new(Arc::new(MemorySource::default()), s.files.clone());
    loader.ensure(&blended, &mut s.assets).unwrap();
    assert!(build(&ClientWorld::default(), &blended, &rules(&s), &s.assets).is_err());
}

fn cel(s: &Setup, name: &str, x: i32, y: i32, look: CelLook) -> UiDraw {
    UiDraw::Image(ImageRequest {
        image: ImageRef {
            file: s.files.id(name).unwrap(),
            frame: 0,
        },
        at: Point::new(x, y),
        clip: FRAME,
        look,
        call: crate::ui::draw::CelCall::Draw,
    })
}

fn src(name: &str, a: u8, b: u8) -> MemorySource {
    let mut m = MemorySource::default();
    m.insert(&format!("data\\global\\ui\\{name}.dc6"), dc6(a, b));
    m
}

const ICON: &str = "spells\\soskillicon";
const HORADRIC: &str = "menu\\horadric";

#[test]
fn colored_cel_draws_remap_by_k() {
    let mut s = setup(true);
    let (a, b) = (30u8, 31u8);
    // The skill tree's grey icon: the colored cel draw with k = 5,
    // mode 5: d' = map_5[s].
    let grey = CelLook {
        mode: 5,
        remap: Remap::Palette(5),
    };
    let draws = [
        cel(&s, ICON, 10, 10, CelLook::PLAIN),
        cel(&s, ICON, 20, 10, grey),
        cel(
            &s,
            ICON,
            30,
            10,
            CelLook {
                mode: 5,
                remap: Remap::Palette(0),
            },
        ),
    ];
    let px = compose(&mut s, src(ICON, a, b), &draws, 2);
    assert_eq!((at(&px, 10, 10), at(&px, 11, 10)), (a, b));
    let m5 = text_map(5);
    assert_eq!(
        (at(&px, 20, 10), at(&px, 21, 10)),
        (m5[usize::from(a)], m5[usize::from(b)])
    );
    assert_eq!((at(&px, 30, 10), at(&px, 31, 10)), (a, b));
}

#[test]
fn negative_k_reads_the_light_maps_and_the_rest_is_an_error() {
    let s = setup(true);
    let t = s.assets.shades.unwrap();
    let colors = Some(&s.colors);
    // −18 … −48 → light maps 31 … 1.
    assert_eq!(
        ui_remap(Remap::Palette(-18), colors, &s.assets).unwrap(),
        Some(t.light_map(31))
    );
    assert_eq!(
        ui_remap(Remap::Palette(-48), colors, &s.assets).unwrap(),
        Some(t.light_map(1))
    );
    assert_eq!(
        ui_remap(Remap::Palette(12), colors, &s.assets).unwrap(),
        Some(s.colors.maps[11])
    );
    // −1 (selected-unit shift), −2 … −17 (inventory variations), −49
    // and 13 have no map here: errors, never a default.
    for k in [-1, -2, -17, -49, 13] {
        assert!(
            ui_remap(Remap::Palette(k), colors, &s.assets).is_err(),
            "{k}"
        );
    }
    // k 1–12 need the frame's text maps.
    assert!(ui_remap(Remap::Palette(3), None, &s.assets).is_err());
}

#[test]
fn draw_mode_3_cels_add_and_the_other_modes_pick_their_table() {
    let mut s = setup(true);
    let (a, b) = (30u8, 31u8);
    let file = HORADRIC;
    let look = |mode| CelLook {
        mode,
        remap: Remap::None,
    };
    let draws = [
        cel(&s, file, 10, 10, look(3)),
        cel(&s, file, 20, 10, look(1)),
        cel(&s, file, 30, 10, look(8)),
    ];
    let mut m = MemorySource::default();
    m.insert("data\\global\\ui\\menu\\horadric.dc6", dc6(a, b));
    let d = 50u8;
    let px = compose(&mut s, m, &draws, d);
    let (add, a1) = (&s.pl2.additive_blend, &s.pl2.alpha_blend[1]);
    let (d, a, b) = (usize::from(d), usize::from(a), usize::from(b));
    assert_eq!((at(&px, 10, 10), at(&px, 11, 10)), (add[d][a], add[d][b]));
    assert_eq!((at(&px, 20, 10), at(&px, 21, 10)), (a1[d][a], a1[d][b]));
    // A mode outside 0–7 draws as mode 5.
    assert_eq!((at(&px, 30, 10), at(&px, 31, 10)), (a as u8, b as u8));
}

#[test]
fn a_remapped_blended_cel_reads_the_remap_then_the_table() {
    // T[256·d + P[s]] (cel drawers): the remap first, then the table.
    let s = setup(true);
    let t = s.assets.shades.unwrap();
    let p = s.colors.maps[4];
    let (shade, blend) = ui_cel_ops(Some(&t), 2, Some(p)).unwrap();
    assert_eq!(shade, ShadeChain::new(&[p]).unwrap());
    assert_eq!(blend, BlendOp::IndexTable(t.alpha[0]));
    // Without the act's tables only the table-free modes draw.
    assert_eq!(
        ui_cel_ops(None, 5, Some(p)).unwrap(),
        (ShadeChain::new(&[p]).unwrap(), BlendOp::Opaque)
    );
    assert!(ui_cel_ops(None, 0, None).is_err());
    assert!(ui_cel_ops(None, 7, None).is_err());
}

#[test]
fn text_drawn_with_a_mode_blends_its_remapped_glyphs() {
    let s = setup(true);
    let t = s.assets.shades.unwrap();
    let hooks = OriginalTextHooks {
        colors: Some(s.colors),
        shades: Some(t),
    };
    // The glyph draw with mode: colour k remaps, the mode picks T.
    let (shade, blend) = hooks.glyph_look(4, 3).unwrap();
    assert_eq!(shade, ShadeChain::new(&[s.colors.maps[3]]).unwrap());
    assert_eq!(blend, BlendOp::IndexTable(t.additive));
    // The plain call (mode 5) keeps the opaque remap.
    assert_eq!(
        hooks.glyph_look(4, 5).unwrap(),
        (
            ShadeChain::new(&[s.colors.maps[3]]).unwrap(),
            BlendOp::Opaque
        )
    );
}

#[test]
fn framed_text_boxes_draw_under_their_text() {
    // DrawFramedText: the rectangle (x', b − H)–(x' + W, b) with the
    // caller's colour and mode, then the text: list order is draw order.
    let mut s = setup(true);
    let a0 = s.pl2.alpha_blend[0].clone();
    let draws = [
        rect(100, 100, 140, 116, 0, 2),
        cel(&s, ICON, 110, 108, CelLook::PLAIN),
    ];
    let px = compose(&mut s, src(ICON, 30, 31), &draws, 60);
    assert_eq!(at(&px, 100, 100), a0[60][0]);
    assert_eq!(at(&px, 110, 108), 30);
    assert_eq!(at(&px, 139, 115), a0[60][0]);
    assert_eq!(at(&px, 140, 115), 60);
}

/// An item palette file of 21 maps; map `c` of file `t` sends `i` to
/// `i + 21·t + c`.
fn item_palette(t: u32) -> Vec<u8> {
    (0..21u32)
        .flat_map(|c| map(move |i| i + 21 * t + c))
        .collect()
}

#[test]
fn item_colour_remaps_read_the_item_palette_maps() {
    use crate::rules::shading::ITEM_PALETTE_FILES;
    let mut s = setup(true);
    let mut m = src(ICON, 30, 31);
    for (i, name) in ITEM_PALETTE_FILES.iter().enumerate() {
        m.insert(
            &crate::world_view::panel_art::item_palette_name(name),
            item_palette(i as u32 + 1),
        );
    }
    let look = |t, c| CelLook {
        mode: 5,
        remap: Remap::ItemColor { t, c },
    };
    let draws = [
        cel(&s, ICON, 10, 10, look(6, 20)), // invgrey, map 20
        cel(&s, ICON, 20, 10, look(3, 0)),  // gold: never selected
        cel(&s, ICON, 30, 10, look(1, 21)), // c ≥ 21: no map
    ];
    let px = compose(&mut s, m, &draws, 2);
    assert_eq!(at(&px, 10, 10), map(|i| i + 21 * 6 + 20)[30]);
    assert_eq!(at(&px, 20, 10), 30);
    assert_eq!(at(&px, 30, 10), 30);
    // A missing palette file is an error, never skipped.
    let mut s = setup(true);
    let loader = PanelArtLoader::new(Arc::new(src(ICON, 1, 2)), s.files.clone());
    assert!(loader
        .ensure(&[cel(&s, ICON, 0, 0, look(1, 0))], &mut s.assets)
        .is_err());
}

#[test]
fn the_play_frame_is_800_by_600_unless_chosen_once() {
    use crate::rules::camera::PlayFrameError;
    use crate::ui::layout::Screen;
    // Not set in this process: the 800 × 600 default everywhere.
    assert_eq!(FrameSize::play(), FrameSize::D2RUST);
    assert_eq!(Screen::play(), Screen::R800);
    assert_eq!(crate::world_view::play_view(), scene::Rect::FRAME);
    // Only the two resolution modes; refusing leaves the default.
    assert_eq!(
        FrameSize::set_play(FrameSize {
            width: 1024,
            height: 768
        }),
        Err(PlayFrameError::Unsupported(FrameSize {
            width: 1024,
            height: 768
        }))
    );
    assert_eq!(FrameSize::play(), FrameSize::D2RUST);
    // The 640 × 480 screen: shift (0, 0), panels at the 640 positions.
    let low = Screen::R640;
    assert_eq!((low.sx(), low.sy()), (0, 0));
    assert_eq!((Screen::R800.sx(), Screen::R800.sy()), (80, -60));
}

#[test]
fn a_640_frame_maps_the_window_with_its_own_size() {
    use crate::ui::frame::{FramePos, Presentation};
    // 640 × 480 window: scale 1, no bars.
    let p = Presentation::for_frame(640, 480, 640, 480).unwrap();
    assert_eq!((p.scale, p.left, p.top), (1, 0, 0));
    assert_eq!(p.to_frame(639, 479), FramePos::Inside(Point::new(639, 479)));
    assert_eq!(p.to_frame(640, 0), FramePos::Outside);
    // In an 800 × 600 window: centred, bars of 80 and 60.
    let p = Presentation::for_frame(800, 600, 640, 480).unwrap();
    assert_eq!((p.scale, p.left, p.top), (1, 80, 60));
    assert_eq!(p.to_frame(80, 60), FramePos::Inside(Point::new(0, 0)));
    assert_eq!(p.to_frame(79, 60), FramePos::Outside);
    assert_eq!(p.from_frame(0, 0), (80, 60));
    // 1280 × 960: scale 2.
    let p = Presentation::for_frame(1280, 960, 640, 480).unwrap();
    assert_eq!(p.scale, 2);
    // The 800 × 600 frame keeps refusing a 640 × 480 window.
    assert!(Presentation::new(640, 480).is_err());
}
