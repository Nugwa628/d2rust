//! Ground items in the play preview's world view: each item of the model
//! in mode 3 / 5 drawn at its sub-tile with its flippy DC6, and the world
//! clicks that pick one up (C→S 0x16) or drop the cursor item (C→S 0x17).
//!
//! What follows the specs:
//! - the art file: the item's `flippyfile`, path
//!   `data\global\items\<name>.dc6`;
//! - the position: a static unit at its sub-tile, drawn at
//!   the camera's unit draw position with no extra offset, the DC6
//!   cel placed bottom-anchored;
//! - the draw key: the item's slot in the frame's draw order (a room unit:
//!   flat in mode 3, so in the shadow list of its
//!   cell, pass 5; a dropping item (mode 5) in the unit list, pass 6;
//!   sight-tested); an item the order does not draw is hidden;
//! - draw mode 5 (not highlighted): the opaque copy;
//! - a left press in the world while the local player holds a cursor item
//!   sends 0x17 with its GUID (control kind 0).
//!
//! - the cel: the unit's animation frame,
//!   which the item update steps while the item drops (mode 5) and which
//!   rests on the last frame on the ground.
//!
//! D2RUST's own, unverified (the play preview's fills):
//! - the unique / set flippy override is not
//!   applied: the model's item view has no quality or unique / set row.
//!   A compact gold pile reads its amount from its record
//!   ([`items::ItemView::gold`]) and draws the amount class's direction
//!   (< 100, < 500, < 5,000, else);
//! - no colormap; the light: the cel takes light map
//!   `v >> 3` of the low byte `v` of its unit's light word
//!   (the unit draw's EDX,
//!   `frame.unit_rows`), as every lit cel; measured on a Nova hit scene
//!   (a ground club, word 0xFEC7C7D0: map 26, pixel-equal to the base game; it
//!   was drawn unshaded, one shade step too bright);
//! - without a draw order (no map feed): pass 5 with major
//!   `DrawKey::MAJOR_MAX` and minor by (sx + sy, GUID);
//! - no shake in the camera (no effect starts one here);
//! - the click on an item: the base game hovers the unit
//!   (no hover model yet) and walks to it / interacts
//!   (item: 0x13 or a walk-to-unit code); the
//!   preview sends 0x16 PickItem with cursor flag 0 (the server walks the
//!   player to the item) when the press
//!   lands inside the art rectangle of a drawn item (topmost in draw
//!   order) that passes the hover's selectable gate (on the floor,
//!   mode 3, flag +0xC4 bit 0x2); a dropping
//!   item is not a click target and the press goes on to the walk;
//! - a file that is missing or fails to parse is logged once and its
//!   items are not drawn.
//!
//! The name labels of Show Items (Alt, ui state 0xD) are drawn by
//! `ui::show_items`; a label under the pointer
//! takes the pick-up click ([`GroundItems::set_labels`]).
//!
//! The client decides no outcome here (step 7): the draws read
//! the model, the clicks are requests the server checks.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::assets::path::{CanonicalPath, FileSource};
use crate::bridge::items::{self, ItemArtRows};
use crate::bridge::link::ServerLink;
use crate::bridge::world::{ClientWorld, UnitKey};
use crate::bridge::{Bridge, BridgeError};
use crate::frames::{FramePart, FrameSet, FrameSetKey};
use crate::rules::camera::{static_to_client, Camera, FrameSize};
use crate::rules::draw_order::UnitSlot;
use crate::rules::placement::place;
use crate::scene::order::pass;
use crate::scene::{BlendOp, DrawItem, DrawKey, ItemTag, Rect, ShadeChain};
use crate::ui::{PointerButton, UiEvent};

use super::feed::ViewFeed;
use super::{ViewAssets, WorldFrame};

/// The archive name of a flippy file.
pub fn flippy_archive_name(name: &str) -> String {
    format!("data\\global\\items\\{name}.dc6")
}

/// One drawn ground item: its draw and the screen rectangle its art
/// covers (inside the frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroundDraw {
    pub guid: u32,
    pub item: DrawItem,
    pub hit: Rect,
}

/// Frames to wait for the walk to show before picking up anyway, and
/// between two pick-up requests. D2RUST's own, unverified.
pub const PICK_WAIT_FRAMES: u32 = 8;
/// Pick-up requests after the first for one click. D2RUST's own, unverified.
pub const PICK_RETRIES: u32 = 3;

/// A click on a ground item whose pick-up waits for the walk to it
/// (D2RUST's own, unverified: the base game hovers the unit and walks to it,
/// which the model does not hold).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PendingPick {
    guid: u32,
    /// The walk toward the item was sent.
    walked: bool,
    /// The predicted walk was seen.
    seen_walk: bool,
    frames: u32,
    retries: u32,
}

