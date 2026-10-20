//! The automap in the play preview's world view: the session's draw pass
//! ([`crate::ui::automap::Automap::draw_pass`]) turned into frame items.
//! The cel files (`DATA\GLOBAL\UI\AutoMap\*.dc6`) are read from the
//! user's archives on demand; the marker lines are drawn as one-pixel
//! cels of the line's palette index.
//!
//! Each cel draws in its automap mode `m` through the act's blend tables
//! (0/1/2 → the 25/50/75 % alpha tables, 5 opaque),
//! unlit; the lines are opaque. The pass is the
//! UI pass's step 3: after
//! every world pass, before every panel draw.
//!
//! D2RUST's own, unverified (the preview):
//! - without the act's shade tables (no feed loaded them) a cel draws
//!   opaque, whatever its mode;
//! - the header text is drawn by the UI text path
//!   ([`header_draws`], right-aligned at W − 16); the marker name texts
//!   are not (the markers carry no names yet);
//! - the local player is the only marker unit (the near rooms' unit
//!   lists are not turned into markers); the player's byte +0x18 is 0;
//! - a file no archive holds is logged once and its cels are not drawn.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::assets::path::{CanonicalPath, FileSource};
use crate::bridge::world::ClientWorld;
use crate::frames::{FramePart, FrameSet, FrameSetKey, IndexFrame};
use crate::rules::blend::{cel_ops, gdi_line_pixels};
use crate::rules::camera::{ClientPos, FrameSize, OpenMode};
use crate::rules::placement::draw_position;
use crate::scene::order::pass;
use crate::scene::{BlendOp, DrawItem, DrawKey, ItemTag, Rect, ShadeChain};
use crate::ui::automap::draw::{AutomapDraw, Label, TextAlign};
use crate::ui::automap::header::HeaderFacts;
use crate::ui::automap::markers::{MarkerPalette, MarkerSubject, MarkerUnit};
use crate::ui::automap::options::{cel_paths, CelFile, MemoryStore};
use crate::ui::automap::session::AutomapSession;
use crate::ui::automap::{CelLimits, FrameFacts, PassInput};
use crate::ui::draw::{CelCall, UiCelInfo};

use super::{ViewAssets, WorldFrame};

/// The light byte of an unlit cel draw.
const UNLIT: u8 = 0xFF;

/// The frame sets of the one-pixel line cels (not archive paths): a
/// line's first pixel and its other pixels, so that the facts export
/// writes one `DrawLine` row per line.
use crate::facts::export::{UI_LINE_REST, UI_LINE_START};

/// How the automap's marker names become glyphs: the UI text
/// rules and the font's width measure.
pub struct AutomapText<'a> {
    pub rules: &'a dyn super::ui_bind::UiRules,
    pub width: &'a dyn Fn(u16, &[u16]) -> Option<i32>,
    /// A string-table id's text (object 267's label).
    pub string: &'a dyn Fn(u16) -> Option<Vec<u16>>,
    /// Makes the fonts of the text draws resident.
    pub loader: Option<&'a super::ui_bind::TextAssetLoader>,
}

/// The cel files and line pixels of the automap, and what was logged.
pub struct AutomapView {
    source: Arc<dyn FileSource>,
    expansion: bool,
    /// (file, mini) → its frame set; `None`: tried and missing.
    files: BTreeMap<(CelFile, bool), Option<FrameSetKey>>,
    /// Option store of the pass (the registry seam; in memory here).
    store: MemoryStore,
    /// Draws of the last frame, for tests and logs.
    pub last: Vec<AutomapDraw>,
}

