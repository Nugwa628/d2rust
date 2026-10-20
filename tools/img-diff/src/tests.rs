//! Compare rules on synthetic images (never game art).

use super::*;

/// A 64 x 48 test picture: a colour gradient with a solid square at
/// (20, 12)-(35, 27), so a shift moves hard edges.
fn picture() -> RgbaImage {
    RgbaImage::from_fn(64, 48, |x, y| {
        if (20..36).contains(&x) && (12..28).contains(&y) {
            Rgba([250, 240, 10, 255])
        } else {
            Rgba([(x * 3) as u8, (y * 5) as u8, 80, 255])
        }
    })
}

fn run(a: &RgbaImage, b: &RgbaImage, opts: &Options) -> Outcome {
    compare(a, b, opts).expect("same size").0
}

#[test]
fn identical_pictures_pass_strict() {
    let a = picture();
    let o = run(&a, &a.clone(), &Options::default());
    assert!(o.pass);
    assert_eq!((o.compared, o.changed, o.max_delta), (64 * 48, 0, 0));
    assert_eq!(o.changed_box, None);
}

#[test]
fn one_pixel_off_fails_strict_and_is_located() {
    let a = picture();
    let mut b = a.clone();
    b.put_pixel(5, 40, Rgba([0, 255, 0, 255]));
    let (o, diff) = compare(&a, &b, &Options::default()).unwrap();
    assert!(!o.pass);
    assert_eq!(o.changed, 1);
    assert_eq!(
        o.changed_box,
        Some(Rect {
            x: 5,
            y: 40,
            w: 1,
            h: 1
        })
    );
    assert_eq!(*diff.get_pixel(5, 40), CHANGED);
    assert_ne!(*diff.get_pixel(6, 40), CHANGED);
    // One pixel of 3072 is 0.033 %: inside a 0.5 % allowance.
    let loose = Options {
        max_pct: 0.5,
        ..Options::default()
    };
    assert!(run(&a, &b, &loose).pass);
}

#[test]
fn a_one_pixel_shift_fails_even_with_tolerance() {
    let a = picture();
    let b = RgbaImage::from_fn(64, 48, |x, y| *a.get_pixel(x.saturating_sub(1), y));
    let opts = Options {
        tol: 3,
        max_pct: 0.5,
        masks: Vec::new(),
    };
    let o = run(&a, &b, &opts);
    assert!(!o.pass);
    // The square's left and right sides changed (16 rows each), plus the
    // gradient, which steps by 3 per column (within tolerance).
    assert_eq!(o.changed, 32);
    let r = o.changed_box.unwrap();
    assert_eq!((r.x, r.y, r.w, r.h), (20, 12, 17, 16));
}

#[test]
fn small_colour_noise_passes_within_tolerance() {
    let a = picture();
    let b = RgbaImage::from_fn(64, 48, |x, y| {
        let p = a.get_pixel(x, y).0;
        Rgba([
            p[0].saturating_add(2),
            p[1].saturating_sub(2),
            p[2] + 1,
            p[3],
        ])
    });
    let within = Options {
        tol: 3,
        ..Options::default()
    };
    let o = run(&a, &b, &within);
    assert!(o.pass);
    assert_eq!((o.changed, o.max_delta), (0, 2));
    let below = Options {
        tol: 1,
        ..Options::default()
    };
    assert!(!run(&a, &b, &below).pass);
}

#[test]
fn different_sizes_are_not_compared() {
    let a = picture();
    let b = RgbaImage::new(64, 47);
    assert_eq!(
        compare(&a, &b, &Options::default()).unwrap_err(),
        SizeMismatch {
            a: (64, 48),
            b: (64, 47)
        }
    );
}

#[test]
fn an_edge_strip_change_shows_only_in_the_edge_strip() {
    // An edge smear: the outer 3 columns and rows change, the rest not.
    let a = picture();
    let b = RgbaImage::from_fn(64, 48, |x, y| {
        if x < 3 || y < 3 || x >= 61 || y >= 45 {
            Rgba([0, 0, 0, 255])
        } else {
            *a.get_pixel(x, y)
        }
    });
    let strict = Options::default();
    let o = run(&a, &b, &strict);
    assert!(!o.pass);
    assert_eq!(
        o.changed_box,
        Some(Rect {
            x: 0,
            y: 0,
            w: 64,
            h: 48
        })
    );
    let edges = |n| Options {
        masks: vec![Mask::Edges(n)],
        ..Options::default()
    };
    let o = run(&a, &b, &edges(3));
    assert!(o.pass);
    assert_eq!(o.compared, 58 * 42);
    assert!(!run(&a, &b, &edges(2)).pass);
}

