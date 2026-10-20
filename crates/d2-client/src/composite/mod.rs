//! Composite units: characters and monsters drawn as a stack of component sprites.
//!
//! A composite unit is not one picture. Its COF file lists up to sixteen components (head,
//! torso, legs, both arms, both hands, shield, special slots S1-S8) and, for every direction
//! and animation frame, the order in which they are stacked. A sword swung behind the body
//! in one frame is drawn in front of it a few frames later just by changing that order.
//!
//! This module turns a parsed COF, a COF direction and a frame into
//! 1. the *slot order*: the components to draw, back to front ([`slot_order`]), and
//! 2. one scene [`DrawItem`] per drawn component ([`build`], [`build_with`]), keyed so the
//!    scene sorts them as one block, in slot order, among everything else in the frame.
//!
//! Only the mechanism lives here. Every answer that depends on game rules (which component
//! file and frame to draw, where on screen, which colour maps and which blend) comes from a
//! [`ComponentResolver`], one hook per question. The frame's scene id is the one exception:
//! [`build_with`] looks it up in a frame store ([`FrameIds`], implemented by
//! [`crate::frames::FrameStore`]). Plain Rust, no Bevy types.

#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod tests;

use d2_formats::cof::{Cof, CofLayer, COMPONENTS};

use crate::frames::{FrameSetKey, FrameStore, StoreError};
use crate::scene::{BlendOp, DrawItem, DrawKey, FrameId, ItemTag, Rect, SceneError, ShadeChain};

/// Component id of S7. S7 has no graphic of its own: its place in the draw order marks
/// where a linked missile or attached unit is drawn inline with its host ([`inline_slot`]).
pub const S7: u8 = 14;

/// Errors of composite building.
///
/// Input is strict: a COF this module cannot read without guessing is an error, never a
/// silently skipped slot.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompositeError {
    /// The requested COF direction does not exist in the file.
    #[error("direction {dir} out of range ({directions} in the COF)")]
    Direction { dir: usize, directions: u8 },
    /// The draw-order row of (`dir`, `frame`) reaches past the end of the file. Frames are
    /// not bound-checked (see [`draw_order_row`]), so this is where a frame number that is
    /// far too large ends up.
    #[error("draw-order row of direction {dir}, frame {frame} is past the file end")]
    RowPastEnd { dir: usize, frame: usize },
    /// The header's layer count and the number of layer records disagree.
    #[error("COF has {count} layer records for {layers} layers")]
    LayerCount { layers: u8, count: usize },
    /// A layer record names a component id outside `0..16`.
    #[error("layer {index}: component {component} is not 0..{COMPONENTS}")]
    LayerComponent { index: usize, component: u8 },
    /// The draw-order table is not `directions × frames × layers` bytes long.
    #[error("COF draw order has {len} bytes, header needs {expected}")]
    DrawOrderLength { len: usize, expected: usize },
    /// A [`ComponentResolver`] hook (or the frame store) could not answer: frame not
    /// resident, unknown variant, ... `what` names the hook.
    #[error("slot {slot} ({component}): {what}: {message}")]
    Unresolved {
        slot: u8,
        component: u8,
        what: &'static str,
        message: String,
    },
    /// The scene rejected the draw key built for this slot (a unit key field out of range).
    #[error("slot {slot}: {error}")]
    Scene { slot: u8, error: SceneError },
}

/// One entry of a frame's slot order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// Position in the draw-order row, back to front. Becomes the draw key's `sub`, so
    /// slots that draw nothing leave gaps instead of renumbering the others.
    pub slot: u8,
    /// Component id (0-15, `COMPONENT_NAMES` in `d2-formats`).
    pub component: u8,
    /// Index of the layer record that carries `component`.
    pub layer: usize,
}

/// Checks the parts of `cof` this module indexes into. Parsed files always pass; a
/// hand-built [`Cof`] may not.
fn check(cof: &Cof) -> Result<(), CompositeError> {
    let layers = usize::from(cof.layers_count);
    if cof.layers.len() != layers {
        return Err(CompositeError::LayerCount {
            layers: cof.layers_count,
            count: cof.layers.len(),
        });
    }
    let expected = usize::from(cof.directions) * usize::from(cof.frames) * layers;
    if cof.draw_order.len() != expected {
        return Err(CompositeError::DrawOrderLength {
            len: cof.draw_order.len(),
            expected,
        });
    }
    // Two records for the same component are allowed: lookups take the first one.
    for (index, layer) in cof.layers.iter().enumerate() {
        if usize::from(layer.component) >= COMPONENTS {
            return Err(CompositeError::LayerComponent {
                index,
                component: layer.component,
            });
        }
    }
    Ok(())
}

