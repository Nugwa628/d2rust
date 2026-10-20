//! The draw order wired into the world view: [`ordered_source`] reads the
//! map-tile feed of a [`ViewFeed`], orders the frame, turns each tile item
//! into a [`MapTile`] (its art through [`ViewFeed::tile_art`]) and wraps
//! the feed in an [`OrderedSource`], the [`ViewSource`] whose `map_tiles`
//! are the ordered tiles and whose `unit_slot` gives each unit's draw key
//! (or not drawn). The frame's weather (the water-floor spawns and passes
//! 4 and 9) is rolled here too, on the seed the feed lends.

use std::collections::BTreeMap;

use crate::bridge::world::{ClientWorld, UnitKey};
use crate::bridge::ClientUnit;
use crate::composite::ComponentFrame;
use crate::scene::{BlendOp, DrawKey, ShadeChain};
use crate::world_view::{UnitPose, ViewAssets, ViewError, ViewFeed};
use d2_sim::rng::Seed;

use super::sky::{SkyFrame, SkyPasses};
use super::weather::{FloorContext, LocalPlayer, Pass9Input, ThunderSound, Weather};

use super::super::camera::{Camera, ClientPos, OpenMode, TileList, UnitPosition};
use super::super::view::{BlockRect, MapTile, ViewSource};
use super::{
    mark_drawn, order_frame, sets_drawn_flag, FrameOrder, OrderKey, Ordered, OrderedTile,
    TileArray, TileKind, UnitSlot, SPEC,
};

/// The art of one ordered tile, as the tile rules answer it: the DT1
/// frame (the room's tile library maps the record to a
/// file and index), its blocks, shading / lighting and blend (alpha byte
/// and draw mode 4 of shadow tiles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileArt {
    pub frame: ComponentFrame,
    pub blocks: Vec<BlockRect>,
    pub shade: ShadeChain,
    pub blend: BlendOp,
}

/// The camera list a tile kind is placed with: lower walls and shadow
/// tiles are placed as walls, roofs lifted by their roof height.
pub fn placement_list(tile: &OrderedTile) -> TileList {
    match tile.kind {
        TileKind::Floor { .. } => TileList::Floor,
        TileKind::Roof { .. } => TileList::Roof {
            roof_height: tile.dt1.roof_height,
        },
        TileKind::LowerWall | TileKind::ShadowTile | TileKind::Wall => TileList::Wall,
    }
}

fn order_error(e: super::OrderError) -> ViewError {
    ViewError::Unresolved {
        what: "draw order",
        spec: SPEC,
        message: e.to_string(),
    }
}

fn open(what: &'static str, message: String) -> ViewError {
    ViewError::Unresolved {
        what,
        spec: SPEC,
        message,
    }
}

/// The weather state the floor pass writes, lent by the feed for one
/// frame: the weather pools, the floor pass's persistent context, the
/// local player's seed, the client update count and the level's `Mud`.
#[derive(Debug)]
pub struct WeatherFrame<'a> {
    pub weather: &'a mut Weather,
    pub floors: &'a mut FloorContext,
    pub seed: &'a mut Seed,
    pub update_count: u32,
    pub mud: bool,
    /// What passes 4 and 9 need. `None`: the passes are not wired for this
    /// feed, and a frame with live pools or lightning fails
    /// ([`WeatherFrame::unwired_passes`]).
    pub sky: Option<SkyFrame>,
    /// The sound layer the thunder step requests the thunder from; `None`:
    /// no sound layer, no request (it returns 0, so no position rolls).
    pub thunder: Option<&'a mut (dyn ThunderSound + Send + Sync)>,
    /// Where the lent seed goes back when the frame ends: the client seed
    /// the weather shares with the sound draws. `None`: the seed is the
    /// view's own.
    pub commit: Option<SeedCommit<'a>>,
}

/// The write-back of a lent seed (`WeatherFrame::commit`).
pub struct SeedCommit<'a>(pub Box<dyn FnMut(&Seed) + Send + 'a>);

impl std::fmt::Debug for SeedCommit<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SeedCommit")
    }
}

impl Drop for WeatherFrame<'_> {
    fn drop(&mut self) {
        if let Some(mut c) = self.commit.take() {
            (c.0)(self.seed);
        }
    }
}

impl WeatherFrame<'_> {
    /// The floor pass's water effects: the frame's context, then per drawn
    /// water floor in draw order, at its handed floor position (X, Y), one
    /// seed draw and the gated spawns.
    pub fn floor_pass(&mut self, water: &[(i32, i32)]) {
        self.floors
            .begin_frame(self.update_count, self.weather, self.mud);
        for &(x, y) in water {
            self.weather.water_floor(self.floors, x, y, self.seed);
        }
    }
}

