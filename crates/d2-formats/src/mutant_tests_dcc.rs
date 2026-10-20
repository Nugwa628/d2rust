//! Rule-by-rule tests of the DCC decoder: header limits, direction offsets,
//! optional bytes, the direction box, and both pixel decode stages (masks, raw
//! and displaced codes, fills, equal-cell copies and clears).
//!
//! Each test is aimed so that changing any single branch or constant of the
//! decoder (as `cargo mutants` does) makes a test fail. The tests build small DCC
//! files bit by bit (LSB-first, as the format stores them) and compare what
//! [`Dcc::parse`] decodes against a model painted by hand. Test-only module of
//! d2-formats.

use crate::dcc::Dcc;

/// Field widths of the 4-bit width codes (the 4-bit code indexes this table).
/// Kept apart from the decoder's own copy on purpose, so a change to that
/// table fails these tests.
const ENCODED_BITS: [u32; 16] = [0, 1, 2, 4, 6, 8, 10, 12, 14, 16, 20, 24, 26, 28, 30, 32];

/// An LSB-first bit sequence.
#[derive(Default)]
struct Bits(Vec<bool>);

impl Bits {
    fn put(&mut self, value: u32, n: u32) {
        for i in 0..n {
            self.0.push(value >> i & 1 == 1);
        }
    }

    fn put_all(&mut self, values: &[u32], n: u32) {
        for &v in values {
            self.put(v, n);
        }
    }

    fn append(&mut self, other: &Bits) {
        self.0.extend_from_slice(&other.0);
    }

    fn align(&mut self) {
        while !self.0.len().is_multiple_of(8) {
            self.0.push(false);
        }
    }

    fn len(&self) -> u32 {
        self.0.len() as u32
    }

    fn bytes(&self) -> Vec<u8> {
        self.0
            .chunks(8)
            .map(|c| {
                c.iter()
                    .enumerate()
                    .fold(0u8, |a, (i, &b)| a | (u8::from(b) << i))
            })
            .collect()
    }
}

struct Frame {
    w: u32,
    h: u32,
    x: i32,
    y: i32,
    optional: Vec<u8>,
}

/// A bottom-up frame (`y_min = y offset`) without optional bytes.
fn frame(w: u32, h: u32, x: i32, y: i32) -> Frame {
    Frame {
        w,
        h,
        x,
        y,
        optional: Vec::new(),
    }
}

/// The sub-streams of a direction: equal cells, pixel mask, encoding type, raw
/// pixel, and pixel code and displacement.
#[derive(Default)]
struct Streams {
    equal: Bits,
    mask: Bits,
    encoding: Bits,
    raw: Bits,
    pcd: Bits,
}

/// One direction: width, height, x and y as 32-bit fields, variable0 and
/// coded bytes 0 bits wide, optional bytes `optional_code` wide, and every
/// palette index present, so pixel code `c` is palette index `c`.
fn direction(flags: u32, optional_code: u32, frames: &[Frame], s: &Streams) -> Vec<u8> {
    let mut w = Bits::default();
    w.put(0, 32); // outsize coded
    w.put(flags, 2);
    for code in [0, 15, 15, 15, 15, optional_code, 0] {
        w.put(code, 4);
    }
    for f in frames {
        w.put(f.w, 32);
        w.put(f.h, 32);
        w.put(f.x as u32, 32);
        w.put(f.y as u32, 32);
        w.put(
            f.optional.len() as u32,
            ENCODED_BITS[optional_code as usize],
        );
        w.put(1, 1); // bottom-up
    }
    if frames.iter().any(|f| !f.optional.is_empty()) {
        w.align();
        for f in frames {
            for &b in &f.optional {
                w.put(u32::from(b), 8);
            }
        }
    }
    if flags & 2 != 0 {
        w.put(s.equal.len(), 20);
    } else {
        assert!(s.equal.0.is_empty());
    }
    w.put(s.mask.len(), 20);
    if flags & 1 != 0 {
        w.put(s.encoding.len(), 20);
        w.put(s.raw.len(), 20);
    } else {
        assert!(s.encoding.0.is_empty() && s.raw.0.is_empty());
    }
    w.put_all(&[u32::MAX; 8], 32); // pixel values: all 256 present
    for stream in [&s.equal, &s.mask, &s.encoding, &s.raw, &s.pcd] {
        w.append(stream);
    }
    w.bytes()
}