/// The `L` component ids of the draw-order row for COF direction `dir`, frame `frame`
/// (`L` = layer count, `F` = frames per direction).
///
/// The row of (`dir`, `frame`) is found as if the event block before the draw order were
/// exactly `F` bytes long: it starts `(dir × F + frame) × L` bytes after the first `F`
/// event bytes. In a parsed [`Cof`] those bytes are `event_padding` followed by
/// `draw_order`, so in a file whose event block is padded every row is read `padding`
/// bytes early. That is how such files draw, and it is kept on purpose.
///
/// `frame` is not checked against `F`: a frame at or past `F` reads the following
/// direction's rows. Only a row that runs past the end of the file is an error.
pub fn draw_order_row(cof: &Cof, dir: usize, frame: usize) -> Result<Vec<u8>, CompositeError> {
    check(cof)?;
    if dir >= usize::from(cof.directions) {
        return Err(CompositeError::Direction {
            dir,
            directions: cof.directions,
        });
    }
    let layers = usize::from(cof.layers_count);
    let past_end = || CompositeError::RowPastEnd { dir, frame };
    let start = dir
        .checked_mul(usize::from(cof.frames))
        .and_then(|row| row.checked_add(frame))
        .and_then(|row| row.checked_mul(layers))
        .ok_or_else(past_end)?;
    let end = start.checked_add(layers).ok_or_else(past_end)?;
    if end > cof.event_padding.len() + cof.draw_order.len() {
        return Err(past_end());
    }
    Ok(cof
        .event_padding
        .iter()
        .chain(&cof.draw_order)
        .skip(start)
        .take(layers)
        .copied()
        .collect())
}

/// The slots drawn for COF direction `dir`, frame `frame`, back to front.
///
/// Slot `s` is byte `s` of the [`draw_order_row`], paired with the first layer record of
/// that component. Two kinds of slot draw nothing and are left out, while every other slot
/// keeps its index `s`:
/// - a component without a layer record (the unit simply has no such part; not an error);
/// - [`S7`], which has no graphic of its own (see [`inline_slot`]).
pub fn slot_order(cof: &Cof, dir: usize, frame: usize) -> Result<Vec<Slot>, CompositeError> {
    let row = draw_order_row(cof, dir, frame)?;
    Ok(row
        .into_iter()
        .zip(0u8..)
        .filter(|&(component, _)| component != S7)
        .filter_map(|(component, slot)| {
            let layer = cof.layers.iter().position(|l| l.component == component)?;
            Some(Slot {
                slot,
                component,
                layer,
            })
        })
        .collect())
}

/// The first slot whose component is [`S7`]: where a linked missile or attached unit is
/// drawn inline, sharing the host's slot index as its draw key `sub`. Whether such a unit
/// exists is up to the caller.
pub fn inline_slot(cof: &Cof, dir: usize, frame: usize) -> Result<Option<u8>, CompositeError> {
    let row = draw_order_row(cof, dir, frame)?;
    Ok(row
        .iter()
        .zip(0u8..)
        .find(|&(&component, _)| component == S7)
        .map(|(_, slot)| slot))
}

/// What a hook is asked about: one slot of one COF frame.
#[derive(Debug, Clone, Copy)]
pub struct ComponentRequest<'a> {
    /// The COF being composed.
    pub cof: &'a Cof,
    /// COF direction.
    pub dir: usize,
    /// COF frame.
    pub frame: usize,
    /// The slot being drawn.
    pub slot: Slot,
    /// The layer record of `slot.component`: shadow, selectable, translucency override,
    /// weapon class.
    pub layer: &'a CofLayer,
}

/// Which frame set, and which frame in it, a component draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentFrame {
    /// The component's sprite file and direction.
    pub set: FrameSetKey,
    /// Frame index inside that set.
    pub index: usize,
}

