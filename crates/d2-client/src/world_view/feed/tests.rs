//! The camera feed: no local player → no camera, and nothing placeable;
//! the frame camera from the feed; the shake on the tick time base
//! (`t = 40 × ticks since the start`, two seed draws per drawn
//! frame while `a ≠ 0`); BlankScreen from a `Levels.txt` row.

use d2_sim::rng::Seed;

use super::*;
use crate::bridge::UnitKey;
use crate::frames::{FramePart, FrameSetKey};
use crate::rules::{ClientPos, TileList};
use crate::scene::DrawKey;
use crate::world_view::Unspecified;

/// A feed with what a test sets.
#[derive(Default)]
struct Feed {
    player: Option<UnitPosition>,
    tiles: usize,
    shake: Option<RunningShake>,
    seed: Seed,
}

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
        Ok(self.player)
    }
    fn open_mode(&self, _: &ClientWorld) -> Result<OpenMode, ViewError> {
        Ok(OpenMode::new(1).unwrap())
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

fn at_tick(ticks: u64) -> ClientWorld {
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
fn no_player_no_camera_and_nothing_placeable() {
    let world = at_tick(1);
    assert_eq!(frame_camera(&world, &mut NoFeed).unwrap(), None);
    let frame = build_frame(&world, &[], &Unspecified, &mut NoFeed, &assets()).unwrap();
    assert!(frame.items.is_empty());

    // A listed tile without a player is an error naming.
    let mut feed = Feed {
        tiles: 1,
        ..Feed::default()
    };
    let e = build_frame(&world, &[], &Unspecified, &mut feed, &assets()).unwrap_err();
    assert!(e.to_string().contains("render/camera.md"), "{e}");

    // A unit the rules draw without a player is an error too.
    let mut world = at_tick(1);
    let key = UnitKey {
        unit_type: 0,
        guid: 1,
    };
    world.units.insert(key, ClientUnit::new(key));
    let view = NoCamera {
        rules: &Unspecified,
        source: &NoFeed,
    };
    let pose = UnitPose {
        sub_frame: 0,
        cof: crate::assets::path::CanonicalPath::new("data/global/t.cof").unwrap(),
        dir: 0,
        frame: 0,
        dir64: 0,
    };
    let e = view
        .unit_params(&world, &world.units[&key], &pose)
        .unwrap_err();
    assert!(e.to_string().contains("no local player"), "{e}");
    // The placeholder feed refuses what needs a model input; its open
    // mode is 0 (no base-game UI).
    assert_eq!(NoFeed.open_mode(&world).unwrap(), OpenMode::NONE);
    assert!(NoFeed.player_seed(&world).is_err());
    assert!(NoFeed.unit_position(&world.units[&key]).is_err());
}

#[test]
fn camera_from_the_feed() {
    // Player client (1000, 2000) (16.16: a = 5000, b = 3000), mode 1.
    let mut feed = Feed {
        player: Some(UnitPosition::Moving {
            x16: 5000 << 11,
            y16: 3000 << 11,
        }),
        ..Feed::default()
    };
    let cam = frame_camera(&at_tick(7), &mut feed).unwrap().unwrap();
    assert_eq!(
        cam,
        Camera::new(
            FrameSize::D2RUST,
            OpenMode::new(1).unwrap(),
            ClientPos { x: 1000, y: 2000 },
            (0, 0)
        )
    );
    assert_eq!(
        cam.unit_draw(ClientPos { x: 1000, y: 2000 }, (0, 0)),
        (200, 292)
    );
}

#[test]
fn shake_runs_on_the_tick_time_base() {
    let shake = Shake::start(10, 100, 200, 100).unwrap();
    let mut seed = Seed::default();
    seed.set(77, 666);
    let feed = |start_tick| Feed {
        shake: Some(RunningShake { shake, start_tick }),
        seed,
        ..Feed::default()
    };
    // Tick 10, started at 10: t = 0, a = 0: no draw, no offset.
    let mut f = feed(10);
    assert_eq!(frame_shake(&at_tick(10), &mut f).unwrap(), (0, 0));
    assert_eq!(f.seed, seed);
    // Started at 9: t = 40, a = 4: two draws of roll_range(−4, 8).
    let mut f = feed(9);
    let mut want = seed;
    let o = shake_offsets(4, &mut want);
    assert_eq!(frame_shake(&at_tick(10), &mut f).unwrap(), o);
    assert_eq!(f.seed, want);
    assert!((-4..4).contains(&o.0) && (-4..4).contains(&o.1));
    // Started at 0: t = 400 = t1 + t2 + t3, a = 0 (release end): no draw.
    let mut f = feed(0);
    assert_eq!(frame_shake(&at_tick(10), &mut f).unwrap(), (0, 0));
    assert_eq!(f.seed, seed);
    // t = 440 > 400: ended.
    assert_eq!(frame_shake(&at_tick(11), &mut feed(0)).unwrap(), (0, 0));
    // A shake that starts after the frame's tick is an error, not a guess.
    assert!(frame_shake(&at_tick(3), &mut feed(4)).is_err());
    // t3 = 0: the release row (t = t1 + t2 only) gives a = 0, no draw.
    let mut f = Feed {
        shake: Some(RunningShake {
            shake: Shake::start(10, 0, 40, 0).unwrap(),
            start_tick: 0,
        }),
        seed,
        ..Feed::default()
    };
    assert_eq!(frame_shake(&at_tick(1), &mut f).unwrap(), (0, 0));
    assert_eq!(f.seed, seed);
}

// BlankScreen is the Levels record's +0x218 word: `bClear` clears when
// non-zero; the placeholder feed answers the live data's 1.
#[test]
fn blank_screen_from_the_levels_row() {
    use d2_data::tables::{Levels, Record};
    let mut bytes = vec![0u8; Levels::SIZE];
    assert!(!blank_screen(&Levels::decode(&bytes)));
    bytes[0x218] = 1;
    assert!(blank_screen(&Levels::decode(&bytes)));
    bytes[0x218] = 0;
    bytes[0x21B] = 0x80;
    assert!(blank_screen(&Levels::decode(&bytes)));
    assert!(NoFeed.blank_screen(&ClientWorld::default()).unwrap());
}

// The open mode is UI state only: without the base-game UI a feed answers
// 0; a UI hand-over does not change the placeholder's answer.
#[test]
fn open_mode_without_the_ui_is_0() {
    let world = ClientWorld::default();
    let mut feed = NoFeed;
    assert_eq!(feed.open_mode(&world).unwrap().get(), 0);
    feed.set_ui_open_mode(OpenMode::new(2).unwrap());
    assert_eq!(feed.open_mode(&world).unwrap().get(), 0);
}

// S→C 0x5A code 0x12 starts the shake (6, 4000, 10000, 4000); no other
// code starts a shake (provisional: the only known caller).
#[test]
fn event_0x12_starts_the_shake() {
    let r = event_shake(0x12, 5).unwrap();
    assert_eq!(r.start_tick, 5);
    assert_eq!(
        (
            r.shake.peak,
            r.shake.attack,
            r.shake.sustain,
            r.shake.release
        ),
        (6, 4000, 10000, 4000)
    );
    for code in [0u8, 2, 7, 0xC, 0x11, 0x13, 0xFF] {
        assert_eq!(event_shake(code, 5), None, "code {code:#x}");
    }
    // On the tick time base: 50 ticks in (t = 2000 ms), a = 6·2000/4000 = 3;
    // after t1 + t2 + t3 = 18000 ms (450 ticks) it has ended.
    assert_eq!(r.shake.amplitude(Shake::time_of(50)), Some(3));
    assert_eq!(r.shake.amplitude(Shake::time_of(451)), None);
}

// The default fade clock: GDI reference, every ramp instant; `now` a host
// millisecond count that does not run backwards between reads.
#[test]
fn default_fade_clock_is_instant_on_the_host_clock() {
    let world = ClientWorld::default();
    let a = NoFeed.fade_clock(&world).unwrap();
    let b = NoFeed.fade_clock(&world).unwrap();
    assert!(a.instant && b.instant);
    assert!(b.now.wrapping_sub(a.now) < 60_000);
}

#[test]
fn diablo_appears_shake_at_150_frames_left() {
    assert!(diablo_appears_shake(149, 3).is_none());
    let s = diablo_appears_shake(150, 3).unwrap();
    assert_eq!(s.start_tick, 3);
}
