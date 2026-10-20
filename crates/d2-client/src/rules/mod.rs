//! The rules the world view draws by: answers to its hooks, one area of the game per
//! module. Plain Rust, integer math, no Bevy types.
//!
//! - [`camera`]: frame sizes and the play area, world to client pixels, the two camera
//!   origins of a frame, unit and tile draw positions, view culling, screen shake.
//! - [`placement`]: what a draw at (X, Y) covers, row clipping, DT1 blocks, and the
//!   `DrawItem` top-left of an `IndexFrame`.
//! - [`unit_composite`]: COF and component file names, directions and cels, component
//!   requests, colormap sources, COF box culling, extra offsets and single-cel units.
//! - [`unit_visibility`]: which units the client draws at all.
//! - [`view`]: [`view::OriginalView`], the `ViewRules` implementation that answers
//!   `tiles`, `unit_params` and `place` with the modules above and hands every other hook
//!   (pose, component frame, draw keys, shading, blend, UI) to the rules it wraps.
//! - [`draw_order`]: the draw-cell grid, its lists, the world passes and the draw keys of
//!   a frame, wired in through [`draw_order::source::OrderedSource`].
//! - [`shading`]: the act palette-table block as map-table rows (light maps, the computed
//!   highlight and red maps, remaps), cel light maps, DT1 wall and floor block light.
//! - [`blend`]: draw mode to blend table and the pixel ops of a cel (through
//!   `scene::PixelTables`), the draw modes of components, missiles, items and overlays,
//!   shadows, translucent walls.
//! - [`umod_hooks`]: the client hooks of monster modifiers and the order they run in.
//! - [`lighting`]: the light map, light records and sources, the day / night environment
//!   and the light value of each draw.

pub mod adaptive_integer;
pub mod blend;
pub mod camera;
pub mod draw_order;
pub mod lighting;
pub mod placement;
pub mod shading;
pub mod umod_hooks;
pub mod unit_composite;
pub mod unit_visibility;
pub mod view;
pub mod view_sync;

#[cfg(test)]
mod blend_gdi_tests;
#[cfg(test)]
mod gaps_numbered_tests;
#[cfg(test)]
mod shading_blend_tests;
#[cfg(test)]
mod shading_shift_tests;
#[cfg(test)]
mod tests;

pub use camera::{Camera, ClientPos, FrameSize, OpenMode, Shake, TileList, UnitPosition, ViewRect};
pub use placement::{Cel, Placed, RowPlan};
pub use view::{BlockRect, BlockShade, MapTile, OriginalView, ViewSource};
