//! UI text: the font table, the Latin glyph lookup, color codes, the measuring
//! functions, the draw call and its variants, framed hover text, word wrap and
//! the clipping answer (decision CG2).
//!
//! [`layout_text`] splits the work in two: [`TextRules`] place the glyphs
//! ([`OriginalText`] is the one implementation), then each placed code unit is
//! resolved to its font record and DC6 frame ([`GlyphLookup::record`]). The pen
//! is the bottom row of the glyph cell; turning it into a sprite top-left is
//! the sprite placement rule (`world_view::text_sprites`).
//!
//! Strings stay UTF-16 code units as the string tables hold them; a
//! string ends at its first NUL unit or at the end of the slice.

use d2_formats::font::FontTable;

use super::draw::TextStyle;
use super::geom::Point;

/// Line feed.
pub const LF: u16 = 0x000A;
/// The color-code lead `ÿ`.
pub const COLOR_LEAD: u16 = 0x00FF;
/// Text color maps the PL2 loader copies; map 0 is never used.
pub const TEXT_COLORS: usize = 13;
/// File offset of text-color map 0 in an act `pal.pl2` (`0x6B627`).
pub const TEXT_COLOR_MAP_OFFSET: usize = 439_847;
/// The draw mode of every glyph of the draw call.
pub const TEXT_DRAW_MODE: u8 = 5;
/// Line factors in tenths of the font height, English (the draw step and
/// the text height).
const LINE_STEP_TENTHS: i32 = 16;
const TEXT_HEIGHT_TENTHS: i32 = 16;

/// One row of `text-fonts.tsv`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontInfo {
    pub id: u16,
    /// Name as in the font table.
    pub name: &'static str,
    /// Lowercase archive paths (`\` separators).
    pub tbl_path: &'static str,
    pub dc6_path: &'static str,
    /// Archive the DC6 is read from (in the archive search order).
    pub dc6_archive: &'static str,
    /// `.tbl` header byte 10.
    pub height: u8,
    /// `trunc(height × 16 / 10)`.
    pub line_step: i32,
    /// Size of every DC6 frame of the font.
    pub frame_w: u32,
    pub frame_h: u32,
}

macro_rules! font {
    ($id:literal, $name:literal, $file:literal, $arc:literal, $h:literal, $step:literal, $w:literal, $fh:literal) => {
        FontInfo {
            id: $id,
            name: $name,
            tbl_path: concat!(r"data\local\font\latin\", $file, ".tbl"),
            dc6_path: concat!(r"data\local\font\latin\", $file, ".dc6"),
            dc6_archive: $arc,
            height: $h,
            line_step: $step,
            frame_w: $w,
            frame_h: $fh,
        }
    };
}

/// The 14 fonts of the English game files (a test checks this table against the file).
pub const FONTS: [FontInfo; 14] = [
    font!(0, "Font8", "font8", "d2data", 14, 22, 15, 14),
    font!(1, "Font16", "font16", "d2data", 10, 16, 14, 16),
    font!(2, "Font30", "font30", "d2data", 19, 30, 29, 30),
    font!(3, "Font42", "font42", "d2data", 27, 43, 39, 41),
    font!(4, "FontFormal10", "fontformal10", "d2exp", 15, 24, 15, 15),
    font!(5, "FontFormal12", "fontformal12", "d2data", 20, 32, 35, 30),
    font!(6, "Font6", "font6", "d2data", 7, 11, 9, 11),
    font!(7, "Font24", "font24", "d2data", 15, 24, 22, 26),
    font!(8, "FontFormal11", "fontformal11", "d2data", 16, 25, 18, 16),
    font!(9, "FontExocet10", "fontexocet10", "d2data", 10, 16, 17, 18),
    font!(
        10,
        "FontRidiculous",
        "fontridiculous",
        "d2data",
        7,
        11,
        12,
        14
    ),
    font!(11, "FontExocet8", "fontexocet8", "d2data", 13, 20, 13, 13),
    font!(
        12,
        "ReallyTheLastSucker",
        "reallythelastsucker",
        "d2exp",
        7,
        11,
        9,
        11
    ),
    font!(
        13,
        "FontInGameChat",
        "fontingamechat",
        "d2data",
        15,
        24,
        16,
        15
    ),
];

/// The font a [`TextStyle::font`] id names; `None` past id 13.
pub fn font_info(id: u16) -> Option<&'static FontInfo> {
    FONTS.get(usize::from(id))
}

/// The language: the first byte of `data\local\use`; >= 14 reads as 0. A
/// missing or empty file reads as 0.
pub fn language_byte(use_file: &[u8]) -> u8 {
    match use_file.first() {
        Some(&b) if b < 14 => b,
        _ => 0,
    }
}

/// The font directory and glyph lookup kind of a language; only `Latin\` is in
/// the stock archives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontLocale {
    pub dir: &'static str,
    /// `true`: glyph lookup by code; `false`: by position.
    pub by_code: bool,
}

