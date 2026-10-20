//! The in-game UI bound to the bridge: each frame the panels see the
//! client world model read-only ([`UiCtx`]), route the frame's input
//! events, hand their intents to the bridge (only the root forwards), and
//! make draw requests that become scene items here. [`ui_elements`] lists
//! the open panels' clickable ids for scripts (`click ui:<id>`).
//!
//! No panel is defined here: the game's panels (art, layout, open/close
//! rules) live in `crate::ui`. What a request draws and where is a
//! [`UiRules`] hook. Text goes through `ui::text::layout_text`
//! ([`text_sprites`]); what the request does not carry (font, layout
//! rules, glyph look) is a [`TextHooks`] hook, answered for the game's
//! fonts by [`OriginalTextHooks`].

use crate::assets::path::CanonicalPath;
use crate::assets::path::{read_dc6, read_font_table};
use crate::bridge::click::ClickView;
use crate::bridge::link::ServerLink;
use crate::bridge::output::Output;
use crate::bridge::{Bridge, BridgeError};
use crate::composite::ComponentFrame;
use crate::controls::click::{ClickOut, ClickState, Kind};
use crate::frames::{FramePart, FrameSet, FrameSetKey, IndexFrame};
use crate::rules::blend::{
    cel_ops, color_row, gdi_rectangle_box, gdi_rectangle_ops, mode_table, MODE_HIGHLIGHT,
};
use crate::rules::camera::FrameSize;
use crate::rules::placement::draw_position;
use crate::rules::shading::{item_color, ShadeTables, ITEM_PALETTE_MAPS};
use crate::scene::{BlendOp, DrawItem, DrawKey, ItemTag, MapId, MapTable, Rect, ShadeChain};
use crate::ui::original::{OriginalUi, OriginalUiError};
use crate::ui::text::{TEXT_COLORS, TEXT_COLOR_MAP_OFFSET};
use crate::ui::Routed;
use crate::ui::{
    font_info, layout_text, ImageRequest, LineRequest, OriginalText, PointerButton, RectRequest,
    Remap, StringLookup, TextOpts, TextRequest, TextRules, TextStyle, UiCtx, UiDraw, UiEvent,
    UiInput, UiRoot,
};

use super::{Unspecified, ViewAssets, ViewError};
use crate::facts::export::{UI_LINE_REST, UI_LINE_START};

/// One sprite of a UI request: an image, or one glyph of laid-out text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSprite {
    pub frame: ComponentFrame,
    /// Screen top-left.
    pub x: i32,
    pub y: i32,
    pub shade: ShadeChain,
    pub blend: BlendOp,
    /// A clip narrower than the request's (cut to it): the rows of a
    /// vertical-window glyph (`ui/text.md` §9 details r2). `None`: the
    /// request's clip.
    pub clip: Option<Rect>,
}

/// The original-behavior questions of UI drawing, one hook per question.
pub trait UiRules {
    /// The sprite of an image request (`ui/panels-2.md` §22 r4): the DC6
    /// file (under `data\global\ui\` or its rule's path) and frame an
    /// [`crate::ui::ImageRef`] names, direction 0; `req.at` is the cel draw
    /// point (X, Y), the frame covering columns `X + xoff …`, rows
    /// `Y + yoff − h + 1 … Y + yoff` (`render/sprite-placement.md` §2);
    /// the plain cel draw (mode 5, light 0xFF) has no shade and blend
    /// `Opaque`. [`super::panel_art::panel_sprite`] answers it.
    fn ui_image(&self, req: &ImageRequest, assets: &ViewAssets) -> Result<UiSprite, ViewError>;

    /// One sprite per drawn glyph, in drawing order (`ui/text.md`).
    /// [`text_sprites`] answers it through `layout_text` from
    /// [`TextHooks`]; [`Unspecified`] does so.
    fn ui_text(&self, req: &TextRequest, assets: &ViewAssets) -> Result<Vec<UiSprite>, ViewError>;

    /// The pass number of UI items: 11, everything after `0x00476BC0`
    /// (`render/draw-order.md` §10). Every UI item gets key
    /// `(pass, 0, 0, 0)`, so the stable sort keeps emission order (draw
    /// order = list order).
    fn ui_pass(&self) -> Result<u32, ViewError>;
}

/// The font of a text style: its glyph table (`.tbl`, `formats/font-tbl.md`)
/// and the DC6 frame set its glyph frames index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFont {
    pub table: CanonicalPath,
    pub glyphs: FrameSetKey,
}

/// What UI text needs besides the request and `layout_text`, one hook per
/// question. [`OriginalTextHooks`] answers them from `ui/text.md`.
pub trait TextHooks {
    /// The font a style's `font` id names: glyph table and DC6.
    fn text_font(&self, style: TextStyle) -> Result<TextFont, ViewError>;

    /// The layout rules (advance, line step, color codes, centering).
    fn text_rules(&self) -> &dyn TextRules;

    /// Shade and blend of a glyph in text color `color` drawn with draw
    /// mode `mode` (`ui/text.md` §4.3).
    fn glyph_look(&self, color: i32, mode: u8) -> Result<(ShadeChain, BlendOp), ViewError>;
}

/// UI text through `ui::text::layout_text` (`ui.md` §A3): the style's
/// font from `assets`, the rules' placements resolved to glyph records,
/// one sprite per placed glyph in placement order. A glyph is its DC6
/// frame drawn at the pen (`ui/text.md` §4.2): the sprite top-left is
/// `sprite-placement.md` §2/§8 of that frame. The request's `clip` is the
/// item clip; the original's text has no other clip (§12, CG2).
pub fn text_sprites<H: TextHooks + ?Sized>(
    hooks: &H,
    req: &TextRequest,
    assets: &ViewAssets,
) -> Result<Vec<UiSprite>, ViewError> {
    let font = hooks.text_font(req.style)?;
    let table = assets
        .fonts
        .get(&font.table)
        .ok_or_else(|| ViewError::FontMissing(font.table.clone()))?;
    let glyphs = layout_text(
        table,
        &req.text,
        req.at,
        req.style,
        &req.opts,
        hooks.text_rules(),
    )?;
    let mut out = Vec::with_capacity(glyphs.len());
    for g in glyphs {
        let (shade, blend) = hooks.glyph_look(g.color, req.opts.mode())?;
        let index = usize::from(g.frame);
        let frame = assets.frame(&font.glyphs, index)?;
        let (x, y) = draw_position(frame, g.at.x, g.at.y);
        let clip = match req.opts {
            TextOpts::Vertical { skip, lines } => {
                match vertical_rows(y, frame.height as i32, skip, lines) {
                    Some(r) => Some(r),
                    None => continue,
                }
            }
            _ => None,
        };
        out.push(UiSprite {
            frame: ComponentFrame {
                set: font.glyphs.clone(),
                index,
            },
            x,
            y,
            shade,
            blend,
            clip,
        });
    }
    Ok(out)
}

