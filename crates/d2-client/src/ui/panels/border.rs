//! The 800 × 600 stone border and the control panel base
//! art.
//!
//! The rows are `panel-layout.tsv` `border` and `ctrlpnl`. The control panel
//! rows use y = H, the GDI reference (under DirectDraw the outer two pieces sit
//! at H − 1; not reproduced: the reference driver is GDI). The overlays drawn
//! after the base (globes, belt, skill buttons, run / menu buttons, level name
//! timer) are not settled and not drawn here.

use super::{emit_static_draws, no_extra, PanelEnv, PanelTables};
use crate::frames::IndexFrame;
use crate::ui::draw::UiDrawSink;
use crate::ui::layout::PanelKey;
use d2_formats::palette::Palette;

/// Draws the border (resolution mode 2 only: left frames 0–4 when the open
/// mode is 2 or 3, right frames 5–9 when it is 1 or 3) and then the
/// control panel base (`ctrlpnl7` frames 0–4 at resolution mode ≠ 2,
/// `800ctrlpnl7` frames 0–5 at mode 2), in that order.
///
/// The border's pieces are placed for the 800 × 600 layout. In a wider
/// frame each side's pieces move with that side's panel
/// ([`crate::ui::layout::Screen::edge_dx`]), so a panel and its border keep
/// the 800 × 600 look; the 80 outer columns of each border then lie past
/// the frame's edge.
pub fn draw_border_and_ctrlpnl(t: &PanelTables, env: &PanelEnv, out: &mut dyn UiDrawSink) {
    let cond = env.cond(false, &no_extra);
    let dx = env.screen.edge_dx();
    let dy = env.screen.panel_dy();
    let left = |r: &crate::ui::layout::LayoutRow| r.item.starts_with("left");
    let mut l = crate::ui::draw::OffsetSink(&mut *out, dx, dy);
    emit_static_draws(t, PanelKey::Border, &cond, None, &left, &mut l);
    let mut r = crate::ui::draw::OffsetSink(&mut *out, -dx + env.screen.extra_right, dy);
    emit_static_draws(t, PanelKey::Border, &cond, None, &|r| !left(r), &mut r);
    // The centered HUD's native width is independent of an odd right column.
    let hud = PanelEnv {
        screen: crate::ui::layout::Screen {
            extra_right: 0,
            ..env.screen
        },
        ..*env
    };
    emit_static_draws(
        t,
        PanelKey::CtrlPnl,
        &hud.cond(false, &no_extra),
        None,
        &|_| true,
        out,
    );
}

/// The control panel art the side bars are built from.
pub const SIDE_ART: &str = "panel\\800ctrlpnl7";

/// Width of one vertical tile of the side bars' frame set.
pub const SIDE_TILE_W: i32 = 134;

/// The side bars' own frame set, made in code from [`SIDE_ART`] when the
/// art loader first needs it ([`side_bar_frames`]): frame 0 the left bar,
/// frame 1 the right bar. It never comes from an archive.
pub const SIDE_FILE: &str = "d2rust\\sidebar";

/// One vertical slice of a control panel frame: `frame` of [`SIDE_ART`], its
/// columns `x.. x + w`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slice {
    pub frame: u32,
    pub x: i32,
    pub w: i32,
}

/// Plain stone of the panel's bar, between its scrollwork, sockets and
/// boxes (frames 1 and 4). Every piece carries the bar's top and bottom
/// edge lines in the same rows.
pub const SIDE_STONE: [Slice; 3] = [
    Slice {
        frame: 1,
        x: 26,
        w: 12,
    },
    Slice {
        frame: 1,
        x: 74,
        w: 14,
    },
    Slice {
        frame: 4,
        x: 46,
        w: 15,
    },
];

/// The panel frames beside the bars: the life globe's statue (frame 0,
/// its first column touches the left bar) and the mana globe's statue
/// (frame 5, its last column touches the right bar).
pub const SIDE_LEFT_NEIGHBOUR: u32 = 0;
pub const SIDE_RIGHT_NEIGHBOUR: u32 = 5;

/// Columns blended across each join between two stone pieces.
pub const SIDE_BLEND: usize = 4;
/// Columns over which a bar's inner end fades into the statue beside it.
pub const SIDE_EDGE: usize = 6;