/// Directory and lookup for `font` under `language`: with
/// language 12 the chat font (13) uses `KOR\` and the lookup by code,
/// the other fonts `Latin\` by position.
pub fn font_locale(language: u8, font: u16) -> FontLocale {
    let (dir, by_code) = match language {
        0..=5 => ("Latin\\", false),
        6 => ("JPN\\", true),
        7 => ("KOR\\", true),
        8 | 9 => ("CHI\\", true),
        10 => ("LATIN2\\", true),
        11 => ("CYR\\", true),
        12 if font == 13 => ("KOR\\", true),
        12 => ("Latin\\", false),
        _ => ("Latin\\", true),
    };
    FontLocale { dir, by_code }
}

/// The current font id (`D2Win_SetUnicodeTextFont`);
/// font 1 is loaded at start. Loading and unloading change no pixel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentFont(pub u16);

impl Default for CurrentFont {
    fn default() -> Self {
        Self(1)
    }
}

impl CurrentFont {
    /// Makes `n` current and returns the previous id.
    pub fn set(&mut self, n: u16) -> u16 {
        std::mem::replace(&mut self.0, n)
    }
}

/// Where a text color `k` takes its 256-byte shift map from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextColorSource {
    /// PL2 text-color map `k` (0..=12; map 0 is never used for drawing).
    Map(u8),
    /// `k` = -1: the selected-unit shift map (`+0xCC`).
    SelectedUnitShift,
    /// `k` = -2..-17: inventory color variation 15..0 (`+0xC8`..`+0x8C`).
    InventoryVariation(u8),
    /// `k` = -18..-48: light map 31..1 (`+0x88`..`+0x10`).
    LightMap(u8),
    /// 13: additive blend (`+0x104`).
    AdditiveBlend,
    /// 14: `+0x108`.
    Plus108,
    /// 15: `+0x10C`.
    Plus10C,
    /// 16: darkened shift (`+0x110`).
    Darkened,
    /// 17: the text RGB triples (`+0x114`).
    TextRgb,
    /// 18: `H`.
    H,
    /// 19: `R`.
    R,
    /// `k` >= 20 or < -48: past the block.
    PastBlock,
}

/// `k` -> its map source. A code's `k >= 13` never gets here (the colour code
/// sets 0); a caller's `k` is not range-checked.
pub fn text_color_source(k: i32) -> TextColorSource {
    use TextColorSource::*;
    match k {
        0..=12 => Map(k as u8),
        -1 => SelectedUnitShift,
        -17..=-2 => InventoryVariation((17 + k) as u8),
        -48..=-18 => LightMap((49 + k) as u8),
        13 => AdditiveBlend,
        14 => Plus108,
        15 => Plus10C,
        16 => Darkened,
        17 => TextRgb,
        18 => H,
        19 => R,
        _ => PastBlock,
    }
}

/// Byte offset into the 0x48-pointer block for `k < 0`:
/// `0xD0 + 4k`.
pub fn text_color_block_offset(k: i32) -> i32 {
    0xD0 + 4 * k
}

/// Which text call draws the string (decision CG2: the game takes no clip
/// rectangle; so none is carried here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextOpts {
    /// `DrawText` and the mode variant.
    Draw {
        centered: bool,
        /// Centering block width; `None` = max width + 8. The framed
        /// variant passes the max width.
        block_w: Option<i32>,
        /// Draw mode of every glyph; 5 for `DrawText`.
        mode: u8,
    },
    /// Horizontal window: text-box scroll `s`, width `w`.
    Horizontal { s: i32, w: i32 },
    /// Vertical window: each glyph draws its encoded
    /// rows `skip … skip + lines − 1` from the bottom (the
    /// row window is the sprite's, `world_view::ui_bind::text_sprites`).
    Vertical { skip: i32, lines: i32 },
    // The game's no-color text variant is never called: no
    // counterpart.
}

