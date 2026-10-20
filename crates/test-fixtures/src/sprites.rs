//! Live-shaped synthetic sprites for the format benches
//! (`d2-formats/benches/formats.rs`): many
//! directions and frames at realistic sizes, a moving silhouette on a
//! transparent ground, per-pixel colours. Built from the formats' layouts
//! only; no value comes from a Blizzard file.
//!
//! - [`dc6_frames`] + [`dc6_file`]: images, then the DC6 encoding
//!   (bottom row first, skip / literal runs, row ends).
//! - [`dcc_file`]: a DCC whose directions use every sub-stream of
//!   the DCC format (equal cells, pixel masks, raw and displacement-coded cell
//!   colours, 1- and 2-bit pixel indices, partial masks keeping earlier
//!   colours), and the frames the decoder must produce. The writer runs
//!   the decoder's stage 2 on its own choices to know those frames.

/// Deterministic xorshift32 (fixture shapes only).
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// `0..n` (`n > 0`).
    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }

    /// True with probability `pct` %.
    pub fn chance(&mut self, pct: u32) -> bool {
        self.below(100) < pct
    }

    /// `base + (-spread..=spread)`.
    fn jitter(&mut self, base: i32, spread: u32) -> i32 {
        base + self.below(2 * spread + 1) as i32 - spread as i32
    }
}

/// A decoded frame: palette indices, top row first, 0 = transparent;
/// `x`, `y` are the DC6 offsets, or the DCC frame box's `x_min`, `y_min`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub pixels: Vec<u8>,
}

/// Inside the ellipse of radii `(rx, ry)` centred on `(cx, cy)`.
fn inside(x: i32, y: i32, (cx, cy): (i32, i32), (rx, ry): (i32, i32)) -> bool {
    let (dx, dy) = (
        i64::from(x - cx) * i64::from(ry),
        i64::from(y - cy) * i64::from(rx),
    );
    dx * dx + dy * dy <= i64::from(rx * ry).pow(2)
}

// ---- DC6 ---------------------------------------------------------------------

/// Shape of a DC6: `directions × frames` frames of about
/// `width × height`. `panel`: opaque frames of exactly that size with a
/// few transparent windows (UI panels); otherwise a silhouette that moves
/// from frame to frame, sizes jittered by a few pixels (objects,
/// overlays, inventory art).
#[derive(Debug, Clone, Copy)]
pub struct Dc6Shape {
    pub directions: u32,
    pub frames: u32,
    pub width: u32,
    pub height: u32,
    pub panel: bool,
}

/// The frames of `shape`, direction-major.
pub fn dc6_frames(shape: Dc6Shape, seed: u32) -> Vec<Image> {
    let mut rng = Rng::new(seed);
    let mut out = Vec::new();
    for _ in 0..shape.directions {
        for f in 0..shape.frames as i32 {
            let (w, h) = if shape.panel {
                (shape.width, shape.height)
            } else {
                (
                    rng.jitter(shape.width as i32, 3).max(4) as u32,
                    rng.jitter(shape.height as i32, 3).max(4) as u32,
                )
            };
            let (wi, hi) = (w as i32, h as i32);
            let centre = (wi / 2 + f % 3 - 1, hi / 2 + f % 2);
            let radii = (wi * 2 / 5 + f % 4, hi * 9 / 20 - f % 3);
            // Panels: transparent windows (a quarter of each axis).
            let window = |x: i32, y: i32| {
                (x / (wi / 4).max(1) + y / (hi / 4).max(1)) % 5 == 4
                    && x % (wi / 4).max(1) > 2
                    && y % (hi / 4).max(1) > 2
            };
            let mut pixels = vec![0u8; (w * h) as usize];
            for y in 0..hi {
                for x in 0..wi {
                    let opaque = if shape.panel {
                        !window(x, y)
                    } else {
                        inside(x, y, centre, radii)
                    };
                    if opaque {
                        // Shaded runs: neighbours share a ramp, with noise.
                        let base = 1 + ((x / 6 + y / 5 + f) % 24) as u32 * 10;
                        pixels[(y * wi + x) as usize] = (base + rng.below(8)).min(255) as u8;
                    }
                }
            }
            out.push(Image {
                width: w,
                height: h,
                x: rng.jitter(-wi / 2, 2),
                y: rng.jitter(0, 2),
                pixels,
            });
        }
    }
    out
}

