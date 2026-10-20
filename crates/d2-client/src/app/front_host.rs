//! The Bevy host of the front end ([`crate::ui::front_end`]): it runs
//! [`FrontEnd::tick`] every 40 ms, feeds it the window's mouse and keys,
//! and draws [`FrontEnd::draw`] with the DC6 art and the sky palette of the
//! player's archives into an 800×600 sprite. Text is drawn as glyphs, art
//! as frame `frame` of its DC6 with the bottom-left at the control's
//! position. The game starts with the character the select / create
//! screens chose (`front_start::StartChoice`). No game logic lives here.
//!
//! In `play` the front end is the session's [`Screen::Front`]
//! ([`add_front_session`], `app::session`): it shares the one window with
//! the game, and a game load switches the screen instead of closing the
//! window. [`run_front_end`] still runs it alone in its own app (tools and
//! tests); [`Handoff`] is that standalone host's way to keep its window up
//! for a game started elsewhere.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::MouseWheel;
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use d2_formats::cof::Cof;
use d2_formats::dc6::{Dc6, Dc6Frame};
use d2_formats::dcc::Dcc;
use d2_formats::font::FontTable;
use d2_formats::palette::{Palette, Pl2};

use crate::app::front_cels::{cel_key, Cel, CelGlyph, CelPixels};
use crate::app::front_fill;
use crate::app::front_ui::{
    FrontUiCels, FrontUiFrame, FrontUiImages, FrontUiJob, FrontUiOn, FrontUiQuad,
};
use crate::app::session::Screen;
use crate::assets::path::FileSource;
use crate::audio::environment::frontend_stream::FrontMusic;
use crate::gpu_compositor::ui_pass::UiPlacement;
use crate::ui::front_end::glyphs::text_quads;
use crate::ui::front_end::screens::create::{NewCharacter, NewCharacterSink};
use crate::ui::front_end::screens::credits;

use crate::ui::front_end::{
    DrawItem, FrontEnd, FrontInput, Outcome, Registry, SaveFolder, SKY_PALETTE, TICK_MS,
};
use crate::ui::geom::Point;
use crate::ui::text::font_info;

pub const WIDTH: u32 = 800;
pub const HEIGHT: u32 = 600;

/// `<dir>/*.d2s` exists.
pub struct DirSaves(pub std::path::PathBuf);

impl SaveFolder for DirSaves {
    fn has_saves(&self) -> bool {
        std::fs::read_dir(&self.0).is_ok_and(|mut d| {
            d.any(|e| {
                e.is_ok_and(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("d2s"))
                })
            })
        })
    }
}

/// DC6 art from the archives and the screen's palette.
pub struct FrontArt {
    source: Arc<dyn FileSource>,
    /// The palette files in use and their tables (`pal.dat`, `pal.pl2`).
    palette_files: [&'static str; 2],
    palette: Option<Palette>,
    pl2: Option<Pl2>,
    cache: HashMap<&'static str, Option<Dc6>>,
    /// In-game UI art by its panel-file name ([`DrawItem::UiCel`]).
    ui_cache: HashMap<String, Option<Dc6>>,
    fonts: HashMap<u16, Option<(FontTable, Dc6)>>,
    /// UTF-16 text of a string id (button labels); `None`: labels blank.
    strings: Option<Box<dyn Fn(u32) -> Vec<u16>>>,
    /// The paper dolls' tables (`DrawItem::Doll`); `None`: no dolls.
    dolls: Option<crate::ui::front_end::doll::DollTables>,
    cofs: HashMap<String, Option<Cof>>,
    dccs: HashMap<String, Option<Dcc>>,
    /// Item colour maps by file name (21 maps of 256).
    colour_files: HashMap<&'static str, Option<Vec<u8>>>,
    /// Per doll key: draws so far and the 8.8 frame phase.
    doll_phase: HashMap<u32, (u32, u32)>,
    doll_epoch: u64,
}

fn read_palette(source: &dyn FileSource, files: [&str; 2]) -> (Option<Palette>, Option<Pl2>) {
    use crate::assets::path as p;
    (
        p::read_palette(source, files[0]).and_then(Result::ok),
        p::read_pl2(source, files[1]).and_then(Result::ok),
    )
}

impl FrontArt {
    pub fn new(source: Arc<dyn FileSource>) -> Self {
        let (palette, pl2) = read_palette(&*source, SKY_PALETTE);
        Self {
            source,
            palette_files: SKY_PALETTE,
            palette,
            pl2,
            cache: HashMap::new(),
            ui_cache: HashMap::new(),
            fonts: HashMap::new(),
            strings: None,
            dolls: None,
            cofs: HashMap::new(),
            dccs: HashMap::new(),
            colour_files: HashMap::new(),
            doll_phase: HashMap::new(),
            doll_epoch: u64::MAX,
        }
    }

    /// The tables the character-select paper dolls are built from.
    pub fn with_dolls(mut self, t: crate::ui::front_end::doll::DollTables) -> Self {
        self.dolls = Some(t);
        self
    }

    fn cof(&mut self, path: &str) -> Option<&Cof> {
        let source = &self.source;
        self.cofs
            .entry(path.to_owned())
            .or_insert_with(|| crate::assets::path::read_cof(&**source, path)?.ok())
            .as_ref()
    }

    fn dcc(&mut self, path: &str) -> Option<&Dcc> {
        let source = &self.source;
        self.dccs
            .entry(path.to_owned())
            .or_insert_with(|| crate::assets::path::read_dcc(&**source, path)?.ok())
            .as_ref()
    }

    /// The 256-byte map `i` of item colour file `name`.
    fn colour_map(&mut self, name: &'static str, i: usize) -> Option<[u8; 256]> {
        let source = &self.source;
        let bytes = self
            .colour_files
            .entry(name)
            .or_insert_with(|| {
                source
                    .read_file(&format!(r"data\global\items\palette\{name}.dat"))?
                    .ok()
            })
            .as_ref()?;
        bytes.get(i * 256..i * 256 + 256)?.try_into().ok()
    }

    /// Switches to the palette `files` the current screen loaded
    /// (`FrontEnd::palette`); `None` keeps the current one.
    pub fn use_palette(&mut self, files: Option<[&'static str; 2]>) {
        let Some(files) = files else { return };
        if files != self.palette_files {
            let (palette, pl2) = read_palette(&*self.source, files);
            self.palette_files = files;
            self.palette = palette;
            self.pl2 = pl2;
        }
    }

    /// The string-table lookup the button labels use.
    pub fn with_strings(mut self, f: impl Fn(u32) -> Vec<u16> + 'static) -> Self {
        self.strings = Some(Box::new(f));
        self
    }

    fn font(&mut self, id: u16) -> Option<&(FontTable, Dc6)> {
        let source = &self.source;
        self.fonts
            .entry(id)
            .or_insert_with(|| {
                let info = font_info(id)?;
                let table = crate::assets::path::read_font_table(&**source, info.tbl_path)?.ok()?;
                let dc6 = crate::assets::path::read_dc6(&**source, info.dc6_path)?.ok()?;
                Some((table, dc6))
            })
            .as_ref()
    }

    /// Width of `text` in font `id`, from the font table's advances (0 when
    /// the font is missing).
    pub fn text_width(&mut self, id: u16, text: &[u16]) -> i32 {
        self.font(id).map_or(0, |(t, _)| {
            crate::ui::front_end::glyphs::text_width(t, text)
        })
    }

    fn dc6(&mut self, file: &'static str) -> Option<&Dc6> {
        let source = &self.source;
        self.cache
            .entry(file)
            .or_insert_with(|| {
                let name = format!(r"data\global\ui\{file}.dc6");
                crate::assets::path::read_dc6(&**source, &name)?.ok()
            })
            .as_ref()
    }

    /// The in-game UI art `name` (a panel-file name), read from the path
    /// the game's panels read it from.
    fn ui_dc6(&mut self, name: &str) -> Option<&Dc6> {
        let source = &self.source;
        self.ui_cache
            .entry(name.to_owned())
            .or_insert_with(|| {
                let path = crate::world_view::panel_art::archive_name(name);
                crate::assets::path::read_dc6(&**source, &path)?.ok()
            })
            .as_ref()
    }
}

/// The 8-bit frame the front end draws into (every write is a palette index;
/// blends read the PL2 tables).
struct IndexFrame {
    width: u32,
    height: u32,
    px: Vec<u8>,
}

impl IndexFrame {
    /// Writes source index `s` at (x, y) with draw mode `mode`: 5 (and
    /// every mode without a table, or no PL2) writes `s`; 0–2 alpha, 3
    /// additive, 4 multiplicative, 6 max-component read `T[256·d + s]`
    /// (row = destination).
    fn put(&mut self, x: i32, y: i32, s: u8, mode: u8, pl2: Option<&Pl2>) {
        if !(0..self.width as i32).contains(&x) || !(0..self.height as i32).contains(&y) {
            return;
        }
        let i = (y as u32 * self.width + x as u32) as usize;
        let d = usize::from(self.px[i]);
        let s = usize::from(s);
        self.px[i] = match (mode, pl2) {
            // Mode 0 is alpha level 2, mode 2 level 0.
            (0..=2, Some(t)) => t.alpha_blend[usize::from(2 - mode)][d][s],
            (3, Some(t)) => t.additive_blend[d][s],
            (4, Some(t)) => t.multiplicative_blend[d][s],
            (6, Some(t)) => t.max_component_blend[d][s],
            _ => s as u8,
        };
    }

