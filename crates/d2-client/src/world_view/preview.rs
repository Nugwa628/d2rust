//! The play preview:
//! labelled fills for the inputs the client model lacks, so the
//! first playable build draws the map instead of failing the frame. The
//! strict path ([`super::ModelFeed`] without a [`Preview`]) is unchanged
//! and stays the one the capture compare uses. Nothing drawn through a
//! fill is verified against the base game.
//!
//! The fills, each `D2RUST's own, unverified`:
//! - **Light:** lit by [`super::preview_light`] (the light map, PL2
//!   shades); with `D2RUST_FULLBRIGHT=1` full bright, as follows.
//! - **Full-bright light:** Tiles draw with no light map (the source
//!   index unchanged, as a light byte of 0xFF); the
//!   feed states no frame light, so unit shade stays the rules'.
//! - **Unit facts:** zero unit flags and flag-ex, no states, objects
//!   `DrawUnder` 0, `unflatDead` off, and the sight test answers
//!   "visible" (the draw order's inputs are not in the model). With
//!   flag-ex 0 the fill lists no unit shadows (their draw is an open
//!   question).
//! - **Unit offsets:** no motion record and no table offset:
//!   `(0, 0)`.
//! - **Water floors:** material bit 0x2 is cleared in the near rooms, so
//!   the floor pass spawns no water effects (no weather state is fed).
//! - **Errors:** a tile whose art cannot be made resident is skipped (drawn
//!   as the empty [`skip_key`] frame) and logged once; a near-room build
//!   that fails leaves the frame without the map, logged once.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use crate::bridge::world::ClientWorld;
use crate::bridge::ClientUnit;
use crate::composite::ComponentFrame;
use crate::frames::{FramePart, FrameSet, FrameSetKey, IndexFrame};
use crate::rules::blend::{shadow_tile_ops, wall_draw, WallDraw};
use crate::rules::draw_order::source::TileArt;
use crate::rules::draw_order::{NearRooms, OrderedTile, TileArray, TileKind, UnitFacts};
use crate::rules::shading::ShadeTables;
use crate::scene::{BlendOp, DrawKey, ShadeChain};

use super::near_rooms::{Dt1Entry, MapState};
use super::tile_assets::{tile_key, TileAssets};
use super::{ViewAssets, ViewError};

/// The path of the empty frame a skipped item draws (not an archive path).
pub const SKIP_PATH: &str = "d2rust/preview/skip";

/// The key of the empty frame set: one 1 × 1 frame of index 0
/// (transparent).
pub fn skip_key() -> FrameSetKey {
    FrameSetKey::new(SKIP_PATH, FramePart::Tile(0)).expect("canonical")
}

/// Makes the empty frame set resident.
pub fn ensure_skip(assets: &mut ViewAssets) -> Result<(), ViewError> {
    let key = skip_key();
    if assets.frames.contains(&key) {
        return Ok(());
    }
    let frame = IndexFrame::new(1, 1, 0, 0, vec![0]).map_err(|e| ViewError::Unresolved {
        what: "preview skip frame",
        spec: "docs/PLAN.md D1",
        message: e.to_string(),
    })?;
    assets.frames.insert(
        key,
        FrameSet {
            frames: vec![frame],
        },
    )?;
    Ok(())
}

/// The art of a skipped tile: the empty frame, no blocks.
pub fn skipped_art() -> TileArt {
    TileArt {
        frame: ComponentFrame {
            set: skip_key(),
            index: 0,
        },
        blocks: Vec::new(),
        shade: ShadeChain::EMPTY,
        blend: BlendOp::Opaque,
    }
}

// D2RUST's own, unverified (the preview): the unit facts the model lacks are zero, the
// sight test answers visible (with the unit tables the feed runs the test
// of `model_feed::preview_facts`). Flag-ex 0x80 is
// the last frame's sight test (`MapState` keeps it).
/// The facts of a room unit in the preview.
pub fn unit_facts(world: &ClientWorld, unit: &ClientUnit) -> UnitFacts {
    UnitFacts {
        unit_type: unit.key.unit_type,
        mode: unit.mode,
        local: world.local_player == Some(unit.key),
        sight_hidden: Some(false),
        ..UnitFacts::default()
    }
}

// D2RUST's own, unverified (the preview): no motion record; the table offset is the
// feed's ([`Preview::object_offsets`]).
/// A unit's extra offset in the preview (the composite with no
/// record and `TableOffset::None`).
pub fn unit_offset() -> (i32, i32) {
    (0, 0)
}

