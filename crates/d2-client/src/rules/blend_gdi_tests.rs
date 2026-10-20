//! Unit tests of the unit shadow position and skips, the GDI line and
//! rectangle (`rules::blend`) and the isometric floor gradient
//! (`rules::shading`), on synthetic tables (repo only).

use d2_formats::palette::{Palette, Pl2, Rgb};

use super::blend::{self, BlendError, ObjectShadow, ShadowMotion};
use super::camera::{Camera, ClientPos, FrameSize, OpenMode};
use super::shading::{self, BlockLight, ShadeTables};
use crate::scene::{self, FrameId, FrameImage, GradientKind, MapTable, Rect};

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

fn tables() -> (MapTable, ShadeTables, Pl2) {
    let p = pl2();
    let mut maps = MapTable::new();
    let t = ShadeTables::push(&mut maps, &p);
    (maps, t, p)
}

const SURFACE: FrameSize = FrameSize {
    width: 10,
    height: 8,
};

/// A 10 × 8 base whose pixel `i` holds `i * 3 + 1`.
fn base() -> Vec<u8> {
    (0..80u32).map(|i| (i * 3 + 1) as u8).collect()
}

/// Composes one GDI draw over `base` on the 10 × 8 surface.
fn draw(maps: &MapTable, d: &blend::GdiDraw, base: &[u8]) -> Vec<u8> {
    let frames = vec![d.image.clone()];
    let view = SURFACE.rect();
    scene::compose_frame(
        &[d.item(FrameId(0))],
        &frames,
        maps,
        view,
        base,
        scene::FramePlan::NONE,
    )
    .expect("composes")
}

fn camera(mode: u8) -> Camera {
    Camera::new(
        FrameSize::D2RUST,
        OpenMode::new(mode).unwrap(),
        ClientPos { x: 1000, y: 2000 },
        (0, 0),
    )
}

const AT: ClientPos = ClientPos { x: 1000, y: 2000 };

#[test]
fn composite_shadow_position_halves_oz_and_shifts_two_left() {
    // cx_u = 600, cy_u = 1716 (the camera vector), shiftX 0.
    let c = camera(0);
    let m = ShadowMotion {
        ox: 3,
        oy: 4,
        oz: -7,
        ..Default::default()
    };
    // h = −7 / 2 = −3 (C division, toward zero).
    // X = 1000 − 3 + 3 − 600 − 2, Y = 2000 − 3 + 4 − 1708.
    assert_eq!(
        blend::composite_shadow_position(&c, false, AT, &m, None),
        Ok((398, 293))
    );
    // The unit draw of the same unit takes oz in y only.
    assert_eq!(c.unit_draw(AT, (m.ox, m.oy + m.oz)), (403, 289));
    // Open mode 1: shiftX = −200.
    assert_eq!(
        blend::composite_shadow_position(&camera(1), false, AT, &m, None),
        Ok((198, 293))
    );
    // Objects add Xoffset / Yoffset with no Draw test.
    let o = ObjectShadow {
        xoffset: -5,
        yoffset: 9,
        draw: false,
        blocks_light: false,
    };
    assert_eq!(
        blend::composite_shadow_position(&c, false, AT, &m, Some(&o)),
        Ok((393, 302))
    );
    assert_eq!(
        blend::unit_shadow_position(&c, false, 2, 0, false, AT, &m, Some(&o)),
        Ok(Some((393, 302)))
    );
    // Perspective is not GDI: unspecified, an error.
    assert_eq!(
        blend::composite_shadow_position(&c, true, AT, &m, None),
        Err(BlendError::Perspective)
    );
}

