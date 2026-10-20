//! Unit tests of `rules::shading` and `rules::blend` on a synthetic PL2
//! (repo only). The act 1 values need the game's PL2: `tests/game_shading.rs` (ignored, local run queue).

use d2_formats::palette::{Palette, Pl2, Rgb};

use super::blend::{self, BlendTable, OverrideInput, UnitKind, WallDraw};
use super::shading::{self, BlockLight, ShadeTables, ShadingError};
use crate::scene::{
    self, gradient, BlendOp, DrawItem, FrameId, FrameImage, FrameView, GradientKind, MapId,
    MapTable, Rect, ShadeChain,
};

/// A map with every entry `f(i)`.
fn map(f: impl Fn(u32) -> u32) -> [u8; 256] {
    let mut m = [0u8; 256];
    for (i, v) in m.iter_mut().enumerate() {
        *v = (f(i as u32) % 256) as u8;
    }
    m
}

/// An asymmetric 256×256 table, `[destination][source]`.
fn table(k: u32) -> Vec<[u8; 256]> {
    (0..256u32)
        .map(|d| map(|s| d * (3 + k) + s * (5 + 2 * k) + k + 1))
        .collect()
}

/// A gray ramp palette: entry `i` = (i, i, i).
fn gray_palette() -> Palette {
    let mut colors = [Rgb::default(); 256];
    for (i, c) in colors.iter_mut().enumerate() {
        *c = Rgb {
            r: i as u8,
            g: i as u8,
            b: i as u8,
        };
    }
    Palette { colors }
}

/// A synthetic PL2: every map distinct and recognizable.
fn pl2() -> Pl2 {
    Pl2 {
        base_palette: gray_palette(),
        light_levels: (0..32).map(|k| map(move |i| i * (k + 1) / 32)).collect(),
        inventory_variations: (0..16).map(|k| map(move |i| i + 100 + k)).collect(),
        selected_unit_shift: map(|i| i + 7),
        alpha_blend: (0..3).map(table).collect(),
        additive_blend: table(3),
        multiplicative_blend: table(4),
        hue_variations: (0..111).map(|k| map(move |i| i + k + 1)).collect(),
        red_tones: map(|i| i ^ 0x11),
        green_tones: map(|i| i ^ 0x22),
        blue_tones: map(|i| i ^ 0x33),
        unknown_variations: (0..14).map(|k| map(move |i| i ^ (0x40 + k))).collect(),
        max_component_blend: table(5),
        darkened_shift: map(|i| i / 2),
        text_colors: Vec::new(),
        text_color_shifts: Vec::new(),
    }
}

fn tables() -> (MapTable, ShadeTables, Pl2) {
    let p = pl2();
    let mut maps = MapTable::new();
    let t = ShadeTables::push(&mut maps, &p);
    (maps, t, p)
}

/// Composes `items` over `base` (view sized), returns the framebuffer.
fn compose(
    maps: &MapTable,
    frames: &[FrameImage],
    items: &[DrawItem],
    base: &[u8],
    view: Rect,
) -> Vec<u8> {
    scene::compose_frame(items, frames, maps, view, base, scene::FramePlan::NONE).expect("composes")
}

#[test]
fn the_block_is_pushed_unchanged() {
    let (maps, t, p) = tables();
    for k in 0..32u8 {
        assert_eq!(
            maps.get(t.light_map(k)),
            Some(&p.light_levels[usize::from(k)])
        );
    }
    assert_eq!(maps.get(t.zero), Some(&[0u8; 256]));
    for (i, base) in t.alpha.iter().enumerate() {
        for d in [0u32, 1, 172, 255] {
            assert_eq!(
                maps.get(MapId(base.0 + d)),
                Some(&p.alpha_blend[i][d as usize]),
                "alpha {i} row {d} is the destination row, as stored"
            );
        }
    }
    assert_eq!(
        maps.get(MapId(t.additive.0 + 9)),
        Some(&p.additive_blend[9])
    );
    assert_eq!(
        maps.get(MapId(t.multiplicative.0 + 9)),
        Some(&p.multiplicative_blend[9])
    );
    assert_eq!(
        maps.get(MapId(t.max_component.0 + 9)),
        Some(&p.max_component_blend[9])
    );
    // The 128 remap maps in file order: hue 0…110, red, green, blue, unknown.
    assert_eq!(maps.get(t.remap0), Some(&p.hue_variations[0]));
    assert_eq!(maps.get(MapId(t.remap0.0 + 111)), Some(&p.red_tones));
    assert_eq!(maps.get(MapId(t.remap0.0 + 113)), Some(&p.blue_tones));
    assert_eq!(
        maps.get(MapId(t.remap0.0 + 127)),
        Some(&p.unknown_variations[13])
    );
}