/// The ground-item state of the world view: the item art rows (handed in
/// by the app), the file source the flippy files are read from, the files
/// loaded so far and the art rectangles of the last drawn frame (for the
/// clicks). The default (no rows, no source) draws nothing.
#[derive(Default)]
pub struct GroundItems {
    rows: ItemArtRows,
    source: Option<Arc<dyn FileSource>>,
    /// (Flippy name, direction) → its frame set and frame count; `None`:
    /// tried and missing or refused.
    files: BTreeMap<(String, u8), Option<(FrameSetKey, usize)>>,
    /// The Show Items labels of the last frame (`ui::show_items`).
    labels: Option<crate::ui::show_items::Layout>,
    /// The last drawn frame's items, in draw order.
    last: Vec<GroundDraw>,
    /// The click waiting for its walk ([`PendingPick`]).
    pending: Option<PendingPick>,
}

impl std::fmt::Debug for GroundItems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroundItems")
            .field("rows", &self.rows.0.len())
            .field("source", &self.source.is_some())
            .field("files", &self.files)
            .field("last", &self.last)
            .finish()
    }
}

impl GroundItems {
    /// The state with the item art rows and the archives to read the
    /// flippy files from.
    pub fn new(source: Arc<dyn FileSource>, rows: ItemArtRows) -> Self {
        GroundItems {
            rows,
            source: Some(source),
            files: BTreeMap::new(),
            labels: None,
            last: Vec::new(),
            pending: None,
        }
    }

    /// Replaces the item art rows (files already loaded stay).
    pub fn set_rows(&mut self, rows: ItemArtRows) {
        self.rows = rows;
    }

    /// Replaces the file source; files not yet tried are read from it.
    pub fn set_source(&mut self, source: Arc<dyn FileSource>) {
        self.source = Some(source);
    }

    /// The last drawn frame's ground items, in draw order.
    pub fn last(&self) -> &[GroundDraw] {
        &self.last
    }

    /// The flippy name of a ground item, `None` without a row or a name.
    fn flippy(&self, code: Option<[u8; 4]>) -> Option<&str> {
        let row = self.rows.get(code?)?;
        let name = row.flippy_file.trim();
        (!name.is_empty()).then_some(name)
    }

    /// Makes the flippy file of every ground item resident, once each.
    /// Returns one log line per new failure (the file's items are not
    /// drawn, D2RUST's own).
    pub fn ensure(&mut self, world: &ClientWorld, assets: &mut ViewAssets) -> Vec<String> {
        let mut log = Vec::new();
        let Some(source) = self.source.clone() else {
            return log;
        };
        for item in items::ground_items(world) {
            let Some(name) = self.flippy(item.code).map(str::to_owned) else {
                continue;
            };
            let key = (name.clone(), art_direction(&item));
            if self.files.contains_key(&key) {
                continue;
            }
            let loaded = load(source.as_ref(), &name, key.1, assets);
            if let Err(e) = &loaded {
                log.push(format!(
                    "ground item art: {}: {e}",
                    flippy_archive_name(&name)
                ));
            }
            self.files.insert(key, loaded.ok());
        }
        log
    }

