//! Round trip from bytes: synthetic files (test-fixtures writers, no game
//! data) are parsed by the `d2-formats` readers, written native, read back
//! and compared, so the value under test is a real decoder result.

use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::dt1::Dt1;
use d2_native::kind::{grey_palette, round_trip};
use test_fixtures::dt1 as dt1w;
use test_fixtures::sprites::{dc6_file, dc6_frames, dcc_file, Dc6Shape, DccShape};

#[test]
fn parsed_dc6_round_trips() {
    for panel in [false, true] {
        let shape = Dc6Shape {
            directions: 4,
            frames: 5,
            width: 40,
            height: 30,
            panel,
        };
        let frames = dc6_frames(shape, 7);
        let dc6 = Dc6::parse(&dc6_file(&frames, 4, 5)).unwrap();
        round_trip(&dc6, "data/global/ui/x.dc6", &grey_palette()).unwrap();
    }
}

#[test]
fn parsed_dcc_round_trips() {
    let out = dcc_file(
        DccShape {
            directions: 4,
            frames: 6,
            width: 48,
            height: 40,
        },
        3,
    );
    let dcc = Dcc::parse(&out.file).unwrap();
    round_trip(&dcc, "data/global/chars/x.dcc", &grey_palette()).unwrap();
}

#[test]
fn parsed_dt1_round_trips() {
    let mut t = dt1w::tile(1, 2, 3, 4);
    let mut px = vec![0u8; 256 + 224];
    for (i, v) in px.iter_mut().enumerate() {
        *v = (i % 250) as u8 + 1;
    }
    t.blocks.push(d2_formats::dt1::Dt1Block {
        x: 0,
        y: 0,
        unknown1: 0,
        grid_x: 0,
        grid_y: 0,
        format: 1,
        unknown2: 0,
        pixels: {
            // keep only the diamond
            let skip = [14, 12, 10, 8, 6, 4, 2, 0, 2, 4, 6, 8, 10, 12, 14];
            let run = [4, 8, 12, 16, 20, 24, 28, 32, 28, 24, 20, 16, 12, 8, 4];
            let mut p = vec![0u8; 480];
            for r in 0..15 {
                for c in skip[r]..skip[r] + run[r] {
                    p[r * 32 + c] = px[r * 16 + c % 16];
                }
            }
            p
        },
    });
    let bytes = dt1w::write(&dt1w::file(vec![t, dt1w::tile(0, 0, 0, 1)]));
    let dt1 = Dt1::parse(&bytes).unwrap();
    assert_eq!(dt1.tiles.len(), 2);
    round_trip(&dt1, "data/global/tiles/x.dt1", &grey_palette()).unwrap();
}