// D2RUST's own, unverified (the preview): full bright (no light map); the
// blend is the tile rule's for the record's kind and alpha byte. Shadow tiles draw
// blended (Blended Shadows on); without the act's tables they and
// translucent walls cannot be drawn as the rules say: shadow tiles are
// skipped, translucent walls drawn opaque.
/// The shade and blend of a tile of `kind` with alpha byte `alpha`, or
/// `None` when it is not drawn (alpha below 0x40;
/// a shadow tile without tables).
pub fn tile_ops(
    kind: TileKind,
    alpha: u8,
    tables: Option<&ShadeTables>,
) -> Option<(ShadeChain, BlendOp)> {
    match kind {
        TileKind::Floor { .. } => Some((ShadeChain::EMPTY, BlendOp::Opaque)),
        TileKind::ShadowTile => {
            tables.map(|t| shadow_tile_ops(t, crate::world_view::sharp::blended_shadows()))
        }
        TileKind::LowerWall | TileKind::Wall | TileKind::Roof { .. } => match wall_draw(alpha) {
            WallDraw::Lit => Some((ShadeChain::EMPTY, BlendOp::Opaque)),
            WallDraw::Hidden => None,
            WallDraw::Translucent(table) => Some((
                ShadeChain::EMPTY,
                tables.map_or(BlendOp::Opaque, |t| {
                    BlendOp::IndexTableSrcRow(table.base(t))
                }),
            )),
        },
    }
}

// D2RUST's own, unverified (the preview): no weather state is fed, so no water floor.
/// Clears the water bit (material 0x2) of every floor record.
pub fn dry_floors(near: &mut NearRooms) {
    for room in &mut near.rooms {
        for r in &mut room.floors {
            r.dt1.material &= !0x2;
        }
    }
}

/// The wall, floor and shadow record counts of each near room.
pub fn record_counts(near: Option<&NearRooms>) -> Vec<[usize; 3]> {
    near.map_or_else(Vec::new, |n| {
        n.rooms
            .iter()
            .map(|r| [r.walls.len(), r.floors.len(), r.shadows.len()])
            .collect()
    })
}

/// The DT1 entries of every record of the near rooms with `counts`.
pub fn entries(map: &MapState, counts: &[[usize; 3]]) -> Vec<Dt1Entry> {
    let mut out = Vec::new();
    for (ri, c) in counts.iter().enumerate() {
        for (array, n) in [
            (TileArray::Wall, c[0]),
            (TileArray::Floor, c[1]),
            (TileArray::Shadow, c[2]),
        ] {
            out.extend((0..n).filter_map(|i| map.entry(ri, array, i).cloned()));
        }
    }
    // The act's edge floor record.
    out.extend(map.entry(0, TileArray::Edge, 0).cloned());
    out
}

/// The preview's state in the model feed: the tile art and the log of
/// what was skipped.
#[derive(Debug, Clone, Default)]
pub struct Preview {
    pub tiles: TileAssets,
    /// The palette act of the last prepared frame (its shade tables).
    act: u8,
    /// The preview's light (`preview_light`); full bright with
    /// `D2RUST_FULLBRIGHT=1`.
    pub light: super::preview_light::PreviewLight,
    logged: Arc<Mutex<BTreeSet<String>>>,
    /// Selected warp map records, rebuilt each frame without changing DRLG.
    warp_highlight: Vec<(
        crate::bridge::drlg::DrlgRoomId,
        d2_sim::drlg::tiles::RecordKind,
        usize,
    )>,
    /// The frame's per-block shades by draw key (`preview_blocks`), filled
    /// by [`Self::tile_art`], read by `ViewSource::tile_blocks`.
    block_shades: Arc<Mutex<BTreeMap<DrawKey, Vec<crate::rules::BlockShade>>>>,
    /// The frame's tile light digests by draw key, for the rendering facts,
    /// filled by [`Self::tile_art`].
    tile_lights: Arc<Mutex<BTreeMap<DrawKey, String>>>,
    /// The frame's wall alpha bytes by draw key, filled by
    /// [`Self::tile_art`] for walls drawn translucent (alpha below 0xFF).
    tile_alphas: Arc<Mutex<BTreeMap<DrawKey, u8>>>,
    /// `objects` Xoffset / Yoffset by class (`unit_assets::object_offsets`),
    /// added to an object's draw offset.
    pub object_offsets: Arc<BTreeMap<u32, (i32, i32)>>,
}

