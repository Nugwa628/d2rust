//! The front end's native UI pass: packing cels, placement on the output
//! and the pass (CPU emulation) against the CPU picture.

use d2_formats::palette::{Palette, Rgb};

use super::*;
use crate::gpu_compositor::ui_pass::{emulate_ui, UiPixel};
use crate::scene::MapTable;
use crate::ui::frame::{FramePos, Presentation, ScaleMode};

const W: u32 = 800;
const H: u32 = 600;

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

fn place(out_w: u32, out_h: u32) -> UiPlacement {
    UiPlacement {
        frame_w: W,
        frame_h: H,
        out_w,
        out_h,
    }
}

/// An owned cel for the tests.
#[derive(Clone)]
struct TestCel {
    key: u64,
    px: Option<Vec<u8>>,
    w: u32,
    h: u32,
    index: u8,
    x: i32,
    y: i32,
    mode: u8,
    tables: bool,
    map: Option<[u8; 256]>,
}

fn image(key: u64, w: u32, h: u32, x: i32, y: i32, f: impl Fn(u32, u32) -> u8) -> TestCel {
    let px = (0..h)
        .flat_map(|r| (0..w).map(move |c| (r, c)))
        .map(|(r, c)| f(c, r))
        .collect();
    TestCel {
        key,
        px: Some(px),
        w,
        h,
        index: 0,
        x,
        y,
        mode: 5,
        tables: true,
        map: None,
    }
}

fn solid(w: u32, h: u32, x: i32, y: i32, index: u8) -> TestCel {
    TestCel {
        key: 0,
        px: None,
        w,
        h,
        index,
        x,
        y,
        mode: 5,
        tables: false,
        map: None,
    }
}

fn emit_all(cels: &[TestCel], emit: &mut dyn FnMut(Cel<'_>)) {
    for c in cels {
        emit(Cel {
            key: c.key,
            pixels: match &c.px {
                Some(px) => CelPixels::Image { px, w: c.w, h: c.h },
                None => CelPixels::Solid {
                    w: c.w,
                    h: c.h,
                    index: c.index,
                },
            },
            x: c.x,
            y: c.y,
            mode: c.mode,
            tables: c.tables,
            glyph: None,
            map: c.map,
        });
    }
}

fn pack(cels: &mut FrontUiCels, list: &[TestCel], p: UiPlacement) -> FrontUiFrame {
    cels.pack(None, &palette(), p, None, &mut |emit| emit_all(list, emit))
        .expect("packs")
}

/// The pass over `frame` on the CPU: one [`UiPixel`] per output pixel.
fn run(cels: &mut FrontUiCels, frame: &FrontUiFrame) -> Vec<UiPixel> {
    let mut maps = MapTable::new();
    for row in frame.maps.as_chunks::<256>().0 {
        maps.push(*row);
    }
    let (_, pages) = cels.pages();
    emulate_ui(&frame.packed, &pages, &maps, &palette())
}

/// The CPU picture of opaque cels (index 0 background), as the front end
/// draws them: images skip index 0 and go through their map; solids write
/// their index.
fn cpu(list: &[TestCel]) -> Vec<u8> {
    let mut px = vec![0u8; (W * H) as usize];
    let mut put = |x: i32, y: i32, s: u8| {
        if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) {
            px[(y as u32 * W + x as u32) as usize] = s;
        }
    };
    for c in list {
        for r in 0..c.h as i32 {
            for col in 0..c.w as i32 {
                match &c.px {
                    Some(p) => {
                        let i = p[(r as u32 * c.w + col as u32) as usize];
                        if i != 0 {
                            let s = c.map.map_or(i, |m| m[usize::from(i)]);
                            put(c.x + col, c.y + r, s);
                        }
                    }
                    None => put(c.x + col, c.y + r, c.index),
                }
            }
        }
    }
    px
}

