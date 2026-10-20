//! The overlay call record of the model. The hit overlays
//! are made from `ClientUnit::last_hit_overlay` ([`super::modes`]); the
//! state overlays ([`super::state_overlays`]) queue these records for
//! the effect layer, which makes each one on the unit
//! (`world_view::missiles`).

use super::world::UnitKey;

/// One overlay call of the model, in call order: the server tick it ran
/// at, the unit and the `overlay` id. A create is the overlay create
/// with (U, id, kind, 0, A, B, C, 0) (the hit overlays: kind 2, no seed draws; the
/// state overlays: [`super::state_overlays`]); a removal is the overlay
/// removal with (U, id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitOverlay {
    pub tick: u64,
    pub unit: UnitKey,
    pub id: u16,
    pub kind: u8,
    /// A, B, C.
    pub args: [i32; 3],
    /// instead of a create.
    pub remove: bool,
}
