//! The D2RUST side of a rendering fact set: one built frame's draw list as `draws.tsv`,
//! `frame.tsv` and `sprites.tsv`.
//!
//! Each draw item of the frame becomes one row named after the draw call a classic renderer would
//! make for it (`FloorTileDraw`, `CelDraw`, `CelDrawShadow`, `DrawLine`, ...), so the rows can be
//! compared one by one with a reference set by [`super::compare`]. The module also writes the
//! `--dump-draws` request's directories and the optional `frame.png`. It reads the world view's
//! output only; nothing here feeds back into the game or the frame.

use std::collections::BTreeMap;
use std::path::Path;

use super::{
    sha256_hex, write, FactsError, Header, DRAWS_FILE, DRAW_COLUMNS, FRAME_COLUMNS, FRAME_FILE,
    FRAME_KEYS, NA, SPRITES_FILE, SPRITE_COLUMNS, UNKNOWN,
};
use crate::frames::{FrameAnchor, FramePart, FrameSetKey, FrameStore, IndexFrame};
use crate::rules::blend::MODE_HIGHLIGHT;
use crate::rules::camera::{Camera, CELL_HALF_WIDTH};
use crate::rules::draw_order::weather::SkyDraw;
use crate::rules::placement;
use crate::rules::shading::hover_light;
use crate::scene::order::pass;
use crate::scene::{BlendOp, CelArgs, CelPal, DrawItem, DrawKey, FrameCycle, ItemTag, MapId};
use crate::world_view::weather_view::is_sky_call_path;
use crate::world_view::UnitCall;

/// What the exporter needs besides the items.
pub struct ExportContext<'a> {
    /// The frame store the items' frame ids resolve in (sizes, offsets, owners).
    pub frames: &'a FrameStore,
    /// The frame's camera `view.left` (`None`: no camera, tile positions are `?`).
    pub view_left: Option<i32>,
    /// The unit type of a drawn unit's GUID; `None` when unknown.
    pub unit_type: &'a dyn Fn(u32) -> Option<u8>,
    /// Pass 9's calls (`WorldFrame::sky`), written in place of their pixel and flash items.
    pub sky: &'a [SkyDraw],
    /// Each drawn unit's cel context direction by GUID (`WorldFrame::unit_dirs`).
    pub unit_dirs: &'a BTreeMap<u64, u8>,
    /// Each listed unit's unit-draw arguments by draw slot (`WorldFrame::unit_rows`): the unit
    /// row's type, x, y and light; the light is also its body cels'.
    pub unit_rows: &'a BTreeMap<u64, crate::world_view::UnitRowArgs>,
    /// The DT1 header `orientation.main.sub.rarity` of each resident tile
    /// (`ViewAssets::tile_ids`).
    pub tile_ids: &'a BTreeMap<FrameSetKey, [u32; 4]>,
    /// Each tile's light digest by draw key (`WorldFrame::tile_lights`).
    pub tile_lights: &'a BTreeMap<DrawKey, String>,
    /// Each translucent wall's alpha byte by draw key
    /// (`WorldFrame::tile_alphas`): the `TileDrawTrans` `mode`.
    pub tile_alphas: &'a BTreeMap<DrawKey, u8>,
    /// Cel calls without pixels (`WorldFrame::unit_calls`), sorted by key; one row each, merged
    /// with the items by key.
    pub unit_calls: &'a [UnitCall],
    /// `WorldFrame::hit_flash`: the hit flash pre-pass cels of a unit draw by its draw slot,
    /// written after its unit row.
    pub hit_flash: &'a BTreeMap<u64, crate::world_view::HitFlash>,
    /// The first of the 256 colour rows of the UI rectangles (`ViewAssets::color_rows`): a
    /// rectangle's colour is its shade map minus this.
    pub color_rows: Option<MapId>,
    /// `WorldFrame::ui_calls`: the draw call (op) of each UI cel.
    pub ui_calls: &'a [crate::ui::draw::UiCelInfo],
    /// The act's text-colour maps (`ViewAssets::text_colors`): a glyph's
    /// colour index `k` is the position + 1 of its shade map.
    pub text_maps: &'a [MapId],
}

/// The frame-set path prefix of the UI rectangles
/// (`world_view::ui_bind::rect_key`).
const UI_RECT_PREFIX: &str = "d2rust/ui/rect/";
/// The frame set of a UI line's first pixel (frame = palette index): one `DrawLine` row per
/// line. The line's other pixels come from a set without rows ([`UI_LINE_REST`]).
pub const UI_LINE_START: &str = "d2rust/ui/line/start";
/// The frame set of a UI line's other pixels (no row).
pub const UI_LINE_REST: &str = "d2rust/ui/line/rest";

/// `draws.tsv` and `sprites.tsv` rows (without the header and column row).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rows {
    /// `draws.tsv` data rows.
    pub draws: Vec<Vec<String>>,
    /// `sprites.tsv` data rows.
    pub sprites: Vec<Vec<String>>,
}

