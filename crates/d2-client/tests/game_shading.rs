//! The act 1 values of the shading and blend-mode test vectors, on the
//! user's `pal.pl2`, through `rules::shading`, `rules::blend` and the CPU
//! reference compositor.
//!
//! Expected values unconfirmed in this repo: written without game files,
//! so no test here carries a `Covers` claim until its first local run
//! passes (local checks).
//!
//! Ignored by default. Run:
//! `D2_GAME_DIR=<install> cargo test -p d2-client --test game_shading -- --ignored --nocapture`

use d2_client::rules::blend;
use d2_client::rules::shading::{self, ShadeTables};
use d2_client::scene::{self, DrawItem, FrameId, FrameImage, MapId, MapTable, Rect};
use d2_formats::mpq::ArchiveSet;
use d2_formats::palette::Pl2;

const PL2: &str = r"data\global\palette\act1\pal.pl2";

fn act1() -> (Vec<u8>, Pl2) {
    let dir = std::env::var("D2_GAME_DIR").expect("D2_GAME_DIR must be set");
    let set = ArchiveSet::open_dir(&dir).expect("archives in D2_GAME_DIR open");
    let bytes = set.read(PL2).expect("act 1 pal.pl2 reads");
    let pl2 = Pl2::parse(&bytes).expect("act 1 pal.pl2 parses");
    (bytes, pl2)
}

/// One pixel `s` drawn over `d` with the ops of a cel in `mode`.
fn cel(
    maps: &MapTable,
    t: &ShadeTables,
    mode: u8,
    remap: Option<MapId>,
    v: u8,
    s: u8,
    d: u8,
) -> u8 {
    let (shade, blend) = blend::cel_ops(t, mode, remap, v);
    let mut it = DrawItem::new(FrameId(0), 0, 0);
    it.shade = shade;
    it.blend = blend;
    let frames = [FrameImage {
        width: 1,
        height: 1,
        pixels: vec![s],
    }];
    scene::compose_frame(
        &[it],
        &frames[..],
        maps,
        Rect::new(0, 0, 1, 1),
        &[d],
        scene::FramePlan::NONE,
    )
    .expect("composes")[0]
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn act1_shading_vectors() {
    let (bytes, pl2) = act1();
    assert_eq!(bytes.len(), 443_175);
    let mut maps = MapTable::new();
    let t = ShadeTables::push(&mut maps, &pl2);
    // v = 0x7F → map 15, L[100] = 68; v = 0xFF unlit; v = 0xF8 map 31.
    assert_eq!(bytes[0x400 + 15 * 256 + 100], 68);
    assert_eq!(cel(&maps, &t, 5, None, 0x7F, 100, 0), 68);
    assert_eq!(cel(&maps, &t, 5, None, 0xFF, 100, 0), 100);
    assert_eq!(cel(&maps, &t, 5, None, 0xF8, 31, 0), 31);
    // Map 31 is the identity, every light map sends 0 to 0.
    for i in 0..256 {
        assert_eq!(usize::from(pl2.light_levels[31][i]), i);
    }
    assert!(pl2.light_levels.iter().all(|m| m[0] == 0));
    // H.
    let h = shading::highlight_map(&pl2.base_palette);
    assert_eq!(
        (h[31], h[100], h[172], h[200], h[0]),
        (255, 168, 33, 217, 0)
    );
    for v in [0u8, 0x7F, 0xFF] {
        assert_eq!(cel(&maps, &t, 7, None, v, 31, 0), 255);
    }
    // The PL2 selected-unit shift is not H (equal in 63 of 256).
    let same = (0..256)
        .filter(|&i| h[i] == pl2.selected_unit_shift[i])
        .count();
    assert_eq!(same, 63);
    // R.
    let r = shading::red_map(&pl2.base_palette);
    assert_eq!((r[255], r[100], r[0]), (98, 10, 0));
    // p = 2 → hue map 1, p = 112 → map 111 (red tones).
    assert_eq!(
        cel(&maps, &t, 5, t.unit_remap(2).unwrap(), 0xFF, 100, 0),
        106
    );
    assert_eq!(bytes[0x53500 + 256 + 100], 106);
    assert_eq!(
        cel(&maps, &t, 5, t.unit_remap(112).unwrap(), 0xFF, 100, 0),
        33
    );
}

#[test]
#[ignore = "needs original game files in D2_GAME_DIR"]
fn act1_blend_vectors() {
    let (bytes, pl2) = act1();
    let mut maps = MapTable::new();
    let t = ShadeTables::push(&mut maps, &pl2);
    // Cels read row = destination.
    assert_eq!(bytes[0xE1FF], 31);
    assert_eq!(cel(&maps, &t, 2, None, 0xFF, 255, 172), 31);
    assert_eq!(pl2.alpha_blend[0][255][172], 190, "the transposed read");
    assert_eq!(cel(&maps, &t, 2, None, 0xFF, 200, 100), 207);
    assert_eq!(pl2.alpha_blend[0][200][100], 94);
    assert_eq!(cel(&maps, &t, 0, None, 0xFF, 255, 172), 190);
    assert_eq!(bytes[0x2E1FF], 190);
    assert_eq!(cel(&maps, &t, 1, None, 0xFF, 255, 172), 29);
    assert_eq!(bytes[0x1E1FF], 29);
    assert_eq!(cel(&maps, &t, 3, None, 0xFF, 200, 100), 170);
    assert_eq!(cel(&maps, &t, 4, None, 0xFF, 255, 172), 172);
    assert_eq!(
        cel(&maps, &t, 6, None, 0xFF, 0, 31),
        31,
        "s = 0 is not drawn"
    );
    assert_eq!(pl2.max_component_blend[31][0], 31);
    assert_eq!(pl2.max_component_blend[0][31], 30);
    for mode in [5u8, 7, 8] {
        let want = if mode == 7 {
            shading::highlight_map(&pl2.base_palette)[100]
        } else {
            100
        };
        assert_eq!(cel(&maps, &t, mode, None, 0xFF, 100, 9), want);
    }
    // MAX[256·d + 0] = d for every d; ADD and MUL symmetric.
    for d in 0..256 {
        assert_eq!(usize::from(pl2.max_component_blend[d][0]), d);
        for s in 0..256 {
            assert_eq!(pl2.additive_blend[d][s], pl2.additive_blend[s][d]);
            assert_eq!(
                pl2.multiplicative_blend[d][s],
                pl2.multiplicative_blend[s][d]
            );
        }
    }
    // Unit shadow, blended, d = 31 → 183; shadow tile d = 100, s = 200 → 207.
    assert_eq!(pl2.alpha_blend[0][31][0], 183);
    let (_, op) = blend::shadow_tile_ops(&t, true);
    assert_eq!(op, scene::BlendOp::IndexTable(t.alpha[0]));
    // Translucent wall a = 0xC0, L[s] = 255, d = 172 → A0[256·255 + 172] = 190.
    assert_eq!(pl2.alpha_blend[0][255][172], 190);
}
