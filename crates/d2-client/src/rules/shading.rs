//! Shading: the act's palette-table block as map-table rows, and which rows a draw reads.
//!
//! Every act has a PL2 file: its base palette plus the 256-entry index maps the renderer
//! shades with. [`ShadeTables::push`] loads that block into a [`MapTable`] (with the computed
//! highlight `H` and red `R` maps), and the rules here pick from it: the light map `L` of a
//! cel draw, the light of DT1 wall and floor blocks (flat, gradient or unlit), the hover
//! light byte, and the remap maps `P` (unit palettes, item colors, monster palette shifts and
//! the green-blood map). Integer math only; the maps are the PL2's own bytes, never a color
//! formula, so every act shades with exactly its own tables.
//!
//! How `P`, `L` and `T` combine is `scene::PixelTables`; which draw mode (and so which blend
//! table `T`) a draw uses is [`super::blend`].

use d2_formats::palette::{Palette, Pl2};

use crate::scene::{GradientKind, LightGradient, MapId, MapTable, ShadeChain};

/// Light maps of the block (`+0x0C + 4k`, k = 0…31).
pub const LIGHT_MAPS: u32 = 32;
/// Hue, tone and unknown variation maps at PL2 `0x53500`: 111 hue variations, the red, green
/// and blue tones, 14 unknown variations.
pub const REMAP_MAPS: u32 = 128;
/// Highlight factor of `H`: `⌊c · 170 / 100⌋`.
const HIGHLIGHT_NUM: u32 = 170;
const HIGHLIGHT_DEN: u32 = 100;

/// Errors of the shading rules. A question the rules leave open is an error naming it, never
/// a default.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShadingError {
    #[error(
        "unit palette index {0} is past the 128 remap maps (render/shading.md §6 r1: p = 1…128 → map p − 1)"
    )]
    RemapIndex(u8),
    #[error("floor light grid of {len} cells has no cell {need} for block ({gx}, {gy})")]
    FloorGrid {
        len: usize,
        need: usize,
        gx: u8,
        gy: u8,
    },
}

/// The palette-table block of one act PL2 pushed into a [`MapTable`]: the rows the cel and
/// tile draws read, plus `Z` (all zero), the shade chain of a unit shadow. Every table is
/// pushed as stored, never transposed (blend tables are `[destination][source]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShadeTables {
    /// Light map 0; map `k` is row `light0 + k`.
    pub light0: MapId,
    /// Highlight `H` (block `+0x118`, computed from the base palette).
    pub highlight: MapId,
    /// Red `R` (block `+0x11C`, computed; no primitive reads it yet).
    pub red: MapId,
    /// `Z[i] = 0` for all `i` (the unit shadow chain).
    pub zero: MapId,
    /// First of the 128 remap maps at PL2 `0x53500`.
    pub remap0: MapId,
    /// Alpha blend levels 0, 1, 2 (block `+0x00`, `+0x04`, `+0x08`).
    pub alpha: [MapId; 3],
    /// Additive (block `+0x104`).
    pub additive: MapId,
    /// Multiplicative (block `+0x108`).
    pub multiplicative: MapId,
    /// Max-component (block `+0x10C`).
    pub max_component: MapId,
}

impl ShadeTables {
    /// Pushes the block of `pl2` (the act's palette) into `maps`. `H` and `R` are computed
    /// from `pl2`'s base palette.
    pub fn push(maps: &mut MapTable, pl2: &Pl2) -> ShadeTables {
        let light0 = push_rows(maps, &pl2.light_levels);
        let highlight = maps.push(highlight_map(&pl2.base_palette));
        let red = maps.push(red_map(&pl2.base_palette));
        let zero = maps.push([0; 256]);
        let remap0 = push_rows(maps, &pl2.hue_variations);
        maps.push(pl2.red_tones);
        maps.push(pl2.green_tones);
        maps.push(pl2.blue_tones);
        push_rows(maps, &pl2.unknown_variations);
        let alpha = [
            push_rows(maps, &pl2.alpha_blend[0]),
            push_rows(maps, &pl2.alpha_blend[1]),
            push_rows(maps, &pl2.alpha_blend[2]),
        ];
        let additive = push_rows(maps, &pl2.additive_blend);
        let multiplicative = push_rows(maps, &pl2.multiplicative_blend);
        let max_component = push_rows(maps, &pl2.max_component_blend);
        ShadeTables {
            light0,
            highlight,
            red,
            zero,
            remap0,
            alpha,
            additive,
            multiplicative,
            max_component,
        }
    }