/// `yoff` as the cel itself holds it: the bottom row for a bottom-up cel.
fn cel_yoff(f: &IndexFrame) -> i32 {
    match f.anchor {
        FrameAnchor::Top => f.y_off + f.height as i32 - 1,
        FrameAnchor::Bottom | FrameAnchor::TopDown => f.y_off,
    }
}

/// The draw call's (X, Y) of a cel item, checked by placing the frame there again.
fn cel_xy(f: &IndexFrame, item: &DrawItem) -> Option<(i32, i32)> {
    let x = item.x - f.x_off;
    let y = match f.anchor {
        FrameAnchor::Top | FrameAnchor::TopDown => item.y - f.y_off,
        FrameAnchor::Bottom => item.y - f.y_off + f.height as i32 - 1,
    };
    let p = placement::place(f, x, y, item.clip);
    (p.x == item.x && p.y == item.y).then_some((x, y))
}

/// The suffix of a derived unit-shadow frame set
/// (`world_view::unit_shadow::shadow_key`).
const SHADOW_SUFFIX: &str = "#shadow";

/// The archive folder of the overlay cels.
const OVERLAY_PREFIX: &str = "data/global/overlays/";

/// The draw-order pass of the weather calls.
const SKY_PASS: u32 = pass::UNIDENTIFIED_9;

/// The draw call name of an item, from its pass and blend. A cel in the shadow pass is
/// `CelDrawShadow` only when it is a derived unit shadow (`shadow`: a `#shadow` frame set); any
/// other cel there (an object with `DrawUnder`, a ground item) is the unit draw's `CelDraw`.
fn op(tile: bool, shadow: bool, item: &DrawItem) -> &'static str {
    let p = item.key.pass();
    match (tile, p) {
        (true, pass::FLOORS | pass::ROOFS) => "FloorTileDraw",
        (true, pass::SHADOWS) => "ShadowTileDraw",
        (true, _) if matches!(item.blend, BlendOp::IndexTableSrcRow(_)) => "TileDrawTrans",
        (true, _) => "TileDrawLit",
        (false, pass::SHADOWS) if shadow => "CelDrawShadow",
        (false, _) => "CelDraw",
    }
}

/// Whether `item` is a derived unit shadow: its frame set is the unit cel's `#shadow` set.
fn is_shadow_item(item: &DrawItem, cx: &ExportContext<'_>) -> bool {
    item.key.pass() == pass::SHADOWS
        && cx
            .frames
            .owner(item.frame)
            .is_some_and(|(k, _)| k.path().ends_with(SHADOW_SUFFIX))
}

/// The (X, Y) a tile draw call is handed. Floors and roofs are given relative to the camera's
/// `view.left`; other tiles use the frame's own origin.
fn tile_xy(f: &IndexFrame, item: &DrawItem, view_left: Option<i32>) -> Option<(i32, i32)> {
    let origin = (item.x - f.x_off, item.y - f.y_off);
    match item.key.pass() {
        pass::FLOORS | pass::ROOFS => view_left.map(|l| (origin.0 + CELL_HALF_WIDTH - l, origin.1)),
        _ => Some(origin),
    }
}

/// One row per pass 9 (weather) call. `DrawLine` (x0, y0, x1, y1, color, alpha) and the flash's
/// `DrawBox` (x0, y0, x1, y1, color, mode): `x`, `y` = x0, y0, `mode` = the color, `at` =
/// [`PASS9_TAG`](super::compare::PASS9_TAG), which lets `--skip-weather` drop them.
fn sky_rows(sky: &[SkyDraw]) -> Vec<Vec<String>> {
    sky.iter()
        .map(|d| {
            let (op, x, y, color) = match *d {
                SkyDraw::Line { x0, y0, color, .. } => ("DrawLine", x0, y0, color),
                SkyDraw::Flash { x0, y0, .. } => ("DrawBox", x0, y0, SkyDraw::FLASH_COLOR),
            };
            let mut row = vec![NA.to_owned(); DRAW_COLUMNS.len()];
            row[1] = op.into();
            row[6] = x.to_string();
            row[7] = y.to_string();
            row[12] = color.to_string();
            row[16] = super::compare::PASS9_TAG.into();
            row
        })
        .collect()
}

/// A light word as written in `draws.tsv`: lowercase hex with a `0x` prefix.
fn light_hex(word: u32) -> String {
    format!("{word:#x}")
}

fn num(v: Option<i32>) -> String {
    v.map_or_else(|| UNKNOWN.to_owned(), |v| v.to_string())
}