#[test]
fn cel_light_is_v_shifted_by_3() {
    let (_, t, _) = tables();
    assert_eq!(shading::cel_light_level(0xFF), None);
    assert_eq!(shading::cel_light_level(0x7F), Some(15));
    assert_eq!(shading::cel_light_level(0), Some(0));
    assert_eq!(shading::cel_light_level(7), Some(0));
    assert_eq!(shading::cel_light_level(0xF8), Some(31));
    assert_eq!(shading::cel_light_level(0xFE), Some(31));
    assert_eq!(t.cel_light(0x7F), Some(MapId(t.light0.0 + 15)));
    // Mode 7: L = H whatever v is.
    for v in [0u8, 0x7F, 0xFF] {
        assert_eq!(blend::cel_tables(&t, 7, None, v).light, Some(t.highlight));
    }
}

#[test]
fn highlight_is_nearest_of_170_percent() {
    // Gray ramp: H[i] = nearest(min(255, ⌊170·i/100⌋)) = that gray.
    let p = gray_palette();
    let h = shading::highlight_map(&p);
    for i in 0..256u32 {
        assert_eq!(u32::from(h[i as usize]), (i * 170 / 100).min(255), "H[{i}]");
    }
    // Ties go to the lowest index, index 0 included.
    let mut colors = [Rgb { r: 9, g: 9, b: 9 }; 256];
    colors[0] = Rgb { r: 0, g: 0, b: 0 };
    let p = Palette { colors };
    assert_eq!(shading::nearest(&p, 9, 9, 9), 1);
    assert_eq!(shading::nearest(&p, 0, 0, 0), 0);
    assert_eq!(shading::nearest(&p, 4, 4, 4), 0, "16·3 vs 25·3: index 0");
    assert_eq!(shading::nearest(&p, 5, 5, 5), 1);
}

#[test]
fn red_map_is_nearest_red() {
    let mut colors = [Rgb::default(); 256];
    colors[1] = Rgb { r: 100, g: 0, b: 0 };
    colors[2] = Rgb { r: 250, g: 0, b: 0 };
    colors[3] = Rgb {
        r: 240,
        g: 200,
        b: 10,
    };
    let p = Palette { colors };
    let r = shading::red_map(&p);
    assert_eq!(r[0], 0);
    assert_eq!(r[1], 1);
    assert_eq!(r[2], 2);
    assert_eq!(r[3], 2, "(240, 0, 0) is nearest to (250, 0, 0)");
}

#[test]
fn hover_light_doubles_and_clamps() {
    assert_eq!(shading::hover_light(0), 0x40);
    assert_eq!(shading::hover_light(0x10), 0x40);
    assert_eq!(shading::hover_light(0x30), 0x60);
    assert_eq!(shading::hover_light(0x80), 0xFF);
    assert_eq!(shading::hover_light(0xFF), 0xFF);
}

#[test]
fn gradient_table_vectors() {
    assert_eq!(gradient(31, 0, 1), 30);
    assert_eq!(gradient(31, 0, 31), 0);
    assert_eq!(gradient(0, 31, 31), 30);
    assert_eq!(gradient(10, 20, 16), 15);
    assert_eq!(gradient(20, 10, 1), 19);
    for a in 0..32 {
        for b in 0..32 {
            assert_eq!(u32::from(gradient(a, b, 0)), a);
            for x in 0..32 {
                assert!(gradient(a, b, x) < 32);
            }
        }
    }
}