impl Preview {
    pub fn new(tiles: TileAssets) -> Self {
        Preview {
            tiles,
            light: super::preview_light::PreviewLight::new(),
            ..Preview::default()
        }
    }

    /// Stores the per-block shades of the tile with draw key `key`.
    pub(crate) fn put_block_shades(&self, key: DrawKey, shades: Vec<crate::rules::BlockShade>) {
        if let Ok(mut b) = self.block_shades.lock() {
            b.insert(key, shades);
        }
    }

    /// The light digest of the tile with draw key `key` this frame.
    pub fn tile_light_of(&self, key: DrawKey) -> Option<String> {
        self.tile_lights
            .lock()
            .ok()
            .and_then(|b| b.get(&key).cloned())
    }

    /// The alpha byte of the translucent wall with draw key `key` this
    /// frame.
    pub fn tile_alpha_of(&self, key: DrawKey) -> Option<u8> {
        self.tile_alphas
            .lock()
            .ok()
            .and_then(|b| b.get(&key).copied())
    }

    /// The per-block shades of the tile with draw key `key` this frame.
    pub fn block_shades_of(&self, key: DrawKey) -> Vec<crate::rules::BlockShade> {
        self.block_shades
            .lock()
            .ok()
            .and_then(|b| b.get(&key).cloned())
            .unwrap_or_default()
    }

    /// Logs `message` the first time it is seen; returns whether it was new.
    pub fn log_once(&self, message: String) -> bool {
        let new = self
            .logged
            .lock()
            .map(|mut l| l.insert(message.clone()))
            .unwrap_or(false);
        if new {
            bevy::log::warn!("preview (D2RUST's own, unverified): skipped: {message}");
        }
        new
    }