/// The screen rows a vertical-window glyph draws (`0x00501DF0` →
/// `0x004F64E0`, `ui/text.md` §9 details r2) as a clip, for a cel whose
/// top row is `top_y` and height `h`: rows `skip`, `lines` when `skip +
/// lines ≤ h`, else rows 0, `h`; with `B` the cel's bottom row, `top = B −
/// skip − lines`, `bot = B − skip`, nothing when `lines` = 0, `bot < 0` or
/// `top ≥ H`; else the screen rows `max(top, 0) + 1 … min(bot, H − 1)`
/// (row 0 is never drawn when the window is cut at the top). `None`:
/// nothing drawn.
pub fn vertical_rows(top_y: i32, h: i32, skip: i32, lines: i32) -> Option<Rect> {
    let (skip, lines) = if skip + lines <= h {
        (skip, lines)
    } else {
        (0, h)
    };
    let frame = Rect::world_clip();
    let screen_h = frame.height as i32;
    let b = top_y + h - 1;
    let (top, bot) = (b - skip - lines, b - skip);
    if lines == 0 || bot < 0 || top >= screen_h {
        return None;
    }
    let (top, bot) = (top.max(0), bot.min(screen_h - 1));
    (bot > top).then(|| Rect::new(0, top + 1, frame.width, (bot - top) as u32))
}

/// The 12 text-color maps of the frame's act `pal.pl2` in the map table
/// (`ui/text.md` §4.4): `maps[k − 1]` is map `k`. Map 0 is never drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextColors {
    pub maps: [MapId; TEXT_COLORS - 1],
}

impl TextColors {
    /// Pushes maps 1–12 of `pl2` (file offset `439,847 + 256k`) onto
    /// `table`. The loader copies 13 maps, so the file must hold all 13.
    pub fn push(table: &mut MapTable, pl2: &[u8]) -> Result<TextColors, ViewError> {
        let end = TEXT_COLOR_MAP_OFFSET + 256 * TEXT_COLORS;
        let bytes = pl2
            .get(TEXT_COLOR_MAP_OFFSET..end)
            .ok_or_else(|| ViewError::Unresolved {
                what: "UI text color maps",
                spec: "ui/text.md",
                message: format!(
                "PL2 of {} bytes ends before its text-color maps (offset {TEXT_COLOR_MAP_OFFSET}, \
                 {} bytes)",
                pl2.len(),
                256 * TEXT_COLORS
            ),
            })?;
        let mut maps = [MapId(0); TEXT_COLORS - 1];
        for (k, map) in maps.iter_mut().enumerate() {
            let row: [u8; 256] = bytes
                .get(256 * (k + 1)..256 * (k + 2))
                .and_then(|r| r.try_into().ok())
                .ok_or_else(|| ViewError::Unresolved {
                    what: "UI text color maps",
                    spec: "ui/text.md",
                    message: format!("text-color map {} is not 256 bytes", k + 1),
                })?;
            *map = table.push(row);
        }
        Ok(TextColors { maps })
    }
}

/// Makes the fonts of a frame's UI text resident (`ui/text.md` §1.3): for
/// each text draw, its style's glyph table (`.tbl`) into
/// [`ViewAssets::fonts`] and its glyph DC6 (direction 0) into
/// [`ViewAssets::frames`], read from `source` once. The image half is
/// [`super::panel_art::PanelArtLoader`]. A file in no archive, or one
/// that does not parse, is an error.
pub struct TextAssetLoader {
    pub source: std::sync::Arc<dyn crate::assets::path::FileSource>,
}

impl TextAssetLoader {
    pub fn ensure(&self, draws: &[UiDraw], assets: &mut ViewAssets) -> Result<(), ViewError> {
        for d in draws {
            let UiDraw::Text(req) = d else { continue };
            let font = original_text_font(req.style)?;
            if !assets.fonts.contains_key(&font.table) {
                let table = self.read_typed(font.table.as_str(), read_font_table)?;
                assets.fonts.insert(font.table.clone(), table);
            }
            if !assets.frames.contains(&font.glyphs) {
                let path = font.glyphs.path().to_owned();
                let dc6 = self.read_typed(&path, read_dc6)?;
                let frames = crate::frames::FrameSet::from_dc6(&dc6, 0)
                    .map_err(|e| fail(&path, e.to_string()))?;
                assets.frames.insert(font.glyphs.clone(), frames)?;
            }
        }
        Ok(())
    }

    fn read_typed<T>(
        &self,
        path: &str,
        read: fn(&dyn crate::assets::path::FileSource, &str) -> Option<Result<T, String>>,
    ) -> Result<T, ViewError> {
        let archive = path.replace('/', "\\");
        read(self.source.as_ref(), &archive)
            .ok_or_else(|| fail(path, "in no archive".into()))?
            .map_err(|e| fail(path, e))
    }
}

fn fail(path: &str, message: String) -> ViewError {
    ViewError::Unresolved {
        what: "UI font file",
        spec: "ui/text.md",
        message: format!("{path}: {message}"),
    }
}

/// The text hooks of `ui/text.md`: fonts from `text-fonts.tsv` (§1.3),
/// the [`OriginalText`] rules, glyph look per §4.3. `colors` are the
/// frame's text-color maps; without them only color 0 draws. `shades` are
/// the act's blend tables (`render/blend-modes.md` §1): a draw mode other
/// than 5 (the §9 draw with mode) needs them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OriginalTextHooks {
    pub colors: Option<TextColors>,
    pub shades: Option<ShadeTables>,
}

