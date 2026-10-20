//! Draw items and their inputs: frames, the map table, shade chains, blend ops.
//!
//! A `DrawItem` is one sprite draw: which frame, where, clipped to what, shaded through which
//! chain of index maps and blended onto the framebuffer with which op. The compositor in
//! [`super::cpu`] turns each covered pixel into one framebuffer write: skip index 0, run the
//! shade chain, then apply the blend op. This module defines those pieces and the per-pixel
//! maths, including the per-pixel light gradient of DT1 tile blocks.

use super::order::DrawKey;
use super::{Rect, SceneError, MAX_SHADE};

/// Identifies one decoded index frame. The asset side (`IndexFrame` and the atlas) owns the numbering; the atlas slot is looked up from it by the
/// GPU compositor, never stored here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameId(pub u32);

/// A borrowed 8-bit index image: row-major, top row first, index 0 =
/// transparent. The pixel count is checked on construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameView<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

impl<'a> FrameView<'a> {
    /// Wraps `pixels` as a `width × height` image; the length must match.
    pub fn new(width: u32, height: u32, pixels: &'a [u8]) -> Result<Self, SceneError> {
        if pixels.len() as u64 != u64::from(width) * u64::from(height) {
            return Err(SceneError::FrameSize {
                width,
                height,
                len: pixels.len(),
            });
        }
        Ok(FrameView {
            width,
            height,
            pixels,
        })
    }

    /// Image width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The row-major index pixels.
    pub fn pixels(&self) -> &'a [u8] {
        self.pixels
    }

    /// Index at frame coordinates; the caller keeps them in range.
    fn at(&self, x: u32, y: u32) -> u8 {
        self.pixels[y as usize * self.width as usize + x as usize]
    }
}

/// Where the compositor reads frames from. The frame store implements this; a frame that is not
/// resident is [`SceneError::FrameMissing`], never a skipped draw.
pub trait FrameSource {
    /// The frame with this id, or an error if it is not resident.
    fn frame(&self, id: FrameId) -> Result<FrameView<'_>, SceneError>;
}

/// An owned index image, for tests and synthetic cases. Offsets are not
/// part of it: a [`DrawItem`] position is already the image's top-left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Row-major index pixels, 0 = transparent.
    pub pixels: Vec<u8>,
}

/// A slice of images is a frame source: `FrameId(n)` is element `n`.
impl FrameSource for [FrameImage] {
    fn frame(&self, id: FrameId) -> Result<FrameView<'_>, SceneError> {
        let img = self
            .get(id.0 as usize)
            .ok_or(SceneError::FrameMissing(id))?;
        FrameView::new(img.width, img.height, &img.pixels)
    }
}

impl FrameSource for Vec<FrameImage> {
    fn frame(&self, id: FrameId) -> Result<FrameView<'_>, SceneError> {
        self.as_slice().frame(id)
    }
}

/// A row of the [`MapTable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MapId(pub u32);

/// Every index map of the frame, one row of 256 bytes each: PL2 maps and
/// the rows of 256×256 blend tables alike. The GPU uploads the same
/// rows as one storage buffer, so a `MapId` means the same on both sides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapTable {
    rows: Vec<[u8; 256]>,
}

impl MapTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one row and returns its id.
    pub fn push(&mut self, row: [u8; 256]) -> MapId {
        let id = MapId(self.rows.len() as u32);
        self.rows.push(row);
        id
    }

    /// Appends a 256×256 blend table and returns the base id for
    /// [`BlendOp::IndexTable`]. The table is `table[dest][src]`: row =
    /// destination index, column = source index, which is the layout of the PL2 blend tables, so
    /// a PL2 table is pushed unchanged.
    pub fn push_table(&mut self, table: &[[u8; 256]; 256]) -> MapId {
        let id = MapId(self.rows.len() as u32);
        self.rows.extend_from_slice(table);
        id
    }

    /// The row with this id, if it exists.
    pub fn get(&self, id: MapId) -> Option<&[u8; 256]> {
        self.rows.get(id.0 as usize)
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when the table has no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// All rows in id order (what the GPU uploads).
    pub fn rows(&self) -> &[[u8; 256]] {
        &self.rows
    }

    fn row(&self, id: MapId) -> &[u8; 256] {
        &self.rows[id.0 as usize]
    }
}

