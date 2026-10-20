//! The front end's art on the user's game install:
//! every DC6 a screen draws is in the archives, and the fire overlay's
//! draw mode 3 is the sky PL2's additive table in index space.

use d2_client::app::front_host::{compose, FrontArt, HEIGHT, WIDTH};
use d2_client::ui::front_end::screens::main_menu::{LOGO_LEFT, LOGO_RIGHT};
use d2_client::ui::front_end::*;
use d2_client::ui::geom::Point;
use d2_formats::dc6::Dc6;
use d2_formats::palette::{Palette, Pl2};

use crate::app_support;

fn menu() -> FrontEnd {
    let mut f = FrontEnd::with_screens(true, Box::new(true));
    f.start(false);
    f
}

fn read(path: &str) -> Vec<u8> {
    app_support::live()
        .archives
        .source()
        .read_file(path)
        .unwrap_or_else(|| panic!("{path} not in the archives"))
        .unwrap()
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn every_art_file_the_main_menu_draws_is_in_the_archives() {
    let mut f = menu();
    f.tick();
    let mut files = std::collections::BTreeSet::new();
    for d in f.draw() {
        if let DrawItem::Art { file, .. } | DrawItem::Blend { file, .. } = d {
            files.insert(file);
        }
    }
    assert!(files.contains(LOGO_LEFT) && files.contains(LOGO_RIGHT));
    for file in files {
        let bytes = read(&format!(r"data\global\ui\{file}.dc6"));
        Dc6::parse(&bytes).unwrap_or_else(|e| panic!("{file}: {e}"));
    }
}

#[test]
#[ignore = "real data: needs D2_GAME_DIR (tools/realdata-gate.sh)"]
fn fire_overlay_is_the_pl2_additive_table_in_index_space() {
    let pal = Palette::parse(&read(SKY_PALETTE[0])).unwrap();
    let pl2 = Pl2::parse(&read(SKY_PALETTE[1])).unwrap();
    let load = |f: &str| Dc6::parse(&read(&format!(r"data\global\ui\{f}.dc6"))).unwrap();
    let (black, fire) = (load(LOGO_LEFT), load(r"FrontEnd\D2logoFireLeft"));
    let at = Point::new(400, 120);
    let frame = 7;
    let items = [
        DrawItem::Art {
            file: LOGO_LEFT,
            frame,
            at,
        },
        DrawItem::Blend {
            file: r"FrontEnd\D2logoFireLeft",
            frame,
            at,
            mode: 3,
            boxed: false,
        },
    ];
    let mut art = FrontArt::new(app_support::live().archives.source());
    let px = compose(&items, Some(&mut art));

    // The reference: an index plane built from the two cels by the spec's rule.
    let mut idx = vec![0u8; (WIDTH * HEIGHT) as usize];
    let mut put = |dc6: &Dc6, add: bool| {
        let f = &dc6.frames[frame as usize];
        let top = at.y + f.offset_y - f.height as i32 + 1;
        for row in 0..f.height {
            for col in 0..f.width {
                let s = f.pixels[(row * f.width + col) as usize];
                let (x, y) = (at.x + f.offset_x + col as i32, top + row as i32);
                if s != 0 && (0..WIDTH as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
                    let n = (y as u32 * WIDTH + x as u32) as usize;
                    idx[n] = if add {
                        pl2.additive_blend[usize::from(idx[n])][usize::from(s)]
                    } else {
                        s
                    };
                }
            }
        }
    };
    put(&black, false);
    put(&fire, true);
    let mut differing = 0;
    for (n, &i) in idx.iter().enumerate() {
        let c = pal.colors[usize::from(i)];
        if px[n * 4..n * 4 + 3] != [c.r, c.g, c.b] {
            differing += 1;
        }
    }
    assert_eq!(differing, 0, "pixels differing from the PL2 ADD result");
}