impl Default for TextOpts {
    /// `DrawText`, not centered.
    fn default() -> Self {
        TextOpts::Draw {
            centered: false,
            block_w: None,
            mode: TEXT_DRAW_MODE,
        }
    }
}

impl TextOpts {
    /// The draw mode of the call's glyphs.
    pub fn mode(&self) -> u8 {
        match *self {
            TextOpts::Draw { mode, .. } => mode,
            _ => TEXT_DRAW_MODE,
        }
    }

    /// `DrawText(.., centered)` with the derived block.
    pub fn centered() -> Self {
        TextOpts::Draw {
            centered: true,
            block_w: None,
            mode: TEXT_DRAW_MODE,
        }
    }
}

/// One glyph the rules place: the code unit to draw, the pen (bottom row of the
/// glyph cell) and the text color `k` (may be outside 0–12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphPlacement {
    pub code: u16,
    pub at: Point,
    pub color: i32,
}

/// A placed glyph resolved to its font record and DC6 frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphDraw {
    pub code: u16,
    /// Index of the record in [`FontTable::glyphs`].
    pub record: usize,
    /// Frame in the font's DC6 (the font table's `frame`).
    pub frame: u16,
    pub at: Point,
    pub color: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    /// The font has fewer records than the lookup names. Every stock Latin
    /// font has 256 records, so this is malformed input, never a
    /// fallback glyph.
    #[error("font has no glyph record {record} (code {code:#06x})")]
    MissingGlyph { code: u16, record: usize },
    /// A rule the text layout has not settled yet.
    #[error("text layout not specified: TODO(spec: {0})")]
    Unspecified(&'static str),
}

/// The glyph lookup and metrics of the current font.
pub struct GlyphLookup<'a> {
    font: &'a FontTable,
}

impl<'a> GlyphLookup<'a> {
    pub fn new(font: &'a FontTable) -> Self {
        GlyphLookup { font }
    }

    pub fn font(&self) -> &'a FontTable {
        self.font
    }

    /// The record of code unit `code`, by position (Latin): record `code` for
    /// `code ≤ 0xFF`, else record 0. The record's `code` field is not read. The
    /// by-code lookup of other locales is out of scope (English).
    pub fn record(&self, code: u16) -> Result<usize, TextError> {
        let record = if code <= 0xFF { usize::from(code) } else { 0 };
        if record < self.font.glyphs.len() {
            Ok(record)
        } else {
            Err(TextError::MissingGlyph { code, record })
        }
    }

    /// `adv(c)`: the record's `width`.
    pub fn adv(&self, code: u16) -> Result<i32, TextError> {
        Ok(i32::from(self.font.glyphs[self.record(code)?].width))
    }

    /// Header `height` (font height).
    pub fn height(&self) -> i32 {
        i32::from(self.font.height)
    }

    /// Line step `trunc(height × 16 / 10)`.
    pub fn line_step(&self) -> i32 {
        self.height() * LINE_STEP_TENTHS / 10
    }
}

/// The string up to its first NUL unit (the string length).
pub fn units(text: &[u16]) -> &[u16] {
    let len = text.iter().position(|&u| u == 0).unwrap_or(text.len());
    &text[..len]
}

/// Unit `i` of the NUL-ended string: NUL at and past its end.
fn at(text: &[u16], i: usize) -> u16 {
    text.get(i).copied().unwrap_or(0)
}

/// Width B: the first `n` units, `LF` 0, every other
/// unit (color codes too) its advance.
pub fn width_b(g: &GlyphLookup<'_>, text: &[u16], n: usize) -> Result<i32, TextError> {
    let text = units(text);
    let mut w = 0;
    for &u in &text[..n.min(text.len())] {
        if u != LF {
            w += g.adv(u)?;
        }
    }
    Ok(w)
}

/// Width A: width B over the whole string.
pub fn width_a(g: &GlyphLookup<'_>, text: &[u16]) -> Result<i32, TextError> {
    width_b(g, text, usize::MAX)
}

