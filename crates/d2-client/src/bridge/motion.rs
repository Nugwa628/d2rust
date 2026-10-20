//! Provisional monster motion between messages (first playable preview).
//!
//! A monster's S→C 0x67 (`MonsterMove`, to a point) or 0x68
//! (`MonsterMoveToTarget`, to a unit) puts it in walk (2) or run (15)
//! through the mode request ([`super::modes`]),
//! but the model moves a unit only when a message places it (the
//! monster mode machine with its client path is an open question).
//! Until that is specified, this module stands in:
//!
//! - a monster in walk / run whose last request is a move code of the
//!   server's mode table (walk 1 / 0, run 23 / 24 to a point / a unit)
//!   gets a track from its cell toward the
//!   request's point, or the target unit's cell (the local player's
//!   cell is its own position, the walk prediction;
//!   its model cell is the last
//!   placement, possibly hundreds of sub-tiles behind);
//! - from the tick after the request (the server path's first step) the
//!   track steps each server tick by the velocity vector of
//!   ((m · direction vector) >> 12 per axis,
//!   the direction vector toward the goal, m = path velocity ·
//!   0x400 >> 6), starting at the cell centre; toward a point it stops
//!   where the next step would pass the goal's centre (no snap); it
//!   writes the reached cell and the precise position to the model, so
//!   hover, clicks and the draw follow (measured against the base-game state
//!   and draws of the Rogue Encampment NPCs). The
//!   message's velocity field is stat 67 `velocitypercent`, not a path
//!   velocity: the path velocity
//!   is the server's own computation
//!   (`path::walk::velocity::mode_velocity`) on the class's `monstats`
//!   `Velocity` and that percent; a class without the column does not
//!   move;
//! - any other request (stop 0x6D, attack, hit, death) or a placement by
//!   a message ends the track.
//!
//! A straight line instead of the client path, no collision; the server
//! stays the authority (its next placement or stop wins). This matches
//! three Fallen walking to a Necromancer around a Bone Wall (141 moving
//! frames, 160/160): every turn of a base-game client walk comes with a
//! server 0x67 / 0x68 or a stop, none between them; the open-ground
//! case matches too. (Monsters in town do not walk to the player.)

use std::collections::BTreeMap;

use d2_sim::monsters::mode_message::{mode, MODE_ROWS};
use d2_sim::path::walk::velocity::{mode_velocity, STAT_VELOCITYPERCENT};
use d2_sim::path::walk::WalkUnits;
use d2_sim::rng::Seed;
use d2_sim::tick::TickRate;
use d2_sim::units::{UnitId, UnitType};

use super::world::{ClientWorld, ModeRequest, MonsterClass, UnitKey, MONSTER};

/// One monster as the server's velocity computation reads it: class,
/// mode, the stat-67 total the message carried and the class's
/// `monstats` `Velocity` / `npc`.
struct Mover {
    class: u32,
    mode: u32,
    percent: i32,
    velocity: i32,
    npc: bool,
    seed: Seed,
}

impl WalkUnits for Mover {
    /// Only the velocity is read here, a base-rate amount; each tick's
    /// share is taken where the track steps ([`tick_share`]).
    fn tick_rate(&self) -> TickRate {
        TickRate::BASE
    }
    fn unit_type(&self, _: UnitId) -> UnitType {
        UnitType::Monster
    }
    fn class(&self, _: UnitId) -> u32 {
        self.class
    }
    fn frame(&self) -> i32 {
        0
    }
    fn mode(&self, _: UnitId) -> u32 {
        self.mode
    }
    fn stat(&self, _: UnitId, stat: u16) -> i32 {
        if stat == STAT_VELOCITYPERCENT {
            self.percent
        } else {
            0
        }
    }
    fn seed(&mut self, _: UnitId) -> &mut Seed {
        &mut self.seed
    }
    fn monstats_velocity(&self, _: UnitId) -> (i32, bool) {
        (self.velocity, self.npc)
    }
}

