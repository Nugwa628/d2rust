//! The player position history (player data +0xA0 next index, +0xA4
//! time of the last write, +0xA8: 20 × {u32 x, u32 y}, a ring). Written
//! by the placement (unconditionally) and by the walk step
//! (when the player moved far enough). Monster AI reads it,
//! so it lives in
//! the simulation.
//!
//! The walk step's only wall-clock input, the 25 ms gate on +0xA4, is
//! read as always open (the walk step runs
//! at most once per player per tick and ticks are 40 ms apart), so the
//! time field is not kept.

/// Ring length (player data +0xA8: 20 entries).
pub const HISTORY_LEN: usize = 20;
/// The walk step writes when the squared distance to the newest entry is
/// above this (dx² + dy²).
pub const WALK_MIN_DIST_SQ: i32 = 45;

/// The ring ([module doc](self)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PositionHistory {
    /// +0xA0: the next index to write (u8, 0..20).
    pub next: u8,
    /// +0xA8 + 8·i: {x, y} in sub-tiles.
    pub entries: [(u32, u32); HISTORY_LEN],
}

impl PositionHistory {
    fn push(&mut self, x: u32, y: u32) {
        let i = usize::from(self.next) % HISTORY_LEN;
        self.entries[i] = (x, y);
        self.next = ((i + 1) % HISTORY_LEN) as u8;
    }

    /// The newest entry (index − 1, 0 → 19).
    pub fn newest(&self) -> (u32, u32) {
        let p = (usize::from(self.next) + HISTORY_LEN - 1) % HISTORY_LEN;
        self.entries[p]
    }

    /// Placement write: entry
    /// := (x, y) unconditionally; index + 1 (20 → 0).
    pub fn place_write(&mut self, x: i32, y: i32) {
        self.push(x as u32, y as u32);
    }

    /// Walk-step write
    /// (walk step 5): when dx² + dy² from the newest entry
    /// to (x, y) is above 45 (32-bit arithmetic, signed compare), entry
    /// := (x, y). Returns whether it wrote.
    pub fn walk_write(&mut self, x: i32, y: i32) -> bool {
        let (px, py) = self.newest();
        let dx = x.wrapping_sub(px as i32);
        let dy = y.wrapping_sub(py as i32);
        let d = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
        if d > WALK_MIN_DIST_SQ {
            self.push(x as u32, y as u32);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_writes_and_the_walk_distance_gate() {
        let mut h = PositionHistory::default();
        // Placement writes unconditionally, even the same point.
        h.place_write(10, 10);
        h.place_write(10, 10);
        assert_eq!(h.next, 2);
        assert_eq!(h.newest(), (10, 10));
        // Walk: d² = 36 + 9 = 45 is not above 45; 49 + 0 is.
        assert!(!h.walk_write(16, 13));
        assert_eq!(h.next, 2);
        assert!(h.walk_write(17, 10));
        assert_eq!((h.next, h.newest()), (3, (17, 10)));
        // The ring wraps at 20; the newest of index 0 is entry 19.
        for i in 0..17 {
            h.place_write(100 + i, 5);
        }
        assert_eq!(h.next, 0);
        assert_eq!(h.newest(), (116, 5));
        h.place_write(1, 2);
        assert_eq!((h.next, h.entries[0]), (1, (1, 2)));
    }
}
