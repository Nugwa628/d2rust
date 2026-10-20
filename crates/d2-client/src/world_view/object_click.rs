//! The object interact reach of the `play` preview. Object clicks take the
//! same path as every unit click: the hover pick (`bridge::hover::pick`)
//! names the object, the walk-then-pend interaction (`world_view/interact.rs`)
//! sends C→S 0x13 once when the walk ends, and the server operates it
//! (`Pending::object_preview_range`). D2RUST's own, unverified (stitch-objects).

/// The server's interact reach in sub-tiles (`LocalSeams::object_preview_range`).
pub const INTERACT_RANGE: i32 = 5;
