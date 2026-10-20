//! Property tests of the path wiring on the wired sim: `wiring::path`
//! over [`WorldSim`] with the synthetic Act I fixture (the worldgen
//! fixture: act 0's DRLG through the level-type dispatcher, the ISLE
//! preset level generated and all 15 rooms streamed, population on).
//! Random sequences of C→S walk / run requests (point and unit forms;
//! points anywhere within 50 or next to another unit), ticks, monster
//! placements and same-act level warps, one or two players with clients,
//! random static cells, monstats2 `SizeX` 0..=3, keep:
//!
//! 1. every player's and monster's footprint lies on sub-tiles free of
//!    its move mask in the static grid;
//! 2. the collision grids equal a reference rebuilt from the static grid
//!    and every unit's footprint: no bit without an owner;
//!    every owner's bit present where no other unit's footprint touched
//!    the cell since (clears AND the complement, so a shared bit is
//!    lost when either owner clears it: those cells are only checked
//!    for ghosts);
//! 3. the NO_PATH / PET markers of two units never share a cell, and a
//!    unit that moved in a tick holds no other unit's marker in its
//!    pattern, as tested by the move mask;
//! 4. per tick a unit moves at most its velocity vector per axis,
//!    16·velocity, plus half a sub-tile when blocked movement or a reset
//!    ends on a cell centre;
//! 5. the unit's room is active, contains its cell, equals its room
//!    list entry; a change in a tick goes to the old room or one of its
//!    adjacent rooms, keeps the old room as the previous
//!    room and the room-change messages clear flag 0x2;
//! 6. a warp places the player in an active room of the
//!    destination level that contains it, on a free point, with flags 2
//!    0x10000 and event 14 at frame + 50;
//! 7. two runs of the same input give identical digests after every
//!    step; no panic, overflow (debug build) or wiring error.
//!
//! Regression cases pin warp stamping from the destination room and two
//! limits on the checks: the fixture's no-op compression hook leaves stale
//! monster records after room deactivation, and coarse search can return
//! a room that does not contain its point. The model excludes these cases
//! from the affected checks; inactive-storage behavior remains untested.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use d2_sim::rng::Seed;
use std::sync::Arc;

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};

use d2_sim::bench_fixtures::{ds1, Ds1s, Fx, ISLE, ISLE_DEF};
use d2_sim::drlg::collision::bits;
use d2_sim::path::{CollisionRooms, DynamicPath, UnitPath};
use d2_sim::units::lifecycle::AllocRequest;
use d2_sim::units::lists::client_state;
use d2_sim::units::{RoomId, UnitId, UnitType};
use d2_sim::wiring::path::{place, walk, PathCtx};

/// Stat IDs for `velocitypercent` and `stamina`.
const STAT_VELOCITY: u16 = 67;
const STAT_STAMINA: u16 = 10;
/// ISLE's sub-tile box: tile (8000, 8000), 40 × 18 tiles.
const X0: i32 = 40000;
const Y0: i32 = 40000;
const W: i32 = 200;
const H: i32 = 90;
/// Player spawn points (kept wall-free).
const SPAWNS: [(i32, i32); 2] = [(40020, 40020), (40150, 40060)];
/// Bits only footprints write.
const UNIT_BITS: u16 =
    bits::PLAYER | bits::MONSTER | bits::ITEM | bits::NO_PATH | bits::PET | bits::CORPSE;
/// Player move mask.
const PLAYER_MOVE: u16 = 0x1C09;
/// Population's coarse search mask.
const MONSTER_PLACE: u32 = 0x3C01;

fn cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn runner(default: u32) -> TestRunner {
    TestRunner::new(Config {
        cases: cases(default),
        failure_persistence: None,
        ..Config::default()
    })
}

// ---- input -------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Setup {
    players: usize,
    /// monstats2 `SizeX` of class 0 (path size): 0..=3.
    monster_size: u8,
    /// Stamina (8.8) of the players: 0 makes runs walks.
    stamina: i32,
    /// Static cells: (x offset, y offset, bits).
    walls: Vec<(i32, i32, u16)>,
}

#[derive(Clone, Debug)]
enum Op {
    /// C→S 0x01 / 0x03 to (player + dx, player + dy), |d| ≤ 50.
    Walk {
        who: usize,
        run: bool,
        dx: i32,
        dy: i32,
    },
    /// C→S 0x02 / 0x04 to a unit in the same act, within 50.
    WalkUnit {
        who: usize,
        run: bool,
        target: usize,
    },
    /// C→S 0x01 / 0x03 to a point next to another unit (dx, dy ∈ −2..=2
    /// from it; within 50 of the player): moves that
    /// meet other footprints.
    WalkNear {
        who: usize,
        run: bool,
        target: usize,
        dx: i32,
        dy: i32,
    },
    Ticks(u32),
    /// A monster at the coarse free box (n 1, mask 0x3C01) around
    /// the point, using population's placement parameters.
    Monster {
        x: i32,
        y: i32,
    },
    /// Same-act level warp to ISLE, tile index 0.
    Warp {
        who: usize,
    },
}

fn setup() -> impl Strategy<Value = Setup> {
    (
        1usize..=2,
        0u8..=3,
        prop_oneof![Just(0), Just(1 << 8), Just(100 << 8)],
        prop::collection::vec(
            (
                0..W,
                0..H,
                prop_oneof![
                    4 => Just(bits::WALL),
                    1 => Just(bits::MISSILE_BARRIER),
                    1 => Just(bits::NOPLAYER),
                ],
            ),
            0..400,
        ),
    )
        .prop_map(|(players, monster_size, stamina, walls)| Setup {
            players,
            monster_size,
            stamina,
            walls,
        })
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (0usize..2, any::<bool>(), -50i32..=50, -50i32..=50)
            .prop_map(|(who, run, dx, dy)| Op::Walk { who, run, dx, dy }),
        1 => (0usize..2, any::<bool>(), 0usize..8)
            .prop_map(|(who, run, target)| Op::WalkUnit { who, run, target }),
        3 => (0usize..2, any::<bool>(), 0usize..8, -2i32..=2, -2i32..=2).prop_map(
            |(who, run, target, dx, dy)| Op::WalkNear {
                who,
                run,
                target,
                dx,
                dy
            }
        ),
        8 => (1u32..=30).prop_map(Op::Ticks),
        2 => (0..W, 0..H).prop_map(|(x, y)| Op::Monster { x: X0 + x, y: Y0 + y }),
        1 => (0usize..2).prop_map(|who| Op::Warp { who }),
    ]
}

