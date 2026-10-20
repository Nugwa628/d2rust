//! Blend modes: how a draw's pixels meet the pixels already on screen.
//!
//! Every cel draw carries a *draw mode* (0–7). The mode picks one of the six blend tables of
//! the act's palette-table block (or none: opaque), and together with the cel's light map and
//! colormap it becomes the shade chain and blend op of a `scene::DrawItem`
//! (through `scene::PixelTables`; blend tables are stored destination-major, row =
//! destination). This module decides that mode for every kind of draw:
//!
//! - composite components (unit fade, ethereal items, item transparency, the hover
//!   highlight, COF layer overrides), missiles, ground items and overlays;
//! - unit shadows and shadow tiles: the pixel ops, the sheared shadow shape, its position on
//!   screen and the units that cast none;
//! - translucent walls and roofs, by their alpha byte;
//! - the line and rectangle primitives the UI and automap draw with (`gdi_line`,
//!   `gdi_rectangle`).
//!
//! Integer math only.

use crate::scene::{
    BlendOp, DrawItem, FrameId, FrameImage, FrameView, GradientKind, MapId, PixelTables, Rect,
    ShadeChain,
};

use super::camera::{Camera, ClientPos, FrameSize};

use super::shading::{BlockLight, ShadeTables};

/// The opaque draw mode: no blend table.
pub const MODE_OPAQUE: u8 = 5;
/// The hover highlight: the cel is drawn through the highlight map instead of its light map.
pub const MODE_HIGHLIGHT: u8 = 7;

/// The six blend tables of the palette-table block. The comments give each table's offset in
/// the act's PL2 file and its slot in the loaded block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendTable {
    /// Alpha level 0, the most opaque translucency (PL2 `0x3500`, block `+0x00`).
    A0,
    /// Alpha level 1 (`0x13500`, `+0x04`).
    A1,
    /// Alpha level 2, the most transparent (`0x23500`, `+0x08`).
    A2,
    /// Additive (`0x33500`, `+0x104`).
    Add,
    /// Multiplicative (`0x43500`, `+0x108`).
    Mul,
    /// Max-component (`0x5B500`, `+0x10C`).
    Max,
}

impl BlendTable {
    /// The table's first row in `tables`.
    pub fn base(self, tables: &ShadeTables) -> MapId {
        match self {
            BlendTable::A0 => tables.alpha[0],
            BlendTable::A1 => tables.alpha[1],
            BlendTable::A2 => tables.alpha[2],
            BlendTable::Add => tables.additive,
            BlendTable::Mul => tables.multiplicative,
            BlendTable::Max => tables.max_component,
        }
    }
}

/// The blend table `T` of a draw mode: 0 → `A2`, 1 → `A1`, 2 → `A0`, 3 → `ADD`, 4 → `MUL`,
/// 6 → `MAX`. Modes 5 (opaque), 7 (highlight) and any other value (data can give modes
/// outside 0–7) blend with no table.
pub fn mode_table(mode: u8) -> Option<BlendTable> {
    match mode {
        0 => Some(BlendTable::A2),
        1 => Some(BlendTable::A1),
        2 => Some(BlendTable::A0),
        3 => Some(BlendTable::Add),
        4 => Some(BlendTable::Mul),
        6 => Some(BlendTable::Max),
        _ => None,
    }
}

/// The tables a cel draw hands the row drawer: `P` the colormap (`remap`), `L` the light map
/// of light byte `v` (the highlight map `H` in mode 7), `T` the blend table of the mode.
pub fn cel_tables(tables: &ShadeTables, mode: u8, remap: Option<MapId>, v: u8) -> PixelTables {
    let light = if mode == MODE_HIGHLIGHT {
        Some(tables.highlight)
    } else {
        tables.cel_light(v)
    };
    PixelTables {
        remap,
        light,
        blend: mode_table(mode).map(|t| t.base(tables)),
    }
}