/// The font of id `style.font` (`text-fonts.tsv`).
pub fn original_text_font(style: TextStyle) -> Result<TextFont, ViewError> {
    // Memoized per font id (q-perf): the 14 fonts' paths are validated
    // once per thread, not per text draw per frame.
    thread_local! {
        static FONTS: std::cell::RefCell<[Option<TextFont>; 14]> =
            const { std::cell::RefCell::new([const { None }; 14]) };
    }
    let id = usize::from(style.font);
    if id < 14 {
        if let Some(f) = FONTS.with(|c| c.borrow()[id].clone()) {
            return Ok(f);
        }
    }
    let font = build_text_font(style)?;
    if id < 14 {
        FONTS.with(|c| c.borrow_mut()[id] = Some(font.clone()));
    }
    Ok(font)
}

fn build_text_font(style: TextStyle) -> Result<TextFont, ViewError> {
    let info = font_info(style.font).ok_or_else(|| ViewError::Unresolved {
        what: "UI text font",
        spec: "ui/text.md",
        message: format!("font id {} is not one of the 14 fonts (0–13)", style.font),
    })?;
    let bad = |e: String| ViewError::Unresolved {
        what: "UI text font",
        spec: "ui/text.md",
        message: e,
    };
    Ok(TextFont {
        table: CanonicalPath::new(info.tbl_path).map_err(|e| bad(e.to_string()))?,
        glyphs: FrameSetKey::new(
            CanonicalPath::new(info.dc6_path)
                .map_err(|e| bad(e.to_string()))?
                .as_str(),
            FramePart::Dir(0),
        )
        .map_err(|e| bad(e.to_string()))?,
    })
}

/// Glyph look of `ui/text.md` §4.3: no light (byte 0xFF), remap =
/// text-color map `k` (none for 0), blend by the draw mode (5 for the
/// plain call, the §9 argument for the draw with mode;
/// [`ui_cel_ops`]).
fn original_glyph_look(
    colors: Option<&TextColors>,
    shades: Option<&ShadeTables>,
    color: i32,
    mode: u8,
) -> Result<(ShadeChain, BlendOp), ViewError> {
    let remap = if color == 0 {
        None
    } else {
        if !(1..TEXT_COLORS as i32).contains(&color) {
            return Err(ViewError::Unresolved {
                what: "UI text color",
                spec: "ui/text.md",
                message: format!("color {color} outside 0–12 (open question 2)"),
            });
        }
        let colors = colors.ok_or_else(|| ViewError::Unresolved {
            what: "UI text color",
            spec: "ui/text.md",
            message: "the frame's PL2 text-color maps are not loaded".into(),
        })?;
        Some(colors.maps[color as usize - 1])
    };
    ui_cel_ops(shades, mode, remap)
}

/// Shade and blend of a UI cel draw (light byte 0xFF, `ui/panels.md`
/// §1.4, §1.6) of draw mode `mode` with remap `P` (`render/blend-modes.md`
/// §1, §2): `d' = T[256·d + P[s]]` with `T` of the mode, `d' = P[s]`
/// without (modes 5 and other values), `d' = H[s]` in mode 7
/// ([`cel_ops`]). Without the act's tables only a mode with neither `T`
/// nor `H` draws; the others are an error naming the missing tables.
pub fn ui_cel_ops(
    shades: Option<&ShadeTables>,
    mode: u8,
    remap: Option<MapId>,
) -> Result<(ShadeChain, BlendOp), ViewError> {
    if let Some(t) = shades {
        return Ok(cel_ops(t, mode, remap, 0xFF));
    }
    if mode_table(mode).is_some() || mode == MODE_HIGHLIGHT {
        return Err(ViewError::Unresolved {
            what: "UI draw mode",
            spec: "render/blend-modes.md",
            message: format!("draw mode {mode} needs the act's blend tables, not loaded"),
        });
    }
    let maps: Vec<MapId> = remap.into_iter().collect();
    Ok((ShadeChain::new(&maps)?, BlendOp::Opaque))
}

/// The remap `P` of a UI cel's [`Remap`] (`ui/panels.md` §1.6):
/// - `Palette(k)` reads pointer `+0xD0 + 4k` of the palette-table block
///   (`ui/text.md` §4.3–§4.5): 0 none, 1–12 PL2 text-colour map `k`
///   (`colors`), −18 … −48 light maps 31 … 1 (`shades`); −1 (selected
///   unit shift), −2 … −17 (inventory colour variations) and every other
///   `k` have no map in D2RUST and are errors;
/// - `ItemColor { t, c }`: `rules::shading::item_color` (`shading.md`
///   §6 r4): no map for `t` 0, 3, 4, ≥ 9 or `c` ≥ 21, else map `c` of
///   file `t` (`assets.item_palettes`).
pub fn ui_remap(
    remap: Remap,
    colors: Option<&TextColors>,
    assets: &ViewAssets,
) -> Result<Option<MapId>, ViewError> {
    let fail = |message: String| ViewError::Unresolved {
        what: "UI cel remap",
        spec: "ui/text.md",
        message,
    };
    match remap {
        Remap::None | Remap::Palette(0) => Ok(None),
        Remap::Palette(k) if (1..TEXT_COLORS as i32).contains(&k) => {
            let colors = colors
                .ok_or_else(|| fail("the frame's PL2 text-color maps are not loaded".into()))?;
            Ok(Some(colors.maps[k as usize - 1]))
        }
        Remap::Palette(k) if (-48..=-18).contains(&k) => {
            let shades = assets
                .shades
                .as_ref()
                .ok_or_else(|| fail(format!("k {k}: the act's light maps are not loaded")))?;
            Ok(Some(shades.light_map((49 + k) as u8)))
        }
        Remap::Palette(k) => Err(fail(format!(
            "palette argument {k}: no map of the §4.5 block in D2RUST"
        ))),
        Remap::ItemColor { t, c } => {
            let Some((t, c)) = item_color(t, c) else {
                return Ok(None);
            };
            let base = assets.item_palettes.ok_or_else(|| ViewError::Unresolved {
                what: "UI cel remap",
                spec: "render/shading.md",
                message: format!("item colour ({t}, {c}): the item palette maps are not loaded"),
            })?;
            Ok(Some(MapId(
                base.0 + u32::from(ITEM_PALETTE_MAPS) * (u32::from(t) - 1) + u32::from(c),
            )))
        }
    }
}

