//! The UI draw sink on the user's game files: rectangles of each
//! blend kind over the act 1 `pal.pl2` (measured vectors), the
//! skill-tree grey icon remap (`k` = 5) on the real sorceress icon file,
//! the cube's draw-mode-3 transmute frame on the real `horadric.dc6`, and
//! an item colour from the real item palette files, each through the
//! world view's UI path (`PanelArtLoader`, `PanelArtRules`, the CPU
//! reference compositor).
//!
//! Ignored by default. Run:
//! `D2_GAME_DIR=<install> cargo nextest run -p d2-client --test game_ui_draw_sink --run-ignored only`

use std::sync::{Arc, RwLock};

use d2_client::bridge::world::ClientWorld;
use d2_client::rules::shading::ShadeTables;
use d2_client::scene::{self, FramePlan, Rect};
use d2_client::ui::panels::{PanelTables, UiFiles};
use d2_client::ui::{CelLook, ImageRef, ImageRequest, Point, RectRequest, Remap, UiDraw, FRAME};
use d2_client::world_view::panel_art::{item_palette_name, PanelArtLoader, PanelArtRules};
use d2_client::world_view::{build, TextColors, Unspecified, ViewAssets};
use d2_formats::mpq::ArchiveSet;
use d2_formats::palette::Pl2;

const PL2: &str = r"data\global\palette\act1\pal.pl2";
const ICON: &str = r"spells\soskillicon";
const HORADRIC: &str = r"menu\horadric";

struct Real {
    set: Arc<ArchiveSet>,
    bytes: Vec<u8>,
    pl2: Pl2,
    files: UiFiles,
}

fn real() -> Real {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let set = ArchiveSet::open_dir(&dir).expect("archives in D2_GAME_DIR open");
    let bytes = set.read(PL2).expect("act 1 pal.pl2 reads");
    let pl2 = Pl2::parse(&bytes).expect("act 1 pal.pl2 parses");
    let mut files = PanelTables::load().unwrap().files;
    files.add(ICON);
    files.add(HORADRIC);
    Real {
        set: Arc::new(set),
        bytes,
        pl2,
        files,
    }
}

/// The world view's UI path over a frame of index `under`.
fn compose(r: &Real, draws: &[UiDraw], under: u8) -> Vec<u8> {
    let mut assets = ViewAssets::from_pl2(&r.bytes).unwrap();
    assets.shades = Some(ShadeTables::push(&mut assets.maps, &r.pl2));
    let colors = TextColors::push(&mut assets.maps, &r.bytes).unwrap();
    let loader = PanelArtLoader::new(r.set.clone(), r.files.clone());
    loader.ensure(draws, &mut assets).unwrap();
    let rules = PanelArtRules {
        rules: Unspecified,
        files: r.files.clone(),
        text: Some(Arc::new(RwLock::new(Some(colors)))),
    };
    let frame = build(&ClientWorld::default(), draws, &rules, &assets).unwrap();
    let view = Rect::FRAME;
    let base = vec![under; (view.width * view.height) as usize];
    scene::compose_frame(
        &frame.items,
        &assets.frames,
        &assets.maps,
        view,
        &base,
        FramePlan::NONE,
    )
    .unwrap()
}

fn at(px: &[u8], x: i32, y: i32) -> u8 {
    px[(y * scene::FRAME_WIDTH as i32 + x) as usize]
}

