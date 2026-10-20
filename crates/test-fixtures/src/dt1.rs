//! DT1 writer: the inverse of `d2_formats::dt1::Dt1::parse`, from a
//! [`Dt1`] value. Layout: the 276-byte file header, the 96-byte tile
//! headers right after it (first tile header offset 276), then per tile
//! its 20-byte block headers followed by the encoded block data; each
//! block's data offset is relative to its tile's block headers offset.
//!
//! Block pixels are encoded from the decoded form the parser returns:
//! format 1 takes the 256 diamond pixels (`SKIP` / `RUN` rows; pixels
//! outside the diamond are dropped), any other format is RLE-encoded
//! row by row as `(skip, count)` pairs, runs of non-zero pixels, each
//! row closed by `(0, 0)` (rows after the last pixel are not written).
//! Reserved and padding bytes are 0. The tile's "block data length"
//! (0x4C, not read by the parser) is the size of its headers + data.

use d2_formats::dt1::{Dt1, Dt1Block, Dt1Tile, ISO_FORMAT, ISO_HEIGHT, ISO_WIDTH, RLE_WIDTH};

const HEADER_LEN: usize = 276;
const TILE_LEN: usize = 96;
const BLOCK_LEN: usize = 20;
const ISO_SKIP: [usize; ISO_HEIGHT] = [14, 12, 10, 8, 6, 4, 2, 0, 2, 4, 6, 8, 10, 12, 14];
const ISO_RUN: [usize; ISO_HEIGHT] = [4, 8, 12, 16, 20, 24, 28, 32, 28, 24, 20, 16, 12, 8, 4];

/// The 256 bytes of an isometric block.
pub fn encode_iso(pixels: &[u8]) -> Vec<u8> {
    assert_eq!(pixels.len(), ISO_WIDTH * ISO_HEIGHT, "iso block size");
    let mut out = Vec::with_capacity(256);
    for r in 0..ISO_HEIGHT {
        let start = r * ISO_WIDTH + ISO_SKIP[r];
        out.extend_from_slice(&pixels[start..start + ISO_RUN[r]]);
    }
    out
}