/// The frame set of a rectangle of `w × h` pixels: one frame of index 1
/// (`rules::blend` GDI draws: the chain turns 1 into the written source).
pub fn rect_key(w: u32, h: u32) -> FrameSetKey {
    FrameSetKey::new(format!("d2rust/ui/rect/{w}x{h}"), FramePart::Tile(0)).expect("canonical")
}

/// Makes what the frame's rectangles need resident: the colour rows
/// (once) and one `w × h` frame per clamped size (`blend-modes.md` §8
/// r2 on the `size` surface). A rectangle the original refuses (rows or
/// columns reversed) is an error.
pub fn ensure_rects(
    draws: &[UiDraw],
    size: FrameSize,
    assets: &mut ViewAssets,
) -> Result<(), ViewError> {
    for d in draws {
        if let UiDraw::Line(_) = d {
            for path in [UI_LINE_START, UI_LINE_REST] {
                line_pixel_key(assets, path)?;
            }
            continue;
        }
        let UiDraw::Rect(r) = d else { continue };
        if assets.color_rows.is_none() {
            let base = MapId(assets.maps.len() as u32);
            for c in 0..=255u8 {
                assets.maps.push(color_row(c));
            }
            assets.color_rows = Some(base);
        }
        let Some((_, _, w, h)) = rect_box(r, size)? else {
            continue;
        };
        let key = rect_key(w, h);
        if assets.frames.contains(&key) {
            continue;
        }
        let frame = IndexFrame::new(w, h, 0, 0, vec![1; w as usize * h as usize]).map_err(|e| {
            ViewError::Unresolved {
                what: "UI rectangle",
                spec: "render/blend-modes.md",
                message: e.to_string(),
            }
        })?;
        assets.frames.insert(
            key,
            FrameSet {
                frames: vec![frame],
            },
        )?;
    }
    Ok(())
}

/// A pixel set: frame `i` is one pixel of palette index `i` (as the
/// automap's lines).
fn line_pixel_key(assets: &mut ViewAssets, path: &str) -> Result<FrameSetKey, ViewError> {
    let bad = |m: String| ViewError::Unresolved {
        what: "UI line",
        spec: "render/blend-modes.md",
        message: m,
    };
    let key = FrameSetKey::new(path, FramePart::Tile(0)).map_err(|e| bad(e.to_string()))?;
    if !assets.frames.contains(&key) {
        let frames = (0..=255u8)
            .map(|i| IndexFrame::new(1, 1, 0, 0, vec![i]).map_err(|e| bad(e.to_string())))
            .collect::<Result<Vec<_>, _>>()?;
        assets.frames.insert(key.clone(), FrameSet { frames })?;
    }
    Ok(key)
}

/// The pixels of a UI line (`blend-modes.md` §8 r1): the first one in the
/// set the facts export reads as the call's `DrawLine` row.
fn line_sprites(l: &LineRequest) -> Result<Vec<UiSprite>, ViewError> {
    let bad = |m: String| ViewError::Unresolved {
        what: "UI line",
        spec: "render/blend-modes.md",
        message: m,
    };
    let key = |p: &str| FrameSetKey::new(p, FramePart::Tile(0)).map_err(|e| bad(e.to_string()));
    let (first, rest) = (key(UI_LINE_START)?, key(UI_LINE_REST)?);
    let pixels = crate::rules::blend::gdi_line_pixels(l.x0, l.y0, l.x1, l.y1)
        .map_err(|e| bad(e.to_string()))?;
    Ok(pixels
        .into_iter()
        .enumerate()
        .map(|(i, (x, y))| UiSprite {
            frame: ComponentFrame {
                set: if i == 0 { first.clone() } else { rest.clone() },
                index: usize::from(l.color),
            },
            x,
            y,
            shade: ShadeChain::default(),
            blend: BlendOp::Opaque,
            clip: None,
        })
        .collect())
}

fn rect_box(r: &RectRequest, size: FrameSize) -> Result<Option<(i32, i32, u32, u32)>, ViewError> {
    gdi_rectangle_box(size, r.x0, r.y0, r.x1, r.y1).map_err(|e| ViewError::Unresolved {
        what: "UI rectangle",
        spec: "render/blend-modes.md",
        message: e.to_string(),
    })
}

/// The sprite of a rectangle (`blend-modes.md` §8 r2) on the `size`
/// surface: its clamped box ([`gdi_rectangle_box`], `None`: nothing
/// drawn) and the write of its draw mode ([`gdi_rectangle_ops`]). The
/// resident rows and frame are [`ensure_rects`]'.
pub fn rect_sprite(
    r: &RectRequest,
    size: FrameSize,
    assets: &ViewAssets,
) -> Result<Option<UiSprite>, ViewError> {
    let Some((x, y, w, h)) = rect_box(r, size)? else {
        return Ok(None);
    };
    let rows = assets.color_rows.ok_or_else(|| ViewError::Unresolved {
        what: "UI rectangle",
        spec: "render/blend-modes.md",
        message: "the colour rows are not resident".into(),
    })?;
    let color = MapId(rows.0 + u32::from(r.color));
    let (shade, blend) =
        gdi_rectangle_ops(assets.shades.as_ref(), color, r.mode).ok_or_else(|| {
            ViewError::Unresolved {
                what: "UI rectangle",
                spec: "render/blend-modes.md",
                message: format!(
                    "draw mode {} needs the act's blend tables, not loaded",
                    r.mode
                ),
            }
        })?;
    Ok(Some(UiSprite {
        frame: ComponentFrame {
            set: rect_key(w, h),
            index: 0,
        },
        x,
        y,
        shade,
        blend,
        clip: None,
    }))
}

impl TextHooks for OriginalTextHooks {
    fn text_font(&self, style: TextStyle) -> Result<TextFont, ViewError> {
        original_text_font(style)
    }

