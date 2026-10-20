//! The camera and sprite placement rules on hand-computed pixel positions, the culling and
//! shake rules, and a CPU-compositor golden scene placed through [`OriginalView`] (exact
//! index framebuffer, with a deliberate one-pixel perturbation that must be caught).

use d2_formats::cof::{Cof, CofLayer};
use d2_formats::palette::{Palette, Rgb};
use d2_sim::rng::Seed;

use super::camera::*;
use super::placement::{self, Cel, RowPlan};
use super::view::*;
use crate::assets::path::CanonicalPath;
use crate::bridge::world::ClientWorld;
use crate::bridge::{ClientUnit, UnitKey};
use crate::composite::{ComponentFrame, ComponentRequest, CompositeError, UnitParams};
use crate::frames::{FrameAnchor, FramePart, FrameSet, FrameSetKey, IndexFrame};
use crate::scene::{self, BlendOp, DrawKey, ItemTag, Rect, ShadeChain};
use crate::ui::{ImageRequest, TextRequest};
use crate::world_view::{
    self, TileDraw, UiRules, UiSprite, UnitPose, ViewAssets, ViewError, ViewRules, VIEW,
};

const W: i32 = 800;
const H: i32 = 600;

fn pos(x: i32, y: i32) -> ClientPos {
    ClientPos { x, y }
}

/// Moving-unit 16.16 coordinates whose client position is `(px, py)`
/// (a − b = 2 px, a + b = 4 py).
fn moving(px: i32, py: i32) -> UnitPosition {
    let a = (2 * px + 4 * py) / 2;
    let b = (4 * py - 2 * px) / 2;
    UnitPosition::Moving {
        x16: (a as u32) << 11,
        y16: (b as u32) << 11,
    }
}

fn camera(mode: u8, player: ClientPos) -> Camera {
    Camera::new(
        FrameSize::D2RUST,
        OpenMode::new(mode).unwrap(),
        player,
        (0, 0),
    )
}

// ---- Client pixels --------------------------------------------------------

#[test]
fn moving_static_and_tile_coordinates() {
    assert_eq!(moving_to_client(10 << 16, 10 << 16), pos(0, 160));
    // a = 320, b = 321: floor shift gives −1 (a truncating `/ 2` would give 0).
    assert_eq!(moving_to_client(320 << 11, 321 << 11), pos(-1, 160));
    // The low 11 bits are dropped (logical shift).
    assert_eq!(
        moving_to_client((320 << 11) | 0x7FF, 321 << 11),
        pos(-1, 160)
    );
    assert_eq!(static_to_client(12, 7), pos(80, 152));
    assert_eq!(
        UnitPosition::Static { sx: 12, sy: 7 }.client(),
        pos(80, 152)
    );
    assert_eq!(cell_origin(3, 1), pos(160, 160));
    assert_eq!(tile_entry(3, 1), (80, 240));
    assert_eq!(moving(1000, 2000).client(), pos(1000, 2000));
}

// ---- Camera origins and unit positions ------------------------------------

#[test]
fn origins_and_player_position_at_800x600() {
    let c = camera(0, pos(1000, 2000));
    assert_eq!((c.tile, c.unit), (pos(600, 1720), pos(600, 1716)));
    assert_eq!(c.unit_draw(pos(1000, 2000), (0, 0)), (400, 292));
    assert_eq!(c.unit_draw(pos(1000, 2000), (3, -2)), (403, 290));

    let v = ViewRect::new(FrameSize::D2RUST, OpenMode::new(0).unwrap());
    assert_eq!(
        (v.left, v.top, v.right, v.bottom, v.shift_x),
        (0, 0, W, H - 40, 0)
    );
    let m1 = camera(1, pos(1000, 2000));
    assert_eq!(
        (m1.view.left, m1.view.shift_x, m1.view.right),
        (-200, -200, 600)
    );
    assert_eq!(m1.unit_draw(pos(1000, 2000), (0, 0)), (200, 292));
    // The tile origin uses right − left = W in every mode.
    assert_eq!(m1.tile, pos(600, 1720));
    let m2 = camera(2, pos(1000, 2000));
    assert_eq!(
        (m2.view.left, m2.view.shift_x, m2.view.right),
        (200, 200, 1000)
    );
    assert_eq!(m2.unit_draw(pos(1000, 2000), (0, 0)), (600, 292));
    assert_eq!(camera(3, pos(1000, 2000)), c);
    assert_eq!(OpenMode::new(4), None);

    let low = Camera::new(FrameSize::LOW, OpenMode::NONE, pos(0, 0), (0, 0));
    assert_eq!(low.tile, pos(-320, -220));
    assert_eq!(low.unit_draw(pos(0, 0), (0, 0)), (320, 232));
}

#[test]
fn shake_offsets_move_both_origins() {
    let c = Camera::new(FrameSize::D2RUST, OpenMode::NONE, pos(1000, 2000), (3, -4));
    assert_eq!((c.tile, c.unit), (pos(603, 1716), pos(603, 1712)));
}

// ---- Tiles ----------------------------------------------------------------