/// Writes the unit-draw row that starts a unit's run of cel rows outside the shadow pass. The
/// shadow pass has `CelDrawShadow` rows with no unit-draw row before them. A run after the
/// unit's own shadow still starts with it. One unit draw is one draw-order slot: a new slot
/// starts a run even under the same tag (a GUID two unit types share, such as a monster 1:3
/// followed by an object 2:3). `last_run` is the previous entry's (tag, shadow, slot) of any
/// kind.
fn unit_row(
    tag: ItemTag,
    shadow: bool,
    slot: u64,
    cx: &ExportContext<'_>,
    xy: &BTreeMap<(u64, bool), (i32, i32)>,
    last_run: &mut Option<(ItemTag, bool, u64)>,
    draws: &mut Vec<Vec<String>>,
) {
    if let (ItemTag::Unit(guid), false) = (tag, shadow) {
        if *last_run != Some((tag, false, slot)) {
            let args = cx.unit_rows.get(&slot);
            // A client-made unit's view key carries `VIEW_CLIENT_BIT`; the
            // unit draw sees its own GUID.
            let guid = guid & !crate::bridge::world::VIEW_CLIENT_BIT;
            let unit = match args.map(|a| a.unit_type).or_else(|| (cx.unit_type)(guid)) {
                Some(t) => format!("{t}:{guid}"),
                None => UNKNOWN.to_owned(),
            };
            let mut row = vec![NA.to_owned(); DRAW_COLUMNS.len()];
            row[1] = "unit".into();
            let point = args.and_then(|a| a.point);
            row[6] = num(point.map(|p| p.0));
            row[7] = num(point.map(|p| p.1));
            row[13] = args
                .and_then(|a| a.light)
                .map_or_else(|| UNKNOWN.to_owned(), light_hex);
            row[15] = unit;
            draws.push(row);
            hit_flash_rows(slot, cx, xy, draws);
        }
    }
    *last_run = Some((tag, shadow, slot));
}

/// The hit flash pre-pass of a unit draw: one `CelDrawHilight` row per body cel in slot order,
/// right after the unit row. The row has the cel's file, the context's `dir64` and frame, the
/// unit draw's (X, Y), the cel's size as its `CelDraw` row has it (`-` for a file in no
/// archive), no mode or light (`-`: the call takes only context, X, Y and a colour) and the
/// colour index as `pal`.
fn hit_flash_rows(
    slot: u64,
    cx: &ExportContext<'_>,
    xy: &BTreeMap<(u64, bool), (i32, i32)>,
    draws: &mut Vec<Vec<String>>,
) {
    use crate::world_view::HitFlashCel;
    let Some(flash) = cx.hit_flash.get(&slot) else {
        return;
    };
    let at = xy.get(&(slot, false));
    for cel in &flash.cels {
        let (path, file_dir, index, size) = match cel {
            HitFlashCel::Frame(id) => {
                let (Some((key, index)), Some(f)) = (cx.frames.owner(*id), cx.frames.frame(*id))
                else {
                    continue;
                };
                let FramePart::Dir(d) = key.part() else {
                    continue;
                };
                let size = [
                    f.width.to_string(),
                    f.height.to_string(),
                    f.x_off.to_string(),
                    cel_yoff(f).to_string(),
                ];
                (key.path(), Some(d), index, Some(size))
            }
            HitFlashCel::Call { path, frame } => (path.as_str(), None, *frame, None),
        };
        let Some(dir) = cx.unit_dirs.get(&slot).copied().or(file_dir) else {
            continue;
        };
        let mut row = vec![NA.to_owned(); DRAW_COLUMNS.len()];
        row[1] = "CelDrawHilight".into();
        row[2] = path.to_owned();
        row[3] = dir.to_string();
        row[4] = index.to_string();
        row[6] = num(at.map(|p| p.0));
        row[7] = num(at.map(|p| p.1));
        if let Some(size) = size {
            row[8..12].clone_from_slice(&size);
        }
        row[14] = flash
            .color
            .map_or_else(|| UNKNOWN.to_owned(), |c| c.to_string());
        draws.push(row);
    }
}

/// The `mode light pal` cells (12–14) of a UI cel by its draw call's arguments: `CelDraw` and
/// `CelDrawColor` (X, Y, light 0xFF, mode, palette / colour index) fill all three; `CelDrawEx`
/// (X, Y, skip, lines, mode) and `CelDrawClipped` (X, Y, clip, mode) carry no light and no
/// palette (`-`).
fn ui_cells(
    info: &crate::ui::draw::UiCelInfo,
    item: &DrawItem,
    text: &[MapId],
    row: &mut [String],
) {
    use crate::ui::draw::CelCall;
    let k = if info.text {
        // The glyph's colour: its text map's position + 1, else 0.
        Some(
            item.shade
                .maps()
                .iter()
                .find_map(|m| text.iter().position(|t| t == m))
                .map_or(0, |p| p as i32 + 1),
        )
    } else {
        info.pal
    };
    let pal = k.map_or_else(|| UNKNOWN.to_owned(), |k| k.to_string());
    match info.call {
        CelCall::Draw | CelCall::Color => {
            row[12] = info.mode.to_string();
            row[13] = "0xffffffff".into();
            row[14] = pal;
        }
        CelCall::Ex | CelCall::Clipped => {
            row[12] = info.mode.to_string();
            row[13] = NA.into();
            row[14] = NA.into();
        }
    }
}