#[test]
fn wall_block_light_vectors() {
    use shading::wall_block_light as w;
    assert_eq!(w([200; 4], false), BlockLight::Flat(25));
    assert_eq!(
        w([0xFF, 0xFF, 0xFF, 0xF8], false),
        BlockLight::Unlit,
        "Δ = 7"
    );
    assert_eq!(
        w([0, 255, 255, 0], false),
        BlockLight::Gradient([0, 255, 255, 0])
    );
    // Δ = 9 is flat, 10 a gradient; low quality is always flat.
    assert_eq!(w([100, 109, 109, 100], false), BlockLight::Flat(12));
    assert_eq!(
        w([100, 110, 110, 100], false),
        BlockLight::Gradient([100, 110, 110, 100])
    );
    assert_eq!(w([100, 110, 110, 100], true), BlockLight::Flat(12));
    assert_eq!(w([0xF8, 0xFF, 0xFF, 0xF8], true), BlockLight::Unlit);
}

#[test]
fn wall_gradient_rows_and_columns() {
    let (_, t, _) = tables();
    // c0 = 0, c1 = 255, c2 = 255, c3 = 0, row 0: a = 0, b = 31, column x
    // uses map ⌊31x/32⌋.
    let g = t.gradient(GradientKind::Wall, 0, 0, [0, 255, 255, 0]);
    for x in 0..32 {
        assert_eq!(g.level(x, 0), 31 * x / 32, "column {x}");
    }
    // Row interpolation, arithmetic shift: c0 = 255 → c3 = 0 on the left.
    let g = t.gradient(GradientKind::Wall, 0, 0, [255, 255, 255, 0]);
    assert_eq!(g.level(0, 0), (32 * 255) >> 8);
    assert_eq!(g.level(0, 31), (32 * 255 - 31 * 255) >> 8);
    assert_eq!(g.level(0, 16), (32 * 255 - 16 * 255) >> 8);
}

#[test]
fn floor_block_light_vectors() {
    // Every cell 0x80: Δ = 0, flat map 16.
    let cells = vec![0x80u8; 8 * 6];
    assert_eq!(
        shading::floor_block_light(&cells, 0, 0, false),
        Ok(BlockLight::Flat(16))
    );
    // A gradient grid: cell n = 4n; base g = 1 + 8 = 9.
    let cells: Vec<u8> = (0..64u32).map(|n| (4 * n) as u8).collect();
    let e = |n: u32| 4 * (9 + n);
    // Δ = |e10 − e9| + |e17 − e9| + |e18 − e10| = 4 + 32 + 32.
    let c = |a, b, c, d| ((e(a) + e(b) + e(c) + e(d)) >> 2) as u8;
    assert_eq!(
        shading::floor_block_light(&cells, 1, 1, false),
        Ok(BlockLight::Gradient([
            c(8, 9, 16, 17),
            c(1, 2, 9, 10),
            c(10, 11, 17, 19),
            c(17, 18, 25, 26),
        ]))
    );
    // c2 takes e[g+17], not e[g+18].
    let Ok(BlockLight::Gradient(corners)) = shading::floor_block_light(&cells, 1, 1, false) else {
        unreachable!()
    };
    assert_ne!(corners[2], c(10, 11, 18, 19));
    // Low quality: flat with v = e[g+9], no unlit copy even at 0xFF.
    assert_eq!(
        shading::floor_block_light(&cells, 1, 1, true),
        Ok(BlockLight::Flat((e(9) >> 3) as u8))
    );
    let bright = vec![0xFFu8; 64];
    assert_eq!(
        shading::floor_block_light(&bright, 0, 0, false),
        Ok(BlockLight::Flat(31))
    );
    // A grid too small for the block is an error, not a default.
    assert!(matches!(
        shading::floor_block_light(&cells, 7, 5, false),
        Err(ShadingError::FloorGrid { .. })
    ));
    // RLE rows: a_r = (16·c0 + r·(c3 − c0)) >> 7.
    let (_, t, _) = tables();
    let g = t.gradient(GradientKind::RleFloor, 0, 0, [255, 0, 0, 0]);
    assert_eq!(g.level(0, 0), (16 * 255) >> 7);
    assert_eq!(g.level(0, 14), (16 * 255 - 14 * 255) >> 7);
    assert_eq!(GradientKind::RleFloor.rows(), 15);
    // Isometric floors with a gradient: the same 15-row gradient as RLE blocks.
    assert_eq!(
        shading::floor_block_chain(&t, BlockLight::Gradient([0, 99, 0, 0]), 5, 6),
        ShadeChain::EMPTY.with_gradient(t.gradient(GradientKind::RleFloor, 5, 6, [0, 99, 0, 0]))
    );
    assert_eq!(
        shading::floor_block_chain(&t, BlockLight::Flat(3), 0, 0),
        ShadeChain::new(&[t.light_map(3)]).unwrap()
    );
}