impl WeatherFrame<'_> {
    /// Passes 4 and 9 of the frame: the pool cels and the
    /// sky draws, pass 9's seed draws on the lent seed. Without a
    /// [`SkyFrame`], [`WeatherFrame::unwired_passes`] and no draws.
    pub fn sky_passes(&mut self) -> Result<SkyPasses, ViewError> {
        let Some(s) = self.sky else {
            self.unwired_passes()?;
            return Ok(SkyPasses::default());
        };
        let pools = self.weather.pass4(s.frame, s.mode, s.shift_x);
        let player = LocalPlayer {
            seed: &mut *self.seed,
            level: s.level,
        };
        let input = Pass9Input {
            frame: s.frame,
            mode: s.mode,
            frame_rate: s.frame_rate,
            low_quality: s.low_quality,
            // Read only without a sound layer: no request, no handle.
            thunder_sound_starts: false,
        };
        let sound = self
            .thunder
            .as_deref_mut()
            .map(|t| t as &mut dyn ThunderSound);
        let p9 = self
            .weather
            .pass9_with(Some(player), &input, sound)
            .map_err(|e| open("weather pass 9", e.to_string()))?;
        Ok(SkyPasses {
            pools,
            sky: p9.draws,
            thunder: p9.thunder,
        })
    }
}

impl WeatherFrame<'_> {
    /// Passes 4 and 9 have no art path in
    /// the world view yet (overlay cels, lines, the flash rectangle): a
    /// frame with live pools or a running lightning fails rather than
    /// dropping them.
    pub fn unwired_passes(&self) -> Result<(), ViewError> {
        let w = &*self.weather;
        let live = [
            ("splash", w.splashes().live()),
            ("bubble", w.bubbles().live()),
            ("particle", w.particles().live()),
        ];
        if let Some((what, n)) = live.iter().find(|(_, n)| *n != 0) {
            return Err(open(
                "environment pools",
                format!("{n} live {what} record(s): passes 4 / 9 are not wired to the view"),
            ));
        }
        if w.lightning_on {
            return Err(open(
                "lightning",
                "lightning is on: pass 9 is not wired to the view".into(),
            ));
        }
        Ok(())
    }
}

/// What [`resolve_drawn`] gives besides the tiles and unit slots: the
/// records whose draw marks them drawn ([`sets_drawn_flag`]) and the
/// handed (X, Y) of every drawn water floor, in draw order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrawEffects {
    pub drawn: Vec<(usize, TileArray, usize)>,
    pub water: Vec<(i32, i32)>,
    /// The shadow-pass entries of standing units: the draw key position of
    /// each unit's shadow.
    pub shadows: BTreeMap<UnitKey, OrderKey>,
}

/// A [`ViewSource`] over a feed with the frame's draw order.
#[derive(Debug, Clone)]
pub struct OrderedSource<'a, S: ?Sized> {
    pub source: &'a S,
    pub tiles: Vec<MapTile>,
    pub units: BTreeMap<UnitKey, UnitSlot>,
    /// Unit shadows ([`DrawEffects::shadows`]).
    pub shadows: BTreeMap<UnitKey, OrderKey>,
    /// Passes 4 and 9 of the frame.
    pub sky: SkyPasses,
}

impl<S: ViewSource + ?Sized> ViewSource for OrderedSource<'_, S> {
    fn unit_position(&self, unit: &ClientUnit) -> Result<UnitPosition, String> {
        self.source.unit_position(unit)
    }

    fn unit_offset(&self, unit: &ClientUnit, pose: &UnitPose) -> Result<(i32, i32), String> {
        self.source.unit_offset(unit, pose)
    }

    fn unit_shadow_offset(&self, unit: &ClientUnit, pose: &UnitPose) -> Result<(i32, i32), String> {
        self.source.unit_shadow_offset(unit, pose)
    }

    /// The ordered tiles (the fill's and passes' tests kept), keys filled.
    fn map_tiles(&self, _: &ClientWorld, _: &ViewAssets) -> Result<Vec<MapTile>, ViewError> {
        Ok(self.tiles.clone())
    }

    fn tile_blocks(
        &self,
        tile: &MapTile,
    ) -> Result<Vec<super::super::view::BlockShade>, ViewError> {
        self.source.tile_blocks(tile)
    }

    fn tile_light(&self, tile: &MapTile) -> Option<String> {
        self.source.tile_light(tile)
    }

    fn tile_alpha(&self, tile: &MapTile) -> Option<u8> {
        self.source.tile_alpha(tile)
    }

    /// The unit shadow entry's slot; `None` for a unit whose shadow is
    /// skipped: unit flag 0x20 (a held death) or state 146 `invis`.
    /// Neither its cels nor its call rows are drawn.
    fn unit_shadow_slot(&self, unit: &ClientUnit) -> Option<OrderKey> {
        use crate::rules::blend::{unit_shadow_skipped, STATE_INVIS, UNIT_FLAG_NO_SHADOW};
        let flags = if unit.flag_20 { UNIT_FLAG_NO_SHADOW } else { 0 };
        let invis = u8::try_from(STATE_INVIS).is_ok_and(|s| unit.states.contains(&s));
        if unit_shadow_skipped(flags, invis) {
            return None;
        }
        self.shadows.get(&unit.key).copied()
    }

    /// Drawn units with their key; every other unit is not drawn.
    fn unit_slot(&self, unit: &ClientUnit) -> UnitSlot {
        self.units
            .get(&unit.key)
            .copied()
            .unwrap_or(UnitSlot::NotDrawn)
    }
}