    /// The draws of the ground items whose art is resident, under
    /// `camera`, in unit-key order (module doc).
    pub fn draws(
        &self,
        world: &ClientWorld,
        camera: &Camera,
        assets: &ViewAssets,
        slots: Option<&BTreeMap<UnitKey, UnitSlot>>,
    ) -> Vec<GroundDraw> {
        let mut found = Vec::new();
        for item in items::ground_items(world) {
            // The order's slot; not drawn → hidden.
            let slot = match slots.map(|s| s.get(&item.key).copied()) {
                None => None,
                Some(Some(UnitSlot::Drawn(k))) => Some(k),
                Some(_) => continue,
            };
            let Some(name) = self.flippy(item.code) else {
                continue;
            };
            let Some(Some((set, frames))) =
                self.files.get(&(name.to_owned(), art_direction(&item)))
            else {
                continue;
            };
            // The cel is frame unit +0x44 >> 8, the
            // drop animation's 8.8 counter (item
            // update); D2RUST's own, unverified: a frame past the
            // file's last draws the last.
            let Some(last) = frames.checked_sub(1) else {
                continue;
            };
            let index = world
                .units
                .get(&item.key)
                .and_then(|u| usize::try_from(u.frame >> 8).ok())
                .map_or(last, |f| f.min(last));
            let (Ok(id), Ok(image)) = (assets.id(set, index), assets.frame(set, index)) else {
                continue;
            };
            let (sx, sy) = (i32::from(item.x), i32::from(item.y));
            let (x, y) = camera.unit_draw(static_to_client(sx, sy), (0, 0));
            let placed = place(image, x, y, Rect::world_clip());
            // An item cel is drawn with no box
            // pre-test; off the view it keeps its call (empty clip when no
            // row is on the view) and has no click area.
            let empty = Rect::new(placed.x, placed.y, 0, 0);
            let clip = placed.clip.unwrap_or(empty);
            let art = Rect::new(placed.x, placed.y, image.width, image.height);
            // (measured 2026-10-12): an item is a
            // click target only on the floor with its selectable flag set
            // (the mode-3 update); a dropping one keeps its draw.
            let selectable = world
                .units
                .get(&item.key)
                .is_some_and(crate::bridge::hover::item_selectable);
            let hit = if selectable {
                art.intersect(&clip).unwrap_or(empty)
            } else {
                empty
            };
            let mut d = DrawItem::new(id, placed.x, placed.y);
            d.clip = clip;
            d.shade = ShadeChain::EMPTY;
            d.blend = BlendOp::Opaque;
            // An item cel is mode 5
            // with no palette; the item-color palette is not modelled yet.
            d.call = Some(crate::scene::CelArgs {
                mode: crate::rules::blend::MODE_OPAQUE,
                pal: crate::scene::CelPal::Null,
            });
            d.tag = ItemTag::Unit(item.key.guid);
            found.push((slot, sx + sy, item.key.guid, d, hit));
        }
        // Unordered: by (sx + sy, GUID) after every pass-5 entry.
        found.sort_by_key(|&(slot, depth, guid, ..)| (slot.is_some(), depth, guid));
        let mut out = found
            .into_iter()
            .enumerate()
            .filter_map(|(minor, (slot, _, guid, mut item, hit))| {
                item.key = match slot {
                    Some(k) => DrawKey::new(k.pass, k.major, k.minor, 0).ok()?,
                    None => {
                        let minor = u32::try_from(minor).ok()?;
                        DrawKey::new(pass::SHADOWS, DrawKey::MAJOR_MAX, minor, 0).ok()?
                    }
                };
                Some(GroundDraw { guid, item, hit })
            })
            .collect::<Vec<_>>();
        // In draw order (the hit test takes the last).
        out.sort_by_key(|d| d.item.key);
        out
    }

    /// Loads the art ([`Self::ensure`]) and adds the ground items to a
    /// built `frame` (re-sorted by key), under the frame's one camera
    /// (`frame.camera`). No camera, or
    /// open mode 3 (no world): nothing. Never fails the frame; returns log lines.
    pub fn add_to_frame<F: ViewFeed + ?Sized>(
        &mut self,
        world: &ClientWorld,
        feed: &F,
        assets: &mut ViewAssets,
        frame: &mut WorldFrame,
    ) -> Vec<String> {
        self.last.clear();
        if self.rows.0.is_empty() {
            return Vec::new();
        }
        let mut log = self.ensure(world, assets);
        let camera = match (feed.player(world), feed.open_mode(world)) {
            // The frame's one camera, shake included;
            // a frame built without one:
            // the player's, unshaken.
            (Ok(Some(p)), Ok(mode)) if !super::feed::no_world(mode) => frame
                .camera
                .unwrap_or_else(|| Camera::new(FrameSize::play(), mode, p.client(), (0, 0))),
            (Err(e), _) | (_, Err(e)) => {
                log.push(format!("ground items: no camera: {e}"));
                return log;
            }
            _ => return log,
        };
        self.last = self.draws(world, &camera, assets, frame.slots.as_ref());
        // The item cel's light map is the
        // low byte of its unit draw's light word (module doc).
        if let Some(t) = assets.shades.as_ref() {
            for d in &mut self.last {
                let word = frame
                    .unit_rows
                    .get(&d.item.key.slot())
                    .and_then(|r| r.light);
                if let Some(w) = word {
                    let (shade, blend) = crate::rules::blend::cel_ops(
                        t,
                        crate::rules::blend::MODE_OPAQUE,
                        None,
                        w as u8,
                    );
                    d.item.shade = shade;
                    d.item.blend = blend;
                }
            }
        }
        if !self.last.is_empty() {
            frame.items.extend(self.last.iter().map(|d| d.item));
            crate::scene::order(&mut frame.items);
        }
        log
    }

    /// Records the labels of this frame (`None`: Show Items is off).
    pub fn set_labels(&mut self, labels: Option<crate::ui::show_items::Layout>) {
        self.labels = labels;
    }

