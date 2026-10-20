//! Tests for the draw log: what it records, the owner assignment, the lines, and
//! that recording changes no outcome (the sim stays deterministic).

use super::*;
use crate::wiring::action::tests::fight::Fight;

fn state_of(f: &Fight) -> String {
    let fx = &f.fx;
    let s = &fx.sim.sys;
    format!(
        "{:?}\n{:?}\n{:?}\n{:?}\n{:?}\n{}",
        fx.game, s.units, s.stats, s.hooks.paths, s.hooks.game_seed, f.fired
    )
}

fn fight(ticks: u32, log: bool) -> (String, Vec<Entry>) {
    if log {
        start();
    }
    let mut f = Fight::new(4);
    for _ in 0..ticks {
        f.tick();
    }
    let entries = stop();
    (state_of(&f), entries)
}

// A fight runs the same with the log on or off; the log holds one marker per tick.
#[test]
fn outcomes_do_not_depend_on_the_log() {
    let (off, none) = fight(40, false);
    assert!(
        none.is_empty(),
        "a log that was never started records nothing"
    );
    let (on, entries) = fight(40, true);
    assert_eq!(off, on, "the game differs with the log on");
    let draws = entries
        .iter()
        .filter(|e| matches!(e, Entry::Draw(_)))
        .count();
    let frames = entries
        .iter()
        .filter(|e| matches!(e, Entry::Frame(_)))
        .count();
    assert!(draws > 0, "the fight draws");
    assert_eq!(frames, 40, "one marker per tick");
    assert!(!is_on());
}

// Each helper logs one record with its arguments, states and returned value.
#[test]
fn every_helper_is_one_record_with_its_math() {
    start();
    let mut s = Seed::init();
    assert_eq!(s.roll(10), 1);
    assert_eq!(s.roll(0), 0); // no step, still a record
    let mut t = Seed::init();
    assert_eq!(t.roll_range(-3, 8), 4);
    let mut u = Seed::init();
    let child = u.derive();
    assert_eq!(child, Seed::init_low(1_791_398_751));
    let mut v = Seed::init();
    assert_eq!(v.mask_range(3, 8), 10);
    let e = stop();
    let ds: Vec<Draw> = e
        .iter()
        .filter_map(|e| match e {
            Entry::Draw(d) => Some(*d),
            Entry::Frame(_) => None,
        })
        .collect();
    assert_eq!(ds.len(), 5, "one record per call, none for the inner step");
    assert_eq!(ds[0].op, Op::Roll(10));
    assert_eq!(
        (ds[0].before, ds[0].after, ds[0].ret),
        (Seed::init(), Seed::new(1_791_398_751, 0), 1)
    );
    assert_eq!(ds[1].before, ds[1].after);
    assert_eq!(ds[2].op, Op::RollRange(-3, 8));
    assert_eq!(ds[2].ret, 4);
    assert_eq!(ds[3].op, Op::Step, "derive is one step");
    assert_eq!(ds[4].ret, 10);
    // the site is this file (track_caller), not rng.rs
    assert!(
        ds.iter()
            .all(|d| d.site.file().ends_with("rng_trace/tests.rs")),
        "{:?}",
        ds[0].site
    );
}

fn d(before: Seed, addr: usize) -> Entry {
    let mut s = before;
    s.raw_step_for_tests();
    Entry::Draw(Draw {
        op: Op::Step,
        before,
        after: s,
        ret: s.lo,
        addr,
        site: Location::caller(),
    })
}

impl Seed {
    fn raw_step_for_tests(&mut self) {
        let v = u64::from(self.lo) * u64::from(crate::rng::K) + u64::from(self.hi);
        self.lo = v as u32;
        self.hi = (v >> 32) as u32;
    }
}

fn stepped(mut s: Seed, n: usize) -> Seed {
    for _ in 0..n {
        s.raw_step_for_tests();
    }
    s
}

// Owners are found by following value chains forward and backward across drains.
#[test]
fn owners_follow_value_chains_both_ways() {
    let g0 = Seed::init_low(1234);
    let u0 = Seed::init_low(77);
    let x0 = Seed::init_low(5);
    let mut o = Owners::new();
    // first drain: no previous values; the game and the unit are found
    // backward from their values now, the other seed stays "other"
    let entries = vec![
        d(g0, 1),
        Entry::Frame(1),
        d(stepped(g0, 1), 1),
        d(x0, 99),
        d(u0, 2),
    ];
    let end = vec![
        Known {
            owner: Owner::Game,
            seed: stepped(g0, 2),
            addr: 1,
        },
        Known {
            owner: Owner::Unit(1, 3),
            seed: stepped(u0, 1),
            addr: 2,
        },
    ];
    let r = o.resolve(entries, end);
    let got: Vec<(i32, Option<Owner>)> = r.iter().map(|r| (r.frame, r.owner)).collect();
    assert_eq!(
        got,
        [
            (0, Some(Owner::Game)),
            (1, Some(Owner::Game)),
            (1, None),
            (1, Some(Owner::Unit(1, 3)))
        ]
    );
    // second drain: forward from the previous values, even when the unit is
    // gone at the end (removed in the frame) and the address moved
    // (the game seed drawn on a copy at another address is still the game's;
    // a later copy of a state the chain already passed is not)
    let entries = vec![
        Entry::Frame(2),
        d(stepped(u0, 1), 50),
        d(stepped(g0, 2), 51),
        d(stepped(g0, 2), 52),
    ];
    let end = vec![Known {
        owner: Owner::Game,
        seed: stepped(g0, 3),
        addr: 7,
    }];
    let r = o.resolve(entries, end);
    let got: Vec<(i32, Option<Owner>)> = r.iter().map(|r| (r.frame, r.owner)).collect();
    assert_eq!(
        got,
        [
            (2, Some(Owner::Unit(1, 3))),
            (2, Some(Owner::Game)),
            (2, None)
        ]
    );
    // lines
    let line = o.line(&r[0]);
    assert!(
        line.starts_with(r#"{"type":"draw","via":"helper","op":"step","site":""#),
        "{line}"
    );
    assert!(
        line.contains(r#""seed":"unit 1:3","#)
            && line.contains(r#""frame":2,"owner":"unit 1:3","tid":0,"seq":0}"#),
        "{line}"
    );
    let other = Resolved {
        frame: 2,
        owner: None,
        draw: match d(x0, 99) {
            Entry::Draw(d) => d,
            Entry::Frame(_) => unreachable!(),
        },
    };
    let l = o.line(&other);
    assert!(
        l.contains(r#""seed":"D2RUST#0""#) && l.contains(r#""owner":"other""#),
        "{l}"
    );
}

// The header and footer lines have the documented keys and escaping.
#[test]
fn header_and_footer() {
    assert_eq!(
        header_line("d2-client state-dump 0.1.0", "2026-10-09", "a \"b\"", 1234),
        concat!(
            r#"{"type":"header","format":"rng-raw-1","side":"d2rust","tool":"d2-client state-dump 0.1.0","#,
            r#""date":"2026-10-09","command":"a \"b\"","seed":1234,"frames":true,"owners":true}"#
        )
    );
    assert_eq!(
        footer_line(3, &["x".into()]),
        r#"{"type":"footer","events":3,"notes":["x"]}"#
    );
}