/// The cel counts the load checks stored cels against,
/// one per cel file slot (the DC6 header's frames per direction). A
/// slot with no file, or a file
/// no archive holds, counts 0.
pub fn cel_limits(source: &dyn FileSource, mini: bool, expansion: bool) -> CelLimits {
    let count = |file: CelFile| {
        cel_paths(mini, expansion)[file as usize]
            .as_deref()
            .and_then(|path| crate::assets::path::read_dc6(source, path))
            .and_then(Result::ok)
            .map_or(0, |dc6| dc6.header.frames_per_direction)
    };
    CelLimits {
        maxi: count(CelFile::MaxiMap),
        town: [
            count(CelFile::MaxiMap),
            count(CelFile::Act2Map),
            count(CelFile::Act4Map),
            count(CelFile::ExTnMap),
        ],
    }
}

impl AutomapView {
    /// The option store the key commands write (commands 9–11, 45).
    pub fn limits(&self, mini: bool) -> CelLimits {
        cel_limits(self.source.as_ref(), mini, self.expansion)
    }

    pub fn store_mut(&mut self) -> &mut MemoryStore {
        &mut self.store
    }

    pub fn new(source: Arc<dyn FileSource>, expansion: bool) -> Self {
        AutomapView {
            source,
            expansion,
            files: BTreeMap::new(),
            store: MemoryStore::default(),
            last: Vec::new(),
        }
    }

    fn file_key(
        &mut self,
        file: CelFile,
        mini: bool,
        assets: &mut ViewAssets,
        log: &mut Vec<String>,
    ) -> Option<FrameSetKey> {
        if let Some(k) = self.files.get(&(file, mini)) {
            return k.clone();
        }
        let loaded = self.load(file, mini, assets);
        if let Err(e) = &loaded {
            log.push(format!("automap (D2RUST's own, unverified): {e}"));
        }
        let key = loaded.ok();
        self.files.insert((file, mini), key.clone());
        key
    }

    fn load(
        &self,
        file: CelFile,
        mini: bool,
        assets: &mut ViewAssets,
    ) -> Result<FrameSetKey, String> {
        let path = cel_paths(mini, self.expansion)[file as usize]
            .clone()
            .ok_or_else(|| format!("{file:?} has no file in this size"))?;
        let canon = CanonicalPath::new(&path).map_err(|e| format!("{path}: {e}"))?;
        let key = FrameSetKey::new(canon.as_str(), FramePart::Dir(0)).map_err(|e| e.to_string())?;
        if assets.frames.contains(&key) {
            return Ok(key);
        }
        let dc6 = crate::assets::path::read_dc6(self.source.as_ref(), &path)
            .ok_or_else(|| format!("{path}: in no archive"))?
            .map_err(|e| format!("{path}: {e}"))?;
        let set = FrameSet::from_dc6(&dc6, 0).map_err(|e| format!("{path}: {e}"))?;
        assets
            .frames
            .insert(key.clone(), set)
            .map_err(|e| e.to_string())?;
        Ok(key)
    }