fn cel(r: &Real, name: &str, frame: u32, x: i32, y: i32, look: CelLook) -> UiDraw {
    UiDraw::Image(ImageRequest {
        image: ImageRef {
            file: r.files.id(name).unwrap(),
            frame,
        },
        at: Point::new(x, y),
        clip: FRAME,
        look,
        call: d2_client::ui::draw::CelCall::Draw,
    })
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn act1_rectangles_write_the_measured_blend_values() {
    let r = real();
    let rect = |x, color, mode| UiDraw::Rect(RectRequest::sized(x, 10, 4, 4, color, mode));
    // Blend kind 2: T[256·d + color] with d = 172, color 255: A0 → 31,
    // A2 (mode 0) → 190, A1 (mode 1) → 29 (the spec's act 1 vectors).
    let px = compose(
        &r,
        &[
            rect(10, 255, 2),
            rect(20, 255, 0),
            rect(30, 255, 1),
            rect(40, 255, 5),
        ],
        172,
    );
    assert_eq!(at(&px, 10, 10), 31);
    assert_eq!(at(&px, 20, 10), 190);
    assert_eq!(at(&px, 30, 10), 29);
    assert_eq!(at(&px, 40, 10), 255);
    assert_eq!(at(&px, 14, 10), 172, "column x0 + w is not drawn");
    // d = 100, color 200, mode 2 → 207 (the transposed read gives 94).
    let px = compose(&r, &[rect(10, 200, 2)], 100);
    assert_eq!(at(&px, 13, 13), 207);
    // Blend kind 1 (mode 3): d' = ADD[d], row 0, column d.
    let px = compose(&r, &[rect(10, 99, 3)], 100);
    assert_eq!(at(&px, 10, 10), r.pl2.additive_blend[0][100]);
}

/// Pixels of `name` frame `frame` drawn plainly at (x, y) and with `look`
/// 400 columns to the right, over index `under`: the (plain, looked) pairs
/// where the plain draw wrote a pixel other than `under`.
fn pairs(
    r: &Real,
    name: &str,
    frame: u32,
    (x, y): (i32, i32),
    look: CelLook,
    under: u8,
) -> Vec<(u8, u8)> {
    let px = compose(
        r,
        &[
            cel(r, name, frame, x, y, CelLook::PLAIN),
            cel(r, name, frame, x + 400, y, look),
        ],
        under,
    );
    let mut out = Vec::new();
    for y in 0..600 {
        for x in 0..400 {
            let p = at(&px, x, y);
            if p != under {
                out.push((p, at(&px, x + 400, y)));
            }
        }
    }
    out
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn the_grey_skill_icon_is_text_colour_map_5() {
    let r = real();
    let grey = CelLook {
        mode: 5,
        remap: Remap::Palette(5),
    };
    let p = pairs(&r, ICON, 0, (50, 300), grey, 0);
    assert!(p.len() > 500, "the icon draws ({} pixels)", p.len());
    // Map k is the PL2 text-colour map at 439,847 + 256k.
    let map5 = &r.bytes[439_847 + 5 * 256..439_847 + 6 * 256];
    assert!(p.iter().all(|&(s, d)| d == map5[usize::from(s)]));
    assert!(p.iter().any(|&(s, d)| s != d), "the remap changes pixels");
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn the_transmute_frame_adds() {
    let r = real();
    let add = CelLook {
        mode: 3,
        remap: Remap::None,
    };
    // Frame 1 (92 × 121 at offset (−205, 17)) over index 100.
    let under = 100u8;
    let p = pairs(&r, HORADRIC, 1, (300, 200), add, under);
    assert!(p.len() > 1000, "frame 1 draws ({} pixels)", p.len());
    let row = &r.pl2.additive_blend[usize::from(under)];
    assert!(p.iter().all(|&(s, d)| d == row[usize::from(s)]));
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn item_colours_read_the_item_palette_files() {
    let r = real();
    // invgrey (t = 6), map 3: the file's bytes 3·256 … 4·256.
    let file = r
        .set
        .read(&item_palette_name("invgrey"))
        .expect("invgrey.dat reads");
    assert!(file.len() >= 21 * 256);
    let look = CelLook {
        mode: 5,
        remap: Remap::ItemColor { t: 6, c: 3 },
    };
    let p = pairs(&r, ICON, 0, (50, 300), look, 0);
    assert!(p.len() > 500);
    let m = &file[3 * 256..4 * 256];
    assert!(p.iter().all(|&(s, d)| d == m[usize::from(s)]));
}