#[test]
fn floor_wall_and_roof_positions() {
    let c = camera(0, pos(1000, 2000));
    let s = pos(640, 1720);
    let floor = c.handed_at(TileList::Floor, s);
    assert_eq!(floor, (40, 0));
    let origin = c.block_origin(TileList::Floor, floor);
    assert_eq!(placement::block_pixel(origin, (0, 0), (0, 0)), (-40, 0));
    let wall = c.handed_at(TileList::Wall, s);
    assert_eq!(wall, (-40, 80));
    let origin = c.block_origin(TileList::Wall, wall);
    assert_eq!(placement::block_pixel(origin, (0, -64), (0, 0)), (-40, 16));
    let roof = TileList::Roof { roof_height: 30 };
    let handed = c.handed_at(roof, s);
    assert_eq!(handed, (40, -30));
    assert_eq!(c.block_origin(roof, handed), (-40, -30));

    // Panel shift: floors move by `left` in the drawer, walls when handed.
    let m1 = camera(1, pos(1000, 2000));
    let floor = m1.handed_at(TileList::Floor, s);
    assert_eq!(m1.block_origin(TileList::Floor, floor), (-240, 0));
    let wall = m1.handed_at(TileList::Wall, s);
    assert_eq!(m1.block_origin(TileList::Wall, wall), (-240, 80));
    assert_eq!(
        m1.tile_handed(TileList::Floor, 3, 1),
        (160 - 600, 160 - 1720)
    );
}

#[test]
fn units_sit_12_rows_below_the_floor_vertex() {
    let c = camera(0, pos(1000, 2000));
    let s = cell_origin(30, 20);
    let floor_y = c.handed_at(TileList::Floor, s).1;
    let unit_y = c.unit_draw(s, (0, 0)).1;
    assert_eq!(unit_y - floor_y, 12);
}

// ---- Culling --------------------------------------------------------------

#[test]
fn view_culling() {
    let c = camera(0, pos(0, 0));
    assert!(!c.floor_roof_visible((-81, 0)));
    assert!(c.floor_roof_visible((-80, 0)));
    assert!(!c.floor_roof_visible((0, 553)));
    assert!(c.floor_roof_visible((0, 552)));
    assert!(c.floor_roof_visible((879, -80)));
    assert!(!c.floor_roof_visible((880, 0)));
    assert!(!c.floor_roof_visible((0, -81)));

    assert!(c.wall_block_visible(-32, -32));
    assert!(!c.wall_block_visible(-33, 0));
    assert!(!c.wall_block_visible(800, 0));
    assert!(c.wall_block_visible(799, 631));
    assert!(!c.wall_block_visible(0, 632));
    let m1 = camera(1, pos(0, 0));
    assert!(m1.wall_block_visible(399, 0));
    assert!(!m1.wall_block_visible(400, 0));
    assert!(m1.wall_block_visible(-32, 0));
    let m2 = camera(2, pos(0, 0));
    assert!(!m2.wall_block_visible(367, 0));
    assert!(m2.wall_block_visible(368, 0));
    assert!(!m2.wall_block_visible(800, 0));
}

// ---- Shake and time base --------------------------------------------------

#[test]
fn shake_envelope_and_offsets() {
    assert_eq!(Shake::start(10, 100, 0, 100), None);
    let s = Shake::start(10, 100, 200, 100).unwrap();
    let a = |t| s.amplitude(t);
    assert_eq!(a(0), Some(0));
    assert_eq!(a(50), Some(5));
    assert_eq!(a(99), Some(9));
    assert_eq!(a(100), Some(10));
    assert_eq!(a(299), Some(10));
    assert_eq!(a(300), Some(10));
    assert_eq!(a(350), Some(5));
    assert_eq!(a(399), Some(0));
    assert_eq!(a(400), Some(0));
    assert_eq!(a(401), None);
    assert_eq!(Shake::time_of(0), 0);
    assert_eq!(Shake::time_of(3), 120);

    // Offsets: two roll_range(−a, 2a) draws, x then y; range [−a, a − 1].
    let mut seed = Seed::new(0x1234_5678, 666);
    let mut copy = seed;
    let (dx, dy) = shake_offsets(5, &mut seed);
    assert_eq!(dx, copy.roll_range(-5, 10));
    assert_eq!(dy, copy.roll_range(-5, 10));
    assert_eq!(seed, copy);
    let (mut lo, mut hi) = (0, 0);
    for _ in 0..2000 {
        let (dx, dy) = shake_offsets(5, &mut seed);
        lo = lo.min(dx.min(dy));
        hi = hi.max(dx.max(dy));
    }
    assert_eq!((lo, hi), (-5, 4));
    // a = 0 draws nothing.
    let before = seed;
    assert_eq!(shake_offsets(0, &mut seed), (0, 0));
    assert_eq!(seed, before);
}

// The shake envelope's 32-bit arithmetic: a zero release time gives a = 0 for the
// one frame at t = t1 + t2 (no division, no draw), then the shake ends;
// the attack product keeps its low 32 bits.
#[test]
fn shake_envelope_is_unsigned_32_bit() {
    let no_release = Shake::start(10, 100, 200, 0).unwrap();
    assert_eq!(no_release.amplitude(299), Some(10));
    assert_eq!(no_release.amplitude(300), Some(0));
    assert_eq!(no_release.amplitude(301), None);
    // a = 0 while not ended: no draw, the origins get no offset.
    let mut seed = Seed::new(7, 666);
    let before = seed;
    let offsets = shake_offsets(0, &mut seed);
    assert_eq!((offsets, seed), ((0, 0), before));
    let c = Camera::new(FrameSize::D2RUST, OpenMode::NONE, pos(0, 0), offsets);
    assert_eq!((c.tile, c.unit), (pos(-400, -280), pos(-400, -284)));

    // A = 0x10000, t = 0x10000 in the attack: product 2^32 keeps 0.
    let wide = Shake::start(0x1_0000, 0x2_0000, 1, 1).unwrap();
    assert_eq!(wide.amplitude(0x1_0000), Some(0));
    // t = 0x18000: 0x18000 × 0x10000 mod 2^32 = 2^31; 2^31 / 2^17 = 0x4000.
    assert_eq!(wide.amplitude(0x1_8000), Some(0x4000));
    // Release product wraps the same way: t1 + t2 + t3 − t = 0x10000.
    let rel = Shake::start(0x1_0000, 1, 1, 0x2_0000).unwrap();
    assert_eq!(rel.amplitude(2), Some(0));
    // Sustain returns the peak unchanged, whatever its size.
    assert_eq!(rel.amplitude(1), Some(0x1_0000));
}

