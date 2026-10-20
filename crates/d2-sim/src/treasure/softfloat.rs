//! The IEEE binary64 operations of the NoDrop scaling,
//! evaluated with integers: round to nearest even after every operation,
//! as the base game's x87 code does with 53-bit precision. The project
//! rules forbid host floating point in `d2-sim`.
//!
//! Only zero and finite values whose exponent stays in the binary64
//! normal range are represented. Anything else (infinity, NaN, subnormal,
//! overflow) is reported as [`OutOfRange`]: on x87 the exponent range of
//! intermediates depends on the precision-control setting (an open
//! question), which nothing has measured yet.

/// A value outside the represented subset (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

/// Smallest and largest unbiased exponent of a binary64 normal number.
const MIN_EXP: i32 = -1022;
const MAX_EXP: i32 = 1023;

/// A binary64 value: zero, or `±m × 2^e` with `2^52 ≤ m < 2^53`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct F64 {
    neg: bool,
    /// 0 for zero, else the 53-bit significand.
    m: u64,
    /// Exponent of the significand's least significant bit.
    e: i32,
}

impl F64 {
    pub const ZERO: F64 = F64 {
        neg: false,
        m: 0,
        e: 0,
    };
    pub const ONE: F64 = F64 {
        neg: false,
        m: 1 << 52,
        e: -52,
    };

    /// The exact value of an integer up to 2^53 in magnitude (`fild`).
    pub fn from_i64(v: i64) -> Result<F64, OutOfRange> {
        round(v < 0, u128::from(v.unsigned_abs()), 0)
    }

    pub fn is_zero(self) -> bool {
        self.m == 0
    }

    /// The raw IEEE bit pattern (tests compare it with a host oracle).
    pub fn to_bits(self) -> u64 {
        if self.m == 0 {
            return u64::from(self.neg) << 63;
        }
        let biased = (self.e + 52 + 1023) as u64;
        (u64::from(self.neg) << 63) | (biased << 52) | (self.m & ((1 << 52) - 1))
    }

    pub fn negate(self) -> F64 {
        F64 {
            neg: !self.neg,
            ..self
        }
    }

    pub fn mul_rne(self, o: F64) -> Result<F64, OutOfRange> {
        if self.m == 0 || o.m == 0 {
            return Ok(F64 {
                neg: self.neg != o.neg,
                ..F64::ZERO
            });
        }
        let p = u128::from(self.m) * u128::from(o.m);
        round(self.neg != o.neg, p, self.e + o.e)
    }

    pub fn div_rne(self, o: F64) -> Result<F64, OutOfRange> {
        if o.m == 0 {
            return Err(OutOfRange);
        }
        if self.m == 0 {
            return Ok(F64 {
                neg: self.neg != o.neg,
                ..F64::ZERO
            });
        }
        let num = u128::from(self.m) << 64;
        let den = u128::from(o.m);
        // The quotient has at least 64 bits, so folding the remainder into
        // its lowest bit keeps round-to-nearest-even exact.
        let q = (num / den) | u128::from(num % den != 0);
        round(self.neg != o.neg, q, self.e - o.e - 64)
    }

    pub fn add_rne(self, o: F64) -> Result<F64, OutOfRange> {
        if o.m == 0 {
            if self.m == 0 {
                // (+0) + (−0) = +0 under round to nearest.
                return Ok(F64 {
                    neg: self.neg && o.neg,
                    ..F64::ZERO
                });
            }
            return Ok(self);
        }
        if self.m == 0 {
            return Ok(o);
        }
        // `a` has the larger magnitude.
        let (a, b) = if (self.e + 52, self.m) >= (o.e + 52, o.m) {
            (self, o)
        } else {
            (o, self)
        };
        // 64 guard bits; anything shifted further out folds into bit 0.
        let am = u128::from(a.m) << 64;
        let bm = u128::from(b.m) << 64;
        let d = (a.e - b.e) as u32;
        let bs = if d >= 120 {
            1
        } else {
            (bm >> d) | u128::from(bm & ((1u128 << d) - 1) != 0)
        };
        let m = if a.neg == b.neg { am + bs } else { am - bs };
        if m == 0 {
            return Ok(F64::ZERO);
        }
        round(a.neg, m, a.e - 64)
    }

    pub fn sub_rne(self, o: F64) -> Result<F64, OutOfRange> {
        self.add_rne(o.negate())
    }

    /// Truncation toward zero to i32 (the conversion after
    /// rounding to double). A value outside i32 is [`OutOfRange`]: what
    /// the conversion returns then is not known.
    pub fn trunc_i32(self) -> Result<i32, OutOfRange> {
        if self.m == 0 {
            return Ok(0);
        }
        let mag: u64 = if self.e >= 0 {
            if self.e > 10 {
                return Err(OutOfRange);
            }
            self.m << self.e
        } else if self.e <= -53 {
            0
        } else {
            self.m >> (-self.e)
        };
        let v = if self.neg {
            -(mag as i128)
        } else {
            mag as i128
        };
        i32::try_from(v).map_err(|_| OutOfRange)
    }
}

