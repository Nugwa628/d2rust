//! DCC animations: cell-based compressed 8-bit indexed frames.
//!
//! [`Dcc::parse`] takes the bytes of a `.dcc` file (character and monster
//! animations read from the player's game data) and returns every direction with
//! all of its frames decoded to palette indices. Animation and rendering code in
//! D2RUST draws from the decoded frames.

use crate::cursor::{invalid, FormatError};

const FORMAT: &str = "dcc";
const SIGNATURE: u8 = 0x74;
const MAX_FRAMES: u32 = 256;
const MAX_DIR_PIXELS: u64 = 0x100_0000;
/// Implementation limit on the direction boxes of all directions together:
/// decoding a direction costs time and memory in proportion to its box,
/// which a few header bits can make large.
const MAX_TOTAL_DIR_PIXELS: u64 = 0x400_0000;
const ENCODED_BITS: [u32; 16] = [0, 1, 2, 4, 6, 8, 10, 12, 14, 16, 20, 24, 26, 28, 30, 32];

/// LSB-first bit cursor over `data[.. end_bit]`.
#[derive(Clone)]
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            end: data.len() * 8,
        }
    }

    /// A sub-stream of the next `len` bits; this cursor skips past it.
    fn take(&mut self, len: usize, what: &str) -> Result<Bits<'a>, FormatError> {
        if self.pos + len > self.end {
            return Err(invalid(
                FORMAT,
                format!("{what} stream extends past the data"),
            ));
        }
        let sub = Bits {
            data: self.data,
            pos: self.pos,
            end: self.pos + len,
        };
        self.pos += len;
        Ok(sub)
    }

    fn remaining(&self) -> usize {
        self.end - self.pos
    }

    fn read(&mut self, n: u32) -> Result<u32, FormatError> {
        if n == 0 {
            return Ok(0);
        }
        if self.pos + n as usize > self.end {
            return Err(invalid(FORMAT, "bit stream ended early"));
        }
        let byte = self.pos / 8;
        let shift = self.pos % 8;
        let mut v: u64 = 0;
        for i in 0..5 {
            if let Some(&b) = self.data.get(byte + i) {
                v |= u64::from(b) << (8 * i);
            }
        }
        self.pos += n as usize;
        Ok(((v >> shift) & ((1u64 << n) - 1)) as u32)
    }

    fn read_signed(&mut self, n: u32) -> Result<i32, FormatError> {
        let v = self.read(n)?;
        if n == 0 || n == 32 {
            return Ok(v as i32);
        }
        let sign = 1u32 << (n - 1);
        Ok(((v ^ sign).wrapping_sub(sign)) as i32)
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

/// One decoded DCC frame: header fields plus its pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DccFrame {
    /// Frame header word `variable0`, kept as read.
    pub variable0: u32,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Horizontal draw offset of the frame.
    pub x_offset: i32,
    /// Vertical draw offset of the frame.
    pub y_offset: i32,
    /// Size in bytes of the frame's coded data, as stored in its header.
    pub coded_bytes: u32,
    /// True if the frame's rows are stored bottom-up.
    pub bottom_up: bool,
    /// Optional bytes that follow the frame header, kept as read.
    pub optional_data: Vec<u8>,
    /// Box in direction space: `x_min..=x_max`, `y_min..=y_max`.
    pub x_min: i32,
    /// Top edge of the box in direction space.
    pub y_min: i32,
    /// Palette indices, `width × height`, top row first; 0 = transparent.
    pub pixels: Vec<u8>,
}

/// One decoded DCC direction: its box, header fields and frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DccDirection {
    /// Size in bytes of the direction's coded pixel data, as stored.
    pub outsize_coded: u32,
    /// Compression flags byte of the direction header.
    pub compression_flags: u8,
    /// Left edge of the direction box.
    pub x_min: i32,
    /// Top edge of the direction box.
    pub y_min: i32,
    /// Width of the direction box in pixels.
    pub width: u32,
    /// Height of the direction box in pixels.
    pub height: u32,
    /// The direction's frames, in order.
    pub frames: Vec<DccFrame>,
    /// Unread bits at the end of the pixel-code-and-displacement stream.
    pub pcd_leftover_bits: usize,
}

