//! `.tbl` assets: font tables and string tables share the extension; the "Woo!" magic at the
//! start of the file picks the parser. Loaded through the `AssetServer` like the other D2 formats
//! (see the parent `assets` module).

use bevy::prelude::*;
use d2_formats::font::FontTable;
use d2_formats::tbl::StringTable;
use d2_formats::FormatError;

/// A parsed `.tbl` file: a font table or a string table.
#[derive(Asset, TypePath, Debug)]
pub enum TblAsset {
    /// A font table (the file starts with the "Woo!" magic).
    Font(FontTable),
    /// A string table.
    Strings(StringTable),
}

impl TblAsset {
    /// A file with the font magic is a font; one that then fails to parse
    /// is an error, never retried as a string table.
    pub fn parse(data: &[u8]) -> Result<TblAsset, FormatError> {
        if FontTable::is_font_table(data) {
            FontTable::parse(data).map(TblAsset::Font)
        } else {
            StringTable::parse(data).map(TblAsset::Strings)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Font header (`Woo!`, version 1, 4 unknown bytes, height, width) and
    /// no glyphs.
    fn font() -> Vec<u8> {
        let mut v = b"Woo!".to_vec();
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&[0; 4]);
        v.extend_from_slice(&[16, 12]);
        v
    }

    /// Empty string table: a 21-byte header with no elements or slots.
    fn strings() -> Vec<u8> {
        vec![0; 21]
    }

    #[test]
    fn font_magic_gives_font() {
        // A fixed example.
        match TblAsset::parse(&font()).unwrap() {
            TblAsset::Font(f) => assert_eq!((f.height, f.width), (16, 12)),
            other => panic!("expected a font, got {other:?}"),
        }
    }

    #[test]
    fn no_magic_gives_strings() {
        // A fixed example.
        assert!(matches!(
            TblAsset::parse(&strings()).unwrap(),
            TblAsset::Strings(_)
        ));
    }

    #[test]
    fn broken_font_is_an_error_not_strings() {
        // Variation: version 2 breaks the font; the result is the
        // font parser's error, not a string-table fallback.
        let mut f = font();
        f[4] = 2;
        let err = TblAsset::parse(&f).unwrap_err();
        assert!(err.to_string().contains("version 2"), "{err}");
    }
}
