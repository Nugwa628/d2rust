//! Tests for the level linkers, one call at a time: the first call's draws on the
//! driver's seed copy, each retry's alternative, the "false" that ends them, and the
//! placement each one makes. Place A/B/C are tested on their own.

use super::super::tests::data;
use super::*;

/// The parent rect every child row links to (row 0).
const PARENT: TileRect = TileRect {
    x: 400,
    y: 300,
    w: 80,
    h: 64,
};
/// Child size before the linker runs.
const SIZE: (i32, i32) = (40, 48);
/// Leveldefs offset of the child level (Def, BD).
const OFFSET: (i32, i32) = (1234, -56);
const CHILD: u32 = 5;

fn setup(linker: Linker, seed: u32) -> (Driver, [LinkRow; 2], DrlgData) {
    let mut data = data();
    data.levels[CHILD as usize].offset = OFFSET;
    let mut d = Driver::new(Seed::init_low(seed));
    d.c[0] = PARENT;
    (d.c[1].w, d.c[1].h) = SIZE;
    let rows = [
        LinkRow {
            linker: Def,
            level: 4,
            link: -1,
        },
        LinkRow {
            linker,
            level: CHILD,
            link: 0,
        },
    ];
    (d, rows, data)
}

fn call(d: &mut Driver, rows: &[LinkRow], data: &DrlgData) -> bool {
    run_linker(d, rows, 1, data).unwrap()
}

fn sized(w: i32, h: i32) -> TileRect {
    TileRect { x: 0, y: 0, w, h }
}

fn a(case: i32, v: i32, c: TileRect) -> TileRect {
    let mut c = c;
    place_a(PARENT, &mut c, case, v);
    c
}

fn b(case: i32, v: i32, c: TileRect) -> TileRect {
    let mut c = c;
    place_b(PARENT, &mut c, case, v);
    c
}

fn cc(case: i32, v: i32, c: TileRect) -> TileRect {
    let mut c = c;
    place_c(PARENT, &mut c, case, v);
    c
}

/// R4 / R8 / VS: first R1 := R0 := lo' & (n − 1); each retry R0 + 1 mod n
/// until it would reach R1 again (false), so n alternatives in all.
fn rotating(linker: Linker, n: i32, place: fn(i32, TileRect) -> TileRect) {
    let mut seen = vec![false; n as usize];
    for seed in 1..=64 {
        let (mut d, rows, data) = setup(linker, seed);
        let mut s = d.seed;
        let first = (s.step() & (n as u32 - 1)) as i32;
        seen[first as usize] = true;
        let mut tried = Vec::new();
        while call(&mut d, &rows, &data) {
            assert_eq!(d.seed, s, "{linker:?}: one draw on the first call only");
            assert_eq!(d.r[1][1], first);
            tried.push(d.r[0][1]);
            assert_eq!(d.c[1], place(d.r[0][1], sized(SIZE.0, SIZE.1)));
        }
        let expect: Vec<i32> = (0..n).map(|k| (first + k) % n).collect();
        assert_eq!(tried, expect, "{linker:?} seed {seed}");
    }
    assert!(
        seen.iter().all(|s| *s),
        "{linker:?}: every first value drawn"
    );
}

/// BM / RE: first R1 := R0 := lo' & 3, then R3 := lo' & 1, t := R3; a
/// retry t := (R2 + 1) mod 2, r := (R2 + R0) mod 4, false when r = R1 and
/// t = R3, else R0 := r; then R2 := t and the placement.
fn turning(linker: Linker, v: i32) {
    let mut seen = [[false; 2]; 4];
    for seed in 1..=64 {
        let (mut d, rows, data) = setup(linker, seed);
        let mut s = d.seed;
        let (r1, r3) = ((s.step() & 3) as i32, (s.step() & 1) as i32);
        seen[r1 as usize][r3 as usize] = true;
        let (mut r0, mut r2) = (r1, r3);
        let mut calls = 0;
        loop {
            let ok = call(&mut d, &rows, &data);
            assert_eq!(d.seed, s, "{linker:?}: two draws on the first call only");
            if calls > 0 {
                let t = (r2 + 1) % 2;
                let r = (r2 + r0) % 4;
                if r == r1 && t == r3 {
                    assert!(!ok, "{linker:?} seed {seed} call {calls}");
                    break;
                }
                r0 = r;
                r2 = t;
            }
            assert!(ok);
            calls += 1;
            assert_eq!(
                (d.r[0][1], d.r[1][1], d.r[2][1], d.r[3][1]),
                (r0, r1, r2, r3)
            );
            let size = match linker {
                Bm if r0 % 2 == 1 => sized(96, 56),
                Bm => sized(56, 96),
                _ => sized(SIZE.0, SIZE.1),
            };
            let want = if r2 == 1 {
                a(r0, v, size)
            } else {
                b(r0, v, size)
            };
            assert_eq!(d.c[1], want, "{linker:?} seed {seed} call {calls}");
            assert!(calls < 16, "{linker:?}: retries end");
        }
    }
    assert_eq!(seen, [[true; 2]; 4]);
}

