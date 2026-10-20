//! Picture selection for a resolved unit component. COF order and frame events
//! remain the caller's responsibility; a source only replaces the drawn picture.

use crate::bridge::ClientUnit;
use crate::composite::ComponentFrame;
use crate::rules::unit_composite::ComponentCodes;

/// Draw context after component variants and direction fallback are resolved.
pub struct FrameRequest<'a> {
    pub unit: &'a ClientUnit,
    pub codes: &'a ComponentCodes,
    /// COF component id, 0–15.
    pub component: u8,
    /// The client's unsnapped 64-direction facing. Pack keys use it; the
    /// source cel uses the snapped COF/file direction separately.
    pub direction: u8,
    /// The 64-space facing of the file direction the source cel is drawn
    /// from: the direction a baked pack keys that cel's samples under.
    pub source_direction: u8,
    /// Starting source frame, also used to select COF layer order.
    pub frame: usize,
    /// Fraction toward the next source frame, in 1/256 units.
    pub sub_frame: u8,
}

/// Chooses a resident picture for a component. The fallback is already resolved
/// by the existing file-direction and cel rules. Missing component files never
/// reach this hook, preserving their skip behavior.
pub trait FrameSource {
    fn frame(&self, request: &FrameRequest<'_>, source: ComponentFrame) -> ComponentFrame;
}

/// Default selection: retain today's component picture for every fraction.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceFrames;

impl FrameSource for SourceFrames {
    fn frame(&self, _: &FrameRequest<'_>, source: ComponentFrame) -> ComponentFrame {
        source
    }
}