/// The shade chain and blend op of a cel draw (row = destination): `d' = T[256·d + P[s]]`,
/// `T[256·d + L[s]]` when lit (a lit blended draw drops `P`), or `L[P[s]]` without `T`.
pub fn cel_ops(
    tables: &ShadeTables,
    mode: u8,
    remap: Option<MapId>,
    v: u8,
) -> (ShadeChain, BlendOp) {
    cel_tables(tables, mode, remap, v).ops()
}

/// What kind of unit a composite belongs to, for the unit override `r`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnitKind {
    Player,
    Monster,
    Other,
}

/// The inputs of the unit override `r`: the per-unit and per-item reasons a component is
/// drawn translucent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverrideInput {
    pub kind: UnitKind,
    /// Stat 181 `fade`.
    pub fade: i32,
    /// Monster only: some item of its inventory list has item flag 0x400000 (ethereal).
    /// Which inventory list is read is still open.
    pub monster_has_ethereal: bool,
    /// The component's item record: `transparent` (items `+300`) and `transtbl` (`+301`);
    /// `None` without an item.
    pub item_trans: Option<(u8, u8)>,
    /// COF component id of the draw (5 RH, 6 LH, 7 SH).
    pub component: u8,
    /// The item on this component is ethereal.
    pub item_ethereal: bool,
}

/// The unit override mode `r`, `None` for no override. First match wins:
///
/// 1. a fading player, or a monster with fade 1–15, draws in mode 1;
/// 2. a monster with fade above 15 draws in mode 1 only when it carries an ethereal item,
///    and otherwise stops here with no override;
/// 3. an item flagged `transparent` draws in its `transtbl` mode;
/// 4. a player's ethereal weapon (right or left hand) draws in mode 1, an ethereal shield in
///    mode 2.
pub fn unit_override(i: &OverrideInput) -> Option<u8> {
    match i.kind {
        UnitKind::Player if i.fade != 0 => return Some(1),
        UnitKind::Monster if (1..=15).contains(&i.fade) => return Some(1),
        UnitKind::Monster if i.fade > 15 => {
            // No override and stop: the item's `transparent` is not consulted.
            return i.monster_has_ethereal.then_some(1);
        }
        _ => {}
    }
    if let Some((transparent, transtbl)) = i.item_trans {
        if transparent != 0 {
            return Some(transtbl);
        }
    }
    if i.kind == UnitKind::Player && i.item_ethereal {
        match i.component {
            5 | 6 => return Some(1),
            7 => return Some(2),
            _ => {}
        }
    }
    None
}

/// Whether the hover target is drawn highlighted (`h`): every hover target except an object
/// whose `objects.SubClass` has bit 0x80. `object_subclass` is `None` for non-objects.
pub fn hover_highlighted(is_hover_target: bool, object_subclass: Option<u8>) -> bool {
    is_hover_target && object_subclass.is_none_or(|s| s & 0x80 == 0)
}

/// The draw mode of a composite component: ghostly units draw in mode 1, then the unit
/// override wins, then the COF layer's own mode, then the hover highlight, else opaque.
/// `layer_override` is the COF layer's new level `lv` when its override byte is set.
pub fn component_mode(
    ghostly: bool,
    unit_override: Option<u8>,
    hovered: bool,
    layer_override: Option<u8>,
) -> u8 {
    if ghostly {
        return 1;
    }
    if let Some(r) = unit_override {
        return r;
    }
    match layer_override {
        // An override layer is never highlighted.
        Some(lv) => lv,
        None if hovered => MODE_HIGHLIGHT,
        None => MODE_OPAQUE,
    }
}

/// The draw mode of a missile: `missiles.Trans` 1 → 3 (additive), 2 → 4 (multiplicative),
/// else 5; 7 when it is the hover target.
pub fn missile_mode(trans: u8, hovered: bool) -> u8 {
    if hovered {
        return MODE_HIGHLIGHT;
    }
    match trans {
        1 => 3,
        2 => 4,
        _ => MODE_OPAQUE,
    }
}