// ---- cells -------------------------------------------------------------------

/// A value per sub-tile of ISLE's box; `None`: no active room there.
#[derive(Clone, PartialEq)]
struct Cells(Vec<Option<u16>>);

impl Cells {
    fn empty() -> Self {
        Cells(vec![None; (W * H) as usize])
    }
    fn index(x: i32, y: i32) -> Option<usize> {
        let (dx, dy) = (x - X0, y - Y0);
        ((0..W).contains(&dx) && (0..H).contains(&dy)).then(|| (dy * W + dx) as usize)
    }
    fn get(&self, x: i32, y: i32) -> Option<u16> {
        self.0[Self::index(x, y)?]
    }
    fn get_mut(&mut self, x: i32, y: i32) -> Option<&mut u16> {
        self.0[Self::index(x, y)?].as_mut()
    }
    fn iter(&self) -> impl Iterator<Item = ((i32, i32), u16)> + '_ {
        self.0.iter().enumerate().filter_map(|(i, v)| {
            let i = i as i32;
            Some(((X0 + i % W, Y0 + i / W), (*v)?))
        })
    }
}

// ---- the host ------------------------------------------------------------------

/// The fixture with the path provider on (charstats walk 6 / run 9,
/// vector V1), ISLE generated and streamed, the static cells staged and
/// the players allocated at their spawn points.
struct Host {
    fx: Fx,
    players: Vec<UnitId>,
    /// The static grid (before any unit): (x, y) → bits.
    base: Cells,
}

fn host(s: &Setup) -> Host {
    let mut m = BTreeMap::new();
    m.insert(
        format!("def{ISLE_DEF}.ds1").into_bytes(),
        ds1(40, 18, &[(0, 12, 10)]),
    );
    // One monstats2 `SizeX` for population's placement test
    // and for the path size: the fixture keeps three copies
    // (world tables, population's typed rows, the action tables).
    let mut fx = Fx::with_tables(Ds1s(m), |t| {
        let ex = usize::from(t.monstats[0].monstatsex);
        t.monstats2[ex].sizex = s.monster_size;
        t.pop.monstats2[ex].size_x = s.monster_size as i8;
    });
    fx.sim.create_regions();
    {
        let h = fx.sim.action.hooks();
        h.enable_paths().expect("embedded tables");
        let t = Arc::make_mut(&mut h.tables);
        t.combat.charstats[0].walkvelocity = 6;
        t.combat.charstats[0].runvelocity = 9;
        let ex = usize::from(t.combat.monstats[0].monstatsex);
        t.combat.monstats2[ex].sizex = s.monster_size;
    }
    let (_, drlg_rooms) = fx.generate(ISLE).expect("ISLE");
    let rooms = fx.stream(&drlg_rooms).expect("streamed");
    // Static cells, away from the spawn points.
    for &(dx, dy, b) in &s.walls {
        let (x, y) = (X0 + dx, Y0 + dy);
        if SPAWNS
            .iter()
            .any(|&(sx, sy)| (x - sx).abs() <= 2 && (y - sy).abs() <= 2)
        {
            continue;
        }
        let r = room_at(&fx, &rooms, x, y).expect("in ISLE");
        *fx.sim
            .action
            .hooks()
            .drlg
            .grid_mut(r)
            .unwrap()
            .get_mut(x, y)
            .unwrap() |= b;
    }
    let base = grid(&fx, &rooms);
    let mut players = Vec::new();
    for &(x, y) in &SPAWNS[..s.players] {
        let room = room_at(&fx, &rooms, x, y).unwrap();
        let req = AllocRequest {
            ty: UnitType::Player,
            class: 0,
            room: Some(room),
            add: true,
            fixed_guid: None,
            mode: 1,
            allied: true,
        };
        let p = fx
            .sim
            .action
            .with(&mut fx.game, |g, v| {
                // Players start in mode 0; the player mode starts are
                // not run here: neutral, set before `SUNIT_Add` so the
                // dead-body path settings do not apply (as the action
                // fixture does).
                let u = v.allocate_unadded(g, &req)?;
                v.units.get_mut(u).unwrap().mode = 1;
                v.add_allocated(g, u, &req, x, y).then_some(u)
            })
            .expect("allocated");
        fx.sim.action.with(&mut fx.game, |_, v| {
            v.set_base(p, STAT_VELOCITY, 100);
            v.set_base(p, STAT_STAMINA, s.stamina);
        });
        // The client pass moves the player's client into the player's room,
        // keeping the surrounding rooms active.
        fx.game
            .lists
            .add_client(Some(p), None, client_state::IN_GAME);
        players.push(p);
    }
    Host { fx, players, base }
}

impl Host {
    /// The static grid follows the act's active rooms (the tick's room
    /// pass streams rooms in and out): cells of rooms gone are dropped;
    /// a room streamed in again was rebuilt from its tiles, so its static
    /// bits are taken as they are now and its cells
    /// are tainted (footprints of units standing next to it were not
    /// stamped into it).
    fn refresh(&mut self, m: &mut Model) {
        let now = grid(&self.fx, &active(&self.fx));
        for (i, (b, n)) in self.base.0.iter_mut().zip(&now.0).enumerate() {
            match (*b, *n) {
                (_, None) => *b = None,
                (None, Some(v)) => {
                    *b = Some(v & !UNIT_BITS);
                    let i = i as i32;
                    m.tainted.insert((X0 + i % W, Y0 + i / W));
                }
                (Some(_), Some(_)) => {}
            }
        }
    }
}

/// Act 0's active rooms.
fn active(fx: &Fx) -> Vec<RoomId> {
    fx.game.lists.active_rooms(0)
}

