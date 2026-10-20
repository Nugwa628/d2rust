//! Byte counts charged to the "parsed files" pool: the parser
//! output's size estimate, `size_of` the value plus the `len` of its owned
//! buffers. An estimate for budgeting only; it never affects the image.

use std::mem::{size_of, size_of_val};

use d2_formats::cof::Cof;
use d2_formats::dc6::Dc6;
use d2_formats::dcc::Dcc;
use d2_formats::ds1::Ds1;
use d2_formats::dt1::Dt1;
use d2_formats::font::FontTable;
use d2_formats::palette::{Palette, Pl2};
use d2_formats::tbl::StringTable;

use super::tbl::TblAsset;

/// Estimated bytes held by a parsed asset.
pub trait ByteSize {
    fn byte_size(&self) -> u64;
}

fn vec_bytes<T>(v: &[T]) -> u64 {
    size_of_val(v) as u64
}

fn maps(v: &[[u8; 256]]) -> u64 {
    vec_bytes(v)
}

impl ByteSize for Palette {
    fn byte_size(&self) -> u64 {
        size_of::<Palette>() as u64
    }
}

impl ByteSize for Pl2 {
    fn byte_size(&self) -> u64 {
        size_of::<Pl2>() as u64
            + maps(&self.light_levels)
            + maps(&self.inventory_variations)
            + self.alpha_blend.iter().map(|l| maps(l)).sum::<u64>()
            + vec_bytes(&self.alpha_blend)
            + maps(&self.additive_blend)
            + maps(&self.multiplicative_blend)
            + maps(&self.hue_variations)
            + maps(&self.unknown_variations)
            + maps(&self.max_component_blend)
            + vec_bytes(&self.text_colors)
            + maps(&self.text_color_shifts)
    }
}

impl ByteSize for Cof {
    fn byte_size(&self) -> u64 {
        size_of::<Cof>() as u64
            + vec_bytes(&self.layers)
            + vec_bytes(&self.events)
            + vec_bytes(&self.event_padding)
            + vec_bytes(&self.draw_order)
    }
}

impl ByteSize for FontTable {
    fn byte_size(&self) -> u64 {
        size_of::<FontTable>() as u64 + vec_bytes(&self.glyphs)
    }
}

impl ByteSize for StringTable {
    fn byte_size(&self) -> u64 {
        size_of::<StringTable>() as u64
            + vec_bytes(&self.indices)
            + vec_bytes(&self.entries)
            + self
                .entries
                .iter()
                .map(|e| (e.key.len() + e.value.len()) as u64)
                .sum::<u64>()
    }
}

impl ByteSize for TblAsset {
    fn byte_size(&self) -> u64 {
        match self {
            TblAsset::Font(f) => f.byte_size(),
            TblAsset::Strings(s) => s.byte_size(),
        }
    }
}

impl ByteSize for Dc6 {
    fn byte_size(&self) -> u64 {
        size_of::<Dc6>() as u64
            + vec_bytes(&self.frames)
            + self
                .frames
                .iter()
                .map(|f| f.pixels.len() as u64)
                .sum::<u64>()
    }
}

impl ByteSize for Dcc {
    fn byte_size(&self) -> u64 {
        size_of::<Dcc>() as u64
            + vec_bytes(&self.directions)
            + self
                .directions
                .iter()
                .map(|d| {
                    vec_bytes(&d.frames)
                        + d.frames
                            .iter()
                            .map(|f| (f.pixels.len() + f.optional_data.len()) as u64)
                            .sum::<u64>()
                })
                .sum::<u64>()
    }
}

impl ByteSize for Dt1 {
    fn byte_size(&self) -> u64 {
        size_of::<Dt1>() as u64
            + vec_bytes(&self.tiles)
            + self
                .tiles
                .iter()
                .map(|t| {
                    vec_bytes(&t.blocks)
                        + t.blocks.iter().map(|b| b.pixels.len() as u64).sum::<u64>()
                })
                .sum::<u64>()
    }
}

impl ByteSize for Ds1 {
    fn byte_size(&self) -> u64 {
        let layers = |v: &[Vec<u32>]| vec_bytes(v) + v.iter().map(|l| vec_bytes(l)).sum::<u64>();
        size_of::<Ds1>() as u64
            + vec_bytes(&self.files)
            + self.files.iter().map(|f| f.len() as u64).sum::<u64>()
            + layers(&self.walls)
            + layers(&self.orientations)
            + layers(&self.floors)
            + vec_bytes(&self.shadow)
            + self.tags.as_deref().map_or(0, vec_bytes)
            + vec_bytes(&self.objects)
            + vec_bytes(&self.groups)
            + vec_bytes(&self.paths)
            + self.paths.iter().map(|p| vec_bytes(&p.points)).sum::<u64>()
            + self.trailing.len() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2_formats::font::Glyph;

    #[test]
    fn font_size_grows_with_glyphs() {
        let mut v = b"Woo!".to_vec();
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&[0; 6]);
        let empty = FontTable::parse(&v).unwrap().byte_size();
        v.extend_from_slice(&[0; 14 * 3]);
        let three = FontTable::parse(&v).unwrap().byte_size();
        assert_eq!(three - empty, 3 * size_of::<Glyph>() as u64);
    }
}