/// Up to four maps applied in order to a non-zero source index: `i' = m3[m2[m1[m0[i]]]]`, unused
/// slots skipped; then, when the chain has a [`LightGradient`], the gradient's light map of the
/// pixel (a tile block's light map varies per pixel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShadeChain {
    maps: [MapId; MAX_SHADE],
    len: u8,
    gradient: Option<LightGradient>,
}

impl ShadeChain {
    /// The empty chain: the source index is used unchanged.
    pub const EMPTY: ShadeChain = ShadeChain {
        maps: [MapId(0); MAX_SHADE],
        len: 0,
        gradient: None,
    };

    /// A chain of `maps`, applied in order; more than [`MAX_SHADE`] is an error.
    pub fn new(maps: &[MapId]) -> Result<Self, SceneError> {
        if maps.len() > MAX_SHADE {
            return Err(SceneError::ShadeChainTooLong { len: maps.len() });
        }
        let mut chain = Self::EMPTY;
        chain.maps[..maps.len()].copy_from_slice(maps);
        chain.len = maps.len() as u8;
        Ok(chain)
    }

    /// The chain followed by the per-pixel light map of `gradient`.
    pub fn with_gradient(mut self, gradient: LightGradient) -> Self {
        self.gradient = Some(gradient);
        self
    }

    /// The maps of the chain, in application order.
    pub fn maps(&self) -> &[MapId] {
        &self.maps[..usize::from(self.len)]
    }

    /// The per-pixel light gradient applied after the maps, if any.
    pub fn gradient(&self) -> Option<&LightGradient> {
        self.gradient.as_ref()
    }

    /// Applies the chain at screen pixel `(sx, sy)`. Index 0 is tested by the caller before this;
    /// a mapped result of 0 is used as is: it draws opaque index 0.
    pub(super) fn apply(
        &self,
        maps: &MapTable,
        index: u8,
        sx: i64,
        sy: i64,
        area: &Rect,
        run_cut: u32,
    ) -> u8 {
        let i = self
            .maps()
            .iter()
            .fold(index, |i, &m| maps.row(m)[usize::from(i)]);
        match &self.gradient {
            None => i,
            Some(g) => maps.row(g.map_at(sx, sy, area, run_cut))[usize::from(i)],
        }
    }
}

/// The row geometry of a gradient-lit DT1 block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GradientKind {
    /// Walls and roofs (lit wall and translucent wall drawers): 32 rows,
    /// `a_r = (32·c0 + r·(c3 − c0)) >> 8`.
    Wall,
    /// Floor blocks, RLE and isometric alike: 15 rows, `a_r = (16·c0 + r·(c3 − c0)) >> 7`.
    RleFloor,
    /// Isometric floor blocks: the rows of `RleFloor`, but the light origin is the first drawn
    /// pixel of the clipped block, not the block's own column and row.
    IsoFloor,
}

impl GradientKind {
    /// Width of a block in pixels.
    pub const WIDTH: u32 = 32;

    /// Rows of a block.
    pub fn rows(self) -> u32 {
        match self {
            GradientKind::Wall => 32,
            GradientKind::RleFloor | GradientKind::IsoFloor => 15,
        }
    }

    /// `(scale, shift)` of the row interpolation.
    pub fn row_terms(self) -> (i32, u32) {
        match self {
            GradientKind::Wall => (32, 8),
            GradientKind::RleFloor | GradientKind::IsoFloor => (16, 7),
        }
    }

    /// The GPU code of the kind (0 = no gradient).
    pub fn code(self) -> u32 {
        match self {
            GradientKind::Wall => 1,
            GradientKind::RleFloor => 2,
            GradientKind::IsoFloor => 3,
        }
    }
}