    fn text_rules(&self) -> &dyn TextRules {
        &OriginalText
    }

    fn glyph_look(&self, color: i32, mode: u8) -> Result<(ShadeChain, BlendOp), ViewError> {
        original_glyph_look(self.colors.as_ref(), self.shades.as_ref(), color, mode)
    }
}

/// `ui/text.md` without the frame's text-color maps (they are frame data,
/// not rules): colored glyphs are errors.
impl TextHooks for Unspecified {
    fn text_font(&self, style: TextStyle) -> Result<TextFont, ViewError> {
        original_text_font(style)
    }

    fn text_rules(&self) -> &dyn TextRules {
        &OriginalText
    }

    fn glyph_look(&self, color: i32, mode: u8) -> Result<(ShadeChain, BlendOp), ViewError> {
        original_glyph_look(None, None, color, mode)
    }
}

impl UiRules for Unspecified {
    /// No panel file registry here: [`super::panel_art::PanelArtRules`]
    /// names the files (§22 r4).
    fn ui_image(&self, _: &ImageRequest, _: &ViewAssets) -> Result<UiSprite, ViewError> {
        Err(ViewError::unresolved("UI image", "ui/panels.md"))
    }

    fn ui_text(&self, req: &TextRequest, assets: &ViewAssets) -> Result<Vec<UiSprite>, ViewError> {
        text_sprites(self, req, assets)
    }

    /// Pass 11 (`render/draw-order.md` §10).
    fn ui_pass(&self) -> Result<u32, ViewError> {
        Ok(crate::scene::order::pass::UI)
    }
}

/// A UI rectangle as a scene clip rectangle (same integer edges).
pub fn clip_rect(r: crate::ui::Rect) -> Rect {
    Rect::new(r.x, r.y, u32::from(r.w), u32::from(r.h))
}

/// Appends the items of `draws` (emission order) to `items`. Asks for the
/// pass only when there is something to draw.
pub(super) fn ui_items<R: UiRules + ?Sized>(
    draws: &[UiDraw],
    rules: &R,
    assets: &ViewAssets,
    items: &mut Vec<DrawItem>,
) -> Result<(), ViewError> {
    if draws.is_empty() {
        return Ok(());
    }
    let key = DrawKey::new(
        rules.ui_pass()?,
        crate::scene::order::pass::UI_PANELS_MAJOR,
        0,
        0,
    )?;
    for (index, d) in draws.iter().enumerate() {
        let at = |error| ViewError::Ui {
            index,
            error: Box::new(error),
        };
        let (sprites, clip) = match d {
            UiDraw::Image(r) => (
                vec![rules.ui_image(r, assets).map_err(at)?],
                clip_rect(r.clip),
            ),
            UiDraw::Text(r) => (rules.ui_text(r, assets).map_err(at)?, clip_rect(r.clip)),
            // The GDI rectangle clamps to the surface, no other clip (§8 r2).
            UiDraw::Rect(r) => {
                let size = FrameSize::play();
                let s = rect_sprite(r, size, assets).map_err(at)?;
                (s.into_iter().collect(), size.rect())
            }
            UiDraw::Line(l) => (line_sprites(l).map_err(at)?, FrameSize::play().rect()),
        };
        for s in sprites {
            let id = assets.id(&s.frame.set, s.frame.index).map_err(at)?;
            let mut item = DrawItem::new(id, s.x, s.y);
            item.clip = match s.clip {
                Some(rows) => match rows.intersect(&clip) {
                    Some(c) => c,
                    None => continue,
                },
                None => clip,
            };
            item.shade = s.shade;
            item.blend = s.blend;
            item.key = key;
            item.tag = ItemTag::Ui(index as u32);
            items.push(item);
        }
    }
    Ok(())
}

/// The frame's UI input, filled by the input edge and drained by the root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiQueue(pub Vec<UiEvent>);

impl UiInput for UiQueue {
    fn drain(&mut self, out: &mut Vec<UiEvent>) {
        out.append(&mut self.0);
    }
}

/// What one UI frame did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiFrame {
    /// Events no panel took, in order. Left / right presses and releases
    /// become world-click dispatches ([`world_clicks`], `ui/controls.md`
    /// §6, §7 r5).
    pub unhandled: Vec<UiEvent>,
    /// Intents handed to the bridge this frame.
    pub sent: usize,
    /// The open panels' draw requests, bottom-most panel first.
    pub draws: Vec<UiDraw>,
}

/// One UI frame against the bridge (§A2): route `input` with the world
/// model as context, forward the queued intents through the bridge (sent
/// this frame, drained by the server's next pump, `bridge.md` §8 rule 3),
/// then collect the draw requests. `tick` is the bridge frame count.
pub fn run_ui<L: ServerLink>(
    root: &mut UiRoot,
    input: &mut dyn UiInput,
    bridge: &mut Bridge<L>,
    strings: &dyn StringLookup,
) -> Result<UiFrame, BridgeError> {
    run_ui_with(root, input, bridge, strings, None).map_err(|e| match e {
        UiRunError::Bridge(e) => e,
        UiRunError::Original(_) => unreachable!("no original UI"),
    })
}