/// The game-rule answers a composite needs, one hook per question.
///
/// An implementation that lacks the inputs to answer returns an error
/// ([`CompositeError::Unresolved`]), never a made-up default.
pub trait ComponentResolver {
    /// The component's sprite file (unit token, armor-class variant, mode, weapon class),
    /// the file direction for the unit's facing, and the frame inside that direction. The
    /// rules live in `rules::unit_composite`. Whether a slot draws at all is
    /// [`Self::slot_frame`].
    fn frame(&self, req: &ComponentRequest<'_>) -> Result<ComponentFrame, CompositeError>;

    /// The slot's frame, or `None` when the slot draws nothing: the component's file cannot
    /// be resolved or does not load. That leaves a gap in the unit, not an error.
    /// [`build`] and [`build_with`] call this; the default draws every slot with
    /// [`Self::frame`].
    fn slot_frame(
        &self,
        req: &ComponentRequest<'_>,
    ) -> Result<Option<ComponentFrame>, CompositeError> {
        self.frame(req).map(Some)
    }

    /// Screen top-left of the component's image, from the unit's screen position and the
    /// frame's own offsets.
    fn place(
        &self,
        req: &ComponentRequest<'_>,
        frame: &ComponentFrame,
    ) -> Result<(i32, i32), CompositeError>;

    /// The colour maps the component's pixels go through: its palette remap (colour
    /// variant), then the light map of the unit's light level (or the highlight map while
    /// the unit is highlighted).
    fn shade(&self, req: &ComponentRequest<'_>) -> Result<ShadeChain, CompositeError>;

    /// The blend op of the component's draw mode, including the COF layer's translucency
    /// override.
    fn blend(&self, req: &ComponentRequest<'_>) -> Result<BlendOp, CompositeError>;

    /// The scene id of a resident frame. A frame that is not resident is an error.
    ///
    /// Only [`build`] asks this. [`build_with`] takes ids from a frame store and never
    /// calls it, so a resolver used only with [`build_with`] keeps this default, which
    /// refuses.
    fn frame_id(
        &self,
        req: &ComponentRequest<'_>,
        _frame: &ComponentFrame,
    ) -> Result<FrameId, CompositeError> {
        Err(CompositeError::Unresolved {
            slot: req.slot.slot,
            component: req.slot.component,
            what: "frame_id",
            message: "no frame store: use composite::build_with".into(),
        })
    }
}

/// `(FrameSetKey, index)` to scene [`FrameId`] for resident frames. A frame that is not
/// resident, or an index past the end of its set, is an error.
pub trait FrameIds {
    fn frame_id(&self, set: &FrameSetKey, index: usize) -> Result<FrameId, StoreError>;
}

impl FrameIds for FrameStore {
    fn frame_id(&self, set: &FrameSetKey, index: usize) -> Result<FrameId, StoreError> {
        self.id(set, index)
    }
}

/// Per-unit draw parameters shared by all of the unit's components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitParams {
    /// Draw key fields shared by every component, from the unit's place in the frame's
    /// draw order (for example: the unit pass, the cell index as `major`, walls drawn so
    /// far plus the unit's list position as `minor`). The key's `sub` is the slot index.
    pub pass: u32,
    pub major: u32,
    pub minor: u32,
    /// The whole frame. Units are never clipped to the play area and never wall-culled.
    pub clip: Rect,
    /// Debug label, e.g. [`ItemTag::Unit`] with the unit's id.
    pub tag: ItemTag,
}

/// One drawn component: its slot, the frame it shows and its scene item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentDraw {
    pub slot: Slot,
    pub frame: ComponentFrame,
    pub item: DrawItem,
}

/// The draw items of one unit for COF direction `dir`, frame `frame`, with frame ids from
/// the resolver's [`ComponentResolver::frame_id`].
///
/// Items come in slot order (back to front), and every key shares the unit's
/// pass/major/minor with `sub` = slot index, so list order equals key order. Slots that draw
/// nothing ([`slot_order`], [`ComponentResolver::slot_frame`]) give no item. Any hook error
/// fails the whole unit: a composite is never drawn half-built.
pub fn build<R: ComponentResolver + ?Sized>(
    cof: &Cof,
    dir: usize,
    frame: usize,
    unit: &UnitParams,
    resolver: &R,
) -> Result<Vec<ComponentDraw>, CompositeError> {
    build_items(cof, dir, frame, unit, resolver, |req, cf| {
        resolver.frame_id(req, cf)
    })
}

/// [`build`] with frame ids from the frame store `frames`: each component's frame
/// ([`ComponentResolver::frame`]) is looked up as `(set, index)`. A frame missing from the
/// store fails the unit ([`CompositeError::Unresolved`] with `what` = `"frame_id"`). The
/// resolver's own `frame_id` is never called.
pub fn build_with<R: ComponentResolver + ?Sized, S: FrameIds + ?Sized>(
    cof: &Cof,
    dir: usize,
    frame: usize,
    unit: &UnitParams,
    resolver: &R,
    frames: &S,
) -> Result<Vec<ComponentDraw>, CompositeError> {
    build_items(cof, dir, frame, unit, resolver, |req, cf| {
        frames
            .frame_id(&cf.set, cf.index)
            .map_err(|e| CompositeError::Unresolved {
                slot: req.slot.slot,
                component: req.slot.component,
                what: "frame_id",
                message: e.to_string(),
            })
    })
}

/// The shared body of [`build`] and [`build_with`]; `frame_id` is where they differ.
fn build_items<R: ComponentResolver + ?Sized>(
    cof: &Cof,
    dir: usize,
    frame: usize,
    unit: &UnitParams,
    resolver: &R,
    frame_id: impl Fn(&ComponentRequest<'_>, &ComponentFrame) -> Result<FrameId, CompositeError>,
) -> Result<Vec<ComponentDraw>, CompositeError> {
    let slots = slot_order(cof, dir, frame)?;
    let mut draws = Vec::with_capacity(slots.len());
    for slot in slots {
        let req = ComponentRequest {
            cof,
            dir,
            frame,
            slot,
            layer: &cof.layers[slot.layer],
        };
        let Some(component_frame) = resolver.slot_frame(&req)? else {
            continue;
        };
        let id = frame_id(&req, &component_frame)?;
        let (x, y) = resolver.place(&req, &component_frame)?;
        let mut item = DrawItem::new(id, x, y);
        item.clip = unit.clip;
        item.shade = resolver.shade(&req)?;
        item.blend = resolver.blend(&req)?;
        item.key = DrawKey::new(unit.pass, unit.major, unit.minor, slot.slot).map_err(|error| {
            CompositeError::Scene {
                slot: slot.slot,
                error,
            }
        })?;
        item.tag = unit.tag;
        draws.push(ComponentDraw {
            slot,
            frame: component_frame,
            item,
        });
    }
    Ok(draws)
}