    /// Light map `k` (0…31; larger `k` is clamped by the callers' `>> 3`).
    pub fn light_map(&self, k: u8) -> MapId {
        debug_assert!(u32::from(k) < LIGHT_MAPS);
        MapId(self.light0.0 + u32::from(k))
    }

    /// `L` of a cel draw with light byte `v`: none for 0xFF (unlit), else light map `v >> 3`.
    /// Draw mode 7 replaces it with [`ShadeTables::highlight`] ([`super::blend::cel_ops`]).
    pub fn cel_light(&self, v: u8) -> Option<MapId> {
        cel_light_level(v).map(|k| self.light_map(k))
    }

    /// `P` of a unit palette index `p` (unit `+0x6C`): none for `p = 0`, else remap map
    /// `p − 1` (1…111 hue, 112–114 red, green, blue tones, 115–128 unknown variations).
    pub fn unit_remap(&self, p: u8) -> Result<Option<MapId>, ShadingError> {
        match p {
            0 => Ok(None),
            p if u32::from(p) <= REMAP_MAPS => Ok(Some(MapId(self.remap0.0 + u32::from(p) - 1))),
            p => Err(ShadingError::RemapIndex(p)),
        }
    }

    /// The light gradient of a block at screen `(x, y)` with `corners`.
    pub fn gradient(&self, kind: GradientKind, x: i32, y: i32, corners: [u8; 4]) -> LightGradient {
        LightGradient {
            kind,
            x,
            y,
            corners,
            light0: self.light0,
        }
    }

    /// The shade chain `[L]` of a lit DT1 block: an unlit block has none, a flat one a single
    /// map, a gradient one the per-pixel map of `kind` at the block's screen top-left
    /// `(x, y)`.
    pub fn block_chain(&self, light: BlockLight, kind: GradientKind, x: i32, y: i32) -> ShadeChain {
        match light {
            BlockLight::Unlit => ShadeChain::EMPTY,
            BlockLight::Flat(k) => ShadeChain::new(&[self.light_map(k)]).expect("one map"),
            BlockLight::Gradient(c) => {
                ShadeChain::EMPTY.with_gradient(self.gradient(kind, x, y, c))
            }
        }
    }
}

fn push_rows(maps: &mut MapTable, rows: &[[u8; 256]]) -> MapId {
    let first = MapId(maps.len() as u32);
    for row in rows {
        maps.push(*row);
    }
    first
}

/// Light map number of a cel light byte `v`: `None` for 0xFF (unlit), else `v >> 3`.
pub fn cel_light_level(v: u8) -> Option<u8> {
    (v != 0xFF).then_some(v >> 3)
}

/// The light byte of a hovered unit, missile or item: doubled and clamped to 0x40…0xFF. The
/// cel draw ignores it in mode 7 (the highlight map replaces the light map); it is kept for
/// the light record.
pub fn hover_light(v: u8) -> u8 {
    (u32::from(v) * 2).clamp(0x40, 0xFF) as u8
}

/// The palette index nearest to `(r, g, b)`: smallest `(R_j − r)² + (G_j − g)² + (B_j − b)²`,
/// the lowest `j` on ties, index 0 included.
pub fn nearest(palette: &Palette, r: u32, g: u32, b: u32) -> u8 {
    let dist = |c: &d2_formats::palette::Rgb| {
        let d = |p: u8, q: u32| (i64::from(p) - i64::from(q)).pow(2);
        d(c.r, r) + d(c.g, g) + d(c.b, b)
    };
    let mut best = 0usize;
    for (j, c) in palette.colors.iter().enumerate().skip(1) {
        if dist(c) < dist(&palette.colors[best]) {
            best = j;
        }
    }
    best as u8
}

/// The highlight map `H`: each color brightened by 70 %, `H[i] = nearest(min(255,
/// ⌊R·170/100⌋), …)`.
pub fn highlight_map(palette: &Palette) -> [u8; 256] {
    let up = |c: u8| (u32::from(c) * HIGHLIGHT_NUM / HIGHLIGHT_DEN).min(255);
    let mut out = [0u8; 256];
    for (o, c) in out.iter_mut().zip(&palette.colors) {
        *o = nearest(palette, up(c.r), up(c.g), up(c.b));
    }
    out
}

/// The red map `R`: each color's red channel alone, `R[i] = nearest(R_i, 0, 0)`.
pub fn red_map(palette: &Palette) -> [u8; 256] {
    let mut out = [0u8; 256];
    for (o, c) in out.iter_mut().zip(&palette.colors) {
        *o = nearest(palette, u32::from(c.r), 0, 0);
    }
    out
}

