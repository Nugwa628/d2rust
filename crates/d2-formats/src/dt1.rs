//! DT1 tile libraries: tile metadata and block pixels.
//!
//! [`Dt1::parse`] takes the bytes of a `.dt1` file (floor, wall and roof tiles from
//! the player's game data) and returns each tile's header fields with its blocks
//! decoded to palette indices. Map rendering in D2RUST places tiles from DS1 map
//! presets using these libraries.

use crate::cursor::{invalid, Cursor, FormatError};

const FORMAT: &str = "dt1";
const BLOCK_HEADER_LEN: usize = 20;
/// Block `format` value for isometric blocks; any other value is an RLE block.
pub const ISO_FORMAT: u16 = 1;
/// Width in pixels of an isometric block.
pub const ISO_WIDTH: usize = 32;
/// Height in pixels of an isometric block.
pub const ISO_HEIGHT: usize = 15;
/// Width in pixels of an RLE block.
pub const RLE_WIDTH: usize = 32;
/// Height in pixels of an RLE block.
pub const RLE_HEIGHT: usize = 32;

const ISO_SKIP: [usize; 15] = [14, 12, 10, 8, 6, 4, 2, 0, 2, 4, 6, 8, 10, 12, 14];
const ISO_RUN: [usize; 15] = [4, 8, 12, 16, 20, 24, 28, 32, 28, 24, 20, 16, 12, 8, 4];

/// One block of a tile: placement, format and decoded pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dt1Block {
    /// Horizontal position of the block within the tile.
    pub x: i16,
    /// Vertical position of the block within the tile.
    pub y: i16,
    /// Block header field at +0x04, kept as read.
    pub unknown1: u16,
    /// Grid column of the block.
    pub grid_x: u8,
    /// Grid row of the block.
    pub grid_y: u8,
    /// Block format; [`ISO_FORMAT`] means isometric, anything else is RLE.
    pub format: u16,
    /// Block header field at +0x0E, kept as read.
    pub unknown2: u16,
    /// Decoded pixels: 32×15 for isometric blocks, 32×32 for RLE blocks,
    /// row-major, 0 = transparent.
    pub pixels: Vec<u8>,
}

impl Dt1Block {
    /// True if this block uses the isometric format.
    pub fn is_iso(&self) -> bool {
        self.format == ISO_FORMAT
    }
    /// Pixel size `(width, height)` of the decoded block.
    pub fn size(&self) -> (usize, usize) {
        if self.is_iso() {
            (ISO_WIDTH, ISO_HEIGHT)
        } else {
            (RLE_WIDTH, RLE_HEIGHT)
        }
    }
}

/// One tile: its header fields, sub-tile flags and blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dt1Tile {
    /// Light direction field of the tile header.
    pub light_direction: u32,
    /// Roof height field of the tile header.
    pub roof_height: u16,
    /// Material flags of the tile.
    pub material_flags: u16,
    /// Tile height.
    pub height: i32,
    /// Tile width.
    pub width: i32,
    /// Tile header height-like field, kept as read.
    pub unknown_height: i32,
    /// Tile orientation.
    pub orientation: u32,
    /// Main index of the tile, used with `sub_index` and `orientation` to look tiles up.
    pub main_index: u32,
    /// Sub index of the tile.
    pub sub_index: u32,
    /// Rarity weight of the tile.
    pub rarity: u32,
    /// Tile header color-like field, kept as read.
    pub unknown_color: u32,
    /// 5×5 sub-tile flags, in file order.
    pub subtile_flags: [u8; 25],
    /// Tile header field at +0x58, kept as read.
    pub unknown_58: u16,
    /// Cache index field of the tile header.
    pub cache_index: u16,
    /// Tile header field at +0x5C, kept as read.
    pub unknown_5c: u32,
    /// The tile's blocks, in file order.
    pub blocks: Vec<Dt1Block>,
}

/// A parsed DT1 tile library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dt1 {
    /// Major version; `parse` accepts only 7.
    pub version: i32,
    /// Minor version word, kept as read.
    pub minor_version: i32,
    /// All tiles, in file order.
    pub tiles: Vec<Dt1Tile>,
}