/// The side bars of a frame wider than the 800-wide layout: the control
/// panel stays centred, and the strip under the world on each side of it
/// (`ui_left` pixels wide, left of 0 and right of the layout's W in UI
/// space) gets its stone bar from [`SIDE_FILE`] (frame 0 left, frame 1
/// right), drawn at the panel's row (y = H). Nothing at 4:3 (`ui_left`
/// 0) or at 640 × 480.
pub fn draw_ctrlpnl_sides(t: &PanelTables, env: &PanelEnv, ui_left: i32, out: &mut dyn UiDrawSink) {
    if !env.screen.res2() {
        return;
    }
    let Some(file) = t.files.id(SIDE_FILE) else {
        return;
    };
    let (w, h) = (env.screen.w, env.screen.h);
    for (frame, x0, width) in [
        (0, -ui_left, ui_left),
        (1, w, ui_left + env.screen.extra_right),
    ] {
        let tiles = (width + SIDE_TILE_W - 1) / SIDE_TILE_W;
        for k in 0..tiles {
            let x = if frame == 0 {
                -(tiles - k) * SIDE_TILE_W
            } else {
                x0 + k * SIDE_TILE_W
            };
            let mut d = super::cel(file, frame, x, h);
            if let crate::ui::draw::UiDraw::Image(i) = &mut d {
                i.clip = crate::ui::geom::Rect::new(x0, 0, width as u16, h as u16);
            }
            out.push(d);
        }
    }
}

/// The two stone bars `width` pixels wide, made from the control panel's
/// frames (`panel`, the frames of [`SIDE_ART`]): one smooth stone bar
/// each, with no end pieces.
///
/// - The stone is the [`SIDE_STONE`] pieces in turn, every other one
///   mirrored, so a piece's edge meets a column that looks like it, and
///   across each join [`SIDE_BLEND`] columns crossfade from one piece to
///   the next (in colour, mapped back to the nearest `palette` index):
///   no join shows as a vertical line.
/// - The stone runs straight out to the frame's edge on the outer side.
/// - On the inner side its last [`SIDE_EDGE`] columns fade into the
///   column of the statue they touch, so the bar meets the panel with no
///   seam. The bar's top and bottom edge lines (its first two opaque rows
///   and its last row) and its transparent pixels are left as they are,
///   so the lines run unbroken along the whole bar.
/// - The right bar is the left bar's stone mirrored, faded into the mana
///   globe's statue.
///
/// The frames keep the panel bar's height and anchor, so they draw at the
/// panel's row. `None` when the frames are not the panel's shape or the
/// bar is narrower than one blended join and its edge.
pub fn side_bar_frames(
    panel: &[IndexFrame],
    palette: &Palette,
    width: i32,
) -> Option<[IndexFrame; 2]> {
    let f = |i: u32| panel.get(i as usize);
    let (bar, left_n, right_n) = (f(1)?, f(SIDE_LEFT_NEIGHBOUR)?, f(SIDE_RIGHT_NEIGHBOUR)?);
    let h = bar.height as usize;
    let fits = |s: &Slice| {
        f(s.frame).is_some_and(|fr| fr.height as usize == h && (s.x + s.w) as u32 <= fr.width)
    };
    let width = usize::try_from(width).ok()?;
    if !SIDE_STONE.iter().all(fits)
        || (left_n.height as usize) < h
        || (right_n.height as usize) < h
        || left_n.width == 0
        || right_n.width == 0
        || width < SIDE_BLEND + SIDE_EDGE
    {
        return None;
    }
    let column = |fr: &IndexFrame, x: usize| -> Vec<u8> {
        // Bottom rows: the statues are taller than the bar, and both stand
        // on the panel's row.
        let skip = fr.height as usize - h;
        (0..h)
            .map(|y| fr.pixels[(skip + y) * fr.width as usize + x])
            .collect()
    };
    let piece = |s: &Slice, mirrored: bool| -> Vec<Vec<u8>> {
        let fr = f(s.frame).expect("checked");
        (0..s.w)
            .map(|i| column(fr, (s.x + if mirrored { s.w - 1 - i } else { i }) as usize))
            .collect()
    };
    let mut stone: Vec<Vec<u8>> = Vec::new();
    let mut n = 0;
    while stone.len() < width {
        join(
            &mut stone,
            piece(&SIDE_STONE[n % SIDE_STONE.len()], n % 2 == 1),
            palette,
        );
        n += 1;
    }
    stone.truncate(width);
    let lines = edge_rows(&stone);
    let mut left = stone.clone();
    let towards = column(left_n, 0);
    fade_end(&mut left, &towards, &lines, palette);
    // The right bar is the same stone mirrored: faded at the stone's end,
    // which the mirror puts against the mana globe's statue.
    let mut right = stone;
    let towards = column(right_n, right_n.width as usize - 1);
    fade_end(&mut right, &towards, &lines, palette);
    let right: Vec<Vec<u8>> = right.into_iter().rev().collect();
    let frame = |cols: &[Vec<u8>]| {
        let w = cols.len();
        let mut pixels = vec![0u8; w * h];
        for (x, c) in cols.iter().enumerate() {
            for (y, &p) in c.iter().enumerate() {
                pixels[y * w + x] = p;
            }
        }
        IndexFrame::new(w as u32, h as u32, bar.x_off, bar.y_off, pixels)
            .ok()
            .map(|fr| fr.with_anchor(bar.anchor))
    };
    Some([frame(&left)?, frame(&right)?])
}