/// The `mode light pal` cells (12–14) of a world cel: the call's mode and palette
/// (`DrawItem::call`) and the unit draw's light word, which the unit draw passes on to its cel
/// calls. For the highlight mode (`MODE_HIGHLIGHT`) the light's low byte is the hover-lit value
/// (`hover_light`).
fn cel_cells(call: Option<CelArgs>, light: Option<u32>, row: &mut [String]) {
    if let Some(c) = call {
        row[12] = c.mode.to_string();
        row[14] = match c.pal {
            CelPal::Null => "0".into(),
            CelPal::Hue(k) => format!("h{k}"),
            CelPal::Shift(s) => format!("s{s}"),
            CelPal::Other => UNKNOWN.into(),
        };
    }
    if let Some(mut w) = light {
        if call.is_some_and(|c| c.mode == MODE_HIGHLIGHT) {
            w = (w & !0xFF) | u32::from(hover_light((w & 0xFF) as u8));
        }
        row[13] = light_hex(w);
    }
}

/// A cel call without pixels (a component file in no archive): `CelDrawShadow` / `CelDraw`, the
/// file, the context's `dir64` and frame; nothing else is measured (no cel: no size, no
/// `sprites.tsv` row).
fn call_rows(
    c: &UnitCall,
    cx: &ExportContext<'_>,
    xy: &BTreeMap<(u64, bool), (i32, i32)>,
    last_run: &mut Option<(ItemTag, bool, u64)>,
    draws: &mut Vec<Vec<String>>,
) {
    unit_row(c.tag, c.shadow, c.key.slot(), cx, xy, last_run, draws);
    // No path: the unit draw alone (the body failed the pre-test).
    let Some(path) = &c.path else {
        return;
    };
    let mut row = vec![NA.to_owned(); DRAW_COLUMNS.len()];
    row[1] = if c.shadow { "CelDrawShadow" } else { "CelDraw" }.into();
    row[2] = path.as_str().to_owned();
    row[3] = c.dir64.to_string();
    row[4] = c.frame.to_string();
    for cell in &mut row[6..15] {
        *cell = UNKNOWN.into();
    }
    // No cel reaches the rasterizer, so there is no size.
    for cell in &mut row[8..12] {
        *cell = NA.into();
    }
    // The call's (X, Y): every cel call of one unit draw (or of its shadow) takes the same
    // (X, Y).
    if let Some(&(x, y)) = xy.get(&(c.key.slot(), c.shadow)) {
        row[6] = x.to_string();
        row[7] = y.to_string();
    }
    if c.shadow {
        for cell in &mut row[12..15] {
            *cell = NA.into();
        }
    } else {
        let light = cx.unit_rows.get(&c.key.slot()).and_then(|a| a.light);
        cel_cells(c.call, light, &mut row);
    }
    draws.push(row);
}

/// The cel call (X, Y) of each unit draw and unit shadow draw, by (draw slot, shadow), from its
/// first cel item whose position inverts: the calls without pixels of the same draw take it. An
/// overlay cel (`data/global/overlays/`) is drawn by its own call at its own (X, Y) and is not
/// one of the composite's calls.
fn unit_call_xy(items: &[DrawItem], cx: &ExportContext<'_>) -> BTreeMap<(u64, bool), (i32, i32)> {
    let mut out = BTreeMap::new();
    for item in items {
        let ItemTag::Unit(_) = item.tag else { continue };
        let (Some((key, _)), Some(f)) = (cx.frames.owner(item.frame), cx.frames.frame(item.frame))
        else {
            continue;
        };
        if !matches!(key.part(), FramePart::Dir(_)) || key.path().starts_with(OVERLAY_PREFIX) {
            continue;
        }
        let shadow = item.key.pass() == pass::SHADOWS;
        let xy = if key.path().ends_with(SHADOW_SUFFIX) {
            let (l, t) = placement::draw_position(f, 0, 0);
            Some((item.x - l - 2, item.y - t))
        } else if shadow {
            None
        } else {
            cel_xy(f, item)
        };
        if let Some(p) = xy {
            out.entry((item.key.slot(), shadow)).or_insert(p);
        }
    }
    out
}

/// Whether a unit cel of file direction `file_dir` drawn at call direction `call_dir` came
/// through the direction-block fallback: the file direction that the call's `dir64` maps to
/// (`file_direction`, given `df`, the number of direction sets the file has resident; a
/// component file is resident whole) is another one.
fn is_block_fallback(
    cx: &ExportContext<'_>,
    path: &str,
    file_dir: u8,
    item: &DrawItem,
    call_dir: u8,
) -> bool {
    if !matches!(item.tag, ItemTag::Unit(_)) || path.ends_with(SHADOW_SUFFIX) {
        return false;
    }
    let df = (0u8..64)
        .take_while(|&k| {
            FrameSetKey::new(path, FramePart::Dir(k))
                .ok()
                .is_some_and(|key| cx.frames.id(&key, 0).is_ok())
        })
        .count();
    let Ok(df) = u8::try_from(df) else {
        return false;
    };
    crate::rules::unit_composite::file_direction(df, call_dir).is_ok_and(|f| f != file_dir)
}