/// Tick `tick`'s share of `per_base_tick`, a 16.16 distance the tables
/// state per base-rate tick ([`TickRate::share`]): the whole distance at
/// the base rate, and at a faster rate the same distance a second, spread
/// over its ticks. The client steps once per server tick, so `tick` is
/// the authoritative game-frame phase ([`ClientWorld::share_frame`]).
pub(super) fn tick_share(rate: TickRate, per_base_tick: i64, tick: u64) -> i64 {
    if rate.is_base() {
        return per_base_tick;
    }
    let v = per_base_tick.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    // Only the tick's place in its second matters to the share.
    let i = (tick % u64::from(rate.hz())) as i32;
    i64::from(rate.share(v, i))
}

/// The 16.16 distance of one base-rate tick of a monster of `class` in
/// `mode` at velocity percent `percent` (module doc): the server's path
/// velocity · 0x400 >> 6. 0 when
/// the class has no `monstats` row or the mode no velocity.
pub fn monster_step(monsters: &[Option<MonsterClass>], class: u32, mode: u32, percent: i32) -> i64 {
    let Some((row, t)) = monsters
        .get(class as usize)
        .copied()
        .flatten()
        .zip(super::predict::path_tables())
    else {
        return 0;
    };
    let Some(setup) = row.setup else {
        return 0;
    };
    let u = Mover {
        class,
        mode,
        percent,
        velocity: i32::from(setup.velocity),
        npc: row.npc,
        seed: Seed::default(),
    };
    let v = mode_velocity(t, &u, UnitId(0), mode).unwrap_or(0);
    (i64::from(v) * 0x400) >> 6
}

/// The monster mode request code of a stop (neutral's row of the mode table).
const STOP_CODE: u8 = MODE_ROWS[mode::NU as usize].code_to_point;

/// Where a monster walks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Goal {
    Point(u16, u16),
    Unit(UnitKey),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Track {
    /// The request the track was made from.
    request: ModeRequest,
    goal: Goal,
    /// Precise position, 16.16 sub-tiles.
    at: (i64, i64),
    /// 16.16 distance per base-rate tick.
    step: i64,
    /// The cell last written to the model.
    cell: (u16, u16),
    /// Made in this frame's refresh: the server path moves first on the
    /// tick after the request (event 0 of the
    /// next tick), so a new track does not step yet.
    fresh: bool,
    /// The unit's request count when the track was made (a new request,
    /// even one equal to the last, starts a fresh track).
    requests: u32,
}

/// The tracks of the walking monsters (module doc).
#[derive(Clone, Debug, Default)]
pub struct MonsterMotion {
    tracks: BTreeMap<UnitKey, Track>,
    seen_ticks: u64,
    /// The monsters whose stop walk reached its point in the last step.
    arrived: Vec<UnitKey>,
}

/// The goal of a monster's move request: the to-point / to-unit codes of
/// the walk and run rows of the server's mode table.
fn goal_of(r: &ModeRequest) -> Option<Goal> {
    let walk = MODE_ROWS[mode::WL as usize];
    let run = MODE_ROWS[mode::RN as usize];
    // A stop away from the client's cell walks to the stop point (the
    // monster mode request of code 7, `modes::monster`), then ends in
    // neutral at arrival ([`MonsterMotion::frame`]).
    if r.code == STOP_CODE {
        return Some(Goal::Point(r.record[0] as u16, r.record[1] as u16));
    }
    if r.code == walk.code_to_point || r.code == run.code_to_point {
        return Some(Goal::Point(r.record[0] as u16, r.record[1] as u16));
    }
    if r.code == walk.code_to_unit || r.code == run.code_to_unit {
        return Some(Goal::Unit(UnitKey::new(
            r.record[0] as u8,
            r.record[1] as u32,
        )));
    }
    None
}

fn centre(c: (u16, u16)) -> (i64, i64) {
    (
        (i64::from(c.0) << 16) | 0x8000,
        (i64::from(c.1) << 16) | 0x8000,
    )
}

/// The cell of a unit goal: the local player's own position
/// (the client path that
/// aims at is the player's walk prediction, not its last
/// placement), else the model cell.
fn target_cell(world: &ClientWorld, key: UnitKey) -> Option<(u16, u16)> {
    if world.local_player == Some(key) {
        return world.local_cell();
    }
    world.units.get(&key).and_then(|u| u.position)
}

impl MonsterMotion {
    /// The number of monsters with a track.
    pub fn walking(&self) -> usize {
        self.tracks.len()
    }