/// Per-pixel light of one DT1 block: the pixel in block column `x`, row `r` uses light map `G[a_r][b_r][x]` with
/// `G[a][b][x] = ⌊(32·a + x·(b − a)) / 32⌋`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightGradient {
    /// Row geometry of the block.
    pub kind: GradientKind,
    /// Screen position of the block's top-left pixel. Every drawn pixel of
    /// the item must lie inside the block (checked by `resolve`).
    pub x: i32,
    /// Screen y of the block's top-left pixel.
    pub y: i32,
    /// `c0` top-left, `c1` top-right, `c2` bottom-right, `c3` bottom-left.
    pub corners: [u8; 4],
    /// Light map 0; light map `k` (0…31) is row `light0 + k`.
    pub light0: MapId,
}

impl LightGradient {
    /// Number of light maps the gradient indexes.
    pub const LIGHT_MAPS: u32 = 32;

    /// The block's screen rectangle.
    pub fn block(&self) -> Rect {
        Rect::new(self.x, self.y, GradientKind::WIDTH, self.kind.rows())
    }

    /// Light map number (0…31) of block column `x`, row `r` (both inside
    /// the block).
    pub fn level(&self, x: u32, r: u32) -> u32 {
        let [c0, c1, c2, c3] = self.corners.map(i32::from);
        let (scale, shift) = self.kind.row_terms();
        let r = r as i32;
        let a = (scale * c0 + r * (c3 - c0)) >> shift;
        let b = (scale * c1 + r * (c2 - c1)) >> shift;
        gradient(a as u32, b as u32, x) as u32
    }

    fn map_at(&self, sx: i64, sy: i64, area: &Rect, run_cut: u32) -> MapId {
        let mut x = (sx - i64::from(self.x)) as u32;
        let mut r = (sy - i64::from(self.y)) as u32;
        if self.kind == GradientKind::IsoFloor {
            // Isometric floor: the light row starts at 0 on the first drawn row, and in each row
            // the light column starts at the row's own first column `s_r` on the first drawn
            // pixel, however many pixels of the row the left clip cut.
            let (cut_x, cut_y) = (
                (i64::from(area.x) - i64::from(self.x)).max(0) as u32,
                (i64::from(area.y) - i64::from(self.y)).max(0) as u32,
            );
            r -= cut_y;
            x -= cut_x.saturating_sub((14 - 2 * (r + cut_y) as i32).unsigned_abs());
        }
        // Clipped wall or RLE floor block: the light column is not advanced by the pixels the
        // left clip cut from the run (`run_cut`).
        x = x.saturating_sub(run_cut);
        MapId(self.light0.0 + self.level(x, r))
    }
}

/// The gradient table `G[a][b][x] = ⌊(32·a + x·(b − a)) / 32⌋` (`a`, `b`, `x` in 0…31, result
/// in 0…31).
pub fn gradient(a: u32, b: u32, x: u32) -> u8 {
    let (a, b, x) = (a as i32, b as i32, x as i32);
    ((32 * a + x * (b - a)) >> 5) as u8
}

impl Default for ShadeChain {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// How a shaded source index combines with the destination. The framebuffer is indexed (u8 per
/// pixel, palette applied at present): D2RUST composes in the index domain, so RGB blending is
/// not an op.
///
/// One pixel write is a shade chain then a blend op; [`PixelTables::ops`] builds both from the
/// draw's `P`, `L` and `T` (with `L` and `T` the remap `P` is dropped). Which op and table each
/// draw uses is decided in `rules::blend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendOp {
    /// `dest = src` (no `T`: `d' = L[P[s]]`).
    Opaque,
    /// `dest = map[base + dest][src]`: a 256×256 table stored as rows
    /// `base..base + 256` of the map table, row = destination, column =
    /// source (`T[256 × d + s]`): cels, shadows, shadow tiles.
    IndexTable(MapId),
    /// `dest = map[base + src][dest]`: the same table read transposed, row
    /// = source (`T[256 × L[s] + d]`): the lit translucent wall draw.
    IndexTableSrcRow(MapId),
}

