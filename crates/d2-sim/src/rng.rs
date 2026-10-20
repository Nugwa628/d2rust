//! The D2 seeded RNG: a 64-bit multiply-with-carry state split into two
//! u32 words, the draw helpers built on it, and the seed setters.
//!
//! Every game object that needs randomness owns a [`Seed`]. Seeds are value
//! types: copying one copies the generator, and the copy then draws the same sequence.
//!
//! With the `rng-trace` feature every draw is also offered to the debug
//! log of `crate::debug::rng_trace`, which records it only while an export has started it
//! on this thread; the values and states are the same either way.

#[cfg(feature = "rng-trace")]
use crate::debug::rng_trace as trace;

/// Step multiplier of the multiply-with-carry generator.
pub const K: u32 = 0x6AC6_90C5;

/// High word of every freshly initialized seed.
pub const INIT_HI: u32 = 666;

/// One seed: the generator state. D2MOO: `D2SeedStrc`.
/// `repr(C)` pins `lo` at offset 0 and `hi` at offset 4, so the pair reads as one
/// little-endian u64.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Seed {
    /// Low word (offset 0); a draw's raw output.
    pub lo: u32,
    /// High word (offset 4); the multiply-with-carry carry.
    pub hi: u32,
}

impl Default for Seed {
    /// `{1, 666}`, the state `init()` writes.
    fn default() -> Self {
        Self::init()
    }
}

impl Seed {
    /// The seed `{lo, hi}` as given.
    pub const fn new(lo: u32, hi: u32) -> Self {
        Self { lo, hi }
    }

    /// The default seed `{1, 666}`.
    pub const fn init() -> Self {
        Self::init_low(1)
    }

    /// The seed `{x, 666}`: the given low word with the fixed initial high word.
    pub const fn init_low(x: u32) -> Self {
        Self { lo: x, hi: INIT_HI }
    }

    /// Overwrites the whole state with `{lo, hi}`.
    pub fn set(&mut self, lo: u32, hi: u32) {
        *self = Self::new(lo, hi);
    }

    /// One step: `v = lo·K + hi`, state becomes `{v mod 2^32, v >> 32}`. Returns the new
    /// low word.
    #[cfg_attr(feature = "rng-trace", track_caller)]
    pub fn step(&mut self) -> u32 {
        #[cfg(feature = "rng-trace")]
        let before = *self;
        let lo = self.raw_step();
        #[cfg(feature = "rng-trace")]
        trace::draw(self, trace::Op::Step, before, lo);
        lo
    }

    /// The step itself, without logging (the helpers log one draw each).
    fn raw_step(&mut self) -> u32 {
        let v = u64::from(self.lo) * u64::from(K) + u64::from(self.hi);
        // Truncations are the rule: low and high halves of v.
        self.lo = v as u32;
        self.hi = (v >> 32) as u32;
        self.lo
    }

    /// A child seed derived from this one: one step, then `init_low(lo')` on the child.
    #[cfg_attr(feature = "rng-trace", track_caller)]
    pub fn derive(&mut self) -> Seed {
        Seed::init_low(self.step())
    }

    /// `roll(n)`: a value in `[0, n)`. `n < 1` (signed) returns 0 **without stepping**.
    /// Otherwise one step and `lo' mod n` (unsigned). For a power-of-two `n` this equals
    /// masking with `n − 1` (`tests::power_of_two_branch_equals_modulo`).
    #[cfg_attr(feature = "rng-trace", track_caller)]
    pub fn roll(&mut self, n: i32) -> u32 {
        #[cfg(feature = "rng-trace")]
        let before = *self;
        let r = self.raw_roll(n);
        #[cfg(feature = "rng-trace")]
        trace::draw(self, trace::Op::Roll(n), before, r);
        r
    }

    /// The roll without logging.
    fn raw_roll(&mut self, n: i32) -> u32 {
        if n < 1 {
            return 0;
        }
        self.raw_step() % n.unsigned_abs()
    }