/// One frame's pixel data (DC6, flip 0): bottom row
/// first; transparent runs as skips (≤ 127 each) when an opaque pixel
/// follows in the row, opaque runs as literals (≤ 127), `0x80` per row.
fn dc6_encode(img: &Image) -> Vec<u8> {
    let w = img.width as usize;
    let mut out = Vec::new();
    for row in img.pixels.chunks(w.max(1)).rev() {
        let mut x = 0;
        while x < row.len() {
            let start = x;
            if row[x] == 0 {
                while x < row.len() && row[x] == 0 {
                    x += 1;
                }
                if x == row.len() {
                    break;
                }
                let mut n = x - start;
                while n > 0 {
                    let k = n.min(127);
                    out.push(0x80 | k as u8);
                    n -= k;
                }
            } else {
                while x < row.len() && row[x] != 0 && x - start < 127 {
                    x += 1;
                }
                out.push((x - start) as u8);
                out.extend_from_slice(&row[start..x]);
            }
        }
        out.push(0x80);
    }
    out
}

/// A DC6 file of `frames`
/// (direction-major, `directions × frames_per_direction` of them).
pub fn dc6_file(frames: &[Image], directions: u32, frames_per_direction: u32) -> Vec<u8> {
    assert_eq!(frames.len(), (directions * frames_per_direction) as usize);
    let mut d = Vec::new();
    for v in [6i32, 1, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&[0xEE; 4]);
    d.extend_from_slice(&directions.to_le_bytes());
    d.extend_from_slice(&frames_per_direction.to_le_bytes());
    let mut at = d.len() + 4 * frames.len();
    let mut body = Vec::new();
    for img in frames {
        d.extend_from_slice(&(at as u32).to_le_bytes());
        let data = dc6_encode(img);
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&img.width.to_le_bytes());
        body.extend_from_slice(&img.height.to_le_bytes());
        body.extend_from_slice(&img.x.to_le_bytes());
        body.extend_from_slice(&img.y.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(&data);
        body.extend_from_slice(&[0xEE; 3]);
        at += 32 + data.len() + 3;
    }
    d.extend(body);
    d
}

// ---- DCC ---------------------------------------------------------------------

/// LSB-first bit fields.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    len: usize,
}

impl Bits {
    fn put(&mut self, value: u32, n: u32) {
        for i in 0..n {
            if self.len.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if value >> i & 1 == 1 {
                *self.bytes.last_mut().expect("byte") |= 1 << (self.len % 8);
            }
            self.len += 1;
        }
    }

    fn append(&mut self, other: &Bits) {
        for i in 0..other.len {
            self.put(u32::from(other.bytes[i / 8] >> (i % 8) & 1), 1);
        }
    }
}

/// Shape of a DCC: `directions × frames` frames of about
/// `width × height` (jittered by a few pixels per frame, as animation
/// frames are), a moving silhouette on a transparent ground.
#[derive(Debug, Clone, Copy)]
pub struct DccShape {
    pub directions: u32,
    pub frames: u32,
    pub width: u32,
    pub height: u32,
}

/// A written DCC and the frames `Dcc::parse` must decode from it
/// (`frames[direction][frame]`, `x` / `y` = the frame box's minimum).
pub struct DccOut {
    pub file: Vec<u8>,
    pub frames: Vec<Vec<Image>>,
    /// Bits written per sub-stream over all directions: equal cells,
    /// pixel mask, encoding type, raw pixels, PCD codes, PCD pixels.
    pub stream_bits: [usize; 6],
}

/// Width code 5 = 8 bits (`ENCODED_BITS`): every frame field fits.
const CODE_8: u32 = 5;

/// Palette indices in the pixel-values list (`PV`): 0, then 1..=255 except
/// multiples of 7 (PV is not the identity map).
fn pixel_values() -> Vec<u8> {
    (0..=255u8).filter(|&i| i == 0 || i % 7 != 0).collect()
}

/// DCC cells: the frame-cell sizes along one axis.
fn cell_sizes(size: i32, offset: i32) -> Vec<i32> {
    let first = 4 - offset % 4;
    if size - first <= 1 {
        return vec![size];
    }
    let rem = size - first;
    let mut v = vec![first];
    match rem % 4 {
        0 => v.extend(std::iter::repeat_n(4, (rem / 4) as usize)),
        1 => {
            v.extend(std::iter::repeat_n(4, (rem / 4 - 1) as usize));
            v.push(5);
        }
        r => {
            v.extend(std::iter::repeat_n(4, (rem / 4) as usize));
            v.push(r);
        }
    }
    v
}

/// A frame header before the direction box is known.
struct Head {
    width: i32,
    height: i32,
    x_off: i32,
    /// `y_max` (bottom-up 0).
    y_off: i32,
}