impl BlendOp {
    pub(super) fn apply(&self, maps: &MapTable, src: u8, dest: u8) -> u8 {
        match *self {
            BlendOp::Opaque => src,
            BlendOp::IndexTable(base) => {
                maps.row(MapId(base.0 + u32::from(dest)))[usize::from(src)]
            }
            BlendOp::IndexTableSrcRow(base) => {
                maps.row(MapId(base.0 + u32::from(src)))[usize::from(dest)]
            }
        }
    }

    /// The first row of the op's 256×256 table, if it has one.
    pub fn table(&self) -> Option<MapId> {
        match *self {
            BlendOp::Opaque => None,
            BlendOp::IndexTable(base) | BlendOp::IndexTableSrcRow(base) => Some(base),
        }
    }
}

/// The up to three tables one draw passes to the pixel writer: remap `P`, light `L` and blend
/// table `T` (the base id of a 256×256 table pushed with [`MapTable::push_table`]). Which draw
/// passes which tables is decided in `rules::blend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PixelTables {
    /// Remap `P`.
    pub remap: Option<MapId>,
    /// Light map `L`.
    pub light: Option<MapId>,
    /// Base of the blend table `T`.
    pub blend: Option<MapId>,
}

impl PixelTables {
    /// The shade chain and blend op for these tables:
    /// - no `T`: `d' = L[P[s]]` (chain `P`, `L`; absent steps skipped);
    /// - `T`, no `L`: `d' = T[256 × d + P[s]]`;
    /// - `T` and `L`: `d' = T[256 × d + L[s]]`: `P` is not applied, whether
    ///   or not the draw passed one (the dispatcher picks the `L` routine).
    pub fn ops(&self) -> (ShadeChain, BlendOp) {
        let chain = |maps: &[Option<MapId>]| {
            let mut c = ShadeChain::EMPTY;
            for m in maps.iter().flatten() {
                c.maps[usize::from(c.len)] = *m;
                c.len += 1;
            }
            c
        };
        match (self.light, self.blend) {
            (_, None) => (chain(&[self.remap, self.light]), BlendOp::Opaque),
            (None, Some(t)) => (chain(&[self.remap]), BlendOp::IndexTable(t)),
            (Some(l), Some(t)) => (chain(&[Some(l)]), BlendOp::IndexTable(t)),
        }
    }
}

/// Debug label of an item; never read by the compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ItemTag {
    /// No label.
    #[default]
    None,
    /// A unit by GUID.
    Unit(u32),
    /// A map tile by cell.
    Tile { x: i32, y: i32 },
    /// A UI element by id.
    Ui(u32),
}

/// The palette argument of a cel draw call: none, hue map `k` of the act PL2, a monster's palette
/// shift map `s` (`palshift.dat` map `s`, or RandTransforms map `s − 8`), or another table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CelPal {
    /// No palette.
    Null,
    /// Hue map `k` of the act PL2.
    Hue(u8),
    /// A monster's palette shift map.
    Shift(u8),
    /// Some other table.
    Other,
}

/// The arguments of the cel draw call an item stands for: draw mode and palette. Read by the
/// render facts export only; composition never reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CelArgs {
    /// Draw mode.
    pub mode: u8,
    /// Palette argument.
    pub pal: CelPal,
}

/// One sprite draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawItem {
    /// The frame to draw.
    pub frame: FrameId,
    /// Screen position of the image's top-left, integer pixels. Turning format offsets into
    /// this is the builder's job.
    pub x: i32,
    /// Screen y of the image's top-left.
    pub y: i32,
    /// Reserved; always `false`. No D2RUST draw mirrors horizontally, so `true` is rejected
    /// ([`SceneError::FlipX`]).
    pub flip_x: bool,
    /// Pixels outside this screen rectangle are not drawn.
    pub clip: Rect,
    /// The shade chain applied to each source pixel.
    pub shade: ShadeChain,
    /// How the shaded pixel combines with the framebuffer.
    pub blend: BlendOp,
    /// The sort key that gives the item its place in the draw order.
    pub key: DrawKey,
    /// Debug label.
    pub tag: ItemTag,
    /// The cel call's mode and palette, when the builder knows them.
    pub call: Option<CelArgs>,
}