/// The rows of one frame's sorted draw list.
pub fn draw_rows(items: &[DrawItem], cx: &ExportContext<'_>) -> Result<Rows, FactsError> {
    let mut draws: Vec<Vec<String>> = Vec::new();
    let mut sprites = BTreeMap::new();
    let mut last: Option<&DrawItem> = None;
    let mut last_tile: Option<(ItemTag, DrawKey, Vec<String>)> = None;
    let mut sky = Some(sky_rows(cx.sky));
    // The last unit-pass entry (item or call): its tag, whether it was a shadow, and its slot;
    // decides where a new unit row starts.
    let mut last_run: Option<(ItemTag, bool, u64)> = None;
    let mut calls = cx.unit_calls.iter().peekable();
    let xy = unit_call_xy(items, cx);
    for item in items {
        // The calls without pixels that sort before this item's key.
        while let Some(c) = calls.next_if(|c| c.key < item.key) {
            call_rows(c, cx, &xy, &mut last_run, &mut draws);
            last_tile = None;
        }
        // The pass 9 calls stand where their first drawing is, or before the first later pass
        // when every line is off-screen.
        let call_item = cx
            .frames
            .owner(item.frame)
            .is_some_and(|(k, _)| is_sky_call_path(k.path()));
        if call_item || item.key.pass() > SKY_PASS {
            if let Some(rows) = sky.take() {
                draws.extend(rows);
            }
        }
        if call_item {
            continue;
        }
        // The per-block draws of one tile are one row; two tile records (two draw keys) are two
        // calls.
        if last.is_some_and(|l| {
            l.tag == item.tag
                && l.frame == item.frame
                && l.x == item.x
                && l.y == item.y
                && l.key == item.key
        }) {
            continue;
        }
        // A unit draw row starts a unit's run except for a derived shadow: the shadow pass has
        // `CelDrawShadow` rows with no unit draw, but an object drawn under the units there
        // (a corpse with `DrawUnder`) is a unit draw.
        let shadow = is_shadow_item(item, cx);
        unit_row(
            item.tag,
            shadow,
            item.key.slot(),
            cx,
            &xy,
            &mut last_run,
            &mut draws,
        );
        last = Some(item);
        let (key, index) = cx
            .frames
            .owner(item.frame)
            .ok_or_else(|| FactsError::Export(format!("frame {:?} has no owner", item.frame)))?;
        let f = cx
            .frames
            .frame(item.frame)
            .ok_or_else(|| FactsError::Export(format!("frame {:?} not resident", item.frame)))?;
        let mut row = vec![NA.to_owned(); DRAW_COLUMNS.len()];
        // A UI line (the automap markers): one `DrawLine` row at its first pixel (x0, y0) with
        // its colour.
        if key.path() == UI_LINE_REST {
            continue;
        }
        if key.path() == UI_LINE_START {
            row[1] = "DrawLine".into();
            row[6] = item.x.to_string();
            row[7] = item.y.to_string();
            row[12] = index.to_string();
            last_tile = None;
            draws.push(row);
            continue;
        }
        // A UI rectangle is one `DrawBox` row: its left, top and colour.
        if key.path().starts_with(UI_RECT_PREFIX) {
            row[1] = "DrawBox".into();
            row[6] = item.x.to_string();
            row[7] = item.y.to_string();
            row[12] = match (cx.color_rows, item.shade.maps().first()) {
                (Some(base), Some(m)) if m.0 >= base.0 && m.0 - base.0 < 256 => {
                    (m.0 - base.0).to_string()
                }
                _ => UNKNOWN.into(),
            };
            last_tile = None;
            draws.push(row);
            continue;
        }
        row[2] = key.path().to_owned();
        // A pass 4 pool cel is tagged so `--skip-weather` can drop it.
        if item.key.pass() == pass::UNIDENTIFIED_4 {
            row[16] = super::compare::POOLS_TAG.into();
        }
        match key.part() {
            FramePart::Tile(t) => {
                row[1] = op(true, shadow, item).into();
                row[4] = t.to_string();
                row[5] = cx.tile_ids.get(key).map_or_else(
                    || UNKNOWN.to_owned(),
                    |i| format!("{}.{}.{}.{}", i[0], i[1], i[2], i[3]),
                );
                let xy = tile_xy(f, item, cx.view_left);
                row[6] = num(xy.map(|p| p.0));
                row[7] = num(xy.map(|p| p.1));
                if row[1] == "TileDrawTrans" {
                    row[12] = cx
                        .tile_alphas
                        .get(&item.key)
                        .map_or_else(|| UNKNOWN.into(), u8::to_string);
                }
                if row[1] != "ShadowTileDraw" {
                    row[13] = cx
                        .tile_lights
                        .get(&item.key)
                        .cloned()
                        .unwrap_or_else(|| UNKNOWN.into());
                }
            }
            FramePart::Dir(d) => {
                let file_dir = d;
                row[1] = match item.tag {
                    // A UI cel takes the draw call its UI code made.
                    ItemTag::Ui(i) => cx
                        .ui_calls
                        .get(i as usize)
                        .map_or(op(false, shadow, item), |c| c.call.op()),
                    _ => op(false, shadow, item),
                }
                .into();
                // A unit cel's `dir` is the context's `dir64`, not the file direction the frame
                // set is keyed by.
                let d = match item.tag {
                    ItemTag::Unit(_) => cx.unit_dirs.get(&item.key.slot()).copied().unwrap_or(d),
                    _ => d,
                };
                row[3] = d.to_string();
                row[4] = index.to_string();
                // A unit shadow is drawn from the unit's own cel; D2RUST composes it from a
                // derived `#shadow` set of sheared frames. The row names the cel's file and
                // size, and the call's X, Y read back from the derived image's placement.
                let (f, xy) = match key.path().strip_suffix(SHADOW_SUFFIX) {
                    Some(base) => {
                        row[2] = base.to_owned();
                        let base_key = FrameSetKey::new(base, key.part())
                            .map_err(|e| FactsError::Export(e.to_string()))?;
                        let id = cx
                            .frames
                            .id(&base_key, index)
                            .map_err(|e| FactsError::Export(format!("{base}: {e}")))?;
                        let base_frame = cx.frames.frame(id).ok_or_else(|| {
                            FactsError::Export(format!("{base} frame {index} not resident"))
                        })?;
                        // The shadow call's (X, Y) is the unit's X − 2 and Y; the derived image
                        // was placed at the unit's own (X, Y) (`unit_shadow::draws`).
                        let (l, t) = placement::draw_position(f, 0, 0);
                        (base_frame, Some((item.x - l - 2, item.y - t)))
                    }
                    None => (f, cel_xy(f, item)),
                };
                row[6] = num(xy.map(|p| p.0));
                row[7] = num(xy.map(|p| p.1));
                let size = [
                    f.width.to_string(),
                    f.height.to_string(),
                    f.x_off.to_string(),
                    cel_yoff(f).to_string(),
                ];
                // A cel clipped wholly off-screen, or one the driver's pre-test rejects, has its
                // size too: the cel is resolved before its screen test.
                row[8..12].clone_from_slice(&size);
                // `CelDrawShadow` takes only context, X, Y: no mode, light or palette.
                let shadow_call = row[1] == "CelDrawShadow";
                for cell in &mut row[12..15] {
                    *cell = if shadow_call { NA } else { UNKNOWN }.into();
                }
                if row[1] == "CelDraw" {
                    let light = match item.tag {
                        ItemTag::Unit(_) => {
                            cx.unit_rows.get(&item.key.slot()).and_then(|a| a.light)
                        }
                        // A pool cel is drawn unlit, light −1.
                        _ if item.key.pass() == pass::UNIDENTIFIED_4 => Some(u32::MAX),
                        _ => None,
                    };
                    cel_cells(item.call, light, &mut row);
                }
                if let ItemTag::Ui(i) = item.tag {
                    if let Some(info) = cx.ui_calls.get(i as usize) {
                        ui_cells(info, item, cx.text_maps, &mut row);
                    }
                }
                // A cel drawn through the direction-block fallback (its file direction is not
                // the call direction's) has no `sprites.tsv` key; its `draws.tsv` row carries
                // the size.
                if !is_block_fallback(cx, key.path(), file_dir, item, d) {
                    sprites.insert((row[2].clone(), u32::from(d), index), size);
                }
            }
        }
        // A tile's block draws are separate frames (each block with its own offsets) that give
        // the same row: one row per tile.
        let tile_row = matches!(key.part(), FramePart::Tile(_));
        if tile_row
            && last_tile
                .as_ref()
                .is_some_and(|(tag, key, r)| *tag == item.tag && *key == item.key && *r == row)
        {
            continue;
        }
        last_tile = tile_row.then(|| (item.tag, item.key, row.clone()));
        draws.push(row);
    }
    for c in calls {
        call_rows(c, cx, &xy, &mut last_run, &mut draws);
    }
    if let Some(rows) = sky.take() {
        draws.extend(rows);
    }
    for (i, r) in draws.iter_mut().enumerate() {
        r[0] = i.to_string();
    }
    let sprites = sprites
        .into_iter()
        .map(|((file, dir, frame), size)| {
            let mut r = vec![file, dir.to_string(), frame.to_string()];
            r.extend(size);
            r
        })
        .collect();
    Ok(Rows { draws, sprites })
}