/// The draw mode of an item on the ground: 5, or 7 when hovered.
pub fn item_mode(hovered: bool) -> u8 {
    if hovered {
        MODE_HIGHLIGHT
    } else {
        MODE_OPAQUE
    }
}

/// The draw mode of an overlay: `overlay.Trans` as is. Overlays draw without a colormap, so
/// a blood colormap on the unit does not tint them.
pub fn overlay_mode(trans: u8) -> u8 {
    trans
}

/// The ops of a unit shadow pixel: the chain `[Z]` turns every source pixel into index 0,
/// then `d' = A0[256·d + 0]` with Blended Shadows on (darkening what is below), else
/// `d' = 0` (a solid black shadow).
pub fn unit_shadow_ops(tables: &ShadeTables, blended: bool) -> (ShadeChain, BlendOp) {
    let chain = ShadeChain::new(&[tables.zero]).expect("one map");
    if blended {
        (chain, BlendOp::IndexTable(tables.alpha[0]))
    } else {
        (chain, BlendOp::Opaque)
    }
}

/// The ops of a shadow tile: blended `d' = A0[256·d + s]`, else an opaque copy.
pub fn shadow_tile_ops(tables: &ShadeTables, blended: bool) -> (ShadeChain, BlendOp) {
    if blended {
        (ShadeChain::EMPTY, BlendOp::IndexTable(tables.alpha[0]))
    } else {
        (ShadeChain::EMPTY, BlendOp::Opaque)
    }
}

/// A unit shadow of one cel as an image: the cel squashed to half height and sheared to the
/// left as it rises. Drawn row `k` is source row `2k` from the bottom, on screen row `y0 − k`
/// from column `x0 − k`, `⌊h / 2⌋` rows, `y0 = Y + trunc(yoff / 2)`,
/// `x0 = X + xoff + trunc(yoff / 2)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowImage {
    /// Opaque where the source pixel is (any non-zero index; the shadow ops ignore it).
    pub image: FrameImage,
    /// Screen top-left of `image` (the `DrawItem` position).
    pub x: i32,
    pub y: i32,
}

/// Builds the sheared shadow of `cel` (rows top first, the bottom row last) drawn at
/// `(x, y)` with offsets `(xoff, yoff)`. Rows and columns outside the frame and the cel clip
/// are left to the draw item's clip.
pub fn shadow_image(cel: &FrameView<'_>, x: i32, y: i32, xoff: i32, yoff: i32) -> ShadowImage {
    let (w, h) = (cel.width(), cel.height());
    let rows = h / 2;
    let half = yoff / 2; // truncation toward zero
    let x0 = x + xoff + half;
    let y0 = y + half;
    let width = if rows == 0 { 0 } else { w + rows - 1 };
    let mut pixels = vec![0u8; width as usize * rows as usize];
    let src = cel.pixels();
    for j in 0..rows {
        // Image row j (top first) is drawn row k = rows − 1 − j, source row
        // 2k from the bottom, shifted right by j inside the image.
        let k = rows - 1 - j;
        let sy = (h - 1 - 2 * k) as usize;
        let line = &src[sy * w as usize..(sy + 1) * w as usize];
        let out = &mut pixels[(j * width + j) as usize..(j * width + j + w) as usize];
        out.copy_from_slice(line);
    }
    let lift = rows.saturating_sub(1) as i32;
    ShadowImage {
        image: FrameImage {
            width,
            height: rows,
            pixels,
        },
        x: x0 - lift,
        y: y0 - lift,
    }
}

/// Whether the shadow slot draws a cel at all: a coarse on-screen test made before the
/// shadow is built. With `(x, y)` the passed shadow position (the unit's draw position,
/// `x − 2`), the cel's size `w × h`, its `xoff` and bottom-row `yoff`, and the view
/// `view_w × view_h`, the cel is drawn only when `left = x − h + xoff` is left of the right
/// edge, `left + w + h` is not left of the left edge, `y + yoff` is not above the top and
/// `y + yoff − ⌊h / 2⌋` is above the bottom.
#[allow(clippy::too_many_arguments)]
pub fn shadow_cel_drawn(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    xoff: i32,
    yoff: i32,
    view_w: i32,
    view_h: i32,
) -> bool {
    let left = x - h + xoff;
    let bottom = y + yoff;
    left < view_w && left + w + h >= 0 && bottom >= 0 && bottom - (h >> 1) < view_h
}