    /// One bridge frame: refresh the tracks from the model (speeds from
    /// the `monstats` rows `monsters`), then step them once if the server
    /// ticked since the last frame. Returns the monsters that walked to the
    /// point of a stop request and are there: the caller ends their walk in
    /// neutral.
    pub fn frame(
        &mut self,
        world: &mut ClientWorld,
        monsters: &[Option<MonsterClass>],
    ) -> Vec<UnitKey> {
        self.arrived.clear();
        let ticked = world.server_ticks != self.seen_ticks;
        self.seen_ticks = world.server_ticks;
        self.refresh(world, monsters);
        if ticked {
            self.step(world);
        }
        for t in self.tracks.values_mut() {
            t.fresh = false;
        }
        std::mem::take(&mut self.arrived)
    }

    fn refresh(&mut self, world: &ClientWorld, monsters: &[Option<MonsterClass>]) {
        let mut next = BTreeMap::new();
        for (key, u) in &world.units {
            if key.unit_type != MONSTER || !matches!(u.mode, mode::WL | mode::RN) {
                continue;
            }
            let (Some(r), Some(cell)) = (u.last_mode_request, u.position) else {
                continue;
            };
            let Some(goal) = goal_of(&r) else {
                continue;
            };
            let step = monster_step(monsters, u.class, u.mode, r.record[4]);
            let old = self.tracks.get(key);
            let keep = old
                .filter(|t| t.request == r && t.cell == cell && t.requests == u.mode_requests)
                .copied();
            // Only a new request waits for the next tick; a track remade
            // from a placement steps at once.
            let fresh = old.is_none_or(|t| t.requests != u.mode_requests);
            next.insert(
                *key,
                keep.unwrap_or(Track {
                    request: r,
                    goal,
                    // A walk starts at the cell centre (Warriv's
                    // x fraction 32016 at the end of a walk, 32768 on the
                    // tick of the next request).
                    at: centre(cell),
                    step,
                    cell,
                    fresh,
                    requests: u.mode_requests,
                }),
            );
        }
        self.tracks = next;
    }

    fn step(&mut self, world: &mut ClientWorld) {
        let (rate, tick) = (
            world.tick_rate,
            world.share_frame().rem_euclid(world.tick_rate.hz() as i32) as u64,
        );
        let mut arrived = Vec::new();
        for (key, t) in self.tracks.iter_mut() {
            if t.fresh {
                continue;
            }
            let goal = match t.goal {
                Goal::Point(x, y) => Some((x, y)),
                Goal::Unit(u) => target_cell(world, u),
            };
            let Some(goal) = goal else { continue };
            let (gx, gy) = centre(goal);
            let (dx, dy) = (gx - t.at.0, gy - t.at.1);
            let len = dx.abs().max(dy.abs());
            if (len == 0 || t.step == 0) && t.request.code == STOP_CODE {
                arrived.push(*key);
            }
            if len == 0 || t.step == 0 {
                continue;
            }
            // This tick's share of the base-rate step.
            let step = tick_share(rate, t.step, tick);
            // A unit goal: stop one sub-tile short (the target's cell is
            // taken).
            let room = if matches!(t.goal, Goal::Unit(_)) {
                (len - 0x10000).max(0)
            } else {
                len
            };
            // The velocity vector is (m ·
            // direction vector) >> 12 per axis, the direction vector of
            // toward the goal (a straight line: no client path).
            let Some(tables) = super::predict::path_tables() else {
                continue;
            };
            let ((vx, vy), _) = d2_sim::path::walk::geom::direction_vector(
                tables,
                (t.at.0 as u32, t.at.1 as u32),
                (gx as u32, gy as u32),
            );
            let (mx, my) = ((step * i64::from(vx)) >> 12, (step * i64::from(vy)) >> 12);
            let reach = mx.abs().max(my.abs());
            // A stop walk is done when the next step reaches the point.
            if t.request.code == STOP_CODE && matches!(t.goal, Goal::Point(..)) && len <= reach {
                arrived.push(*key);
            }
            t.at = match t.goal {
                // A point: full steps while the next one does not pass
                // the goal's centre; then the path stops where it is (no
                // snap: Warriv at rest 376 short of the centre
                // on both axes, (32392, 33144)).
                Goal::Point(..) if dx.abs() < mx.abs() && dy.abs() < my.abs() => t.at,
                Goal::Point(..) if dx == 0 && dy == 0 => t.at,
                Goal::Point(..) => (t.at.0 + mx, t.at.1 + my),
                // A unit: one sub-tile short of it.
                Goal::Unit(_) if room == 0 => t.at,
                Goal::Unit(_) if room <= reach => {
                    (t.at.0 + dx * room / len, t.at.1 + dy * room / len)
                }
                Goal::Unit(_) => (t.at.0 + mx, t.at.1 + my),
            };
            let cell = ((t.at.0 >> 16) as u16, (t.at.1 >> 16) as u16);
            if let Some(u) = world.units.get_mut(key) {
                u.precise = Some((t.at.0 as u32, t.at.1 as u32));
                if cell != t.cell {
                    u.position = Some(cell);
                }
            }
            if cell != t.cell {
                recache_room(world, *key, cell);
            }
            t.cell = cell;
        }
        self.arrived.extend(arrived);
    }
}