/// Adds the frame cycle's own draw calls around the items: first `StartDraw(bClear, 0, 0, 0)`
/// with `bClear` the BlankScreen flag of the player's level, last `ClearScreen(0)` when the
/// frame's plan clears after drawing. Renumbers `i`.
pub fn add_cycle_rows(rows: &mut Rows, blank_screen: bool, clear_after: bool) {
    let mut start = vec![NA.to_owned(); DRAW_COLUMNS.len()];
    start[1] = "StartDraw".into();
    start[6] = u8::from(blank_screen).to_string();
    start[7] = "0".into();
    rows.draws.insert(0, start);
    if clear_after {
        let mut clear = vec![NA.to_owned(); DRAW_COLUMNS.len()];
        clear[1] = "ClearScreen".into();
        clear[6] = "0".into();
        rows.draws.push(clear);
    }
    for (i, r) in rows.draws.iter_mut().enumerate() {
        r[0] = i.to_string();
    }
}

/// The `frame.tsv` input cells D2RUST has: the player's client position (the camera anchor),
/// the light quality replayed from the frame schedule (a host input), the level's `Rain` /
/// `Mud` flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameInputs {
    /// The player's client position (x, y).
    pub player: Option<(i32, i32)>,
    /// The light quality setting the frame was drawn with.
    pub light_quality: Option<u8>,
    /// The level's (rain, snow) flags.
    pub weather: Option<(bool, bool)>,
}

