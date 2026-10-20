//! The Horadric Cube as a made-up item (`box `, a 2 × 4 `misc` row of its
//! own item type) for tests that need a character to carry one. Added on
//! top of [`crate::synth::synthetic`] by a caller that wants it, so the
//! shared set (and the counts other tests assert on) stays unchanged.

use crate::synth::TableSet;

/// Appends the cube's item type and `misc` row to `t`. Call it before
/// the install is built (`install::build`).
pub fn add_cube_item(t: &mut TableSet) {
    t.row(
        "itemtypes",
        &[
            ("code", "box"),
            ("equiv1", "misc"),
            ("storepage", "misc"),
            ("body", "0"),
            ("normal", "1"),
        ],
    );
    t.row(
        "misc",
        &[
            ("code", "box"),
            ("namestr", "box"),
            ("type", "box"),
            ("level", "1"),
            ("invwidth", "2"),
            ("invheight", "4"),
            ("stackable", "0"),
            ("cost", "0"),
        ],
    );
}