/// How a wall or roof block with alpha byte `a` is drawn. Walls in front of the player fade
/// by lowering this byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WallDraw {
    /// `a` = 0xFF: the lit wall drawer (flat, unlit or gradient light).
    Lit,
    /// `a` < 0xFF: the translucent wall drawer with this table.
    Translucent(BlendTable),
    /// `a` < 0x40: the block is not drawn.
    Hidden,
}

/// The wall drawer of alpha byte `a`: 0xC0…0xFE `A0`, 0x80…0xBF `A1`, 0x40…0x7F `A2`, below
/// 0x40 not drawn.
pub fn wall_draw(a: u8) -> WallDraw {
    match a {
        0xFF => WallDraw::Lit,
        0xC0..=0xFE => WallDraw::Translucent(BlendTable::A0),
        0x80..=0xBF => WallDraw::Translucent(BlendTable::A1),
        0x40..=0x7F => WallDraw::Translucent(BlendTable::A2),
        _ => WallDraw::Hidden,
    }
}

/// The ops of a wall or roof block at screen `(x, y)` with alpha `a` and light corners
/// `c0…c3`, or `None` when the block is not drawn. Lit: flat, unlit or gradient light,
/// opaque. Translucent: always the gradient light map (no flat or unlit branch) and the
/// transposed read `d' = T[256·L[s] + d]`.
pub fn wall_block_ops(
    tables: &ShadeTables,
    a: u8,
    corners: [u8; 4],
    low_quality: bool,
    x: i32,
    y: i32,
) -> Option<(ShadeChain, BlendOp)> {
    match wall_draw(a) {
        WallDraw::Hidden => None,
        WallDraw::Lit => {
            let light = super::shading::wall_block_light(corners, low_quality);
            Some((
                tables.block_chain(light, GradientKind::Wall, x, y),
                BlendOp::Opaque,
            ))
        }
        WallDraw::Translucent(t) => Some((
            tables.block_chain(BlockLight::Gradient(corners), GradientKind::Wall, x, y),
            BlendOp::IndexTableSrcRow(t.base(tables)),
        )),
    }
}

/// Errors of the blend rules. A case the rules leave open is an error naming it, never a
/// default.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlendError {
    #[error("perspective mode is not GDI (§5 r3: perspective position is not specified)")]
    Perspective,
    #[error("GDI rectangle y1 {y1} < y0 {y0}: fatal error 0x32 (§8 r2)")]
    RectangleRowsReversed { y0: i32, y1: i32 },
    #[error("GDI rectangle x1 {x1} < x0 {x0}: negative row width (§8 r2)")]
    RectangleColumnsReversed { x0: i32, x1: i32 },
}

/// Unit flags `+0xC4` bit 5: the unit casts no shadow.
pub const UNIT_FLAG_NO_SHADOW: u32 = 1 << 5;
/// State 146 `invis`: an invisible unit casts no shadow.
pub const STATE_INVIS: u16 = 146;

/// Whether a unit has no shadow (for both shadow kinds): unit flags `+0xC4` bit 5 set, or
/// state 146 `invis`.
pub fn unit_shadow_skipped(unit_flags: u32, invis: bool) -> bool {
    unit_flags & UNIT_FLAG_NO_SHADOW != 0 || invis
}

/// The `objects` fields a shadow reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObjectShadow {
    /// `Xoffset` (`+0x148`), `Yoffset` (`+0x14C`).
    pub xoffset: i32,
    pub yoffset: i32,
    /// `Draw` (`+0x150`) ≠ 0.
    pub draw: bool,
    /// `BlocksLight` of the object's mode (`+0x118 + mode`) ≠ 0.
    pub blocks_light: bool,
}