    /// `mask(n)`: one step, `lo' & (n−1)`. No range check: `n = 0` gives `lo'`, and a
    /// non-power-of-two gives a biased mask (`tests::mask_non_power_of_two_is_biased`).
    #[cfg_attr(feature = "rng-trace", track_caller)]
    pub fn mask(&mut self, n: u32) -> u32 {
        #[cfg(feature = "rng-trace")]
        let before = *self;
        let r = self.raw_step() & n.wrapping_sub(1);
        #[cfg(feature = "rng-trace")]
        trace::draw(self, trace::Op::Mask(n), before, r);
        r
    }

    /// `mask_range(min, n)`: one step, `(lo' & (n−1)) + min`, in wrapping 32-bit arithmetic
    /// read as i32.
    #[cfg_attr(feature = "rng-trace", track_caller)]
    pub fn mask_range(&mut self, min: i32, n: u32) -> i32 {
        #[cfg(feature = "rng-trace")]
        let before = *self;
        let r = ((self.raw_step() & n.wrapping_sub(1)) as i32).wrapping_add(min);
        #[cfg(feature = "rng-trace")]
        trace::draw(self, trace::Op::MaskRange(min, n), before, r as u32);
        r
    }

    /// `roll_range(min, n)`: `n < 1` returns `min` without stepping, else `roll(n) + min`
    /// (wrapping, read as i32).
    #[cfg_attr(feature = "rng-trace", track_caller)]
    pub fn roll_range(&mut self, min: i32, n: i32) -> i32 {
        #[cfg(feature = "rng-trace")]
        let before = *self;
        let r = if n < 1 {
            min
        } else {
            (self.raw_roll(n) as i32).wrapping_add(min)
        };
        #[cfg(feature = "rng-trace")]
        trace::draw(self, trace::Op::RollRange(min, n), before, r as u32);
        r
    }
}

/// The time-value mix used to seed a game from the clock. `x` is the caller's sum
/// `time() + GetTickCount() + v` (u32 wrap), supplied as an input: the sim never reads the
/// clock. Three LCG rounds, then `& 0x7FFFFFFF`.
pub fn time_value(x: u32) -> u32 {
    let mut x = x;
    for _ in 0..3 {
        x = x.wrapping_mul(0x0019_660D).wrapping_add(0x3C6E_F35F);
    }
    x & 0x7FFF_FFFF
}

#[cfg(test)]
mod tests {
    // Fixed step / roll / mask vectors and property sweeps over the generator.
    use super::*;

    const START: Seed = Seed::init();

    #[test]
    fn step_vectors() {
        let cases = [
            ((1, 666), (1_791_398_751, 0)),
            ((0, 666), (666, 0)),
            ((666, 0), (3_365_183_618, 277)),
            ((0xFFFF_FFFF, 0xFFFF_FFFF), (0x9539_6F3A, 0x6AC6_90C5)),
        ];
        for ((lo, hi), (lo2, hi2)) in cases {
            let mut s = Seed::new(lo, hi);
            assert_eq!(s.step(), lo2);
            assert_eq!(s, Seed::new(lo2, hi2), "step from {{{lo}, {hi}}}");
        }
    }

    #[test]
    fn zero_is_fixed_point() {
        let mut s = Seed::new(0, 0);
        for _ in 0..4 {
            assert_eq!(s.step(), 0);
        }
        assert_eq!(s, Seed::new(0, 0));
    }

    #[test]
    fn five_steps_from_default() {
        let lo = [
            1_791_398_751,
            791_599_131,
            671_516_612,
            3_064_641_593,
            3_217_527_747,
        ];
        let hi = [0, 747_178_749, 330_169_957, 280_084_454, 1_278_238_622];
        let mut s = Seed::default();
        for i in 0..5 {
            assert_eq!(s.step(), lo[i]);
            assert_eq!(s.hi, hi[i]);
        }
    }

    #[test]
    fn previous_hi_from_two_low_words() {
        // hi = (lo' − lo·K) mod 2^32, so the previous high word follows from two low words.
        let mut s = Seed::new(971_488_495, 666);
        for _ in 0..16 {
            let before = s;
            let lo2 = s.step();
            assert_eq!(lo2.wrapping_sub(before.lo.wrapping_mul(K)), before.hi);
        }
    }