/// One direction's bit stream and decoded frames.
fn dcc_direction(shape: DccShape, rng: &mut Rng, pv: &[u8]) -> (Bits, Vec<Image>, [usize; 6]) {
    let heads: Vec<Head> = (0..shape.frames)
        .map(|_| Head {
            width: rng.jitter(shape.width as i32, 4).max(2),
            height: rng.jitter(shape.height as i32, 4).max(2),
            x_off: rng.jitter(-(shape.width as i32) / 2, 3),
            y_off: rng.jitter(0, 2),
        })
        .collect();
    let x0 = heads.iter().map(|h| h.x_off).min().expect("frames");
    let x1 = heads.iter().map(|h| h.x_off + h.width - 1).max().unwrap();
    let y0 = heads.iter().map(|h| h.y_off - h.height + 1).min().unwrap();
    let y1 = heads.iter().map(|h| h.y_off).max().unwrap();
    let (dw, dh) = (x1 - x0 + 1, y1 - y0 + 1);
    let cells_w = (dw + 3) / 4;
    let ncells = (cells_w * ((dh + 3) / 4)) as usize;

    let (mut eq, mut mask_s, mut enc, mut raw_s, mut pcd1, mut pcd2) = (
        Bits::default(),
        Bits::default(),
        Bits::default(),
        Bits::default(),
        Bits::default(),
        Bits::default(),
    );
    let ncodes = pv.len() as u32;
    let mut entry: Vec<Option<[u8; 4]>> = vec![None; ncells];
    let mut last_rect: Vec<Option<(i32, i32, i32, i32)>> = vec![None; ncells];
    let mut buf = vec![0u8; (dw * dh) as usize];
    let mut frames = Vec::new();
    for (f, h) in heads.iter().enumerate() {
        let f = f as i32;
        let (fx, fy) = (h.x_off - x0, h.y_off - h.height + 1 - y0);
        let mut out = vec![0u8; (dw * dh) as usize];
        let centre = (dw / 2 + f % 5 - 2, dh / 2 + f % 3 - 1);
        let radii = (dw * 2 / 5 + f % 4, dh * 9 / 20 - f % 3);
        let (mut y, ws, hs) = (fy, cell_sizes(h.width, fx), cell_sizes(h.height, fy));
        for (cy, &ch) in hs.iter().enumerate() {
            let mut x = fx;
            for (cx, &cw) in ws.iter().enumerate() {
                let d = ((fy / 4 + cy as i32) * cells_w + fx / 4 + cx as i32) as usize;
                let r = (x, y, cw, ch);
                let body = inside(x + cw / 2, y + ch / 2, centre, radii);
                let copy_out = |buf: &[u8], out: &mut [u8]| {
                    for yy in y..y + ch {
                        for xx in x..x + cw {
                            let i = (yy * dw + xx) as usize;
                            out[i] = buf[i];
                        }
                    }
                };
                // Stage 1: equal (unchanged ground, or a still body part).
                let mask = if let Some(prev) = entry[d] {
                    let still = prev == [0; 4] && !body || body && rng.chance(30);
                    eq.put(u32::from(still), 1);
                    if still {
                        // Stage 2, equal cell.
                        match last_rect[d] {
                            Some((sx, sy, sw, sh)) if (sw, sh) == (cw, ch) => {
                                let src: Vec<u8> = (0..ch)
                                    .flat_map(|j| (0..cw).map(move |i| (i, j)))
                                    .map(|(i, j)| buf[((sy + j) * dw + sx + i) as usize])
                                    .collect();
                                for j in 0..ch {
                                    for i in 0..cw {
                                        buf[((y + j) * dw + x + i) as usize] =
                                            src[(j * cw + i) as usize];
                                    }
                                }
                                copy_out(&buf, &mut out);
                            }
                            _ => {
                                for yy in y..y + ch {
                                    for xx in x..x + cw {
                                        buf[(yy * dw + xx) as usize] = 0;
                                    }
                                }
                            }
                        }
                        last_rect[d] = Some(r);
                        x += cw;
                        continue;
                    }
                    let m = if !body {
                        0xF
                    } else {
                        [0xF, 0xF, 0x1, 0x3, 0x7, 0x6, 0x2][rng.below(7) as usize]
                    };
                    mask_s.put(m, 4);
                    m
                } else {
                    0xF
                };
                let n = mask.count_ones();
                // Codes pushed: none for the transparent ground (all 0),
                // else mostly `n` (sometimes fewer: the rest pop 0).
                let k = if !body {
                    0
                } else if n > 1 && rng.chance(15) {
                    n - 1
                } else {
                    n
                };
                let mut codes = Vec::new();
                let mut last = 0u32;
                for _ in 0..k {
                    let mut c = 1 + rng.below(ncodes - 1);
                    if c == last {
                        c = 1 + c % (ncodes - 1);
                    }
                    codes.push(c);
                    last = c;
                }
                let raw = n > 0 && rng.chance(10);
                if n > 0 {
                    enc.put(u32::from(raw), 1);
                }
                let mut last = 0u32;
                for &c in &codes {
                    if raw {
                        raw_s.put(c, 8);
                    } else {
                        let delta = (c + 256 - last) % 256;
                        for _ in 0..delta / 15 {
                            pcd1.put(15, 4);
                        }
                        pcd1.put(delta % 15, 4);
                    }
                    last = c;
                }
                if k < n {
                    // Stop early: a code equal to the last one.
                    if raw {
                        raw_s.put(last, 8);
                    } else {
                        pcd1.put(0, 4);
                    }
                }
                let prev = entry[d].unwrap_or([0; 4]);
                let mut stack = codes;
                let mut v = [0u8; 4];
                for (i, slot) in v.iter_mut().enumerate() {
                    *slot = if mask >> i & 1 == 1 {
                        stack.pop().unwrap_or(0) as u8
                    } else {
                        prev[i]
                    };
                }
                entry[d] = Some(v);
                // Stage 2, new entry.
                if v[0] == v[1] {
                    for yy in y..y + ch {
                        for xx in x..x + cw {
                            buf[(yy * dw + xx) as usize] = v[0];
                        }
                    }
                } else {
                    let bits = if v[1] == v[2] { 1 } else { 2 };
                    for yy in y..y + ch {
                        for xx in x..x + cw {
                            let i = if rng.chance(60) {
                                ((xx + yy + f) as u32) % (1 << bits)
                            } else {
                                rng.below(1 << bits)
                            };
                            pcd2.put(i, bits);
                            buf[(yy * dw + xx) as usize] = v[i as usize];
                        }
                    }
                }
                copy_out(&buf, &mut out);
                last_rect[d] = Some(r);
                x += cw;
            }
            y += ch;
        }
        let mut pixels = Vec::with_capacity((h.width * h.height) as usize);
        for yy in fy..fy + h.height {
            for xx in fx..fx + h.width {
                pixels.push(pv[out[(yy * dw + xx) as usize] as usize]);
            }
        }
        frames.push(Image {
            width: h.width as u32,
            height: h.height as u32,
            x: h.x_off,
            y: h.y_off - h.height + 1,
            pixels,
        });
    }

    let mut w = Bits::default();
    w.put(0, 32); // outsize coded
    w.put(3, 2); // raw pixels + equal cells present
    for code in [0, CODE_8, CODE_8, CODE_8, CODE_8, 0, 0] {
        w.put(code, 4);
    }
    for h in &heads {
        w.put(h.width as u32, 8);
        w.put(h.height as u32, 8);
        w.put(h.x_off as u32 & 0xFF, 8);
        w.put(h.y_off as u32 & 0xFF, 8);
        w.put(0, 1); // bottom-up
    }
    for s in [&eq, &mask_s, &enc, &raw_s] {
        assert!(s.len < 1 << 20, "sub-stream over 20 bits of size");
        w.put(s.len as u32, 20);
    }
    for i in 0..=255u8 {
        w.put(u32::from(pv.contains(&i)), 1);
    }
    let streams = [&eq, &mask_s, &enc, &raw_s, &pcd1, &pcd2];
    for s in streams {
        w.append(s);
    }
    (w, frames, streams.map(|s| s.len))
}