    /// Everything logged so far (tests, the run summary).
    pub fn logged(&self) -> Vec<String> {
        self.logged
            .lock()
            .map(|l| l.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Before the frame's build: the act's shade tables, the empty frame
    /// and the near rooms' DT1 `entries` resident; failures logged.
    pub fn prepare(
        &mut self,
        world: &ClientWorld,
        entries: &[Dt1Entry],
        assets: &mut ViewAssets,
    ) -> Result<(), ViewError> {
        self.act = world.palette_act.unwrap_or(0);
        self.warp_highlight.clear();
        if let Some(key) = self
            .light
            .look()
            .hover
            .filter(|k| k.unit_type == crate::bridge::world::TILE)
        {
            if let (Some(u), Some(room), Some(drlg)) = (
                world.units.get(&key),
                world.unit_room(key),
                world.drlg.as_ref(),
            ) {
                if let Some(position) = u.position {
                    self.warp_highlight
                        .extend(drlg.warp_highlight_records(room.room, u.class, position));
                }
            }
        }
        if let Ok(mut b) = self.block_shades.lock() {
            b.clear();
        }
        if let Err(m) = self.tiles.ensure_shades(self.act, &mut assets.maps) {
            self.log_once(m);
        }
        self.light.ensure_shifts(world, &mut assets.maps);
        // The act's tables when the tiles hold them; else the UI path's
        // (`app::ui::push_text_colors`) stay: the UI rectangles of draw
        // mode 2 read them.
        if let Some(t) = self.tiles.shades(self.act).copied() {
            assets.shades = Some(t);
        }
        ensure_skip(assets)?;
        for m in self.tiles.ensure(entries, assets) {
            self.log_once(m);
        }
        Ok(())
    }

    fn warp_tile_highlighted(&self, map: Option<&MapState>, tile: &OrderedTile) -> bool {
        use d2_sim::drlg::tiles::RecordKind;
        let Some(room) = map.and_then(|m| m.room(tile.room)) else {
            return false;
        };
        let kind = match tile.array {
            TileArray::Wall => RecordKind::Wall,
            TileArray::Floor => RecordKind::Floor,
            _ => return false,
        };
        self.warp_highlight.contains(&(room, kind, tile.record))
    }

    /// The art of an ordered tile: its DT1 frame and blocks with the
    /// preview's [`tile_ops`]; a tile that is not resident is skipped
    /// ([`skipped_art`]), and so is one that is not drawn.
    pub fn tile_art(&self, map: Option<&MapState>, tile: &OrderedTile) -> TileArt {
        let Some((path, index)) = map.and_then(|m| m.entry(tile.room, tile.array, tile.record))
        else {
            self.log_once(format!(
                "no DT1 entry for room {} {:?} record {}",
                tile.room, tile.array, tile.record
            ));
            return skipped_art();
        };
        let key = match tile_key(path, *index) {
            Ok(k) => k,
            Err(m) => {
                self.log_once(m);
                return skipped_art();
            }
        };
        let Some(blocks) = self.tiles.blocks(&key) else {
            // `prepare` logged why (or the entry appeared after it ran).
            if self.tiles.failure(&key).is_none() {
                self.log_once(format!("{} tile {index}: not resident", key.path()));
            }
            return skipped_art();
        };
        let Some((shade, blend)) = tile_ops(tile.kind, tile.alpha, self.tiles.shades(self.act))
        else {
            return skipped_art();
        };
        // Lit (preview_light, D2RUST's own, unverified): a tile's light is one
        // flat value; shadow tiles keep their blend-table chain.
        let shade = match tile.kind {
            TileKind::ShadowTile => shade,
            _ => self
                .light
                .tile_chain(tile.kind, &tile.dt1, tile.cell)
                .unwrap_or(shade),
        };
        if !matches!(tile.kind, TileKind::ShadowTile) {
            let grids = self.tiles.grids(&key).unwrap_or_default();
            let shades = self.light.block_shades(
                tile.kind,
                &tile.dt1,
                tile.cell,
                (tile.alpha, tile.fade_state),
                blend,
                blocks,
                grids,
            );
            if let Ok(k) = DrawKey::new(tile.key.pass, tile.key.major, tile.key.minor, 0) {
                if let (Some(d), Ok(mut m)) = (
                    self.light
                        .tile_light_digest(tile.kind, &tile.dt1, tile.cell, tile.fade_state),
                    self.tile_lights.lock(),
                ) {
                    m.insert(k, d);
                }
                // pass 1: TileDrawTrans takes the alpha.
                if let Ok(mut m) = self.tile_alphas.lock() {
                    if matches!(blend, BlendOp::IndexTableSrcRow(_)) {
                        m.insert(k, tile.alpha);
                    } else {
                        m.remove(&k);
                    }
                }
                if !shades.is_empty() {
                    self.put_block_shades(k, shades);
                }
            }
        }
        let (shade, blend) = if self.warp_tile_highlighted(map, tile) {
            match self.tiles.shades(self.act) {
                Some(t) => {
                    if let Ok(k) = DrawKey::new(tile.key.pass, tile.key.major, tile.key.minor, 0) {
                        self.put_block_shades(k, Vec::new());
                    }
                    (
                        ShadeChain::new(&[t.highlight]).expect("one highlight map"),
                        BlendOp::Opaque,
                    )
                }
                None => (shade, blend),
            }
        } else {
            (shade, blend)
        };
        TileArt {
            frame: ComponentFrame { set: key, index: 0 },
            blocks: blocks.to_vec(),
            shade,
            blend,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{MONSTER, PLAYER};
    use crate::bridge::UnitKey;
    use crate::rules::shading::ShadeTables;
    use crate::scene::MapTable;

    fn tables() -> ShadeTables {
        let pl2 =
            d2_formats::palette::Pl2::parse(&super::super::tile_assets::tests::pl2()).unwrap();
        ShadeTables::push(&mut MapTable::new(), &pl2)
    }

    #[test]
    fn tile_ops_are_full_bright_with_the_kinds_blend() {
        let t = tables();
        let floor = TileKind::Floor { layer: 1 };
        assert_eq!(
            tile_ops(floor, 0x10, None),
            Some((ShadeChain::EMPTY, BlendOp::Opaque))
        );
        assert_eq!(
            tile_ops(TileKind::Wall, 0xFF, Some(&t)),
            Some((ShadeChain::EMPTY, BlendOp::Opaque))
        );
        assert_eq!(tile_ops(TileKind::Roof { pass: 1 }, 0x3F, Some(&t)), None);
        assert_eq!(
            tile_ops(TileKind::LowerWall, 0x80, Some(&t)),
            Some((ShadeChain::EMPTY, BlendOp::IndexTableSrcRow(t.alpha[1])))
        );
        assert_eq!(
            tile_ops(TileKind::Wall, 0xC0, None),
            Some((ShadeChain::EMPTY, BlendOp::Opaque))
        );
        assert_eq!(tile_ops(TileKind::ShadowTile, 0xFF, None), None);
        assert_eq!(
            tile_ops(TileKind::ShadowTile, 0xFF, Some(&t)),
            Some((ShadeChain::EMPTY, BlendOp::IndexTable(t.alpha[0])))
        );
    }

    /// What the hover highlight of an entrance tile looks like on screen:
    /// the highlight map replaces the tile's shade and index 0 stays
    /// transparent, so a wall of rock follows its silhouette, while a floor
    /// tile fills its whole diamond. This is why a cave entrance's arch (walls),
    /// not its floor, carries the highlight.
    #[test]
    fn a_highlighted_wall_follows_its_silhouette_and_a_floor_fills_its_diamond() {
        use crate::scene::{compose, DrawItem, FrameId, FrameImage, Rect};

        let pl2 =
            d2_formats::palette::Pl2::parse(&super::super::tile_assets::tests::pl2()).unwrap();
        let mut maps = MapTable::new();
        let t = ShadeTables::push(&mut maps, &pl2);
        // An 8 x 4 arch: two pillars and a lintel, a hole between them.
        let arch: [&str; 4] = ["########", "##....##", "##....##", "##....##"];
        let wall = FrameImage {
            width: 8,
            height: 4,
            pixels: arch
                .iter()
                .flat_map(|row| row.bytes().map(|c| if c == b'#' { 20u8 } else { 0 }))
                .collect(),
        };
        let diamond: [&str; 4] = ["..####..", ".######.", ".######.", "..####.."];
        let floor = FrameImage {
            width: 8,
            height: 4,
            pixels: diamond
                .iter()
                .flat_map(|row| row.bytes().map(|c| if c == b'#' { 20u8 } else { 0 }))
                .collect(),
        };
        let frames = [wall, floor];
        let view = Rect::new(0, 0, 8, 4);
        let highlighted = |id: u32| {
            let mut item = DrawItem::new(FrameId(id), 0, 0);
            item.clip = view;
            item.shade = ShadeChain::new(&[t.highlight]).unwrap();
            item
        };
        let lit = |src: u8| maps.get(t.highlight).unwrap()[usize::from(src)];
        for (id, shape) in [(0, arch), (1, diamond)] {
            let got = compose(&[highlighted(id)], &frames[..], &maps, view).unwrap();
            for (i, c) in shape.iter().flat_map(|r| r.bytes()).enumerate() {
                let want = if c == b'#' { lit(20) } else { 0 };
                assert_eq!(got[i], want, "shape {id} pixel {i}");
            }
        }
        assert_ne!(lit(20), 20, "the highlight map brightens the art");
    }

    #[test]
    fn unit_facts_are_zero_and_visible() {
        let mut w = ClientWorld::default();
        let key = UnitKey::new(PLAYER, 7);
        let mut u = ClientUnit::new(key);
        u.mode = 2;
        w.local_player = Some(key);
        let f = unit_facts(&w, &u);
        assert_eq!(
            f,
            UnitFacts {
                unit_type: PLAYER,
                mode: 2,
                local: true,
                sight_hidden: Some(false),
                // Flag-ex 0x80 is the last frame's sight test, kept by
                // `MapState`, not a fact of the unit.
                ..UnitFacts::default()
            }
        );
        let m = ClientUnit::new(UnitKey::new(MONSTER, 7));
        let f = unit_facts(&w, &m);
        assert!(!f.local);
        // A sight-tested unit draws (no shadow entry: flag-ex 0).
        let mut f = f;
        f.mode = 1;
        assert!(crate::rules::draw_order::unit_draws(&mut f).unwrap());
        assert_eq!(unit_offset(), (0, 0));
    }

    #[test]
    fn skip_frame_is_resident_once_and_logs_are_deduplicated() {
        let mut a = ViewAssets::new(crate::app::play::unspecified_palette());
        ensure_skip(&mut a).unwrap();
        ensure_skip(&mut a).unwrap();
        let f = a.frame(&skip_key(), 0).unwrap();
        assert_eq!((f.width, f.height), (1, 1));
        let p = Preview::default();
        assert!(p.log_once("x".into()));
        assert!(!p.log_once("x".into()));
        assert_eq!(p.logged(), ["x"]);
    }
}