/// The `frame.tsv` values D2RUST knows; keys it does not measure are written as `?`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameState {
    /// Dump sequence number (informational; never compared).
    pub seq: u64,
    /// Server tick of the frame.
    pub tick: u64,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Current act, if known.
    pub act: Option<u8>,
    /// Current level id, if known.
    pub level: Option<u16>,
    /// The frame's camera, if it had one.
    pub camera: Option<Camera>,
    /// The open panel mode, if known.
    pub open_mode: Option<u8>,
    /// The inputs of the frame D2RUST runs on (`frame.tsv` `player_x`, `player_y`,
    /// `light_quality`, `rain`, `snow`).
    pub inputs: FrameInputs,
    /// Number of `draws.tsv` rows.
    pub draws: usize,
    /// SHA-256 of the composed index buffer.
    pub index_sha256: Option<String>,
    /// SHA-256 of the palette as RGB bytes.
    pub palette_sha256: String,
}

/// `frame.tsv` rows in [`FRAME_KEYS`] order.
pub fn frame_rows(s: &FrameState) -> Vec<Vec<String>> {
    let opt = |v: Option<i64>| v.map_or_else(|| UNKNOWN.to_owned(), |v| v.to_string());
    let cam = s.camera.as_ref();
    let value = |key: &str| -> String {
        match key {
            "seq" => s.seq.to_string(),
            "tick" => s.tick.to_string(),
            "w" => s.width.to_string(),
            "h" => s.height.to_string(),
            "act" => opt(s.act.map(i64::from)),
            "level" => opt(s.level.map(i64::from)),
            "tile_origin_x" => opt(cam.map(|c| i64::from(c.tile.x))),
            "tile_origin_y" => opt(cam.map(|c| i64::from(c.tile.y))),
            "unit_origin_x" => opt(cam.map(|c| i64::from(c.unit.x))),
            "unit_origin_y" => opt(cam.map(|c| i64::from(c.unit.y))),
            "open_mode" => opt(s.open_mode.map(i64::from)),
            "shift_x" => opt(cam.map(|c| i64::from(c.view.shift_x))),
            "player_x" => opt(s.inputs.player.map(|p| i64::from(p.0))),
            "player_y" => opt(s.inputs.player.map(|p| i64::from(p.1))),
            "light_quality" => opt(s.inputs.light_quality.map(i64::from)),
            "rain" => opt(s.inputs.weather.map(|w| i64::from(w.0))),
            "snow" => opt(s.inputs.weather.map(|w| i64::from(w.1))),
            "draws" => s.draws.to_string(),
            "index_sha256" => s.index_sha256.clone().unwrap_or_else(|| UNKNOWN.into()),
            "palette_sha256" => s.palette_sha256.clone(),
            // Keys D2RUST does not measure.
            _ => UNKNOWN.to_owned(),
        }
    };
    FRAME_KEYS
        .iter()
        .map(|(k, _)| vec![(*k).to_owned(), value(k)])
        .collect()
}

/// The three files' text.
pub fn files(header: &Header, rows: &Rows, frame: &FrameState) -> [(&'static str, String); 3] {
    [
        (DRAWS_FILE, write(header, &DRAW_COLUMNS, &rows.draws)),
        (
            FRAME_FILE,
            write(header, &FRAME_COLUMNS, &frame_rows(frame)),
        ),
        (SPRITES_FILE, write(header, &SPRITE_COLUMNS, &rows.sprites)),
    ]
}

/// The `--dump-draws DIR --at-tick N[,M...] [--dump-image]` request of `play`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpRequest {
    /// Output directory (one sub-folder per tick when several ticks are requested).
    pub dir: std::path::PathBuf,
    /// Per dump, the first drawn frame whose server tick is at least this;
    /// strictly increasing, at least one.
    pub at_ticks: Vec<u64>,
    /// Also write the composed frame as `frame.png`.
    pub image: bool,
    /// Window sizes to capture the frame at, without and with the native
    /// UI pass (`world_view::native_ui_shots`).
    pub native: Vec<(u32, u32)>,
    /// The header's `command` (the command line).
    pub command: String,
}

impl DumpRequest {
    /// The folder of dump `i`: `dir` itself for a single tick, else `dir/tick-<N>`.
    pub fn dir_for(&self, i: usize) -> std::path::PathBuf {
        match self.at_ticks.as_slice() {
            [_] => self.dir.clone(),
            ticks => self.dir.join(format!("tick-{}", ticks[i])),
        }
    }

    /// The header line written to every file of the dump.
    pub fn header(&self) -> Header {
        let v = env!("CARGO_PKG_VERSION");
        Header {
            tool: format!("d2-client {v} play --dump-draws"),
            command: self.command.clone(),
            game: format!("D2RUST {v}"),
        }
    }
}