/// A DCC file of `dirs`, each `frames_per_direction` frames, directly after
/// the header and offset table.
fn dcc_file(frames_per_direction: u32, dirs: &[Vec<u8>]) -> Vec<u8> {
    let mut file = vec![0x74, 6, dirs.len() as u8];
    file.extend_from_slice(&frames_per_direction.to_le_bytes());
    file.extend_from_slice(&1u32.to_le_bytes()); // tag
    file.extend_from_slice(&0u32.to_le_bytes()); // final DC6 size
    let mut at = file.len() + 4 * dirs.len();
    for d in dirs {
        file.extend_from_slice(&(at as u32).to_le_bytes());
        at += d.len();
    }
    for d in dirs {
        file.extend_from_slice(d);
    }
    file
}

/// Writes a `w × h` cell's pixel indices, row-major, `bits` each (the pixel-code
/// stream of a cell).
fn put_pixels(pcd: &mut Bits, bits: u32, w: usize, h: usize, idx: impl Fn(usize, usize) -> u32) {
    for y in 0..h {
        for x in 0..w {
            pcd.put(idx(x, y), bits);
        }
    }
}

/// Model of a cell drawn into an image of width `stride`: pixel `(x, y)`
/// of the cell is `value(x, y)`.
fn paint(
    img: &mut [u8],
    stride: usize,
    (x0, y0, w, h): (usize, usize, usize, usize),
    value: impl Fn(usize, usize) -> u8,
) {
    for y in 0..h {
        for x in 0..w {
            img[(y0 + y) * stride + x0 + x] = value(x, y);
        }
    }
}

/// The `w × h` rectangle at `(x0, y0)` of an image of width `stride`.
fn crop(img: &[u8], stride: usize, (x0, y0, w, h): (usize, usize, usize, usize)) -> Vec<u8> {
    (0..h)
        .flat_map(|y| img[(y0 + y) * stride + x0..(y0 + y) * stride + x0 + w].to_vec())
        .collect()
}

// ---------------------------------------------------------------- header

#[test]
fn frames_per_direction_limit_is_inclusive() {
    // The frames-per-direction count must be at most 256. With no directions there is nothing
    // else to decode.
    let ok = Dcc::parse(&dcc_file(256, &[])).unwrap();
    assert_eq!(ok.frames_per_direction, 256);
    assert!(Dcc::parse(&dcc_file(257, &[])).is_err());
}

#[test]
fn direction_offset_may_skip_bytes_after_the_header() {
    // A direction runs from its (absolute) offset; bytes
    // between the offset table and the first direction are not read.
    let mut s = Streams::default();
    s.pcd.put_all(&[0], 4); // first code repeats 0: v = [0, 0, 0, 0], a fill
    let dir = direction(0, 0, &[frame(1, 1, 0, 0)], &s);
    let plain = dcc_file(1, std::slice::from_ref(&dir));
    let mut gapped = plain[..15].to_vec();
    gapped.extend_from_slice(&(19u32 + 3).to_le_bytes());
    gapped.extend_from_slice(&[0xEE; 3]);
    gapped.extend_from_slice(&dir);
    assert_eq!(Dcc::parse(&gapped).unwrap(), Dcc::parse(&plain).unwrap());
    // An offset inside the header is out of order.
    let mut inside = plain.clone();
    inside[15..19].copy_from_slice(&18u32.to_le_bytes());
    assert!(Dcc::parse(&inside).is_err());
}

#[test]
fn frame_of_zero_width_or_height_is_an_error() {
    // A frame width or height of 0 is an error, each on its own.
    for (w, h) in [(0, 1), (1, 0)] {
        let dir = direction(0, 0, &[frame(w, h, 0, 0)], &Streams::default());
        assert!(Dcc::parse(&dcc_file(1, &[dir])).is_err(), "{w}x{h}");
    }
}

// ---------------------------------------------------------------- optional bytes

#[test]
fn optional_bytes_start_at_the_next_byte_boundary() {
    // With optional bytes, skip to the next byte
    // boundary, read them, and continue right after them. The header here
    // ends at bit 199 (not a boundary): the bytes are bytes 25 and 26.
    let mut f = frame(4, 4, 0, 0);
    f.optional = vec![0xAB, 0xCD];
    let mut s = Streams::default();
    s.pcd.put_all(&[1, 1, 1, 1], 4); // v = [4, 3, 2, 1]
    let idx = |x: usize, y: usize| ((x + 2 * y) % 4) as u32;
    put_pixels(&mut s.pcd, 2, 4, 4, idx);
    let dir = direction(0, 5, &[f], &s); // code 5: 8-bit optional size
    let dcc = Dcc::parse(&dcc_file(1, &[dir])).unwrap();
    let out = &dcc.directions[0].frames[0];
    assert_eq!(out.optional_data, [0xAB, 0xCD]);
    let v = [4u8, 3, 2, 1];
    let mut expected = vec![0u8; 16];
    paint(&mut expected, 4, (0, 0, 4, 4), |x, y| v[idx(x, y) as usize]);
    assert_eq!(out.pixels, expected);
}

