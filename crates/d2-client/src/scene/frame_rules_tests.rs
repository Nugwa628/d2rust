//! Tests for frame-composition rules not covered by `tests.rs`, one synthetic case each: the
//! PL2 palette is presented unmodified, the camera and shake are resolved once per frame before
//! the draws, and the world is skipped in screen open mode 3 while the UI is still built.

use d2_sim::rng::Seed;

use super::{present_palette, to_rgba};
use crate::bridge::world::ClientWorld;
use crate::bridge::ClientUnit;
use crate::composite::ComponentFrame;
use crate::frames::{FramePart, FrameSetKey};
use crate::rules::camera::shake_offsets;
use crate::rules::{
    Camera, ClientPos, FrameSize, MapTile, OpenMode, Shake, TileList, UnitPosition, ViewSource,
};
use crate::scene::{BlendOp, DrawKey, ShadeChain};
use crate::ui::{ImageRef, ImageRequest, Point, UiDraw};
use crate::world_view::{
    build_frame, frame_camera, RunningShake, UnitPose, Unspecified, ViewAssets, ViewError, ViewFeed,
};

#[test]
fn presents_the_pl2_palette_unmodified() {
    // Every entry distinct and entry 0 not black: no gamma ramp, no
    // forced-black entry 0.
    let mut pl2 = vec![0u8; 1024 + 0x3500];
    for i in 0..256usize {
        pl2[4 * i] = i as u8;
        pl2[4 * i + 1] = 255 - i as u8;
        pl2[4 * i + 2] = (i as u8).wrapping_mul(7).wrapping_add(13);
        pl2[4 * i + 3] = 0xA5;
    }
    let palette = present_palette(&pl2).unwrap();
    let indices: Vec<u8> = (0..=255).collect();
    let rgba = to_rgba(&indices, &palette);
    for (i, px) in rgba.chunks(4).enumerate() {
        assert_eq!(
            px,
            [pl2[4 * i], pl2[4 * i + 1], pl2[4 * i + 2], 255],
            "index {i}"
        );
    }
    assert_eq!(&rgba[..4], [0, 255, 13, 255]);
}

/// A feed with a local player, an open mode, an optional shake and map
/// tiles whose frames are not resident (a drawn tile is an error).
struct Feed {
    mode: u8,
    tiles: usize,
    shake: Option<RunningShake>,
    seed: Seed,
}

impl Feed {
    fn new(mode: u8, tiles: usize, shake: Option<RunningShake>) -> Self {
        let mut seed = Seed::default();
        seed.set(1234, 666);
        Feed {
            mode,
            tiles,
            shake,
            seed,
        }
    }
}

const PLAYER: UnitPosition = UnitPosition::Static { sx: 100, sy: 40 };

impl ViewSource for Feed {
    fn unit_position(&self, _: &ClientUnit) -> Result<UnitPosition, String> {
        Err("none".into())
    }
    fn unit_offset(&self, _: &ClientUnit, _: &UnitPose) -> Result<(i32, i32), String> {
        Err("none".into())
    }
    fn map_tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<MapTile>, ViewError> {
        let key = FrameSetKey::new("data/global/tiles/t.dt1", FramePart::Tile(0)).unwrap();
        Ok((0..self.tiles)
            .map(|i| MapTile {
                cell: (i as i32, 0),
                list: TileList::Floor,
                frame: ComponentFrame {
                    set: key.clone(),
                    index: 0,
                },
                blocks: Vec::new(),
                shade: ShadeChain::EMPTY,
                blend: BlendOp::Opaque,
                key: DrawKey::default(),
            })
            .collect())
    }
}

impl ViewFeed for Feed {
    fn player(&self, _: &ClientWorld) -> Result<Option<UnitPosition>, ViewError> {
        Ok(Some(PLAYER))
    }
    fn open_mode(&self, _: &ClientWorld) -> Result<OpenMode, ViewError> {
        Ok(OpenMode::new(self.mode).unwrap())
    }
    fn shake(&self, _: &ClientWorld) -> Result<Option<RunningShake>, ViewError> {
        Ok(self.shake)
    }
    fn player_seed(&mut self, _: &ClientWorld) -> Result<&mut Seed, ViewError> {
        Ok(&mut self.seed)
    }
    fn blank_screen(&self, _: &ClientWorld) -> Result<bool, ViewError> {
        Ok(true)
    }
}