/// Errors of [`run_ui_with`].
#[derive(Debug, thiserror::Error)]
pub enum UiRunError {
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[error(transparent)]
    Original(#[from] OriginalUiError),
}

/// [`run_ui`] with the original UI (`ui/panels.md`, [`OriginalUi`]): each
/// event is routed by the root, then the original UI applies what it
/// asked for (panel outputs in order, a hotkey action no panel took) and
/// the root mirrors the UI flags, before the next event. Intents still
/// leave only through the root and the bridge.
///
/// `input` and the returned `unhandled` events are in frame space; the
/// panels see them, and draw, in the UI box's space. The box is the
/// 800-wide layout placed [`FrameSize::ui_left`] pixels into the frame
/// (0 at 4:3, centred at 16:9), so the returned draws are moved by it.
pub fn run_ui_with<L: ServerLink>(
    root: &mut UiRoot,
    input: &mut dyn UiInput,
    bridge: &mut Bridge<L>,
    strings: &dyn StringLookup,
    mut original: Option<&mut OriginalUi>,
) -> Result<UiFrame, UiRunError> {
    let ui_left = FrameSize::play().ui_left();
    let mut unhandled = Vec::new();
    {
        let world = bridge.world();
        let ctx = UiCtx {
            tick: world.frames,
            world,
            strings,
        };
        let mut events = Vec::new();
        input.drain(&mut events);
        // The flags the delivered S→C outputs set since the last event
        // (S→C 0x63 opens ui 0x14), mirrored before routing.
        if let Some(o) = original.as_deref_mut() {
            o.npc_menu_poll(world, root, strings);
            o.shop_poll(world, root);
            o.cube_poll(world, root)?;
            // The close hooks of the outputs delivered since the last pass.
            o.flush_hooks(root);
            o.sync_root(root);
        }
        // The panels route in UI space (the 800-wide box, `ui_left` pixels
        // into the frame); an event no panel took goes on to the world
        // clicks in frame space.
        for frame_e in events {
            let e = frame_e.shifted(-ui_left);
            if let Some(o) = original.as_deref_mut() {
                o.before_event(e, world);
            }
            let routed = root.dispatch(e, &ctx);
            if let Some(o) = original.as_deref_mut() {
                o.after_event(root, e, routed)?;
            }
            if routed == Routed::Unhandled {
                unhandled.push(frame_e);
            }
        }
    }
    let sent = root.forward(bridge)?;
    let mut draws = Vec::new();
    let world = bridge.world();
    let ctx = UiCtx {
        tick: world.frames,
        world,
        strings,
    };
    root.draw(&ctx, &mut draws);
    place_in_frame(&mut draws, FrameSize::play());
    Ok(UiFrame {
        unhandled,
        sent,
        draws,
    })
}

/// Moves the panels' draws from the UI box's space into the frame, as
/// [`run_ui_with`] presents them: both the legacy default clip and the
/// current UI box mean the whole committed canvas (explicit local clips
/// remain local), and every draw moves [`FrameSize::ui_left`] pixels across.
pub fn place_in_frame(draws: &mut [crate::ui::draw::UiDraw], frame: FrameSize) {
    let ui_left = frame.ui_left();
    let screen = crate::ui::layout::Screen::for_frame(frame);
    for d in draws {
        if screen != crate::ui::layout::Screen::R640 {
            d.widen_clip(crate::ui::geom::FRAME, screen.frame_rect());
            d.widen_clip(screen.rect(), screen.frame_rect());
        }
        d.shift_x(ui_left);
    }
}

/// The clickable elements of the open panels with their ids
/// ([`crate::ui::ui_ids`]), in frame space: the panels lay them out in the
/// 800-wide UI box, which sits [`FrameSize::ui_left`] pixels into the frame,
/// so each is moved by that, as the panels' draws are in [`run_ui_with`].
pub fn ui_elements(
    root: &UiRoot,
    world: &crate::bridge::world::ClientWorld,
    strings: &dyn StringLookup,
) -> Vec<crate::ui::ui_ids::UiElement> {
    let ui_left = FrameSize::play().ui_left();
    let ctx = UiCtx {
        tick: world.frames,
        world,
        strings,
    };
    let mut out = root.ui_elements(&ctx);
    for e in &mut out {
        e.rect.x += ui_left;
        e.at.x += ui_left;
    }
    out
}

/// The world clicks of one loop pass (`ui/controls.md` §6 r1, r6; §7 r5):
/// each left / right press or release no panel took becomes its click
/// kind at the event position (left up at the current mouse), in event
/// order; then the held repeat (kinds 1 and 4); then the per-pass latch
/// is cleared (`0x00462920`). Returns what the UI layer applies (sounds,
/// hover calls, the pending record) and the interact outputs (object
/// sounds, effects) the click produced.
///
/// `mods` is the §4.3 r1 word (`RunMods::word`; 0 without the play
/// preview's bindings) and `local_at` the local player's position the
/// click reads (the preview's prediction, decision D2; `None`: the
/// model's cell).
pub fn world_clicks<L: ServerLink>(
    bridge: &mut Bridge<L>,
    st: &mut ClickState,
    view: ClickView,
    unhandled: &[UiEvent],
    mods: u32,
    local_at: Option<(u32, u32)>,
) -> Result<(Vec<ClickOut>, Vec<Output>), BridgeError> {
    let mut rest = Vec::new();
    let mut outputs = Vec::new();
    for e in unhandled {
        let (kind, at) = match *e {
            UiEvent::Press {
                button: PointerButton::Left,
                at,
            } => (Kind::LeftDown, Some((at.x, at.y))),
            UiEvent::Release {
                button: PointerButton::Left,
                ..
            } => (Kind::LeftUp, None),
            UiEvent::Press {
                button: PointerButton::Right,
                at,
            } => (Kind::RightDown, Some((at.x, at.y))),
            UiEvent::Release {
                button: PointerButton::Right,
                at,
            } => (Kind::RightUp, Some((at.x, at.y))),
            _ => continue,
        };
        // D2RUST's own, unverified (D1): the preview's hover pick reads the
        // event position.
        let view = match at {
            Some(p) if view.pick => ClickView { mouse: p, ..view },
            _ => view,
        };
        let (r, o) = bridge.world_click_at(st, view, kind, at, mods, local_at)?;
        rest.extend(r);
        outputs.extend(o);
    }
    let (r, o) = bridge.click_repeat_at(st, view, mods, local_at)?;
    rest.extend(r);
    outputs.extend(o);
    st.end_pass();
    Ok((rest, outputs))
}

#[cfg(test)]
mod text_tests {
    use d2_formats::font::{FontTable, Glyph};
    use d2_formats::palette::{Palette, Rgb};

    use super::*;
    use crate::frames::{FrameAnchor, FrameSet, IndexFrame};
    use crate::scene::FRAME_WIDTH;
    use crate::ui::{Point, TextOpts, FRAME};

