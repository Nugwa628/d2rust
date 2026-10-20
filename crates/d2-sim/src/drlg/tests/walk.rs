//! Regression for the live panic "live DRLG room" (gap G1,
//! in play): a client walks across the rooms of two
//! neighbouring levels and away to a third for 3000 ticks while the
//! server's tick steps 9 (room deactivation, every 12 frames) and 10
//! (free inactive levels, every 11 frames) run, so both levels are freed
//! and regenerated several times. The level types keep a per-room record
//! the way the outdoor type does (reset after the rooms are freed), through
//! the real [`Outdoor`] code. After every tick no live structure names a
//! freed room.

use super::fakes::*;
use crate::drlg::outdoor::{Outdoor, OutdoorRoom};
use crate::drlg::*;
use crate::units::{ClientId, UnitLists};

const INIT: u32 = 644_409_375;
const TICKS: u32 = 3000;
/// Tick periods of steps 9 and 10 (`tick::period`).
const DEACTIVATION_PERIOD: u32 = 12;
const FREE_PERIOD: u32 = 11;
/// Ticks the client stays in each room of the route.
const STAY: u32 = 30;

/// [`FakeTypes`] plus outdoor room records for every generated room.
struct RecordTypes {
    fake: FakeTypes,
    outdoor: Outdoor,
}

impl LevelTypes for RecordTypes {
    fn create_act_levels(&mut self, drlg: &mut Drlg, data: &DrlgData) -> Result<(), DrlgError> {
        self.fake.create_act_levels(drlg, data)
    }

    fn init_level(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        l: LevelIdx,
    ) -> Result<(), DrlgError> {
        self.fake.init_level(drlg, data, l)
    }

    fn generate(&mut self, drlg: &mut Drlg, data: &DrlgData, l: LevelIdx) -> Result<(), DrlgError> {
        self.fake.generate(drlg, data, l)?;
        for r in drlg.level_rooms(l) {
            self.outdoor.rooms.insert(r, OutdoorRoom::default());
        }
        Ok(())
    }

    fn reset_level(&mut self, drlg: &mut Drlg, l: LevelIdx) {
        self.fake.reset_level(drlg, l);
        self.outdoor.reset_level(drlg, l);
    }

    fn add_preset_units(&mut self, drlg: &mut Drlg, room: DrlgRoomId) -> Result<(), DrlgError> {
        self.fake.add_preset_units(drlg, room)
    }

    fn preset_units(&self, drlg: &Drlg, room: DrlgRoomId) -> Vec<PresetUnit> {
        self.fake.preset_units(drlg, room)
    }

    fn room_grids(
        &mut self,
        drlg: &mut Drlg,
        data: &DrlgData,
        room: DrlgRoomId,
    ) -> Result<RoomGrids, DrlgError> {
        self.fake.room_grids(drlg, data, room)
    }

    fn free_room_tiles(&mut self, drlg: &mut Drlg, room: DrlgRoomId) {
        self.fake.free_room_tiles(drlg, room);
        self.outdoor.free_room_tiles(room);
    }
}

/// A 3×3 block of 8×8 rooms at tile (x0, 0).
fn block(x0: i32) -> Vec<RoomSpec> {
    (0..9)
        .map(|k| preset(x0 + 8 * (k % 3), 8 * (k / 3), 8, 8))
        .collect()
}