// ---- Sprite placement -----------------------------------------------------

#[test]
fn dc6_cels_both_orientations() {
    let up = Cel::dc6(0, 3, 2, 5, 10);
    assert_eq!(up.columns(100), (105, 107));
    assert_eq!(up.rows(200), (209, 210));
    let f = IndexFrame::new(3, 2, 5, 10, vec![1; 6])
        .unwrap()
        .with_anchor(FrameAnchor::Bottom);
    assert_eq!(placement::draw_position(&f, 100, 200), (105, 209));

    let down = Cel::dc6(1, 3, 2, 5, 10);
    assert_eq!(down.rows(200), (210, 211));
    // First stored (encoded) row on 210.
    assert_eq!(down.row_plan(200, H).screen_row(0), 210);
    assert_eq!(up.row_plan(200, H).screen_row(0), 210);
    let f = f.with_anchor(FrameAnchor::TopDown);
    assert_eq!(placement::draw_position(&f, 100, 200), (105, 210));
    // Only bit 0 of the orientation word counts.
    assert!(!Cel::dc6(2, 3, 2, 5, 10).top_down);
    assert!(Cel::dc6(3, 3, 2, 5, 10).top_down);
}

#[test]
fn dcc_frame_box() {
    // w 8, h 20, x offset −4, y offset −1: box x −4…3, y −20…−1.
    let cel = Cel::dcc(-4, -1, 8, 20);
    assert_eq!(cel.columns(400), (396, 403));
    assert_eq!(cel.rows(292), (272, 291));
    let f = IndexFrame::new(8, 20, -4, -20, vec![1; 160]).unwrap();
    assert_eq!(f.anchor, FrameAnchor::Top);
    assert_eq!(placement::draw_position(&f, 400, 292), (396, 272));
    let p = placement::place(&f, 400, 292, Rect::FRAME);
    assert_eq!((p.x, p.y, p.clip), (396, 272, Some(Rect::FRAME)));
}

#[test]
fn row_clipping() {
    // h 5, yoff 0 at Y = 2: the bottom 3 stored rows on rows 2, 1, 0.
    let p = Cel::dc6(0, 1, 5, 0, 0).row_plan(2, H);
    assert_eq!(
        p,
        RowPlan {
            skip: 0,
            start: 2,
            count: 3,
            top_down: false
        }
    );
    assert_eq!(
        (0..3).map(|k| p.screen_row(k)).collect::<Vec<_>>(),
        [2, 1, 0]
    );
    // h 4, yoff 0 at Y = H + 1: 2 bottom rows skipped, rows H − 1, H − 2.
    let p = Cel::dc6(0, 1, 4, 0, 0).row_plan(H + 1, H);
    assert_eq!((p.skip, p.start, p.count), (2, i64::from(H - 1), 2));
    assert_eq!(
        (2..4).map(|k| p.screen_row(k)).collect::<Vec<_>>(),
        [i64::from(H - 1), i64::from(H - 2)]
    );
    // Wholly above or below: nothing.
    assert_eq!(Cel::dc6(0, 1, 4, 0, 0).row_plan(-1, H).count, 0);
    assert_eq!(Cel::dc6(0, 1, 4, 0, 0).row_plan(H + 4, H).count, 0);
    // A bottom-up cel keeps the frame clip: the compositor's clip draws
    // the same rows as the plan.
    let f = IndexFrame::new(1, 5, 0, 0, vec![1; 5])
        .unwrap()
        .with_anchor(FrameAnchor::Bottom);
    let placed = placement::place(&f, 0, 2, Rect::FRAME);
    assert_eq!((placed.y, placed.clip), (-2, Some(Rect::FRAME)));
}

#[test]
fn top_down_cels_clip_as_bottom_up() {
    let f = IndexFrame::new(2, 10, 0, 0, vec![1; 20])
        .unwrap()
        .with_anchor(FrameAnchor::TopDown);
    // Inside the frame but Y + yoff + 1 = 6 < h: only 6 rows drawn.
    let p = placement::place(&f, 10, 5, Rect::FRAME);
    assert_eq!((p.x, p.y, p.clip), (10, 5, Some(Rect::new(0, 5, 800, 6))));
    assert!(!p.same_as_frame(2, 10, Rect::FRAME));
    // Far enough down: unchanged.
    let p = placement::place(&f, 10, 100, Rect::FRAME);
    assert!(p.same_as_frame(2, 10, Rect::FRAME));
    // Starting below the frame: top rows skipped, image row 2 on H − 1.
    let f = IndexFrame::new(1, 4, 0, 0, vec![1; 4])
        .unwrap()
        .with_anchor(FrameAnchor::TopDown);
    let p = placement::place(&f, 0, H + 1, Rect::FRAME);
    assert_eq!((p.y, p.clip), (H - 3, Some(Rect::new(0, H - 1, 800, 1))));
    // Above the top: nothing at all.
    assert_eq!(placement::place(&f, 0, -1, Rect::FRAME).clip, None);
    // First row at H − 2: rows H and H + 1 would fall past the surface;
    // D2RUST keeps the two rows inside the frame.
    let p = placement::place(&f, 0, H - 2, Rect::FRAME);
    assert_eq!((p.y, p.clip), (H - 2, Some(Rect::new(0, H - 2, 800, 2))));
}