/// The light of one DT1 block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockLight {
    /// Copied without a light map (walls only, `c0` in 0xF8…0xFF).
    Unlit,
    /// Every pixel uses light map `k`.
    Flat(u8),
    /// Per-pixel gradient from the corners `c0` (top-left), `c1` (top-right), `c2`
    /// (bottom-right), `c3` (bottom-left).
    Gradient([u8; 4]),
}

/// Light of a wall or roof block from its corner values. A gradient when the corners differ
/// by more than 9 in total (and low quality is off); otherwise one flat map from `c0`, or no
/// light map at all when `c0` is fully lit (0xF8 and up). `low_quality` is the low-quality
/// lighting setting (default off).
pub fn wall_block_light(corners: [u8; 4], low_quality: bool) -> BlockLight {
    let [c0, c1, c2, c3] = corners.map(i32::from);
    let delta = (c1 - c0).abs() + (c3 - c0).abs() + (c2 - c1).abs();
    if !low_quality && delta > 9 {
        BlockLight::Gradient(corners)
    } else if corners[0] >= 0xF8 {
        BlockLight::Unlit
    } else {
        BlockLight::Flat(corners[0] >> 3)
    }
}

/// Light of the floor block at grid `(gx, gy)` (block record bytes 6, 7) from the floor light
/// grid `cells` (byte 0 of each 12-byte cell, 8 cells per row). Flat from the block's own
/// cell when its neighbours differ by less than 10 in total (or low quality is on), else a
/// gradient whose corners average the four cells around each corner. A flat floor block is
/// never unlit.
pub fn floor_block_light(
    cells: &[u8],
    gx: u8,
    gy: u8,
    low_quality: bool,
) -> Result<BlockLight, ShadingError> {
    let g = usize::from(gx) + 8 * usize::from(gy);
    let need = g + 26;
    if need >= cells.len() {
        return Err(ShadingError::FloorGrid {
            len: cells.len(),
            need,
            gx,
            gy,
        });
    }
    let e = |n: usize| u32::from(cells[g + n]);
    let delta = e(10).abs_diff(e(9)) + e(17).abs_diff(e(9)) + e(18).abs_diff(e(10));
    if low_quality || delta < 10 {
        return Ok(BlockLight::Flat(cells[g + 9] >> 3));
    }
    // c2 averages e[g+17], not e[g+18]: an asymmetric corner, kept on purpose so floor light
    // stays exactly as designed.
    let avg = |a, b, c, d| ((e(a) + e(b) + e(c) + e(d)) >> 2) as u8;
    Ok(BlockLight::Gradient([
        avg(8, 9, 16, 17),
        avg(1, 2, 9, 10),
        avg(10, 11, 17, 19),
        avg(17, 18, 25, 26),
    ]))
}

/// The shade chain of a floor block at screen `(x, y)`: RLE (block flag bit 2) and isometric
/// blocks alike use the 15-row gradient `a_r = (16·c0 + r·(c3 − c0)) >> 7` with `x` the
/// column inside the 32-wide block; an isometric block's diamond rows only cover their pixels
/// of that block (its transparent image corners draw nothing).
pub fn floor_block_chain(tables: &ShadeTables, light: BlockLight, x: i32, y: i32) -> ShadeChain {
    tables.block_chain(light, GradientKind::RleFloor, x, y)
}

/// The item palette files by color table `t` (1…8), loaded as 21 maps each from
/// `Data\Global\Items\Palette\<name>.dat`.
pub const ITEM_PALETTE_FILES: [&str; 8] = [
    "grey",
    "grey2",
    "gold",
    "brown",
    "greybrown",
    "invgrey",
    "invgrey2",
    "invgreybrown",
];
/// Maps per item palette file.
pub const ITEM_PALETTE_MAPS: u8 = 21;