/// The RLE pairs of a 32-wide block.
pub fn encode_rle(pixels: &[u8]) -> Vec<u8> {
    assert_eq!(pixels.len() % RLE_WIDTH, 0, "RLE block width");
    let rows: Vec<&[u8]> = pixels.chunks(RLE_WIDTH).collect();
    let last = rows.iter().rposition(|r| r.iter().any(|&p| p != 0));
    let mut out = Vec::new();
    for row in &rows[..last.map_or(0, |l| l + 1)] {
        let mut x = 0;
        while x < RLE_WIDTH {
            let start = match row[x..].iter().position(|&p| p != 0) {
                Some(s) => x + s,
                None => break,
            };
            let end = row[start..]
                .iter()
                .position(|&p| p == 0)
                .map_or(RLE_WIDTH, |e| start + e);
            out.push((start - x) as u8);
            out.push((end - start) as u8);
            out.extend_from_slice(&row[start..end]);
            x = end;
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

fn encode(b: &Dt1Block) -> Vec<u8> {
    if b.format == ISO_FORMAT {
        encode_iso(&b.pixels)
    } else {
        encode_rle(&b.pixels)
    }
}

/// The file bytes of `d`.
pub fn write(d: &Dt1) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&d.version.to_le_bytes());
    out.extend_from_slice(&d.minor_version.to_le_bytes());
    out.extend_from_slice(&[0; 260]);
    out.extend_from_slice(&(d.tiles.len() as u32).to_le_bytes());
    out.extend_from_slice(&(HEADER_LEN as u32).to_le_bytes());

    // Block sections after all tile headers.
    let encoded: Vec<Vec<Vec<u8>>> = d
        .tiles
        .iter()
        .map(|t| t.blocks.iter().map(encode).collect())
        .collect();
    let mut at = HEADER_LEN + TILE_LEN * d.tiles.len();
    let mut sections = Vec::new();
    for (t, blocks) in d.tiles.iter().zip(&encoded) {
        let headers = BLOCK_LEN * blocks.len();
        let len = headers + blocks.iter().map(Vec::len).sum::<usize>();
        out.extend_from_slice(&tile_header(t, at, len));
        let mut section = Vec::with_capacity(len);
        let mut data_at = headers;
        for (b, bytes) in t.blocks.iter().zip(blocks) {
            section.extend_from_slice(&b.x.to_le_bytes());
            section.extend_from_slice(&b.y.to_le_bytes());
            section.extend_from_slice(&b.unknown1.to_le_bytes());
            section.push(b.grid_x);
            section.push(b.grid_y);
            section.extend_from_slice(&b.format.to_le_bytes());
            section.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            section.extend_from_slice(&b.unknown2.to_le_bytes());
            section.extend_from_slice(&(data_at as u32).to_le_bytes());
            data_at += bytes.len();
        }
        for bytes in blocks {
            section.extend_from_slice(bytes);
        }
        at += len;
        sections.push(section);
    }
    for s in sections {
        out.extend_from_slice(&s);
    }
    out
}

fn tile_header(t: &Dt1Tile, blocks_at: usize, blocks_len: usize) -> [u8; TILE_LEN] {
    let mut h = [0u8; TILE_LEN];
    let mut put = |off: usize, b: &[u8]| h[off..off + b.len()].copy_from_slice(b);
    put(0x00, &t.light_direction.to_le_bytes());
    put(0x04, &t.roof_height.to_le_bytes());
    put(0x06, &t.material_flags.to_le_bytes());
    put(0x08, &t.height.to_le_bytes());
    put(0x0C, &t.width.to_le_bytes());
    put(0x10, &t.unknown_height.to_le_bytes());
    put(0x14, &t.orientation.to_le_bytes());
    put(0x18, &t.main_index.to_le_bytes());
    put(0x1C, &t.sub_index.to_le_bytes());
    put(0x20, &t.rarity.to_le_bytes());
    put(0x24, &t.unknown_color.to_le_bytes());
    put(0x28, &t.subtile_flags);
    put(0x48, &(blocks_at as u32).to_le_bytes());
    put(0x4C, &(blocks_len as u32).to_le_bytes());
    put(0x50, &(t.blocks.len() as u32).to_le_bytes());
    put(0x58, &t.unknown_58.to_le_bytes());
    put(0x5A, &t.cache_index.to_le_bytes());
    put(0x5C, &t.unknown_5c.to_le_bytes());
    h
}

/// A tile with no blocks: key (`orientation`, `main`, `sub`), `rarity`,
/// every sub-tile flag 0 (walkable), size 0.
pub fn tile(orientation: u32, main: u32, sub: u32, rarity: u32) -> Dt1Tile {
    Dt1Tile {
        light_direction: 0,
        roof_height: 0,
        material_flags: 0,
        height: 0,
        width: 0,
        unknown_height: 0,
        orientation,
        main_index: main,
        sub_index: sub,
        rarity,
        unknown_color: 0,
        subtile_flags: [0; 25],
        unknown_58: 0,
        cache_index: 0,
        unknown_5c: 0,
        blocks: Vec::new(),
    }
}

/// A version 7.6 file of `tiles` (minor version 6, the game files' own).
pub fn file(tiles: Vec<Dt1Tile>) -> Dt1 {
    Dt1 {
        version: 7,
        minor_version: 6,
        tiles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::dt1::RLE_HEIGHT;

    fn iso_block(seed: u8) -> Dt1Block {
        let mut pixels = vec![0u8; ISO_WIDTH * ISO_HEIGHT];
        let mut k = seed;
        for r in 0..ISO_HEIGHT {
            for x in ISO_SKIP[r]..ISO_SKIP[r] + ISO_RUN[r] {
                pixels[r * ISO_WIDTH + x] = k;
                k = k.wrapping_add(1);
            }
        }
        Dt1Block {
            x: -32,
            y: 16,
            unknown1: 5,
            grid_x: 1,
            grid_y: 2,
            format: ISO_FORMAT,
            unknown2: 9,
            pixels,
        }
    }

    fn rle_block(format: u16) -> Dt1Block {
        let mut pixels = vec![0u8; RLE_WIDTH * RLE_HEIGHT];
        // Runs at the start, the middle and the end of rows, a fully
        // set row, empty rows between, nothing after row 20.
        for (i, x) in [0, 1, 2, 15, 16, 30, 31].into_iter().enumerate() {
            pixels[3 * RLE_WIDTH + x] = 10 + i as u8;
        }
        for x in 0..RLE_WIDTH {
            pixels[7 * RLE_WIDTH + x] = 200 + (x % 50) as u8;
        }
        pixels[20 * RLE_WIDTH + 31] = 1;
        Dt1Block {
            x: 0,
            y: -64,
            unknown1: 0,
            grid_x: 0,
            grid_y: 0,
            format,
            unknown2: 0,
            pixels,
        }
    }

    fn sample() -> Dt1 {
        let mut a = tile(0, 1, 2, 3);
        a.light_direction = 1;
        a.roof_height = 80;
        a.material_flags = 0x100;
        a.height = -80;
        a.width = 160;
        a.unknown_height = 7;
        a.unknown_color = 0xFF;
        a.subtile_flags = std::array::from_fn(|i| i as u8);
        a.unknown_58 = 1;
        a.cache_index = 2;
        a.unknown_5c = 3;
        a.blocks = vec![iso_block(0), iso_block(100)];
        let mut b = tile(1, 4, 5, 0);
        b.blocks = vec![rle_block(0x1001), rle_block(0x2005)];
        file(vec![a, tile(10, 0, 0, 1), b])
    }

    #[test]
    fn round_trips() {
        let d = sample();
        assert_eq!(Dt1::parse(&write(&d)).unwrap(), d);
        let empty = file(Vec::new());
        assert_eq!(Dt1::parse(&write(&empty)).unwrap(), empty);
    }

    /// Reference vector: an iso block of bytes 0..255 encodes back to
    /// the same 256 bytes.
    #[test]
    fn iso_vector() {
        let b = iso_block(0);
        assert_eq!(encode_iso(&b.pixels), (0..=255u8).collect::<Vec<_>>());
    }

    /// Reference vector: row 0 x2..4 = a b c, row 1 x1 = d encodes as
    /// `02 03 a b c 00 00 01 01 d 00 00`.
    #[test]
    fn rle_vector() {
        let mut p = vec![0u8; RLE_WIDTH * RLE_HEIGHT];
        p[2..5].copy_from_slice(&[0xa, 0xb, 0xc]);
        p[RLE_WIDTH + 1] = 0xd;
        assert_eq!(encode_rle(&p), [2, 3, 0xa, 0xb, 0xc, 0, 0, 1, 1, 0xd, 0, 0]);
    }

    #[test]
    fn header_fields() {
        let bytes = write(&sample());
        assert_eq!(&bytes[0..8], &[7, 0, 0, 0, 6, 0, 0, 0]);
        assert_eq!(&bytes[268..276], &[3, 0, 0, 0, 0x14, 1, 0, 0]);
    }
}