/// The rows of the bar's top and bottom edge lines: the first two rows
/// with an opaque pixel, and the last row.
fn edge_rows(cols: &[Vec<u8>]) -> Vec<usize> {
    let h = cols.first().map_or(0, Vec::len);
    let opaque = |y: usize| cols.iter().any(|c| c[y] != 0);
    let mut rows: Vec<usize> = (0..h).filter(|&y| opaque(y)).take(2).collect();
    if h > 0 && !rows.contains(&(h - 1)) {
        rows.push(h - 1);
    }
    rows
}

/// Fades the last [`SIDE_EDGE`] columns of `cols` into `towards` (the
/// column of what the bar touches at its end): the last column is mostly
/// `towards`, the sixth from the end barely. `keep` rows and transparent
/// pixels (in either) are left as they are.
fn fade_end(cols: &mut [Vec<u8>], towards: &[u8], keep: &[usize], palette: &Palette) {
    let k = SIDE_EDGE.min(cols.len());
    let d = (k + 1) as u32;
    let start = cols.len() - k;
    for (j, c) in cols[start..].iter_mut().enumerate() {
        let t = (j + 1) as u32;
        for (y, (a, &b)) in c.iter_mut().zip(towards).enumerate() {
            if keep.contains(&y) || *a == 0 || b == 0 {
                continue;
            }
            *a = blend(*a, b, t, d, palette);
        }
    }
}

/// Appends `next` to `cols`, the last [`SIDE_BLEND`] columns of `cols`
/// crossfaded into `next`'s first ones (the first piece is appended as
/// it is).
fn join(cols: &mut Vec<Vec<u8>>, next: Vec<Vec<u8>>, palette: &Palette) {
    let k = SIDE_BLEND.min(cols.len()).min(next.len());
    let start = cols.len() - k;
    let d = (k + 1) as u32;
    for (j, n) in next.iter().take(k).enumerate() {
        // The next piece's weight: 1/(k+1) at the first blended column,
        // k/(k+1) at the last.
        let t = (j + 1) as u32;
        for (a, &b) in cols[start + j].iter_mut().zip(n) {
            *a = blend(*a, b, t, d, palette);
        }
    }
    cols.extend(next.into_iter().skip(k));
}