fn room_at(fx: &Fx, rooms: &[RoomId], x: i32, y: i32) -> Option<RoomId> {
    let d = &fx.sim.action.sys.hooks.drlg;
    rooms
        .iter()
        .copied()
        .find(|&r| d.subtile_rect(r).is_some_and(|t| t.contains(x, y)))
}

/// Every cell of the active rooms.
fn grid(fx: &Fx, rooms: &[RoomId]) -> Cells {
    let d = &fx.sim.action.sys.hooks.drlg;
    let mut out = Cells::empty();
    for &r in rooms {
        let Some(g) = d.grid(r) else { continue };
        let t = g.rect;
        for y in t.y..t.y + t.h {
            for x in t.x..t.x + t.w {
                let i = Cells::index(x, y).expect("an ISLE room");
                out.0[i] = g.get(x, y);
            }
        }
    }
    out
}

fn dynamic(fx: &Fx, u: UnitId) -> Option<&DynamicPath> {
    fx.sim.action.sys.hooks.paths.as_ref()?.dynamic(u)
}

fn tick(fx: &mut Fx) {
    d2_sim::tick::tick(&mut fx.game, &mut fx.sim);
}

// ---- the reference footprint ------------------------------------------------

/// What a unit's pattern stamps: (x, y, bits) per cell.
fn footprint(pattern: u32, mask: u16, x: i32, y: i32) -> Vec<(i32, i32, u16)> {
    let plus = [(0, 0), (-1, 0), (1, 0), (0, -1), (0, 1)];
    let boxed: Vec<(i32, i32)> = (-1..=1)
        .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
        .collect();
    type Shape<'a> = (Vec<(i32, i32)>, u16, &'a [(i32, i32)]);
    let (cells, marker, marked): Shape<'_> = match pattern {
        0 => (vec![(0, 0)], 0, &[]),
        1 => (plus.to_vec(), bits::NO_PATH, &plus[..1]),
        2 => (boxed, bits::NO_PATH, &plus[..]),
        3 => (plus.to_vec(), bits::PET, &plus[..1]),
        4 => (boxed, bits::PET, &plus[..]),
        5 => (plus.to_vec(), 0, &[]),
        p => panic!("pattern {p} outside §3's table"),
    };
    let mut out: Vec<(i32, i32, u16)> = cells
        .iter()
        .map(|&(dx, dy)| (x + dx, y + dy, mask))
        .collect();
    if mask != 0 && marker != 0 {
        for c in out.iter_mut() {
            if marked.contains(&(c.0 - x, c.1 - y)) {
                c.2 |= marker;
            }
        }
    }
    out
}

/// A unit's footprint state as the property sees it.
#[derive(Clone, Debug, PartialEq)]
struct Foot {
    room: Option<RoomId>,
    x: i32,
    y: i32,
    precise: (u32, u32),
    velocity: i32,
    size: i32,
    pattern: u32,
    mask: u16,
    move_mask: u16,
    flags: u32,
}

fn feet(fx: &Fx) -> BTreeMap<UnitId, Foot> {
    let Some(p) = fx.sim.action.sys.hooks.paths.as_ref() else {
        return BTreeMap::new();
    };
    p.records
        .iter()
        .filter_map(|(&u, r)| match r {
            UnitPath::Dynamic(d) => Some((
                u,
                Foot {
                    room: d.room,
                    x: d.x(),
                    y: d.y(),
                    precise: (d.precise_x, d.precise_y),
                    velocity: d.velocity,
                    size: d.unit_size,
                    pattern: d.pattern,
                    mask: d.foot_mask,
                    move_mask: d.move_mask,
                    flags: d.flags,
                },
            )),
            UnitPath::Static(_) => None,
        })
        .collect()
}

impl Foot {
    fn cells(&self) -> Vec<(i32, i32, u16)> {
        // Pattern 0 stamps nothing.
        if self.pattern == 0 {
            return Vec::new();
        }
        footprint(self.pattern, self.mask, self.x, self.y)
    }

    /// The size shape used by placement collision checks:
    /// 0 and 1 test the cell, 2 the plus, 3 the box.
    fn size_cells(&self) -> Vec<(i32, i32)> {
        let shape = match self.size {
            0 | 1 => 0,
            2 => 1,
            3 => 2,
            s => panic!("size {s}: §4 rule 5 collides everywhere"),
        };
        footprint(shape, 0, self.x, self.y)
            .into_iter()
            .map(|(x, y, _)| (x, y))
            .collect()
    }

    /// The move mask of the try move: 0x3401 → 0x3C01.
    fn test_mask(&self) -> u16 {
        if self.move_mask == 0x3401 {
            0x3C01
        } else {
            self.move_mask
        }
    }
}

/// Cells whose unit bits may be missing (a shared bit cleared by
/// another owner), and units whose footprint is not known to be
/// stamped (none since teleport stamps from the destination
/// room; the set stays for the checks that skip such units).
#[derive(Default)]
struct Model {
    tainted: BTreeSet<(i32, i32)>,
    unstamped: BTreeSet<UnitId>,
    /// Monsters whose room the tick's step 9 deactivated: the fixture's
    /// inactive-storage compression hook is a no-op, so they keep a stale
    /// path record and are left out of every check from then on, except
    /// that their last footprint may stay in a neighbouring room's grid.
    compressed: BTreeMap<UnitId, Foot>,
    /// Monsters placed by the coarse search into a room that does not
    /// hold their cell (coarse search tests the row against the rect last
    /// read: `regress_coarse_box_room_need_not_hold_the_point`), until
    /// they change cell and recache the room.
    outside: BTreeSet<UnitId>,
}

impl Model {
    /// The footprints after a step, without compressed monsters; a
    /// monster whose room did not change but is no longer active joins
    /// them.
    fn observe(&mut self, fx: &Fx, prev: &BTreeMap<UnitId, Foot>) -> BTreeMap<UnitId, Foot> {
        let act = active(fx);
        let mut now = feet(fx);
        for (u, f) in &now {
            let gone = f.room.is_some_and(|r| !act.contains(&r));
            let same = prev.get(u).is_some_and(|b| b.room == f.room);
            let monster = fx
                .game
                .lists
                .unit(*u)
                .is_some_and(|e| e.ty == UnitType::Monster);
            if gone && same && monster {
                self.compressed.entry(*u).or_insert_with(|| f.clone());
            }
        }
        now.retain(|u, _| !self.compressed.contains_key(u));
        now
    }