    /// The ground item whose art (last drawn frame) covers screen point
    /// `(x, y)`: the topmost, the last in draw order.
    pub fn hit(&self, x: i32, y: i32) -> Option<u32> {
        // A label is drawn over the world: it takes the click first.
        if let Some(g) = self.labels.as_ref().and_then(|l| l.hit(x, y)) {
            return Some(g);
        }
        self.last
            .iter()
            .rev()
            .find(|d| d.hit.contains(i64::from(x), i64::from(y)))
            .map(|d| d.guid)
    }

    /// The world clicks of the item rules, before the world-click
    /// dispatcher: a left press while the local player holds a cursor item
    /// → C→S 0x17; a left press on a drawn ground
    /// item → C→S 0x16, cursor flag 0 (D2RUST's own, module doc). The events
    /// taken are left out of the returned list, the rest go on to the
    /// dispatcher.
    pub fn take_clicks<L: ServerLink>(
        &mut self,
        bridge: &mut Bridge<L>,
        unhandled: &[UiEvent],
    ) -> Result<Vec<UiEvent>, BridgeError> {
        let mut rest = Vec::with_capacity(unhandled.len());
        for e in unhandled {
            if let UiEvent::Press {
                button: PointerButton::Left,
                at,
            } = *e
            {
                if let Some(held) = items::cursor_item(bridge.world()) {
                    bridge.send(&items::drop(held.key.guid))?;
                    continue;
                }
                if let Some(guid) = self.hit(at.x, at.y) {
                    bridge.send(&items::pick(guid, false))?;
                    self.pending = Some(PendingPick {
                        guid,
                        ..PendingPick::default()
                    });
                    continue;
                }
            }
            rest.push(*e);
        }
        Ok(rest)
    }
}

impl GroundItems {
    /// One frame of the click waiting for its walk: the pick-up of a far
    /// item is a walk the server does not make (the server's
    /// `walk_to_item` is a seam), so after the first request the player
    /// walks to the item, and once the predicted walk has ended
    /// (`walking` false) the pick-up is asked again, up to
    /// [`PICK_RETRIES`] times [`PICK_WAIT_FRAMES`] frames apart. The
    /// record ends when the item leaves the ground. D2RUST's own, unverified.
    pub fn frame<L: ServerLink>(
        &mut self,
        bridge: &mut Bridge<L>,
        walking: bool,
    ) -> Result<(), BridgeError> {
        let Some(mut p) = self.pending else {
            return Ok(());
        };
        let Some(item) = items::ground_items(bridge.world())
            .into_iter()
            .find(|i| i.key.guid == p.guid)
        else {
            self.pending = None;
            return Ok(());
        };
        p.frames += 1;
        if !p.walked {
            p.walked = true;
            p.frames = 0;
            bridge.send(&d2_proto::client::Walk {
                x: item.x,
                y: item.y,
            })?;
        } else if walking {
            p.seen_walk = true;
            p.frames = 0;
        } else if p.frames >= PICK_WAIT_FRAMES || p.seen_walk {
            if p.retries >= PICK_RETRIES {
                self.pending = None;
                return Ok(());
            }
            p.retries += 1;
            p.seen_walk = false;
            p.frames = 0;
            bridge.send(&items::pick(p.guid, false))?;
        }
        self.pending = Some(p);
        Ok(())
    }
}

/// The direction of a ground item's art: a gold pile's amount class,
/// else 0.
fn art_direction(item: &items::ItemView) -> u8 {
    item.gold
        .map_or(0, crate::rules::unit_composite::gold_direction)
}

/// Reads direction `dir` of flippy `name`'s DC6 into the frame store.
fn load(
    source: &dyn FileSource,
    name: &str,
    dir: u8,
    assets: &mut ViewAssets,
) -> Result<(FrameSetKey, usize), String> {
    let archive = flippy_archive_name(name);
    let path = CanonicalPath::new(&archive).map_err(|e| e.to_string())?;
    let key = FrameSetKey::new(path.as_str(), FramePart::Dir(dir)).map_err(|e| e.to_string())?;
    let dc6 = crate::assets::path::read_dc6(source, &archive)
        .ok_or_else(|| "in no archive".to_string())??;
    let frames = dc6.header.frames_per_direction as usize;
    if !assets.frames.contains(&key) {
        let set = FrameSet::from_dc6(&dc6, dir).map_err(|e| e.to_string())?;
        assets
            .frames
            .insert(key.clone(), set)
            .map_err(|e| e.to_string())?;
    }
    Ok((key, frames))
}

#[cfg(test)]
#[path = "ground_items_tests.rs"]
mod tests;