/// Rounds `±m × 2^e` to 53 bits, ties to even. Bit 0 of `m` may carry a
/// sticky bit when `m` has more than 54 significant bits.
fn round(neg: bool, m: u128, e: i32) -> Result<F64, OutOfRange> {
    if m == 0 {
        return Ok(F64 { neg, ..F64::ZERO });
    }
    let bits = 128 - m.leading_zeros() as i32;
    let (mut keep, mut e) = if bits > 53 {
        let shift = (bits - 53) as u32;
        let keep = m >> shift;
        let rem = m & ((1u128 << shift) - 1);
        let half = 1u128 << (shift - 1);
        let up = rem > half || (rem == half && keep & 1 == 1);
        (keep + u128::from(up), e + shift as i32)
    } else {
        let shift = (53 - bits) as u32;
        (m << shift, e - shift as i32)
    };
    if keep == 1 << 53 {
        keep >>= 1;
        e += 1;
    }
    let top = e + 52;
    if !(MIN_EXP..=MAX_EXP).contains(&top) {
        return Err(OutOfRange);
    }
    Ok(F64 {
        neg,
        m: keep as u64,
        e,
    })
}

#[cfg(test)]
mod tests {
    //! The host's IEEE binary64 is the oracle here; tests are not game
    //! logic (hard step 6 governs the sim's code paths).
    #![allow(clippy::float_arithmetic)]
    use super::*;

    fn f(v: i64) -> F64 {
        F64::from_i64(v).unwrap()
    }

    fn host(x: F64) -> f64 {
        f64::from_bits(x.to_bits())
    }

    /// A small deterministic generator for operands (not the game RNG).
    fn values() -> Vec<i64> {
        let mut v: Vec<i64> = (-40..=40).collect();
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..200 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            v.push((x as i64) >> (x % 40));
        }
        v.extend([
            i64::from(i32::MAX),
            i64::from(i32::MIN),
            1 << 53,
            -(1 << 53),
        ]);
        v
    }

    #[test]
    fn integers_are_exact() {
        for v in values() {
            assert_eq!(host(f(v)), v as f64, "{v}");
        }
    }

    #[test]
    fn operations_match_ieee() {
        let vs = values();
        let mut ops = 0;
        for &a in &vs {
            for &b in &vs {
                // Quotients and products of quotients exercise rounding.
                let (x, y) = (f(a), f(b));
                let (hx, hy) = (a as f64, b as f64);
                if b != 0 {
                    let q = x.div_rne(y).unwrap();
                    assert_eq!(host(q), hx / hy, "{a} / {b}");
                    let p = q.mul_rne(q).unwrap();
                    assert_eq!(host(p), (hx / hy) * (hx / hy), "({a}/{b})^2");
                    let s = F64::ONE.sub_rne(q).unwrap();
                    assert_eq!(host(s), 1.0 - hx / hy, "1 - {a}/{b}");
                    let t = q.add_rne(p).unwrap();
                    assert_eq!(host(t), hx / hy + (hx / hy) * (hx / hy));
                    ops += 4;
                }
                assert_eq!(host(x.mul_rne(y).unwrap()), hx * hy, "{a} * {b}");
                assert_eq!(host(x.add_rne(y).unwrap()), hx + hy, "{a} + {b}");
                assert_eq!(host(x.sub_rne(y).unwrap()), hx - hy, "{a} - {b}");
            }
        }
        assert!(ops > 10_000);
    }

    #[test]
    fn truncation() {
        let third = f(7).div_rne(f(2)).unwrap();
        assert_eq!(third.trunc_i32(), Ok(3));
        assert_eq!(third.negate().trunc_i32(), Ok(-3));
        assert_eq!(f(1).div_rne(f(3)).unwrap().trunc_i32(), Ok(0));
        assert_eq!(f(i64::from(i32::MIN)).trunc_i32(), Ok(i32::MIN));
        assert_eq!(f(1 << 31).trunc_i32(), Err(OutOfRange));
        assert_eq!(f(1).div_rne(F64::ZERO), Err(OutOfRange));
    }

    #[test]
    fn oracle_catches_a_perturbed_rounding() {
        // Flipping the last bit of a correctly rounded result must
        // fail the comparison the tests above rely on.
        let q = f(1).div_rne(f(3)).unwrap();
        let bad = F64 { m: q.m ^ 1, ..q };
        assert_eq!(host(q), 1.0 / 3.0);
        assert_ne!(host(bad), 1.0 / 3.0);
    }
}