#[test]
fn unit_palette_index_selects_map_p_minus_1() {
    let (maps, t, p) = tables();
    assert_eq!(t.unit_remap(0), Ok(None));
    let m = |p| maps.get(t.unit_remap(p).unwrap().unwrap()).copied();
    assert_eq!(m(2), Some(p.hue_variations[1]));
    assert_eq!(m(112), Some(p.red_tones));
    assert_eq!(m(114), Some(p.blue_tones));
    assert_eq!(m(128), Some(p.unknown_variations[13]));
    assert_eq!(t.unit_remap(129), Err(ShadingError::RemapIndex(129)));
}

#[test]
fn item_color_selection() {
    for c in 0..21 {
        assert_eq!(shading::item_color(3, c), None, "gold is never selected");
        assert_eq!(shading::item_color(4, c), None, "brown is never selected");
        assert_eq!(shading::item_color(0, c), None);
        assert_eq!(shading::item_color(9, c), None);
        for t in [1, 2, 5, 6, 7, 8] {
            assert_eq!(shading::item_color(t, c), Some((t, c)));
        }
    }
    assert_eq!(shading::item_color(1, 21), None);
    assert_eq!(shading::ITEM_PALETTE_FILES[2], "gold");
    assert_eq!(shading::ITEM_PALETTE_FILES[7], "invgreybrown");
}

#[test]
fn a_mapped_zero_is_drawn_as_index_0() {
    let mut maps = MapTable::new();
    let p = maps.push(map(|i| if i == 5 { 0 } else { i }));
    let frames = [FrameImage {
        width: 2,
        height: 1,
        pixels: vec![5, 6],
    }];
    let mut it = DrawItem::new(FrameId(0), 0, 0);
    it.shade = ShadeChain::new(&[p]).unwrap();
    let out = compose(&maps, &frames, &[it], &[9, 9], Rect::new(0, 0, 2, 1));
    assert_eq!(out, vec![0, 6]);
}

#[test]
fn draw_mode_tables() {
    assert_eq!(blend::mode_table(0), Some(BlendTable::A2));
    assert_eq!(blend::mode_table(1), Some(BlendTable::A1));
    assert_eq!(blend::mode_table(2), Some(BlendTable::A0));
    assert_eq!(blend::mode_table(3), Some(BlendTable::Add));
    assert_eq!(blend::mode_table(4), Some(BlendTable::Mul));
    assert_eq!(blend::mode_table(5), None);
    assert_eq!(blend::mode_table(6), Some(BlendTable::Max));
    assert_eq!(blend::mode_table(7), None);
    assert_eq!(blend::mode_table(8), None);
    assert_eq!(blend::mode_table(255), None);
}