/// Orders the frame from the feed's near rooms and wraps the feed;
/// `None` when the feed states no map (the feed answers alone).
pub fn ordered_source<'a, F: ViewFeed + ?Sized>(
    world: &ClientWorld,
    camera: &Camera,
    mode: OpenMode,
    feed: &'a mut F,
    assets: &ViewAssets,
) -> Result<Option<OrderedSource<'a, F>>, ViewError> {
    let Some(positions) = unit_positions(world, feed)? else {
        return Ok(None);
    };
    let (tiles, units, effects, sky) =
        order_and_roll(world, camera, mode, feed, assets, &positions)?;
    // Records are marked drawn after the frame's draws.
    let near = feed
        .near_rooms(world)?
        .ok_or_else(|| open("draw order", "the near rooms vanished mid-frame".into()))?;
    mark_drawn(near, &effects.drawn);
    let feed: &'a F = feed;
    Ok(Some(OrderedSource {
        source: feed,
        tiles,
        units,
        shadows: effects.shadows,
        sky,
    }))
}

/// The world draw's client-seed draws run ahead of its image: a pass whose
/// draw waits for the GPU is still the pass that owns the tick, so its
/// floor pass and passes 4 / 9 roll the seed there, before that pass's
/// cursor step and sound tick. The feed keeps the passes for the pass
/// that draws the tick ([`ViewFeed::keep_sky`]), which rolls nothing
/// again. No record is marked drawn here: the drawing pass orders the
/// same frame and marks them.
pub fn roll_ahead<F: ViewFeed + ?Sized>(
    world: &ClientWorld,
    camera: &Camera,
    mode: OpenMode,
    feed: &mut F,
    assets: &ViewAssets,
) -> Result<(), ViewError> {
    let Some(positions) = unit_positions(world, feed)? else {
        return Ok(());
    };
    let (_, _, effects) = resolve_frame(world, camera, mode, feed, assets, &positions)?;
    let sky = roll(world, camera, mode, feed, &effects.water)?;
    feed.keep_sky(world.server_ticks, sky);
    Ok(())
}

/// The near rooms' units at their positions; `None` when the feed states
/// no map.
fn unit_positions<F: ViewFeed + ?Sized>(
    world: &ClientWorld,
    feed: &mut F,
) -> Result<Option<BTreeMap<UnitKey, ClientPos>>, ViewError> {
    let keys: Vec<(usize, UnitKey)> = match feed.near_rooms(world)? {
        None => return Ok(None),
        Some(near) => near
            .rooms
            .iter()
            .enumerate()
            .flat_map(|(ri, r)| r.units.iter().map(move |u| (ri, u.key)))
            .collect(),
    };
    let mut positions: BTreeMap<UnitKey, ClientPos> = BTreeMap::new();
    for (room, key) in keys {
        let unit = world.view_unit(&key).ok_or_else(|| {
            open(
                "draw order",
                format!(
                    "room {room} lists unit ({}, {}), which the client world does not hold",
                    key.unit_type, key.guid
                ),
            )
        })?;
        let at = feed
            .unit_position(&unit)
            .map_err(|m| ViewError::Unresolved {
                what: "unit position",
                spec: "render/camera.md",
                message: m,
            })?;
        positions.insert(key, at.client());
    }
    Ok(Some(positions))
}

/// A frame's tiles, unit slots, effects and passes 4 / 9.
type RolledFrame = (
    Vec<MapTile>,
    BTreeMap<UnitKey, UnitSlot>,
    DrawEffects,
    SkyPasses,
);