/// A parsed DCC animation: file header and every direction decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dcc {
    /// Format version byte.
    pub version: u8,
    /// Frames in each direction.
    pub frames_per_direction: u32,
    /// Header tag word, kept as read.
    pub tag: u32,
    /// Final DC6 size word from the header, kept as read.
    pub final_dc6_size: u32,
    /// The decoded directions, in file order.
    pub directions: Vec<DccDirection>,
}

impl Dcc {
    /// Parses a DCC file, decoding every direction. Fails on a wrong signature, too
    /// many frames, out-of-order direction offsets or malformed direction data.
    pub fn parse(data: &[u8]) -> Result<Dcc, FormatError> {
        let mut c = crate::cursor::Cursor::new(data, FORMAT);
        let signature = c.u8()?;
        if signature != SIGNATURE {
            return Err(invalid(FORMAT, format!("signature {signature:#x}")));
        }
        let version = c.u8()?;
        let directions = usize::from(c.u8()?);
        let frames_per_direction = c.u32()?;
        if frames_per_direction > MAX_FRAMES {
            return Err(invalid(FORMAT, format!("{frames_per_direction} frames")));
        }
        let tag = c.u32()?;
        let final_dc6_size = c.u32()?;
        let mut offsets = (0..directions)
            .map(|_| c.u32().map(|o| o as usize))
            .collect::<Result<Vec<_>, _>>()?;
        offsets.push(data.len());
        if offsets[0] < c.pos() || offsets.windows(2).any(|w| w[1] < w[0]) {
            return Err(invalid(FORMAT, "direction offsets out of order"));
        }
        let mut box_budget = MAX_TOTAL_DIR_PIXELS;
        let dirs = (0..directions)
            .map(|d| {
                decode_direction(
                    &data[offsets[d]..offsets[d + 1]],
                    frames_per_direction as usize,
                    &mut box_budget,
                )
                .map_err(|e| invalid(FORMAT, format!("direction {d}: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Dcc {
            version,
            frames_per_direction,
            tag,
            final_dc6_size,
            directions: dirs,
        })
    }
}

/// One frame cell: position relative to the direction box, size, and its
/// direction cell.
#[derive(Clone, Copy)]
struct Cell {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    dcell: usize,
}

/// Cell sizes along one axis (the first cell ends on a 4-pixel boundary).
fn axis_cells(offset: usize, len: usize) -> Vec<usize> {
    let first = 4 - offset % 4;
    if len <= first + 1 {
        return vec![len];
    }
    let rem = len - first;
    let mut cells = vec![first];
    match rem % 4 {
        0 => cells.extend(std::iter::repeat_n(4, rem / 4)),
        1 => {
            cells.extend(std::iter::repeat_n(4, rem / 4 - 1));
            cells.push(5);
        }
        r => {
            cells.extend(std::iter::repeat_n(4, rem / 4));
            cells.push(r);
        }
    }
    cells
}

struct Entry {
    frame: usize,
    cell: usize,
    v: [u8; 4],
}

fn decode_direction(
    data: &[u8],
    frame_count: usize,
    box_budget: &mut u64,
) -> Result<DccDirection, FormatError> {
    let mut b = Bits::new(data);
    let outsize_coded = b.read(32)?;
    let compression_flags = b.read(2)? as u8;
    let mut codes = [0u32; 7];
    for code in &mut codes {
        *code = ENCODED_BITS[b.read(4)? as usize];
    }
    let [var0_bits, width_bits, height_bits, x_bits, y_bits, optional_bits, coded_bits] = codes;

    struct Header {
        variable0: u32,
        width: u32,
        height: u32,
        x_offset: i32,
        y_offset: i32,
        optional: u32,
        coded_bytes: u32,
        bottom_up: bool,
    }
    let mut headers = Vec::with_capacity(frame_count);
    for _ in 0..frame_count {
        headers.push(Header {
            variable0: b.read(var0_bits)?,
            width: b.read(width_bits)?,
            height: b.read(height_bits)?,
            x_offset: b.read_signed(x_bits)?,
            y_offset: b.read_signed(y_bits)?,
            optional: b.read(optional_bits)?,
            coded_bytes: b.read(coded_bits)?,
            bottom_up: b.read(1)? == 1,
        });
    }
    let mut optional_data = vec![Vec::new(); frame_count];
    if headers.iter().any(|h| h.optional > 0) {
        b.align();
        for (f, h) in headers.iter().enumerate() {
            let start = b.pos / 8;
            let end = start + h.optional as usize;
            optional_data[f] = data
                .get(start..end)
                .ok_or_else(|| invalid(FORMAT, "optional data past the end"))?
                .to_vec();
            b.pos = end * 8;
        }
        if b.pos > b.end {
            return Err(invalid(FORMAT, "optional data past the end"));
        }
    }

    let equal_size = if compression_flags & 2 != 0 {
        b.read(20)?
    } else {
        0
    } as usize;
    let mask_size = b.read(20)? as usize;
    let (encoding_size, raw_size) = if compression_flags & 1 != 0 {
        (b.read(20)? as usize, b.read(20)? as usize)
    } else {
        (0, 0)
    };
    let mut pv = Vec::with_capacity(256);
    for i in 0..=255u8 {
        if b.read(1)? == 1 {
            pv.push(i);
        }
    }
    let mut equal = b.take(equal_size, "equal cells")?;
    let mut mask = b.take(mask_size, "pixel mask")?;
    let mut encoding = b.take(encoding_size, "encoding type")?;
    let mut raw = b.take(raw_size, "raw pixel")?;
    let mut pcd = b.take(b.remaining(), "pixel code")?;

    // Boxes.
    if headers.is_empty() {
        return Ok(DccDirection {
            outsize_coded,
            compression_flags,
            x_min: 0,
            y_min: 0,
            width: 0,
            height: 0,
            frames: Vec::new(),
            pcd_leftover_bits: pcd.remaining(),
        });
    }
    let mut boxes = Vec::with_capacity(frame_count);
    for h in &headers {
        if h.width == 0 || h.height == 0 {
            return Err(invalid(
                FORMAT,
                format!("frame size {}x{}", h.width, h.height),
            ));
        }
        let x_min = i64::from(h.x_offset);
        let y_min = if h.bottom_up {
            i64::from(h.y_offset)
        } else {
            i64::from(h.y_offset) - i64::from(h.height) + 1
        };
        boxes.push((x_min, y_min, i64::from(h.width), i64::from(h.height)));
    }
    let dx_min = boxes.iter().map(|b| b.0).min().unwrap();
    let dy_min = boxes.iter().map(|b| b.1).min().unwrap();
    let dx_max = boxes.iter().map(|b| b.0 + b.2).max().unwrap();
    let dy_max = boxes.iter().map(|b| b.1 + b.3).max().unwrap();
    let (dir_w, dir_h) = ((dx_max - dx_min) as u64, (dy_max - dy_min) as u64);
    let area = dir_w
        .checked_mul(dir_h)
        .filter(|&n| n <= MAX_DIR_PIXELS)
        .ok_or_else(|| invalid(FORMAT, format!("direction box {dir_w}x{dir_h}")))?;
    *box_budget = box_budget.checked_sub(area).ok_or_else(|| {
        invalid(
            FORMAT,
            format!("direction boxes add up to more than {MAX_TOTAL_DIR_PIXELS} pixels"),
        )
    })?;
    // Box corners are reported as i32 (a bottom-up offset minus the height
    // can leave that range).
    if boxes
        .iter()
        .any(|b| i32::try_from(b.0).is_err() || i32::try_from(b.1).is_err())
    {
        return Err(invalid(FORMAT, "frame box outside the i32 range"));
    }
    let (dir_w, dir_h) = (dir_w as usize, dir_h as usize);
    let cells_w = dir_w.div_ceil(4);
    let cells_h = dir_h.div_ceil(4);

    // Frame cell layouts.
    let layouts: Vec<Vec<Cell>> = boxes
        .iter()
        .map(|&(x, y, w, h)| {
            let fx = (x - dx_min) as usize;
            let fy = (y - dy_min) as usize;
            let xs = axis_cells(fx, w as usize);
            let ys = axis_cells(fy, h as usize);
            let mut cells = Vec::with_capacity(xs.len() * ys.len());
            let mut cy = fy;
            for (j, &ch) in ys.iter().enumerate() {
                let mut cx = fx;
                for (i, &cw) in xs.iter().enumerate() {
                    let dcell = (fy / 4 + j) * cells_w + fx / 4 + i;
                    cells.push(Cell {
                        x: cx,
                        y: cy,
                        w: cw,
                        h: ch,
                        dcell,
                    });
                    cx += cw;
                }
                cy += ch;
            }
            cells
        })
        .collect();

    // Stage 1: cell colors.
    let mut latest: Vec<Option<[u8; 4]>> = vec![None; cells_w * cells_h];
    let mut queue: Vec<Entry> = Vec::new();
    for (f, cells) in layouts.iter().enumerate() {
        for (ci, cell) in cells.iter().enumerate() {
            let previous = latest[cell.dcell];
            let pixel_mask = if previous.is_some() {
                let is_equal = equal_size > 0 && equal.read(1)? == 1;
                if is_equal {
                    continue;
                }
                mask.read(4)?
            } else {
                0xF
            };
            let n = pixel_mask.count_ones();
            let is_raw = n > 0 && encoding_size > 0 && encoding.read(1)? == 1;
            let mut stack = [0u8; 4];
            let mut pushed = 0;
            let mut last = 0u8;
            for _ in 0..n {
                let code = if is_raw {
                    raw.read(8)? as u8
                } else {
                    let mut code = u32::from(last);
                    loop {
                        let disp = pcd.read(4)?;
                        // Codes are bytes: only the low 8 bits count.
                        code = code.wrapping_add(disp);
                        if disp != 15 {
                            break;
                        }
                    }
                    code as u8
                };
                if code == last {
                    break;
                }
                stack[pushed] = code;
                pushed += 1;
                last = code;
            }
            let prev = previous.unwrap_or([0; 4]);
            let mut v = [0u8; 4];
            for (i, value) in v.iter_mut().enumerate() {
                *value = if pixel_mask & (1 << i) != 0 {
                    if pushed > 0 {
                        pushed -= 1;
                        stack[pushed]
                    } else {
                        0
                    }
                } else {
                    prev[i]
                };
            }
            latest[cell.dcell] = Some(v);
            queue.push(Entry {
                frame: f,
                cell: ci,
                v,
            });
        }
    }

    // Stage 2: build frames.
    let mut buffer = vec![0u8; dir_w * dir_h];
    let mut last_rect: Vec<Option<(usize, usize, usize, usize)>> = vec![None; cells_w * cells_h];
    let mut next = 0;
    let mut frames = Vec::with_capacity(frame_count);
    for (f, cells) in layouts.iter().enumerate() {
        // The frame's cells all lie in its box, so only the box is kept.
        let (bx, by, bw, bh) = boxes[f];
        let frame_box = Frame {
            x: (bx - dx_min) as usize,
            y: (by - dy_min) as usize,
            w: bw as usize,
            h: bh as usize,
        };
        let mut out = vec![0u8; frame_box.w * frame_box.h];
        for (ci, cell) in cells.iter().enumerate() {
            let rect = (cell.x, cell.y, cell.w, cell.h);
            if next < queue.len() && queue[next].frame == f && queue[next].cell == ci {
                let v = queue[next].v;
                next += 1;
                if v[0] == v[1] {
                    for y in cell.y..cell.y + cell.h {
                        buffer[y * dir_w + cell.x..y * dir_w + cell.x + cell.w].fill(v[0]);
                    }
                } else {
                    let bits = if v[1] == v[2] { 1 } else { 2 };
                    for y in cell.y..cell.y + cell.h {
                        for x in cell.x..cell.x + cell.w {
                            buffer[y * dir_w + x] = v[pcd.read(bits)? as usize];
                        }
                    }
                }
                copy_rect(&buffer, dir_w, &mut out, &frame_box, rect);
            } else {
                match last_rect[cell.dcell] {
                    Some((lx, ly, lw, lh)) if lw == cell.w && lh == cell.h => {
                        // Read the whole source before writing (overlap-safe).
                        let mut tmp = Vec::with_capacity(lw * lh);
                        for y in ly..ly + lh {
                            tmp.extend_from_slice(&buffer[y * dir_w + lx..y * dir_w + lx + lw]);
                        }
                        for (row, y) in (cell.y..cell.y + cell.h).enumerate() {
                            buffer[y * dir_w + cell.x..y * dir_w + cell.x + cell.w]
                                .copy_from_slice(&tmp[row * lw..(row + 1) * lw]);
                        }
                        copy_rect(&buffer, dir_w, &mut out, &frame_box, rect);
                    }
                    _ => {
                        for y in cell.y..cell.y + cell.h {
                            buffer[y * dir_w + cell.x..y * dir_w + cell.x + cell.w].fill(0);
                        }
                    }
                }
            }
            last_rect[cell.dcell] = Some(rect);
        }

        // Map codes to palette indices.
        let h = &headers[f];
        let pixels = out
            .iter()
            .map(|&code| {
                pv.get(usize::from(code)).copied().ok_or_else(|| {
                    invalid(
                        FORMAT,
                        format!("pixel code {code} with {} colors", pv.len()),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        frames.push(DccFrame {
            variable0: h.variable0,
            width: h.width,
            height: h.height,
            x_offset: h.x_offset,
            y_offset: h.y_offset,
            coded_bytes: h.coded_bytes,
            bottom_up: h.bottom_up,
            optional_data: std::mem::take(&mut optional_data[f]),
            x_min: bx as i32,
            y_min: by as i32,
            pixels,
        });
    }

    for (stream, name) in [
        (&equal, "equal cells"),
        (&mask, "pixel mask"),
        (&encoding, "encoding type"),
        (&raw, "raw pixel"),
    ] {
        if stream.remaining() != 0 {
            return Err(invalid(
                FORMAT,
                format!("{name} stream has {} unread bits", stream.remaining()),
            ));
        }
    }
    Ok(DccDirection {
        outsize_coded,
        compression_flags,
        x_min: dx_min as i32,
        y_min: dy_min as i32,
        width: dir_w as u32,
        height: dir_h as u32,
        frames,
        pcd_leftover_bits: pcd.remaining(),
    })
}

/// A frame's box in direction coordinates.
struct Frame {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}

/// Copies `rect` (direction coordinates, inside `frame`) from the direction
/// buffer `src` to the frame image `dst`.
fn copy_rect(
    src: &[u8],
    src_stride: usize,
    dst: &mut [u8],
    frame: &Frame,
    (x, y, w, h): (usize, usize, usize, usize),
) {
    debug_assert!(x >= frame.x && x + w <= frame.x + frame.w);
    debug_assert!(y >= frame.y && y + h <= frame.y + frame.h);
    for row in y..y + h {
        let from = row * src_stride + x;
        let to = (row - frame.y) * frame.w + (x - frame.x);
        dst[to..to + w].copy_from_slice(&src[from..from + w]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_sizes() {
        assert_eq!(axis_cells(0, 1), [1]);
        assert_eq!(axis_cells(0, 9), [4, 5]);
        assert_eq!(axis_cells(2, 10), [2, 4, 4]);
        assert_eq!(axis_cells(3, 7), [1, 4, 2]);
        assert_eq!(axis_cells(3, 2), [2]);
        assert_eq!(axis_cells(0, 4), [4]);
        assert_eq!(axis_cells(0, 8), [4, 4]);
        for off in 0..4 {
            for len in 1..40 {
                assert_eq!(
                    axis_cells(off, len).iter().sum::<usize>(),
                    len,
                    "{off} {len}"
                );
            }
        }
    }

    #[test]
    fn signed_fields() {
        // 0b1111 as a 4-bit signed field is -1; 0b0111 is 7.
        let mut b = Bits::new(&[0x7F]);
        assert_eq!(b.read_signed(4).unwrap(), -1);
        assert_eq!(b.read_signed(4).unwrap(), 7);
    }

    /// Writes LSB-first bit fields.
    #[derive(Default)]
    struct BitWriter {
        bytes: Vec<u8>,
        bits: usize,
    }
    impl BitWriter {
        fn put(&mut self, value: u32, n: u32) {
            for i in 0..n {
                if self.bits.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                if value >> i & 1 == 1 {
                    *self.bytes.last_mut().unwrap() |= 1 << (self.bits % 8);
                }
                self.bits += 1;
            }
        }
    }

    /// One direction, one 4×4 frame at (0, -3) top-down: a single cell with
    /// colors {10, 20}: the left half 10, the right half 20.
    fn one_frame_file() -> Vec<u8> {
        let mut w = BitWriter::default();
        w.put(0, 32); // outsize coded
        w.put(0, 2); // flags: no equal cells, no raw encoding
                     // width codes: variable0 0 bits, width 4 bits (code 3), height 4 bits,
                     // x 4 bits, y 4 bits, optional 0, coded 0
        for code in [0, 3, 3, 3, 3, 0, 0] {
            w.put(code, 4);
        }
        w.put(4, 4); // width
        w.put(4, 4); // height
        w.put(0, 4); // x offset
        w.put(3, 4); // y offset (y_max = 3, so y_min = 0)
        w.put(0, 1); // top-down
        w.put(0, 20); // pixel mask stream size (first touch needs none)
                      // pixel values: palette indices 0, 10, 20 present (codes 0, 1, 2)
        for i in 0..256u32 {
            w.put(u32::from(i == 0 || i == 10 || i == 20), 1);
        }
        // Stage 1, PCD: codes pushed 1 then 2, then a repeat (disp 0) stops.
        w.put(1, 4); // code 1
        w.put(1, 4); // code 2
        w.put(0, 4); // code 2 again: stop
                     // v = [2, 1, 0, 0] (pops 2 then 1, then zeros): v0 != v1, v1 != v2,
                     // so 2 bits per pixel. Pixels: indices 1 (code 1) on the left half,
                     // 0 (code 2) on the right half.
        for _y in 0..4 {
            for x in 0..4 {
                w.put(if x < 2 { 1 } else { 0 }, 2);
            }
        }
        let dir = w.bytes;
        let mut file = vec![SIGNATURE, 6, 1];
        file.extend_from_slice(&1u32.to_le_bytes()); // frames per direction
        file.extend_from_slice(&1u32.to_le_bytes()); // tag
        file.extend_from_slice(&0u32.to_le_bytes()); // final dc6 size
        file.extend_from_slice(&19u32.to_le_bytes()); // direction offset
        file.extend(dir);
        file
    }

    #[test]
    fn decodes_synthetic_frame() {
        let dcc = Dcc::parse(&one_frame_file()).unwrap();
        let frame = &dcc.directions[0].frames[0];
        assert_eq!((frame.width, frame.height), (4, 4));
        let expected: Vec<u8> = (0..16).map(|i| if i % 4 < 2 { 10 } else { 20 }).collect();
        assert_eq!(frame.pixels, expected);
        assert!(dcc.directions[0].pcd_leftover_bits < 8);
    }

    #[test]
    fn rejects_bad_signature() {
        let mut data = one_frame_file();
        data[0] = 0x75;
        assert!(Dcc::parse(&data).is_err());
    }

    /// A direction of 1-pixel-or-larger frames `(width, height, x, y,
    /// bottom_up)` with 32-bit header fields, palette {0}, and every
    /// first-touch cell code 0 (one 4-bit PCD read each).
    fn direction(frames: &[(u32, u32, i32, i32, bool)], first_touch_cells: usize) -> Vec<u8> {
        let mut w = BitWriter::default();
        w.put(0, 32); // outsize coded
        w.put(0, 2); // flags
        for code in [0, 15, 15, 15, 15, 0, 0] {
            w.put(code, 4);
        }
        for &(fw, fh, x, y, up) in frames {
            w.put(fw, 32);
            w.put(fh, 32);
            w.put(x as u32, 32);
            w.put(y as u32, 32);
            w.put(u32::from(up), 1);
        }
        w.put(0, 20); // pixel mask stream size
        for i in 0..256u32 {
            w.put(u32::from(i == 0), 1);
        }
        for _ in 0..first_touch_cells {
            w.put(0, 4);
        }
        w.bytes
    }

    fn dcc_file(frames_per_direction: u32, dirs: &[Vec<u8>]) -> Vec<u8> {
        let mut file = vec![SIGNATURE, 6, dirs.len() as u8];
        file.extend_from_slice(&frames_per_direction.to_le_bytes());
        file.extend_from_slice(&1u32.to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
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

    /// Three 1×1 frames, top-down, palette {0, 10, 20}: frame 0 at x 0
    /// (direction cell 0), frame 1 at x 4 (direction cell 1, so cell 0 is
    /// skipped), frame 2 at x 0 again as an equal cell.
    fn skipped_cell_file() -> Vec<u8> {
        let mut w = BitWriter::default();
        w.put(0, 32); // outsize coded
        w.put(2, 2); // flags: equal-cells stream present
        for code in [0, 15, 15, 15, 15, 0, 0] {
            w.put(code, 4);
        }
        for x in [0u32, 4, 0] {
            for v in [1, 1, x, 0] {
                w.put(v, 32); // width, height, x, y
            }
            w.put(0, 1); // top-down
        }
        w.put(1, 20); // equal-cells stream: 1 bit
        w.put(0, 20); // pixel-mask stream: none (first touches use 0xF)
        for i in 0..256u32 {
            w.put(u32::from(i == 0 || i == 10 || i == 20), 1);
        }
        w.put(1, 1); // frame 2, cell 0: equal
                     // Stage 1 PCD: frame 0 pushes code 1, frame 1 pushes code 2.
        for disp in [1, 0, 2, 0] {
            w.put(disp, 4);
        }
        // Stage 2: v = [c, 0, 0, 0], 1 bit per pixel, index 0.
        w.put(0, 1);
        w.put(0, 1);
        dcc_file(3, &[w.bytes])
    }

    #[test]
    fn equal_cell_copies_from_last_draw_of_its_cell() {
        let dcc = Dcc::parse(&skipped_cell_file()).unwrap();
        let frames = &dcc.directions[0].frames;
        assert_eq!(frames[0].pixels, [10]);
        assert_eq!(frames[1].pixels, [20]);
        // Frame 1 never drew direction cell 0 (its output there is 0); the
        // equal cell of frame 2 copies what frame 0 drew into the
        // persistent buffer, not frame 1's output.
        assert_eq!(frames[2].pixels, [10]);
    }

    #[test]
    fn regress_sparse_boxes_bounded() {
        // Two 1×1 frames at opposite corners of a 4000×4000 box: ~80 bytes
        // per direction, 16M pixels of decode work each. 255 of them took
        // seconds; the whole-file box budget stops at the fifth.
        let dir = direction(&[(1, 1, 0, 0, true), (1, 1, 3999, 3999, true)], 2);
        let one = Dcc::parse(&dcc_file(2, std::slice::from_ref(&dir))).unwrap();
        assert_eq!(one.directions[0].width, 4000);
        assert_eq!(one.directions[0].frames[1].pixels, [0]);
        let err = Dcc::parse(&dcc_file(2, &vec![dir; 255])).unwrap_err();
        assert!(err.to_string().contains("direction 4:"), "{err}");
    }

    #[test]
    fn regress_box_area_overflow() {
        // Corners i32::MIN and i32::MAX + u32::MAX: the box is ~2^33 on each
        // side, and its area overflowed u64 (panic in debug builds).
        let dir = direction(
            &[
                (1, 1, i32::MIN, i32::MIN, true),
                (u32::MAX, u32::MAX, i32::MAX, i32::MAX, true),
            ],
            0,
        );
        let err = Dcc::parse(&dcc_file(2, &[dir])).unwrap_err();
        assert!(err.to_string().contains("direction box"), "{err}");
    }

    #[test]
    fn regress_box_corner_outside_i32() {
        // Top-down frame at y_offset i32::MIN, height 2: y_min is
        // i32::MIN - 1, which was reported wrapped to i32::MAX.
        let dir = direction(&[(1, 2, 0, i32::MIN, false)], 1);
        let err = Dcc::parse(&dcc_file(1, &[dir])).unwrap_err();
        assert!(err.to_string().contains("i32"), "{err}");
        // One row higher is in range and decodes.
        let dir = direction(&[(1, 2, 0, i32::MIN + 1, false)], 1);
        let dcc = Dcc::parse(&dcc_file(1, &[dir])).unwrap();
        assert_eq!(dcc.directions[0].frames[0].y_min, i32::MIN);
    }

    mod robust {
        use super::*;
        use crate::robust::mutated;
        use crate::robust_tests::{check, config};
        use proptest::prelude::*;

        #[test]
        fn builder_is_valid() {
            assert!(Dcc::parse(&one_frame_file()).is_ok());
        }

        proptest! {
            #![proptest_config(config(64))]

            #[test]
            fn mutated_file(data in mutated(one_frame_file())) {
                check(data, Dcc::parse);
            }
        }
    }
}