/// Width C of the `n` units from `start`, stopping at
/// a NUL unit (the NUL at the end counts in `n` but adds nothing, so `n`
/// past the end adds nothing): `LF` 0; `ÿ`, `c`, `0`–`6` skipped only
/// when the code starts at span index `i` with `i + 3 < n`; every other
/// unit its advance.
pub fn width_c(
    g: &GlyphLookup<'_>,
    text: &[u16],
    start: usize,
    n: usize,
) -> Result<i32, TextError> {
    let text = units(text);
    let mut w = 0;
    let mut i = 0;
    while i < n {
        let u = at(text, start + i);
        if u == 0 {
            break;
        }
        if u == COLOR_LEAD
            && i + 3 < n
            && at(text, start + i + 1) == u16::from(b'c')
            && (u16::from(b'0')..=u16::from(b'6')).contains(&at(text, start + i + 2))
        {
            i += 3;
            continue;
        }
        if u != LF {
            w += g.adv(u)?;
        }
        i += 1;
    }
    Ok(w)
}

/// Line width from unit `start` to the first `LF` or
/// the end, and the index where it stopped: `ÿ` and the next two units
/// count 0, plus `adv('m')` if the unit after `ÿ` is `m` or `M`.
fn line_extent(g: &GlyphLookup<'_>, text: &[u16], start: usize) -> Result<(i32, usize), TextError> {
    let mut w = 0;
    let mut i = start;
    while i < text.len() && text[i] != LF {
        if text[i] == COLOR_LEAD {
            if matches!(at(text, i + 1), 0x6D | 0x4D) {
                w += g.adv(u16::from(b'm'))?;
            }
            // The two units are passed over unread: a `LF` among them
            // does not end the line, a skip past the end ends the walk.
            i = (i + 3).min(text.len());
        } else {
            w += g.adv(text[i])?;
            i += 1;
        }
    }
    Ok((w, i))
}

/// Line width of the line starting at unit `start`.
pub fn line_width(g: &GlyphLookup<'_>, text: &[u16], start: usize) -> Result<i32, TextError> {
    Ok(line_extent(g, units(text), start)?.0)
}

/// Max width: the widest `LF` line, by line width.
pub fn max_width(g: &GlyphLookup<'_>, text: &[u16]) -> Result<i32, TextError> {
    let text = units(text);
    let mut best = 0;
    let mut start = 0;
    loop {
        let (w, end) = line_extent(g, text, start)?;
        best = best.max(w);
        if end >= text.len() {
            return Ok(best);
        }
        start = end + 1;
    }
}

/// Text height: `trunc(height × 16 × lines / 10)`,
/// lines = 1 + number of `LF`.
pub fn text_height(g: &GlyphLookup<'_>, text: &[u16]) -> i32 {
    let lines = 1 + units(text).iter().filter(|&&u| u == LF).count() as i32;
    g.height() * TEXT_HEIGHT_TENTHS * lines / 10
}

/// The game's text layout.
pub trait TextRules {
    fn place(
        &self,
        glyphs: &GlyphLookup<'_>,
        text: &[u16],
        origin: Point,
        style: TextStyle,
        opts: &TextOpts,
    ) -> Result<Vec<GlyphPlacement>, TextError>;
}

/// The draw call and its variants as the game's text layer runs them:
/// pen moves right by the advance, `LF` moves it up one line step, `ÿc`
/// codes switch the color, no kerning, tabs or spacing.
#[derive(Clone, Copy, Debug, Default)]
pub struct OriginalText;

/// What the unit at `i` is (glyph, colour code or line feed).
enum Unit {
    /// A color code: the new `k` and the units it takes.
    Color(i32, usize),
    /// `ÿ` or `ÿc` at the end: color 0, drawing ends.
    End,
    /// An ordinary glyph (including `ÿ` + anything else).
    Glyph,
}

fn color_code(text: &[u16], i: usize) -> Unit {
    if text[i] != COLOR_LEAD {
        return Unit::Glyph;
    }
    match text.get(i + 1) {
        None => Unit::End,
        Some(&c) if c == u16::from(b'c') => match text.get(i + 2) {
            None => Unit::End,
            Some(&d) => {
                let k = i32::from(d) - 0x30;
                Unit::Color(if k >= TEXT_COLORS as i32 { 0 } else { k }, 3)
            }
        },
        Some(_) => Unit::Glyph,
    }
}

