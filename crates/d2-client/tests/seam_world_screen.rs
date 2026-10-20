//! Contract checks of the world ↔ screen seam: the draw side
//! (`rules::camera`, the world view's feed) and the pick side
//! (`bridge::click::screen_to_world`, `bridge::hover`) convert the same
//! values with the same origin, signs and rounding. No game files.

use d2_client::bridge::click::screen_to_world;
use d2_client::bridge::world::{ClientUnit, ClientWorld, UnitKey, PLAYER};
use d2_client::rules::camera::{
    moving_to_client, static_to_client, Camera, ClientPos, FrameSize, OpenMode,
};
use d2_client::world_view::corpse_click::camera_for;
use d2_client::world_view::preview::Preview;
use d2_client::world_view::{frame_camera, ModelFeed, NoFeed, ViewFeed};

fn centre(c: i32) -> u32 {
    ((c as u32) << 16) | 0x8000
}

fn cameras() -> Vec<Camera> {
    let mut out = Vec::new();
    for mode in 0..=3 {
        let mode = OpenMode::new(mode).unwrap();
        for player in [(0, 0), (1000, 2000), (-37, 411), (16 * 5, 8 * 9 + 8)] {
            let at = ClientPos {
                x: player.0,
                y: player.1,
            };
            out.push(Camera::new(FrameSize::D2RUST, mode, at, (0, 0)));
            out.push(Camera::new(FrameSize::D2RUST, mode, at, (-3, 2)));
        }
    }
    out
}

#[test]
fn moving_unit_drawn_feet_pick_the_subtile_below() {
    // The pick adds no row constant: a moving unit drawn with its
    // feet at the subtile centre picks the subtile one step down on screen.
    for cam in cameras() {
        for sx in 90..110 {
            for sy in 90..110 {
                let at = moving_to_client(centre(sx), centre(sy));
                let (x, y) = cam.unit_draw(at, (0, 0));
                assert_eq!(screen_to_world(&cam, x, y), (sx + 1, sy + 1), "{cam:?}");
            }
        }
    }
}

#[test]
fn static_unit_draw_point_picks_back_to_its_subtile() {
    for cam in cameras() {
        for sx in 90..110 {
            for sy in 90..110 {
                let (x, y) = cam.unit_draw(static_to_client(sx, sy), (0, 0));
                assert_eq!(screen_to_world(&cam, x, y), (sx, sy), "{cam:?}");
            }
        }
    }
}

#[test]
fn every_pixel_of_a_subtile_diamond_picks_that_subtile() {
    // The subtile diamond whose top vertex is 8 rows above the static
    // draw point (the pick adds no row constant) is 32 wide
    // and 16 high (× 16, × 8): the pixels of one diamond
    // map to one subtile and no pixel is lost.
    let cam = Camera::new(
        FrameSize::D2RUST,
        OpenMode::NONE,
        ClientPos::default(),
        (0, 0),
    );
    let (sx, sy) = (7, 3);
    let (x0, y0) = cam.unit_draw(static_to_client(sx, sy), (0, 0));
    let mut hits = 0;
    for dy in -8i32..8 {
        for dx in -16i32..16 {
            let (a, b) = (dx + 2 * (dy + 8), 2 * (dy + 8) - dx);
            if (0..32).contains(&a) && (0..32).contains(&b) {
                assert_eq!(screen_to_world(&cam, x0 + dx, y0 + dy), (sx, sy));
                hits += 1;
            }
        }
    }
    assert_eq!(hits, 256);
}

#[test]
fn local_player_draws_mid_play_area_and_picks_the_subtile_below() {
    // A moving unit drawn at its subtile picks the subtile one step down
    // on screen (the pick adds no row constant).
    for mode in 0..=3 {
        let mode = OpenMode::new(mode).unwrap();
        let at = moving_to_client(centre(5000), centre(4000));
        let cam = Camera::new(FrameSize::D2RUST, mode, at, (0, 0));
        let (x, y) = cam.unit_draw(at, (0, 0));
        assert_eq!((x, y), (400 + cam.view.shift_x, 292));
        assert_eq!(screen_to_world(&cam, x, y), (5001, 4001));
    }
}