    /// Draws one cel ([`Cel`]): an image's indices (0 transparent) through
    /// its map, or a solid rectangle's one index, each written with the
    /// cel's draw mode (through the PL2 tables when the cel uses them).
    fn draw(&mut self, c: &Cel<'_>, pl2: Option<&Pl2>) {
        let pl2 = if c.tables { pl2 } else { None };
        match c.pixels {
            CelPixels::Image { px, w, .. } => {
                if w == 0 {
                    return;
                }
                for (i, &idx) in px.iter().enumerate() {
                    if idx != 0 {
                        let s = c.map.as_ref().map_or(idx, |m| m[usize::from(idx)]);
                        let (col, row) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
                        self.put(c.x + col, c.y + row, s, c.mode, pl2);
                    }
                }
            }
            CelPixels::Solid { w, h, index } => {
                for y in c.y..c.y + h as i32 {
                    for x in c.x..c.x + w as i32 {
                        self.put(x, y, index, c.mode, pl2);
                    }
                }
            }
        }
    }
}

/// The palette index nearest to `rgb` (D2RUST's own boxes).
fn nearest(pal: &Palette, rgb: [u8; 3]) -> u8 {
    let dist = |i: u8| {
        let c = pal.colors[usize::from(i)];
        let e = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        e(c.r, rgb[0]) + e(c.g, rgb[1]) + e(c.b, rgb[2])
    };
    (0..=255u8).min_by_key(|&i| dist(i)).unwrap_or(0)
}

/// Composes the draw items into 800×600 RGBA (opaque black background):
/// every item writes palette indices into one 8-bit frame (draw modes and
/// text colours through the PL2 blend tables), shown with the screen's
/// palette.
pub fn compose(items: &[DrawItem], art: Option<&mut FrontArt>) -> Vec<u8> {
    compose_split(items, items.len(), art, |_, _| {})
}

/// [`compose`], handing `under` the 8-bit frame and its palette once the
/// first `split` items are drawn (the picture under the pointer, which
/// [`super::front_fill`] blurs into the side fill).
pub fn compose_split(
    items: &[DrawItem],
    split: usize,
    art: Option<&mut FrontArt>,
    under: impl FnOnce(&[u8], Option<&Palette>),
) -> Vec<u8> {
    compose_canvas_split(items, split, art, (WIDTH, HEIGHT), under)
}

/// Compose a committed front-end canvas without scaling its native art.
pub fn compose_canvas_split(
    items: &[DrawItem],
    split: usize,
    art: Option<&mut FrontArt>,
    size: (u32, u32),
    under: impl FnOnce(&[u8], Option<&Palette>),
) -> Vec<u8> {
    let (index, shown) = compose_index_canvas(items, split, art, size, under);
    let mut px = vec![0u8; (size.0 * size.1 * 4) as usize];
    for (o, &i) in px.as_chunks_mut::<4>().0.iter_mut().zip(&index) {
        let c = shown
            .as_ref()
            .map_or(Default::default(), |p| p.colors[usize::from(i)]);
        *o = [c.r, c.g, c.b, 255];
    }
    px
}

/// The 8-bit index frame (800 × 600, row-major) and the palette shown with
/// it: the pair the front-end picture tests hash.
pub fn compose_index(items: &[DrawItem], art: Option<&mut FrontArt>) -> (Vec<u8>, Option<Palette>) {
    compose_index_split(items, items.len(), art, |_, _| {})
}

/// [`compose_index`] with [`compose_split`]'s `under` call.
fn compose_index_split(
    items: &[DrawItem],
    split: usize,
    art: Option<&mut FrontArt>,
    under: impl FnOnce(&[u8], Option<&Palette>),
) -> (Vec<u8>, Option<Palette>) {
    compose_index_canvas(items, split, art, (WIDTH, HEIGHT), under)
}

fn compose_index_canvas(
    items: &[DrawItem],
    split: usize,
    art: Option<&mut FrontArt>,
    size: (u32, u32),
    under: impl FnOnce(&[u8], Option<&Palette>),
) -> (Vec<u8>, Option<Palette>) {
    let mut frame = IndexFrame {
        width: size.0,
        height: size.1,
        px: vec![0u8; (size.0 * size.1) as usize],
    };
    let mut art = art;
    // The palette shown: the PL2's, else `pal.dat`. Drawing doesn't change
    // it.
    let shown = art.as_deref().and_then(|a| {
        a.pl2
            .as_ref()
            .map(|t| t.base_palette.clone())
            .or_else(|| a.palette.clone())
    });
    let (base, top) = items.split_at(split.min(items.len()));
    let pl2 = art.as_deref_mut().and_then(|a| a.pl2.take());
    if let Some(a) = art.as_deref_mut() {
        for it in base {
            draw_item(&mut frame, a, pl2.as_ref(), it);
        }
    }
    under(&frame.px, shown.as_ref());
    if let Some(a) = art {
        for it in top {
            draw_item(&mut frame, a, pl2.as_ref(), it);
        }
        a.pl2 = pl2;
    }
    (frame.px, shown)
}

/// The fact-file path of a front-end DC6 (`facts/render/scenes/frontend-*`):
/// lower case, `/` separators, under `data/global/ui/`.
fn fact_path(file: &str) -> String {
    format!(
        "data/global/ui/{}.dc6",
        file.replace('\\', "/").to_lowercase()
    )
}

/// The `draws.tsv` rows of one front-end
/// frame: `StartDraw`, `ClearScreen`, then a `CelDraw` per art / blend cel
/// and a `CelDrawColor` per text glyph, in draw order. Dolls and boxes
/// (D2RUST's own) are not rows. Measured cells come from the DC6 frames.
/// A cel's width, height and offsets.
type Dims = (u32, u32, i32, i32);

fn dims(f: &Dc6Frame) -> Dims {
    (f.width, f.height, f.offset_x, f.offset_y)
}

pub fn draw_rows(items: &[DrawItem], art: &mut FrontArt) -> Vec<Vec<String>> {
    const NA: &str = "-";
    let row = |op: &str, file: String, frame: u32, f: Option<Dims>, at: Point, mode: &str| {
        let m = |v: Option<String>| v.unwrap_or_else(|| "?".into());
        let color = op == "CelDrawColor";
        let tail = |on: &str| if color { "?".to_owned() } else { on.to_owned() };
        vec![
            String::new(),
            op.to_owned(),
            file,
            "0".into(),
            frame.to_string(),
            NA.into(),
            at.x.to_string(),
            at.y.to_string(),
            m(f.map(|f| f.0.to_string())),
            m(f.map(|f| f.1.to_string())),
            m(f.map(|f| f.2.to_string())),
            m(f.map(|f| f.3.to_string())),
            if color { "?".into() } else { mode.to_owned() },
            tail("0xffffffff"),
            tail("0"),
            NA.into(),
            NA.into(),
        ]
    };
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut start = vec![NA.to_owned(); 17];
    start[1] = "StartDraw".into();
    start[6] = "0".into();
    start[7] = "0".into();
    out.push(start);
    let mut clear = vec![NA.to_owned(); 17];
    clear[1] = "ClearScreen".into();
    clear[6] = "0".into();
    out.push(clear);
    for it in items {
        match it {
            DrawItem::Art { file, frame, at } => {
                let f = art
                    .dc6(file)
                    .and_then(|d| d.frames.get(*frame as usize))
                    .map(dims);
                out.push(row("CelDraw", fact_path(file), *frame, f, *at, "5"));
            }
            DrawItem::Blend {
                file,
                frame,
                at,
                mode,
                boxed,
            } => {
                let f = art
                    .dc6(file)
                    .and_then(|d| d.frames.get(*frame as usize))
                    .map(dims);
                let mut r = row(
                    "CelDraw",
                    fact_path(file),
                    *frame,
                    f,
                    *at,
                    &mode.to_string(),
                );
                // Boxed (bottom-left at `at`, offsets unused): the row
                // carries the position the offsets move from, same pixels.
                if let (true, Some(f)) = (*boxed, f) {
                    r[6] = (at.x - f.2).to_string();
                    r[7] = (at.y - f.3).to_string();
                }
                out.push(r);
            }
            DrawItem::Text { .. } => {
                let none = |_| Vec::new();
                let strings = art.strings.take();
                let resolve: &dyn Fn(u32) -> Vec<u16> = match &strings {
                    Some(f) => f,
                    None => &none,
                };
                let font = match it {
                    DrawItem::Text { font, .. } => *font,
                    _ => unreachable!(),
                };
                let table = art.font(font).map(|(t, _)| t.clone());
                let quads = text_quads(it, resolve, &|_| table.clone());
                art.strings = strings;
                for q in quads {
                    let Some(info) = font_info(q.font) else {
                        continue;
                    };
                    let f = art
                        .font(q.font)
                        .and_then(|(_, d)| d.frames.get(usize::from(q.frame)))
                        .map(dims);
                    let file = info.dc6_path.replace('\\', "/").to_lowercase();
                    out.push(row("CelDrawColor", file, u32::from(q.frame), f, q.at, "?"));
                }
            }
            DrawItem::UiCel {
                file,
                frame,
                at,
                mode,
            } => {
                let f = art
                    .ui_dc6(file)
                    .and_then(|d| d.frames.get(*frame as usize))
                    .map(dims);
                let path = crate::world_view::panel_art::archive_name(file)
                    .replace('\\', "/")
                    .to_lowercase();
                out.push(row("CelDraw", path, *frame, f, *at, &mode.to_string()));
            }
            DrawItem::Doll { .. }
            | DrawItem::Rect { .. }
            | DrawItem::Shade { .. }
            | DrawItem::Border { .. }
            | DrawItem::Dim { .. } => {}
        }
    }
    for (i, r) in out.iter_mut().enumerate() {
        r[0] = i.to_string();
    }
    out
}

fn draw_item(frame: &mut IndexFrame, a: &mut FrontArt, pl2: Option<&Pl2>, it: &DrawItem) {
    let size = (frame.width, frame.height);
    item_cels(a, pl2, it, size, &mut |c| frame.draw(&c, pl2));
}

/// A DC6 frame as a cel: its bottom-left at `at`, moved by the frame's
/// offsets when `offsets` is set (index 0 is transparent).
fn dc6_cel(
    f: &Dc6Frame,
    key: u64,
    at: Point,
    offsets: bool,
    mode: u8,
    map: Option<[u8; 256]>,
) -> Cel<'_> {
    let (ox, oy) = if offsets {
        (f.offset_x, f.offset_y)
    } else {
        (0, 0)
    };
    Cel {
        key,
        pixels: CelPixels::Image {
            px: &f.pixels,
            w: f.width,
            h: f.height,
        },
        x: at.x + ox,
        y: at.y + oy - f.height as i32 + 1,
        mode,
        tables: true,
        map,
        glyph: None,
    }
}

/// A solid rectangle as a cel (nothing for an empty one).
fn solid(
    emit: &mut dyn FnMut(Cel<'_>),
    (x, y, w, h): (i32, i32, i32, i32),
    index: u8,
    mode: u8,
    tables: bool,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    emit(Cel {
        key: 0,
        pixels: CelPixels::Solid {
            w: w as u32,
            h: h as u32,
            index,
        },
        x,
        y,
        mode,
        tables,
        map: None,
        glyph: None,
    });
}

/// The cels of one draw item, in draw order, handed to `emit`: the one
/// list both the CPU picture and the native UI pass draw.
pub(crate) fn item_cels(
    a: &mut FrontArt,
    pl2: Option<&Pl2>,
    it: &DrawItem,
    size: (u32, u32),
    emit: &mut dyn FnMut(Cel<'_>),
) {
    let Some(pal) = a.palette.clone() else {
        return;
    };
    match it {
        DrawItem::Art { file, frame: n, at } => {
            let key = cel_key("dc6", file, *n, 0);
            if let Some(f) = a.dc6(file).and_then(|d| d.frames.get(*n as usize)) {
                emit(dc6_cel(f, key, *at, true, 5, None));
            }
        }
        DrawItem::Doll {
            class,
            mode,
            components,
            colours,
            at,
            draw_mode,
            key,
            epoch,
            ticks,
        } => doll_cels(
            a,
            (*class, *mode, components, colours),
            (*at, *draw_mode),
            (*key, *epoch, *ticks),
            emit,
        ),
        // D2RUST's own boxes: a dark box and (`Border`) a 1 px outline.
        DrawItem::Rect { at, w, h } => {
            let c = nearest(&pal, [8, 8, 12]);
            solid(emit, (at.x, at.y, *w, *h), c, 5, false);
        }
        // The in-game UI's art and rectangles (the Options screen): a cel
        // with its own draw mode; a rectangle blends its colour with the
        // picture in modes 0–2 and is written opaque otherwise.
        DrawItem::UiCel {
            file,
            frame: n,
            at,
            mode,
        } => {
            let key = cel_key("ui", file, *n, 0);
            if let Some(f) = a.ui_dc6(file).and_then(|d| d.frames.get(*n as usize)) {
                emit(dc6_cel(f, key, *at, true, *mode, None));
            }
        }
        DrawItem::Shade {
            at,
            w,
            h,
            color,
            mode,
        } => {
            let mode = if *mode <= 2 { *mode } else { 5 };
            solid(emit, (at.x, at.y, *w, *h), *color, mode, true);
        }
        // With a modal pop-up up, the whole picture is dimmed: colour 0
        // through draw mode `mode` (1, half alpha) over every pixel but the
        // last column and row.
        DrawItem::Dim { mode } => {
            let (w, h) = (size.0 as i32 - 1, size.1 as i32 - 1);
            solid(emit, (0, 0, w, h), 0, *mode, true);
        }
        // A 1 px outline: the top and bottom rows, then the left and right
        // columns.
        DrawItem::Border { at, w, h } => {
            let c = nearest(&pal, [120, 100, 60]);
            let (x, y, w, h) = (at.x, at.y, *w, *h);
            if w > 0 {
                solid(emit, (x, y, w, 1), c, 5, false);
                solid(emit, (x, y + h - 1, w, 1), c, 5, false);
            }
            if h > 0 {
                solid(emit, (x, y, 1, h), c, 5, false);
                solid(emit, (x + w - 1, y, 1, h), c, 5, false);
            }
        }
        // Draw mode `mode` through the PL2 table (3 = additive); the
        // frame's offsets add to the position unless `boxed`.
        DrawItem::Blend {
            file,
            frame: n,
            at,
            mode,
            boxed,
        } => {
            let key = cel_key("dc6", file, *n, 0);
            if let Some(f) = a.dc6(file).and_then(|d| d.frames.get(*n as usize)) {
                emit(dc6_cel(f, key, *at, !*boxed, *mode, None));
            }
        }
        DrawItem::Text { font, label, .. } => {
            let none = |_| Vec::new();
            let strings = a.strings.take();
            let resolve: &dyn Fn(u32) -> Vec<u16> = match &strings {
                Some(f) => f,
                None => &none,
            };
            let table = a.font(*font).map(|(t, _)| t.clone());
            let quads = text_quads(it, resolve, &|_| table.clone());
            a.strings = strings;
            // A button label draws every glyph multiplied into the button
            // (mode 4), other text written as is (mode 5).
            let mode = if label.is_some() { 4 } else { 5 };
            for q in quads {
                // Colour k ≠ 0: the PL2 text-colour map k; 0 draws the
                // glyph's own indices.
                let map = match (q.color, pl2) {
                    (1..=12, Some(t)) => t.text_color_shifts.get(q.color as usize).copied(),
                    _ => None,
                };
                let key = cel_key("font", "", u32::from(q.font), u32::from(q.frame));
                let Some(info) = font_info(q.font) else {
                    continue;
                };
                if let Some((table, dc6)) = a.font(q.font) {
                    if let Some(f) = dc6.frames.get(usize::from(q.frame)) {
                        // The glyph's sheet rides along for HD Text
                        // (`super::front_ui`); the CPU picture ignores it.
                        let mut cel = dc6_cel(f, key, q.at, true, mode, map);
                        cel.glyph = Some(CelGlyph {
                            sheet: info.dc6_path,
                            table,
                            dc6,
                            frame: q.frame,
                        });
                        emit(cel);
                    }
                }
            }
        }
    }
}

/// No row shift: a doll layer's cel covers rows `anchor.y + y_min …
/// anchor.y + y_max` of its frame box.
const DOLL_DY: i32 = 0;

/// The cels of one paper doll: the COF's layer order for direction 0 at
/// the animation frame, each component's DCC cel at the anchor, through
/// the colour map of its stored colour.
fn doll_cels(
    a: &mut FrontArt,
    (class, mode, comp, col): (u8, u8, &[u8; 16], &[u8; 16]),
    (at, draw_mode): (Point, u8),
    (key, epoch, ticks): (u32, u64, u32),
    emit: &mut dyn FnMut(Cel<'_>),
) {
    use crate::ui::front_end::doll::{colour_map, Doll};
    let Some(tables) = a.dolls.as_ref() else {
        return;
    };
    // A doll that can't be built falls back to class 7, mode 5, `hth`;
    // without that either, nothing is drawn.
    let built = tables.build(class, mode, comp, col);
    let mut doll = built.unwrap_or_else(Doll::fallback);
    if a.cof(&doll.cof_path()).is_none() {
        doll = Doll::fallback();
    }
    let Some(cof) = a.cof(&doll.cof_path()).cloned() else {
        return;
    };
    if a.doll_epoch != epoch {
        a.doll_epoch = epoch;
        a.doll_phase.clear();
    }
    let (frames, step) = (u32::from(cof.frames), cof.animation_rate);
    let st = a.doll_phase.entry(key).or_insert((0, 0));
    if ticks < st.0 {
        *st = (0, 0);
    }
    while st.0 < ticks {
        st.1 = crate::ui::front_end::doll::advance(st.1, frames, step);
        st.0 += 1;
    }
    let f = (st.1 >> 8) as usize;
    let Ok(dir) = crate::rules::unit_composite::file_direction(cof.directions, 0) else {
        return;
    };
    for slot in 0..usize::from(cof.layers_count) {
        let Some(c) = cof.component_at(0, f, slot) else {
            continue;
        };
        let c = usize::from(c);
        let wc = cof
            .layers
            .iter()
            .find(|l| usize::from(l.component) == c)
            .map(|l| {
                String::from_utf8_lossy(&l.weapon_class)
                    .trim_end_matches(['\0', ' '])
                    .to_owned()
            })
            .unwrap_or_else(|| doll.weapon_class_name().to_owned());
        let path = doll.dcc_path(c, &wc);
        let map = colour_map(doll.colours[c]).and_then(|(n, i)| a.colour_map(n, i));
        let Some(dcc) = a.dcc(&path) else {
            continue;
        };
        let Some(fr) = dcc
            .directions
            .get(usize::from(dir))
            .and_then(|d| d.frames.get(f))
        else {
            continue;
        };
        let w = fr.width;
        if w == 0 {
            continue;
        }
        emit(Cel {
            key: cel_key("dcc", &path, u32::from(dir), f as u32),
            pixels: CelPixels::Image {
                px: &fr.pixels,
                w,
                h: (fr.pixels.len() as u32).div_ceil(w),
            },
            x: at.x + fr.x_min,
            y: at.y + fr.y_min + DOLL_DY,
            mode: draw_mode,
            tables: true,
            map,
            glyph: None,
        });
    }
}

/// The front end in the app (a non-send resource: screens are not `Send`).
pub struct FrontHost {
    pub front: FrontEnd,
    pub art: Option<FrontArt>,
    acc_ms: u64,
    /// The last frame's draw items (tests read them).
    pub drawn: Vec<DrawItem>,
    /// Set when the flow ended.
    pub outcome: Option<Outcome>,
    pub frames: u32,
    /// Where the create screen leaves its choice, and the Save folder the
    /// stub is written to ([`FrontHost::with_stub_writer`]).
    stub: Option<(NewCharacterSink, PathBuf)>,
    /// The stub `.d2s` written for the last created character, or why not.
    pub created: Option<Result<PathBuf, String>>,
    /// A copy of [`Self::outcome`] the caller keeps: the app's world (and
    /// this resource with it) may be gone once the window has closed.
    outcome_out: std::sync::Arc<std::sync::Mutex<Option<Outcome>>>,
    /// The front-end music: serviced every
    /// tick, left when the flow ends.
    music: Option<std::sync::Arc<std::sync::Mutex<FrontMusic>>>,
    /// D2RUST's own, tooling only: the pinned front-end clock ([`FePin`]).
    pin: Option<FePin>,
    /// D2RUST's own: the window hand-off to the game process ([`Handoff`]).
    handoff: Option<Handoff>,
    /// The native UI pass's atlas of this front end's cels
    /// (`super::front_ui`).
    native: FrontUiCels,
    /// The pass's job number.
    native_seq: u64,
}

/// The pinned front-end clock of the side-by-side harness (D2RUST's own; it
/// pins `GetTickCount` for the front end).
/// `D2RUST_FE_PIN=<dir>` makes the front end
/// tick only when `<dir>/target` (decimal) is above the ticks run so far, and
/// publish `<dir>/ack` (the tick number) once that tick has been drawn and
/// presented. Wall-clock time then plays no part in what the screen shows.
struct FePin {
    dir: PathBuf,
    /// The tick drawn by the latest frame and the frame it must be published at.
    drawn: u64,
    ack_at: u32,
    acked: Option<u64>,
}

impl FePin {
    fn from_env() -> Option<Self> {
        let dir = PathBuf::from(crate::launch::env_os("D2RUST_FE_PIN").filter(|v| !v.is_empty())?);
        Some(Self {
            dir,
            drawn: 0,
            ack_at: 0,
            acked: None,
        })
    }

    fn target(&self) -> u64 {
        std::fs::read_to_string(self.dir.join("target"))
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0)
    }
}

/// D2RUST's own: the front
/// end and the game run in two processes, so the window would be empty while
/// the game process starts and builds. On a game load the front-end window
/// stays up showing loading frame 0 (the init draw) until the game process
/// has its own window (`ready` exists).
pub struct Handoff {
    /// The 800 x 600 RGBA picture of loading frame 0 on black.
    pub picture: Vec<u8>,
    /// Called once with the outcome when the flow ended in a game load.
    pub on_load: Box<dyn Fn(Outcome)>,
    /// The file the game process creates once its window presents.
    pub ready: PathBuf,
    shown: bool,
}

impl Handoff {
    pub fn new(picture: Vec<u8>, on_load: Box<dyn Fn(Outcome)>, ready: PathBuf) -> Self {
        Self {
            picture,
            on_load,
            ready,
            shown: false,
        }
    }
}

/// The 800 x 600 RGBA picture of frame 0 of the loading art centred on
/// black (L4: Loading palette, rows y-255..y at `placement`); `None` when
/// the art or the palette is not in the archives.
pub fn loading_picture(source: &dyn crate::assets::path::FileSource) -> Option<Vec<u8>> {
    use crate::ui::front_end::screens::loading::{art_path, placement};
    let dc6 = crate::assets::path::read_dc6(source, &format!("{}.dc6", art_path(0)))?.ok()?;
    let pal =
        crate::assets::path::read_palette(source, r"data\global\palette\Loading\pal.dat")?.ok()?;
    let f = dc6.frames.first()?;
    let mut px = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    px.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255);
    let (x0, y0) = placement(WIDTH as i32, HEIGHT as i32);
    let top = y0 - 255;
    for row in 0..f.height as i32 {
        for col in 0..f.width as i32 {
            let i = f.pixels[(row as u32 * f.width + col as u32) as usize];
            let (x, y) = (x0 + col, top + row);
            if i == 0 || !(0..WIDTH as i32).contains(&x) || !(0..HEIGHT as i32).contains(&y) {
                continue;
            }
            let c = pal.colors[usize::from(i)];
            let o = ((y as u32 * WIDTH + x as u32) * 4) as usize;
            px[o..o + 3].copy_from_slice(&[c.r, c.g, c.b]);
        }
    }
    Some(px)
}

/// Registers the credits screen with the text read from the archives when no
/// loose file is under `D2_GAME_DIR`.
pub fn register_credits(reg: &mut Registry, art: &FrontArt) {
    let src = art.source.clone();
    credits::register_with(
        reg,
        Box::new(move |expansion| {
            let name = if expansion {
                "ExpansionCredits.txt"
            } else {
                "Credits.txt"
            };
            // D2RUST's own, unverified: loose file first, then the archives.
            credits::loose_file(expansion)
                .or_else(|| src.read_file(&format!(r"data\local\ui\eng\{name}"))?.ok())
        }),
    );
}

/// The 335-byte stub `.d2s` of a new character,
/// written to `<dir>/<name>.d2s`.
pub fn write_stub(dir: &Path, c: &NewCharacter, time: u32) -> Result<PathBuf, String> {
    use d2_formats::d2s::{self, D2s, SaveTables, StatSave};
    // The stub has no body: no table is consulted.
    struct NoTables;
    impl SaveTables for NoTables {
        fn stat_save(&self, _: u16) -> Option<StatSave> {
            None
        }
        fn item_entry_len(&self, _: &[u8]) -> Result<usize, String> {
            Err("stub has no items".into())
        }
    }
    let mut flags = 0u16;
    if c.hardcore {
        flags |= d2s::status::HARDCORE;
    }
    if c.expansion {
        flags |= d2s::status::EXPANSION;
    }
    let stub = D2s::new_stub(c.name.as_bytes(), c.class.id(), flags, time)
        .ok_or_else(|| format!("name {:?} does not fit", c.name))?;
    let bytes = d2s::write(&stub, &NoTables).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.d2s", c.name));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

impl FrontHost {
    /// `first_entry`: the start-up chain and the trademark (C1); later: the
    /// main menu.
    pub fn new(
        expansion: bool,
        saves: Box<dyn SaveFolder>,
        art: Option<FrontArt>,
        first_entry: bool,
    ) -> Self {
        Self::with_front(FrontEnd::with_screens(expansion, saves), art, first_entry)
    }

    /// [`FrontHost::new`] over a front end the caller built (a registry
    /// with the select / create screens wired, `app::front_start`).
    pub fn with_front(mut front: FrontEnd, art: Option<FrontArt>, first_entry: bool) -> Self {
        front.start(first_entry);
        Self {
            stub: None,
            created: None,
            front,
            art,
            acc_ms: 0,
            drawn: Vec::new(),
            outcome: None,
            frames: 0,
            outcome_out: Default::default(),
            music: None,
            pin: FePin::from_env(),
            handoff: None,
            native: FrontUiCels::new(),
            native_seq: 0,
        }
    }

    /// The flow's outcome once it ended (shared: the session's game start
    /// reads it).
    pub fn outcome_handle(&self) -> std::sync::Arc<std::sync::Mutex<Option<Outcome>>> {
        self.outcome_out.clone()
    }

    /// Plays the front-end jukebox (`Options Music` on) through the app's audio device.
    pub fn with_music(mut self, music: FrontMusic) -> Self {
        self.music = Some(std::sync::Arc::new(std::sync::Mutex::new(music)));
        self
    }

    /// Applies the player's Sound and Music Volume to the menu music at
    /// once ([`FrontMusic::set_levels`]); no music, nothing to do.
    pub fn set_sound_levels(&mut self, s: &super::config::Settings) {
        if let Some(m) = &self.music {
            if let Ok(mut m) = m.lock() {
                m.set_levels(i32::from(s.master_volume), i32::from(s.music_volume));
            }
        }
    }

    /// The menu music's extra gain from the player's levels (hundredths of
    /// a dB); `None` without music.
    pub fn music_level_db(&self) -> Option<i32> {
        let m = self.music.as_ref()?;
        m.lock().ok().map(|m| m.level_db())
    }

    /// The last drawn frame for the native UI pass at `place`
    /// (`super::front_ui`): every cel of [`Self::drawn`], the same list the
    /// CPU picture is drawn from, its text enhanced from loaded glyphs while HD
    /// Text draws. `None` while the hand-off picture is shown, without art,
    /// or when a cel doesn't fit the pass's atlas.
    pub fn native_frame(&mut self, place: UiPlacement) -> Option<FrontUiFrame> {
        if self.handoff.as_ref().is_some_and(|h| h.shown) {
            return None;
        }
        let art = self.art.as_mut()?;
        let shown = art
            .pl2
            .as_ref()
            .map(|t| t.base_palette.clone())
            .or_else(|| art.palette.clone())?;
        let pl2 = art.pl2.take();
        let items = &self.drawn;
        let hd = Some(crate::world_view::ui_art::hd_text_art());
        let frame = self
            .native
            .pack(pl2.as_ref(), &shown, place, hd, &mut |emit| {
                for it in items {
                    item_cels(art, pl2.as_ref(), it, (place.frame_w, place.frame_h), emit);
                }
            });
        art.pl2 = pl2;
        frame
    }

    /// The last drawn frame as the native UI pass shows it at `place`,
    /// run on the CPU (`super::front_ui::emulate_rgba`): RGBA rows of
    /// `place.out_w × place.out_h`. For captures without a window.
    pub fn native_rgba(&mut self, place: UiPlacement) -> Option<Vec<u8>> {
        let frame = self.native_frame(place)?;
        Some(crate::app::front_ui::emulate_rgba(&mut self.native, &frame))
    }

    /// Keeps the window up on a game load ([`Handoff`]).
    pub fn with_handoff(mut self, h: Handoff) -> Self {
        self.handoff = Some(h);
        self
    }

    /// Writes the stub `.d2s` of a character the create screen finishes
    /// into `save_dir`. The choice stays in the sink for the game start.
    pub fn with_stub_writer(mut self, sink: NewCharacterSink, save_dir: PathBuf) -> Self {
        self.stub = Some((sink, save_dir));
        self
    }

    /// The flow ended in a game load from character create: write the stub.
    fn write_created(&mut self) {
        let Some((sink, dir)) = &self.stub else {
            return;
        };
        let Some(c) = sink.borrow().clone() else {
            return;
        };
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as u32);
        let r = write_stub(dir, &c, time);
        match &r {
            Ok(p) => bevy::log::info!("new character stub: {}", p.display()),
            Err(e) => bevy::log::warn!("new character stub not written: {e}"),
        }
        self.created = Some(r);
    }
}

#[derive(Resource)]
struct FrontImage(Handle<Image>);

/// The side fill's grid image ([`super::front_fill`]) and whether the
/// window shows it (set by [`front_present`], read by [`draw`], which
/// only blurs the picture while it's shown).
#[derive(Resource)]
struct FrontFill {
    image: Handle<Image>,
    shown: bool,
}

pub fn add_front_end(app: &mut App, host: FrontHost) {
    use bevy::audio::AddAudioSource;
    crate::world_view::sharp::add_sharp(app);
    crate::app::front_ui::add_front_ui(app);
    if host.music.is_some() && !crate::mute::muted() {
        app.add_audio_source::<MusicStream>();
    }
    app.insert_non_send(host)
        .add_systems(Startup, (setup, start_music))
        .add_systems(
            Update,
            (
                resize_front_canvas,
                feed_input,
                drive,
                crate::app::config::front_settings,
                draw,
                front_sharp
                    .run_if(resource_exists::<Assets<crate::world_view::sharp::SharpMaterial>>),
                front_present,
                front_ui_pack,
                front_ui_present,
                finish,
            )
                .chain(),
        );
}

/// The jukebox voice as a Bevy audio asset: pulls frames from [`FrontMusic`] at the
/// game's 22,050 Hz; the device volume (hundredths of a dB) is applied as
/// an amplitude. Outside the exactness boundary, like the in-game output edge.
#[derive(Asset, TypePath, Clone)]
struct MusicStream(std::sync::Arc<std::sync::Mutex<FrontMusic>>);

struct MusicDecoder {
    music: std::sync::Arc<std::sync::Mutex<FrontMusic>>,
    right: Option<f32>,
}

impl Iterator for MusicDecoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if let Some(r) = self.right.take() {
            return Some(r);
        }
        let mut m = self.music.lock().ok()?;
        let a = m.amplitude() / 32768.0;
        let (l, r) = m.next_frame();
        self.right = Some(f32::from(r) * a);
        Some(f32::from(l) * a)
    }
}

