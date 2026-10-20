//! Draw order: the `DrawKey` layout, the pass numbers and the stable sort.
//!
//! Every scene item carries a `DrawKey`; sorting by it (stably) gives the back-to-front order the
//! compositor draws in. The world builder fills the major and minor fields from
//! `rules::draw_order`, the unit composite fills `sub`, and the passes are named in [`pass`].

use super::item::DrawItem;
use super::SceneError;

/// Sort key of a draw item: `pass:4 | major:28 | minor:24 | sub:8`, most
/// significant first. Items with equal keys keep build order.
///
/// The pass numbers are listed in [`pass`]; the world's `major`/`minor` come from
/// `rules::draw_order`, `sub` from the unit composite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct DrawKey(pub u64);

impl DrawKey {
    /// Largest `pass` value (4 bits).
    pub const PASS_MAX: u32 = (1 << 4) - 1;
    /// Largest `major` value (28 bits).
    pub const MAJOR_MAX: u32 = (1 << 28) - 1;
    /// Largest `minor` value (24 bits).
    pub const MINOR_MAX: u32 = (1 << 24) - 1;

    /// Packs the four fields; a field out of range is an error, never
    /// truncated.
    pub fn new(pass: u32, major: u32, minor: u32, sub: u8) -> Result<Self, SceneError> {
        for (field, value, max) in [
            ("pass", pass, Self::PASS_MAX),
            ("major", major, Self::MAJOR_MAX),
            ("minor", minor, Self::MINOR_MAX),
        ] {
            if value > max {
                return Err(SceneError::KeyField { field, value, max });
            }
        }
        Ok(DrawKey(
            u64::from(pass) << 60 | u64::from(major) << 32 | u64::from(minor) << 8 | u64::from(sub),
        ))
    }

    /// The pass field (top 4 bits).
    pub fn pass(&self) -> u32 {
        (self.0 >> 60) as u32
    }

    /// The major field (28 bits).
    pub fn major(&self) -> u32 {
        (self.0 >> 32) as u32 & Self::MAJOR_MAX
    }

    /// The minor field (24 bits).
    pub fn minor(&self) -> u32 {
        (self.0 >> 8) as u32 & Self::MINOR_MAX
    }

    /// The key without its sub-slot: (pass, major, minor), one drawn
    /// unit's slot (its component cels differ by `sub` only).
    pub fn slot(&self) -> u64 {
        self.0 >> 8
    }

    /// The sub field (low 8 bits): the slot of a component within its unit.
    pub fn sub(&self) -> u8 {
        self.0 as u8
    }
}

/// The `pass` field of each frame pass, in drawing order.
pub mod pass {
    /// A background drawn behind the level; only levels 74 and 120 use it.
    pub const LEVEL_BACKGROUND: u32 = 1;
    /// Lower walls, drawn before the floors.
    pub const LOWER_WALLS: u32 = 2;
    /// Floor tiles.
    pub const FLOORS: u32 = 3;
    /// A pass whose purpose is not identified; the number is kept so the order stays fixed.
    pub const UNIDENTIFIED_4: u32 = 4;
    /// Shadows.
    pub const SHADOWS: u32 = 5;
    /// Walls and units, interleaved by cell.
    pub const WALLS_UNITS: u32 = 6;
    /// Roofs.
    pub const ROOFS: u32 = 7;
    /// A pass whose purpose is not identified; the number is kept so the order stays fixed.
    pub const UNIDENTIFIED_8: u32 = 8;
    /// A pass whose purpose is not identified; the number is kept so the order stays fixed.
    pub const UNIDENTIFIED_9: u32 = 9;
    /// The full-screen fade.
    pub const SCREEN_FADE: u32 = 10;
    /// Everything drawn after the world: the user interface.
    pub const UI: u32 = 11;
    /// The UI pass's majors: the automap draws before every panel.
    pub const UI_AUTOMAP_MAJOR: u32 = 0;
    /// The panels' draws (the UI root's list, in emission order).
    pub const UI_PANELS_MAJOR: u32 = 1;
}

/// Sorts items by key, stably: ties keep build order. The compositor
/// draws in list order and never reorders.
pub fn order(items: &mut [DrawItem]) {
    items.sort_by_key(|item| item.key);
}