#[test]
fn cel_modes_write_row_destination() {
    let (maps, t, p) = tables();
    let frames = [FrameImage {
        width: 1,
        height: 1,
        pixels: vec![255],
    }];
    let d = 172u8;
    let at = |mode: u8, remap: Option<MapId>, v: u8| {
        let (shade, blend) = blend::cel_ops(&t, mode, remap, v);
        let mut it = DrawItem::new(FrameId(0), 0, 0);
        it.shade = shade;
        it.blend = blend;
        compose(&maps, &frames, &[it], &[d], Rect::new(0, 0, 1, 1))[0]
    };
    // Unlit, no remap: d' = T[256·d + s].
    assert_eq!(at(2, None, 0xFF), p.alpha_blend[0][172][255]);
    assert_eq!(at(1, None, 0xFF), p.alpha_blend[1][172][255]);
    assert_eq!(at(0, None, 0xFF), p.alpha_blend[2][172][255]);
    assert_eq!(at(3, None, 0xFF), p.additive_blend[172][255]);
    assert_eq!(at(4, None, 0xFF), p.multiplicative_blend[172][255]);
    assert_eq!(at(6, None, 0xFF), p.max_component_blend[172][255]);
    assert_ne!(
        at(2, None, 0xFF),
        p.alpha_blend[0][255][172],
        "not transposed"
    );
    // Opaque modes: d' = s', mode 8 (data) as 5.
    assert_eq!(at(5, None, 0xFF), 255);
    assert_eq!(at(8, None, 0xFF), 255);
    // Remap, unlit: T[256·d + P[s]].
    let hue = t.unit_remap(2).unwrap();
    let ps = p.hue_variations[1][255];
    assert_eq!(at(2, hue, 0xFF), p.alpha_blend[0][172][usize::from(ps)]);
    // Remap and light with T: P dropped, T[256·d + L[s]].
    let ls = p.light_levels[15][255];
    assert_eq!(at(2, hue, 0x7F), p.alpha_blend[0][172][usize::from(ls)]);
    // No T: L[P[s]].
    assert_eq!(at(5, hue, 0x7F), p.light_levels[15][usize::from(ps)]);
    // Mode 7: H[P[s]], light byte ignored.
    let h = shading::highlight_map(&p.base_palette);
    assert_eq!(at(7, hue, 0x10), h[usize::from(ps)]);
    assert_eq!(at(7, None, 0xFF), h[255]);
}

fn ov(kind: UnitKind) -> OverrideInput {
    OverrideInput {
        kind,
        fade: 0,
        monster_has_ethereal: false,
        item_trans: None,
        component: 0,
        item_ethereal: false,
    }
}

#[test]
fn component_mode_decision() {
    use blend::{component_mode as mode, unit_override as r};
    // Layer override level 3, not ghostly, no r, hovered → 3.
    assert_eq!(mode(false, None, true, Some(3)), 3);
    // No override, ghostly, hovered → 1.
    assert_eq!(mode(true, None, true, None), 1);
    assert_eq!(mode(true, Some(4), false, Some(3)), 1);
    // r wins over the layer and the highlight.
    assert_eq!(mode(false, Some(2), true, Some(3)), 2);
    assert_eq!(mode(false, None, true, None), 7);
    assert_eq!(mode(false, None, false, None), 5);
    assert_eq!(mode(false, None, false, Some(0)), 0);

    // Player: fade, then item transparency, then ethereal RH / LH / SH.
    let mut i = ov(UnitKind::Player);
    assert_eq!(r(&i), None);
    i.fade = -1;
    assert_eq!(r(&i), Some(1));
    i.fade = 0;
    i.item_ethereal = true;
    i.component = 5;
    assert_eq!(r(&i), Some(1));
    i.component = 6;
    assert_eq!(r(&i), Some(1));
    i.component = 7;
    assert_eq!(r(&i), Some(2));
    i.component = 0;
    assert_eq!(r(&i), None);
    i.item_trans = Some((1, 3));
    assert_eq!(r(&i), Some(3));
    i.item_trans = Some((0, 3));
    assert_eq!(r(&i), None);

    // Monster: fade 1…15 → 1; > 15 → 1 with an ethereal item, else no
    // override at all (the item's transparent is ignored).
    let mut m = ov(UnitKind::Monster);
    m.item_trans = Some((1, 3));
    assert_eq!(r(&m), Some(3));
    m.fade = 15;
    assert_eq!(r(&m), Some(1));
    m.fade = 16;
    assert_eq!(r(&m), None);
    m.monster_has_ethereal = true;
    assert_eq!(r(&m), Some(1));
    // Ethereal component items are a player-only rule.
    let mut m = ov(UnitKind::Monster);
    m.item_ethereal = true;
    m.component = 7;
    assert_eq!(r(&m), None);
    // Other units: only the item rule.
    let mut o = ov(UnitKind::Other);
    o.fade = 5;
    assert_eq!(r(&o), None);
    o.item_trans = Some((1, 4));
    assert_eq!(r(&o), Some(4));

    // Hover: objects with SubClass bit 0x80 are never highlighted.
    assert!(blend::hover_highlighted(true, None));
    assert!(blend::hover_highlighted(true, Some(0x7F)));
    assert!(!blend::hover_highlighted(true, Some(0x80)));
    assert!(!blend::hover_highlighted(false, None));
}

