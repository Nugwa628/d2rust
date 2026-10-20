//! Font tables (the `.tbl` files under `data/local/font/`) as `P.toml`.
//!
//! A font table pairs a sprite sheet of glyph pictures with the metrics
//! the text renderer needs: the line height and widest cell, and for every
//! character code its advance width, height and the sheet frame that draws
//! it. The sidecar holds the header fields and one `[[glyph]]` table per
//! glyph, in file order.

use d2_formats::font::{FontTable, Glyph};

use super::text::{check_round_trip, Out, Tab, TextError};

/// Writes a font-table sidecar. Every font table can be written.
pub fn write_font(_file: &str, f: &FontTable) -> Result<String, TextError> {
    let mut o = Out::header("font");
    o.int("version", f.version);
    o.int("unknown", f.unknown);
    o.int("count", f.count);
    o.int("height", f.height);
    o.int("width", f.width);
    for g in &f.glyphs {
        o.table("[[glyph]]");
        o.int("code", g.code);
        o.int("unknown1", g.unknown1);
        o.int("width", g.width);
        o.int("height", g.height);
        o.int("unknown2", g.unknown2);
        o.int("unknown3", g.unknown3);
        o.int("frame", g.frame);
        o.int("unknown5", g.unknown5);
    }
    Ok(o.0)
}

/// Reads a font-table sidecar written by [`write_font`].
pub fn read_font(file: &str, text: &str) -> Result<FontTable, TextError> {
    let mut t = Tab::parse(file, text, "font")?;
    let version = t.u16("version")?;
    let unknown = t.u16("unknown")?;
    let count = t.u16("count")?;
    let height = t.u8("height")?;
    let width = t.u8("width")?;
    let mut glyphs = Vec::new();
    for mut g in t.tables("glyph")? {
        glyphs.push(Glyph {
            code: g.u16("code")?,
            unknown1: g.u8("unknown1")?,
            width: g.u8("width")?,
            height: g.u8("height")?,
            unknown2: g.u8("unknown2")?,
            unknown3: g.u16("unknown3")?,
            frame: g.u16("frame")?,
            unknown5: g.u32("unknown5")?,
        });
        g.finish()?;
    }
    t.finish()?;
    Ok(FontTable {
        version,
        unknown,
        count,
        height,
        width,
        glyphs,
    })
}

/// Round-trip check for font tables: write, read back, compare.
pub fn check_font(file: &str, original: &FontTable) -> Result<(), TextError> {
    check_round_trip(file, original, write_font, read_font)
}