    /// A left-panel text of the wide frame sits left of the UI box
    /// (x < 0 in box space, clipped to the whole-screen clip); placed in
    /// the frame it is inside the frame and its clip is the frame, so the
    /// explorer's text check (which measures placed draws) sees no overflow.
    #[test]
    fn placed_draws_are_in_frame_space() {
        let text = |x: i32| {
            UiDraw::Text(crate::ui::draw::TextRequest {
                text: vec![65],
                at: Point::new(x, 100),
                style: TextStyle { font: 0, color: 0 },
                opts: TextOpts::Draw {
                    centered: false,
                    block_w: None,
                    mode: 5,
                },
                clip: FRAME,
            })
        };
        let mut draws = [text(-116), text(873)];
        place_in_frame(&mut draws, FrameSize::WIDE);
        for (d, x) in draws.iter().zip([18, 1007]) {
            let UiDraw::Text(t) = d else { unreachable!() };
            assert_eq!(t.at.x, x);
            assert_eq!(t.clip, crate::ui::Rect::new(0, 0, 1068, 600));
        }
        let mut draws = [text(16)];
        place_in_frame(&mut draws, FrameSize::D2RUST);
        let UiDraw::Text(t) = &draws[0] else {
            unreachable!()
        };
        assert_eq!((t.at.x, t.clip), (16, FRAME));
    }

    // Covers: specs/ui/text.md §1 r3, §13
    #[test]
    fn font_ids_name_the_tsv_files() {
        let f = original_text_font(TextStyle { font: 1, color: 0 }).unwrap();
        assert_eq!(f.table.as_str(), "data/local/font/latin/font16.tbl");
        assert_eq!(
            f.glyphs,
            FrameSetKey::new("data/local/font/latin/font16.dc6", FramePart::Dir(0)).unwrap()
        );
        assert!(original_text_font(TextStyle { font: 14, color: 0 }).is_err());
    }

    fn pl2() -> Vec<u8> {
        let mut p = vec![0u8; TEXT_COLOR_MAP_OFFSET + 256 * TEXT_COLORS];
        for k in 0..TEXT_COLORS {
            for s in 0..256 {
                p[TEXT_COLOR_MAP_OFFSET + 256 * k + s] = (k * 16 + s % 16) as u8;
            }
        }
        p
    }

    // Covers: specs/ui/text.md §4 r3, §4 r4; specs/render/shading.md §6 r5
    #[test]
    fn glyph_look_is_text_color_map_k() {
        let mut table = MapTable::new();
        table.push([0xEE; 256]);
        let colors = TextColors::push(&mut table, &pl2()).unwrap();
        assert_eq!(table.len(), 13);
        for k in 1..TEXT_COLORS {
            let row = table.get(colors.maps[k - 1]).unwrap();
            assert_eq!(row[3], (k * 16 + 3) as u8, "map {k}");
        }
        let hooks = OriginalTextHooks {
            colors: Some(colors),
            shades: None,
        };
        assert_eq!(
            hooks.glyph_look(0, 5).unwrap(),
            (ShadeChain::EMPTY, BlendOp::Opaque)
        );
        let (shade, blend) = hooks.glyph_look(3, 5).unwrap();
        assert_eq!(
            (shade.maps(), blend),
            (&[colors.maps[2]][..], BlendOp::Opaque)
        );
        // Outside 0–12 (open question 2), another draw mode, no maps.
        assert!(hooks.glyph_look(13, 5).is_err());
        assert!(hooks.glyph_look(-1, 5).is_err());
        assert!(hooks.glyph_look(0, 3).is_err());
        assert!(OriginalTextHooks::default().glyph_look(1, 5).is_err());
        assert_eq!(
            Unspecified.glyph_look(0, 5).unwrap(),
            (ShadeChain::EMPTY, BlendOp::Opaque)
        );
        // The file must hold all 13 maps.
        let short = &pl2()[..TEXT_COLOR_MAP_OFFSET + 256 * 12];
        assert!(TextColors::push(&mut MapTable::new(), short).is_err());
    }

    /// A one-direction DC6 of `frames` frames, each `w` × `h` literal
    /// pixels (`formats/dc6.md`).
    fn dc6(frames: u32, w: u32, h: u32) -> Vec<u8> {
        let mut rows = Vec::new();
        for _ in 0..h {
            rows.push(w as u8);
            rows.extend((0..w).map(|i| 1 + i as u8));
            rows.push(0x80);
        }
        let mut d = Vec::new();
        for v in [6i32, 1, 0] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&[0xEE; 4]);
        d.extend_from_slice(&1u32.to_le_bytes());
        d.extend_from_slice(&frames.to_le_bytes());
        let mut at = d.len() + 4 * frames as usize;
        let mut body = Vec::new();
        for _ in 0..frames {
            d.extend_from_slice(&(at as u32).to_le_bytes());
            for v in [0u32, w, h, 0, 0, 0, 0, rows.len() as u32] {
                body.extend_from_slice(&v.to_le_bytes());
            }
            body.extend_from_slice(&rows);
            body.extend_from_slice(&[0xEE; 3]);
            at += 32 + rows.len() + 3;
        }
        d.extend(body);
        d
    }

    /// A synthetic `.tbl` of 256 records, each 6 wide.
    fn tbl() -> Vec<u8> {
        let mut d = b"Woo!".to_vec();
        d.extend_from_slice(&[1, 0, 0, 0, 0, 1, 10, 0]);
        for i in 0..256u16 {
            d.extend_from_slice(&i.to_le_bytes());
            d.extend_from_slice(&[0, 6, 10, 0, 0, 0]);
            d.extend_from_slice(&i.to_le_bytes());
            d.extend_from_slice(&[0; 4]);
        }
        d
    }

    // Covers: specs/ui/text.md §1 r3
    #[test]
    fn text_assets_load_once_per_font() {
        use crate::assets::path::MemorySource;
        let mut src = MemorySource::default();
        let style = TextStyle { font: 1, color: 0 };
        let font = original_text_font(style).unwrap();
        src.insert(font.table.as_str(), tbl());
        src.insert(font.glyphs.path(), dc6(256, 4, 4));
        let loader = TextAssetLoader {
            source: std::sync::Arc::new(src),
        };
        let mut a = ViewAssets::new(Palette {
            colors: [Rgb::default(); 256],
        });
        let req = |style| {
            UiDraw::Text(TextRequest {
                text: vec![u16::from(b'A')],
                at: Point::new(10, 20),
                style,
                opts: TextOpts::default(),
                clip: FRAME,
            })
        };
        loader.ensure(&[req(style), req(style)], &mut a).unwrap();
        assert_eq!(a.fonts.len(), 1);
        assert!(a.frames.contains(&font.glyphs));
        // The text then draws through `text_sprites`.
        let UiDraw::Text(r) = req(style) else {
            unreachable!()
        };
        assert_eq!(
            text_sprites(&OriginalTextHooks::default(), &r, &a)
                .unwrap()
                .len(),
            1
        );
        // A font in no archive is an error.
        let other = req(TextStyle { font: 6, color: 0 });
        assert!(loader.ensure(&[other], &mut a).is_err());
    }