/// The motion-record values a shadow position reads: the unit's draw offsets and its raw
/// 16.16 position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShadowMotion {
    /// The draw offsets `ox`, `oy` and the height `oz`.
    pub ox: i32,
    pub oy: i32,
    pub oz: i32,
    /// The raw x, y; 0 without a motion record.
    pub mx: i32,
    pub my: i32,
}

/// The shadow position of a composite unit (types 0–2: players, monsters, objects):
/// `X = px + h + ox − (cx_u − shiftX) − 2`, `Y = py + h + oy − (cy_u − 8)` with
/// `h = oz / 2` (truncating division), so a unit lifted off the ground casts its shadow down
/// and to the right. An object (type 2) adds its `Xoffset` / `Yoffset` with no `Draw` test.
/// `perspective` (never in the 2D view) is [`BlendError::Perspective`]. The skips are
/// [`unit_shadow_skipped`].
pub fn composite_shadow_position(
    camera: &Camera,
    perspective: bool,
    at: ClientPos,
    m: &ShadowMotion,
    object: Option<&ObjectShadow>,
) -> Result<(i32, i32), BlendError> {
    if perspective {
        return Err(BlendError::Perspective);
    }
    let h = m.oz / 2; // Rust `/` truncates toward zero, as C
    let mut x = at.x + h + m.ox - (camera.unit.x - camera.view.shift_x) - 2;
    let mut y = at.y + h + m.oy - (camera.unit.y - 8);
    if let Some(o) = object {
        x += o.xoffset;
        y += o.yoffset;
    }
    Ok((x, y))
}

/// The shadow position of a single-cel unit (types 3 and up): `X = px + (mx >> 11) −
/// (cx_u − shiftX)`, `Y = py + (my >> 11) − (cy_u − 8)` (arithmetic shifts; no −2, no
/// `oz`). An object needs `Draw` and `BlocksLight` (else `None`: no shadow), then adds
/// `Xoffset` / `Yoffset`. The skips are [`unit_shadow_skipped`].
pub fn single_cel_shadow_position(
    camera: &Camera,
    perspective: bool,
    at: ClientPos,
    m: &ShadowMotion,
    object: Option<&ObjectShadow>,
) -> Result<Option<(i32, i32)>, BlendError> {
    if perspective {
        return Err(BlendError::Perspective);
    }
    let mut x = at.x + (m.mx >> 11) - (camera.unit.x - camera.view.shift_x);
    let mut y = at.y + (m.my >> 11) - (camera.unit.y - 8);
    if let Some(o) = object {
        if !o.draw || !o.blocks_light {
            return Ok(None);
        }
        x += o.xoffset;
        y += o.yoffset;
    }
    Ok(Some((x, y)))
}

/// The shadow position of a unit of `unit_type`: `None` when the unit casts no shadow (flags
/// `+0xC4` bit 5, state `invis`, or a single-cel object without `Draw` and `BlocksLight`);
/// types 0–2 (composite) by [`composite_shadow_position`], 3 and up by
/// [`single_cel_shadow_position`].
#[allow(clippy::too_many_arguments)]
pub fn unit_shadow_position(
    camera: &Camera,
    perspective: bool,
    unit_type: u8,
    unit_flags: u32,
    invis: bool,
    at: ClientPos,
    m: &ShadowMotion,
    object: Option<&ObjectShadow>,
) -> Result<Option<(i32, i32)>, BlendError> {
    if unit_shadow_skipped(unit_flags, invis) {
        return Ok(None);
    }
    if unit_type <= 2 {
        composite_shadow_position(camera, perspective, at, m, object).map(Some)
    } else {
        single_cel_shadow_position(camera, perspective, at, m, object)
    }
}

/// One line or rectangle primitive as a draw item's inputs: `image` holds index 1 on the
/// touched pixels (0 elsewhere), the chain turns 1 into the written source and the op does
/// the write. The clip is the surface `[0, W) × [0, H)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GdiDraw {
    pub image: FrameImage,
    pub x: i32,
    pub y: i32,
    pub clip: Rect,
    pub shade: ShadeChain,
    pub blend: BlendOp,
}

