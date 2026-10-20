//! Tests of the level backgrounds and pass 8.

use d2_formats::palette::{Palette, Rgb};
use d2_sim::rng::Seed;

use super::*;

fn gray_palette() -> Palette {
    let mut colors = [Rgb { r: 0, g: 0, b: 0 }; 256];
    for (i, c) in colors.iter_mut().enumerate() {
        let v = i as u8;
        *c = Rgb { r: v, g: v, b: v };
    }
    Palette { colors }
}

/// A seed whose next step has `step & 7 == want`.
fn seed_with_low3(want: u32) -> Seed {
    (0u32..)
        .map(|lo| Seed::new(lo, 666))
        .find(|s| {
            let mut c = *s;
            c.step() & 7 == want
        })
        .expect("some seed")
}

/// Stars at x 100 (speed −1) except star 0 at x 0.
fn parked(seed: Seed) -> Stars {
    let mut stars = Stars::new(Seed::new(7, 666), 0, &gray_palette(), 800, 600);
    stars.stars.fill(Star {
        x: 100,
        y: 5,
        speed: -1,
        color: 9,
    });
    stars.stars[0].x = 0;
    stars.seed = seed;
    stars
}

#[test]
fn stars_first_call_colors_then_stars_in_draw_order() {
    let seed = Seed::new(0x1234_5678, 666);
    let p = gray_palette();
    let stars = Stars::new(seed, 0, &p, 800, 600);

    let mut r = seed;
    let mut colors = [0u8; 8];
    for (i, c) in colors.iter_mut().enumerate() {
        let base = 128 + (128 * i as u32) / 7;
        let v: Vec<u32> = (0..3)
            .map(|_| (base + (r.step() & 63) - 32).min(255))
            .collect();
        *c = crate::rules::shading::nearest(&p, v[0], v[1], v[2]);
    }
    assert_eq!(stars.colors, colors);
    for j in 0..STARS {
        let x = r.roll(800) as i32;
        let y = r.roll(560) as i32;
        let speed = -1 - (r.step() % 5) as i32;
        let color = colors[(r.step() & 7) as usize];
        assert_eq!(stars.stars[j], Star { x, y, speed, color }, "star {j}");
        assert!((-5..=-1).contains(&speed));
    }
    assert_eq!(stars.seed, r);
    // On the gray ramp nearest is the gray itself: base 128 + 0…63 − 32.
    assert!(stars.colors.iter().all(|&c| c >= 96));
}

#[test]
fn stars_draw_one_pixel_lines_before_moving() {
    let mut stars = parked(Seed::new(1, 666));
    let out = stars.draw(41, 800, 600);
    assert_eq!(out.len(), STARS);
    assert_eq!(
        out[1],
        BackgroundDraw::Line {
            x0: 100,
            y0: 5,
            x1: 100,
            y1: 5,
            color: 9,
            alpha: 0xFF
        }
    );
    // Star 0 drew at x 0, before the move re-made it.
    assert!(matches!(out[0], BackgroundDraw::Line { x0: 0, x1: 0, .. }));
    assert_eq!(stars.stars[1].x, 99);
}

#[test]
fn star_remake_x_is_w_minus_1_plus_low_bits() {
    // Test vector: step & 7 = 3, W 800 → x = 802.
    let seed = seed_with_low3(3);
    let mut stars = parked(seed);
    stars.draw(41, 800, 600);
    let mut r = seed;
    r.step();
    let y = r.roll(560) as i32;
    let speed = -1 - (r.step() % 5) as i32;
    let color = stars.colors[(r.step() & 7) as usize];
    assert_eq!(
        stars.stars[0],
        Star {
            x: 802,
            y,
            speed,
            color
        }
    );
    assert_eq!(stars.seed, r);
    assert_eq!(stars.last, 41);
}

#[test]
fn star_tick_is_over_40_ms_unsigned() {
    let mut stars = parked(Seed::new(1, 666));
    stars.draw(40, 800, 600);
    assert_eq!((stars.stars[1].x, stars.last), (100, 0));
    // Wrapped tick count: 30 − (2^32 − 6) = 36, no move.
    stars.last = u32::MAX - 5;
    stars.draw(30, 800, 600);
    assert_eq!((stars.stars[1].x, stars.last), (100, u32::MAX - 5));
    stars.draw(35, 800, 600);
    assert_eq!((stars.stars[1].x, stars.last), (99, 35));
}