#[test]
fn single_cel_modes() {
    assert_eq!(blend::missile_mode(1, false), 3);
    assert_eq!(blend::missile_mode(2, false), 4);
    assert_eq!(blend::missile_mode(0, false), 5);
    assert_eq!(blend::missile_mode(3, false), 5);
    assert_eq!(blend::missile_mode(2, true), 7);
    assert_eq!(blend::item_mode(false), 5);
    assert_eq!(blend::item_mode(true), 7);
    assert_eq!(blend::overlay_mode(3), 3);
    assert_eq!(blend::overlay_mode(8), 8);
}

#[test]
fn unit_shadow_pixels() {
    let (maps, t, p) = tables();
    let frames = [FrameImage {
        width: 3,
        height: 1,
        pixels: vec![0, 200, 9],
    }];
    let base = [31u8, 31, 40];
    let draw = |blended| {
        let (shade, blend) = blend::unit_shadow_ops(&t, blended);
        let mut it = DrawItem::new(FrameId(0), 0, 0);
        it.shade = shade;
        it.blend = blend;
        compose(&maps, &frames, &[it], &base, Rect::new(0, 0, 3, 1))
    };
    // Blended: d' = A0[256·d + 0] for every opaque source pixel.
    assert_eq!(
        draw(true),
        vec![31, p.alpha_blend[0][31][0], p.alpha_blend[0][40][0]]
    );
    // Not blended: 0.
    assert_eq!(draw(false), vec![31, 0, 0]);
}

#[test]
fn unit_shadow_shape() {
    // h = 10, yoff = −7 at (100, 50), xoff 4: y0 = 47, x0 = 100 + 4 − 3,
    // 5 rows on screen rows 47…43 from source rows 0, 2, 4, 6, 8 (from
    // the bottom), row k starting at x0 − k.
    let (w, h) = (3u32, 10u32);
    let pixels: Vec<u8> = (0..w * h).map(|i| (i + 1) as u8).collect();
    let cel = FrameView::new(w, h, &pixels).unwrap();
    let s = blend::shadow_image(&cel, 100, 50, 4, -7);
    let (x0, y0) = (101i32, 47i32);
    assert_eq!((s.image.width, s.image.height), (w + 4, 5));
    assert_eq!((s.x, s.y), (x0 - 4, y0 - 4));
    for k in 0..5i32 {
        let row = (s.image.height as i32 - 1 - k) as u32; // screen y0 − k
        assert_eq!(s.y + row as i32, y0 - k);
        let src_row = h - 1 - 2 * k as u32;
        for c in 0..s.image.width as i32 {
            let sx = s.x + c;
            let got = s.image.pixels[(row * s.image.width) as usize + c as usize];
            let col = sx - (x0 - k);
            let want = if (0..w as i32).contains(&col) {
                pixels[(src_row * w + col as u32) as usize]
            } else {
                0
            };
            assert_eq!(got, want, "row k {k}, column {sx}");
        }
    }
    // Odd heights draw ⌊h / 2⌋ rows; yoff truncates toward zero.
    let one = vec![1u8; 3];
    let s = blend::shadow_image(&FrameView::new(1, 3, &one).unwrap(), 0, 0, 0, 7);
    assert_eq!((s.image.height, s.x, s.y), (1, 3, 3));
    let s = blend::shadow_image(&FrameView::new(1, 1, &one[..1]).unwrap(), 0, 0, 0, 0);
    assert_eq!((s.image.width, s.image.height), (0, 0));
}