/// Everything one dump reads from the play loop.
pub struct DumpFrame<'a> {
    /// The client's world mirror.
    pub world: &'a crate::bridge::world::ClientWorld,
    /// The built frame (items, calls, camera, per-draw side tables).
    pub frame: &'a crate::world_view::WorldFrame,
    /// Frame store, tile ids, palette and maps.
    pub assets: &'a crate::world_view::ViewAssets,
    /// The frame cycle that composes the framebuffer.
    pub cycle: &'a FrameCycle,
    /// The BlankScreen flag of the player's level.
    pub blank_screen: bool,
    /// The open panel mode, if known.
    pub open_mode: Option<u8>,
    /// Dump sequence number.
    pub seq: u64,
    /// The `frame.tsv` input cells.
    pub inputs: FrameInputs,
}

/// Writes the three files of `d` (and with `req.image` the frame's `frame.png`) into `dir`.
pub fn dump(req: &DumpRequest, dir: &Path, d: &DumpFrame<'_>) -> Result<(), FactsError> {
    let unit_type = |guid: u32| {
        let mut types = d.world.units.keys().filter(|k| k.guid == guid);
        match (types.next(), types.next()) {
            (Some(k), None) => Some(k.unit_type),
            _ => None,
        }
    };
    let cx = ExportContext {
        frames: &d.assets.frames,
        view_left: d.frame.camera.map(|c| c.view.left),
        unit_type: &unit_type,
        sky: &d.frame.sky,
        unit_dirs: &d.frame.unit_dirs,
        unit_rows: &d.frame.unit_rows,
        tile_ids: &d.assets.tile_ids,
        tile_lights: &d.frame.tile_lights,
        tile_alphas: &d.frame.tile_alphas,
        unit_calls: &d.frame.unit_calls,
        hit_flash: &d.frame.hit_flash,
        color_rows: d.assets.color_rows,
        ui_calls: &d.frame.ui_calls,
        text_maps: d
            .assets
            .text_colors
            .as_ref()
            .map_or(&[][..], |t| &t.maps[..]),
    };
    // The draw calls without pixels join the items by key.
    let mut all = d.frame.items.clone();
    all.extend_from_slice(&d.frame.calls);
    crate::scene::order(&mut all);
    let mut rows = draw_rows(&all, &cx)?;
    add_cycle_rows(
        &mut rows,
        d.blank_screen,
        d.cycle.plan(d.blank_screen).clear_after,
    );
    // The CPU reference composition, drawn onto a copy of the framebuffer.
    let mut cycle = d.cycle.clone();
    let pixels = cycle
        .compose(
            d.blank_screen,
            &d.frame.items,
            &d.assets.frames,
            &d.assets.maps,
        )
        .map(<[u8]>::to_vec)
        .map_err(|e| FactsError::Export(format!("compose: {e}")))?;
    let index = sha256_hex(&pixels);
    let palette: Vec<u8> = d
        .assets
        .palette
        .colors
        .iter()
        .flat_map(|c| [c.r, c.g, c.b])
        .collect();
    let view = cycle.view();
    let state = FrameState {
        seq: d.seq,
        tick: d.world.server_ticks,
        width: view.width,
        height: view.height,
        act: d.world.act.as_ref().map(|a| a.act),
        level: d.world.local_room().map(|r| r.level),
        camera: d.frame.camera,
        open_mode: d.open_mode,
        inputs: d.inputs,
        draws: rows.draws.len(),
        index_sha256: Some(index),
        palette_sha256: sha256_hex(&palette),
    };
    write_dir(dir, &files(&req.header(), &rows, &state))?;
    if req.image {
        let png = indexed_png(view.width, view.height, &pixels, &palette)?;
        let path = dir.join(IMAGE_FILE);
        std::fs::write(&path, png).map_err(|source| FactsError::Io { path, source })?;
    }
    if !req.native.is_empty() {
        crate::world_view::native_ui_shots::write_shots(
            dir,
            &d.frame.items,
            d.assets,
            d.cycle,
            d.blank_screen,
            &req.native,
            crate::world_view::ui_art::current(),
        )
        .map_err(FactsError::Export)?;
    }
    Ok(())
}

/// File name of the composed frame written by `--dump-image`.
pub const IMAGE_FILE: &str = "frame.png";

/// An 8-bit palettized PNG of `pixels` (the index bytes as they are, the palette as `PLTE`).
pub fn indexed_png(
    width: u32,
    height: u32,
    pixels: &[u8],
    palette_rgb: &[u8],
) -> Result<Vec<u8>, FactsError> {
    let err = |e: png::EncodingError| FactsError::Export(format!("png: {e}"));
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(png::ColorType::Indexed);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_palette(palette_rgb.to_vec());
    let mut w = enc.write_header().map_err(err)?;
    w.write_image_data(pixels).map_err(err)?;
    w.finish().map_err(err)?;
    Ok(out)
}

fn write_dir(dir: &Path, files: &[(&'static str, String)]) -> Result<(), FactsError> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| FactsError::Io { path, source }
    };
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    for (name, text) in files {
        let path = dir.join(name);
        std::fs::write(&path, text).map_err(io(&path))?;
    }
    Ok(())
}