    // Covers: specs/ui/text.md §4 r1, §4 r2
    #[test]
    fn glyph_cell_ends_at_the_pen_row() {
        let a = font16_assets();
        let req = TextRequest {
            text: vec![u16::from(b'A')],
            at: Point::new(104, 200),
            style: TextStyle { font: 1, color: 0 },
            opts: TextOpts::default(),
            clip: FRAME,
        };
        let s = text_sprites(&OriginalTextHooks::default(), &req, &a).unwrap();
        // Font16 `A` at pen (104, 200): columns 104–117, rows 185–200.
        assert_eq!(s.len(), 1);
        assert_eq!((s[0].x, s[0].y, s[0].frame.index), (104, 185, 65));
    }

    /// Assets with Font16 (font 1): every glyph a 14 × 16 bottom-anchored
    /// cel, advance 12, height 10.
    fn font16_assets() -> ViewAssets {
        let mut a = ViewAssets::new(Palette {
            colors: [Rgb::default(); 256],
        });
        let font = original_text_font(TextStyle { font: 1, color: 0 }).unwrap();
        let frame = IndexFrame {
            width: 14,
            height: 16,
            x_off: 0,
            y_off: 0,
            anchor: FrameAnchor::Bottom,
            pixels: vec![1; 14 * 16],
        };
        a.frames
            .insert(
                font.glyphs.clone(),
                FrameSet {
                    frames: vec![frame; 256],
                },
            )
            .unwrap();
        let glyphs = (0..256u16)
            .map(|i| Glyph {
                code: i,
                unknown1: 0,
                width: 12,
                height: 10,
                unknown2: 1,
                unknown3: 0,
                frame: i,
                unknown5: 0,
            })
            .collect();
        a.fonts.insert(
            font.table.clone(),
            FontTable {
                version: 1,
                unknown: 0,
                count: 256,
                height: 10,
                width: 0,
                glyphs,
            },
        );
        a
    }

    // The §9 details r2 "row window" test vectors.
    // Covers: specs/ui/text.md §9
    #[test]
    fn vertical_rows_are_the_row_window() {
        // B = 100 (a 16-row cel from row 85), skip 3, lines 10: rows 88 … 97.
        assert_eq!(
            vertical_rows(85, 16, 3, 10),
            Some(Rect::new(0, 88, FRAME_WIDTH, 10))
        );
        // B = 5 (a 20-row cel), skip 0, lines 18: top −13 → 0, rows 1 … 5.
        assert_eq!(
            vertical_rows(-14, 20, 0, 18),
            Some(Rect::new(0, 1, FRAME_WIDTH, 5))
        );
        // skip + lines past the cel height: rows 0, h (the whole cel).
        assert_eq!(
            vertical_rows(85, 16, 10, 10),
            Some(Rect::new(0, 85, FRAME_WIDTH, 16))
        );
        // lines 0, bottom above the screen, top below it: nothing.
        assert_eq!(vertical_rows(85, 16, 3, 0), None);
        assert_eq!(vertical_rows(-30, 16, 0, 10), None);
        assert_eq!(vertical_rows(600, 16, 0, 10), None);
    }

    // B013 (NPC text lag): the NPC dialog's partly visible lines are
    // vertical-window draws (`ui/messages.md` §7 r4); the play frame
    // whose UI has one must build (it was an error, so the preview
    // dropped every such frame while the text scrolled).
    // Covers: specs/ui/text.md §9
    #[test]
    fn vertical_window_text_builds_its_rows() {
        let a = font16_assets();
        let req = TextRequest {
            text: "ÿc4AB".encode_utf16().collect(),
            at: Point::new(104, 200),
            style: TextStyle { font: 1, color: 0 },
            opts: TextOpts::Vertical { skip: 3, lines: 10 },
            clip: FRAME,
        };
        let s = text_sprites(&OriginalTextHooks::default(), &req, &a).unwrap();
        // `ÿc4` skipped, no color: `A` and `B` from the pen, 12 apart; the
        // cel's bottom row is the pen row 200, so rows 188 … 197.
        let rows = Rect::new(0, 188, FRAME_WIDTH, 10);
        let got: Vec<_> = s
            .iter()
            .map(|s| (s.x, s.y, s.frame.index, s.clip))
            .collect();
        assert_eq!(
            got,
            [(104, 185, 65, Some(rows)), (116, 185, 66, Some(rows))]
        );
        assert!(s.iter().all(|s| s.shade == ShadeChain::EMPTY));
        let mut items = Vec::new();
        ui_items(&[UiDraw::Text(req.clone())], &Unspecified, &a, &mut items).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.clip == rows));
        // skip < 0: nothing drawn, no error.
        let none = TextRequest {
            opts: TextOpts::Vertical {
                skip: -1,
                lines: 10,
            },
            ..req
        };
        assert!(text_sprites(&OriginalTextHooks::default(), &none, &a)
            .unwrap()
            .is_empty());
    }

    // q-perf: the memoized font is the font built fresh, every call.
    #[test]
    fn the_memoized_font_equals_the_built_one() {
        for font in 0..14u16 {
            let style = TextStyle { font, color: 0 };
            let built = build_text_font(style).unwrap();
            assert_eq!(original_text_font(style).unwrap(), built);
            assert_eq!(original_text_font(style).unwrap(), built);
        }
        assert!(original_text_font(TextStyle { font: 14, color: 0 }).is_err());
    }
}

#[cfg(test)]
#[path = "ui_draw_sink_tests.rs"]
mod draw_sink_tests;