/// A DCC of `shape`, version 6.
pub fn dcc_file(shape: DccShape, seed: u32) -> DccOut {
    assert!(shape.width <= 240 && shape.height <= 240, "8-bit fields");
    let mut rng = Rng::new(seed);
    let pv = pixel_values();
    let dirs: Vec<(Bits, Vec<Image>, [usize; 6])> = (0..shape.directions)
        .map(|_| dcc_direction(shape, &mut rng, &pv))
        .collect();
    let mut file = vec![0x74, 6, shape.directions as u8];
    file.extend_from_slice(&shape.frames.to_le_bytes());
    file.extend_from_slice(&1u32.to_le_bytes());
    file.extend_from_slice(&0u32.to_le_bytes());
    let mut at = file.len() + 4 * dirs.len();
    for (b, _, _) in &dirs {
        file.extend_from_slice(&(at as u32).to_le_bytes());
        at += b.bytes.len();
    }
    let mut frames = Vec::new();
    let mut stream_bits = [0; 6];
    for (b, f, n) in dirs {
        file.extend_from_slice(&b.bytes);
        frames.push(f);
        for (t, n) in stream_bits.iter_mut().zip(n) {
            *t += n;
        }
    }
    DccOut {
        file,
        frames,
        stream_bits,
    }
}