/// Index `a` faded `t / d` of the way to index `b`, as the nearest palette
/// colour. Index 0 is transparent: where either side is transparent the
/// nearer pixel is taken whole.
fn blend(a: u8, b: u8, t: u32, d: u32, palette: &Palette) -> u8 {
    if a == b {
        return a;
    }
    if a == 0 || b == 0 {
        return if 2 * t < d { a } else { b };
    }
    let (ca, cb) = (palette.colors[a as usize], palette.colors[b as usize]);
    let mix = |x: u8, y: u8| ((u32::from(x) * (d - t) + u32::from(y) * t + d / 2) / d) as i32;
    let want = (mix(ca.r, cb.r), mix(ca.g, cb.g), mix(ca.b, cb.b));
    let mut best = (i32::MAX, a);
    for (i, c) in palette.colors.iter().enumerate().skip(1) {
        let sq = |p: u8, q: i32| (i32::from(p) - q).pow(2);
        let dist = sq(c.r, want.0) + sq(c.g, want.1) + sq(c.b, want.2);
        if dist < best.0 {
            best = (dist, i as u8);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::draw::{ImageRef, UiDraw};
    use crate::ui::layout::Screen;

    /// The two side bars of the 16:9 frame: frame 0 of the bars' own file
    /// left of the UI box and frame 1 right of it, each clipped to its
    /// strip; nothing at 4:3.
    #[test]
    fn the_side_bars_fill_both_strips() {
        let mut t = PanelTables::load().unwrap();
        t.files.extend([SIDE_FILE.to_string()]);
        let env = PanelEnv {
            screen: Screen::R800,
            open_mode: 0,
            exp: true,
        };
        let mut out: Vec<UiDraw> = Vec::new();
        draw_ctrlpnl_sides(&t, &env, 0, &mut out);
        assert!(out.is_empty());
        draw_ctrlpnl_sides(&t, &env, 134, &mut out);
        let id = t.files.id(SIDE_FILE).unwrap();
        let got: Vec<_> = out
            .iter()
            .map(|d| {
                let UiDraw::Image(i) = d else { panic!("{d:?}") };
                assert_eq!(i.image.file, id);
                (i.image.frame, i.at.x, i.at.y, i.clip.x, i.clip.w)
            })
            .collect();
        assert_eq!(
            got,
            vec![(0, -134, 600, -134, 134), (1, 800, 600, 800, 134)]
        );
    }

    /// Edge-line grey of the test bar (rows 1, 2 and 5).
    const LINE: u8 = 250;
    /// The statues' grey.
    const STATUE: u8 = 5;

    /// A grey palette (index i is (i, i, i)) and the panel's six frames:
    /// the bar frames (1–4) are 6 rows, the top one transparent, rows 1,
    /// 2 and 5 the edge lines, and in rows 3 and 4 column x is grey
    /// 10 + x in frame 1 and 150 + x in frame 4; the statues (0 and 5) are
    /// 9 rows of one dark grey.
    fn grey_panel() -> (Vec<IndexFrame>, Palette) {
        let mut pal = Palette {
            colors: [d2_formats::palette::Rgb { r: 0, g: 0, b: 0 }; 256],
        };
        for (i, c) in pal.colors.iter_mut().enumerate() {
            let v = i as u8;
            *c = d2_formats::palette::Rgb { r: v, g: v, b: v };
        }
        let bar = |w: u32, base: u8| {
            let px = (0..6u32)
                .flat_map(|y| {
                    (0..w).map(move |x| match y {
                        0 => 0,
                        1 | 2 | 5 => LINE,
                        _ => base + x as u8,
                    })
                })
                .collect();
            IndexFrame::new(w, 6, 0, 0, px).unwrap()
        };
        let statue = || IndexFrame::new(117, 9, 0, 0, vec![STATUE; 117 * 9]).unwrap();
        let frames = vec![
            statue(),
            bar(128, 10),
            bar(128, 10),
            bar(128, 10),
            bar(86, 150),
            statue(),
        ];
        (frames, pal)
    }

    /// Each bar is one smooth stone bar: stone right out to the frame
    /// edge (no end pieces), every other stone piece mirrored, every join
    /// spread over the blended columns, the inner end fading into the
    /// statue, the edge lines and the transparent top row unbroken along
    /// the whole bar, and the right bar the left one mirrored.
    #[test]
    fn the_side_bars_are_smooth_stone_end_to_end() {
        let (panel, pal) = grey_panel();
        let [left, right] = side_bar_frames(&panel, &pal, 134).unwrap();
        for b in [&left, &right] {
            assert_eq!((b.width, b.height), (134, 6));
            let row = |y: usize| &b.pixels[y * 134..(y + 1) * 134];
            assert!(row(0).iter().all(|&p| p == 0));
            for y in [1, 2, 5] {
                assert!(row(y).iter().all(|&p| p == LINE), "row {y}");
            }
        }
        let l = |x: usize| left.pixels[134 * 3 + x];
        let r = |x: usize| right.pixels[134 * 3 + x];
        // The outer end is plain stone: frame 1's stone at column 26.
        assert_eq!(l(0), 10 + 26);
        assert_eq!(r(133), 10 + 26);
        // Outside the fades the right bar mirrors the left one.
        for x in 0..134 - SIDE_EDGE {
            assert_eq!(r(133 - x), l(x), "column {x}");
        }
        // The inner end fades into the statue: its last column is nearer
        // the statue's grey than the stone it replaces.
        let last = 133;
        assert!(
            l(last) < l(last - SIDE_EDGE) && l(last) <= STATUE + 40,
            "{}",
            l(last)
        );
        assert!(r(0) <= STATUE + 40, "{}", r(0));
        // No hard edge in the stone: the largest jump between two pieces
        // (a frame 1 grey against a frame 4 grey) is spread over the
        // blended columns.
        let max_step = (150 + 60 - 10) / (SIDE_BLEND as i32 + 1) + 2;
        for x in 0..133 - SIDE_EDGE {
            let step = (i32::from(l(x + 1)) - i32::from(l(x))).abs();
            assert!(step <= max_step, "column {x}: {step}");
        }
        // The second piece (frame 1 stone at 74..88) runs mirrored.
        assert!((0..133).any(|x| l(x) == 10 + 80 && l(x + 1) == 10 + 79));
        // Too narrow for a join and the fade: no bars.
        assert!(side_bar_frames(&panel, &pal, 5).is_none());
        assert!(side_bar_frames(&panel[..4], &pal, 134).is_none());
    }

    fn draws(env: PanelEnv) -> (PanelTables, Vec<(String, u32, i32, i32)>) {
        let t = PanelTables::load().unwrap();
        let mut out: Vec<UiDraw> = Vec::new();
        draw_border_and_ctrlpnl(&t, &env, &mut out);
        let v = out
            .into_iter()
            .map(|d| match d {
                UiDraw::Image(i) => {
                    let ImageRef { file, frame } = i.image;
                    (
                        t.files.name(file).unwrap().to_string(),
                        frame,
                        i.at.x,
                        i.at.y,
                    )
                }
                UiDraw::Text(_) => panic!("text"),
                UiDraw::Rect(_) => panic!("rectangle"),
                UiDraw::Line(_) => panic!("line"),
            })
            .collect();
        (t, v)
    }

    #[test]
    fn border_800_mode3_all_frames_then_ctrlpnl() {
        let (_, d) = draws(PanelEnv {
            screen: Screen::R800,
            open_mode: 3,
            exp: true,
        });
        assert_eq!(d.len(), 16);
        let border: Vec<_> = d[..10]
            .iter()
            .map(|(f, fr, x, y)| (f.as_str(), *fr, *x, *y))
            .collect();
        assert_eq!(
            border,
            vec![
                ("panel\\800borderframe", 0, 0, 253),
                ("panel\\800borderframe", 1, 256, 63),
                ("panel\\800borderframe", 2, 0, 484),
                ("panel\\800borderframe", 3, 0, 553),
                ("panel\\800borderframe", 4, 256, 553),
                ("panel\\800borderframe", 5, 400, 63),
                ("panel\\800borderframe", 6, 544, 253),
                ("panel\\800borderframe", 7, 713, 484),
                ("panel\\800borderframe", 8, 544, 553),
                ("panel\\800borderframe", 9, 400, 553),
            ]
        );
        let ctrl: Vec<_> = d[10..]
            .iter()
            .map(|(f, fr, x, y)| (f.as_str(), *fr, *x, *y))
            .collect();
        assert_eq!(
            ctrl,
            vec![
                ("panel\\800ctrlpnl7", 0, 0, 600),
                ("panel\\800ctrlpnl7", 1, 165, 600),
                ("panel\\800ctrlpnl7", 2, 293, 600),
                ("panel\\800ctrlpnl7", 3, 421, 600),
                ("panel\\800ctrlpnl7", 4, 549, 600),
                ("panel\\800ctrlpnl7", 5, 683, 600),
            ]
        );
    }

    #[test]
    fn border_800_one_side_per_open_mode() {
        for (mode, frames) in [
            (0u8, vec![]),
            (1, vec![5, 6, 7, 8, 9]),
            (2, vec![0, 1, 2, 3, 4]),
        ] {
            let (_, d) = draws(PanelEnv {
                screen: Screen::R800,
                open_mode: mode,
                exp: true,
            });
            let got: Vec<u32> = d
                .iter()
                .filter(|(f, ..)| f == "panel\\800borderframe")
                .map(|(_, fr, ..)| *fr)
                .collect();
            assert_eq!(got, frames, "mode {mode}");
            assert_eq!(d.len(), frames.len() + 6);
        }
    }

    #[test]
    fn ctrlpnl_640_five_frames_no_border() {
        let (_, d) = draws(PanelEnv {
            screen: Screen::R640,
            open_mode: 3,
            exp: false,
        });
        let got: Vec<_> = d
            .iter()
            .map(|(f, fr, x, y)| (f.as_str(), *fr, *x, *y))
            .collect();
        // No border at 640 × 480 whatever the open mode.
        assert_eq!(
            got,
            vec![
                ("panel\\ctrlpnl7", 0, 0, 480),
                ("panel\\ctrlpnl7", 1, 165, 480),
                ("panel\\ctrlpnl7", 2, 293, 480),
                ("panel\\ctrlpnl7", 3, 421, 480),
                ("panel\\ctrlpnl7", 4, 523, 480),
            ]
        );
    }
}