impl Dt1 {
    /// Parses a DT1 file, decoding every tile and block. Fails on a wrong version,
    /// a tile count that cannot fit, or truncated or malformed block data.
    pub fn parse(data: &[u8]) -> Result<Dt1, FormatError> {
        let mut c = Cursor::new(data, FORMAT);
        let version = c.i32()?;
        if version != 7 {
            return Err(invalid(FORMAT, format!("version {version}, expected 7")));
        }
        let minor_version = c.i32()?;
        c.bytes(260)?;
        let count = c.u32()? as usize;
        let first = c.u32()? as usize;
        if count.saturating_mul(96) > data.len() {
            return Err(invalid(
                FORMAT,
                format!("{count} tiles cannot fit in the file"),
            ));
        }
        let mut c = Cursor::at(data, first, FORMAT);
        let mut tiles = Vec::with_capacity(count);
        // Blocks the file has room for, over all tiles: one 20-byte header
        // each. Tiles whose block headers overlap could otherwise decode
        // the same headers once per tile (quadratic in the file size).
        let mut block_budget = data.len() / BLOCK_HEADER_LEN;
        for t in 0..count {
            tiles.push(
                read_tile(data, &mut c, &mut block_budget)
                    .map_err(|e| invalid(FORMAT, format!("tile {t}: {e}")))?,
            );
        }
        Ok(Dt1 {
            version,
            minor_version,
            tiles,
        })
    }
}

fn read_tile(
    data: &[u8],
    c: &mut Cursor<'_>,
    block_budget: &mut usize,
) -> Result<Dt1Tile, FormatError> {
    let light_direction = c.u32()?;
    let roof_height = c.u16()?;
    let material_flags = c.u16()?;
    let height = c.i32()?;
    let width = c.i32()?;
    let unknown_height = c.i32()?;
    let orientation = c.u32()?;
    let main_index = c.u32()?;
    let sub_index = c.u32()?;
    let rarity = c.u32()?;
    let unknown_color = c.u32()?;
    let mut subtile_flags = [0u8; 25];
    subtile_flags.copy_from_slice(c.bytes(25)?);
    c.bytes(7)?;
    let blocks_offset = c.u32()? as usize;
    let _blocks_length = c.u32()?;
    let block_count = c.u32()? as usize;
    c.bytes(4)?;
    let unknown_58 = c.u16()?;
    let cache_index = c.u16()?;
    let unknown_5c = c.u32()?;

    *block_budget = block_budget
        .checked_sub(block_count)
        .ok_or_else(|| invalid(FORMAT, format!("{block_count} blocks cannot fit")))?;
    let mut bc = Cursor::at(data, blocks_offset, FORMAT);
    let mut blocks = Vec::with_capacity(block_count);
    for b in 0..block_count {
        let x = bc.u16()? as i16;
        let y = bc.u16()? as i16;
        let unknown1 = bc.u16()?;
        let grid_x = bc.u8()?;
        let grid_y = bc.u8()?;
        let format = bc.u16()?;
        let length = bc.u32()? as usize;
        let unknown2 = bc.u16()?;
        let offset = bc.u32()? as usize;
        let encoded = blocks_offset
            .checked_add(offset)
            .and_then(|start| data.get(start..start.checked_add(length)?))
            .ok_or_else(|| invalid(FORMAT, format!("block {b}: data outside the file")))?;
        let pixels = if format == ISO_FORMAT {
            decode_iso(encoded)
        } else {
            decode_rle(encoded)
        }
        .map_err(|e| invalid(FORMAT, format!("block {b}: {e}")))?;
        blocks.push(Dt1Block {
            x,
            y,
            unknown1,
            grid_x,
            grid_y,
            format,
            unknown2,
            pixels,
        });
    }
    Ok(Dt1Tile {
        light_direction,
        roof_height,
        material_flags,
        height,
        width,
        unknown_height,
        orientation,
        main_index,
        sub_index,
        rarity,
        unknown_color,
        subtile_flags,
        unknown_58,
        cache_index,
        unknown_5c,
        blocks,
    })
}

/// Isometric block: 15 rows of the diamond, 256 bytes.
fn decode_iso(encoded: &[u8]) -> Result<Vec<u8>, FormatError> {
    if encoded.len() != 256 {
        return Err(invalid(
            FORMAT,
            format!("isometric block is {} bytes, expected 256", encoded.len()),
        ));
    }
    let mut pixels = vec![0u8; ISO_WIDTH * ISO_HEIGHT];
    let mut i = 0;
    for (row, (&skip, &run)) in ISO_SKIP.iter().zip(&ISO_RUN).enumerate() {
        let at = row * ISO_WIDTH + skip;
        pixels[at..at + run].copy_from_slice(&encoded[i..i + run]);
        i += run;
    }
    Ok(pixels)
}

