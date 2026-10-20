//! Gap tests for population: the dead t = 1 branch
//! and the tentacle offset set the caller can pass.

use super::placement::flags;
use super::room::{boss_or_pack, populate_room};
use super::spawn::party;
use super::tests::fake::{ctx, seed_for, state_with, tables, Fake, R0};
use super::{CoordRect, Region};
use crate::rng::Seed;

/// Set 0 then set 1, from the rules (not the code under test).
const OFFSETS: [[(i32, i32); 6]; 2] = [
    [(-1, -4), (1, 4), (1, -3), (-1, 3), (0, 2), (0, -2)],
    [(-3, -1), (3, 1), (2, -1), (-2, 1), (1, 0), (-1, 0)],
];

fn steps(mut s: Seed, n: usize) -> Seed {
    for _ in 0..n {
        s.step();
    }
    s
}

/// Room-seed steps from `s0` to `end`.
fn steps_between(s0: Seed, end: Seed) -> usize {
    (0..256)
        .find(|&n| steps(s0, n) == end)
        .expect("end reachable from s0")
}

/// One population of a one-rectangle room whose first density try hits,
/// with region list [(class 5, rarity 1)] and MonUMin = MonUMax = 0, on
/// room seed `room_seed`. Returns (allocated classes, room-seed steps).
fn populate_with_room_seed(room_seed: Seed) -> (Vec<i32>, usize) {
    let mut t = tables();
    t.monstats[5].min_grp = 1;
    t.monstats[5].max_grp = 1;
    let mut st = state_with(&t, 2);
    {
        let r = st.regions.get_mut(2).unwrap();
        r.entries[0].class = 5;
        r.entries[0].rarity = 1;
        r.total_rarity = 1;
        r.mon_count = 1;
        r.entry_count = 1;
    }
    let mut f = Fake::new();
    f.game_seed = Seed::init_low(429); // first density try hits
    f.coords = vec![CoordRect {
        rect: [1, 1, 2, 5],
        node_flag: 0,
        index: 1,
    }];
    f.room_seeds.insert(R0, room_seed);
    populate_room(&mut ctx(&t, &mut st, &mut f), R0);
    assert_eq!(st.regions.get(2).unwrap().rooms_with_spawns, 1);
    (f.allocs(), steps_between(room_seed, f.room_seeds[&R0]))
}

#[test]
fn kind_one_takes_the_pack_path_not_the_dead_branch() {
    // Room-seed draws: the pick's roll(1) (one step), then only
    // (U = 0, MonUMin = MonUMax = 0). Third-draw value ≤ 35 → t = 1,
    // > 35 → t = 2.
    let one = seed_for(100, &[0, 30]);
    let two = seed_for(100, &[0, 99]);
    let r = Region::default();
    assert_eq!(boss_or_pack(&r, &mut steps(one, 1)), 1);
    assert_eq!(boss_or_pack(&r, &mut steps(two, 1)), 2);
    // t = 1 is a pack exactly like t = 2: the same units and the same
    // number of room-seed steps (no extra room-seed step and no pattern
    // spawn of the dead path).
    let (u1, n1) = populate_with_room_seed(one);
    let (u2, n2) = populate_with_room_seed(two);
    assert_eq!(u1, [5]);
    assert_eq!(u2, [5]);
    assert_eq!(n1, n2);
}

#[test]
fn tentacle_caller_passes_only_set_0_or_1() {
    let mut t = tables();
    t.monstats[261].minion1 = 262;
    t.monstats[261].party_min = 6;
    t.monstats[261].party_max = 6;
    let mut st = state_with(&t, 2);
    // Every creation flag value a party runs for (no 0x40): the offsets
    // used are those of set 0 (flag 0x04) or set 1, never a third set.
    for cflags in 0..=u16::MAX {
        if cflags & flags::NO_PARTY != 0 {
            continue;
        }
        let set = if cflags & flags::TENTACLE_SET0 != 0 {
            0
        } else {
            1
        };
        let mut f = Fake::new();
        let head = f.add_unit(261, 100, 100, seed_for(6, &[0]));
        party(&mut ctx(&t, &mut st, &mut f), head, 261, cflags);
        let pos: Vec<(i32, i32)> = f.units[1..].iter().map(|u| (u.x, u.y)).collect();
        // k = 0, then (k + 5) mod 6: 0, 5, 4, 3, 2, 1.
        let want: Vec<(i32, i32)> = [0, 5, 4, 3, 2, 1]
            .iter()
            .map(|&k| {
                let (ox, oy) = OFFSETS[set][k];
                (100 + ox, 100 + oy)
            })
            .collect();
        assert_eq!(pos, want, "flags {cflags:#x}");
    }
}