// ---------------------------------------------------------------- boxes

#[test]
fn direction_box_offset_from_origin() {
    // Frames at x 5 and y 4 / 8 (bottom-up): the direction box is
    // x 5..=8, y 4..=11; frame positions are relative to it, so frame 1 is
    // the lower direction cell (fy = 4). No equal-cells stream: revisiting
    // a cell reads its mask directly (first-touch cells).
    let frames = [frame(4, 4, 5, 4), frame(4, 4, 5, 8), frame(4, 4, 5, 4)];
    let mut s = Streams::default();
    // Stage 1: frames 0 and 1 are first touches (mask 0xF): codes 1..4,
    // v = [4, 3, 2, 1]. Frame 2 revisits cell 0 with mask 0: v = previous.
    s.pcd.put_all(&[1, 1, 1, 1, 1, 1, 1, 1], 4);
    s.mask.put(0, 4);
    let pats: [fn(usize, usize) -> u32; 3] = [
        |x, y| ((x + 2 * y) % 4) as u32,
        |x, y| ((3 * x + y) % 4) as u32,
        |x, y| ((x * y + 1) % 4) as u32,
    ];
    for p in pats {
        put_pixels(&mut s.pcd, 2, 4, 4, p);
    }
    let dcc = Dcc::parse(&dcc_file(3, &[direction(0, 0, &frames, &s)])).unwrap();
    let d = &dcc.directions[0];
    assert_eq!((d.x_min, d.y_min, d.width, d.height), (5, 4, 4, 8));
    let v = [4u8, 3, 2, 1];
    for (f, p) in pats.iter().enumerate() {
        let out = &d.frames[f];
        assert_eq!((out.x_min, out.y_min), (frames[f].x, frames[f].y));
        let mut expected = vec![0u8; 16];
        paint(&mut expected, 4, (0, 0, 4, 4), |x, y| v[p(x, y) as usize]);
        assert_eq!(out.pixels, expected, "frame {f}");
    }
}

// ---------------------------------------------------------------- stages 1 and 2

#[test]
fn cells_masks_raw_codes_fills_and_equal_copies() {
    // Three 8×8 frames at (0, 0): frame cells (0,0) (4,0) (0,4) (4,4) are
    // direction cells 0..3. Equal-cells and raw-pixel streams present.
    let frames = [frame(8, 8, 0, 0), frame(8, 8, 0, 0), frame(8, 8, 0, 0)];
    let rects = [(0, 0, 4, 4), (4, 0, 4, 4), (0, 4, 4, 4), (4, 4, 4, 4)];
    let mut s = Streams::default();
    // Frame 0 (first touches, mask 0xF, one encoding bit each):
    //   cell 0: PCD 3, 0       -> v = [3, 0, 0, 0]   (1 bit per pixel)
    //   cell 1: raw 5, 7, 7    -> v = [7, 5, 0, 0]   (2 bits)
    //   cell 2: PCD 1, 1, 1, 1 -> v = [4, 3, 2, 1]   (2 bits)
    //   cell 3: PCD 15, 2, 0   -> v = [17, 0, 0, 0]  (1 bit)
    // Frame 1:
    //   cell 0: not equal, mask 0b0110, raw 9, 4 -> v = [3, 4, 9, 0] (2 bits)
    //   cell 1: not equal, mask 0b0001, PCD 5    -> v = [5, 5, 0, 0] (fill)
    //   cell 2: not equal, mask 0 (no encoding bit) -> v = [4, 3, 2, 1]
    //   cell 3: equal (copies its own last rectangle)
    // Frame 2: every cell equal.
    s.encoding.put_all(&[0, 1, 0, 0, 1, 0], 1);
    s.raw.put_all(&[5, 7, 7, 9, 4], 8);
    s.pcd.put_all(&[3, 0, 1, 1, 1, 1, 15, 2, 0, 5], 4);
    s.equal.put_all(&[0, 0, 0, 1, 1, 1, 1, 1], 1);
    s.mask.put_all(&[0b0110, 0b0001, 0], 4);
    let p00 = |x: usize, y: usize| ((x + y) % 2) as u32;
    let p01 = |x: usize, y: usize| ((x + y) % 3) as u32;
    let p02 = |x: usize, y: usize| ((x + 2 * y) % 4) as u32;
    let p03 = |x: usize, y: usize| ((x * y) % 2) as u32;
    let p10 = |x: usize, y: usize| ((3 * x + y) % 4) as u32;
    let p12 = |x: usize, y: usize| ((x * y + 1) % 4) as u32;
    put_pixels(&mut s.pcd, 1, 4, 4, p00);
    put_pixels(&mut s.pcd, 2, 4, 4, p01);
    put_pixels(&mut s.pcd, 2, 4, 4, p02);
    put_pixels(&mut s.pcd, 1, 4, 4, p03);
    put_pixels(&mut s.pcd, 2, 4, 4, p10);
    put_pixels(&mut s.pcd, 2, 4, 4, p12);
    let dcc = Dcc::parse(&dcc_file(3, &[direction(3, 0, &frames, &s)])).unwrap();
    let d = &dcc.directions[0];
    assert_eq!((d.width, d.height), (8, 8));

    let mut img = vec![0u8; 64];
    let v = |v: [u8; 4], p: fn(usize, usize) -> u32| move |x, y| v[p(x, y) as usize];
    paint(&mut img, 8, rects[0], v([3, 0, 0, 0], p00));
    paint(&mut img, 8, rects[1], v([7, 5, 0, 0], p01));
    paint(&mut img, 8, rects[2], v([4, 3, 2, 1], p02));
    paint(&mut img, 8, rects[3], v([17, 0, 0, 0], p03));
    assert_eq!(d.frames[0].pixels, img, "frame 0");
    paint(&mut img, 8, rects[0], v([3, 4, 9, 0], p10));
    paint(&mut img, 8, rects[1], |_, _| 5);
    paint(&mut img, 8, rects[2], v([4, 3, 2, 1], p12));
    assert_eq!(d.frames[1].pixels, img, "frame 1");
    assert_eq!(d.frames[2].pixels, img, "frame 2");
}

