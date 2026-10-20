//! Font tables: per-character metrics for a font's DC6 glyph sheet.
//!
//! Each game font is a pair of files: a DC6 sprite holding one frame per
//! glyph, and a `.tbl` font table describing the glyphs. [`FontTable::parse`]
//! takes the bytes of the table from the player's game data. Text rendering in
//! D2RUST looks up a character code here to find its glyph frame and size in
//! the DC6 sheet.
//!
//! Layout (little-endian): a 12-byte header, then 14-byte glyph records up to
//! the end of the file.
//!
//! | offset | size | field                         |
//! |--------|------|-------------------------------|
//! | 0x00   | 4    | magic `Woo!`                  |
//! | 0x04   | 2    | version, always 1             |
//! | 0x06   | 2    | unknown                       |
//! | 0x08   | 2    | glyph count                   |
//! | 0x0A   | 1    | font height                   |
//! | 0x0B   | 1    | font width                    |
//!
//! Glyph record: character code (u16), unknown byte, width, height, unknown
//! byte, unknown u16, DC6 frame index (u16), unknown u32.

use crate::cursor::{invalid, Cursor, FormatError};

const FORMAT: &str = "font tbl";
/// Four magic bytes at the start of a font table (`Woo!`).
pub const MAGIC: [u8; 4] = *b"Woo!";
const HEADER_LEN: usize = 12;
/// Size of one glyph record.
const GLYPH_LEN: usize = 14;

/// One glyph record (14 bytes) of a font table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    /// Character code the glyph is for.
    pub code: u16,
    /// Glyph field at +0x02, kept as read.
    pub unknown1: u8,
    /// Glyph width in pixels.
    pub width: u8,
    /// Glyph height in pixels.
    pub height: u8,
    /// Glyph field at +0x05, kept as read.
    pub unknown2: u8,
    /// Glyph field at +0x06, kept as read.
    pub unknown3: u16,
    /// Frame index in the font's DC6 (a u16).
    pub frame: u16,
    /// Glyph field at +0x0A, kept as read.
    pub unknown5: u32,
}

/// A parsed font table: header plus glyph records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontTable {
    /// Format version; `parse` accepts only 1.
    pub version: u16,
    /// Header field at +0x06, kept as read.
    pub unknown: u16,
    /// Glyph count from the header: how many records a by-code glyph lookup
    /// searches.
    pub count: u16,
    /// Font height from the header.
    pub height: u8,
    /// Font width from the header.
    pub width: u8,
    /// The glyph records, in file order.
    pub glyphs: Vec<Glyph>,
}

impl FontTable {
    /// True if `data` starts with the font-table magic. String tables share
    /// the `.tbl` extension but not the magic.
    pub fn is_font_table(data: &[u8]) -> bool {
        data.starts_with(&MAGIC)
    }

