//! Movement under [`TickRate`]: a scripted walk at the base rate stays
//! exactly as it plays today, and a faster rate covers the same ground in
//! the same real time.

use super::fake::Ctx;
use super::{setup, P};
use crate::path::tables::PathTables;
use crate::path::walk::request::handle_message;
use crate::path::walk::step::{Step, Walk};
use crate::tick::TickRate;

/// Walk message (mode 2) and run message (mode 3).
const WALK: u8 = 0x01;
const RUN: u8 = 0x03;

/// One tick of the scripted walk: precise position, step result, mode
/// and stamina.
type Sample = (u32, u32, Step, u32, i32);

/// Runs player ticks until the walk stops (at most `max`).
fn ticks(t: &PathTables, c: &mut Ctx, max: usize) -> Vec<Sample> {
    let mut out = Vec::new();
    for _ in 0..max {
        c.g.frame += 1;
        let s = Walk { t, c: &mut *c }.player_event0(P).unwrap();
        let p = &c.w.paths[&P];
        let u = &c.u.units[&P];
        out.push((
            p.precise_x,
            p.precise_y,
            s,
            u.mode,
            u.stats.get(&10).copied().unwrap_or(0),
        ));
        if s == Step::Stopped {
            break;
        }
    }
    out
}

/// A walk around a wall, a run back past it and a short walk: three legs
/// that turn, detour, run and stop.
fn scripted_walk(rate: TickRate) -> Vec<Vec<Sample>> {
    let (t, mut c) = setup(200, 200, 100, 100);
    c.g.tick_rate = rate;
    for y in 96..=104 {
        c.w.wall(104, y);
    }
    let legs: [(u8, u32, u32); 3] = [(WALK, 108, 101), (RUN, 97, 106), (WALK, 99, 99)];
    let mut out = Vec::new();
    for (msg, x, y) in legs {
        handle_message(&t, &mut c, P, msg, x, y).unwrap();
        out.push(ticks(&t, &mut c, 4000));
    }
    out
}

/// FNV-1a over every sample, so the whole walk is pinned by one number.
fn fingerprint(legs: &[Vec<Sample>]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |v: u64| {
        for b in v.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for leg in legs {
        eat(leg.len() as u64);
        for &(x, y, s, mode, stamina) in leg {
            eat(u64::from(x));
            eat(u64::from(y));
            eat(s as u64);
            eat(u64::from(mode));
            eat(stamina as u32 as u64);
        }
    }
    h
}

#[test]
fn scripted_walk_at_25_hz_is_unchanged() {
    let legs = scripted_walk(TickRate::BASE);
    let lens: Vec<usize> = legs.iter().map(Vec::len).collect();
    let last = |l: &Vec<Sample>| *l.last().unwrap();
    // Recorded from the movement code before it read the tick rate: tick
    // counts per leg, where each leg ends (position, mode, stamina) and a
    // fingerprint of every tick.
    assert_eq!(lens, [41, 27, 20]);
    assert_eq!(
        last(&legs[0]),
        (0x6C8000, 0x658000, Step::Stopped, 1, 25600)
    );
    assert_eq!(
        last(&legs[1]),
        (0x618000, 0x6A8000, Step::Stopped, 1, 24520)
    );
    assert_eq!(
        last(&legs[2]),
        (0x638000, 0x638000, Step::Stopped, 1, 24520)
    );
    assert_eq!(fingerprint(&legs), 0xf9e8_2acb_1e61_f77e);
}

#[test]
fn faster_rates_walk_the_same_ground_in_the_same_time() {
    let base = scripted_walk(TickRate::BASE);
    // 100 Hz divides into base ticks; 60 and 144 Hz do not, so their
    // per-tick shares of a speed vary from tick to tick.
    for hz in [60, 100, 144] {
        let fast = scripted_walk(TickRate::new(hz).unwrap());
        for (b, f) in base.iter().zip(&fast) {
            let (bx, by, _, _, _) = *b.last().unwrap();
            let (fx, fy, s, _, _) = *f.last().unwrap();
            // Same destination.
            assert_eq!((fx, fy, s), (bx, by, Step::Stopped), "{hz} Hz");
            // The same real time: hz / 25 ticks per base tick. The step
            // that reaches a path point is cut short, which wastes up to
            // one tick per point; shorter ticks waste less, so the fast
            // walk is never slower and at most a few points' worth faster.
            let want = b.len() * hz as usize / 25 + 1;
            assert!(
                f.len() <= want && f.len() * 10 >= want * 9,
                "{hz} Hz: {} vs {want}",
                f.len()
            );
        }
        // Running drains about the same stamina per second.
        let drain = |legs: &[Vec<Sample>]| 25600 - legs[1].last().unwrap().4;
        let (b, f) = (drain(&base), drain(&fast));
        assert!(
            f.abs_diff(b) * 10 <= b.unsigned_abs(),
            "{hz} Hz: {f} vs {b}"
        );
    }
}