#[test]
fn shadow_cel_pre_test() {
    // gen-render-kurast-rain tick 58: the torch cel (h 34, xoff −7) passed
    // at X 846 (left 805) is right of an 800-wide view: not drawn (w and
    // yoff here are illustrative; the left test alone decides).
    assert!(!blend::shadow_cel_drawn(846, 236, 20, 34, -7, -1, 800, 600));
    assert!(blend::shadow_cel_drawn(840, 236, 20, 34, -7, -1, 800, 600));
    // left + w + h < 0: wholly left of the view.
    assert!(!blend::shadow_cel_drawn(0, 100, 4, 10, -5, 0, 800, 600));
    assert!(blend::shadow_cel_drawn(0, 100, 4, 10, -4, 0, 800, 600));
    // Y + yoff above the top; Y + yoff − ⌊h / 2⌋ at or below the bottom.
    assert!(!blend::shadow_cel_drawn(100, 5, 4, 10, 0, -6, 800, 600));
    assert!(blend::shadow_cel_drawn(100, 6, 4, 10, 0, -6, 800, 600));
    assert!(!blend::shadow_cel_drawn(100, 605, 4, 11, 0, 0, 800, 600));
    assert!(blend::shadow_cel_drawn(100, 604, 4, 11, 0, 0, 800, 600));
}

#[test]
fn shadow_tiles() {
    let (maps, t, p) = tables();
    let frames = [FrameImage {
        width: 1,
        height: 1,
        pixels: vec![200],
    }];
    let draw = |blended| {
        let (shade, blend) = blend::shadow_tile_ops(&t, blended);
        let mut it = DrawItem::new(FrameId(0), 0, 0);
        it.shade = shade;
        it.blend = blend;
        compose(&maps, &frames, &[it], &[100], Rect::new(0, 0, 1, 1))[0]
    };
    assert_eq!(draw(true), p.alpha_blend[0][100][200]);
    assert_eq!(draw(false), 200);
}

#[test]
fn translucent_walls_read_the_transpose() {
    assert_eq!(blend::wall_draw(0xFF), WallDraw::Lit);
    assert_eq!(
        blend::wall_draw(0xFE),
        WallDraw::Translucent(BlendTable::A0)
    );
    assert_eq!(
        blend::wall_draw(0xC0),
        WallDraw::Translucent(BlendTable::A0)
    );
    assert_eq!(
        blend::wall_draw(0xBF),
        WallDraw::Translucent(BlendTable::A1)
    );
    assert_eq!(
        blend::wall_draw(0x80),
        WallDraw::Translucent(BlendTable::A1)
    );
    assert_eq!(
        blend::wall_draw(0x7F),
        WallDraw::Translucent(BlendTable::A2)
    );
    assert_eq!(
        blend::wall_draw(0x40),
        WallDraw::Translucent(BlendTable::A2)
    );
    assert_eq!(blend::wall_draw(0x3F), WallDraw::Hidden);
    let (maps, t, p) = tables();
    assert_eq!(blend::wall_block_ops(&t, 0x3F, [0; 4], false, 0, 0), None);

    // a = 0xC0, flat-looking corners: still the gradient light (no flat
    // branch), d' = A0[256·L[s] + d].
    let corners = [200u8; 4];
    let (shade, op) = blend::wall_block_ops(&t, 0xC0, corners, false, 0, 0).unwrap();
    assert_eq!(op, BlendOp::IndexTableSrcRow(t.alpha[0]));
    assert!(shade.gradient().is_some());
    let frames = [FrameImage {
        width: 32,
        height: 32,
        pixels: vec![255; 32 * 32],
    }];
    let mut it = DrawItem::new(FrameId(0), 0, 0);
    it.shade = shade;
    it.blend = op;
    let base = vec![172u8; 32 * 32];
    let out = compose(&maps, &frames, &[it], &base, Rect::new(0, 0, 32, 32));
    let l = p.light_levels[25][255];
    assert!(out
        .iter()
        .all(|&v| v == p.alpha_blend[0][usize::from(l)][172]));
    assert_ne!(
        p.alpha_blend[0][usize::from(l)][172],
        p.alpha_blend[0][172][usize::from(l)]
    );

    // Lit walls: flat, unlit or gradient, opaque.
    let lit = |c| blend::wall_block_ops(&t, 0xFF, c, false, 0, 0).unwrap();
    assert_eq!(
        lit([200; 4]),
        (
            ShadeChain::new(&[t.light_map(25)]).unwrap(),
            BlendOp::Opaque
        )
    );
    assert_eq!(
        lit([0xFF, 0xFF, 0xFF, 0xF8]),
        (ShadeChain::EMPTY, BlendOp::Opaque)
    );
    assert!(lit([0, 255, 255, 0]).0.gradient().is_some());
}