    /// One step from `before` to `after`. A cell touched by two units'
    /// footprints (before or after) is tainted. A cell touched by one
    /// unit that was placed, moved to another cell or removed in the
    /// step (its last write there is its own stamp or clear), or by no
    /// unit, is exact again; otherwise it keeps its state.
    fn step(&mut self, before: &BTreeMap<UnitId, Foot>, after: &BTreeMap<UnitId, Foot>) {
        let mut touch: BTreeMap<(i32, i32), BTreeSet<UnitId>> = BTreeMap::new();
        for map in [before, after] {
            for (&u, f) in map {
                for (x, y, _) in f.cells() {
                    touch.entry((x, y)).or_default().insert(u);
                }
            }
        }
        self.tainted.retain(|c| touch.contains_key(c));
        for (c, us) in touch {
            if us.len() >= 2 {
                self.tainted.insert(c);
                continue;
            }
            let u = *us.iter().next().unwrap();
            let rewritten = match (before.get(&u), after.get(&u)) {
                (Some(b), Some(a)) => (b.x, b.y) != (a.x, a.y),
                _ => true,
            };
            if rewritten && !self.unstamped.contains(&u) {
                self.tainted.remove(&c);
            }
        }
    }
}

// ---- the checks ----------------------------------------------------------------------

fn check_grid(
    h: &Host,
    m: &Model,
    ft: &BTreeMap<UnitId, Foot>,
    at: &str,
) -> Result<(), TestCaseError> {
    let fx = &h.fx;
    let actual = grid(fx, &active(fx));
    let mut expect = h.base.clone();
    let mut owners: BTreeMap<(i32, i32), Vec<(UnitId, u16)>> = BTreeMap::new();
    let mut want = vec![0u16; (W * H) as usize];
    for (&u, f) in ft {
        for (x, y, b) in f.cells() {
            if let Some(v) = expect.get_mut(x, y) {
                *v |= b;
                owners.entry((x, y)).or_default().push((u, b));
                if !m.unstamped.contains(&u) {
                    want[Cells::index(x, y).unwrap()] |= b;
                }
            }
        }
    }
    let mut allowed = expect.clone();
    for f in m.compressed.values() {
        for (x, y, b) in f.cells() {
            if let Some(v) = allowed.get_mut(x, y) {
                *v |= b;
            }
        }
    }
    for (c, a) in actual.iter() {
        let base = h.base.get(c.0, c.1).expect("base follows the rooms");
        prop_assert_eq!(
            a & !UNIT_BITS,
            base & !UNIT_BITS,
            "{}: static bits of {:?} changed",
            at,
            c
        );
        let e = allowed.get(c.0, c.1).unwrap();
        // No bit without an owner (each clear removes what its
        // stamp wrote).
        prop_assert_eq!(
            a & UNIT_BITS & !e,
            0,
            "{}: cell {:?} = {:#x}, reference {:#x} (owners {:?})",
            at,
            c,
            a,
            e,
            owners.get(&c)
        );
        let want = want[Cells::index(c.0, c.1).unwrap()];
        if want != 0 && !m.tainted.contains(&c) {
            prop_assert_eq!(
                a & want,
                want,
                "{}: cell {:?} = {:#x} misses {:#x} (owners {:?})",
                at,
                c,
                a,
                want,
                owners.get(&c)
            );
        }
    }
    // Property 1: the size shape used by searches and population placement
    // on cells free of the move mask in the static grid
    // (a pattern 1 footprint of size 1 is wider than its test);
    // property 3: markers never shared.
    let mut markers: BTreeMap<(i32, i32), UnitId> = BTreeMap::new();
    for (&u, f) in ft {
        for (x, y) in f.size_cells() {
            let s = h.base.get(x, y);
            let mm = f.test_mask();
            prop_assert!(
                s.is_none_or(|s| s & mm & !UNIT_BITS == 0),
                "{}: unit {:?} at ({}, {}) size {} covers ({}, {}) = {:?}, move mask {:#x}",
                at,
                u,
                f.x,
                f.y,
                f.size,
                x,
                y,
                s,
                mm
            );
        }
        // A unit warped without its stamp (`Model::unstamped`) has no
        // marker in the grid, so placements and moves cannot see it
        // (`regress_second_warp_sees_the_first_player`).
        if m.unstamped.contains(&u) {
            continue;
        }
        for (x, y, b) in f.cells() {
            if b & (bits::NO_PATH | bits::PET) != 0 {
                if let Some(o) = markers.insert((x, y), u) {
                    prop_assert!(
                        false,
                        "{}: units {:?} and {:?} share the marker cell ({}, {})",
                        at,
                        o,
                        u,
                        x,
                        y
                    );
                }
            }
        }
    }
    Ok(())
}

/// Property 5 for every unit, after any step.
fn check_rooms(
    h: &Host,
    m: &Model,
    ft: &BTreeMap<UnitId, Foot>,
    at: &str,
) -> Result<(), TestCaseError> {
    let fx = &h.fx;
    let d = &fx.sim.action.sys.hooks.drlg;
    let active = fx.game.lists.active_rooms(0);
    for (&u, f) in ft {
        let room = f.room;
        prop_assert!(room.is_some(), "{}: unit {:?} without a room", at, u);
        let r = room.unwrap();
        prop_assert!(active.contains(&r), "{}: {:?} in inactive {:?}", at, u, r);
        prop_assert!(
            m.outside.contains(&u) || d.subtile_rect(r).is_some_and(|t| t.contains(f.x, f.y)),
            "{}: {:?} at ({}, {}) outside its room {:?} {:?}",
            at,
            u,
            f.x,
            f.y,
            r,
            d.subtile_rect(r)
        );
        let listed = fx.game.lists.unit(u).and_then(|e| e.room());
        prop_assert_eq!(listed, room, "{}: {:?} list room", at, u);
        prop_assert!(fx.game.lists.room_units(r).contains(&u));
        prop_assert_eq!(f.flags & 0x2, 0, "{}: {:?} room-change flag left", at, u);
    }
    Ok(())
}