    #[test]
    fn roll_nonpositive_does_not_step() {
        for n in [0, -5, i32::MIN] {
            let mut s = START;
            assert_eq!(s.roll(n), 0);
            assert_eq!(s, START);
        }
    }

    #[test]
    fn roll_vectors() {
        for (n, want) in [
            (1, 0),
            (8, 7),
            (10, 1),
            (100, 51),
            (0x7FFF_FFFF, 1_791_398_751),
        ] {
            let mut s = START;
            assert_eq!(s.roll(n), want, "roll({n})");
            assert_eq!(s, Seed::new(1_791_398_751, 0));
        }
    }

    #[test]
    fn mask_vectors() {
        for (n, want) in [(0, 1_791_398_751), (16, 15), (10, 9)] {
            let mut s = START;
            assert_eq!(s.mask(n), want, "mask({n})");
            assert_eq!(s, Seed::new(1_791_398_751, 0));
        }
        let mut s = START;
        assert_eq!(s.mask_range(3, 8), 10);
    }

    #[test]
    fn roll_range_vectors() {
        let mut s = START;
        assert_eq!(s.roll_range(5, 0), 5);
        assert_eq!(s, START, "n < 1 must not step");
        for ((min, n), want) in [((5, 10), 6), ((-3, 8), 4)] {
            let mut s = START;
            assert_eq!(s.roll_range(min, n), want, "roll_range({min}, {n})");
        }
    }

    #[test]
    fn time_value_vectors() {
        assert_eq!(time_value(0), 1_372_387_049);
        assert_eq!(time_value(12345), 185_352_726);
    }

    #[test]
    fn setters_and_derive() {
        let mut s = Seed::new(5, 6);
        s.set(7, 8);
        assert_eq!(s, Seed::new(7, 8));
        assert_eq!(Seed::init_low(42), Seed::new(42, 666));
        // The DRLG seed's first step is the level start seed.
        let mut drlg = Seed::init_low(644_409_375);
        assert_eq!(drlg.derive(), Seed::new(4_014_346_869, 666));
        assert_eq!(drlg, Seed::new(4_014_346_869, 268_778_232));
    }

    #[test]
    fn recorded_values() {
        // Fixed values of known draw sequences.
        let mut s = Seed::new(0, 666);
        assert_eq!(s.roll(3), 0);
        assert_eq!(s, Seed::new(666, 0));
        assert_eq!(s.roll(3), 2);
        assert_eq!(s, Seed::new(3_365_183_618, 277));

        let mut s = Seed::new(971_488_495, 666);
        assert_eq!(s.step(), 7_657_093);
        assert_eq!(s, Seed::new(7_657_093, 405_200_438));
        assert_eq!(s.step(), 664_322_703);
        assert_eq!(s, Seed::new(664_322_703, 3_193_715));

        let mut s = Seed::new(1_936_801_471, 624_310_379);
        assert_eq!(s.roll(3), 2);
        assert_eq!(s, Seed::new(1_281_421_670, 807_825_114));
    }

