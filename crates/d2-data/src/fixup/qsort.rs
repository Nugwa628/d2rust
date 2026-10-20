//! The C runtime `qsort` (Visual Studio 2005 era) reproduced for the
//! fix-ups that sort records (`maps` uses it for the itemstatcost
//! description list and the gamble index). It is not stable, so equal keys
//! come out in exactly this algorithm's order; a stable sort would change
//! the result for ties.

use std::cmp::Ordering;

/// Ranges of this many elements or fewer are sorted by the short sort.
const SHORT: isize = 8;

/// Sorts `v` with the C runtime algorithm; `cmp` sees elements, never positions.
pub fn qsort<T>(v: &mut [T], mut cmp: impl FnMut(&T, &T) -> Ordering) {
    if v.len() < 2 {
        return;
    }
    let mut c = |v: &[T], a: isize, b: isize| cmp(&v[a as usize], &v[b as usize]);
    let swap = |v: &mut [T], a: isize, b: isize| v.swap(a as usize, b as usize);
    let mut stack: Vec<(isize, isize)> = Vec::new();
    let (mut lo, mut hi) = (0isize, v.len() as isize - 1);
    loop {
        let size = hi - lo + 1;
        if size <= SHORT {
            // Short sort: move the first maximum to the end, repeatedly.
            let mut h = hi;
            while h > lo {
                let mut m = lo;
                for p in lo + 1..=h {
                    if c(v, p, m) == Ordering::Greater {
                        m = p;
                    }
                }
                swap(v, m, h);
                h -= 1;
            }
        } else {
            let mut mid = lo + size / 2;
            if c(v, lo, mid) == Ordering::Greater {
                swap(v, lo, mid);
            }
            if c(v, lo, hi) == Ordering::Greater {
                swap(v, lo, hi);
            }
            if c(v, mid, hi) == Ordering::Greater {
                swap(v, mid, hi);
            }
            let (mut l, mut h) = (lo, hi);
            loop {
                if mid > l {
                    loop {
                        l += 1;
                        if !(l < mid && c(v, l, mid) != Ordering::Greater) {
                            break;
                        }
                    }
                }
                if mid <= l {
                    loop {
                        l += 1;
                        if !(l <= hi && c(v, l, mid) != Ordering::Greater) {
                            break;
                        }
                    }
                }
                loop {
                    h -= 1;
                    if !(h > mid && c(v, h, mid) == Ordering::Greater) {
                        break;
                    }
                }
                if h < l {
                    break;
                }
                swap(v, l, h);
                if mid == h {
                    mid = l;
                }
            }
            h += 1;
            if mid < h {
                loop {
                    h -= 1;
                    if !(h > mid && c(v, h, mid) == Ordering::Equal) {
                        break;
                    }
                }
            }
            if mid >= h {
                loop {
                    h -= 1;
                    if !(h > lo && c(v, h, mid) == Ordering::Equal) {
                        break;
                    }
                }
            }
            if h - lo >= hi - l {
                if lo < h {
                    stack.push((lo, h));
                }
                if l < hi {
                    lo = l;
                    continue;
                }
            } else {
                if l < hi {
                    stack.push((l, hi));
                }
                if lo < h {
                    hi = h;
                    continue;
                }
            }
        }
        match stack.pop() {
            Some((a, b)) => (lo, hi) = (a, b),
            None => return,
        }
    }
}