/// Properties 3 (movers) and 4, 5 (room change) across one tick.
fn check_tick(
    h: &Host,
    m: &Model,
    before: &BTreeMap<UnitId, Foot>,
    after: &BTreeMap<UnitId, Foot>,
    at: &str,
) -> Result<(), TestCaseError> {
    let d = &h.fx.sim.action.sys.hooks.drlg;
    for (u, a) in after {
        let Some(b) = before.get(u) else { continue };
        let v = i64::from(b.velocity.max(a.velocity).max(0));
        let centre = a.precise.0 & 0xFFFF == 0x8000 && a.precise.1 & 0xFFFF == 0x8000;
        let bound = 16 * v + if centre { 0x8000 } else { 0 };
        for (p, q) in [(b.precise.0, a.precise.0), (b.precise.1, a.precise.1)] {
            let delta = (i64::from(q) - i64::from(p)).abs();
            prop_assert!(
                delta <= bound,
                "{}: {:?} moved {:#x} > {:#x} (velocity {:#x}, {:?} → {:?})",
                at,
                u,
                delta,
                bound,
                v,
                b.precise,
                a.precise
            );
        }
        if a.room != b.room {
            let old = b.room.unwrap();
            let n = d.adjacent_count(old);
            let adj: Vec<_> = (0..n).filter_map(|i| d.adjacent(old, i)).collect();
            prop_assert!(
                adj.contains(&a.room.unwrap()),
                "{}: {:?} changed room {:?} → {:?}, not adjacent ({:?})",
                at,
                u,
                b.room,
                a.room,
                adj
            );
            let prev = dynamic(&h.fx, *u).unwrap().prev_room;
            prop_assert_eq!(prev, b.room, "{}: {:?} previous room", at, u);
        }
        if (a.x, a.y) != (b.x, b.y) {
            // A try move tested the new pattern with the move mask
            // (NO_PATH / PET in it): no other unit's marker inside.
            let mine: BTreeSet<_> = a.cells().iter().map(|&(x, y, _)| (x, y)).collect();
            // The try move tested the pattern with the move mask;
            // a cell without a room collides.
            let act = active(&h.fx);
            for &(x, y) in &mine {
                prop_assert!(
                    room_at(&h.fx, &act, x, y).is_some(),
                    "{}: {:?} moved to ({}, {}) with ({}, {}) outside the active rooms",
                    at,
                    u,
                    a.x,
                    a.y,
                    x,
                    y
                );
                let s = h.base.get(x, y);
                prop_assert!(
                    s.is_none_or(|s| s & a.test_mask() & !UNIT_BITS == 0),
                    "{}: {:?} moved to ({}, {}) whose pattern covers ({}, {}) = {:?}",
                    at,
                    u,
                    a.x,
                    a.y,
                    x,
                    y,
                    s
                );
            }
            for (w, o) in after {
                if w == u
                    || m.unstamped.contains(w)
                    || a.move_mask & (bits::NO_PATH | bits::PET) == 0
                {
                    continue;
                }
                for (x, y, ob) in o.cells() {
                    let hit = ob & a.move_mask & (bits::NO_PATH | bits::PET) != 0;
                    prop_assert!(
                        !(hit && mine.contains(&(x, y))),
                        "{}: {:?} moved to ({}, {}) over {:?}'s marker ({}, {})",
                        at,
                        u,
                        a.x,
                        a.y,
                        w,
                        x,
                        y
                    );
                }
            }
        }
    }
    Ok(())
}

fn clean(h: &Host, at: &str) -> Result<(), TestCaseError> {
    let e = h.fx.sim.errors();
    prop_assert!(e.is_empty(), "{}: wiring errors {:?}", at, e);
    Ok(())
}

// ---- running a case --------------------------------------------------------------------

/// One unit's observable state: list room, mode, flags 2, seed, timers.
type UnitState = (
    UnitId,
    Option<RoomId>,
    u32,
    u32,
    Seed,
    Vec<((u8, u32, u32), i32)>,
);

/// Everything observable after a step: frame, game seed, path records,
/// grids, unit rooms, modes, flags, seeds and timers, errors.
#[derive(PartialEq)]
struct Digest {
    frame: i32,
    seed: Seed,
    records: BTreeMap<UnitId, UnitPath>,
    grid: Cells,
    units: Vec<UnitState>,
    errors: Vec<String>,
}

impl std::fmt::Debug for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut g = DefaultHasher::new();
        self.grid.0.hash(&mut g);
        write!(
            f,
            "frame {} seed {:?} records {:?} grid {:#x} units {:?} errors {:?}",
            self.frame,
            self.seed,
            self.records,
            g.finish(),
            self.units,
            self.errors
        )
    }
}

fn digest(h: &Host) -> Digest {
    let fx = &h.fx;
    let mut units: Vec<UnitId> = [UnitType::Player, UnitType::Monster]
        .into_iter()
        .flat_map(|t| fx.game.lists.units_of_type(t))
        .collect();
    units.sort();
    let units = units
        .into_iter()
        .map(|u| {
            let r = fx.sim.action.sys.units.get(u).unwrap();
            let mut t: Vec<_> = fx
                .game
                .timers
                .unit_timers(u)
                .into_iter()
                .filter_map(|t| Some((fx.game.timers.event(t)?, fx.game.timers.expire(t)?)))
                .collect();
            t.sort();
            let room = fx.game.lists.unit(u).and_then(|e| e.room());
            (u, room, r.mode, r.flags2, r.seed, t)
        })
        .collect();
    Digest {
        frame: fx.game.frame,
        seed: fx.sim.action.sys.hooks.game_seed,
        records: fx
            .sim
            .action
            .sys
            .hooks
            .paths
            .as_ref()
            .map(|p| p.records.clone())
            .unwrap_or_default(),
        grid: grid(fx, &active(fx)),
        units,
        errors: fx.sim.errors(),
    }
}

/// Same-act level warp of `p` to ISLE, tile index 0.
fn warp(h: &mut Host, p: UnitId) -> Option<bool> {
    h.fx.sim.action.with(&mut h.fx.game, |g, v| {
        place::level_warp(PathCtx::of(v, g), p, ISLE, 0)
    })
}