#[test]
fn equal_cells_copy_moved_rectangles_or_clear() {
    // Equal-cells stream only. All frames lie in direction cell 0 except the
    // 8×8 ones; boxes (x, y, w, h):
    //   F0 (0,0,8,8): first touches, v = [4,3,2,1] in every cell (2 bits)
    //   F1 (1,1,3,3): not equal, mask 0 -> v = [4,3,2,1], 9 new pixels
    //   F2 (2,2,3,3): equal, same size as F1's rectangle -> copy it here
    //   F3 (2,3,3,1): equal, same width, other height -> clear R in B
    //   F4 (2,3,3,1): equal, same size -> copy (the cleared row)
    //   F5 (0,0,8,8): equal everywhere: cell 0 clears (size differs), the
    //                 others copy their F0 rectangles from B.
    let frames = [
        frame(8, 8, 0, 0),
        frame(3, 3, 1, 1),
        frame(3, 3, 2, 2),
        frame(3, 1, 2, 3),
        frame(3, 1, 2, 3),
        frame(8, 8, 0, 0),
    ];
    let rects = [(0, 0, 4, 4), (4, 0, 4, 4), (0, 4, 4, 4), (4, 4, 4, 4)];
    let pats: [fn(usize, usize) -> u32; 4] = [
        |x, y| ((x + 2 * y) % 4) as u32,
        |x, y| ((2 * x + y + 1) % 4) as u32,
        |x, y| ((x + 3 * y + 2) % 4) as u32,
        |x, y| ((3 * x + y) % 4) as u32,
    ];
    let pa = |x: usize, y: usize| ((x * y + x) % 4) as u32;
    let v = [4u8, 3, 2, 1];
    let mut s = Streams::default();
    s.pcd.put_all(&[1; 16], 4);
    s.equal.put_all(&[0, 1, 1, 1, 1, 1, 1, 1], 1);
    s.mask.put(0, 4);
    for p in pats {
        put_pixels(&mut s.pcd, 2, 4, 4, p);
    }
    put_pixels(&mut s.pcd, 2, 3, 3, pa);
    let dcc = Dcc::parse(&dcc_file(6, &[direction(2, 0, &frames, &s)])).unwrap();
    let out: Vec<&[u8]> = dcc.directions[0]
        .frames
        .iter()
        .map(|f| f.pixels.as_slice())
        .collect();

    // Model of the persistent buffer B.
    let mut b = vec![0u8; 64];
    for (r, p) in rects.iter().zip(pats) {
        paint(&mut b, 8, *r, |x, y| v[p(x, y) as usize]);
    }
    assert_eq!(out[0], b);
    paint(&mut b, 8, (1, 1, 3, 3), |x, y| v[pa(x, y) as usize]);
    let a = crop(&b, 8, (1, 1, 3, 3));
    assert_eq!(out[1], a);
    paint(&mut b, 8, (2, 2, 3, 3), |x, y| a[y * 3 + x]);
    assert_eq!(out[2], a);
    paint(&mut b, 8, (2, 3, 3, 1), |_, _| 0);
    assert_eq!(out[3], [0, 0, 0]);
    assert_eq!(out[4], [0, 0, 0]);
    paint(&mut b, 8, rects[0], |_, _| 0);
    assert_eq!(out[5], b);
}