#[test]
fn linkers_draw_retry_and_place() {
    // Def: no draw, R0 := −1, the leveldefs offset; a retry is the same.
    let (mut d, rows, data) = setup(Def, 7);
    let s = d.seed;
    for _ in 0..3 {
        assert!(call(&mut d, &rows, &data));
        assert_eq!((d.seed, d.r[0][1]), (s, -1));
        assert_eq!((d.c[1].x, d.c[1].y), OFFSET);
        assert_eq!((d.c[1].w, d.c[1].h), SIZE);
    }

    rotating(R4, 4, |r, c| a(r, 1, c));
    rotating(R8, 8, |r, c| cc(r, 1, c));
    rotating(Vs, 8, |r, c| cc(r, 0, c));
    turning(Bm, 1);
    turning(Re, 2);

    // Fix: no draw, R1 := R0 := 0, Place A(0, 0); always true.
    let (mut d, rows, data) = setup(Fix, 7);
    let s = d.seed;
    for _ in 0..3 {
        assert!(call(&mut d, &rows, &data));
        assert_eq!((d.seed, d.r[0][1], d.r[1][1]), (s, 0, 0));
        assert_eq!(d.c[1], a(0, 0, sized(SIZE.0, SIZE.1)));
    }

    // RW: R1 := R0 := (lo' & 1) + 1; one retry to the other value, then
    // false. R0 = 1: Place A(1, 0), else Place B(R0, 0).
    let mut seen = [false; 2];
    for seed in 1..=16 {
        let (mut d, rows, data) = setup(Rw, seed);
        let mut s = d.seed;
        let first = (s.step() & 1) as i32 + 1;
        seen[first as usize - 1] = true;
        let mut tried = Vec::new();
        while call(&mut d, &rows, &data) {
            assert_eq!(d.seed, s);
            let r0 = d.r[0][1];
            tried.push(r0);
            let c = sized(SIZE.0, SIZE.1);
            assert_eq!(d.c[1], if r0 == 1 { a(1, 0, c) } else { b(r0, 0, c) });
        }
        assert_eq!(tried, [first, 3 - first]);
    }
    assert_eq!(seen, [true; 2]);

    // OS: R1 := R0 := 3; each call (retries too) draws lo' & 1: 0 → Place
    // B(3, 3) and transition 0x400000, else Place A(3, 3) and 0x800000.
    let mut seen = [false; 2];
    let (mut d, rows, data) = setup(Os, 3);
    let mut s = d.seed;
    for _ in 0..12 {
        let bit = s.step() & 1;
        seen[bit as usize] = true;
        assert!(call(&mut d, &rows, &data));
        assert_eq!((d.seed, d.r[0][1], d.r[1][1]), (s, 3, 3));
        let c = sized(SIZE.0, SIZE.1);
        let (want, flag) = if bit == 0 {
            (b(3, 3, c), 0x40_0000)
        } else {
            (a(3, 3, c), 0x80_0000)
        };
        assert_eq!((d.c[1], d.transition), (want, flag));
    }
    assert_eq!(seen, [true; 2]);

    // B1 / BD: each call draws R0 := R1 := lo' & 1; (64, 160) if 0, else
    // (160, 64). B1: x := parent.x − w, y := parent.y + parent.h − h − 16;
    // BD: the leveldefs offset.
    for linker in [B1, Bd] {
        let mut seen = [false; 2];
        let (mut d, rows, data) = setup(linker, 11);
        let mut s = d.seed;
        for _ in 0..12 {
            let v = (s.step() & 1) as i32;
            seen[v as usize] = true;
            assert!(call(&mut d, &rows, &data));
            assert_eq!((d.seed, d.r[0][1], d.r[1][1]), (s, v, v));
            let (w, h) = if v == 0 { (64, 160) } else { (160, 64) };
            let (x, y) = if linker == B1 {
                (PARENT.x - w, PARENT.y + PARENT.h - h - 16)
            } else {
                OFFSET
            };
            assert_eq!(d.c[1], TileRect { x, y, w, h }, "{linker:?}");
        }
        assert_eq!(seen, [true; 2], "{linker:?}");
    }

    // B2: R1 := R0 := lo' & 1; one retry to the other value, then false.
    // Size as B1 from R0; x, y := parent + offset[R0 + 2·R0[link]].
    let offsets = [(0, -160), (-96, -64), (-64, -96), (-160, 0)];
    for link_r0 in [0, 1] {
        let mut seen = [false; 2];
        for seed in 1..=16 {
            let (mut d, rows, data) = setup(B2, seed);
            d.r[0][0] = link_r0;
            let mut s = d.seed;
            let first = (s.step() & 1) as i32;
            seen[first as usize] = true;
            let mut tried = Vec::new();
            while call(&mut d, &rows, &data) {
                assert_eq!((d.seed, d.r[1][1]), (s, first));
                let r0 = d.r[0][1];
                tried.push(r0);
                let (w, h) = if r0 == 0 { (64, 160) } else { (160, 64) };
                let (ox, oy) = offsets[(r0 + 2 * link_r0) as usize];
                let want = TileRect {
                    x: PARENT.x + ox,
                    y: PARENT.y + oy,
                    w,
                    h,
                };
                assert_eq!(d.c[1], want);
            }
            assert_eq!(tried, [first, 1 - first]);
        }
        assert_eq!(seen, [true; 2]);
    }
}