/// Allocates a monster of class 0 at (x, y) in the room holding it.
fn monster(h: &mut Host, x: i32, y: i32) -> UnitId {
    let room = room_at(&h.fx, &active(&h.fx), x, y).expect("active");
    let req = AllocRequest {
        ty: UnitType::Monster,
        class: 0,
        room: Some(room),
        add: true,
        fixed_guid: None,
        mode: 1,
        allied: false,
    };
    h.fx.sim
        .action
        .with(&mut h.fx.game, |g, v| v.allocate(g, &req, x, y))
        .expect("allocated")
}

/// Runs one case; the digest after every step.
fn run(s: &Setup, ops: &[Op]) -> Result<Vec<Digest>, TestCaseError> {
    let mut h = host(s);
    let mut m = Model::default();
    let mut out = vec![digest(&h)];
    let mut prev = m.observe(&h.fx, &BTreeMap::new());
    m.step(&BTreeMap::new(), &prev);
    h.refresh(&mut m);
    check_grid(&h, &m, &prev, "setup")?;
    check_rooms(&h, &m, &prev, "setup")?;
    clean(&h, "setup")?;
    let quiet = Op::Ticks(60);
    for (i, op) in ops.iter().chain(std::iter::once(&quiet)).enumerate() {
        let at = format!("op {i} {op:?}");
        match *op {
            Op::Walk { .. } | Op::WalkNear { .. } => {
                let (who, run, (x, y)) = match *op {
                    Op::Walk { who, run, dx, dy } => {
                        let Some(&p) = h.players.get(who) else {
                            continue;
                        };
                        let (px, py) = h.fx.sim.action.hooks().path_position(p);
                        (who, run, (px + dx, py + dy))
                    }
                    Op::WalkNear {
                        who,
                        run,
                        target,
                        dx,
                        dy,
                    } => {
                        let all: Vec<UnitId> = prev.keys().copied().collect();
                        let t = all[target % all.len()];
                        let (tx, ty) = h.fx.sim.action.hooks().path_position(t);
                        (who, run, (tx + dx, ty + dy))
                    }
                    _ => unreachable!(),
                };
                let Some(&p) = h.players.get(who) else {
                    continue;
                };
                let (px, py) = h.fx.sim.action.hooks().path_position(p);
                if (x - px).abs() > 50 || (y - py).abs() > 50 {
                    continue;
                }
                if x < 0 || y < 0 || x > 0xFFFF || y > 0xFFFF {
                    continue;
                }
                let id = if run { 0x03 } else { 0x01 };
                let r = h.fx.sim.action.with(&mut h.fx.game, |g, v| {
                    walk::walk_message(v, g, p, id, x as u32, y as u32)
                });
                prop_assert_eq!(r.0, 0, "{}: handler result", at);
                // A request computes and starts; it does not move.
                let now = m.observe(&h.fx, &prev);
                for (u, f) in &now {
                    let b = &prev[u];
                    prop_assert_eq!(f.precise, b.precise, "{}: {:?} moved on a request", at, u);
                }
            }
            Op::WalkUnit { who, run, target } => {
                let Some(&p) = h.players.get(who) else {
                    continue;
                };
                let mut all: Vec<UnitId> = prev.keys().copied().collect();
                all.sort();
                let Some(&t) = all.get(target % all.len().max(1)) else {
                    continue;
                };
                let (px, py) = h.fx.sim.action.hooks().path_position(p);
                let (tx, ty) = h.fx.sim.action.hooks().path_position(t);
                if (tx - px).abs() > 50 || (ty - py).abs() > 50 {
                    continue;
                }
                let Some(e) = h.fx.game.lists.unit(t) else {
                    continue;
                };
                let (ty_, guid) = (e.ty as u32, e.guid);
                let id = if run { 0x04 } else { 0x02 };
                let r = h.fx.sim.action.with(&mut h.fx.game, |g, v| {
                    walk::walk_message(v, g, p, id, ty_, guid)
                });
                prop_assert_eq!(r.0, 0, "{}: handler result", at);
            }
            Op::Ticks(n) => {
                for k in 0..n {
                    let at = format!("{at} tick {k}");
                    tick(&mut h.fx);
                    let now = m.observe(&h.fx, &prev);
                    for (u, f) in &now {
                        if prev.get(u).is_some_and(|b| (b.x, b.y) != (f.x, f.y)) {
                            m.unstamped.remove(u);
                            m.outside.remove(u);
                        }
                    }
                    check_tick(&h, &m, &prev, &now, &at)?;
                    m.step(&prev, &now);
                    h.refresh(&mut m);
                    prev = now;
                    check_grid(&h, &m, &prev, &at)?;
                    check_rooms(&h, &m, &prev, &at)?;
                    clean(&h, &at)?;
                    out.push(digest(&h));
                }
                continue;
            }
            Op::Monster { x, y } => {
                let Some(room) = room_at(&h.fx, &active(&h.fx), x, y) else {
                    continue;
                };
                let mut pt = d2_sim::path::coords::Point::new(x, y);
                let Some(room) = place::coarse_free_box(
                    &h.fx.sim.action.sys.hooks.drlg,
                    room,
                    &mut pt,
                    1,
                    MONSTER_PLACE,
                ) else {
                    continue;
                };
                let req = AllocRequest {
                    ty: UnitType::Monster,
                    class: 0,
                    room: Some(room),
                    add: true,
                    fixed_guid: None,
                    mode: 1,
                    allied: false,
                };
                let u =
                    h.fx.sim
                        .action
                        .with(&mut h.fx.game, |g, v| v.allocate(g, &req, pt.x, pt.y));
                let holds =
                    h.fx.sim
                        .action
                        .sys
                        .hooks
                        .drlg
                        .subtile_rect(room)
                        .is_some_and(|t| t.contains(pt.x, pt.y));
                if let (Some(u), false) = (u, holds) {
                    m.outside.insert(u);
                }
            }
            Op::Warp { who } => {
                let Some(&p) = h.players.get(who) else {
                    continue;
                };
                let r = warp(&mut h, p);
                prop_assert!(r.is_some(), "{}: same act", at);
                if r == Some(true) {
                    // The warp activates the spawn room.
                    h.refresh(&mut m);
                    let f = feet(&h.fx)[&p].clone();
                    let room = f.room.unwrap();
                    prop_assert_eq!(
                        h.fx.sim.action.sys.hooks.drlg.level_id(&h.fx.game, room),
                        Some(ISLE),
                        "{}: destination level",
                        at
                    );
                    let u = h.fx.sim.action.sys.units.get(p).unwrap();
                    prop_assert_ne!(u.flags2 & 0x10000, 0, "{}: flags 2", at);
                    let frame = h.fx.game.frame;
                    let has14 = h.fx.game.timers.unit_timers(p).into_iter().any(|t| {
                        h.fx.game.timers.event(t).map(|e| e.0) == Some(14)
                            && h.fx.game.timers.expire(t) == Some(frame + 50)
                    });
                    prop_assert!(has14, "{}: event 14 at frame + 50", at);
                    // Placement searches for a free point with size 2 and mask 0x1C09.
                    for (x, y, _) in f.cells() {
                        let s = h.base.get(x, y);
                        prop_assert!(
                            s.is_some_and(|s| s & PLAYER_MOVE == 0),
                            "{}: warped onto ({}, {}) = {:?}",
                            at,
                            x,
                            y,
                            s
                        );
                    }
                }
            }
        }
        let now = m.observe(&h.fx, &prev);
        m.step(&prev, &now);
        h.refresh(&mut m);
        prev = now;
        check_grid(&h, &m, &prev, &at)?;
        check_rooms(&h, &m, &prev, &at)?;
        clean(&h, &at)?;
        out.push(digest(&h));
    }
    Ok(out)
}