impl DrawItem {
    /// An opaque, unshaded, unflipped item clipped to the frame
    /// ([`Rect::world_clip`]), key 0.
    pub fn new(frame: FrameId, x: i32, y: i32) -> Self {
        DrawItem {
            frame,
            x,
            y,
            flip_x: false,
            clip: Rect::world_clip(),
            shade: ShadeChain::EMPTY,
            blend: BlendOp::Opaque,
            key: DrawKey::default(),
            tag: ItemTag::None,
            call: None,
        }
    }

    /// Checks everything the compositor will index, so composing cannot
    /// fail or panic afterwards. Returns the frame and the screen rectangle
    /// the item may touch inside `view` (`None`: nothing to draw).
    pub(super) fn resolve<'f, F: FrameSource + ?Sized>(
        &self,
        frames: &'f F,
        maps: &MapTable,
        view: &Rect,
    ) -> Result<(FrameView<'f>, Option<Rect>), SceneError> {
        if self.flip_x {
            return Err(SceneError::FlipX);
        }
        let frame = frames.frame(self.frame)?;
        for &m in self.shade.maps() {
            maps.get(m).ok_or(SceneError::MapMissing(m))?;
        }
        if let Some(base) = self.blend.table() {
            if u64::from(base.0) + 256 > maps.len() as u64 {
                return Err(SceneError::BlendTable(base));
            }
        }
        let image = Rect::new(self.x, self.y, frame.width(), frame.height());
        let area = image.intersect(&self.clip).and_then(|r| r.intersect(view));
        if let Some(g) = self.shade.gradient() {
            let last = u64::from(g.light0.0) + u64::from(LightGradient::LIGHT_MAPS);
            if last > maps.len() as u64 {
                return Err(SceneError::LightMaps(g.light0));
            }
            if let Some(a) = area {
                if a.intersect(&g.block()) != Some(a) {
                    return Err(SceneError::GradientArea {
                        area: a,
                        block: g.block(),
                    });
                }
            }
        }
        Ok((frame, area))
    }

    /// The shaded, blended value of screen pixel `(sx, sy)` over `dest`, or
    /// `dest` unchanged where the frame is transparent. `(sx, sy)` lies in
    /// the area returned by [`DrawItem::resolve`].
    pub(super) fn pixel(
        &self,
        frame: &FrameView<'_>,
        maps: &MapTable,
        sx: i64,
        sy: i64,
        area: &Rect,
        dest: u8,
    ) -> u8 {
        let fx = (sx - i64::from(self.x)) as u32;
        let fy = (sy - i64::from(self.y)) as u32;
        let src = frame.at(fx, fy);
        if src == 0 {
            return dest;
        }
        // Clipped wall or RLE floor block: the pixels of the run the left clip cut do not
        // advance the light column.
        let runs = |k| matches!(k, GradientKind::Wall | GradientKind::RleFloor);
        let run_cut = match self.shade.gradient() {
            Some(g) if runs(g.kind) && i64::from(area.x) > i64::from(g.x) => {
                // The scan stops at the block's left edge, as the shader's does.
                let floor = (i64::from(g.x) - i64::from(self.x)).max(0) as u32;
                let mut start = fx;
                while start > floor && frame.at(start - 1, fy) != 0 {
                    start -= 1;
                }
                (i64::from(area.x) - (i64::from(self.x) + i64::from(start))).max(0) as u32
            }
            _ => 0,
        };
        self.blend.apply(
            maps,
            self.shade.apply(maps, src, sx, sy, area, run_cut),
            dest,
        )
    }
}