    /// A pixel set: frame `i` is one pixel of palette index `i`.
    fn pixel_key(assets: &mut ViewAssets, path: &str) -> Result<FrameSetKey, String> {
        let key = FrameSetKey::new(path, FramePart::Tile(0)).map_err(|e| e.to_string())?;
        if !assets.frames.contains(&key) {
            let frames = (0..=255u8)
                .map(|i| IndexFrame::new(1, 1, 0, 0, vec![i]).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?;
            assets
                .frames
                .insert(key.clone(), FrameSet { frames })
                .map_err(|e| e.to_string())?;
        }
        Ok(key)
    }

    /// Runs the draw pass of `session` for `world` and adds the draws to
    /// `frame` (re-sorted by key), under the frame's one camera
    /// (`frame.camera`) with the local
    /// player's marker at `player`, the frame's one player position.
    /// `units`: the automap marker units ([`marker_units`]); `None`:
    /// the local player alone (no near rooms). Closed automap, both panels
    /// covering the frame, no local player, no camera or
    /// no act: nothing. Never fails the frame; returns log lines.
    #[allow(clippy::too_many_arguments)]
    pub fn add_to_frame(
        &mut self,
        session: &mut AutomapSession,
        world: &ClientWorld,
        open_mode: OpenMode,
        player_at: ClientPos,
        units: Option<&[MarkerUnit]>,
        text: Option<&AutomapText<'_>>,
        assets: &mut ViewAssets,
        frame: &mut WorldFrame,
    ) -> Vec<String> {
        self.last.clear();
        let mut log = Vec::new();
        let Some(player) = world.local() else {
            return log;
        };
        let Some(cam) = frame.camera else {
            return log;
        };
        // The automap shows while it is open, unless both side panels cover
        // the frame (open mode 3 at 4:3; a wider frame keeps the world, and
        // the map, between them).
        if !session.open || world.act.is_none() || !FrameSize::play().draws_world(open_mode) {
            return log;
        }
        let at = player_at;
        let facts = FrameFacts {
            width: FrameSize::play().width,
            height: FrameSize::play().height,
            open_mode: open_mode.get(),
            mini_down: false,
            unit_origin: cam.unit,
        };
        // D2RUST's own, unverified: the marker colour is the act palette's
        // nearest match on the presented palette.
        let palette = MarkerPalette::new(|r, g, b| {
            crate::rules::shading::nearest(&assets.palette, r.into(), g.into(), b.into())
        });
        let local = [MarkerUnit {
            subject: MarkerSubject::Player {
                local: true,
                mode: player.mode,
                party: -1,
                own_inventory: true,
                state_7: false,
                name: Vec::new(),
            },
            pos: at,
        }];
        let markers = units.unwrap_or(&local);
        let header = HeaderFacts::default();
        let strings = |_: u16| None;
        let input = PassInput {
            frame: facts,
            open: true,
            ready: true,
            player_byte_18: 0,
            markers,
            local_party: -1,
            player_gate: false,
            palette,
            roster: &[],
            local_act: 0,
            header: &header,
            strings: &strings,
        };
        let draws = match session.map.draw_pass(&input, &mut self.store) {
            Ok(d) => d,
            Err(e) => {
                log.push(format!("automap (D2RUST's own, unverified): {e}"));
                return log;
            }
        };
        let mini = session.map.view.mini;
        let pixel = match Self::pixel_key(assets, UI_LINE_START)
            .and_then(|a| Ok((a, Self::pixel_key(assets, UI_LINE_REST)?)))
        {
            Ok(k) => k,
            Err(e) => {
                log.push(format!("automap (D2RUST's own, unverified): {e}"));
                return log;
            }
        };
        let mut minor = 0u32;
        let mut push = |frame: &mut WorldFrame, mut d: DrawItem, tag: ItemTag| {
            if let Ok(k) = DrawKey::new(pass::UI, pass::UI_AUTOMAP_MAJOR, minor, 0) {
                d.key = k;
                d.tag = tag;
                minor += 1;
                frame.items.push(d);
            }
        };
        // A UI call record: its tag.
        let ui_tag = |frame: &mut WorldFrame, c: UiCelInfo| {
            frame.ui_calls.push(c);
            ItemTag::Ui((frame.ui_calls.len() - 1) as u32)
        };
        for d in &draws {
            match d {
                AutomapDraw::Cel {
                    file,
                    cel,
                    x,
                    y,
                    clip,
                    mode,
                } => {
                    let Some(key) = self.file_key(*file, mini, assets, &mut log) else {
                        continue;
                    };
                    let Ok(index) = usize::try_from(*cel) else {
                        continue;
                    };
                    let (Ok(id), Ok(image)) = (assets.id(&key, index), assets.frame(&key, index))
                    else {
                        continue;
                    };
                    let (px, py) = draw_position(image, *x, *y);
                    let mut item = DrawItem::new(id, px, py);
                    item.clip = rect(clip);
                    (item.shade, item.blend) = cel_blend(assets, *mode);
                    let call = UiCelInfo {
                        call: CelCall::Clipped,
                        mode: *mode,
                        pal: None,
                        text: false,
                    };
                    // A cell is a `CelDrawClipped` call.
                    let tag = ui_tag(frame, call);
                    push(frame, item, tag);
                }
                AutomapDraw::Line { from, to, color } => {
                    let (Ok(first), Ok(rest)) = (
                        assets.id(&pixel.0, usize::from(*color)),
                        assets.id(&pixel.1, usize::from(*color)),
                    ) else {
                        continue;
                    };
                    // The GDI line's pixels.
                    let Ok(pixels) = gdi_line_pixels(from.0, from.1, to.0, to.1) else {
                        continue;
                    };
                    for (i, (x, y)) in pixels.into_iter().enumerate() {
                        let id = if i == 0 { first } else { rest };
                        let mut item = DrawItem::new(id, x, y);
                        item.blend = BlendOp::Opaque;
                        push(frame, item, ItemTag::None);
                    }
                }
                // A marker's name, a `CelDrawColor` glyph run
                // right after its cross.
                AutomapDraw::Text {
                    text: label,
                    font,
                    color,
                    x,
                    y,
                    align,
                } => {
                    // The header (right-aligned) is [`header_draws`]'
                    // UI text with the level's facts; the names are centred.
                    let (Some(text), TextAlign::Centre) = (text, align) else {
                        continue;
                    };
                    let t = match label {
                        Label::Text(t) => t.clone(),
                        Label::String(id) => match (text.string)(*id) {
                            Some(t) => t,
                            None => continue,
                        },
                    };
                    let Some(w) = (text.width)(*font, &t) else {
                        continue;
                    };
                    let crate::ui::UiDraw::Text(req) =
                        crate::ui::panels::text(t, x - (w >> 1), *y, *font, *color)
                    else {
                        continue;
                    };
                    if let Some(l) = text.loader {
                        let d = [crate::ui::UiDraw::Text(req.clone())];
                        if let Err(e) = l.ensure(&d, assets) {
                            log.push(format!("automap (D2RUST's own, unverified): {e}"));
                            continue;
                        }
                    }
                    let sprites = match text.rules.ui_text(&req, assets) {
                        Ok(s) => s,
                        Err(e) => {
                            log.push(format!("automap (D2RUST's own, unverified): {e}"));
                            continue;
                        }
                    };
                    let call = UiCelInfo {
                        call: CelCall::Color,
                        mode: req.opts.mode(),
                        pal: None,
                        text: true,
                    };
                    let tag = ui_tag(frame, call);
                    for sp in sprites {
                        let Ok(id) = assets.id(&sp.frame.set, sp.frame.index) else {
                            continue;
                        };
                        let mut item = DrawItem::new(id, sp.x, sp.y);
                        item.clip = super::ui_bind::clip_rect(req.clip);
                        item.shade = sp.shade;
                        item.blend = sp.blend;
                        push(frame, item, tag);
                    }
                }
            }
        }
        crate::scene::order(&mut frame.items);
        self.last = draws;
        log
    }
}

/// The header lines of the open automap as UI text draws: font 1,
/// colour 4, each right-aligned at `x = W − 16 − width A`, line y from 24
/// stepping 16, W the frame's width. Empty unless the automap is shown
/// (`session.open`, an act, a frame that draws the world in this open mode,
/// as [`AutomapView::add_to_frame`]) and the font measures.
/// `strings` looks a string id up; `width` measures a line in a font.
pub fn header_draws(
    session: &AutomapSession,
    world: &ClientWorld,
    open_mode: OpenMode,
    facts: &HeaderFacts,
    strings: &dyn Fn(u16) -> Option<Vec<u16>>,
    width: &dyn Fn(u16, &[u16]) -> Option<i32>,
) -> Vec<crate::ui::UiDraw> {
    use crate::ui::automap::draw::{Label, TextAlign};
    let shown = FrameSize::play().draws_world(open_mode);
    if !session.open || world.act.is_none() || !shown || world.local().is_none() {
        return Vec::new();
    }
    let frame = FrameSize::play();
    let mut lines = Vec::new();
    if crate::ui::automap::header::header(facts, frame.width, strings, &mut lines).is_err() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for d in lines {
        let AutomapDraw::Text {
            text: Label::Text(t),
            font,
            color,
            x,
            y,
            align,
        } = d
        else {
            continue;
        };
        let Some(w) = width(font, &t) else {
            continue;
        };
        let at_x = match align {
            TextAlign::Right => x - w,
            TextAlign::Centre => x - (w >> 1),
        };
        // Frame space, right-aligned to the frame's own edge: clipped to the
        // whole frame, not the 800-wide UI box.
        let mut d = crate::ui::panels::text(t, at_x, y, font, color);
        if let crate::ui::UiDraw::Text(r) = &mut d {
            r.clip = crate::ui::geom::screen_clip((frame.width, frame.height));
        }
        out.push(d);
    }
    out
}

/// The shade chain and blend op of an automap cel in draw mode `mode`,
/// unlit (light byte 0xFF). Without the
/// act's shade tables: opaque (module doc).
fn cel_blend(assets: &ViewAssets, mode: u8) -> (ShadeChain, BlendOp) {
    match &assets.shades {
        Some(t) => cel_ops(t, mode, None, UNLIT),
        None => (ShadeChain::EMPTY, BlendOp::Opaque),
    }
}

/// The pixel area of a `CelDrawClipped` clip (GDI slot
/// `+0x98`): columns left ≤ x < right (right exclusive), rows top ≤ y ≤
/// bottom (bottom inclusive).
fn rect(b: &crate::ui::automap::view::Bounds) -> Rect {
    let w = (b.right - b.left).max(0) as u32;
    let h = (b.bottom - b.top + 1).max(0) as u32;
    Rect::new(b.left, b.top, w, h)
}

#[cfg(test)]
#[path = "automap_view_tests.rs"]
mod tests;

/// The marker units of the local player's near rooms,
/// each room's unit list in order. A monster reads its class's
/// `monstats` `interact` and name (`labels`, `strings`); the local
/// player is at `player_at` (the frame's one position), every other unit
/// at `pos`. D2RUST's own, unverified: other players' party ids, disguises,
/// relations (single player: none) and portal targets are not modelled.
pub fn marker_units(
    world: &ClientWorld,
    near: &crate::rules::draw_order::NearRooms,
    labels: &super::monster_label::MonsterLabels,
    strings: Option<&dyn crate::ui::panel::StringLookup>,
    player_at: ClientPos,
    pos: &dyn Fn(&crate::bridge::world::ClientUnit) -> Option<ClientPos>,
) -> Vec<MarkerUnit> {
    use crate::bridge::world::{MONSTER, OBJECT, PLAYER};
    let mut out = Vec::new();
    for room in &near.rooms {
        for ru in &room.units {
            let Some(u) = world.units.get(&ru.key) else {
                continue;
            };
            let local = world.local_player == Some(ru.key);
            let subject = match ru.key.unit_type {
                PLAYER => MarkerSubject::Player {
                    local,
                    mode: u.mode,
                    party: -1,
                    own_inventory: true,
                    state_7: false,
                    name: Vec::new(),
                },
                MONSTER => {
                    let row = labels
                        .rows
                        .get(u.class as usize)
                        .copied()
                        .unwrap_or_default();
                    let name = match (row.interact, strings) {
                        (true, Some(s)) => s
                            .get_id(row.name_str)
                            .map(<[u16]>::to_vec)
                            .unwrap_or_default(),
                        _ => Vec::new(),
                    };
                    MarkerSubject::Monster {
                        class: u.class,
                        mode: u.mode,
                        flags: ru.facts.flags,
                        interact: row.interact,
                        disguised: None,
                        relation: 0,
                        name,
                        owner_name: Vec::new(),
                    }
                }
                OBJECT => MarkerSubject::Object {
                    class: u.class,
                    target_level: None,
                },
                _ => MarkerSubject::Other,
            };
            let at = if local { Some(player_at) } else { pos(u) };
            let Some(at) = at else {
                continue;
            };
            out.push(MarkerUnit { subject, pos: at });
        }
    }
    out
}
