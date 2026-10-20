//! `.dat` palettes and `.pl2` palette transforms.
//!
//! Every sprite and tile pixel is an index into a 256-color palette; each act
//! has its own. [`Palette::parse`] reads a `.dat` palette: 256 colors of three
//! bytes, stored blue, green, red. [`Pl2::parse`] reads the matching `.pl2`,
//! which holds the act's on-screen palette and the 256-entry index-to-index
//! maps the renderer uses for lighting, item color variations, translucent
//! blends and text colors. Renderers in D2RUST turn palette indices from
//! DC6/DCC/DT1 pixels into colors, and use the `.pl2` maps for lighting and
//! blends.
//!
//! A `.pl2` is, in order:
//!
//! | maps × 256 bytes | field                                  |
//! |------------------|----------------------------------------|
//! | (1,024 bytes)    | base palette, 256 × (R, G, B, unused)  |
//! | 32               | light levels                           |
//! | 16               | inventory color variations             |
//! | 1                | selected-unit shift                    |
//! | 3 × 256          | alpha blends: level, destination index |
//! | 256              | additive blends                        |
//! | 256              | multiplicative blends                  |
//! | 111              | hue variations                         |
//! | 1 + 1 + 1        | red, green and blue tones              |
//! | 14               | unknown variations                     |
//! | 256              | maximum-component blends               |
//! | 1                | darkened shift                         |
//!
//! then any number of text colors: all their (R, G, B) triples first, then one
//! shift map per text color.

use crate::cursor::{invalid, FormatError};

const FORMAT_DAT: &str = "palette";
const FORMAT_PL2: &str = "pl2";

/// One palette color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    /// Red, 0 to 255.
    pub r: u8,
    /// Green, 0 to 255.
    pub g: u8,
    /// Blue, 0 to 255.
    pub b: u8,
}

/// A 256-color palette (`.dat`). Index 0 is drawn transparent in sprites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    /// The 256 colors, indexed by palette index.
    pub colors: [Rgb; 256],
}

impl Palette {
    /// Parses a 768-byte `.dat` palette stored as blue, green, red.
    pub fn parse(data: &[u8]) -> Result<Palette, FormatError> {
        if data.len() != 768 {
            return Err(invalid(
                FORMAT_DAT,
                format!("expected 768 bytes, got {}", data.len()),
            ));
        }
        let mut colors = [Rgb::default(); 256];
        for (c, bgr) in colors.iter_mut().zip(data.as_chunks::<3>().0) {
            *c = Rgb {
                r: bgr[2],
                g: bgr[1],
                b: bgr[0],
            };
        }
        Ok(Palette { colors })
    }
}

/// A palette index → palette index lookup table.
pub type ColorMap = [u8; 256];

/// Size of the fixed part of a `.pl2`: base palette + 1714 maps.
const PL2_FIXED: usize = 1024 + 1714 * 256;
/// Bytes per text color: 3 (rgb) + one 256-byte shift map.
const PL2_TEXT_COLOR: usize = 3 + 256;

/// Palette transform tables (`.pl2`). Field meanings for rendering are
/// used by the renderer; this type only exposes the layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pl2 {
    /// The first 1,024 bytes: 256 × (R, G, B, x), the 4th byte not a
    /// color. This is the palette shown on screen.
    pub base_palette: Palette,
    /// Light-level maps.
    pub light_levels: Vec<ColorMap>,
    /// Inventory color-variation maps.
    pub inventory_variations: Vec<ColorMap>,
    /// Shift map for a selected unit.
    pub selected_unit_shift: ColorMap,
    /// `[level][destination index]`, each a map over the source index
    /// (row = destination, column = source).
    pub alpha_blend: Vec<Vec<ColorMap>>,
    /// Additive-blend maps.
    pub additive_blend: Vec<ColorMap>,
    /// Multiplicative-blend maps.
    pub multiplicative_blend: Vec<ColorMap>,
    /// Hue-variation maps.
    pub hue_variations: Vec<ColorMap>,
    /// Red-tone map.
    pub red_tones: ColorMap,
    /// Green-tone map.
    pub green_tones: ColorMap,
    /// Blue-tone map.
    pub blue_tones: ColorMap,
    /// Maps in the unknown-variations block.
    pub unknown_variations: Vec<ColorMap>,
    /// Maximum-component blend maps.
    pub max_component_blend: Vec<ColorMap>,
    /// Darkened-shift map.
    pub darkened_shift: ColorMap,
    /// Text colors, one per text-color entry.
    pub text_colors: Vec<Rgb>,
    /// Shift maps for the text colors, in the same order as `text_colors`.
    pub text_color_shifts: Vec<ColorMap>,
}