/// Opaque cels side by side, overlapping, off the edges, a solid and a
/// mapped image.
fn scene() -> Vec<TestCel> {
    let mut mapped = image(3, 7, 5, 300, 200, |c, r| ((c + r) % 3) as u8 + 10);
    let mut map = [0u8; 256];
    for (i, m) in map.iter_mut().enumerate() {
        *m = (i as u8).wrapping_mul(3).max(1);
    }
    mapped.map = Some(map);
    vec![
        solid(800, 600, 0, 0, 40),
        image(1, 10, 6, 5, 5, |c, r| {
            if (c + r) % 4 == 0 {
                0
            } else {
                (c * 9 + r) as u8 + 1
            }
        }),
        image(2, 10, 6, 15, 5, |c, _| c as u8 + 100),
        image(1, 10, 6, 12, 8, |c, r| {
            if (c + r) % 4 == 0 {
                0
            } else {
                (c * 9 + r) as u8 + 1
            }
        }),
        image(4, 20, 20, -7, 590, |c, r| (c ^ r) as u8 | 1),
        mapped,
        solid(3, 9, 700, 100, 200),
    ]
}

#[test]
fn at_whole_scales_the_pass_equals_the_cpu_picture_scaled() {
    let list = scene();
    let picture = cpu(&list);
    let pal = palette();
    for k in [2u32, 3] {
        let mut cels = FrontUiCels::new();
        let frame = pack(&mut cels, &list, place(W * k, H * k));
        let out = run(&mut cels, &frame);
        for y in 0..H * k {
            for x in 0..W * k {
                let want = pal.colors[usize::from(picture[((y / k) * W + x / k) as usize])];
                let got = out[(y * W * k + x) as usize];
                let rgb = got.apply([0.0; 3]);
                let want = [want.r, want.g, want.b].map(|v| f32::from(v) / 255.0);
                for c in 0..3 {
                    assert!(
                        (rgb[c] - want[c]).abs() < 1e-4 && got.cover[c] > 0.999,
                        "{k}x at ({x}, {y}): {rgb:?} vs {want:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn every_output_pixel_of_a_cel_maps_back_into_its_frame_pixels() {
    // 1080p and 1440p: fractional scales, where seams could open.
    for (ow, oh) in [(1440, 1080), (1920, 1440)] {
        let p = place(ow, oh);
        let pres = Presentation::with_mode(ow, oh, W, H, ScaleMode::Fit).unwrap();
        assert_eq!((pres.image_w, pres.image_h), (ow, oh));
        let left = image(1, 13, 7, 100, 50, |_, _| 1);
        let right = image(2, 9, 7, 113, 50, |_, _| 2);
        let mut cels = FrontUiCels::new();
        let frame = pack(&mut cels, &[left.clone(), right.clone()], p);
        let [a, b] = [frame.packed.items[0], frame.packed.items[1]];
        // Side by side: no gap, no overlap.
        assert_eq!(a.area[2], b.area[0]);
        for (it, c) in [(a, &left), (b, &right)] {
            for oy in it.area[1]..it.area[3] {
                for ox in it.area[0]..it.area[2] {
                    let FramePos::Inside(at) = pres.to_frame(i64::from(ox), i64::from(oy)) else {
                        panic!("inside");
                    };
                    assert!((c.x..c.x + c.w as i32).contains(&at.x), "{ox}");
                    assert!((c.y..c.y + c.h as i32).contains(&at.y), "{oy}");
                }
            }
            // And every output pixel mapping into the cel is in its area.
            let (x0, x1) = (p.x(c.x), p.x(c.x + c.w as i32));
            assert_eq!((x0 as u32, x1 as u32), (it.area[0], it.area[2]));
        }
    }
}

#[test]
fn art_is_put_in_the_atlas_once() {
    let list = scene();
    let mut cels = FrontUiCels::new();
    pack(&mut cels, &list, place(1600, 1200));
    let (v1, _) = cels.pages();
    pack(&mut cels, &list, place(2400, 1800));
    let (v2, _) = cels.pages();
    assert_eq!(v1, v2, "nothing new: the same pages");
    let mut more = list.clone();
    more.push(image(99, 4, 4, 0, 0, |_, _| 7));
    pack(&mut cels, &more, place(1600, 1200));
    assert_ne!(cels.pages().0, v2, "a new cel changes the pages");
}

#[test]
fn a_solid_draws_its_index_even_index_0() {
    let pal = palette();
    let list = [solid(800, 600, 0, 0, 77), solid(10, 10, 20, 20, 0)];
    let mut cels = FrontUiCels::new();
    let frame = pack(&mut cels, &list, place(1600, 1200));
    let out = run(&mut cels, &frame);
    let at = |x: u32, y: u32| out[(y * 1600 + x) as usize].apply([0.0; 3]);
    let c = |i: usize| {
        [pal.colors[i].r, pal.colors[i].g, pal.colors[i].b].map(|v| f32::from(v) / 255.0)
    };
    assert_eq!(at(0, 0), c(77));
    assert_eq!(at(45, 45), c(0));
    assert_eq!(at(60, 45), c(77));
}

#[test]
fn draw_modes_without_tables_are_opaque_and_with_them_blend() {
    assert_eq!(mode_blends(None, &palette()), [UiBlend::Opaque; 8]);
    // A cel that skips the tables is opaque whatever its mode.
    let mut c = image(1, 2, 2, 0, 0, |_, _| 5);
    c.mode = 3;
    c.tables = false;
    let mut cels = FrontUiCels::new();
    let frame = pack(&mut cels, &[c], place(1600, 1200));
    assert_eq!(frame.packed.items[0].blend, UiBlend::Opaque.words().0);
}

#[test]
fn cels_off_the_picture_draw_nothing() {
    let list = [
        image(1, 5, 5, -5, 0, |_, _| 1),
        image(2, 5, 5, 800, 600, |_, _| 1),
    ];
    let mut cels = FrontUiCels::new();
    let frame = pack(&mut cels, &list, place(1600, 1200));
    assert!(frame.packed.items.is_empty());
}

/// A font sheet for HD Text: `H`, `A` and `~`, 6 x 10 cels standing on their bottom row, ink in rows 2..10
/// and columns 1..5 of colour 7 (`H`) or 5.
fn font_sheet() -> (d2_formats::font::FontTable, d2_formats::dc6::Dc6) {
    use d2_formats::dc6::{Dc6, Dc6Frame, Dc6Header};
    use d2_formats::font::{FontTable, Glyph};
    let frame = |main: u8| {
        let mut pixels = vec![0u8; 60];
        for y in 2..10 {
            for x in 1..5 {
                pixels[y * 6 + x] = main;
            }
        }
        Dc6Frame {
            flip: 0,
            width: 6,
            height: 10,
            offset_x: 0,
            offset_y: 0,
            unknown: 0,
            next_block: 0,
            pixels,
        }
    };
    let glyph = |code: char, frame: u16| Glyph {
        code: code as u16,
        unknown1: 0,
        width: 6,
        height: 10,
        unknown2: 0,
        unknown3: 0,
        frame,
        unknown5: 0,
    };
    let dc6 = Dc6 {
        header: Dc6Header {
            version: 6,
            flags: 0,
            encoding: 0,
            termination: [0; 4],
            directions: 1,
            frames_per_direction: 3,
        },
        frames: vec![frame(7), frame(5), frame(5)],
    };
    let table = FontTable {
        version: 1,
        unknown: 0,
        count: 3,
        height: 10,
        width: 6,
        glyphs: vec![glyph('H', 0), glyph('A', 1), glyph('~', 2)],
    };
    (table, dc6)
}

/// A text-colour map sending every index `i` to `i + 100`.
fn text_map() -> [u8; 256] {
    std::array::from_fn(|i| (i as u8).wrapping_add(100))
}

/// Packs a menu line: glyph `A` at (100, 50) through [`text_map`], glyph
/// `~` at (110, 50), then a plain image. `glyphs`: whether the glyph cels
/// name their sheet (as `front_host::item_cels` does).
fn pack_text(
    cels: &mut FrontUiCels,
    hd: Option<&HdTextArt>,
    glyphs: bool,
    p: UiPlacement,
) -> FrontUiFrame {
    let (table, dc6) = font_sheet();
    let plain = image(9, 4, 4, 300, 300, |_, _| 3);
    cels.pack(None, &palette(), p, hd, &mut |emit| {
        for (frame, x) in [(1u16, 100), (2, 110)] {
            let f = &dc6.frames[usize::from(frame)];
            emit(Cel {
                key: crate::app::front_cels::cel_key("font", "", 0, u32::from(frame)),
                pixels: CelPixels::Image {
                    px: &f.pixels,
                    w: f.width,
                    h: f.height,
                },
                x,
                y: 50,
                mode: 5,
                tables: true,
                map: Some(text_map()),
                glyph: glyphs.then_some(CelGlyph {
                    sheet: r"data\local\font\latin\font16.dc6",
                    table: &table,
                    dc6: &dc6,
                    frame,
                }),
            });
        }
        emit_all(std::slice::from_ref(&plain), emit);
    })
    .expect("packs")
}

#[test]
fn without_the_glyph_art_glyph_cels_pack_exactly_as_plain_cels() {
    let p = place(1600, 1200);
    let off = pack_text(&mut FrontUiCels::new(), None, true, p);
    let plain = pack_text(&mut FrontUiCels::new(), None, false, p);
    assert_eq!(off, plain);
    assert!(off.packed.rep.is_none());
    assert!(off.packed.items.iter().all(|i| i.source == SOURCE_ATLAS));
}

#[test]
fn bitmap_text_frame_hashes_stay_unchanged() {
    use sha2::{Digest, Sha256};
    // Independently specified bitmap picture: two mapped 4x8 glyphs at
    // (101,52)/(111,52), a 4x4 image at (300,300), palette-zero background.
    // Integer enlargement must preserve these fixed RGBA frame hashes.
    for (scale, expected) in [
        (
            1,
            "4f1c9991b846ef2c0bf901e959c34f18915a47eb7f91e908a8982627dc0e435f",
        ),
        (
            2,
            "370225bf29343591a6cd1cc7799027099146e7c4ae25c83909451bfd1a2c2eaf",
        ),
    ] {
        let mut cels = FrontUiCels::new();
        let frame = pack_text(&mut cels, None, true, place(800 * scale, 600 * scale));
        let pixels = super::emulate_rgba(&mut cels, &frame);
        assert_eq!(format!("{:x}", Sha256::digest(&pixels)), expected);
    }
}

/// How many pixels of `area` (`[x0, y0, x1, y1]`) differ between two outputs
/// of the same size.
fn differing_in(a: &[UiPixel], b: &[UiPixel], out_w: u32, area: [u32; 4]) -> usize {
    let [x0, y0, x1, y1] = area;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (y * out_w + x) as usize))
        .filter(|&i| a[i] != b[i])
        .count()
}

/// Whether the output is uniform inside every `scale`-pixel block of `area`
/// from its corner: a bitmap enlarged by a whole number draws each frame
/// pixel as one flat block, with no detail finer than the frame's pixels.
fn flat_in_blocks(out: &[UiPixel], out_w: u32, area: [u32; 4], scale: u32) -> bool {
    let [x0, y0, x1, y1] = area;
    (y0..y1).step_by(scale as usize).all(|by| {
        (x0..x1).step_by(scale as usize).all(|bx| {
            let first = out[(by * out_w + bx) as usize];
            (by..(by + scale).min(y1))
                .all(|y| (bx..(bx + scale).min(x1)).all(|x| out[(y * out_w + x) as usize] == first))
        })
    })
}

/// Text goes through the display upscaler like everything else: at every
/// window size the picture is shown enlarged at, the pass draws the glyphs
/// at the output resolution (detail below one frame pixel, smooth edges)
/// without moving or recolouring them. The same list drawn without the
/// glyph art is the plain bitmap, uniform inside each frame pixel.
#[test]
fn text_renders_at_the_output_resolution_at_every_display_setting() {
    use crate::gpu_compositor::ui_pass::{REP_INDEXED, SOURCE_REPLACEMENT};
    for (w, h) in [
        (800, 600),
        (1600, 1200),
        (1280, 720),
        (1920, 1080),
        (2560, 1440),
        (3840, 2160),
        (1365, 767),
    ] {
        let p = place(w, h);
        let art = HdTextArt::new();
        let mut hd_cels = FrontUiCels::new();
        let hd = pack_text(&mut hd_cels, Some(&art), true, p);
        let mut off_cels = FrontUiCels::new();
        let off = pack_text(&mut off_cels, None, true, p);
        let plain = pack_text(&mut FrontUiCels::new(), None, false, p);
        assert_eq!(off, plain, "Upscaled packing must stay byte identical");
        for (on, off) in hd.packed.items.iter().zip(&off.packed.items) {
            assert_eq!(
                (on.area, on.place, on.src, on.shade, on.shade_len),
                (off.area, off.place, off.src, off.shade, off.shade_len)
            );
        }
        assert_eq!(hd.packed.items[0].source, SOURCE_REPLACEMENT);
        assert_eq!(hd.packed.items[0].rep[3], REP_INDEXED);
        let bitmap = run(&mut off_cels, &off);
        let enhanced = run(&mut hd_cels, &hd);
        assert_ne!(bitmap, enhanced);
        // The glyph `A` (4 x 8 frame pixels at (101, 52)) and a frame pixel
        // of margin all round.
        let area = [100 * w / 800, 51 * h / 600, 106 * w / 800, 61 * h / 600];
        assert!(
            differing_in(&enhanced, &bitmap, w, area) > 0,
            "text at {w}x{h} must be drawn finer than the frame's own pixels"
        );
        if (w % 800, h % 600) == (0, 0) && w / 800 == h / 600 && w >= 1600 {
            let scale = w / 800;
            // Block edges of the margin start at frame pixel 100 / 51.
            assert!(
                flat_in_blocks(&bitmap, w, area, scale),
                "the plain bitmap at {w}x{h} is flat inside each frame pixel"
            );
            assert!(
                !flat_in_blocks(&enhanced, w, area, scale),
                "the text at {w}x{h} is finer than one frame pixel"
            );
        }
        assert!(
            enhanced
                .iter()
                .any(|p| p.cover[0] > 0.0 && p.cover[0] < 1.0),
            "enhanced edges must have smooth coverage, including at integer scales"
        );
        // The solid interior of A keeps its mapped colour, at every
        // output scale. Edge coverage changes alpha, never the colour map.
        let x = (103.0 * w as f32 / 800.0) as u32;
        let y = (56.0 * h as f32 / 600.0) as u32;
        let at = (y * w + x) as usize;
        let c = palette().colors[105];
        let want = [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0);
        // Filtering can round a channel by a few ULPs. Keep the error far
        // below one palette step (1/255), so a changed index still fails.
        for (mode, pixel) in [("HD", enhanced[at]), ("Upscaled", bitmap[at])] {
            for (actual, expected) in pixel.apply([0.0; 3]).into_iter().zip(want) {
                assert!(
                    (actual - expected).abs() <= 1e-6,
                    "{mode} at {w}x{h}: channel {actual} != {expected}"
                );
            }
        }
        assert_eq!(
            run(&mut hd_cels, &hd),
            enhanced,
            "cached rendering is deterministic"
        );
    }
}
