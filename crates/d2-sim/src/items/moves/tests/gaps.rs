//! Gap tests of item-move rules no other test pinned.

use super::{Fake, P};
use crate::items::moves::{handle, HANDLED};

#[test]
fn transmogrify_0x4c_is_not_an_item_move() {
    // 0x4C belongs to the cube: not in the handler table, and
    // the dispatcher leaves it alone at any length.
    assert!(!HANDLED.iter().any(|&(id, _)| id == 0x4C));
    let mut f = Fake::new();
    for len in [1, 5, 17] {
        let mut m = vec![0u8; len];
        m[0] = 0x4C;
        assert!(handle(&mut f, P, &m).is_none(), "length {len}");
    }
}