/// Sequential reader of 256-byte maps; the caller has checked the length.
struct Maps<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Maps<'_> {
    fn one(&mut self) -> ColorMap {
        let mut m = [0u8; 256];
        m.copy_from_slice(&self.data[self.pos..self.pos + 256]);
        self.pos += 256;
        m
    }
    fn many(&mut self, n: usize) -> Vec<ColorMap> {
        (0..n).map(|_| self.one()).collect()
    }
}

impl Pl2 {
    /// Parses a `.pl2` file. Fails if it is shorter than the fixed part or the tail is
    /// not a whole number of text colors.
    pub fn parse(data: &[u8]) -> Result<Pl2, FormatError> {
        if data.len() < PL2_FIXED {
            return Err(invalid(
                FORMAT_PL2,
                format!("expected at least {PL2_FIXED} bytes, got {}", data.len()),
            ));
        }
        let rest = data.len() - PL2_FIXED;
        if !rest.is_multiple_of(PL2_TEXT_COLOR) {
            return Err(invalid(
                FORMAT_PL2,
                format!("{rest} trailing bytes is not a whole number of text colors"),
            ));
        }
        let text_count = rest / PL2_TEXT_COLOR;

        let mut base_palette = Palette {
            colors: [Rgb::default(); 256],
        };
        for (c, e) in base_palette
            .colors
            .iter_mut()
            .zip(data[..1024].as_chunks::<4>().0)
        {
            *c = Rgb {
                r: e[0],
                g: e[1],
                b: e[2],
            };
        }
        let mut m = Maps { data, pos: 1024 };
        let light_levels = m.many(32);
        let inventory_variations = m.many(16);
        let selected_unit_shift = m.one();
        let alpha_blend = (0..3).map(|_| m.many(256)).collect();
        let additive_blend = m.many(256);
        let multiplicative_blend = m.many(256);
        let hue_variations = m.many(111);
        let red_tones = m.one();
        let green_tones = m.one();
        let blue_tones = m.one();
        let unknown_variations = m.many(14);
        let max_component_blend = m.many(256);
        let darkened_shift = m.one();
        debug_assert_eq!(m.pos, PL2_FIXED);

        let text_colors = data[PL2_FIXED..PL2_FIXED + 3 * text_count]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| Rgb {
                r: c[0],
                g: c[1],
                b: c[2],
            })
            .collect();
        let mut shifts = Maps {
            data,
            pos: PL2_FIXED + 3 * text_count,
        };
        let text_color_shifts = shifts.many(text_count);

        Ok(Pl2 {
            base_palette,
            light_levels,
            inventory_variations,
            selected_unit_shift,
            alpha_blend,
            additive_blend,
            multiplicative_blend,
            hue_variations,
            red_tones,
            green_tones,
            blue_tones,
            unknown_variations,
            max_component_blend,
            darkened_shift,
            text_colors,
            text_color_shifts,
        })
    }
}