/// The frame's tiles, unit slots and effects, and its passes 4 / 9: the
/// ones [`roll_ahead`] rolled for this tick, else rolled now.
fn order_and_roll<F: ViewFeed + ?Sized>(
    world: &ClientWorld,
    camera: &Camera,
    mode: OpenMode,
    feed: &mut F,
    assets: &ViewAssets,
    positions: &BTreeMap<UnitKey, ClientPos>,
) -> Result<RolledFrame, ViewError> {
    let (tiles, units, effects) = resolve_frame(world, camera, mode, feed, assets, positions)?;
    let sky = match feed.take_kept_sky(world.server_ticks) {
        Some(sky) => sky,
        None => roll(world, camera, mode, feed, &effects.water)?,
    };
    Ok((tiles, units, effects, sky))
}

/// The frame's order resolved to its tiles, unit slots and effects.
fn resolve_frame<F: ViewFeed + ?Sized>(
    world: &ClientWorld,
    camera: &Camera,
    mode: OpenMode,
    feed: &mut F,
    assets: &ViewAssets,
    positions: &BTreeMap<UnitKey, ClientPos>,
) -> Result<Resolved, ViewError> {
    let clock = feed.fade_clock(world)?;
    let order = {
        let near = feed
            .near_rooms(world)?
            .ok_or_else(|| open("draw order", "the near rooms vanished mid-frame".into()))?;
        order_frame(camera, mode, near, positions, clock).map_err(order_error)?
    };
    let feed: &F = feed;
    resolve_drawn(camera, &order, |t| feed.tile_art(t, assets))
}

/// The floor pass's water effects, then passes 4 and 9, on the feed's
/// lent weather state. Drawn water floors without a weather state are an
/// error: each one owes the player seed a draw.
fn roll<F: ViewFeed + ?Sized>(
    world: &ClientWorld,
    camera: &Camera,
    mode: OpenMode,
    feed: &mut F,
    water: &[(i32, i32)],
) -> Result<SkyPasses, ViewError> {
    match feed.weather_frame(world)? {
        Some(mut w) => {
            w.floor_pass(water);
            // The frame, mode and shiftX are the camera's.
            w.sky = w.sky.map(|s| SkyFrame {
                frame: camera.size,
                mode,
                shift_x: camera.view.shift_x,
                ..s
            });
            w.sky_passes()
        }
        None if water.is_empty() => Ok(SkyPasses::default()),
        None => Err(open(
            "water floor",
            format!(
                "floor at {:?} has material 0x2 and the feed lends no weather state \
                 (draw-order-2.md §11.5: one player-seed draw per drawn water floor)",
                water[0]
            ),
        )),
    }
}

/// The map tiles and unit slots of an order.
pub fn resolve(
    camera: &Camera,
    order: &FrameOrder,
    art: impl Fn(&OrderedTile) -> Result<TileArt, ViewError>,
) -> Result<(Vec<MapTile>, BTreeMap<UnitKey, UnitSlot>), ViewError> {
    resolve_drawn(camera, order, art).map(|(t, u, _)| (t, u))
}

/// The map tiles, unit slots and [`DrawEffects`] of an order.
pub type Resolved = (Vec<MapTile>, BTreeMap<UnitKey, UnitSlot>, DrawEffects);

/// [`resolve`], plus the frame's [`DrawEffects`].
pub fn resolve_drawn(
    camera: &Camera,
    order: &FrameOrder,
    art: impl Fn(&OrderedTile) -> Result<TileArt, ViewError>,
) -> Result<Resolved, ViewError> {
    let mut tiles = Vec::new();
    let mut units = BTreeMap::new();
    let mut fx = DrawEffects::default();
    for item in &order.items {
        match item {
            Ordered::Tile(t) => {
                let list = placement_list(t);
                let a = art(t)?;
                if sets_drawn_flag(camera, t, &a.blocks) {
                    fx.drawn.push((t.room, t.array, t.record));
                    // A drawn floor (whole-tile test passed) with water.
                    if let TileKind::Floor { .. } = t.kind {
                        if t.dt1.material & 0x2 != 0 {
                            fx.water.push(camera.tile_handed(list, t.cell.0, t.cell.1));
                        }
                    }
                }
                tiles.push(MapTile {
                    cell: t.cell,
                    list,
                    frame: a.frame,
                    blocks: a.blocks,
                    shade: a.shade,
                    blend: a.blend,
                    key: DrawKey::new(t.key.pass, t.key.major, t.key.minor, 0)?,
                });
            }
            Ordered::Unit { key, at } => {
                units.insert(*key, UnitSlot::Drawn(*at));
            }
            Ordered::UnitShadow { key, at } => {
                fx.shadows.insert(*key, *at);
            }
        }
    }
    Ok((tiles, units, fx))
}