/// Which item palette map an item color `(t, c)` selects: `(t, c)` for a map of file
/// `ITEM_PALETTE_FILES[t − 1]`, or `None` (no map) for `t` = 0, 3, 4 or ≥ 9, or `c` ≥ 21.
/// Files 3 (`gold`) and 4 (`brown`) are loaded but never selected.
pub fn item_color(t: u8, c: u8) -> Option<(u8, u8)> {
    match t {
        1 | 2 | 5..=8 if c < ITEM_PALETTE_MAPS => Some((t, c)),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Monster palette shift and blood map.
// ---------------------------------------------------------------------

/// Bytes of one remap map.
pub const MAP_BYTES: usize = 256;
/// Maps of `RandTransforms.dat`.
pub const RAND_TRANSFORMS_MAPS: u32 = 30;
/// The shift index every monster graphic starts with.
pub const SHIFT_DEFAULT: u32 = 2;
/// Monster classes whose shift 0 / 1 picks a `palshift` map: 363 `necroskeleton`, 364
/// `necromage` (the summoned skeletons). Every other class draws shift 0 / 1 unshifted.
pub const SHIFT_LOW_CLASSES: [u32; 2] = [363, 364];
/// Offset of `palshift` map 0 from the class's base: `base + 4 + 256·s`.
pub const PALSHIFT_FIRST: usize = 4;
/// Offset of the second 8-map set (the green-blood set) from the class's base:
/// `base + 0x804 + 256·s`.
pub const PALSHIFT_SECOND: usize = 0x804;
/// Fatal error code of a RandTransforms index past 29.
pub const FATAL_RAND_TRANSFORMS: u32 = 0x160;

/// Errors of the monster palette shift and blood map rules.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShiftError {
    /// `s − 8` > 29 with RandTransforms loaded: fatal error 0x160 (a 0xAC
    /// monster's map keeps `s` ≤ 29).
    #[error("RandTransforms map {0} > 29: fatal 0x160 (render/shading.md §6 r6)")]
    RandTransformsFatal(u32),
    /// A map read past the end of the caller's data (`s` ≥ 8 without RandTransforms reads
    /// past the 8 loaded `palshift` maps).
    #[error("{what} map at byte {offset} is past the {len} bytes given")]
    PastData {
        what: &'static str,
        offset: usize,
        len: usize,
    },
}

/// What the shift index of a monster created by S→C 0xAC depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShiftInputs {
    /// Monster class (`monstats` row).
    pub class: u32,
    /// The name seed: the low byte of the message's 16-bit name field.
    pub name_seed: u16,
    /// `monstats` `TransLvl` (`+0x4D`).
    pub trans_lvl: u8,
    /// Monster type flag 0x8: unique.
    pub unique: bool,
    /// `monstats2` `noUniqueShift` (flag bit 15).
    pub no_unique_shift: bool,
    /// `monstats2` `Utrans` for the current difficulty (`+0x122 + difficulty`).
    pub utrans: u8,
    /// The owner relation test of `Utrans` 255. Which relation it is is still open, so the
    /// caller supplies the result.
    pub owner_relation: bool,
    /// Superunique (type flag 0x2): the `superuniques` row's `Utrans` for the difficulty
    /// (`+0x28 + difficulty`); `None` when not superunique.
    pub superunique_utrans: Option<u8>,
}

/// Shift step 1: `TransLvl + 2` when `TransLvl` < 8, else 2.
pub fn shift_trans_lvl(trans_lvl: u8) -> u32 {
    if trans_lvl < 8 {
        u32::from(trans_lvl) + 2
    } else {
        SHIFT_DEFAULT
    }
}

/// Shift step 2, for uniques: `9 + (n mod 30)`, `n` the new low word of one RNG step of the
/// seed {low = class + name seed, high = 666}. The name seed makes each unique's colors
/// stable for its name.
pub fn shift_unique(class: u32, name_seed: u16) -> u32 {
    let mut seed = d2_sim::rng::Seed::init_low(class.wrapping_add(u32::from(name_seed)));
    9 + seed.step() % 30
}

/// Shift step 3: `Utrans` ≠ 0 replaces `s`; 255 gives 1 when the owner relation test is
/// false, else 0. `None` = no change.
pub fn shift_utrans(utrans: u8, owner_relation: bool) -> Option<u32> {
    match utrans {
        0 => None,
        255 => Some(if owner_relation { 0 } else { 1 }),
        u => Some(u32::from(u)),
    }
}

/// The shift index `s` of a monster created by S→C 0xAC. Five steps, each overriding the
/// previous one: `TransLvl`, the unique shift, `Utrans`, the superunique `Utrans`, and
/// finally any `s` ≥ 30 falls back to the default 2.
pub fn monster_shift_index(m: &ShiftInputs) -> u32 {
    let mut s = shift_trans_lvl(m.trans_lvl);
    if m.unique && !m.no_unique_shift {
        s = shift_unique(m.class, m.name_seed);
    }
    if let Some(u) = shift_utrans(m.utrans, m.owner_relation) {
        s = u;
    }
    if let Some(u) = m.superunique_utrans.filter(|&u| u != 0) {
        s = u32::from(u);
    }
    if s >= 30 {
        s = SHIFT_DEFAULT;
    }
    s
}