#[test]
fn gradient_light_per_pixel_in_the_compositor() {
    let (maps, t, p) = tables();
    // A 40 × 40 tile image whose block at (4, 6) is drawn through a clip,
    // with the view origin off zero.
    let pixels: Vec<u8> = (0..1600u32).map(|i| (i % 251 + 1) as u8).collect();
    let frames = [FrameImage {
        width: 40,
        height: 40,
        pixels: pixels.clone(),
    }];
    let (tx, ty) = (-3, 2);
    let (bx, by) = (tx + 4, ty + 6);
    let corners = [0u8, 255, 120, 40];
    let mut it = DrawItem::new(FrameId(0), tx, ty);
    it.clip = Rect::new(bx, by, 32, 32);
    it.shade = t.block_chain(BlockLight::Gradient(corners), GradientKind::Wall, bx, by);
    let view = Rect::new(-5, 0, 50, 50);
    let out = compose(&maps, &frames, &[it], &vec![0; 2500], view);
    let g = t.gradient(GradientKind::Wall, bx, by, corners);
    for r in 0..32 {
        for x in 0..32 {
            let s = pixels[((by - ty + r) * 40 + (bx - tx + x)) as usize];
            let k = g.level(x as u32, r as u32) as usize;
            let at = ((by + r - view.y) * 50 + (bx + x - view.x)) as usize;
            assert_eq!(out[at], p.light_levels[k][usize::from(s)], "({x}, {r})");
        }
    }
}

#[test]
fn a_gradient_item_must_stay_inside_its_block() {
    let (maps, t, _) = tables();
    let frames = [FrameImage {
        width: 40,
        height: 40,
        pixels: vec![1; 1600],
    }];
    let mut it = DrawItem::new(FrameId(0), 0, 0);
    it.shade = t.block_chain(
        BlockLight::Gradient([0, 99, 0, 0]),
        GradientKind::Wall,
        0,
        0,
    );
    let err = scene::compose(&[it], &frames[..], &maps, Rect::new(0, 0, 64, 64)).unwrap_err();
    assert!(
        matches!(err, scene::SceneError::Item { ref error, .. } if matches!(**error, scene::SceneError::GradientArea { .. })),
        "{err:?}"
    );
    // Light maps past the table: an error.
    let mut small = MapTable::new();
    small.push([0; 256]);
    it.clip = Rect::new(0, 0, 32, 32);
    let err = scene::compose(&[it], &frames[..], &small, Rect::new(0, 0, 64, 64)).unwrap_err();
    assert!(
        matches!(err, scene::SceneError::Item { ref error, .. } if matches!(**error, scene::SceneError::LightMaps(_))),
        "{err:?}"
    );
    // Inside the block: fine.
    assert!(scene::compose(&[it], &frames[..], &maps, Rect::new(0, 0, 64, 64)).is_ok());
}