#[test]
fn a_hud_change_passes_with_the_hud_masked() {
    // 1068 x 600: the 16:9 frame; the control panel's stone bar runs to
    // both sides of the frame.
    let a = RgbaImage::from_pixel(1068, 600, Rgba([10, 20, 30, 255]));
    let mut b = a.clone();
    for x in 0..1068 {
        for y in 500..600 {
            b.put_pixel(x, y, Rgba([200, 0, 0, 255]));
        }
    }
    assert!(!run(&a, &b, &Options::default()).pass);
    let hud = Options {
        masks: vec![Mask::Hud],
        ..Options::default()
    };
    assert!(run(&a, &b, &hud).pass);
    // A change in the world above the panel is still seen.
    b.put_pixel(50, 490, Rgba([200, 0, 0, 255]));
    assert_eq!(run(&a, &b, &hud).changed, 1);
}

#[test]
fn named_masks_scale_with_the_shot() {
    assert_eq!(
        Mask::Hud.rects(800, 600),
        vec![Rect {
            x: 0,
            y: 496,
            w: 800,
            h: 104
        }]
    );
    // 1068 x 600: same lines, the full frame width.
    assert_eq!(
        Mask::Hud.rects(1068, 600),
        vec![Rect {
            x: 0,
            y: 496,
            w: 1068,
            h: 104
        }]
    );
    // 1920 x 1080 is the 16:9 frame scaled by 1.8. 496 x 1.8 = 892.8:
    // the top rounds down and the bottom up, so the mask covers every line
    // the scaled HUD touches.
    assert_eq!(
        Mask::Hud.rects(1920, 1080),
        vec![Rect {
            x: 0,
            y: 892,
            w: 1920,
            h: 188
        }]
    );
    assert_eq!(
        Mask::Belt.rects(800, 600),
        vec![Rect {
            x: 420,
            y: 456,
            w: 132,
            h: 144
        }]
    );
    // Masks never reach past the shot.
    assert_eq!(
        Mask::Rect(Rect {
            x: 60,
            y: 40,
            w: 100,
            h: 100
        })
        .rects(64, 48),
        vec![Rect {
            x: 60,
            y: 40,
            w: 4,
            h: 8
        }]
    );
}

#[test]
fn mask_names_parse() {
    assert_eq!(Mask::parse("hud"), Ok(Mask::Hud));
    assert_eq!(Mask::parse("belt"), Ok(Mask::Belt));
    assert_eq!(Mask::parse("edges:4"), Ok(Mask::Edges(4)));
    assert_eq!(
        Mask::parse("rect:1,2,3,4"),
        Ok(Mask::Rect(Rect {
            x: 1,
            y: 2,
            w: 3,
            h: 4
        }))
    );
    assert!(Mask::parse("edges:x").is_err());
    assert!(Mask::parse("rect:1,2,3").is_err());
    assert!(Mask::parse("globe").is_err());
    assert_eq!(
        Mask::parse_list("hud,edges:2,rect:0,0,5,5"),
        Ok(vec![
            Mask::Hud,
            Mask::Edges(2),
            Mask::Rect(Rect {
                x: 0,
                y: 0,
                w: 5,
                h: 5
            })
        ])
    );
}

#[test]
fn everything_masked_passes() {
    let a = picture();
    let b = RgbaImage::new(64, 48);
    let all = Options {
        masks: vec![Mask::Edges(100)],
        ..Options::default()
    };
    let o = run(&a, &b, &all);
    assert!(o.pass);
    assert_eq!((o.compared, o.changed_pct), (0, 0.0));
}

#[test]
fn folder_pairs_include_one_sided_names() {
    let a = vec!["t100.png".to_string(), "t200.png".to_string()];
    let b = vec!["t300.png".to_string(), "t100.png".to_string()];
    assert_eq!(
        pair_names(&a, &b),
        vec![
            ("t100.png".to_string(), true, true),
            ("t200.png".to_string(), true, false),
            ("t300.png".to_string(), false, true),
        ]
    );
}