impl Pl2 {
    /// The file bytes of this table (the inverse of [`Pl2::parse`]), for a
    /// reader that wants the flat layout from a decoded table. The base
    /// palette's fourth byte per color is not kept by `parse` and is
    /// written as 0; nothing in D2RUST reads it.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PL2_FIXED + self.text_colors.len() * PL2_TEXT_COLOR);
        for c in &self.base_palette.colors {
            out.extend_from_slice(&[c.r, c.g, c.b, 0]);
        }
        let maps = |out: &mut Vec<u8>, v: &[ColorMap]| v.iter().for_each(|m| out.extend(m));
        maps(&mut out, &self.light_levels);
        maps(&mut out, &self.inventory_variations);
        out.extend(self.selected_unit_shift);
        for level in &self.alpha_blend {
            maps(&mut out, level);
        }
        maps(&mut out, &self.additive_blend);
        maps(&mut out, &self.multiplicative_blend);
        maps(&mut out, &self.hue_variations);
        out.extend(self.red_tones);
        out.extend(self.green_tones);
        out.extend(self.blue_tones);
        maps(&mut out, &self.unknown_variations);
        maps(&mut out, &self.max_component_blend);
        out.extend(self.darkened_shift);
        for c in &self.text_colors {
            out.extend_from_slice(&[c.r, c.g, c.b]);
        }
        maps(&mut out, &self.text_color_shifts);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dat_is_bgr() {
        let mut data = vec![0u8; 768];
        data[3..6].copy_from_slice(&[10, 20, 30]);
        let p = Palette::parse(&data).unwrap();
        assert_eq!(
            p.colors[1],
            Rgb {
                r: 30,
                g: 20,
                b: 10
            }
        );
    }

    #[test]
    fn dat_length_is_checked() {
        assert!(Palette::parse(&[0; 767]).is_err());
        assert!(Palette::parse(&[0; 769]).is_err());
    }

    fn pl2(text_colors: usize, extra: usize) -> Vec<u8> {
        let mut d = vec![0u8; PL2_FIXED + text_colors * PL2_TEXT_COLOR + extra];
        // Mark the first light-level map and the last fixed map.
        d[1024] = 0xAA;
        d[PL2_FIXED - 256] = 0xBB;
        if text_colors > 0 {
            d[PL2_FIXED..PL2_FIXED + 3].copy_from_slice(&[1, 2, 3]);
        }
        d
    }

    #[test]
    fn pl2_layout() {
        let p = Pl2::parse(&pl2(13, 0)).unwrap();
        assert_eq!(p.text_colors.len(), 13);
        assert_eq!(p.text_colors[0], Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(p.light_levels[0][0], 0xAA);
        assert_eq!(p.darkened_shift[0], 0xBB);
        assert_eq!(p.alpha_blend.len(), 3);
        assert_eq!(p.alpha_blend[2].len(), 256);
        assert_eq!(p.hue_variations.len(), 111);
    }

    #[test]
    fn pl2_base_palette_is_rgbx_and_blend_rows_are_destinations() {
        let mut d = pl2(0, 0);
        d[..8].copy_from_slice(&[1, 2, 3, 0xEE, 0x10, 0x20, 0x30, 0xFF]);
        // Alpha level 1, destination 7, source 9.
        let at = 1024 + (32 + 16 + 1) * 256 + 256 * 256 + 7 * 256 + 9;
        d[at] = 0x5C;
        let p = Pl2::parse(&d).unwrap();
        assert_eq!(p.base_palette.colors[0], Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(
            p.base_palette.colors[1],
            Rgb {
                r: 0x10,
                g: 0x20,
                b: 0x30
            }
        );
        assert_eq!(p.alpha_blend[1][7][9], 0x5C);
    }

    #[test]
    fn pl2_twelve_text_colors() {
        assert_eq!(Pl2::parse(&pl2(12, 0)).unwrap().text_colors.len(), 12);
    }

    #[test]
    fn pl2_bad_tail() {
        assert!(Pl2::parse(&pl2(0, 100)).is_err());
        assert!(Pl2::parse(&[0; 1000]).is_err());
    }

    mod robust {
        use super::*;
        use crate::robust::mutated;
        use crate::robust_tests::{check, config};
        use proptest::prelude::*;

        #[test]
        fn builders_are_valid() {
            assert!(Palette::parse(&[7; 768]).is_ok());
            assert!(Pl2::parse(&pl2(13, 0)).is_ok());
        }

        proptest! {
            #![proptest_config(config(32))]

            #[test]
            fn mutated_palette(data in mutated(vec![7; 768])) {
                check(data, Palette::parse);
            }

            #[test]
            fn mutated_pl2(data in mutated(pl2(13, 0))) {
                check(data, Pl2::parse);
            }
        }
    }
}