#[test]
fn dt1_block_position() {
    assert_eq!(
        placement::block_pixel((0, 300), (32, -64), (0, 0)),
        (32, 236)
    );
    assert_eq!(
        placement::block_pixel((0, 300), (32, -64), (5, 7)),
        (37, 243)
    );
}

// ---- OriginalView: tiles, wall culling, unit placement, golden ------------

const COF: &str = "data/global/tst/tst.cof";

fn unit_key(guid: u32) -> FrameSetKey {
    FrameSetKey::new(format!("data/global/tst/u{guid}.dcc"), FramePart::Dir(0)).unwrap()
}

fn tile_key(name: &str) -> FrameSetKey {
    FrameSetKey::new(format!("data/global/tiles/{name}.dt1"), FramePart::Tile(0)).unwrap()
}

fn one(frame: IndexFrame) -> FrameSet {
    FrameSet {
        frames: vec![frame],
    }
}

fn filled(w: u32, h: u32, x: i32, y: i32, v: u8) -> IndexFrame {
    IndexFrame::new(w, h, x, y, vec![v; (w * h) as usize]).unwrap()
}

fn cof() -> Cof {
    Cof {
        layers_count: 1,
        frames: 1,
        directions: 1,
        version: 20,
        unknown: [0; 4],
        x_min: 0,
        x_max: 0,
        y_min: 0,
        y_max: 0,
        animation_rate: 256,
        layers: vec![CofLayer {
            component: 0,
            shadow: 0,
            selectable: 1,
            override_translucency: 0,
            new_translucency: 0,
            weapon_class: *b"hth\0",
        }],
        events: vec![0],
        event_padding: Vec::new(),
        draw_order: vec![0],
    }
}

/// The hooks owned by other rule modules, answered by fixture: every unit shows
/// the one-layer COF, its own frame set, draw key (2, guid, 0, slot).
struct Fixture;

impl ViewRules for Fixture {
    fn tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<TileDraw>, ViewError> {
        unreachable!("OriginalView answers tiles")
    }

    fn unit_pose(&self, _: &ClientWorld, _: &ClientUnit) -> Result<Option<UnitPose>, ViewError> {
        Ok(Some(UnitPose {
            sub_frame: 0,
            cof: CanonicalPath::new(COF).unwrap(),
            dir: 0,
            frame: 0,
            dir64: 0,
        }))
    }

    fn unit_params(
        &self,
        _: &ClientWorld,
        u: &ClientUnit,
        _: &UnitPose,
    ) -> Result<UnitParams, ViewError> {
        Ok(UnitParams {
            pass: 2,
            major: u.key.guid,
            minor: 0,
            // Replaced by the frame clip.
            clip: Rect::new(0, 0, 1, 1),
            tag: ItemTag::Unit(u.key.guid),
        })
    }

    fn component_frame(
        &self,
        u: &ClientUnit,
        _: &UnitPose,
        _: &ComponentRequest<'_>,
    ) -> Result<ComponentFrame, CompositeError> {
        Ok(ComponentFrame {
            set: unit_key(u.key.guid),
            index: 0,
        })
    }

    fn place(
        &self,
        _: &ClientUnit,
        _: &UnitPose,
        _: &ComponentRequest<'_>,
        _: &IndexFrame,
    ) -> Result<(i32, i32), CompositeError> {
        unreachable!("OriginalView answers place")
    }

    fn shade(
        &self,
        _: &ClientUnit,
        _: &ComponentRequest<'_>,
    ) -> Result<ShadeChain, CompositeError> {
        Ok(ShadeChain::EMPTY)
    }

    fn blend(&self, _: &ClientUnit, _: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError> {
        Ok(BlendOp::Opaque)
    }
}

impl UiRules for Fixture {
    fn ui_image(&self, _: &ImageRequest, _: &ViewAssets) -> Result<UiSprite, ViewError> {
        unreachable!("no UI in these scenes")
    }

    fn ui_text(&self, _: &TextRequest, _: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        unreachable!("no UI in these scenes")
    }

    fn ui_pass(&self) -> Result<u32, ViewError> {
        Ok(9)
    }
}

/// Unit positions by GUID and the map tiles of the scene.
struct Scene {
    units: Vec<(u32, UnitPosition)>,
    tiles: Vec<MapTile>,
}

impl ViewSource for Scene {
    fn unit_position(&self, unit: &ClientUnit) -> Result<UnitPosition, String> {
        self.units
            .iter()
            .find(|(g, _)| *g == unit.key.guid)
            .map(|(_, p)| *p)
            .ok_or_else(|| format!("no position for guid {}", unit.key.guid))
    }

    fn unit_offset(&self, _: &ClientUnit, _: &UnitPose) -> Result<(i32, i32), String> {
        Ok((0, 0))
    }