/// The unit and class state the shift map choice reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ShiftMapInputs {
    /// Unit type (1 = monster).
    pub unit_type: u32,
    /// Monster class.
    pub class: u32,
    /// The class has `palshift` data loaded.
    pub has_palshift: bool,
    /// The green-blood switch ([`green_blood_switch`]).
    pub green_blood: bool,
    /// `monstats2` `localBlood` (`+0x11C`).
    pub local_blood: u32,
    /// `RandTransforms.dat` is loaded.
    pub rand_transforms_loaded: bool,
}

/// The `P` a monster's shift index selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShiftMap {
    /// No `P`.
    None,
    /// A `palshift` map: `offset` = bytes from the class's base (`base + 4 + 256·s` or
    /// `base + 0x804 + 256·s`).
    Palshift { offset: usize },
    /// RandTransforms map `m` (0…29).
    RandTransforms(u32),
}

/// The map choice for shift index `s`. Only monsters with `palshift` data shift. Shift 0 / 1
/// is unshifted except for [`SHIFT_LOW_CLASSES`]; `s` ≥ 8 with RandTransforms loaded picks
/// RandTransforms map `s − 8`; everything else picks `palshift` map `s`, from the green-blood
/// set when the switch is on and the class bleeds green (`localBlood` 2).
pub fn shift_map(s: u32, m: &ShiftMapInputs) -> Result<ShiftMap, ShiftError> {
    if m.unit_type != 1 || !m.has_palshift {
        return Ok(ShiftMap::None);
    }
    let palshift = |set: usize| ShiftMap::Palshift {
        offset: set + MAP_BYTES * s as usize,
    };
    match s {
        0 | 1 if SHIFT_LOW_CLASSES.contains(&m.class) => Ok(palshift(PALSHIFT_FIRST)),
        0 | 1 => Ok(ShiftMap::None),
        s if s >= 8 && m.rand_transforms_loaded => {
            let k = s - 8;
            if k >= RAND_TRANSFORMS_MAPS {
                Err(ShiftError::RandTransformsFatal(k))
            } else {
                Ok(ShiftMap::RandTransforms(k))
            }
        }
        // 2…7, and ≥ 8 without RandTransforms (reads past the 8 maps).
        _ if m.green_blood && m.local_blood == 2 => Ok(palshift(PALSHIFT_SECOND)),
        _ => Ok(palshift(PALSHIFT_FIRST)),
    }
}

fn map_at<'a>(
    data: &'a [u8],
    offset: usize,
    what: &'static str,
) -> Result<&'a [u8; MAP_BYTES], ShiftError> {
    data.get(offset..offset + MAP_BYTES)
        .map(|m| m.try_into().expect("256 bytes"))
        .ok_or(ShiftError::PastData {
            what,
            offset,
            len: data.len(),
        })
}

/// The 256 bytes of a [`ShiftMap::Palshift`] offset in the class's `palshift` block `base`
/// (the caller's bytes from the class's base; what `base + 0…3` holds is not defined, so the
/// caller supplies the block, not the bare file).
pub fn palshift_map(base: &[u8], offset: usize) -> Result<&[u8; MAP_BYTES], ShiftError> {
    map_at(base, offset, "palshift")
}

/// RandTransforms map `m` from the bytes of `Data\Global\Monsters\RandTransforms.dat` (30
/// maps of 256 bytes).
pub fn rand_transforms_map(file: &[u8], m: u32) -> Result<&[u8; MAP_BYTES], ShiftError> {
    if m >= RAND_TRANSFORMS_MAPS {
        return Err(ShiftError::RandTransformsFatal(m));
    }
    map_at(file, MAP_BYTES * m as usize, "RandTransforms")
}

/// The blood map: the `GreenBlood.dat` map (its first 256 bytes) for an S8 component or a
/// missile with `LocalBlood` (`uses_blood`) when the green-blood switch is on; else none.
pub fn blood_map(
    green_blood_file: &[u8],
    green_blood: bool,
    uses_blood: bool,
) -> Result<Option<&[u8; MAP_BYTES]>, ShiftError> {
    if !(green_blood && uses_blood) {
        return Ok(None);
    }
    map_at(green_blood_file, 0, "GreenBlood").map(Some)
}

/// The green-blood switch (a low-violence option): on when `Data\Local\Color.txt` opens
/// (`color_txt` = its bytes) and its first byte is `1` (0x31).
pub fn green_blood_switch(color_txt: Option<&[u8]>) -> bool {
    color_txt.and_then(|b| b.first()) == Some(&b'1')
}