impl TextRules for OriginalText {
    fn place(
        &self,
        g: &GlyphLookup<'_>,
        text: &[u16],
        origin: Point,
        style: TextStyle,
        opts: &TextOpts,
    ) -> Result<Vec<GlyphPlacement>, TextError> {
        let text = units(text);
        let Point { x, y } = origin;
        let (centered, block_w) = match *opts {
            TextOpts::Draw {
                centered, block_w, ..
            } => (centered, block_w),
            TextOpts::Vertical { skip, .. } if skip < 0 => return Ok(Vec::new()),
            TextOpts::Vertical { .. } => return vertical_window(g, text, origin),
            TextOpts::Horizontal { .. } => (false, None),
        };
        let block = match (centered, block_w) {
            (false, _) => 0,
            (true, Some(w)) => w,
            (true, None) => max_width(g, text)? + 8,
        };
        let line_x = |start: usize| -> Result<i32, TextError> {
            Ok(if centered {
                x + ((block - line_extent(g, text, start)?.0) >> 1)
            } else {
                x
            })
        };
        let step = g.line_step();
        let mut pen = Point::new(line_x(0)?, y);
        let mut k = i32::from(style.color);
        if let TextOpts::Horizontal { s, .. } = *opts {
            pen.x = x + s;
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < text.len() {
            // Horizontal window: the stop test runs before every unit
            // (glyph, `ÿ` code or `LF`).
            if let TextOpts::Horizontal { w, .. } = *opts {
                if pen.x > x + w {
                    break;
                }
            }
            let u = text[i];
            if u == COLOR_LEAD {
                match color_code(text, i) {
                    Unit::Color(code, len) => {
                        k = code;
                        i += len;
                        continue;
                    }
                    Unit::End => break,
                    Unit::Glyph => {}
                }
            }
            if u == LF {
                pen.x = line_x(i + 1)?;
                pen.y -= step;
                i += 1;
                continue;
            }
            let draw = match *opts {
                TextOpts::Horizontal { .. } => pen.x > x,
                _ => true,
            };
            if draw {
                out.push(GlyphPlacement {
                    code: u,
                    at: pen,
                    color: k,
                });
            }
            pen.x += g.adv(u)?;
            i += 1;
        }
        Ok(out)
    }
}

/// The glyphs of the vertical window (`skip ≥ 0`): the pen starts
/// at `origin`; `ÿc` + 1 unit is skipped (no color: every glyph has color 0, no
/// remap), `ÿ` + any other unit but `m` skips those 2 units; `LF` resets x and
/// moves y up one line step; every other unit is drawn at the pen, which then
/// moves right by its advance. `ÿm` (a MonsterIndicators cel) is
/// not drawn by D2RUST: an error.
fn vertical_window(
    g: &GlyphLookup<'_>,
    text: &[u16],
    origin: Point,
) -> Result<Vec<GlyphPlacement>, TextError> {
    let mut pen = origin;
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let u = text[i];
        if u == COLOR_LEAD {
            match at(text, i + 1) {
                c if c == u16::from(b'c') => i += 3,
                m if m == u16::from(b'm') => {
                    return Err(TextError::Unspecified(
                        "ui/text.md §9 vertical window `ÿm` (MonsterIndicators cel)",
                    ))
                }
                _ => i += 2,
            }
            continue;
        }
        if u == LF {
            pen.x = origin.x;
            pen.y -= g.line_step();
            i += 1;
            continue;
        }
        out.push(GlyphPlacement {
            code: u,
            at: pen,
            color: 0,
        });
        pen.x += g.adv(u)?;
        i += 1;
    }
    Ok(out)
}

/// `D2Client_DrawCenteredUnicodeText`: the x at which
/// `DrawText(text, x, y, k, 0)` centers `text` in the span `x1..=x2` by
/// width A (color codes counted).
pub fn centered_span_x(
    g: &GlyphLookup<'_>,
    text: &[u16],
    x1: i32,
    x2: i32,
) -> Result<i32, TextError> {
    let span = x2 - x1 + 1;
    let w = width_a(g, text)?;
    Ok(if w < span { x1 + ((span - w) >> 1) } else { x1 })
}

/// A framed hover box: the rectangle corners handed to
/// `D2GFX_DrawRectangle` and the text call inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramedText {
    /// `(x0, y0)`–`(x1, y1)` as passed to the rectangle draw; its pixels
    /// are the blend table's.
    pub rect: (Point, Point),
    /// Pen of the text call.
    pub pen: Point,
    pub opts: TextOpts,
}