    fn map_tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<MapTile>, ViewError> {
        Ok(self.tiles.clone())
    }
}

fn map_tile(cell: (i32, i32), list: TileList, name: &str, blocks: Vec<BlockRect>) -> MapTile {
    MapTile {
        cell,
        list,
        frame: ComponentFrame {
            set: tile_key(name),
            index: 0,
        },
        blocks,
        shade: ShadeChain::EMPTY,
        blend: BlendOp::Opaque,
        key: DrawKey::default(),
    }
}

fn palette() -> Palette {
    let mut p = Palette {
        colors: [Rgb::default(); 256],
    };
    for (i, c) in p.colors.iter_mut().enumerate() {
        c.r = i as u8;
    }
    p
}

const PLAYER: u32 = 1;
const OBJECT: u32 = 2;

/// Player (moving, DCC box) at client (1000, 2000), an object (static,
/// bottom-up DC6) at subtile (163, 93), a floor and a wall tile on cell
/// (26, 18) and a floor on cell (0, 0) (culled).
fn golden_scene() -> (ClientWorld, ViewAssets, Scene) {
    golden_scene_with(one(filled(3, 2, 5, 10, 6).with_anchor(FrameAnchor::Bottom)))
}

/// [`golden_scene`] with the object's frame set given (the frame store
/// takes each key once).
fn golden_scene_with(object: FrameSet) -> (ClientWorld, ViewAssets, Scene) {
    let mut world = ClientWorld::default();
    for (unit_type, guid) in [(0, PLAYER), (2, OBJECT)] {
        let key = UnitKey { unit_type, guid };
        world.units.insert(key, ClientUnit::new(key));
    }
    let mut assets = ViewAssets::new(palette());
    assets.cofs.insert(CanonicalPath::new(COF).unwrap(), cof());
    assets
        .frames
        .insert(unit_key(PLAYER), one(filled(8, 20, -4, -20, 5)))
        .unwrap();
    assets.frames.insert(unit_key(OBJECT), object).unwrap();
    assets
        .frames
        .insert(tile_key("floor"), one(filled(4, 2, 80, 0, 3)))
        .unwrap();
    assets
        .frames
        .insert(tile_key("wall"), one(filled(2, 2, 64, -64, 4)))
        .unwrap();
    let wall_block = BlockRect {
        x: 64,
        y: -64,
        width: 2,
        height: 2,
    };
    let scene = Scene {
        units: vec![
            (PLAYER, moving(1000, 2000)),
            (OBJECT, UnitPosition::Static { sx: 163, sy: 93 }),
        ],
        tiles: vec![
            map_tile((26, 18), TileList::Floor, "floor", Vec::new()),
            map_tile((0, 0), TileList::Floor, "floor", Vec::new()),
            map_tile((26, 18), TileList::Wall, "wall", vec![wall_block]),
        ],
    };
    (world, assets, scene)
}

/// The tile drawers' clip at 800 × 600: rows 0–552.
const PLAY: Rect = Rect {
    x: 0,
    y: 0,
    width: 800,
    height: 553,
};

fn compose(world: &ClientWorld, assets: &ViewAssets, scene: &Scene, player: ClientPos) -> Vec<u8> {
    let view = OriginalView::new(camera(0, player), &Fixture, scene);
    let frame = world_view::build(world, &[], &view, assets).unwrap();
    for item in &frame.items {
        // Tiles clip at the play area (the rows above the control panel).
        assert!(
            item.clip == Rect::FRAME || item.clip == PLAY,
            "{:?}",
            item.clip
        );
    }
    scene::compose(&frame.items, &assets.frames, &assets.maps, VIEW).unwrap()
}

fn paint(buf: &mut [u8], x: i32, y: i32, w: i32, h: i32, v: u8) {
    for yy in y..y + h {
        for xx in x..x + w {
            buf[(yy * W + xx) as usize] = v;
        }
    }
}

#[test]
fn cpu_golden_scene_through_original_view() {
    let (world, assets, scene) = golden_scene();
    let view = OriginalView::new(camera(0, pos(1000, 2000)), &Fixture, &scene);
    let frame = world_view::build(&world, &[], &view, &assets).unwrap();
    // The (0, 0) floor is culled: 2 tiles + 2 units.
    assert_eq!(frame.items.len(), 4);

    let got = compose(&world, &assets, &scene, pos(1000, 2000));
    let mut want = vec![0u8; (W * H) as usize];
    // Floor: handed (40, 40), blocks at (−40, 40), image x0 = 80.
    paint(&mut want, 40, 40, 4, 2, 3);
    // Wall: handed (−40, 120), image (64, −64) → (24, 56).
    paint(&mut want, 24, 56, 2, 2, 4);
    // Player at (400, 292): DCC box (−4, −20) → (396, 272), 8 × 20.
    paint(&mut want, 396, 272, 8, 20, 5);
    // Object: client (1120, 2048) → (520, 340); DC6 bottom row 350.
    paint(&mut want, 525, 349, 3, 2, 6);
    assert_eq!(got, want);
}

#[test]
fn units_failing_the_cof_box_pre_test_are_not_drawn() {
    let (world, mut assets, scene) = golden_scene();
    let view = OriginalView::new(camera(0, pos(1000, 2000)), &Fixture, &scene);
    let path = CanonicalPath::new(COF).unwrap();
    // Final (X, Y): player (400, 292), object (520, 340); W − 1 = 799.
    // x_min 398: 798 < 799 for the player, 918 for the object.
    assets.cofs.get_mut(&path).unwrap().x_min = 398;
    let f = world_view::build(&world, &[], &view, &assets).unwrap();
    assert_eq!((f.units_drawn, f.units_hidden), (1, 1));
    // x_min 399: 799 is not < 799.
    assets.cofs.get_mut(&path).unwrap().x_min = 399;
    let f = world_view::build(&world, &[], &view, &assets).unwrap();
    assert_eq!((f.units_drawn, f.units_hidden), (0, 2));
    // y_max + Y ≥ 0: −292 keeps the player, −293 drops it.
    let c = assets.cofs.get_mut(&path).unwrap();
    (c.x_min, c.y_max) = (0, -292);
    let f = world_view::build(&world, &[], &view, &assets).unwrap();
    assert_eq!((f.units_drawn, f.units_hidden), (2, 0));
    assets.cofs.get_mut(&path).unwrap().y_max = -293;
    let f = world_view::build(&world, &[], &view, &assets).unwrap();
    assert_eq!((f.units_drawn, f.units_hidden), (1, 1));
}

// A deliberate one-pixel perturbation: moving the player by one client pixel changes exactly the
// non-player pixels.
#[test]
fn golden_scene_catches_a_one_pixel_camera_move() {
    let (world, assets, mut scene) = golden_scene();
    let base = compose(&world, &assets, &scene, pos(1000, 2000));
    scene.units[0].1 = moving(1001, 2000);
    let moved = compose(&world, &assets, &scene, pos(1001, 2000));
    let changed: Vec<usize> = (0..base.len()).filter(|&i| base[i] != moved[i]).collect();
    // Floor, wall and object columns shift left by one: each loses its
    // right column and gains one on the left (2 + 2 + 2 rows × 2 sides).
    assert_eq!(changed.len(), 12);
    let player = |i: usize| {
        let (x, y) = ((i as i32) % W, (i as i32) / W);
        (396..404).contains(&x) && (272..292).contains(&y)
    };
    assert!(changed.iter().all(|&i| !player(i)));
}

#[test]
fn wall_blocks_culled_in_mode_2() {
    // Player (0, 0), mode 2: cx_t = −400, left = 200; wall cell (0, 0)
    // handed (520, 360).
    let c = camera(2, pos(0, 0));
    assert_eq!(c.tile_handed(TileList::Wall, 0, 0), (520, 360));
    let block = |x| BlockRect {
        x,
        y: 0,
        width: 32,
        height: 32,
    };
    let scene = Scene {
        units: Vec::new(),
        tiles: Vec::new(),
    };
    let view = OriginalView::new(c, &Fixture, &scene);
    let image = filled(64, 32, -184, 0, 1);
    let tile = map_tile(
        (0, 0),
        TileList::Wall,
        "wall",
        vec![block(-184), block(-152)],
    );
    // Block 0 at x 336 (< 368: culled), block 1 at 368 (kept).
    let d = view.tile(&tile, &image).unwrap().unwrap();
    assert_eq!((d.x, d.y, d.clip), (336, 360, Rect::new(368, 360, 32, 32)));
    // Both culled: the drawer is still called, with no pixel (a call-only
    // draw).
    let tile = map_tile(
        (0, 0),
        TileList::Wall,
        "wall",
        vec![block(-184), block(-216)],
    );
    let d = view.tile(&tile, &image).unwrap().unwrap();
    assert!(d.is_call_only());
    assert_eq!((d.x, d.y), (336, 360));
    // A culled block overlapping the kept ones cannot be one clip.
    let tile = map_tile(
        (0, 0),
        TileList::Wall,
        "wall",
        vec![block(-152), block(-170)],
    );
    assert!(view.tile(&tile, &image).is_err());
    // Mode 0: nothing culled, frame clip.
    let view = OriginalView::new(camera(0, pos(0, 0)), &Fixture, &scene);
    let tile = map_tile(
        (0, 0),
        TileList::Wall,
        "wall",
        vec![block(-184), block(-152)],
    );
    assert_eq!(view.tile(&tile, &image).unwrap().unwrap().clip, PLAY);
}

// The culling vector: mode 1, W = 800, blocks at screen x 399 and
// 400: 399 is drawn whole (pixels 399–430, past the bound), 400 is
// skipped; the translucent wall drawer culls the same way.
#[test]
fn wall_blocks_culled_in_mode_1_lit_and_translucent() {
    // Mode 1: left = −200, cx_t = −400; wall cell (0, 0) handed (120, 360).
    let c = camera(1, pos(0, 0));
    assert_eq!(c.tile_handed(TileList::Wall, 0, 0), (120, 360));
    let block = |x| BlockRect {
        x,
        y: 0,
        width: 32,
        height: 32,
    };
    let scene = Scene {
        units: Vec::new(),
        tiles: Vec::new(),
    };
    let view = OriginalView::new(c, &Fixture, &scene);
    let image = filled(64, 32, 279, 0, 1);
    for blend in [BlendOp::Opaque, BlendOp::IndexTable(scene::MapId(0))] {
        let mut tile = map_tile((0, 0), TileList::Wall, "wall", vec![block(279), block(311)]);
        tile.blend = blend;
        // Block 0 at x 399 (kept), block 1 at 431 (culled).
        let d = view.tile(&tile, &image).unwrap().unwrap();
        assert_eq!((d.x, d.clip), (399, Rect::new(399, 360, 32, 32)));
        // A block at x 400 alone: skipped (the call stays, without pixels).
        let mut tile = map_tile((0, 0), TileList::Wall, "wall", vec![block(280)]);
        tile.blend = blend;
        let image = filled(32, 32, 280, 0, 1);
        assert!(view.tile(&tile, &image).unwrap().unwrap().is_call_only());
    }
}

// Floors and roofs go through the floor drawer: only the whole-tile view
// test, no per-block culling, the frame as the pixel clip.
#[test]
fn roofs_and_floors_are_not_culled_per_block() {
    let c = camera(2, pos(0, 0));
    let scene = Scene {
        units: Vec::new(),
        tiles: Vec::new(),
    };
    let view = OriginalView::new(c, &Fixture, &scene);
    // Blocks the wall drawer would cull in mode 2 (x < 368).
    let blocks = vec![BlockRect {
        x: 0,
        y: 0,
        width: 32,
        height: 32,
    }];
    let image = filled(32, 32, 0, 0, 1);
    for list in [TileList::Floor, TileList::Roof { roof_height: 40 }] {
        let tile = map_tile((-1, 1), list, "floor", blocks.clone());
        let handed = c.tile_handed(list, -1, 1);
        let origin = c.block_origin(list, handed);
        // −80 and the panel shift (+200) on X.
        assert_eq!(origin.0, handed.0 - 80 + 200);
        let d = view.tile(&tile, &image).unwrap().unwrap();
        assert_eq!((d.x, d.y, d.clip), (origin.0, origin.1, PLAY));
        assert!(!c.wall_block_visible(d.x, d.y));
    }
}

// Units have no view-rectangle test: a unit far outside the view whose
// COF box still reaches into the frame is
// placed; its pixels are cut by the frame clip only.
#[test]
fn units_are_not_culled_by_the_view() {
    let (world, mut assets, mut scene) = golden_scene();
    scene.tiles.clear();
    // A COF box reaching 5,000 pixels left of the unit.
    let path = CanonicalPath::new(COF).unwrap();
    assets.cofs.get_mut(&path).unwrap().x_min = -5_000;
    // The object 2,000 client pixels right of the player.
    scene.units[1].1 = UnitPosition::Static {
        sx: 163 + 125,
        sy: 93 - 125,
    };
    let view = OriginalView::new(camera(0, pos(1000, 2000)), &Fixture, &scene);
    let frame = world_view::build(&world, &[], &view, &assets).unwrap();
    assert_eq!(frame.items.len(), 2);
    let object = frame
        .items
        .iter()
        .find(|i| i.tag == ItemTag::Unit(OBJECT))
        .unwrap();
    assert_eq!((object.x, object.clip), (520 + 4000 + 5, Rect::FRAME));
}

#[test]
fn a_cut_top_down_unit_cel_is_an_error() {
    // The object's frame is top-down near the top edge: rows cut.
    let (world, assets, mut scene) = golden_scene_with(one(
        filled(3, 20, 0, -340, 6).with_anchor(FrameAnchor::TopDown)
    ));
    scene.tiles.clear();
    let view = OriginalView::new(camera(0, pos(1000, 2000)), &Fixture, &scene);
    let err = world_view::build(&world, &[], &view, &assets).unwrap_err();
    assert!(matches!(err, ViewError::Unit { guid: OBJECT, .. }), "{err}");
}

// The play preview's edge clip (decision D1, D2RUST's own, unverified): the
// same cut cel is placed and left to the frame clip; the strict view
// above still fails.
#[test]
fn the_preview_edge_clip_places_a_cut_cel() {
    let (world, assets, mut scene) = golden_scene_with(one(
        filled(3, 20, 0, -340, 6).with_anchor(FrameAnchor::TopDown)
    ));
    scene.tiles.clear();
    let view = OriginalView::new(camera(0, pos(1000, 2000)), &Fixture, &scene).with_edge_clip(true);
    let frame = world_view::build(&world, &[], &view, &assets).unwrap();
    let object = frame
        .items
        .iter()
        .find(|i| i.tag == ItemTag::Unit(OBJECT))
        .expect("the cut cel is drawn");
    assert_eq!(object.clip, Rect::FRAME);
}

// Per-block shade (each 32-pixel block has its own light): one draw per
// block clipped to it, the gradient moved to the block's screen position;
// a culled block draws nothing.
#[test]
fn tile_blocks_draw_one_item_per_block() {
    use crate::scene::{GradientKind, LightGradient, MapId};
    let block = |x| BlockRect {
        x,
        y: 0,
        width: 32,
        height: 32,
    };
    let scene = Scene {
        units: Vec::new(),
        tiles: Vec::new(),
    };
    let image = filled(64, 32, -184, 0, 1);
    let tile = map_tile(
        (0, 0),
        TileList::Wall,
        "wall",
        vec![block(-184), block(-152)],
    );
    let gradient = LightGradient {
        kind: GradientKind::Wall,
        x: 0,
        y: 0,
        corners: [0, 255, 255, 0],
        light0: MapId(3),
    };
    let shades = [
        BlockShade {
            block: block(-184),
            shade: ShadeChain::EMPTY,
            blend: BlendOp::Opaque,
        },
        BlockShade {
            block: block(-152),
            shade: ShadeChain::EMPTY.with_gradient(gradient),
            blend: BlendOp::IndexTableSrcRow(MapId(9)),
        },
    ];
    // Mode 0: both kept; block 0 at x 336, block 1 at 368, y 360.
    let view = OriginalView::new(camera(0, pos(0, 0)), &Fixture, &scene);
    let whole = view.tile(&tile, &image).unwrap().unwrap();
    let draws = view.tile_draws(&tile, &image, &shades).unwrap();
    assert_eq!(draws.len(), 2);
    assert_eq!(draws[0].clip, Rect::new(whole.x, 360, 32, 32));
    assert_eq!(draws[0].shade, ShadeChain::EMPTY);
    assert_eq!(draws[1].clip, Rect::new(whole.x + 32, 360, 32, 32));
    assert_eq!(draws[1].blend, BlendOp::IndexTableSrcRow(MapId(9)));
    let g = draws[1].shade.gradient().unwrap();
    assert_eq!((g.x, g.y), (whole.x + 32, 360));
    // Each block draws its own image (frame 1 + i of
    // the tile's set) at its own position, so it lights only its pixels.
    let at: Vec<_> = draws.iter().map(|d| (d.x, d.y, d.frame.index)).collect();
    assert_eq!(at, vec![(whole.x, 360, 1), (whole.x + 32, 360, 2)]);
    assert!(draws.iter().all(|d| d.frame.set == whole.frame.set));
    // A shade of a block the tile does not have, in order: an error.
    let mut wrong = shades;
    wrong.reverse();
    assert!(view.tile_draws(&tile, &image, &wrong).is_err());
    // A floor gradient shade shorter than its block (rows 0…14, the floor
    // gradient) names that block; the draw covers the shade's rows.
    let mut short = shades;
    short[1].block.height = 15;
    let draws = view.tile_draws(&tile, &image, &short).unwrap();
    assert_eq!(draws[1].clip, Rect::new(whole.x + 32, 360, 32, 15));
    assert_eq!(draws[1].frame.index, 2);
    // Taller than the block, or at another x: not that block.
    let mut tall = shades;
    tall[1].block.height = 33;
    assert!(view.tile_draws(&tile, &image, &tall).is_err());
    // A left-out block (wall alpha) keeps the others' indexes.
    let draws = view.tile_draws(&tile, &image, &shades[1..]).unwrap();
    assert_eq!((draws[0].x, draws[0].frame.index), (whole.x + 32, 2));
    // No per-block shade: the whole tile, unchanged.
    assert_eq!(view.tile_draws(&tile, &image, &[]).unwrap(), vec![whole]);
    // Mode 2: block 0 culled (x 336 < 368), only block 1 drawn.
    let view = OriginalView::new(camera(2, pos(0, 0)), &Fixture, &scene);
    let draws = view.tile_draws(&tile, &image, &shades).unwrap();
    assert_eq!(draws.len(), 1);
    assert_eq!(draws[0].clip, Rect::new(368, 360, 32, 32));
    assert_eq!((draws[0].x, draws[0].frame.index), (368, 2));
}

// A floor handed at X = −80 passes the whole-tile view test but its blocks
// all lie left of the frame: the floor drawer is still called (a call-only
// draw, which the town at arrival produces at X −80) and draws nothing; one
// outside the view rectangle is not called.
#[test]
fn an_off_frame_floor_inside_the_view_rectangle_is_a_call() {
    use crate::rules::view::BlockShade;
    let c = camera(0, pos(0, 0));
    let scene = Scene {
        units: Vec::new(),
        tiles: Vec::new(),
    };
    let view = OriginalView::new(c, &Fixture, &scene);
    let image = filled(160, 80, 0, 0, 1);
    let cell = (-50..50)
        .flat_map(|x| (-50..50).map(move |y| (x, y)))
        .find(|&(x, y)| {
            let h = c.tile_handed(TileList::Floor, x, y);
            h.0 == -80 && (0..520).contains(&h.1)
        })
        .expect("a cell at X -80");
    let block = |x, y| BlockRect {
        x,
        y,
        width: 32,
        height: 15,
    };
    let blocks = vec![block(0, 0), block(64, 32), block(128, 64)];
    let tile = map_tile(cell, TileList::Floor, "floor", blocks.clone());
    let shades: Vec<_> = blocks
        .iter()
        .map(|&b| BlockShade {
            block: b,
            shade: ShadeChain::EMPTY,
            blend: BlendOp::Opaque,
        })
        .collect();
    let draws = view.tile_draws(&tile, &image, &shades).unwrap();
    assert_eq!(draws.len(), 1, "{draws:?}");
    assert!(draws[0].is_call_only());
    assert_eq!(draws[0].key, tile.key);
    // A block reaching into the frame: drawn, no call-only draw.
    let mut wide = tile.clone();
    wide.blocks.push(block(160, 0));
    let mut more = shades.clone();
    more.push(BlockShade {
        block: block(160, 0),
        shade: ShadeChain::EMPTY,
        blend: BlendOp::Opaque,
    });
    let draws = view.tile_draws(&wide, &image, &more).unwrap();
    assert_eq!(draws.len(), 1);
    assert!(!draws[0].is_call_only());
    // Outside the view rectangle: no call at all.
    let far = (-50..50)
        .flat_map(|x| (-50..50).map(move |y| (x, y)))
        .find(|&(x, y)| !c.floor_roof_visible(c.tile_handed(TileList::Floor, x, y)))
        .expect("a culled cell");
    let tile = map_tile(far, TileList::Floor, "floor", blocks);
    assert!(view.tile_draws(&tile, &image, &shades).unwrap().is_empty());
}