fn row(frames: [u32; 4], x0: i32, y: i32) -> Vec<BackgroundDraw> {
    (0..4)
        .map(|k| BackgroundDraw::Cel {
            file: SUMMIT_FILE,
            frame: frames[k],
            x: x0 + 256 * k as i32,
            y,
            light: 0xFFFF_FFFF,
            draw_mode: 5,
        })
        .collect()
}

#[test]
fn summit_vector_frames_1_2_3_0() {
    let sc = summit_scroll(10_063 + 2_056);
    assert_eq!(
        sc,
        SummitScroll {
            q: 257,
            c: 1,
            t: 1,
            x0: -1
        }
    );
    let mut s = Summit::new(Seed::new(3, 666), 800);
    let out = s.draw(10_063 + 2_056, 800, 1);
    let mut want = row([1, 2, 3, 0], -1, 256);
    want.extend(row([5, 6, 7, 4], -1, 512));
    assert_eq!(out[..8], want[..]);
    assert_eq!(out.len(), 8 + 2 * CLOUDS);
    // Resolution mode 2 adds the y 768 row, frames + 8.
    let out = s.draw(10_063 + 2_056, 800, 2);
    assert_eq!(out[8..12], row([9, 10, 11, 8], -1, 768)[..]);
    assert_eq!(out.len(), 12 + 2 * CLOUDS);
}

#[test]
fn summit_left_of_anchor_c_division_and_extra_frame() {
    // d = −8: q = −1, c = −1, t = 0, x0 = 1 > 0.
    assert_eq!(
        summit_scroll(10_063 - 8),
        SummitScroll {
            q: -1,
            c: -1,
            t: 0,
            x0: 1
        }
    );
    // d = −2,056: q = −257, q / 256 = −1, t = −1 + 4 = 3.
    let sc = summit_scroll(10_063 - 2_056);
    assert_eq!((sc.q, sc.c, sc.t, sc.x0), (-257, -1, 3, 1));
    // d = −7 truncates to q = 0.
    assert_eq!(summit_scroll(10_063 - 7).x0, 0);

    let mut s = Summit::new(Seed::new(3, 666), 800);
    let out = s.draw(10_063 - 2_056, 800, 1);
    let mut want = row([3, 0, 1, 2], 1, 256);
    want.push(BackgroundDraw::Cel {
        file: SUMMIT_FILE,
        frame: 2,
        x: 1 - 256,
        y: 256,
        light: SUMMIT_LIGHT,
        draw_mode: 5,
    });
    assert_eq!(out[..5], want[..]);
    assert!(matches!(
        out[9],
        BackgroundDraw::Cel {
            frame: 6,
            x: -255,
            y: 512,
            ..
        }
    ));
}

#[test]
fn clouds_first_use_move_wrap_and_draw() {
    let seed = Seed::new(0xCAFE, 666);
    let mut s = Summit::new(seed, 800);
    let mut r = seed;
    for c in &s.clouds {
        let x16 = r.roll(16 * 800) as i32;
        let y = (r.step() % 250) as i32;
        let speed = (r.step() & 15) as i32 + 8;
        assert_eq!(*c, Cloud { x16, y, speed });
    }
    assert_eq!(s.seed, r);

    // Cloud 0 lands exactly on the limit (no wrap), cloud 1 one past it,
    // cloud 2 negative (C division truncates toward 0).
    let limit = 16 * (800 + 368);
    s.clouds[0] = Cloud {
        x16: limit - 10,
        y: 7,
        speed: 10,
    };
    s.clouds[1] = Cloud {
        x16: limit - 10,
        y: 7,
        speed: 11,
    };
    s.clouds[2] = Cloud {
        x16: -5_888,
        y: 7,
        speed: 8,
    };
    let out = s.draw(0, 800, 1);
    let clouds = &out[out.len() - 2 * CLOUDS..];
    let cel = |frame, x, y| BackgroundDraw::Cel {
        file: CLOUD_FILE,
        frame,
        x,
        y,
        light: 0xDDDD_DDDD,
        draw_mode: 3,
    };
    assert_eq!(clouds[0], cel(0, limit / 16, 7));
    assert_eq!(clouds[1], cel(1, limit / 16 + 256, 7));
    let y = (r.step() % 300) as i32;
    assert_eq!(
        s.clouds[1],
        Cloud {
            x16: -5_888,
            y,
            speed: 11
        }
    );
    assert_eq!(clouds[2], cel(0, -368, y));
    assert_eq!(clouds[3], cel(1, -112, y));
    // −5,880 / 16 = −367 (not −368).
    assert_eq!(clouds[4], cel(0, -367, 7));
    assert_eq!(s.seed, r);
}

