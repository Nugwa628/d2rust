// Spec: specs/sim/rng.md
//! Property tests of the seeded RNG (`d2_sim::rng`): the step, its
//! identities and the draw helpers, derived from the spec's rules on
//! arbitrary seeds and arguments.
//!
//! Stepping identity used below (follows from §2): with Z := lo·K + hi
//! and M := K·2^32 − 1, one step maps Z to Z' with Z' ≡ K·Z (mod M),
//! because K·2^32 ≡ 1 (mod M). So n steps multiply Z by K^n mod M,
//! which gives an independent jump-ahead model of the generator.

use d2_sim::rng::{Seed, INIT_HI, K};
use proptest::prelude::*;

fn config(default: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

const M: u128 = (K as u128) * (1u128 << 32) - 1;

fn z(s: Seed) -> u128 {
    u128::from(s.lo) * u128::from(K) + u128::from(s.hi)
}

fn mulmod(a: u128, b: u128) -> u128 {
    // M < 2^63, so a, b < M keep the product below 2^126.
    (a % M) * (b % M) % M
}

fn powmod(mut b: u128, mut e: u64) -> u128 {
    let mut r = 1u128;
    b %= M;
    while e > 0 {
        if e & 1 == 1 {
            r = mulmod(r, b);
        }
        b = mulmod(b, b);
        e >>= 1;
    }
    r
}

fn seed() -> impl Strategy<Value = Seed> {
    prop_oneof![
        (any::<u32>(), any::<u32>()).prop_map(|(lo, hi)| Seed::new(lo, hi)),
        any::<u32>().prop_map(Seed::init_low),
        Just(Seed::new(u32::MAX, u32::MAX)),
        Just(Seed::new(u32::MAX, K - 1)),
        Just(Seed::new(0, 0)),
    ]
}

/// One draw of a random sequence, with the steps it must take (§3).
#[derive(Clone, Debug)]
enum Draw {
    Step,
    Roll(i32),
    Mask(u32),
    MaskRange(i32, u32),
    RollRange(i32, i32),
    Derive,
}

fn draw() -> impl Strategy<Value = Draw> {
    prop_oneof![
        Just(Draw::Step),
        any::<i32>().prop_map(Draw::Roll),
        (-3i32..70).prop_map(Draw::Roll),
        any::<u32>().prop_map(Draw::Mask),
        (any::<i32>(), any::<u32>()).prop_map(|(m, n)| Draw::MaskRange(m, n)),
        (any::<i32>(), any::<i32>()).prop_map(|(m, n)| Draw::RollRange(m, n)),
        Just(Draw::Derive),
    ]
}

/// Steps a draw takes (§3 table, "Steps" column; derive: §5).
fn steps(d: &Draw) -> u64 {
    match *d {
        Draw::Roll(n) | Draw::RollRange(_, n) if n < 1 => 0,
        _ => 1,
    }
}

proptest! {
    #![proptest_config(config(512))]

    // §2: the step formula on the u64 v; hi' ≤ K, with hi' = K only for
    // lo = hi = 2^32 − 1.
    #[test]
    fn step_is_the_multiply_with_carry(s in seed()) {
        let mut t = s;
        let out = t.step();
        let v = u64::from(s.lo) * u64::from(K) + u64::from(s.hi);
        prop_assert_eq!(out, v as u32);
        prop_assert_eq!(t, Seed::new(v as u32, (v >> 32) as u32));
        prop_assert!(t.hi <= K);
        if t.hi == K {
            prop_assert_eq!(s, Seed::new(u32::MAX, u32::MAX));
        }
    }

    // §2: the old hi is recoverable from two consecutive low words.
    #[test]
    fn hi_recovers_from_consecutive_lows(s in seed()) {
        let mut t = s;
        let lo2 = t.step();
        prop_assert_eq!(lo2.wrapping_sub(s.lo.wrapping_mul(K)), s.hi);
    }

    // §2 (derived): n steps multiply Z = lo·K + hi by K^n modulo
    // K·2^32 − 1; after one step Z' ≤ M.
    #[test]
    fn n_steps_jump_ahead(s in seed(), n in 0u64..300) {
        let mut t = s;
        for _ in 0..n {
            t.step();
        }
        if n > 0 {
            prop_assert!(z(t) <= M);
        }
        prop_assert_eq!(z(t) % M, mulmod(z(s), powmod(u128::from(K), n)));
    }

    // §3: each helper's result and the steps it takes, against the step
    // of a copy (seeds are values, edge case 4).
    #[test]
    fn helpers_match_the_table(s in seed(), draws in prop::collection::vec(draw(), 0..40)) {
        let mut a = s;
        let mut taken = 0u64;
        for d in &draws {
            let mut r = a;
            let before = a;
            match *d {
                Draw::Step => {
                    let lo = r.step();
                    prop_assert_eq!(a.step(), lo);
                }
                Draw::Roll(n) => {
                    let got = a.roll(n);
                    if n < 1 {
                        prop_assert_eq!(got, 0);
                    } else {
                        let lo = r.step();
                        let n = n as u32;
                        prop_assert_eq!(got, lo % n);
                        prop_assert!(got < n);
                        if n.is_power_of_two() {
                            prop_assert_eq!(got, lo & (n - 1));
                        }
                    }
                }
                Draw::Mask(n) => {
                    let lo = r.step();
                    prop_assert_eq!(a.mask(n), lo & n.wrapping_sub(1));
                }
                Draw::MaskRange(min, n) => {
                    let lo = r.step();
                    prop_assert_eq!(
                        a.mask_range(min, n),
                        (lo & n.wrapping_sub(1)).wrapping_add(min as u32) as i32
                    );
                }
                Draw::RollRange(min, n) => {
                    let got = a.roll_range(min, n);
                    if n < 1 {
                        prop_assert_eq!(got, min);
                    } else {
                        let lo = r.step();
                        let off = lo % n as u32;
                        prop_assert_eq!(got, (off.wrapping_add(min as u32)) as i32);
                        // §3 r5: in [min, min + n) on the 32-bit circle.
                        prop_assert!((got.wrapping_sub(min) as u32) < n as u32);
                    }
                }
                Draw::Derive => {
                    let lo = r.step();
                    let child = a.derive();
                    prop_assert_eq!(child, Seed::new(lo, INIT_HI));
                }
            }
            if steps(d) == 0 {
                prop_assert_eq!(a, before);
            } else {
                prop_assert_eq!(a, r);
            }
            taken += steps(d);
        }
        // The whole sequence moved the seed by exactly the counted steps.
        let mut b = s;
        for _ in 0..taken {
            b.step();
        }
        prop_assert_eq!(a, b);
    }

    // §4: setters; edge case 4: a copy is an independent generator with
    // the same sequence.
    #[test]
    fn setters_and_copies(lo in any::<u32>(), hi in any::<u32>(), n in 0usize..20) {
        prop_assert_eq!(Seed::init(), Seed::new(1, 666));
        prop_assert_eq!(Seed::default(), Seed::init());
        prop_assert_eq!(Seed::init_low(lo), Seed::new(lo, 666));
        let mut s = Seed::init();
        s.set(lo, hi);
        prop_assert_eq!(s, Seed::new(lo, hi));
        let mut c = s;
        let a: Vec<u32> = (0..n).map(|_| s.step()).collect();
        let b: Vec<u32> = (0..n).map(|_| c.step()).collect();
        prop_assert_eq!(a, b);
        prop_assert_eq!(s, c);
    }
}

// §2: {0, 0} and {2^32 − 1, K − 1} (Z = M ≡ 0) are fixed points.
#[test]
fn fixed_points() {
    for s in [Seed::new(0, 0), Seed::new(u32::MAX, K - 1)] {
        let mut t = s;
        for _ in 0..4 {
            t.step();
            assert_eq!(t, s);
        }
    }
}