/// RLE block: (skip, count) pairs, (0, 0) = next row.
fn decode_rle(encoded: &[u8]) -> Result<Vec<u8>, FormatError> {
    let mut pixels = vec![0u8; RLE_WIDTH * RLE_HEIGHT];
    let (mut x, mut row, mut i) = (0usize, 0usize, 0usize);
    while i < encoded.len() {
        let skip = usize::from(encoded[i]);
        let count = usize::from(
            *encoded
                .get(i + 1)
                .ok_or_else(|| invalid(FORMAT, "truncated RLE pair"))?,
        );
        i += 2;
        if skip == 0 && count == 0 {
            x = 0;
            row += 1;
            continue;
        }
        x += skip;
        let run = encoded
            .get(i..i + count)
            .ok_or_else(|| invalid(FORMAT, "RLE run past end of data"))?;
        i += count;
        if count == 0 {
            // A skip with no pixels writes nothing, even past the block.
            continue;
        }
        if row >= RLE_HEIGHT || x + count > RLE_WIDTH {
            return Err(invalid(
                FORMAT,
                format!("RLE pixels outside the block at row {row}, x {x}"),
            ));
        }
        if count > 0 {
            pixels[row * RLE_WIDTH + x..row * RLE_WIDTH + x + count].copy_from_slice(run);
        }
        x += count;
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_vector() {
        let encoded: Vec<u8> = (0..=255).collect();
        let px = decode_iso(&encoded).unwrap();
        assert_eq!(&px[14..18], &[0, 1, 2, 3]);
        assert_eq!(px[13], 0);
        assert_eq!(&px[7 * 32..8 * 32], &encoded[112..144]);
        assert_eq!(&px[14 * 32 + 14..14 * 32 + 18], &[252, 253, 254, 255]);
        assert!(decode_iso(&encoded[..255]).is_err());
    }

    #[test]
    fn rle_vector() {
        let px = decode_rle(&[2, 3, 10, 11, 12, 0, 0, 1, 1, 13]).unwrap();
        assert_eq!(&px[2..5], &[10, 11, 12]);
        assert_eq!(px[32 + 1], 13);
        assert!(decode_rle(&[31, 2, 1, 2]).is_err(), "row overflow");
        assert!(decode_rle(&[0, 2, 1]).is_err(), "run past data");
        assert!(decode_rle(&[1]).is_err(), "truncated pair");
    }

    // Found by cargo-fuzz (target dt1): a (skip, 0) pair that moves x or the
    // row past the block wrote an empty slice at an out-of-range start and
    // panicked. Synthetic input. An empty run writes no
    // pixel, so nothing is out of bounds.
    #[test]
    fn regress_rle_empty_run_past_block() {
        // Row 0, x 255 + 0 pixels: start 255 is inside the buffer.
        assert!(decode_rle(&[255, 0]).is_ok());
        // Row 31, x 255: start 31 * 32 + 255 is past the 1024-byte buffer.
        let mut past_x = [0u8, 0].repeat(31);
        past_x.extend_from_slice(&[255, 0]);
        assert_eq!(decode_rle(&past_x).unwrap(), vec![0u8; 1024]);
        // Row 33 (past the last row) with a skip and no pixels.
        let mut past_row = [0u8, 0].repeat(33);
        past_row.extend_from_slice(&[5, 0]);
        assert_eq!(decode_rle(&past_row).unwrap(), vec![0u8; 1024]);
        // A pixel written there is still an error.
        past_row.extend_from_slice(&[0, 1, 9]);
        assert!(decode_rle(&past_row).is_err());
    }

    fn file() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&7i32.to_le_bytes());
        d.extend_from_slice(&6i32.to_le_bytes());
        d.extend_from_slice(&[0; 260]);
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&276u32.to_le_bytes());
        // tile header at 276; blocks at 276 + 96 = 372
        let blocks_at = 372u32;
        let mut t = vec![0u8; 96];
        t[0x14..0x18].copy_from_slice(&3u32.to_le_bytes()); // orientation
        t[0x48..0x4C].copy_from_slice(&blocks_at.to_le_bytes());
        t[0x50..0x54].copy_from_slice(&1u32.to_le_bytes());
        d.extend(t);
        // one RLE block header; data right after it (offset 20)
        let rle = [1u8, 2, 7, 8, 0, 0];
        let mut b = Vec::new();
        b.extend_from_slice(&5i16.to_le_bytes());
        b.extend_from_slice(&(-3i16).to_le_bytes());
        b.extend_from_slice(&[0, 0, 1, 2]);
        b.extend_from_slice(&0x1001u16.to_le_bytes());
        b.extend_from_slice(&(rle.len() as u32).to_le_bytes());
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&20u32.to_le_bytes());
        d.extend(b);
        d.extend_from_slice(&rle);
        d
    }

    #[test]
    fn whole_file() {
        let dt1 = Dt1::parse(&file()).unwrap();
        assert_eq!(dt1.minor_version, 6);
        let t = &dt1.tiles[0];
        assert_eq!(t.orientation, 3);
        let b = &t.blocks[0];
        assert_eq!((b.x, b.y, b.grid_x, b.grid_y), (5, -3, 1, 2));
        assert!(!b.is_iso());
        assert_eq!(&b.pixels[1..3], &[7, 8]);
    }

    #[test]
    fn integers_are_little_endian() {
        let mut data = file();
        data[4..8].copy_from_slice(&[0x04, 0x03, 0x02, 0x01]); // minor version
        data[276..280].copy_from_slice(&[0x78, 0x56, 0x34, 0x12]); // light direction
        data[276 + 4..276 + 6].copy_from_slice(&[0x22, 0x11]); // roof height
        let dt1 = Dt1::parse(&data).unwrap();
        assert_eq!(dt1.minor_version, 0x0102_0304);
        let t = &dt1.tiles[0];
        assert_eq!(t.light_direction, 0x1234_5678);
        assert_eq!(t.roof_height, 0x1122);
        assert_eq!(t.blocks[0].format, 0x1001);
        assert_eq!(t.blocks[0].y, -3);
    }

    #[test]
    fn tile_without_blocks() {
        let mut data = file();
        data[276 + 0x50..276 + 0x54].copy_from_slice(&0u32.to_le_bytes());
        let dt1 = Dt1::parse(&data).unwrap();
        assert!(dt1.tiles[0].blocks.is_empty());
    }

    #[test]
    fn bad_block_offset() {
        let mut data = file();
        let at = data.len() - 6 - 4;
        data[at..at + 4].copy_from_slice(&9999u32.to_le_bytes());
        assert!(Dt1::parse(&data).is_err());
    }

    #[test]
    fn regress_tiles_sharing_block_headers() {
        // Four tiles all pointing at one table of 20 block headers: each
        // header was decoded once per tile (quadratic in the file size).
        // The 1,060-byte file has room for 53 headers, not 80.
        let (tiles, blocks) = (4usize, 20usize);
        let first = 276usize;
        let blocks_at = first + 96 * tiles;
        let mut d = Vec::new();
        d.extend_from_slice(&7i32.to_le_bytes());
        d.extend_from_slice(&6i32.to_le_bytes());
        d.extend_from_slice(&[0; 260]);
        d.extend_from_slice(&(tiles as u32).to_le_bytes());
        d.extend_from_slice(&(first as u32).to_le_bytes());
        for _ in 0..tiles {
            let mut t = vec![0u8; 96];
            t[0x48..0x4C].copy_from_slice(&(blocks_at as u32).to_le_bytes());
            t[0x50..0x54].copy_from_slice(&(blocks as u32).to_le_bytes());
            d.extend(t);
        }
        // Empty RLE blocks (length 0).
        d.extend(std::iter::repeat_n(0u8, 20 * blocks));
        let err = Dt1::parse(&d).unwrap_err();
        assert!(err.to_string().contains("cannot fit"), "{err}");
        // Two tiles fit (40 ≤ 53).
        d[268..272].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(Dt1::parse(&d).unwrap().tiles.len(), 2);
    }

    mod robust {
        use super::*;
        use crate::robust::mutated;
        use crate::robust_tests::{check, config};
        use proptest::prelude::*;

        #[test]
        fn builder_is_valid() {
            assert!(Dt1::parse(&file()).is_ok());
        }

        proptest! {
            #![proptest_config(config(64))]

            #[test]
            fn mutated_file(data in mutated(file())) {
                check(data, Dt1::parse);
            }
        }
    }
}