impl GdiDraw {
    /// The draw item of `image` stored as `frame`.
    pub fn item(&self, frame: FrameId) -> DrawItem {
        let mut it = DrawItem::new(frame, self.x, self.y);
        it.clip = self.clip;
        it.shade = self.shade;
        it.blend = self.blend;
        it
    }
}

/// The map row a primitive of `color` needs (every entry `color`): pushed once per color, its
/// id is the `color_map` of [`gdi_line`] and [`gdi_rectangle`]. A mapped 0 still draws: index
/// 0 is written opaque, not skipped as transparent.
pub fn color_row(color: u8) -> [u8; 256] {
    [color; 256]
}

/// The pixels of a line from `(x0, y0)` to `(x1, y1)` in draw order, unclipped: the first is
/// `(x0, y0)`, then `n = max(|Δx|, |Δy|)` steps along the major axis. The error starts at 0,
/// gains the minor distance each step, and when it **exceeds** the major distance the minor
/// axis advances one pixel toward the end and the error loses the major distance. The major
/// axis is y only when |Δx| < |Δy|, so a 45° line is x-major; a line with a non-zero minor
/// distance ends one pixel short of (x1, y1) on the minor axis. Never an error (the `Result`
/// stays for the callers).
pub fn gdi_line_pixels(x0: i32, y0: i32, x1: i32, y1: i32) -> Result<Vec<(i32, i32)>, BlendError> {
    let (dx, dy) = (i64::from(x1) - i64::from(x0), i64::from(y1) - i64::from(y0));
    let (adx, ady) = (dx.unsigned_abs(), dy.unsigned_abs());
    let x_major = adx >= ady;
    let (major, minor) = if x_major { (adx, ady) } else { (ady, adx) };
    let (sx, sy) = (dx.signum(), dy.signum());
    let (mut x, mut y) = (i64::from(x0), i64::from(y0));
    let mut out = Vec::with_capacity(major as usize + 1);
    out.push((x as i32, y as i32));
    let mut err = 0u64;
    for _ in 0..major {
        err += minor;
        let advance = err > major;
        if advance {
            err -= major;
        }
        if x_major {
            x += sx;
            if advance {
                y += sy;
            }
        } else {
            y += sy;
            if advance {
                x += sx;
            }
        }
        out.push((x as i32, y as i32));
    }
    Ok(out)
}

/// The line primitive: every pixel of [`gdi_line_pixels`] inside `[0, W) × [0, H)` set to
/// `color` (opaque; a line has no alpha). `color_map` is the pushed [`color_row`] of the
/// color. `None` when no pixel is on the surface.
pub fn gdi_line(
    size: FrameSize,
    color_map: MapId,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) -> Result<Option<GdiDraw>, BlendError> {
    let clip = size.rect();
    let pixels: Vec<(i32, i32)> = gdi_line_pixels(x0, y0, x1, y1)?
        .into_iter()
        .filter(|&(x, y)| x >= 0 && y >= 0 && x < size.width && y < size.height)
        .collect();
    let (Some(left), Some(top)) = (
        pixels.iter().map(|p| p.0).min(),
        pixels.iter().map(|p| p.1).min(),
    ) else {
        return Ok(None);
    };
    let right = pixels.iter().map(|p| p.0).max().unwrap_or(left);
    let bottom = pixels.iter().map(|p| p.1).max().unwrap_or(top);
    let (w, h) = ((right - left + 1) as u32, (bottom - top + 1) as u32);
    let mut image = vec![0u8; w as usize * h as usize];
    for (x, y) in pixels {
        image[(y - top) as usize * w as usize + (x - left) as usize] = 1;
    }
    Ok(Some(GdiDraw {
        image: FrameImage {
            width: w,
            height: h,
            pixels: image,
        },
        x: left,
        y: top,
        clip,
        shade: ShadeChain::new(&[color_map]).expect("one map"),
        blend: BlendOp::Opaque,
    }))
}