impl bevy::audio::Source for MusicDecoder {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> bevy::audio::ChannelCount {
        const { bevy::audio::ChannelCount::new(2).unwrap() }
    }
    fn sample_rate(&self) -> bevy::audio::SampleRate {
        const { bevy::audio::SampleRate::new(22_050).unwrap() }
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl bevy::audio::Decodable for MusicStream {
    type Decoder = MusicDecoder;
    fn decoder(&self) -> MusicDecoder {
        MusicDecoder {
            music: self.0.clone(),
            right: None,
        }
    }
}

fn start_music(
    host: NonSend<FrontHost>,
    mut commands: Commands,
    mut assets: Option<ResMut<Assets<MusicStream>>>,
) {
    if let (Some(m), Some(assets)) = (&host.music, assets.as_mut()) {
        commands.spawn((
            AudioPlayer(assets.add(MusicStream(m.clone()))),
            DespawnOnExit(Screen::Front),
        ));
    }
}

/// Builds the front end each time the session opens it: `true` after a
/// game (the main menu), `false` the first time (the start-up chain).
pub struct FrontStarter(pub Box<dyn FnMut(bool) -> FrontHost>);

/// How many times the session opened the front end.
#[derive(Resource, Default)]
struct FrontVisits(u32);

/// The front end as the session's [`Screen::Front`] (`app::session`): built
/// by `starter` on each entry, its picture and music gone on the exit.
pub fn add_front_session(app: &mut App, starter: FrontStarter) {
    use bevy::audio::AddAudioSource;
    crate::world_view::sharp::add_sharp(app);
    crate::app::front_ui::add_front_ui(app);
    // A muted process has no audio plugin: no music source, so
    // `start_music` plays nothing.
    if !crate::mute::muted() {
        app.add_audio_source::<MusicStream>();
    }
    app.insert_non_send(starter)
        .init_resource::<FrontVisits>()
        .add_systems(
            OnEnter(Screen::Front),
            (enter_front, setup_front, start_music).chain(),
        )
        .add_systems(OnExit(Screen::Front), exit_front)
        .add_systems(
            Update,
            (
                resize_front_canvas,
                feed_input,
                drive,
                crate::app::config::front_settings,
                draw,
                front_sharp
                    .run_if(resource_exists::<Assets<crate::world_view::sharp::SharpMaterial>>),
                front_present,
                front_ui_pack,
                front_ui_present,
                finish,
            )
                .chain()
                .run_if(in_state(Screen::Front)),
        );
}

fn enter_front(world: &mut World) {
    let after_game = {
        let mut v = world.resource_mut::<FrontVisits>();
        v.0 += 1;
        v.0 > 1
    };
    // The window keeps the size the player chose; the 800 x 600 picture is
    // presented in it ([`front_present`]).
    let Some(mut starter) = world.remove_non_send::<FrontStarter>() else {
        return;
    };
    let host = (starter.0)(after_game);
    world.insert_non_send(starter);
    world.insert_non_send(host);
}

fn exit_front(world: &mut World) {
    world.remove_non_send::<FrontHost>();
    world.remove_resource::<FrontImage>();
    world.remove_resource::<FrontFill>();
    world.remove_resource::<FrontUiJob>();
    world.remove_resource::<FrontUiImages>();
}

fn setup_front(commands: Commands, images: ResMut<Assets<Image>>, on: Option<Res<FrontUiOn>>) {
    spawn_picture(commands, images, false, on.is_some());
}

fn setup(commands: Commands, images: ResMut<Assets<Image>>, on: Option<Res<FrontUiOn>>) {
    spawn_picture(commands, images, true, on.is_some());
}

fn spawn_picture(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    camera: bool,
    native: bool,
) {
    if native {
        commands.insert_resource(FrontUiImages {
            targets: None,
            background: images.add(crate::app::front_ui::background_image([0; 3])),
            colour: [0; 3],
        });
    }
    let mut image = Image::new(
        Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; (WIDTH * HEIGHT * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = bevy::image::ImageSampler::nearest();
    let handle = images.add(image);
    if camera {
        // No MSAA, like every camera on the window (see `session::spawn_camera`).
        commands.spawn((Camera2d, bevy::render::view::Msaa::Off));
    }
    commands.spawn((
        Sprite::from_image(handle.clone()),
        FrontPicture,
        DespawnOnExit(Screen::Front),
    ));
    commands.insert_resource(FrontImage(handle));
    // The side fill: a small grid stretched over the window behind the
    // picture, smoothed by linear filtering; hidden until a wide window
    // asks for it.
    let mut fill = Image::new(
        Extent3d {
            width: front_fill::FILL_W,
            height: front_fill::FILL_H,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; (front_fill::FILL_W * front_fill::FILL_H * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    fill.sampler = bevy::image::ImageSampler::linear();
    let fill = images.add(fill);
    commands.spawn((
        Sprite::from_image(fill.clone()),
        Transform::from_xyz(0.0, 0.0, -1.0),
        Visibility::Hidden,
        FrontFillSprite,
        DespawnOnExit(Screen::Front),
    ));
    commands.insert_resource(FrontFill {
        image: fill,
        shown: false,
    });
}

/// Windows virtual-key codes of the keys the front end reads.
pub fn vk(key: KeyCode) -> Option<u16> {
    Some(match key {
        KeyCode::Escape => 27,
        KeyCode::Enter | KeyCode::NumpadEnter => 13,
        KeyCode::Space => 32,
        KeyCode::Backspace => 8,
        KeyCode::Tab => 9,
        KeyCode::End => 35,
        KeyCode::Home => 36,
        KeyCode::ArrowLeft => 37,
        KeyCode::ArrowUp => 38,
        KeyCode::ArrowRight => 39,
        KeyCode::ArrowDown => 40,
        KeyCode::Delete => 46,
        k => {
            let s = format!("{k:?}");
            let b = s.strip_prefix("Key").or_else(|| s.strip_prefix("Digit"))?;
            let c = b.bytes().next().filter(|_| b.len() == 1)?;
            u16::from(c)
        }
    })
}

/// Window cursor (logical px) to 800×600 frame coordinates, the whole
/// window stretched over the picture: the mapping when the window is too
/// small to present the picture in.
pub fn frame_point(cursor: Vec2, window: Vec2) -> Point {
    Point::new(
        (cursor.x * WIDTH as f32 / window.x.max(1.0)) as i32,
        (cursor.y * HEIGHT as f32 / window.y.max(1.0)) as i32,
    )
}

/// The entity showing the front end's picture.
#[derive(Component)]
struct FrontPicture;

/// How the 800×600 picture sits in a `window_w × window_h` window
/// (physical pixels) in the scale mode in force: the same placement as the
/// game frame.
pub fn front_presentation(
    window_w: u32,
    window_h: u32,
) -> Result<crate::ui::Presentation, crate::ui::FrameError> {
    let selected = front_selection(window_w, window_h);
    crate::ui::edge::present_in(
        window_w,
        window_h,
        selected.frame.width as u32,
        selected.frame.height as u32,
        selected.mode,
    )
}

fn front_selection(w: u32, h: u32) -> crate::rules::adaptive_integer::FrameSelection {
    crate::rules::adaptive_integer::FrameSelection::shipping(
        w,
        h,
        crate::rules::camera::FrameSize::D2RUST,
        crate::ui::scale_mode(),
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn resize_front_canvas(
    mut commands: Commands,
    mut host: NonSendMut<FrontHost>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    img: Option<Res<FrontImage>>,
    mut images: ResMut<Assets<Image>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    quads: Query<&Mesh2d, Or<(With<FrontPicture>, With<FrontUiQuad>)>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let selected = front_selection(window.physical_width(), window.physical_height());
    let old = host.front.canvas();
    if (old.width, old.height) == (selected.frame.width, selected.frame.height) {
        return;
    }
    let size = UVec2::new(selected.frame.width as u32, selected.frame.height as u32);
    if let Some(mut image) = img.and_then(|i| images.get_mut(&i.0)) {
        image.resize(bevy::render::render_resource::Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        });
        if let Some(px) = image.data.as_mut() {
            px.fill(0);
        }
    }
    if let Some(mut meshes) = meshes {
        for mesh in &quads {
            if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
                *mesh = Mesh::from(Rectangle::new(size.x as f32, size.y as f32));
            }
        }
    }
    commands.remove_resource::<FrontUiJob>();
    host.front.set_canvas(selected.frame);
    host.drawn.clear();
}

/// Keep the loading image's art native-sized within the current canvas.
fn loading_canvas(picture: &[u8], size: (u32, u32)) -> Vec<u8> {
    if size == (WIDTH, HEIGHT) {
        return picture.to_vec();
    }
    let mut out = vec![0; (size.0 * size.1 * 4) as usize];
    for px in out.as_chunks_mut::<4>().0 {
        px[3] = 255;
    }
    let left = (size.0.saturating_sub(WIDTH) / 2) as usize;
    for y in 0..HEIGHT.min(size.1) as usize {
        let n = WIDTH.min(size.0) as usize * 4;
        let src = y * WIDTH as usize * 4;
        let dst = (y * size.0 as usize + left) * 4;
        if let Some(row) = picture.get(src..src + n) {
            out[dst..dst + n].copy_from_slice(row);
        }
    }
    out
}

/// The picture pixel under a physical cursor position. A cursor on the
/// bars beside the picture counts as the nearest edge pixel, so a press
/// that slides off a button still gets its release.
pub fn front_point(p: &crate::ui::Presentation, physical: Vec2) -> Point {
    let (x, y) = crate::ui::edge::window_pixel(physical);
    let (left, top) = (i64::from(p.left), i64::from(p.top));
    let (iw, ih) = p.image_size();
    let x = x.clamp(left, left + i64::from(iw) - 1);
    let y = y.clamp(top, top + i64::from(ih) - 1);
    match p.to_frame(x, y) {
        crate::ui::FramePos::Inside(at) => at,
        crate::ui::FramePos::Outside => Point::new(0, 0),
    }
}

/// The cursor as the front end sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrontPointer {
    /// The picture pixel under it ([`front_point`]: the nearest edge pixel
    /// when it's beside the picture).
    pub at: Point,
    /// It is on the picture itself, not on the bars or the side fill. Only
    /// then does a press count: a click beside the picture does nothing.
    pub on_picture: bool,
}

/// The [`FrontPointer`] of a physical cursor position.
pub fn front_pointer(p: &crate::ui::Presentation, physical: Vec2) -> FrontPointer {
    let (x, y) = crate::ui::edge::window_pixel(physical);
    FrontPointer {
        at: front_point(p, physical),
        on_picture: matches!(p.to_frame(x, y), crate::ui::FramePos::Inside(_)),
    }
}

/// The window's cursor for the front end ([`front_pointer`]); a window too
/// small to present in maps the whole window over the picture
/// ([`frame_point`]), all of it counting as the picture.
fn window_point(w: &Window) -> Option<FrontPointer> {
    let physical = w.physical_cursor_position()?;
    match front_presentation(w.physical_width(), w.physical_height()) {
        Ok(p) => Some(front_pointer(&p, physical)),
        Err(_) => Some(FrontPointer {
            at: frame_point(w.cursor_position()?, Vec2::new(w.width(), w.height())),
            on_picture: true,
        }),
    }
}

/// The entity showing the side fill.
#[derive(Component)]
struct FrontFillSprite;

/// The picture's sprite becomes a sharp quad
/// ([`crate::world_view::present::sharp_quad`]).
fn front_sharp(
    mut commands: Commands,
    sprites: Query<(Entity, &Sprite), With<FrontPicture>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::world_view::sharp::SharpMaterial>>,
) {
    for (e, sprite) in &sprites {
        crate::world_view::present::sharp_quad(
            &mut commands,
            e,
            sprite.image.clone(),
            &mut images,
            &mut meshes,
            &mut materials,
        );
    }
}

/// Scales and places the picture in the window as the game frame is, in
/// physical pixels, through the sharp material when the app renders, and
/// shows the side fill behind it when the player chose one and the window
/// is wider than 4:3 ([`front_fill::shown`]), stretched to cover the
/// window. A window too small to present in leaves the picture as it is.
#[allow(clippy::type_complexity)]
fn front_present(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    choice: Option<Res<crate::app::config::WindowChoice>>,
    fill: Option<ResMut<FrontFill>>,
    mut pictures: Query<
        (
            &mut Transform,
            Option<&MeshMaterial2d<crate::world_view::sharp::SharpMaterial>>,
        ),
        (With<FrontPicture>, Without<FrontFillSprite>),
    >,
    mut fills: Query<(&mut Transform, &mut Visibility), With<FrontFillSprite>>,
    mut materials: Option<ResMut<Assets<crate::world_view::sharp::SharpMaterial>>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let (w, h) = (window.physical_width(), window.physical_height());
    let sides = choice.map_or_else(Default::default, |c| c.sides);
    let selected = front_selection(w, h);
    let adaptive =
        crate::rules::adaptive_integer::ENABLED && selected.mode == crate::ui::ScaleMode::Integer;
    let shown = !adaptive && front_fill::shown(sides, w, h);
    if let Some(mut fill) = fill {
        if fill.shown != shown {
            fill.shown = shown;
        }
    }
    let sf = window.scale_factor();
    for (mut t, mut v) in &mut fills {
        let want = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
        if shown {
            let s = front_fill::cover_scale(w, h) / sf;
            t.scale = Vec3::new(s, s, 1.0);
        }
    }
    let Ok(p) = front_presentation(w, h) else {
        return;
    };
    for (mut t, sharp) in &mut pictures {
        crate::world_view::place_frame(&p, sf, &mut t);
        crate::world_view::present::scale_sharp(&p, sharp, materials.as_deref_mut());
    }
}

/// Packs the drawn frame for the native UI pass (`super::front_ui`) when
/// the window shows the picture enlarged and the pass is on; otherwise
/// drops the job, so the picture is presented as it is.
fn front_ui_pack(
    mut commands: Commands,
    host: Option<NonSendMut<FrontHost>>,
    on: Option<Res<FrontUiOn>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    ui: Option<ResMut<FrontUiImages>>,
) {
    let (Some(mut host), Some(_), Some(mut ui)) = (host, on, ui) else {
        return;
    };
    let place = windows
        .single()
        .ok()
        .and_then(|w| front_presentation(w.physical_width(), w.physical_height()).ok())
        .and_then(|p| crate::world_view::native_ui::placement(&p));
    let frame = place.and_then(|place| host.native_frame(place).map(|f| (place, f)));
    let Some((place, frame)) = frame else {
        commands.remove_resource::<FrontUiJob>();
        return;
    };
    let size = UVec2::new(place.out_w, place.out_h);
    let targets = match ui.targets.clone().filter(|t| t.size == size) {
        Some(t) => t,
        None => {
            let t = crate::world_view::native_ui::new_targets(&mut images, size);
            ui.targets = Some(t.clone());
            t
        }
    };
    // The picture under the UI is palette index 0, as the CPU picture's
    // background.
    let colour = [frame.palette[0], frame.palette[1], frame.palette[2]];
    if ui.colour != colour {
        ui.colour = colour;
        if let Some(mut image) = images.get_mut(&ui.background) {
            image.data = Some(vec![colour[0], colour[1], colour[2], 255]);
        }
    }
    let (pages_version, pages) = host.native.pages();
    host.native_seq += 1;
    commands.insert_resource(FrontUiJob {
        seq: host.native_seq,
        packed: Arc::new(frame.packed),
        pages,
        pages_version,
        maps: Arc::new(frame.maps),
        palette: Arc::new(frame.palette),
        targets,
    });
}

/// Shows the front end through the native UI pass while it has a job: one
/// quad on the picture's place, just over it, with the pass's UI over the
/// background colour. Hidden without a job: the picture shows.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn front_ui_present(
    mut commands: Commands,
    job: Option<Res<FrontUiJob>>,
    ui: Option<Res<FrontUiImages>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    pictures: Query<&Transform, (With<FrontPicture>, Without<FrontUiQuad>)>,
    mut quads: Query<
        (
            &MeshMaterial2d<crate::world_view::native_ui::NativeUiMaterial>,
            &mut Transform,
            &mut Visibility,
        ),
        With<FrontUiQuad>,
    >,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<crate::world_view::native_ui::NativeUiMaterial>>>,
) {
    let (Some(job), Some(ui)) = (job, ui) else {
        for (_, _, mut v) in &mut quads {
            v.set_if_neq(Visibility::Hidden);
        }
        return;
    };
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(p) = front_presentation(window.physical_width(), window.physical_height()) else {
        return;
    };
    let Some(mut at) = pictures.iter().next().copied() else {
        return;
    };
    at.translation.z += 0.5;
    let scale = crate::world_view::sharp::scale(p.sx, p.sy);
    let want = crate::app::front_ui::quad_material(&ui, &job.targets, scale);
    if quads.is_empty() {
        let quad = meshes.add(Rectangle::new(p.frame_w as f32, p.frame_h as f32));
        commands.spawn((
            Mesh2d(quad),
            MeshMaterial2d(materials.add(want)),
            at,
            Visibility::Inherited,
            FrontUiQuad,
            DespawnOnExit(Screen::Front),
        ));
        return;
    }
    for (material, mut t, mut v) in &mut quads {
        v.set_if_neq(Visibility::Inherited);
        if *t != at {
            *t = at;
        }
        // Touching a material re-uploads it: only on a change.
        let same = materials.get(&material.0).is_some_and(|m| {
            m.cover == want.cover
                && m.add == want.add
                && m.frame == want.frame
                && m.scale == want.scale
        });
        if !same {
            if let Some(mut m) = materials.get_mut(&material.0) {
                *m = want.clone();
            }
        }
    }
}

fn feed_input(
    mut host: NonSendMut<FrontHost>,
    windows: Query<&Window>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    keyboard: Option<Res<ButtonInput<KeyCode>>>,
    mut keys: MessageReader<KeyboardInput>,
    mut wheel: MessageReader<MouseWheel>,
) {
    let at = windows.single().ok().and_then(window_point);
    if let Some(FrontPointer { at: p, on_picture }) = at {
        host.front.input(FrontInput::Move(p));
        if let Some(b) = buttons {
            // The left and right buttons press and release alike.
            for button in [MouseButton::Left, MouseButton::Right] {
                // A press beside the picture (the bars or the side fill)
                // does nothing; a release there still ends a press that
                // began on it.
                if b.just_pressed(button) && on_picture {
                    host.front.input(FrontInput::Down(p));
                }
                if b.just_released(button) {
                    host.front.input(FrontInput::Up(p));
                }
            }
            if b.just_pressed(MouseButton::Middle) && on_picture {
                host.front.input(FrontInput::Middle);
            }
        }
    }
    // winit on X11 (observed under Xvfb) reports each notch twice in one
    // frame; the game takes one step per notch, so two identical events of a frame are one.
    let mut last_wheel = None;
    for w in wheel.read() {
        let d = (w.y * 120.0) as i32;
        if last_wheel.replace(d) == Some(d) {
            last_wheel = None;
            continue;
        }
        host.front.input(FrontInput::Wheel(d));
    }
    // Enter with Alt held is the window's Alt+Enter (`config::alt_enter`),
    // not the menus' Enter.
    let alt = keyboard
        .as_deref()
        .is_some_and(|k| k.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]));
    for k in keys.read() {
        if alt
            && k.state == ButtonState::Pressed
            && matches!(k.key_code, KeyCode::Enter | KeyCode::NumpadEnter)
        {
            continue;
        }
        if k.state == ButtonState::Released {
            if let Some(code) = vk(k.key_code) {
                host.front.input(FrontInput::KeyUp(code));
            }
        }
        if k.state == ButtonState::Pressed {
            if let Some(code) = vk(k.key_code) {
                host.front.input(FrontInput::Key(code));
            }
            if let Some(t) = &k.text {
                for u in t.encode_utf16() {
                    host.front.input(FrontInput::Char(u));
                }
            }
        }
    }
}

/// The 40 ms driver: whole ticks out of the frame time.
fn drive(mut host: NonSendMut<FrontHost>, time: Res<Time>) {
    let target = host.pin.as_ref().map(FePin::target);
    match target {
        Some(_) => host.acc_ms = 0,
        None => host.acc_ms += time.delta().as_millis() as u64,
    }
    let mut stepped = false;
    loop {
        let run = match target {
            Some(t) => host.front.now_ms() / TICK_MS < t,
            None => host.acc_ms >= TICK_MS,
        };
        if !run || stepped || host.outcome.is_some() {
            break;
        }
        if target.is_none() {
            host.acc_ms -= TICK_MS;
        }
        host.front.tick();
        // Pinned, the clock walks to the target one tick per
        // frame, so every tick is drawn as in a pinned loop and the
        // per-draw paper-doll adds land on their ticks.
        stepped = target.is_some();
        if let Some(m) = &host.music {
            if let Ok(mut m) = m.lock() {
                m.service();
            }
        }
        host.outcome = host.front.outcome();
        if let Ok(mut out) = host.outcome_out.lock() {
            *out = host.outcome;
        }
        if matches!(host.outcome, Some(Outcome::GameLoad(g)) if g.new_character) {
            host.write_created();
        }
    }
}

fn draw(
    mut host: NonSendMut<FrontHost>,
    img: Option<Res<FrontImage>>,
    fill: Option<Res<FrontFill>>,
    mut images: ResMut<Assets<Image>>,
) {
    let host = &mut *host;
    // The side fill is only made while the window shows it.
    let fill = fill.filter(|f| f.shown).map(|f| f.image.clone());
    if let Some(h) = host.handoff.as_ref().filter(|h| h.shown) {
        if let Some(img) = img {
            if let Some(mut i) = images.get_mut(&img.0) {
                let c = host.front.canvas();
                i.data = Some(loading_canvas(
                    &h.picture,
                    (c.width as u32, c.height as u32),
                ));
            }
        }
        if let Some(mut f) = fill.and_then(|f| images.get_mut(&f)) {
            f.data = Some(front_fill::blur_rgba(&h.picture));
        }
        return;
    }
    host.frames += 1;
    host.drawn = host.front.draw();
    if let Some(art) = host.art.as_mut() {
        art.use_palette(host.front.palette());
    }
    let over = match host.art.as_mut() {
        Some(art) => {
            let art = std::cell::RefCell::new(art);
            host.front
                .overlay(&|font, text| art.borrow_mut().text_width(font, text))
        }
        None => host.front.overlay(&|_, _| 0),
    };
    // The pop-up layer's dimming covers the per-tick items (paper dolls) too.
    let at = host
        .drawn
        .iter()
        .position(|d| matches!(d, DrawItem::Dim { .. }))
        .unwrap_or(host.drawn.len());
    host.drawn.splice(at..at, over);
    let now = host.front.now_ms() / TICK_MS;
    let frames = host.frames;
    if let Some(pin) = host.pin.as_mut() {
        pin_publish(pin, now, frames);
    }
    // The pointer cel is the last thing drawn, over everything else.
    let under_pointer = host.drawn.len();
    if let Some(item) = host.front.cursor_item() {
        host.drawn.push(item);
    }
    let Some(img) = img else {
        return;
    };
    let mut blurred = None;
    let px = match &fill {
        // The fill is the picture without the pointer, so it leaves no
        // smudge beside the picture.
        Some(_) => compose_split(
            &host.drawn,
            under_pointer,
            host.art.as_mut(),
            |index, pal| {
                blurred = Some(front_fill::blur_index(index, pal));
            },
        ),
        None => {
            let c = host.front.canvas();
            compose_canvas_split(
                &host.drawn,
                host.drawn.len(),
                host.art.as_mut(),
                (c.width as u32, c.height as u32),
                |_, _| {},
            )
        }
    };
    if let Some(mut i) = images.get_mut(&img.0) {
        i.data = Some(px);
    }
    if let (Some(f), Some(b)) = (fill, blurred) {
        if let Some(mut f) = images.get_mut(&f) {
            f.data = Some(b);
        }
    }
}

/// Publishes the drawn tick two frames after it was drawn: the window has
/// presented the image by then, so a screenshot taken after the ack shows it.
fn pin_publish(pin: &mut FePin, now: u64, frames: u32) {
    if pin.drawn != now || pin.acked.is_none() && pin.ack_at == 0 {
        pin.drawn = now;
        pin.ack_at = frames + 2;
    }
    if frames >= pin.ack_at
        && pin.acked != Some(pin.drawn)
        && std::fs::write(pin.dir.join("ack"), pin.drawn.to_string()).is_ok()
    {
        pin.acked = Some(pin.drawn);
    }
}

fn finish(
    mut host: NonSendMut<FrontHost>,
    mut exit: MessageWriter<AppExit>,
    session: Option<ResMut<NextState<Screen>>>,
) {
    let Some(outcome) = host.outcome else {
        return;
    };
    let host = &mut *host;
    let leave = |music: &Option<std::sync::Arc<std::sync::Mutex<FrontMusic>>>| {
        if let Some(m) = music {
            if let Ok(mut m) = m.lock() {
                m.leave();
            }
        }
    };
    if let Some(mut next) = session {
        // One app for the session (`app::session`): a game load switches
        // the window to the game, the flow's exit ends the app.
        if let Ok(mut o) = host.outcome_out.lock() {
            *o = Some(outcome);
        }
        leave(&host.music);
        match outcome {
            Outcome::GameLoad(_) => next.set(Screen::Game),
            Outcome::Exit => {
                exit.write(AppExit::Success);
            }
        }
        host.outcome = None;
        return;
    }
    match (&mut host.handoff, outcome) {
        (Some(h), Outcome::GameLoad(_)) => {
            if !h.shown {
                // The picture is drawn from the next pass on.
                h.shown = true;
                leave(&host.music);
                (h.on_load)(outcome);
            } else if h.ready.exists() {
                exit.write(AppExit::Success);
            }
        }
        _ => {
            leave(&host.music);
            exit.write(AppExit::Success);
        }
    }
}

/// Opens the front-end window; returns how the flow ended.
pub fn run_front_end(host: FrontHost) -> Outcome {
    let mut app = App::new();
    app.add_plugins(
        crate::mute::plugins(DefaultPlugins)
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "D2RUST".into(),
                    resolution: (WIDTH, HEIGHT).into(),
                    ..default()
                }),
                // The front end draws the game's cursor into its picture.
                primary_cursor_options: Some(super::session::hidden_cursor()),
                ..default()
            })
            .set(crate::launch::log_plugin(false)),
    );
    let outcome = host.outcome_out.clone();
    add_front_end(&mut app, host);
    app.run();
    // A closed window ends the program. The resource may be gone after
    // the run, so the outcome is read from the caller's copy.
    let out = outcome.lock().ok().and_then(|o| *o);
    out.unwrap_or(Outcome::Exit)
}