fn world(ticks: u64) -> ClientWorld {
    ClientWorld {
        server_ticks: ticks,
        ..ClientWorld::default()
    }
}

fn assets() -> ViewAssets {
    ViewAssets::new(d2_formats::palette::Palette {
        colors: [d2_formats::palette::Rgb::default(); 256],
    })
}

#[test]
fn camera_and_shake_once_per_frame_before_the_draws() {
    // Peak 6 held from tick 0: a = 6 in every frame below.
    let shake = Some(RunningShake {
        shake: Shake::start(6, 0, 1000, 10).unwrap(),
        start_tick: 0,
    });
    for mode in [0, 3] {
        let mut feed = Feed::new(mode, 0, shake);
        let mut want = feed.seed;
        for tick in 1..=3 {
            // One built frame = one camera = exactly two seed draws,
            // whether or not the world is drawn (mode 3).
            let o = shake_offsets(6, &mut want);
            build_frame(&world(tick), &[], &Unspecified, &mut feed, &assets()).unwrap();
            assert_eq!(feed.seed, want, "mode {mode}, tick {tick}");
            assert!((-6..6).contains(&o.0) && (-6..6).contains(&o.1));
        }
    }
    // The one (dx, dy) of the frame moves both origins alike.
    let mut feed = Feed::new(0, 0, shake);
    let mut seed = feed.seed;
    let (dx, dy) = shake_offsets(6, &mut seed);
    let cam = frame_camera(&world(1), &mut feed).unwrap().unwrap();
    let still = Camera::new(FrameSize::D2RUST, OpenMode::NONE, PLAYER.client(), (0, 0));
    assert_eq!(
        (cam.tile, cam.unit),
        (
            ClientPos {
                x: still.tile.x + dx,
                y: still.tile.y + dy
            },
            ClientPos {
                x: still.unit.x + dx,
                y: still.unit.y + dy
            }
        )
    );
}

fn ui_image() -> UiDraw {
    UiDraw::Image(ImageRequest {
        image: ImageRef { file: 1, frame: 0 },
        at: Point::new(0, 0),
        clip: crate::ui::Rect::new(0, 0, 800, 600),
        look: crate::ui::CelLook::PLAIN,
        call: crate::ui::draw::CelCall::Draw,
    })
}

#[test]
fn world_is_skipped_in_open_mode_3_and_the_ui_still_drawn() {
    // A listed tile whose frame is not resident: drawing it fails.
    let e = build_frame(
        &world(1),
        &[],
        &Unspecified,
        &mut Feed::new(0, 1, None),
        &assets(),
    )
    .unwrap_err();
    assert!(matches!(e, ViewError::Tile { index: 0, .. }), "{e}");
    for mode in [1, 2] {
        assert!(build_frame(
            &world(1),
            &[],
            &Unspecified,
            &mut Feed::new(mode, 1, None),
            &assets()
        )
        .is_err());
    }
    // Mode 3: the world (tiles and units) is not drawn at all.
    let mut w = world(1);
    let key = crate::bridge::UnitKey {
        unit_type: 1,
        guid: 7,
    };
    w.units.insert(key, ClientUnit::new(key));
    let frame = build_frame(&w, &[], &Unspecified, &mut Feed::new(3, 1, None), &assets()).unwrap();
    assert!(frame.items.is_empty());
    assert_eq!((frame.units_drawn, frame.units_hidden), (0, 1));
    // The UI is still built after it: its request reaches the UI rules
    // (which refuse it here: no UI rule yet), not a tile error.
    let e = build_frame(
        &w,
        &[ui_image()],
        &Unspecified,
        &mut Feed::new(3, 1, None),
        &assets(),
    )
    .unwrap_err();
    assert!(
        matches!(
            e,
            // The UI rules' refusal of request 0 (the UI binding resolves requests one by one),
            // never a world error.
            ViewError::Ui { index: 0, ref error }
                if matches!(**error, ViewError::Unresolved { what: "UI image", .. })
        ),
        "{e}"
    );
}
