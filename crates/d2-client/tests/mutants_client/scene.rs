//! Mutation-testing gaps of the scene and the CPU reference
//! compositor.

use d2_client::scene::{FrameView, MapTable};

/// A frame view hands out the bytes it was built from.
#[test]
fn frame_view_keeps_its_pixels() {
    let pixels = [0u8, 5, 7, 0, 9, 1];
    let view = FrameView::new(3, 2, &pixels).unwrap();
    assert_eq!(view.pixels(), &pixels);
    assert_eq!((view.width(), view.height()), (3, 2));
}

/// The map table is empty until a row is pushed.
#[test]
fn map_table_emptiness() {
    let mut maps = MapTable::new();
    assert!(maps.is_empty());
    maps.push([0; 256]);
    assert!(!maps.is_empty());
    assert_eq!(maps.len(), 1);
}