#[test]
fn walks_keep_footprints_rooms_and_steps() {
    runner(10)
        .run(
            &(setup(), prop::collection::vec(op(), 10..40)),
            |(s, ops)| {
                let a = run(&s, &ops)?;
                let b = run(&s, &ops)?;
                prop_assert_eq!(a.len(), b.len());
                for (i, (x, y)) in a.iter().zip(&b).enumerate() {
                    prop_assert_eq!(x, y, "step {} differs between two runs", i);
                }
                Ok(())
            },
        )
        .unwrap();
}

// ---- regressions: the counterexamples of the property, as fixed inputs ---------

fn plain(players: usize, monster_size: u8, walls: Vec<(i32, i32, u16)>) -> Setup {
    Setup {
        players,
        monster_size,
        stamina: 0,
        walls,
    }
}

fn monsters(h: &Host) -> Vec<UnitId> {
    let mut v = h.fx.game.lists.units_of_type(UnitType::Monster);
    v.sort();
    v
}

/// The monsters created from the DS1's preset (the pending log's
/// `preset` lines); room population places the
/// others.
fn preset_monsters(h: &Host) -> Vec<UnitId> {
    monsters(h)
        .into_iter()
        .filter(|u| {
            let tag = format!("preset {} ", u.0);
            h.fx.sim
                .action
                .sys
                .hooks
                .x
                .log
                .iter()
                .any(|l| l.starts_with(&tag))
        })
        .collect()
}

/// Counterexample 1: without a client the player's rooms are
/// deactivated after 10 counts of tick step 9 (the inactivity counter
/// is reset only by clients), with the player in
/// them. The property's host gives every player a client; this pins the
/// room keeping it relies on.
#[test]
fn regress_a_client_keeps_the_players_rooms_active() {
    let mut h = host(&plain(1, 0, Vec::new()));
    let p = h.players[0];
    for _ in 0..300 {
        tick(&mut h.fx);
    }
    let r = h.fx.game.lists.unit(p).and_then(|e| e.room());
    assert!(r.is_some_and(|r| active(&h.fx).contains(&r)));
    assert_eq!(dynamic(&h.fx, p).unwrap().room, r);
    // M08: the same game without the client loses the room.
    let mut h = host(&plain(1, 0, Vec::new()));
    let c = h.fx.game.lists.clients().first().copied().expect("client");
    h.fx.game.lists.remove_client(c).expect("removed");
    for _ in 0..300 {
        tick(&mut h.fx);
    }
    assert_eq!(h.fx.game.lists.unit(p).and_then(|e| e.room()), None);
}

/// Counterexample 2: population's preset monster with monstats2 `SizeX`
/// 1 stands with its plus (pattern 1) over a
/// wall. Population's placement collision check tests the size shape,
/// one cell for SizeX 1; property 1 therefore tests that size shape.
#[test]
fn regress_a_size_one_monster_is_placed_by_its_cell_not_its_plus() {
    let wall = vec![(12, 11, bits::WALL)];
    let mut h = host(&plain(1, 1, wall.clone()));
    tick(&mut h.fx);
    let m = preset_monsters(&h);
    assert_eq!(m.len(), 1);
    let f = feet(&h.fx)[&m[0]].clone();
    assert_eq!((f.x, f.y, f.size, f.pattern), (40012, 40010, 1, 1));
    let g = grid(&h.fx, &active(&h.fx));
    assert_eq!(g.get(40012, 40011), Some(bits::WALL | bits::MONSTER));
    assert_eq!(g.get(40012, 40010), Some(bits::MONSTER | bits::NO_PATH));
}

/// Counterexample 2, second half (the bug, fixed in
/// `wiring/worldgen/population.rs`): with `SizeX` 2 population's
/// placement test is the plus, so the DS1 point, whose plus
/// meets the wall, is refused and the second search (r = 4) places
/// the monster elsewhere on a
/// free plus. The adapter read one sub-tile for every size and placed
/// it on the DS1 point.
#[test]
fn regress_population_tests_the_size_shape() {
    let mut h = host(&plain(1, 2, vec![(12, 11, bits::WALL)]));
    tick(&mut h.fx);
    let m = preset_monsters(&h);
    assert_eq!(m.len(), 1);
    let f = feet(&h.fx)[&m[0]].clone();
    assert_ne!((f.x, f.y), (40012, 40010));
    let g = grid(&h.fx, &active(&h.fx));
    for (x, y) in f.size_cells() {
        assert_eq!(g.get(x, y).map(|v| v & bits::WALL), Some(0), "({x}, {y})");
    }
}