fn frame(level: u32) -> BackgroundFrame<'static> {
    BackgroundFrame {
        level,
        w: 800,
        h: 600,
        resolution_mode: 2,
        now: 1_000,
        exiting: false,
        player_x: 10_063,
        stars_seed: None,
        summit_seed: None,
        stars_last: None,
        palette: None,
    }
}

#[test]
fn pass1_only_in_levels_74_and_120_with_recorded_seeds() {
    let mut b = Backgrounds::default();
    assert_eq!(b.pass1(&frame(1)), Ok(Vec::new()));
    assert_eq!(
        b.pass1(&frame(74)),
        Err(BackgroundError::NoSeed { level: 74 })
    );
    assert_eq!(
        b.pass1(&frame(120)),
        Err(BackgroundError::NoSeed { level: 120 })
    );
    // Exit flag set: the summit is skipped, first use included.
    let mut f = frame(120);
    f.exiting = true;
    assert_eq!(b.pass1(&f), Ok(Vec::new()));
    assert!(b.summit.is_none());

    let seed = Seed::new(99, 666);
    let mut f = frame(120);
    f.summit_seed = Some(seed);
    let items = b.pass1(&f).unwrap();
    let mut direct = Summit::new(seed, 800);
    let draws = direct.draw(10_063, 800, 2);
    assert_eq!(items.len(), 12 + 2 * CLOUDS);
    for (i, (it, d)) in items.iter().zip(&draws).enumerate() {
        assert_eq!(
            it.key,
            OrderKey {
                pass: 1,
                major: 0,
                minor: i as u32
            }
        );
        assert_eq!(it.draw, *d);
    }
    // Later calls reuse the state; the seed is no longer read.
    assert_eq!(b.pass1(&frame(120)).unwrap().len(), 12 + 2 * CLOUDS);

    let p = gray_palette();
    let mut f = frame(74);
    f.stars_seed = Some(seed);
    // `last` is 0 before the first call: no error for it.
    assert_eq!(b.pass1(&f), Err(BackgroundError::NoPalette));
    f.stars_last = Some(0);
    assert_eq!(b.pass1(&f), Err(BackgroundError::NoPalette));
    f.palette = Some(&p);
    assert_eq!(b.pass1(&f).unwrap().len(), STARS);
}

#[test]
fn star_tick_starts_at_zero_and_the_first_call_moves_the_stars() {
    let p = gray_palette();
    let mut b = Backgrounds::default();
    let mut f = frame(74);
    f.stars_seed = Some(Seed::new(5, 666));
    f.stars_last = Some(0);
    f.palette = Some(&p);
    f.now = 1_000;
    b.pass1(&f).unwrap();
    let after_first = b.stars.as_ref().unwrap().clone();
    assert_eq!(after_first.last, 1_000);
    // Every star moved by its speed (or was re-made past the left edge).
    let mut fresh = Stars::new(Seed::new(5, 666), 0, &p, 800, 600);
    fresh.draw(1_000, 800, 600);
    assert_eq!(after_first, fresh);
    // 30 ms later: no move.
    f.now = 1_030;
    b.pass1(&f).unwrap();
    assert_eq!(b.stars.as_ref().unwrap(), &after_first);
    // More than 40 ms: moves, last := now.
    f.now = 1_041;
    b.pass1(&f).unwrap();
    assert_eq!(b.stars.as_ref().unwrap().last, 1_041);
}

#[test]
fn pass8_never_runs() {
    const { assert!(!PASS8_RUNS) };
    assert!(pass8_items().is_empty());
}

/// The mountain strip draws 4 cels a row up to 800 wide (4:3 unchanged)
/// and enough for the 16:9 frame: at every scroll position each row's
/// cels reach past its right edge.
#[test]
fn summit_rows_span_the_frame_width() {
    assert_eq!(summit_cels(640), 4);
    assert_eq!(summit_cels(800), 4);
    assert_eq!(summit_cels(1068), 6);
    let mut s = Summit::new(Seed::new(3, 666), 1068);
    for player_x in (0..20_000).step_by(37) {
        let out = s.draw(player_x, 1068, 2);
        let right = out
            .iter()
            .filter_map(|d| match *d {
                BackgroundDraw::Cel { file, x, y, .. } if file == SUMMIT_FILE && y == 256 => {
                    Some(x + SUMMIT_TILE)
                }
                _ => None,
            })
            .max()
            .unwrap();
        assert!(right >= 1068, "player x {player_x}: row ends at {right}");
    }
}