/// Every reference a live structure holds names a live room.
fn assert_no_freed_room(d: &Drlg, outdoor: &Outdoor, tick: u32) {
    let live = |r: DrlgRoomId| d.try_room(r).is_some();
    for l in d.level_list() {
        let mut cur = d.level(l).first_room;
        while let Some(r) = cur {
            assert!(live(r), "tick {tick}: level list names freed {r:?}");
            cur = d.try_room(r).and_then(|x| x.next);
        }
        for r in d.level_rooms(l) {
            let room = d.room(r);
            for &n in room.near().unwrap_or(&[]) {
                assert!(live(n), "tick {tick}: near of {r:?} names freed {n:?}");
            }
            for w in &room.warp_links {
                assert!(live(w.target), "tick {tick}: warp link of {r:?}");
            }
            if let Some(a) = room.active() {
                for &n in &a.adjacency {
                    assert!(live(n), "tick {tick}: adjacency of {r:?} names {n:?}");
                }
            }
        }
    }
    for s in 0..4 {
        for &r in d.status_list(s) {
            assert!(live(r), "tick {tick}: status list {s} names freed {r:?}");
        }
    }
    for (_, r) in d.active_rooms() {
        assert!(live(r), "tick {tick}: active index names freed {r:?}");
    }
    for &r in outdoor.rooms.keys() {
        assert!(live(r), "tick {tick}: outdoor record of freed {r:?}");
    }
}

#[test]
fn walking_across_levels_for_3000_ticks_never_reads_a_freed_room() {
    let mut data = data();
    for (id, x0) in [(2u32, 0), (3, 24), (4, 200)] {
        gen_level(&mut data, id, 2);
        data.levels[id as usize].offset = (x0, 0);
        data.levels[id as usize].size = [(24, 24); 3];
    }
    data.levels[2].vis = [3, 0, 0, 0, 0, 0, 0, 0];
    data.levels[3].vis = [2, 0, 0, 0, 0, 0, 0, 0];
    let mut fake = FakeTypes::default();
    fake.rooms.insert(2, block(0));
    fake.rooms.insert(3, block(24));
    fake.rooms.insert(4, block(200));
    fake.default_grid = Some(floor_grid);
    let mut types = RecordTypes {
        fake,
        outdoor: Outdoor::default(),
    };
    let tiles = tiles();
    let mut lists = UnitLists::new();
    let mut d = Drlg::create(0, INIT, 0, 0, false, &data, &mut types).unwrap();

    // The route: every room of level 2, then of level 3 (across the
    // level border), then a long stay in the far level 4 (2 and 3 lose
    // their activity and are freed), and back.
    let mut route: Vec<(u32, i32, i32)> = Vec::new();
    for (id, x0) in [(2u32, 0), (3, 24)] {
        for k in 0..9 {
            route.push((id, x0 + 8 * (k % 3) + 4, 8 * (k / 3) + 4));
        }
    }
    route.extend([(4, 204, 4); 12]);

    let client = ClientId(0);
    let mut cur: Option<DrlgRoomId> = None;
    for tick in 0..TICKS {
        let mut svc = Services {
            data: &data,
            tiles: &tiles,
            types: &mut types,
            rooms: &mut lists,
        };
        if tick % STAY == 0 {
            let (id, x, y) = route[(tick / STAY) as usize % route.len()];
            let l = d.get_or_alloc_level(svc.data, svc.types, id).unwrap();
            let hint = cur.filter(|&h| d.try_room(h).is_some());
            let new = d
                .room_at(svc.data, svc.types, x, y, hint, Some(l))
                .unwrap()
                .expect("route point in a room");
            if cur != Some(new) {
                d.client_changes_room(&mut svc, client, cur, Some(new))
                    .unwrap();
                cur = Some(new);
            }
            assert!(d.room(new).active().is_some(), "tick {tick}: built");
        }
        if tick % DEACTIVATION_PERIOD == 0 {
            for (room, _) in d.active_rooms() {
                if d.room_inactivity(room).unwrap() > 10 && d.allows_removal(room).unwrap() {
                    d.remove_active_room(&mut svc, room).unwrap();
                }
            }
        }
        if tick % FREE_PERIOD == 0 {
            d.free_inactive_levels(svc.data, svc.types).unwrap();
        }
        assert_no_freed_room(&d, &types.outdoor, tick);
    }
    // Levels 2 and 3 were freed and regenerated more than once.
    let count = |v: &[u32], id| v.iter().filter(|&&x| x == id).count();
    for id in [2, 3] {
        assert!(count(&types.fake.resets, id) >= 2, "level {id} freed");
        assert!(
            count(&types.fake.generated, id) >= 3,
            "level {id} regenerated"
        );
    }
}