    #[test]
    fn seed_layout_lo_then_hi() {
        assert_eq!(std::mem::size_of::<Seed>(), 8);
        assert_eq!(std::mem::offset_of!(Seed, lo), 0);
        assert_eq!(std::mem::offset_of!(Seed, hi), 4);
        // Read as one little-endian u64, lo is the low half: the step's
        // v = lo·K + hi is exactly the new state read that way.
        let mut s = Seed::new(0x0102_0304, 0x0506_0708);
        let v = u64::from(s.lo) * u64::from(K) + u64::from(s.hi);
        s.step();
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&s.lo.to_le_bytes());
        bytes[4..].copy_from_slice(&s.hi.to_le_bytes());
        assert_eq!(u64::from_le_bytes(bytes), v);
    }

    #[test]
    fn copy_duplicates_generator() {
        // Only the two words: 8 bytes, nothing else.
        assert_eq!(std::mem::size_of::<Seed>(), 2 * std::mem::size_of::<u32>());
        let mut a = Seed::new(4_014_346_871, 666);
        a.step();
        let mut b = a; // struct copy
        for n in [3, 100, 7, 1 << 20, 0x7FFF_FFFF] {
            assert_eq!(a.roll(n), b.roll(n));
            assert_eq!(a.step(), b.step());
            assert_eq!(a.mask(16), b.mask(16));
            assert_eq!(a, b);
        }
    }

    /// The low word the next step would produce, without stepping `s`.
    fn next_lo(mut s: Seed) -> u32 {
        s.step()
    }

    /// Small deterministic sweep of low words for property tests.
    fn sweep() -> impl Iterator<Item = Seed> {
        let mut s = Seed::new(971_488_495, 666);
        (0..2000).map(move |_| {
            s.step();
            s
        })
    }

    #[test]
    fn power_of_two_branch_equals_modulo() {
        for start in sweep() {
            for k in 0..31 {
                let n = 1i32 << k;
                let mut s = start;
                let lo2 = next_lo(s);
                let got = s.roll(n);
                // Power-of-two branch and modulo branch agree.
                assert_eq!(lo2 & (n as u32 - 1), lo2 % n as u32);
                assert_eq!(got, lo2 & (n as u32 - 1));
            }
        }
    }

    #[test]
    fn roll_results_in_range() {
        let ns = [1, 2, 3, 7, 10, 100, 1000, 0x7FFF_FFFF];
        let mins = [0, -10, 5, i32::MIN / 2];
        for start in sweep() {
            for n in ns {
                let mut s = start;
                assert!(s.roll(n) < n as u32);
                for min in mins {
                    let mut s = start;
                    let r = i64::from(s.roll_range(min, n));
                    let lo = i64::from(min);
                    assert!(lo <= r && r < lo + i64::from(n), "{min} {n} {r}");
                }
            }
        }
    }

    #[test]
    fn init_then_init_low_and_inline_store() {
        for x in [0, 1, 42, 644_409_375, u32::MAX] {
            // init() then init_low(x): the init() is overwritten.
            let mut s = Seed::init();
            assert_eq!(s, Seed::new(1, INIT_HI));
            s = Seed::init_low(x);
            assert_eq!(s, Seed::init_low(x));
            // An inline store of {x, 0x29A} has the same effect.
            let mut t = Seed::new(9, 9);
            t.set(x, 0x29A);
            assert_eq!(t, s);
        }
    }

    #[test]
    fn roll_nonpositive_returns_zero_seed_unchanged() {
        for start in [START, Seed::new(7_657_093, 405_200_438)] {
            for n in [0, -1, -5, i32::MIN] {
                let mut s = start;
                assert_eq!(s.roll(n), 0);
                assert_eq!(s, start);
            }
        }
    }

    #[test]
    fn mask_non_power_of_two_is_biased() {
        // mask(10) = lo' & 9: only 0, 1, 8 and 9 can occur.
        let mut seen = [false; 10];
        for start in sweep() {
            let mut s = start;
            let lo2 = next_lo(start);
            let v = s.mask(10);
            assert_eq!(v, lo2 & 9);
            seen[v as usize] = true;
        }
        assert_eq!(
            seen,
            [true, true, false, false, false, false, false, false, true, true]
        );
    }

    #[test]
    fn roll_is_low_word_modulo() {
        // n not dividing 2^32: the plain unsigned lo' mod n, with its modulo bias.
        for start in sweep() {
            for n in [3, 7, 100, 0x7FFF_FFFF] {
                let mut s = start;
                let lo2 = next_lo(start);
                assert_eq!(s.roll(n), lo2 % n as u32);
            }
        }
        // lo' = 0xFFFFFFFF with n = 0x7FFFFFFF wraps to 1 (2^32 is not a
        // multiple of n, so 0 and 1 have one more preimage than the rest).
        let mut s = Seed::new(0, 0xFFFF_FFFF);
        assert_eq!(s.roll(0x7FFF_FFFF), 1);
    }
}