/// Counterexample 3: tick step 9 deactivates a room holding a monster
/// (no client near it). This fixture's inactive-storage compression hook
/// does nothing, so the monster leaves the room list but keeps its path
/// record and footprint in the old room. This pins the fixture's limitation;
/// the property skips such monsters without asserting inactive-storage behavior.
#[test]
fn regress_a_deactivated_room_leaves_its_monster_path_record() {
    let mut h = host(&plain(1, 0, Vec::new()));
    let m = monster(&mut h, 40180, 40085);
    let r = dynamic(&h.fx, m).unwrap().room.unwrap();
    for _ in 0..300 {
        tick(&mut h.fx);
    }
    assert!(!active(&h.fx).contains(&r));
    assert_eq!(h.fx.game.lists.unit(m).and_then(|e| e.room()), None);
    assert_eq!(dynamic(&h.fx, m).unwrap().room, Some(r));
}

/// Counterexample 4: the
/// teleport stamps the footprint from the destination room (the forced
/// move's room2), so the first warped player is stamped at the spawn
/// point and the second warp's free search places the second player
/// elsewhere.
#[test]
fn regress_second_warp_sees_the_first_player() {
    let mut h = host(&plain(2, 0, Vec::new()));
    let (p0, p1) = (h.players[0], h.players[1]);
    assert_eq!(warp(&mut h, p0), Some(true));
    let a = feet(&h.fx)[&p0].clone();
    let g = grid(&h.fx, &active(&h.fx));
    assert_eq!(
        g.get(a.x, a.y),
        Some(bits::PLAYER | bits::NO_PATH),
        "footprint at the destination"
    );
    assert_eq!(g.get(SPAWNS[0].0, SPAWNS[0].1), Some(0), "old one cleared");
    assert_eq!(warp(&mut h, p1), Some(true));
    let b = feet(&h.fx)[&p1].clone();
    assert_ne!((b.x, b.y), (a.x, a.y));
    let g = grid(&h.fx, &active(&h.fx));
    assert_eq!(g.get(b.x, b.y), Some(bits::PLAYER | bits::NO_PATH));
    assert_eq!(g.get(a.x, a.y), Some(bits::PLAYER | bits::NO_PATH));
}

/// Counterexample 5 (a test-side mistake, kept as the fixed input): after
/// 132 ticks the rooms away from the player's client are streamed out;
/// the warp makes the spawn room active again
/// and places the player there on a free point.
#[test]
fn regress_a_warp_streams_its_spawn_room_back_in() {
    let mut h = host(&plain(1, 0, Vec::new()));
    let p = h.players[0];
    for _ in 0..132 {
        tick(&mut h.fx);
    }
    let before = active(&h.fx);
    assert_eq!(warp(&mut h, p), Some(true));
    let f = feet(&h.fx)[&p].clone();
    let r = f.room.unwrap();
    assert!(!before.contains(&r), "the spawn room was streamed out");
    assert!(active(&h.fx).contains(&r));
    assert_eq!(h.fx.game.lists.unit(p).and_then(|e| e.room()), Some(r));
    // Free apart from its own footprint (stamped from the destination
    // room).
    let g = grid(&h.fx, &active(&h.fx));
    for (x, y, b) in f.cells() {
        assert!(g.get(x, y).is_some_and(|v| v & !b & PLAYER_MOVE == 0));
        assert_eq!(g.get(x, y).map(|v| v & b), Some(b));
    }
}

/// Counterexample 6 (release hunt, 600 cases): the coarse free-box
/// search from room A = (40000, 40080, 40 × 10) around (40008,
/// 40080), with a wall at (40008, 40078), returns room A with the point
/// (40006, 40078), which lies in the room above. Pass 1's row y = 40079
/// is outside A's rows, so its row room is the room above (B) and the
/// cell visit reads B's rect; pass 2's row y = 40078 is inside the rect
/// last read (B's), so rule 2 takes `room` (A) as the row room, and the
/// cell (40006, 40078) is inside A's columns: out room A. The code
/// reuses the last rectangle read. Whether that room/result mismatch is
/// intended remains unresolved. A monster placed there stands outside its
/// room until its first move.
#[test]
fn regress_coarse_box_room_need_not_hold_the_point() {
    let h = host(&plain(1, 0, vec![(8, 78, bits::WALL)]));
    let a = room_at(&h.fx, &active(&h.fx), 40008, 40080).unwrap();
    let above = room_at(&h.fx, &active(&h.fx), 40008, 40079).unwrap();
    assert_ne!(a, above);
    let mut pt = d2_sim::path::coords::Point::new(40008, 40080);
    let r = place::coarse_free_box(
        &h.fx.sim.action.sys.hooks.drlg,
        a,
        &mut pt,
        1,
        MONSTER_PLACE,
    );
    assert_eq!((r, pt.x, pt.y), (Some(a), 40006, 40078));
    // M08: without the wall pass 1 succeeds in the room above.
    let h2 = host(&plain(1, 0, Vec::new()));
    let mut pt = d2_sim::path::coords::Point::new(40008, 40080);
    let r = place::coarse_free_box(
        &h2.fx.sim.action.sys.hooks.drlg,
        a,
        &mut pt,
        1,
        MONSTER_PLACE,
    );
    assert_eq!((r, pt.x, pt.y), (Some(above), 40007, 40079));
}

/// Player placement queues pet following for the host holding the pet lists
/// (`ActionHooks::pet_follows`); the test checks the queued player entry.
#[test]
fn a_warped_player_queues_its_pet_follow() {
    let mut h = host(&plain(2, 0, Vec::new()));
    let (p0, p1) = (h.players[0], h.players[1]);
    assert_eq!(warp(&mut h, p0), Some(true));
    assert_eq!(h.fx.sim.action.sys.hooks.pet_follows, None);
    h.fx.sim.action.sys.hooks.pet_follows = Some(Vec::new());
    assert_eq!(warp(&mut h, p1), Some(true));
    assert_eq!(h.fx.sim.action.sys.hooks.pet_follows, Some(vec![p1]));
}