    /// Parses a font table. Fails on a missing magic, a wrong version or truncated
    /// records.
    pub fn parse(data: &[u8]) -> Result<FontTable, FormatError> {
        let mut c = Cursor::new(data, FORMAT);
        if c.bytes(4)? != MAGIC {
            return Err(invalid(FORMAT, "missing \"Woo!\" magic"));
        }
        let version = c.u16()?;
        if version != 1 {
            return Err(invalid(FORMAT, format!("version {version}, expected 1")));
        }
        let unknown = c.u16()?;
        let count = c.u16()?;
        let height = c.u8()?;
        let width = c.u8()?;

        // The header reads above succeeded, so the file is at least
        // HEADER_LEN bytes; the records fill the rest.
        let body = data.len() - HEADER_LEN;
        if !body.is_multiple_of(GLYPH_LEN) {
            return Err(invalid(
                FORMAT,
                format!("{body} bytes of records is not a multiple of {GLYPH_LEN}"),
            ));
        }
        let glyphs = (0..body / GLYPH_LEN)
            .map(|_| {
                Ok(Glyph {
                    code: c.u16()?,
                    unknown1: c.u8()?,
                    width: c.u8()?,
                    height: c.u8()?,
                    unknown2: c.u8()?,
                    unknown3: c.u16()?,
                    frame: c.u16()?,
                    unknown5: c.u32()?,
                })
            })
            .collect::<Result<Vec<_>, FormatError>>()?;
        Ok(FontTable {
            version,
            unknown,
            count,
            height,
            width,
            glyphs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(records: &[[u8; 14]]) -> Vec<u8> {
        let mut d = b"Woo!".to_vec();
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&[0, 0, 0, 0, 16, 12]);
        for r in records {
            d.extend_from_slice(r);
        }
        d
    }

    #[test]
    fn parses_records() {
        let a = [b'A', 0, 0, 7, 16, 1, 0, 0, 33, 0, 0, 0, 0, 0];
        let b = [0x10, 0xAC, 0, 9, 16, 1, 0, 0, 200, 0, 0, 0, 0, 0];
        let f = FontTable::parse(&file(&[a, b])).unwrap();
        assert_eq!((f.height, f.width), (16, 12));
        assert_eq!(f.glyphs.len(), 2);
        assert_eq!(f.glyphs[0].code, u16::from(b'A'));
        assert_eq!(f.glyphs[0].width, 7);
        assert_eq!(f.glyphs[0].frame, 33);
        assert_eq!(f.glyphs[1].code, 0xAC10);
        assert_eq!(f.glyphs[1].frame, 200);
    }

    #[test]
    fn errors() {
        let mut bad = file(&[]);
        bad[0] = b'X';
        assert!(FontTable::parse(&bad).is_err(), "magic");
        let mut partial = file(&[]);
        partial.extend_from_slice(&[0; 15]);
        assert!(FontTable::parse(&partial).is_err(), "partial record");
        assert!(FontTable::is_font_table(&file(&[])));
    }

    #[test]
    fn integers_are_little_endian() {
        let mut f = file(&[[
            0x34, 0x12, 0, 0, 0, 0, 0xCD, 0xAB, 0, 0, 0x78, 0x56, 0x34, 0x12,
        ]]);
        f[4..6].copy_from_slice(&[1, 0]); // version 1 as LE u16
        let t = FontTable::parse(&f).unwrap();
        assert_eq!(t.version, 1);
        let g = t.glyphs[0];
        assert_eq!(
            (g.code, g.unknown3, g.unknown5),
            (0x1234, 0xABCD, 0x1234_5678)
        );
        // Big-endian version 1 (0x0100) is not version 1.
        f[4..6].copy_from_slice(&[0, 1]);
        assert!(FontTable::parse(&f).is_err());
    }

    #[test]
    fn frame_is_a_u16_and_count_is_header_byte_8() {
        // `frame` is the u16 at record byte 8: byte 9 is its high byte, so
        // more than 256 frames are addressable.
        let recs: Vec<[u8; 14]> = (0..300u16)
            .map(|i| {
                let mut r = [0u8; 14];
                r[..2].copy_from_slice(&i.to_le_bytes());
                r[8..10].copy_from_slice(&i.to_le_bytes());
                r
            })
            .collect();
        let mut d = file(&recs);
        d[6..10].copy_from_slice(&[0x02, 0x01, 0x2C, 0x01]);
        let t = FontTable::parse(&d).unwrap();
        assert_eq!((t.unknown, t.count), (0x0102, 300));
        assert_eq!(t.glyphs.len(), 300);
        assert_eq!(t.glyphs[255].frame, 255);
        assert_eq!(t.glyphs[256].frame, 256);
        assert_eq!(t.glyphs[299].frame, 299);
    }

    mod robust {
        use super::*;
        use crate::robust::mutated;
        use crate::robust_tests::{check, config};
        use proptest::prelude::*;

        fn valid() -> Vec<u8> {
            file(&[[b'A', 0, 0, 7, 16, 1, 0, 0, 33, 0, 0, 0, 0, 0]; 2])
        }

        #[test]
        fn builder_is_valid() {
            assert!(FontTable::parse(&valid()).is_ok());
        }

        proptest! {
            #![proptest_config(config(64))]

            #[test]
            fn mutated_file(data in mutated(valid())) {
                check(data, FontTable::parse);
            }
        }
    }
}