/// How a rectangle of a draw mode combines with the screen, `k`: modes 0–7 → 2, 2, 2, 1, 1,
/// 0, 1, 0; any other mode → 0. 0 writes the color, 1 runs the screen through the table
/// alone, 2 blends the color with the screen.
pub fn gdi_mode_value(mode: u8) -> u8 {
    match mode {
        0..=2 => 2,
        3 | 4 | 6 => 1,
        _ => 0,
    }
}

/// The pixels the rectangle primitive covers: each coordinate is clamped to `[0, W − 1]` (x)
/// or `[0, H − 1]` (y); `None` (nothing drawn) when `x0 = x1` or `y0 = y1`. After that empty
/// test, `y1 < y0` and then `x1 < x0` are fatal errors. Else the top-left `(x0, y0)` and the
/// size `(x1 − x0, y1 − y0)`: columns `x0 … x1 − 1`, rows `y0 … y1 − 1`.
pub fn gdi_rectangle_box(
    size: FrameSize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
) -> Result<Option<(i32, i32, u32, u32)>, BlendError> {
    let cx = |v: i32| v.clamp(0, (size.width - 1).max(0));
    let cy = |v: i32| v.clamp(0, (size.height - 1).max(0));
    let (x0, y0, x1, y1) = (cx(x0), cy(y0), cx(x1), cy(y1));
    if x0 == x1 || y0 == y1 {
        return Ok(None);
    }
    if y1 < y0 {
        return Err(BlendError::RectangleRowsReversed { y0, y1 });
    }
    if x1 < x0 {
        return Err(BlendError::RectangleColumnsReversed { x0, x1 });
    }
    Ok(Some((x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)))
}

/// The pixel write of a rectangle of draw mode `mode` over an image of index 1, by `k`
/// ([`gdi_mode_value`]): 0 → `d' = color` (chain `[color]`, opaque); 1 → `d' = T[d]` (row 0
/// of `T`, column `d`: chain `[Z]` and the transposed read); 2 → `d' = T[256·d + color]`
/// (chain `[color]`, `T`). `color_map` is the pushed [`color_row`] of the color. `None` when
/// the mode needs `T` and `tables` is `None`.
pub fn gdi_rectangle_ops(
    tables: Option<&ShadeTables>,
    color_map: MapId,
    mode: u8,
) -> Option<(ShadeChain, BlendOp)> {
    let k = gdi_mode_value(mode);
    let table = match (k, tables) {
        (0, _) => None,
        (_, Some(tables)) => mode_table(mode).map(|t| (tables, t.base(tables))),
        (_, None) => return None,
    };
    Some(match (k, table) {
        (1, Some((tables, t))) => (
            ShadeChain::new(&[tables.zero]).expect("one map"),
            BlendOp::IndexTableSrcRow(t),
        ),
        (2, Some((_, t))) => (
            ShadeChain::new(&[color_map]).expect("one map"),
            BlendOp::IndexTable(t),
        ),
        _ => (
            ShadeChain::new(&[color_map]).expect("one map"),
            BlendOp::Opaque,
        ),
    })
}

/// The rectangle primitive: the pixels of [`gdi_rectangle_box`] written by
/// [`gdi_rectangle_ops`].
#[allow(clippy::too_many_arguments)]
pub fn gdi_rectangle(
    tables: &ShadeTables,
    size: FrameSize,
    color_map: MapId,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    mode: u8,
) -> Result<Option<GdiDraw>, BlendError> {
    let Some((x0, y0, w, h)) = gdi_rectangle_box(size, x0, y0, x1, y1)? else {
        return Ok(None);
    };
    let (shade, blend) = gdi_rectangle_ops(Some(tables), color_map, mode).expect("tables given");
    Ok(Some(GdiDraw {
        image: FrameImage {
            width: w,
            height: h,
            pixels: vec![1; w as usize * h as usize],
        },
        x: x0,
        y: y0,
        clip: size.rect(),
        shade,
        blend,
    }))
}