/// `DrawFramedText` on a `screen` of (Sw, Sh).
pub fn framed_text(
    g: &GlyphLookup<'_>,
    text: &[u16],
    at: Point,
    screen: (i32, i32),
) -> Result<FramedText, TextError> {
    let (sw, sh) = screen;
    let w = max_width(g, text)? + 8;
    let h = text_height(g, text);
    let x = at.x.max(0).min(sw - w);
    let y = at.y + 2;
    let mut b = y.max(h);
    if b >= sh - 31 {
        b = sh - 31;
    }
    Ok(FramedText {
        rect: (Point::new(x, b - h), Point::new(x + w, b)),
        pen: Point::new(x, b - 3 * g.height() / 10),
        opts: TextOpts::Draw {
            centered: true,
            block_w: Some(w),
            mode: TEXT_DRAW_MODE,
        },
    })
}

/// The framed variant on a `screen` of (Sw, Sh).
pub fn framed_text_tight(
    g: &GlyphLookup<'_>,
    text: &[u16],
    at: Point,
    screen: (i32, i32),
) -> Result<FramedText, TextError> {
    let (sw, sh) = screen;
    let w = max_width(g, text)?;
    let x = if at.x + w > sw { sw - w } else { at.x };
    let mut y = (at.y + 2).min(sh - 31);
    if y - g.height() < 0 {
        y *= 2;
    }
    Ok(FramedText {
        rect: (Point::new(x, y - g.height()), Point::new(x + w, y)),
        pen: Point::new(x, y - 2),
        opts: TextOpts::Draw {
            centered: true,
            block_w: Some(w),
            mode: TEXT_DRAW_MODE,
        },
    })
}

/// The pop-up text draw of a `(text, x, y, k, centre)`
/// call on a `screen` of (Sw, Sh), in the current font `g`: block W = max width +
/// 8, x0 = x − (W >> 1) when `centre`, else x; y0 = y + 2; x' = 0 when x0 ≤
/// 0, then min(x', Sw − W); b = max(y0, Ht − 5), Sh − 5 when that is not below
/// Sh − 5; the backing box (x', b − Ht)–(x' + W, b) (colour 0, mode 2) and the
/// text at (x', b − trunc(3 · height / 10)), block W, centred. The too-tall
/// font swap is not applied here.
pub fn popup_text(
    g: &GlyphLookup<'_>,
    text: &[u16],
    at: Point,
    centre: bool,
    screen: (i32, i32),
) -> Result<FramedText, TextError> {
    let (sw, sh) = screen;
    let w = max_width(g, text)? + 8;
    let ht = text_height(g, text);
    let x0 = if centre { at.x - (w >> 1) } else { at.x };
    let y0 = at.y + 2;
    let x = if x0 <= 0 { 0 } else { x0 }.min(sw - w);
    let m = y0.max(ht - 5);
    let b = if m < sh - 5 { m } else { sh - 5 };
    Ok(FramedText {
        rect: (Point::new(x, b - ht), Point::new(x + w, b)),
        pen: Point::new(x, b - 3 * g.height() / 10),
        opts: TextOpts::Draw {
            centered: true,
            block_w: Some(w),
            mode: TEXT_DRAW_MODE,
        },
    })
}

/// White space of the wrap break test: `< 0x100`
/// and CRT `isspace` in the "C" locale.
fn is_space(u: u16) -> bool {
    matches!(u, 0x09..=0x0D | 0x20)
}

/// `Wrap(text, M)` for English: the lines, in order.
/// A line keeps its trailing white space and any `LF` (drawn as two lines
/// by the line feed rule); a leading white-space unit is dropped.
pub fn wrap<'t>(
    g: &GlyphLookup<'_>,
    text: &'t [u16],
    max: i32,
) -> Result<Vec<&'t [u16]>, TextError> {
    let text = units(text);
    let l = text.len();
    let c = |s: usize, e: usize| width_c(g, text, s, e - s + 1);
    if width_c(g, text, 0, l)? <= max {
        return Ok(vec![text]);
    }
    let shrink = |s: usize| -> Result<usize, TextError> {
        let mut e = l;
        while e > s && c(s, e)? > max {
            e -= 1;
        }
        Ok(e)
    };
    let mut lines = Vec::new();
    let mut s = 0;
    while s <= l {
        let mut e = shrink(s)?;
        if e > s && e != l {
            while e > s && !(is_space(at(text, e)) && !is_space(at(text, e + 1))) {
                e -= 1;
            }
        }
        if e == s {
            e = shrink(s)?;
        }
        let mut line = &text[s.min(l)..(e + 1).min(l)];
        if line.first().is_some_and(|&u| is_space(u)) {
            line = &line[1..];
        }
        lines.push(line);
        s = e + 1;
    }
    Ok(lines)
}

