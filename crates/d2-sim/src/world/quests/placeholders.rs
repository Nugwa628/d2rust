// Spec: specs/world/objects-2.md §17 (init 46)
//! The trapped-soul placeholder object's init, `0x005506D0` (`InitFn`
//! 46, object 369): it scatters trapped souls (monster classes 403 and
//! 404) over its room while the object is still in mode 0. Every draw
//! comes from the object control's seed, in the original's order.

use super::act3::InitPoint;
use super::QuestWorld;
use crate::drlg::TileRect;
use crate::rng::Seed;
use crate::units::{RoomId, UnitId};

/// What the init needs of a world: the quest host's ([`QuestWorld`],
/// the blanket impl) or the object population's, which runs the init
/// inline while the object control is lent (`ObjectWorld::trapped_soul_init`).
pub trait SoulWorld {
    /// The object control's seed.
    fn soul_seed(&mut self) -> Option<&mut Seed>;
    fn soul_mode(&self, object: UnitId) -> i32;
    fn soul_set_mode(&mut self, object: UnitId, mode: i32);
    fn soul_box(&mut self, room: RoomId) -> Option<TileRect>;
    fn soul_collides(&mut self, room: RoomId, x: i32, y: i32, size: i32, mask: u32) -> bool;
    /// `0x005B2F20(room, x, y, class, mode 1, spread -1, flags 0)`.
    fn soul_spawn(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId>;
}

impl<W: QuestWorld> SoulWorld for W {
    fn soul_seed(&mut self) -> Option<&mut Seed> {
        self.object_seed()
    }
    fn soul_mode(&self, object: UnitId) -> i32 {
        self.object_mode(object)
    }
    fn soul_set_mode(&mut self, object: UnitId, mode: i32) {
        self.set_object_mode(object, mode)
    }
    fn soul_box(&mut self, room: RoomId) -> Option<TileRect> {
        self.room_box(room)
    }
    fn soul_collides(&mut self, room: RoomId, x: i32, y: i32, size: i32, mask: u32) -> bool {
        self.box_collides(room, x, y, size, mask)
    }
    fn soul_spawn(&mut self, room: RoomId, x: i32, y: i32, class: u16) -> Option<UnitId> {
        self.spawn_monster_flags(room, x, y, class, 1, -1, 0)
    }
}

/// The two trapped-soul monster classes the init picks between.
const SOUL_CLASSES: [u16; 2] = [403, 404];
/// Collision masks of the placement test (`0x00550220`).
const MASK_WALL_OBJECT_DOOR: u32 = 0x0C01;
const MASK_PLACE: u32 = 0x3F11;
/// Size of the souls' footprint in the placement test.
const SIZE: i32 = 3;
/// The eight neighbour steps `0x00731B7C` (x) and `0x00731B9C` (y).
const STEP_X: [i32; 8] = [-1, 0, 1, -1, 1, -1, 0, 1];
const STEP_Y: [i32; 8] = [-1, -1, -1, 0, 0, 1, 1, 1];

/// The placement test `0x00550220` for the point (x, y) of the room with
/// the sub-tile box (x0, y0, w, h) and a `SIZE` square footprint.
fn fits<W: SoulWorld>(
    w: &mut W,
    at: InitPoint,
    rect: (i32, i32, i32, i32),
    x: i32,
    y: i32,
) -> bool {
    let (x0, y0, rw, rh) = rect;
    let (x, y) = (i32::from(x as u16), i32::from(y as u16));
    rw >= SIZE + 2
        && rh >= SIZE + 2
        && x > x0 + 1
        && y > y0 + 1
        && x < x0 - SIZE - 2 + rw
        && y < y0 - SIZE - 2 + rh
        && !w.soul_collides(at.room, x, y, SIZE + 7, MASK_WALL_OBJECT_DOOR)
        && !w.soul_collides(at.room, x, y, SIZE, MASK_PLACE)
}

/// One soul: the class draw (one step, low bit), the monster at (x, y)
/// in mode 1 with spread -1, then, when it exists, the object to mode 2.
/// Returns whether a soul was made.
fn spawn<W: SoulWorld>(w: &mut W, object: UnitId, at: InitPoint, x: i32, y: i32) -> bool {
    let low = w.soul_seed().map_or(0, |s| s.step());
    let class = SOUL_CLASSES[(low & 1) as usize];
    if w.soul_spawn(at.room, x, y, class).is_none() {
        return false;
    }
    w.soul_set_mode(object, 2);
    true
}

/// `0x005506D0`: runs only in object mode 0. The soul budget is
/// ((w·h) >> 7) · 30 >> 8 of the room's sub-tile box; up to 12 random
/// points are tried. A point that fits spawns a soul at the object's own
/// position and starts a cluster; the cluster then grows with random
/// neighbour points (8 directions, 10–18 sub-tiles away).
pub fn trapped_soul_init<W: SoulWorld>(w: &mut W, object: UnitId, at: Option<InitPoint>) {
    if w.soul_mode(object) != 0 {
        return;
    }
    // A null room has the all-zero box: a zero budget.
    let Some(at) = at else { return };
    let Some(r) = w.soul_box(at.room) else { return };
    let rect = (r.x, r.y, r.w, r.h);
    let mut budget = (r.w.wrapping_mul(r.h) >> 7).wrapping_mul(30) >> 8;
    let mut tries = 12;
    while budget > 0 && tries > 0 {
        tries -= 1;
        let (px, py) = {
            let Some(seed) = w.soul_seed() else { return };
            let x = seed.roll(r.w - 4) as i32 + r.x;
            let y = seed.roll(r.h - 4) as i32 + r.y;
            (x, y)
        };
        if !fits(w, at, rect, px, py) {
            continue;
        }
        // The first soul goes to the object's own position.
        if spawn(w, object, at, at.x, at.y) {
            budget -= 1;
        }
        let (mut cx, mut cy) = (px, py);
        let mut count = 1;
        let mut last_fit = true;
        loop {
            let half = count >> 1;
            if half > 0 {
                let roll = w.soul_seed().map_or(0, |s| s.roll(half));
                if roll != 0 {
                    break;
                }
            }
            if !last_fit {
                break;
            }
            // Look for a neighbour point that fits.
            last_fit = false;
            let cap = budget.max(4) * 3;
            let mut n = 0;
            while n < cap {
                let (dir, rx, ry) = {
                    let Some(seed) = w.soul_seed() else { return };
                    let dir = (seed.step() & 7) as usize;
                    (dir, seed.step() % 5, seed.step() % 5)
                };
                cx += (rx as i32 + 5) * STEP_X[dir] * 2;
                cy += (ry as i32 + 5) * STEP_Y[dir] * 2;
                last_fit = fits(w, at, rect, cx, cy);
                n += 1;
                if last_fit {
                    break;
                }
            }
            if !last_fit {
                // The original re-enters the cluster head: its roll runs
                // once more, then the failed fit ends the cluster.
                continue;
            }
            count += 1;
            if spawn(w, object, at, cx, cy) {
                budget -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Seed;
    use crate::units::RoomId;
    use crate::world::quests::tests::Fake;

    const OBJ: UnitId = UnitId(7);

    fn fake(mode: i32) -> Fake {
        let mut f = Fake::default();
        f.objects.insert(OBJ, (1, 369, mode));
        f.rooms.insert(RoomId(1), (0, 0, 64, 64));
        f.object_seed = Some(Seed::new(1, 666));
        f.spawns = (0..40).map(|i| Some(UnitId(100 + i))).collect();
        f
    }

    fn at() -> Option<InitPoint> {
        Some(InitPoint {
            room: RoomId(1),
            x: 30,
            y: 31,
        })
    }

    // Covers: specs/world/objects-2.md §17
    #[test]
    fn spawns_souls_in_mode_0_first_at_the_object() {
        let mut f = fake(0);
        trapped_soul_init(&mut f, OBJ, at());
        let spawns: Vec<&String> = f.log.iter().filter(|l| l.starts_with("spawn")).collect();
        assert!(!spawns.is_empty());
        let first = spawns[0];
        assert!(
            first == "spawn 403 30 31 room 1 mode 1 spread -1 flags 0x0"
                || first == "spawn 404 30 31 room 1 mode 1 spread -1 flags 0x0",
            "{first}"
        );
        assert_eq!(f.objects[&OBJ].2, 2);
    }

    // Covers: specs/world/objects-2.md §17
    #[test]
    fn draws_nothing_outside_mode_0_or_without_a_room() {
        let mut f = fake(2);
        let seed = f.object_seed;
        trapped_soul_init(&mut f, OBJ, at());
        assert_eq!(f.object_seed, seed);
        assert!(f.log.iter().all(|l| !l.starts_with("spawn")));
        let mut f = fake(0);
        trapped_soul_init(&mut f, OBJ, None);
        assert_eq!(f.object_seed, seed);
        assert_eq!(f.objects[&OBJ].2, 0);
    }
}