#[test]
fn shadow_skips() {
    let c = camera(0);
    let m = ShadowMotion::default();
    assert!(blend::unit_shadow_skipped(0x20, false));
    assert!(blend::unit_shadow_skipped(0, true));
    assert!(!blend::unit_shadow_skipped(!0x20, false));
    for unit_type in [0, 1, 2, 3, 4] {
        assert_eq!(
            blend::unit_shadow_position(&c, false, unit_type, 0x20, false, AT, &m, None),
            Ok(None)
        );
        assert_eq!(
            blend::unit_shadow_position(&c, false, unit_type, 0, true, AT, &m, None),
            Ok(None)
        );
        assert!(
            blend::unit_shadow_position(&c, false, unit_type, 0x10, false, AT, &m, None)
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn single_cel_shadow_position_uses_raw_motion() {
    let c = camera(0);
    // mx >> 11 with an arithmetic shift: −2049 → −2; 4096 → 2. oz and the
    // ox / oy offsets are not read.
    let m = ShadowMotion {
        ox: 50,
        oy: 50,
        oz: -50,
        mx: -2049,
        my: 4096,
    };
    assert_eq!(
        blend::single_cel_shadow_position(&c, false, AT, &m, None),
        Ok(Some((398, 294)))
    );
    assert_eq!(
        blend::unit_shadow_position(&c, false, 3, 0, false, AT, &m, None),
        Ok(Some((398, 294)))
    );
    // Without a record mx = my = 0: no −2.
    assert_eq!(
        blend::single_cel_shadow_position(&camera(2), false, AT, &ShadowMotion::default(), None),
        Ok(Some((1000 - (600 - 200), 2000 - 1708)))
    );
    // Objects need Draw and BlocksLight, then add their offsets.
    let o = |draw, blocks_light| ObjectShadow {
        xoffset: 7,
        yoffset: -3,
        draw,
        blocks_light,
    };
    for (d, b) in [(false, true), (true, false), (false, false)] {
        assert_eq!(
            blend::single_cel_shadow_position(&c, false, AT, &m, Some(&o(d, b))),
            Ok(None)
        );
    }
    assert_eq!(
        blend::single_cel_shadow_position(&c, false, AT, &m, Some(&o(true, true))),
        Ok(Some((405, 291)))
    );
    assert_eq!(
        blend::single_cel_shadow_position(&c, true, AT, &m, None),
        Err(BlendError::Perspective)
    );
}

/// (x, y) → its image under one of the eight octant symmetries.
fn octant(p: (i32, i32), swap: bool, sx: i32, sy: i32) -> (i32, i32) {
    let (x, y) = if swap { (p.1, p.0) } else { p };
    (x * sx, y * sy)
}

#[test]
fn line_steps_in_all_octants() {
    // (0, 0) → (5, 2): error 2, 4, 6 > 5 (y + 1, error 1), 3, 5 (not > 5).
    let base = [(0, 0), (1, 0), (2, 0), (3, 1), (4, 1), (5, 1)];
    for swap in [false, true] {
        for sx in [1, -1] {
            for sy in [1, -1] {
                let end = octant((5, 2), swap, sx, sy);
                let want: Vec<_> = base.iter().map(|&p| octant(p, swap, sx, sy)).collect();
                assert_eq!(
                    blend::gdi_line_pixels(0, 0, end.0, end.1),
                    Ok(want),
                    "{end:?}"
                );
            }
        }
    }
    // Translated: the first pixel is (x0, y0).
    assert_eq!(
        blend::gdi_line_pixels(3, 4, 7, 4),
        Ok(vec![(3, 4), (4, 4), (5, 4), (6, 4), (7, 4)])
    );
    assert_eq!(
        blend::gdi_line_pixels(3, 4, 3, 1),
        Ok(vec![(3, 4), (3, 3), (3, 2), (3, 1)])
    );
    // The error must exceed the major distance: 1 per step never exceeds 4.
    assert_eq!(
        blend::gdi_line_pixels(0, 0, 4, 1),
        Ok(vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)])
    );
    // Zero length: one pixel.
    assert_eq!(blend::gdi_line_pixels(5, 5, 5, 5), Ok(vec![(5, 5)]));
    // |Δx| = |Δy| > 0 is x-major: one straight step first, ending one
    // short on the minor axis.
    assert_eq!(
        blend::gdi_line_pixels(0, 0, 3, 3),
        Ok(vec![(0, 0), (1, 0), (2, 1), (3, 2)])
    );
    assert_eq!(
        blend::gdi_line_pixels(0, 0, -3, 3),
        Ok(vec![(0, 0), (-1, 0), (-2, 1), (-3, 2)])
    );
    assert_eq!(
        blend::gdi_line_pixels(10, 10, 8, 15),
        Ok(vec![
            (10, 10),
            (10, 11),
            (10, 12),
            (9, 13),
            (9, 14),
            (9, 15)
        ])
    );
    assert_eq!(
        blend::gdi_line_pixels(4, 4, 4, 0),
        Ok(vec![(4, 4), (4, 3), (4, 2), (4, 1), (4, 0)])
    );
}

#[test]
fn line_is_opaque_and_clipped_per_pixel() {
    let (mut maps, _, _) = tables();
    let red = maps.push(blend::color_row(77));
    let b = base();
    // From (−2, 3) to (12, 3): only x 0…9 of row 3 are on the surface.
    let d = blend::gdi_line(SURFACE, red, -2, 3, 12, 3)
        .unwrap()
        .unwrap();
    let out = draw(&maps, &d, &b);
    for (i, (&o, &was)) in out.iter().zip(&b).enumerate() {
        let want = if i / 10 == 3 { 77 } else { was };
        assert_eq!(o, want, "pixel {i}");
    }
    // A steep line leaving the surface at the bottom (rows ≥ 8 skipped).
    let d = blend::gdi_line(SURFACE, red, 9, 5, 7, 11).unwrap().unwrap();
    let out = draw(&maps, &d, &b);
    let px = blend::gdi_line_pixels(9, 5, 7, 11).unwrap();
    let on: Vec<_> = px.iter().filter(|p| p.1 < 8).collect();
    assert_eq!(on.len(), 3);
    for y in 0..8 {
        for x in 0..10 {
            let i = (y * 10 + x) as usize;
            let want = if on.contains(&&(x, y)) { 77 } else { b[i] };
            assert_eq!(out[i], want, "({x}, {y})");
        }
    }
    // Color 0 is written (opaque index 0), not transparent.
    let black = maps.push(blend::color_row(0));
    let d = blend::gdi_line(SURFACE, black, 4, 4, 4, 4)
        .unwrap()
        .unwrap();
    let out = draw(&maps, &d, &b);
    assert_eq!(out[44], 0);
    assert_eq!(out.iter().filter(|&&v| v == 0).count(), 1);
    // Entirely off the surface: nothing.
    assert_eq!(blend::gdi_line(SURFACE, red, -5, -1, -1, -3), Ok(None));
    assert_eq!(blend::gdi_line(SURFACE, red, 10, 0, 20, 7), Ok(None));
}

#[test]
fn rectangle_clamps_and_empty_cases() {
    let (mut maps, t, _) = tables();
    let c = maps.push(blend::color_row(77));
    let b = base();
    // Clamped to x 0…9, y 0…7: columns 0…8, rows 0…6 (the last column and
    // row of the surface are never reached).
    let d = blend::gdi_rectangle(&t, SURFACE, c, -5, -1, 20, 30, 5)
        .unwrap()
        .unwrap();
    let out = draw(&maps, &d, &b);
    for y in 0..8 {
        for x in 0..10 {
            let i = y * 10 + x;
            let want = if x < 9 && y < 7 { 77 } else { b[i] };
            assert_eq!(out[i], want, "({x}, {y})");
        }
    }
    // Columns x0 … x1 − 1, rows y0 … y1 − 1.
    let d = blend::gdi_rectangle(&t, SURFACE, c, 2, 3, 5, 4, 5)
        .unwrap()
        .unwrap();
    assert_eq!((d.x, d.y, d.image.width, d.image.height), (2, 3, 3, 1));
    // Empty: x0 = x1 or y0 = y1, also after clamping.
    for (x0, y0, x1, y1) in [(3, 2, 3, 6), (1, 4, 8, 4), (12, 0, 20, 5), (0, -4, 5, 0)] {
        assert_eq!(
            blend::gdi_rectangle(&t, SURFACE, c, x0, y0, x1, y1, 5),
            Ok(None),
            "{x0} {y0} {x1} {y1}"
        );
    }
    // y1 < y0: fatal 0x32.
    assert_eq!(
        blend::gdi_rectangle(&t, SURFACE, c, 1, 6, 4, 2, 5),
        Err(BlendError::RectangleRowsReversed { y0: 6, y1: 2 })
    );
    // x1 < x0: not specified.
    assert_eq!(
        blend::gdi_rectangle(&t, SURFACE, c, 6, 1, 2, 4, 5),
        Err(BlendError::RectangleColumnsReversed { x0: 6, x1: 2 })
    );
}

#[test]
fn rectangle_writes_by_mode_value() {
    let (mut maps, t, p) = tables();
    assert_eq!(
        (0..=9).map(blend::gdi_mode_value).collect::<Vec<_>>(),
        [2, 2, 2, 1, 1, 0, 1, 0, 0, 0]
    );
    let b = base();
    let color = 200u8;
    let cm = maps.push(blend::color_row(color));
    let rect = |maps: &MapTable, mode: u8| {
        let d = blend::gdi_rectangle(&t, SURFACE, cm, 1, 1, 9, 7, mode)
            .unwrap()
            .unwrap();
        draw(maps, &d, &b)
    };
    // T[d] (row 0, column d) for k = 1; T[256·d + color] for k = 2.
    let row0 = |tab: &Vec<[u8; 256]>, d: u8| tab[0][usize::from(d)];
    let cell = |tab: &Vec<[u8; 256]>, d: u8| tab[usize::from(d)][usize::from(color)];
    type Want<'a> = Box<dyn Fn(u8) -> u8 + 'a>;
    let cases: Vec<(u8, Want)> = vec![
        (0, Box::new(|d| cell(&p.alpha_blend[2], d))),
        (1, Box::new(|d| cell(&p.alpha_blend[1], d))),
        (2, Box::new(|d| cell(&p.alpha_blend[0], d))),
        (3, Box::new(|d| row0(&p.additive_blend, d))),
        (4, Box::new(|d| row0(&p.multiplicative_blend, d))),
        (5, Box::new(|_| color)),
        (6, Box::new(|d| row0(&p.max_component_blend, d))),
        (7, Box::new(|_| color)),
        (8, Box::new(|_| color)),
        (255, Box::new(|_| color)),
    ];
    for (mode, want) in cases {
        let out = rect(&maps, mode);
        for y in 0..8 {
            for x in 0..10 {
                let i = y * 10 + x;
                let w = if (1..9).contains(&x) && (1..7).contains(&y) {
                    want(b[i])
                } else {
                    b[i]
                };
                assert_eq!(out[i], w, "mode {mode} ({x}, {y})");
            }
        }
    }
    // k = 1 does not read the color.
    let other = maps.push(blend::color_row(3));
    let d = blend::gdi_rectangle(&t, SURFACE, other, 1, 1, 9, 7, 3)
        .unwrap()
        .unwrap();
    assert_eq!(draw(&maps, &d, &b), rect(&maps, 3));
}

/// Diamond row `r` of an isometric floor block: start column and width.
fn iso_row(r: u32) -> (u32, u32) {
    let k = if r <= 7 { r } else { 14 - r };
    (14 - 2 * k, 4 + 4 * k)
}

#[test]
fn iso_floor_block_gradient_vector() {
    let (maps, t, _) = tables();
    // Corners 0, 255, 255, 0: every row a = 0, b = 31.
    let corners = [0, 255, 255, 0];
    let chain = shading::floor_block_chain(&t, BlockLight::Gradient(corners), 20, 10);
    let g = *chain.gradient().expect("a gradient");
    assert_eq!(g.kind, GradientKind::RleFloor);
    assert_eq!(g.block(), Rect::new(20, 10, 32, 15));
    for r in 0..15 {
        assert_eq!(g.level(0, r), 0, "row {r}");
        assert_eq!(g.level(31, r), 30, "row {r}");
    }
    // Row 7 columns 0, 14, 31 → maps 0, 13, 30.
    assert_eq!([0, 14, 31].map(|x| g.level(x, 7)), [0, 13, 30]);
    // The diamond image of the block: rows 0 / 7 / 14 composed.
    let mut pixels = vec![0u8; 32 * 15];
    for r in 0..15 {
        let (s, w) = iso_row(r);
        for x in s..s + w {
            pixels[(r * 32 + x) as usize] = 9;
        }
    }
    let frames = vec![FrameImage {
        width: 32,
        height: 15,
        pixels,
    }];
    let mut item = scene::DrawItem::new(FrameId(0), 20, 10);
    item.shade = chain;
    let view = Rect::new(20, 10, 32, 15);
    let out = scene::compose(&[item], &frames, &maps, view).unwrap();
    let row = |r: usize| &out[r * 32..r * 32 + 32];
    // Light map k sends every index to 50 + k.
    let drawn = |r: usize| -> Vec<usize> { (0..32).filter(|&x| row(r)[x] != 0).collect() };
    assert_eq!(drawn(0), vec![14, 15, 16, 17]);
    assert_eq!(drawn(14), vec![14, 15, 16, 17]);
    assert_eq!(drawn(7), (0..32).collect::<Vec<_>>());
    assert_eq!([0, 14, 31].map(|x| row(7)[x]), [50, 63, 80]);
    for x in 14..18 {
        let want = 50 + ((31 * x) >> 5) as u8;
        assert_eq!(row(0)[x as usize], want);
        assert_eq!(row(14)[x as usize], want);
    }
}