/// Lays out `text` (UTF-16 code units, unchanged) at `origin`: `rules`
/// place the glyphs, then each placed code is resolved to its font record
/// and DC6 frame. Output order is the rules' placement order.
pub fn layout_text(
    font: &FontTable,
    text: &[u16],
    origin: Point,
    style: TextStyle,
    opts: &TextOpts,
    rules: &dyn TextRules,
) -> Result<Vec<GlyphDraw>, TextError> {
    let lookup = GlyphLookup::new(font);
    rules
        .place(&lookup, text, origin, style, opts)?
        .into_iter()
        .map(|p| {
            let record = lookup.record(p.code)?;
            Ok(GlyphDraw {
                code: p.code,
                record,
                frame: font.glyphs[record].frame,
                at: p.at,
                color: p.color,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_is_first_byte_of_use() {
        assert_eq!(language_byte(&[0]), 0);
        assert_eq!(language_byte(&[7, 1, 2]), 7);
        assert_eq!(language_byte(&[13]), 13);
        assert_eq!(language_byte(&[14]), 0);
        assert_eq!(language_byte(&[0xFF]), 0);
        assert_eq!(language_byte(&[]), 0);
    }

    #[test]
    fn locale_font_directory() {
        let l = |lang, font| {
            let f = font_locale(lang, font);
            (f.dir, f.by_code)
        };
        for lang in (0..=5).chain([12]) {
            assert_eq!(l(lang, 1), ("Latin\\", false), "lang {lang}");
        }
        assert_eq!(l(6, 0), ("JPN\\", true));
        assert_eq!(l(7, 0), ("KOR\\", true));
        assert_eq!(l(8, 0), ("CHI\\", true));
        assert_eq!(l(9, 0), ("CHI\\", true));
        assert_eq!(l(10, 0), ("LATIN2\\", true));
        assert_eq!(l(11, 0), ("CYR\\", true));
        assert_eq!(l(13, 0), ("Latin\\", true));
        // language 12: the chat font alone moves to KOR by code
        assert_eq!(l(12, 13), ("KOR\\", true));
        assert_eq!(l(12, 12), ("Latin\\", false));
    }

    #[test]
    fn only_fourteen_names_are_font_inputs() {
        assert_eq!(FONTS.len(), 14);
        for f in &FONTS {
            for bad in ["default", "fonter", "readme", "default.map"] {
                assert!(!f.tbl_path.contains(bad), "{}", f.tbl_path);
            }
            assert!(f.tbl_path.ends_with(".tbl") && f.dc6_path.ends_with(".dc6"));
        }
    }

    #[test]
    fn set_font_returns_previous() {
        let mut c = CurrentFont::default();
        assert_eq!(c.0, 1, "font 1 is loaded at start");
        assert_eq!(c.set(4), 1);
        assert_eq!(c.set(0), 4);
        assert_eq!(c.0, 0);
    }

    #[test]
    fn color_k_selects_its_map() {
        use TextColorSource::*;
        // `ÿc!` is k = -15: inventory color variation 2 at +0x94.
        assert_eq!(text_color_source(-15), InventoryVariation(2));
        assert_eq!(text_color_block_offset(-15), 0x94);
        assert_eq!(text_color_source(-1), SelectedUnitShift);
        assert_eq!(text_color_block_offset(-1), 0xCC);
        assert_eq!(text_color_source(-2), InventoryVariation(15));
        assert_eq!(text_color_block_offset(-2), 0xC8);
        assert_eq!(text_color_source(-17), InventoryVariation(0));
        assert_eq!(text_color_block_offset(-17), 0x8C);
        assert_eq!(text_color_source(-18), LightMap(31));
        assert_eq!(text_color_block_offset(-18), 0x88);
        assert_eq!(text_color_source(-48), LightMap(1));
        assert_eq!(text_color_block_offset(-48), 0x10);
        assert_eq!(text_color_source(12), Map(12));
        assert_eq!(text_color_source(13), AdditiveBlend);
        assert_eq!(text_color_source(16), Darkened);
        assert_eq!(text_color_source(17), TextRgb);
        assert_eq!(text_color_source(18), H);
        assert_eq!(text_color_source(19), R);
        assert_eq!(text_color_source(20), PastBlock);
    }
}