const ME: UnitKey = UnitKey {
    unit_type: PLAYER,
    guid: 1,
};

fn world_at(cell: (u16, u16)) -> ClientWorld {
    let mut w = ClientWorld::default();
    let mut p = ClientUnit::new(ME);
    p.position = Some(cell);
    w.units.insert(ME, p);
    w.local_player = Some(ME);
    w
}

#[test]
fn without_prediction_the_pick_camera_is_the_draw_camera() {
    let w = world_at((100, 100));
    let mut feed = ModelFeed::<NoFeed>::default().with_preview(Preview::default());
    feed.set_local_prediction(None);
    let draw = frame_camera(&w, &mut feed).unwrap().unwrap();
    assert_eq!(camera_for(&w, 0), Some(draw));
}

#[test]
fn with_a_prediction_every_camera_of_the_frame_is_the_draw_camera() {
    use d2_client::world_view::{camera_at, frame_anchor};
    let w = world_at((100, 100));
    let mut feed = ModelFeed::<NoFeed>::default().with_preview(Preview::default());
    feed.set_local_prediction(Some((ME, (centre(103), centre(100)))));
    let draw = frame_camera(&w, &mut feed).unwrap().unwrap();
    // The pick / label / corpse / automap camera: the frame's anchor
    // under the frame's open mode, the draw camera exactly.
    let anchor = frame_anchor(&w, &mut feed).unwrap();
    let (pick, _) = camera_at(&w, &feed, anchor).unwrap().unwrap();
    assert_eq!(pick, draw);
    // It stands on the predicted sub-tile, not the model cell.
    assert_ne!(camera_for(&w, 0), Some(draw));
    let at = moving_to_client(centre(103), centre(100));
    assert_eq!(draw.unit_draw(at, (0, 0)), (400, 292));
}

#[test]
fn with_a_prediction_every_camera_is_the_draw_camera() {
    // Model (100, 100), the walk prediction three sub-tiles on: the draw,
    // the pick / hover / label camera and the local position all read
    // the prediction.
    let mut w = world_at((100, 100));
    let own = (centre(103), centre(100));
    w.set_local_walk(Some(own), Some(2));
    let mut feed = ModelFeed::<NoFeed>::default().with_preview(Preview::default());
    feed.set_local_prediction(Some((ME, own)));
    let draw = frame_camera(&w, &mut feed).unwrap().unwrap();
    assert_eq!(camera_for(&w, 0), Some(draw));
    assert_eq!(w.local_position(), Some(own));
    assert_eq!(w.local_cell(), Some((103, 100)));
    // A placement since the prediction was recorded wins.
    w.units.get_mut(&ME).unwrap().position = Some((90, 90));
    assert_eq!(w.local_cell(), Some((90, 90)));
}

#[test]
fn a_units_pick_anchor_is_its_draw_anchor() {
    use d2_client::bridge::hover::unit_feet as feet;
    use d2_client::bridge::world::{ITEM, MONSTER, OBJECT};
    let cam = Camera::new(
        FrameSize::D2RUST,
        OpenMode::NONE,
        ClientPos { x: 0, y: 0 },
        (0, 0),
    );
    // The spec vector: an object at (7, 3) with the player at client
    // (0, 0) is drawn (and picked) at (464, 372).
    assert_eq!(feet(&cam, OBJECT, (7, 3)), (464, 372));
    for cell in [(7u16, 3u16), (100, 90), (0, 0)] {
        let (sx, sy) = (i32::from(cell.0), i32::from(cell.1));
        for ty in [OBJECT, ITEM] {
            assert_eq!(
                feet(&cam, ty, cell),
                cam.unit_draw(static_to_client(sx, sy), (0, 0))
            );
        }
        for ty in [PLAYER, MONSTER] {
            assert_eq!(
                feet(&cam, ty, cell),
                cam.unit_draw(moving_to_client(centre(sx), centre(sy)), (0, 0))
            );
        }
    }
}