/// The bench shapes: a monster walk
/// (8 directions × 8 frames, ~70 × 100), a large player / death
/// animation (16 × 24, ~110 × 130).
pub const DCC_MONSTER: DccShape = DccShape {
    directions: 8,
    frames: 8,
    width: 70,
    height: 100,
};
pub const DCC_LARGE: DccShape = DccShape {
    directions: 16,
    frames: 24,
    width: 110,
    height: 130,
};
/// A UI panel (1 × 4 of 256 × 256, opaque with windows) and an object /
/// overlay animation (8 × 16 of ~96 × 96, silhouette).
pub const DC6_PANEL: Dc6Shape = Dc6Shape {
    directions: 1,
    frames: 4,
    width: 256,
    height: 256,
    panel: true,
};
pub const DC6_SPRITE: Dc6Shape = Dc6Shape {
    directions: 8,
    frames: 16,
    width: 96,
    height: 96,
    panel: false,
};

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::dc6::Dc6;
    use d2_formats::dcc::Dcc;

    #[test]
    fn cell_sizes_match_the_spec_vectors() {
        assert_eq!(cell_sizes(1, 0), [1]);
        assert_eq!(cell_sizes(9, 0), [4, 5]);
        assert_eq!(cell_sizes(10, 2), [2, 4, 4]);
        assert_eq!(cell_sizes(7, 3), [1, 4, 2]);
        assert_eq!(cell_sizes(2, 3), [2]);
    }

    #[test]
    fn dc6_decodes_to_the_written_frames() {
        for shape in [DC6_PANEL, DC6_SPRITE] {
            let frames = dc6_frames(shape, 7);
            let file = dc6_file(&frames, shape.directions, shape.frames);
            let dc6 = Dc6::parse(&file).expect("dc6");
            assert_eq!(dc6.frames.len(), frames.len());
            for (got, want) in dc6.frames.iter().zip(&frames) {
                assert_eq!((got.width, got.height), (want.width, want.height));
                assert_eq!((got.offset_x, got.offset_y), (want.x, want.y));
                assert_eq!(got.pixels, want.pixels);
            }
        }
    }

    #[test]
    fn dcc_decodes_to_the_written_frames_using_every_stream() {
        for shape in [DCC_MONSTER, DCC_LARGE] {
            let out = dcc_file(shape, 11);
            let dcc = Dcc::parse(&out.file).expect("dcc");
            assert_eq!(dcc.directions.len(), shape.directions as usize);
            for (dir, want) in dcc.directions.iter().zip(&out.frames) {
                assert_eq!(dir.compression_flags, 3);
                assert!(dir.pcd_leftover_bits < 8);
                assert_eq!(dir.frames.len(), want.len());
                for (got, want) in dir.frames.iter().zip(want) {
                    assert_eq!((got.width, got.height), (want.width, want.height));
                    assert_eq!((got.x_min, got.y_min), (want.x, want.y));
                    assert_eq!(got.pixels, want.pixels);
                }
            }
            assert!(
                out.stream_bits.iter().all(|&n| n > 0),
                "{:?}",
                out.stream_bits
            );
            // Live-shaped: a body of many colours on a transparent ground.
            let px = &dcc.directions[0].frames[0].pixels;
            let opaque = px.iter().filter(|&&p| p != 0).count();
            assert!(opaque > px.len() / 4 && opaque < px.len(), "{opaque}");
            let mut seen = [false; 256];
            px.iter().for_each(|&p| seen[p as usize] = true);
            assert!(seen.iter().filter(|&&s| s).count() > 64);
        }
    }

    /// The checks above can fail: one flipped pixel-code bit changes the
    /// decoded frames (METHODS M08).
    #[test]
    fn a_flipped_bit_changes_the_decode() {
        let out = dcc_file(DCC_MONSTER, 11);
        let mut file = out.file.clone();
        let n = file.len();
        file[n - 40] ^= 0x10;
        let d = Dcc::parse(&file).expect("still a valid stream");
        assert_ne!(
            d.directions.last().unwrap().frames,
            Dcc::parse(&out.file)
                .unwrap()
                .directions
                .last()
                .unwrap()
                .frames
        );
    }
}