/// The path step's room recache:
/// a cell outside the
/// unit's room moves it to the head of the room holding the cell (none:
/// it leaves its room). Without the client DRLG nothing changes.
fn recache_room(world: &mut ClientWorld, key: UnitKey, cell: (u16, u16)) {
    if world.active_rooms.is_none() {
        return;
    }
    let new = world
        .room_from(world.unit_room(key), cell.0, cell.1)
        .map(|r| r.room);
    if new.is_some() && new == world.room_units.room_of(key) {
        return;
    }
    world.room_units.place(key, new);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::world::{ClientUnit, MonsterSetup, PLAYER};

    /// `monstats` rows: class 0 with `Velocity` 16 (path velocity 0x1000
    /// at 100 %: one sub-tile a tick).
    fn classes() -> Vec<Option<MonsterClass>> {
        vec![Some(MonsterClass {
            setup: Some(MonsterSetup {
                velocity: 16,
                ..Default::default()
            }),
            ..Default::default()
        })]
    }

    fn monster(w: &mut ClientWorld, guid: u32, at: (u16, u16), mode: u32, r: ModeRequest) {
        let k = UnitKey::new(MONSTER, guid);
        let mut u = ClientUnit::new(k);
        u.position = Some(at);
        u.mode = mode;
        u.last_mode_request = Some(r);
        w.units.insert(k, u);
    }

    /// A move request; `percent` is the message's stat 67.
    fn request(code: u8, a: i32, b: i32, percent: i32) -> ModeRequest {
        ModeRequest {
            code,
            record: [a, b, 0, 0, percent, 0, 0],
        }
    }

    fn tick(m: &mut MonsterMotion, w: &mut ClientWorld) {
        w.server_ticks += 1;
        m.frame(w, &classes());
    }

    #[test]
    fn a_stop_walk_reports_its_arrival_once_it_is_at_the_point() {
        let mut w = ClientWorld::default();
        // A stop 5 sub-tiles away walks there, then reports the arrival.
        monster(&mut w, 5, (100, 100), mode::WL, request(7, 105, 100, 100));
        let mut m = MonsterMotion::default();
        assert!(m.frame(&mut w, &classes()).is_empty());
        let mut arrived = Vec::new();
        for _ in 0..12 {
            w.server_ticks += 1;
            arrived.extend(m.frame(&mut w, &classes()));
            if !arrived.is_empty() {
                break;
            }
        }
        assert_eq!(arrived, vec![UnitKey::new(MONSTER, 5)]);
        let (x, _) = w.units[&UnitKey::new(MONSTER, 5)].position.unwrap();
        assert!((104..=105).contains(&x), "x {x}");
    }

    #[test]
    fn a_walk_to_a_point_does_not_report_an_arrival() {
        let mut w = ClientWorld::default();
        monster(&mut w, 5, (100, 100), mode::WL, request(1, 105, 100, 100));
        let mut m = MonsterMotion::default();
        for _ in 0..12 {
            w.server_ticks += 1;
            assert!(m.frame(&mut w, &classes()).is_empty());
        }
    }

    #[test]
    fn a_walking_monster_steps_to_its_point_and_stops() {
        let mut w = ClientWorld::default();
        // Walk to (110, 100) at 100 %: one sub-tile a tick.
        monster(&mut w, 5, (100, 100), mode::WL, request(1, 110, 100, 100));
        let mut m = MonsterMotion::default();
        m.frame(&mut w, &classes());
        assert_eq!(m.walking(), 1);
        for i in 1..=3 {
            tick(&mut m, &mut w);
            assert_eq!(
                w.units[&UnitKey::new(MONSTER, 5)].position,
                Some((100 + i, 100))
            );
        }
        for _ in 0..20 {
            tick(&mut m, &mut w);
        }
        assert_eq!(
            w.units[&UnitKey::new(MONSTER, 5)].position,
            Some((110, 100))
        );
    }

    #[test]
    fn a_monster_walks_to_a_unit_and_stops_next_to_it() {
        let mut w = ClientWorld::default();
        let p = UnitKey::new(PLAYER, 1);
        let mut pu = ClientUnit::new(p);
        pu.position = Some((100, 108));
        w.units.insert(p, pu);
        // Run (15) to the player, code 24, at 200 %.
        monster(&mut w, 5, (100, 100), mode::RN, request(24, 0, 1, 200));
        let mut m = MonsterMotion::default();
        m.frame(&mut w, &classes());
        for _ in 0..20 {
            tick(&mut m, &mut w);
        }
        assert_eq!(
            w.units[&UnitKey::new(MONSTER, 5)].position,
            Some((100, 107))
        );
    }

    /// The autoplay crash (Cold Plains, seed 1234): monsters walking to
    /// the local player (code 0) aimed at its model cell, the town
    /// placement about 300 sub-tiles back, and the direction vector's
    /// `127 × l` wrapped into a negative `tan` index. They aim at the player's
    /// own position, the walk prediction.
    #[test]
    fn a_monster_walks_to_the_local_players_predicted_position() {
        let mut w = ClientWorld::default();
        let p = UnitKey::new(PLAYER, 1);
        let mut pu = ClientUnit::new(p);
        pu.position = Some((4873, 4228));
        w.units.insert(p, pu);
        w.local_player = Some(p);
        let near = (5164u32 << 16 | 0x8000, 4564u32 << 16 | 0x8000);
        w.set_local_walk(Some(near), Some(2));
        monster(&mut w, 45, (5150, 4550), mode::WL, request(0, 0, 1, 75));
        let mut m = MonsterMotion::default();
        m.frame(&mut w, &classes());
        for _ in 0..4 {
            tick(&mut m, &mut w);
        }
        let k = UnitKey::new(MONSTER, 45);
        let (x, y) = w.units[&k].precise.expect("the monster moved");
        assert!(x > (5150 << 16 | 0x8000) && y > (4550 << 16 | 0x8000));
    }

    #[test]
    fn other_modes_and_placements_end_the_track() {
        let mut w = ClientWorld::default();
        let k = UnitKey::new(MONSTER, 5);
        monster(&mut w, 5, (100, 100), mode::WL, request(1, 120, 100, 100));
        let mut m = MonsterMotion::default();
        m.frame(&mut w, &classes());
        tick(&mut m, &mut w);
        // A server placement (position check) moves it elsewhere: snap.
        w.units.get_mut(&k).unwrap().position = Some((50, 100));
        tick(&mut m, &mut w);
        assert_eq!(
            w.units[&k].position,
            Some((51, 100)),
            "steps from the placement"
        );
        // Neutral: no track, no motion.
        w.units.get_mut(&k).unwrap().mode = mode::NU;
        tick(&mut m, &mut w);
        assert_eq!(m.walking(), 0);
        assert_eq!(w.units[&k].position, Some((51, 100)));
    }

    #[test]
    fn a_diagonal_npc_walk_follows_the_recorded_server_positions() {
        // Warriv's second walk at the Rogue Encampment arrival: from the
        // centre of (4868, 4233) toward (4869, 4232)
        // at monstats Velocity 3 (npc) and 75 %: 6516 per axis a tick from
        // the tick after the request, resting at (4869 + 32392, 4232 +
        // 33144) in 16.16, 376 short of the centre.
        let rows = vec![Some(MonsterClass {
            npc: true,
            setup: Some(MonsterSetup {
                velocity: 3,
                ..Default::default()
            }),
            ..Default::default()
        })];
        let mut w = ClientWorld::default();
        let k = UnitKey::new(MONSTER, 7);
        monster(
            &mut w,
            7,
            (4868, 4233),
            mode::WL,
            request(1, 4869, 4232, 75),
        );
        w.units.get_mut(&k).unwrap().mode_requests = 1;
        let mut m = MonsterMotion::default();
        w.server_ticks += 1;
        m.frame(&mut w, &rows); // the request's tick: no step yet
        let p = |w: &ClientWorld| w.units[&k].precise.unwrap();
        let x0 = (4868u32 << 16) | 0x8000;
        let y0 = (4233u32 << 16) | 0x8000;
        assert_eq!(w.units[&k].precise, None);
        for n in 1..=10u32 {
            w.server_ticks += 1;
            m.frame(&mut w, &rows);
            assert_eq!(p(&w), (x0 + 6516 * n, y0 - 6516 * n), "step {n}");
        }
        assert_eq!(p(&w), ((4869 << 16) + 32392, (4232 << 16) + 33144));
        w.server_ticks += 1;
        m.frame(&mut w, &rows);
        assert_eq!(p(&w), ((4869 << 16) + 32392, (4232 << 16) + 33144));
        assert_eq!(w.units[&k].position, Some((4869, 4232)));
    }

    #[test]
    fn a_step_into_another_room_moves_the_unit_to_its_head() {
        use crate::bridge::drlg::DrlgRoomId;
        use crate::bridge::world::ActiveRoom;
        let room = |id, x0| ActiveRoom {
            x0,
            y0: 0,
            w: 40,
            h: 40,
            level: 1,
            room: DrlgRoomId(id),
        };
        let mut w = ClientWorld {
            active_rooms: Some(vec![room(16, 0), room(17, 40)]),
            ..ClientWorld::default()
        };
        let (k, other) = (UnitKey::new(MONSTER, 4), UnitKey::new(2, 5));
        w.units.insert(k, ClientUnit::new(k));
        w.room_units.place(k, Some(DrlgRoomId(16)));
        w.room_units.place(other, Some(DrlgRoomId(17)));
        // Inside its room: nothing.
        recache_room(&mut w, k, (39, 10));
        assert_eq!(w.room_units.room_of(k), Some(DrlgRoomId(16)));
        // Across the edge: leaves room 16, at the head of room 17.
        recache_room(&mut w, k, (40, 10));
        assert_eq!(w.room_units.list(DrlgRoomId(17)), [k, other]);
        assert!(w.room_units.list(DrlgRoomId(16)).is_empty());
        // Outside every room: no room.
        recache_room(&mut w, k, (200, 10));
        assert_eq!(w.room_units.room_of(k), None);
    }

    #[test]
    fn a_faster_rate_spreads_one_base_tick_step_over_its_ticks() {
        let fast = TickRate::new(60).unwrap();
        let step = 0x1_0000;
        assert_eq!(tick_share(TickRate::BASE, step, 7), step);
        // One second of ticks at either rate covers 25 base steps.
        let second = |rate: TickRate, from: u64| {
            (from..from + u64::from(rate.hz()))
                .map(|t| tick_share(rate, step, t))
                .sum::<i64>()
        };
        assert_eq!(second(TickRate::BASE, 0), 25 * step);
        assert_eq!(second(fast, 0), 25 * step);
        // The share depends on the tick's place in its second only.
        assert_eq!(second(fast, u64::from(u32::MAX) * 60), 25 * step);
        assert_eq!(tick_share(fast, step, 61), tick_share(fast, step, 1));
    }

    #[test]
    fn monster_motion_keeps_its_distance_per_second() {
        let distance = |rate: TickRate| {
            let mut w = ClientWorld {
                tick_rate: rate,
                ..ClientWorld::default()
            };
            let key = UnitKey::new(MONSTER, 5);
            monster(&mut w, 5, (100, 100), mode::WL, request(1, 200, 100, 100));
            let mut motion = MonsterMotion::default();
            motion.frame(&mut w, &classes());
            for _ in 0..rate.hz() {
                tick(&mut motion, &mut w);
            }
            let end = w.units[&key].precise.unwrap();
            assert_eq!(end.1, 100 << 16 | 0x8000);
            end.0 - (100 << 16 | 0x8000)
        };
        assert_eq!(distance(TickRate::BASE), 25 << 16);
        assert_eq!(distance(TickRate::new(60).unwrap()), 25 << 16);
    }
}
