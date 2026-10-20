//! The `play` preview's hover target: which unit a world click is on.
//!
//! D2RUST's own, unverified. The hover model is not settled (the base
//! game hit-tests the drawn sprites),
//! so the preview picks the unit whose feet are nearest the mouse,
//! inside a box standing on the feet ([`HIT_HALF_WIDTH`],
//! [`HIT_ABOVE`], [`HIT_BELOW`]). Only monsters, objects and items are
//! picked (never a player: town players need the rest of the hover
//! model); a dead monster that no skill of the local player selects
//! ([`corpse_selectable`]) and an object whose mode is not `Selectable` (the
//! "Dummy" under a town NPC) and an item not on the floor
//! ([`ITEM_ON_FLOOR`]) are skipped. Ties go to the lower unit key.
//! The strict path never calls this ([`super::click::ClickView::pick`]
//! is `false`).

use crate::rules::camera::{moving_to_client, static_to_client, Camera};

use std::collections::BTreeMap;

use super::world::{ClientWorld, UnitKey, TILE};

/// The only item mode the hover test passes (item
/// in mode 3, on the floor): a dropping item (mode 5, still in its drop
/// animation) is not a target, so a click on it is a click on the ground.
/// In the base game a click 6 frames after the drop sends the walk C→S
/// 0x01 to the item's cell, not the pick-up.
pub const ITEM_ON_FLOOR: u32 = 3;

/// An item passes the hover gates when it is on the floor (mode 3)
/// and its selectable flag (+0xC4 bit 0x2) is set.
/// The item update sets that flag in its mode-3 branch only:
/// the update in which a dropping item lands leaves it clear, so the item
/// is a target from the next update on. In the base game a click on an
/// item 17 frames after its
/// drop (the frame it lands) sends the walk C→S 0x01, not 0x02.
pub fn item_selectable(u: &super::world::ClientUnit) -> bool {
    u.mode == ITEM_ON_FLOOR && u.flag_2 == Some(true)
}

/// A monster passes the hover test's selectable gate when its flag
/// +0xC4 bit 0x2 is set: `monstats2` `isSel` at set-up
/// (step 6.6), cleared while it is
/// quest-untargetable. A monster without set-up (`None`) passes.
/// Measured in the Spider Forest: the Act3Female (class
/// 296, `isSel` 0) under the cursor is not hovered: the base game walks to the
/// ground (C→S 0x01, target prepared).
pub fn monster_selectable(u: &super::world::ClientUnit) -> bool {
    u.flag_2 != Some(false) && !u.quest_untargetable
}

/// The monster mode `WL` (2).
const MODE_WALK: u16 = 2;

/// What the corpse test reads beside the model: each skill's `SelectProc`,
/// each monster class's `monstats2` / `monstats` columns and the `udead`
/// state group. Filled by the bridge from its tables; empty: no corpse
/// is a target.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorpseTables {
    /// `skills` `SelectProc` (+0x15) by skill id.
    pub selectproc: Vec<u8>,
    /// By monster class: `None` = no `monstats2` row.
    pub monsters: Vec<Option<CorpseClass>>,
    /// `states` `udead` by state id.
    pub udead: Vec<bool>,
}

/// The columns of one monster class the select procs read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CorpseClass {
    /// `monstats2` `corpseSel` (+0x04 bit 7).
    pub corpse_sel: bool,
    /// `monstats2` `soft` (+0x05 bit 4).
    pub soft: bool,
    /// `monstats` `Velocity` (+0x32) ≠ 0; `None` = no set-up columns.
    pub moves: Option<bool>,
    /// The switch test: the class has the `WL` mode
    /// (mode 2), is not `boss` and has `switchai`.
    pub can_switch: bool,
    /// `monstats` `BaseId`.
    pub base_id: u16,
}

impl CorpseTables {
    pub fn of(t: &super::world::ClientTables) -> Self {
        CorpseTables {
            selectproc: t.skills.iter().map(|r| r.selectproc).collect(),
            monsters: t
                .monsters
                .iter()
                .map(|m| {
                    m.map(|m| CorpseClass {
                        corpse_sel: m.corpse_sel,
                        soft: m.soft,
                        moves: m.setup.map(|s| s.velocity != 0),
                        can_switch: m.modes & (1 << MODE_WALK) != 0 && !m.boss && m.switchai,
                        base_id: m.base_id,
                    })
                })
                .collect(),
            udead: t.states.iter().map(|r| r.udead).collect(),
        }
    }
}

/// The corpse select proc `SelectProc` of `skill` (the
/// table, procs 1..=6) on the corpse `u`. Every proc needs
/// the core test: `u` in mode 12 (dead), in no state of
/// the `udead` group and its class `corpseSel`. Procs 4
/// and 6 add `soft`; proc 2 adds a
/// non-zero `Velocity`. Proc 3 (Revive)
/// is proc 2, then: the class passes the switch test
/// and `u` is neither unique (type flag 0x8) nor
/// superunique (0x2); last its `BaseId` is not 78
/// (`sandleaper1`).
fn select_proc(t: &CorpseTables, skill: u16, u: &super::world::ClientUnit) -> bool {
    const MODE_DEAD: u32 = 12;
    let proc_ = t.selectproc.get(skill as usize).copied().unwrap_or(0);
    if !(1..=6).contains(&proc_) || u.mode != MODE_DEAD {
        return false;
    }
    let Some(class) = t.monsters.get(u.class as usize).copied().flatten() else {
        return false;
    };
    let udead = u
        .states
        .iter()
        .any(|&s| t.udead.get(s as usize).copied().unwrap_or(false));
    if udead || !class.corpse_sel {
        return false;
    }
    match proc_ {
        1 | 5 => true,
        2 => class.moves == Some(true),
        3 => {
            const UNIQUE: u8 = 0x8;
            const SUPERUNIQUE: u8 = 0x2;
            const BASE_SANDLEAPER: u16 = 78;
            let flags = match &u.kind {
                super::world::KindData::Monster(m) => m.flags,
                _ => 0,
            };
            class.moves == Some(true)
                && class.can_switch
                && flags & (UNIQUE | SUPERUNIQUE) == 0
                && class.base_id != BASE_SANDLEAPER
        }
        4 | 6 => class.soft,
        _ => false,
    }
}

/// The corpse test with (T): T a
/// monster in mode 12, in no state of the `udead` group,
/// its class `corpseSel` and `soft`.
pub fn soft_corpse(t: &CorpseTables, u: &super::world::ClientUnit) -> bool {
    if u.key.unit_type != super::world::MONSTER || u.mode != 12 {
        return false;
    }
    let Some(class) = t.monsters.get(u.class as usize).copied().flatten() else {
        return false;
    };
    let udead = u
        .states
        .iter()
        .any(|&s| t.udead.get(s as usize).copied().unwrap_or(false));
    !udead && class.corpse_sel && class.soft
}

/// A dead monster passes the hover test
/// when its flag +0xC4 bit 0x4 is set (`monstats2` `isAtt`)
/// and the local player's left skill, else its right skill, selects it
/// ([`select_proc`]): Find Potion (proc 4) and Find Item
/// (proc 5) on a Fallen's corpse. Measured on both skills (the base game
/// casts C→S 0x0D on the corpse).
pub fn corpse_selectable(world: &ClientWorld, u: &super::world::ClientUnit) -> bool {
    if !u.flag_4 {
        return false;
    }
    let Some(list) = world
        .local_player
        .and_then(|k| world.units.get(&k))
        .and_then(|p| p.skills.as_ref())
    else {
        return false;
    };
    [list.left_entry(), list.right_entry()]
        .into_iter()
        .flatten()
        .any(|e| select_proc(&world.corpse_tables, e.skill, u))
}

/// The monster gate of the hover test: a live monster by
/// [`monster_selectable`], a dead one by [`corpse_selectable`].
pub fn monster_hoverable(world: &ClientWorld, u: &super::world::ClientUnit) -> bool {
    if u.is_dead() {
        corpse_selectable(world, u)
    } else {
        monster_selectable(u)
    }
}

/// Half the box width, in screen pixels. D2RUST's own, unverified.
pub const HIT_HALF_WIDTH: i32 = 24;
/// Half the box width of an object: its sprite (a waypoint stone, a chest)
/// is wider than a unit's. The fallback of the feet box only (no drawn
/// rectangles): the base game hit-tests the drawn frame rectangles,
/// which the rectangle pick reproduces and which is settled (the hover
/// pick matches the packets 310/310); the Act I waypoint click path of the scene
/// scripts hovers the waypoint 28 px from its feet. D2RUST's own.
pub const HIT_HALF_WIDTH_OBJECT: i32 = 48;
/// How far the box reaches above the feet. D2RUST's own, unverified.
pub const HIT_ABOVE: i32 = 96;
/// How far the box reaches below the feet. D2RUST's own, unverified.
pub const HIT_BELOW: i32 = 16;

/// The screen point of a unit's feet: its cell centre drawn by the unit
/// rule with no extra offset.
pub fn feet(cam: &Camera, cell: (u16, u16)) -> (i32, i32) {
    let (x, y) = (u32::from(cell.0), u32::from(cell.1));
    cam.unit_draw(
        moving_to_client((x << 16) | 0x8000, (y << 16) | 0x8000),
        (0, 0),
    )
}

/// The screen point of a unit's feet by its type: static units (objects
/// 2, items 4, tiles 5) use the static rule `(sx − sy) × 16, (sx + sy) × 8`
/// they are drawn with, moving units (players 0, monsters 1, missiles 3)
/// the cell centre of [`feet`].
pub fn unit_feet(cam: &Camera, unit_type: u8, cell: (u16, u16)) -> (i32, i32) {
    if matches!(unit_type, 2 | 4 | 5) {
        cam.unit_draw(
            static_to_client(i32::from(cell.0), i32::from(cell.1)),
            (0, 0),
        )
    } else {
        feet(cam, cell)
    }
}

/// The screen rectangle `(left, top, right, bottom)`, both ends
/// inclusive, a warp tile (unit type 5) is hovered in: its client pixel
/// point `((sx − sy) × 16, (sx + sy) × 8)` plus the
/// `SelectX`, `SelectY`, `SelectDX`, `SelectDY` of its lvlwarp record.
/// `None`: the unit has no
/// position or no lvlwarp record with its class.
pub fn tile_rect(
    world: &ClientWorld,
    cam: &Camera,
    u: &super::world::ClientUnit,
) -> Option<(i32, i32, i32, i32)> {
    let (sx, sy) = u.position?;
    let (x, y, w, h) = world.drlg.as_ref()?.warp_select(u.class)?;
    let (fx, fy) = unit_feet(cam, 5, (sx, sy));
    Some((fx + x, fy + y, fx + x + w, fy + y + h))
}

/// The warp tiles' hover rectangles of one frame, by unit
/// ([`tile_rect`]); they join the drawn-unit rectangles of
/// [`unit_rects`] for [`pick_rects`], which tests tiles inclusive and
/// unpadded.
pub fn tile_rects(world: &ClientWorld, cam: &Camera) -> BTreeMap<UnitKey, (i32, i32, i32, i32)> {
    world
        .units
        .iter()
        .filter(|(k, _)| k.unit_type == TILE)
        .filter_map(|(k, u)| Some((*k, tile_rect(world, cam, u)?)))
        .collect()
}

/// The hover target at screen point `mouse` (module doc).
pub fn pick(world: &ClientWorld, cam: &Camera, mouse: (i32, i32)) -> Option<UnitKey> {
    let mut best: Option<(i32, UnitKey)> = None;
    // Type priority first, then nearness.
    let rank = |k: &UnitKey, d: i32| (type_priority(k.unit_type), -d);
    for (key, u) in &world.units {
        if !matches!(key.unit_type, 1 | 2 | 4 | 5) || Some(*key) == world.local_player {
            continue;
        }
        if key.unit_type == TILE {
            let Some((l, t, r, b)) = tile_rect(world, cam, u) else {
                continue;
            };
            if (l..=r).contains(&mouse.0) && (t..=b).contains(&mouse.1) {
                let (fx, fy) = unit_feet(cam, key.unit_type, u.position.unwrap_or_default());
                let d = (mouse.0 - fx).abs() + (mouse.1 - fy).abs();
                if best.is_none_or(|(bd, bk)| rank(key, d) > rank(&bk, bd)) {
                    best = Some((d, *key));
                }
            }
            continue;
        }
        if key.unit_type == 1 && !monster_hoverable(world, u) {
            continue;
        }
        if key.unit_type == 4 && !item_selectable(u) {
            continue;
        }
        let Some(cell) = u.position else { continue };
        if key.unit_type == 2
            && world
                .objclient
                .selectable
                .get(u.class as usize)
                .is_some_and(|s| s[(u.mode & 7) as usize] == 0)
        {
            continue;
        }
        let (fx, fy) = unit_feet(cam, key.unit_type, cell);
        let (dx, dy) = (mouse.0 - fx, mouse.1 - fy);
        let half = if key.unit_type == 2 {
            HIT_HALF_WIDTH_OBJECT
        } else {
            HIT_HALF_WIDTH
        };
        if dx.abs() > half || !(-HIT_ABOVE..=HIT_BELOW).contains(&dy) {
            continue;
        }
        let d = dx.abs() + dy.abs();
        if best.is_none_or(|(bd, bk)| rank(key, d) > rank(&bk, bd)) {
            best = Some((d, *key));
        }
    }
    best.map(|(_, k)| k)
}

/// How far the base game's hover test widens a unit's drawn frame rectangle on each
/// side (the mouse must lie inside `left − 16.. right + 16`
/// and `top − 16 .. bottom + 16`).
pub const FRAME_RECT_PAD: i32 = 16;

/// How far the base game's layer box widens a cel on each side.
pub const ITEM_BOX_PAD: i32 = 12;

/// Screen rectangles `(left, top, right, bottom)` by unit, as
/// [`pick_rects`] reads them.
pub type UnitRects = BTreeMap<UnitKey, (i32, i32, i32, i32)>;

/// The box of a drawn cel `(left, top, right, bottom)`:
/// the cel widened by [`ITEM_BOX_PAD`] on each side,
/// then each axis shorter than 32 grown by half the shortfall (rounded down)
/// on both sides. The hover test of an item (there is no frame
/// rectangle for items) holds it inclusive. The drawn cel covers rows
/// `Y + yoff − h + 1 ..= Y + yoff` (`bottom` exclusive at `Y + yoff + 1`);
/// The base game's box takes `bottom = Y + yoff + 12`, `top = bottom − h − 24`,
/// one row above the drawn rows (measured in the Cold Plains).
pub fn item_box((l, t, r, b): (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let (mut l, mut t, mut r, mut b) = (
        l - ITEM_BOX_PAD,
        t - 1 - ITEM_BOX_PAD,
        r + ITEM_BOX_PAD,
        b - 1 + ITEM_BOX_PAD,
    );
    if r - l < 32 {
        let d = (32 - (r - l)) >> 1;
        r += d;
        l -= d;
    }
    if b - t < 32 {
        let d = (32 - (b - t)) >> 1;
        t -= d;
        b += d;
    }
    (l, t, r, b)
}

/// The screen rectangle `(left, top, right, bottom)` of each unit's drawn
/// cels (shadows, overlays and missiles left out) of one built frame, by
/// unit: an item belongs to the unit whose draw-order slot (`slots`) it
/// carries.
pub fn unit_rects(
    items: &[crate::scene::DrawItem],
    frames: &crate::frames::FrameStore,
    slots: &BTreeMap<UnitKey, crate::rules::draw_order::UnitSlot>,
) -> BTreeMap<UnitKey, (i32, i32, i32, i32)> {
    let mut by_slot: BTreeMap<u64, UnitKey> = BTreeMap::new();
    for (k, s) in slots {
        if let crate::rules::draw_order::UnitSlot::Drawn(o) = s {
            if let Ok(dk) = crate::scene::DrawKey::new(o.pass, o.major, o.minor, 0) {
                by_slot.insert(dk.slot(), *k);
            }
        }
    }
    let mut out: BTreeMap<UnitKey, (i32, i32, i32, i32)> = BTreeMap::new();
    for it in items {
        if !matches!(it.tag, crate::scene::ItemTag::Unit(_))
            || it.key.pass() == crate::scene::order::pass::SHADOWS
        {
            continue;
        }
        let (Some(key), Some(f)) = (by_slot.get(&it.key.slot()), frames.frame(it.frame)) else {
            continue;
        };
        // The overlays and missiles filed in a unit's slot are not its
        // graphics.
        if frames
            .owner(it.frame)
            .is_some_and(|(k, _)| is_effect_art(k.path()))
        {
            continue;
        }
        let (l, t) = (it.x, it.y);
        let (r, b) = (l + f.width as i32, t + f.height as i32);
        out.entry(*key)
            .and_modify(|e| *e = (e.0.min(l), e.1.min(t), e.2.max(r), e.3.max(b)))
            .or_insert((l, t, r, b));
    }
    out
}

/// One drawn cel of a unit for the layer test:
/// its COF layer index, the layer's component (HD 0, TR 1, … S8 15;
/// layer record byte 0), the layer's `Selectable` byte (layer record
/// byte 2, read) and the drawn cel `(left, top, right,
/// bottom)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerRect {
    pub layer: usize,
    pub component: u8,
    pub selectable: bool,
    pub rect: (i32, i32, i32, i32),
}

/// The drawn layers of each unit, as [`pick_layers`] reads them.
pub type UnitLayers = BTreeMap<UnitKey, Vec<LayerRect>>;

/// The `objects.txt` fields that choose an object's layer test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectLayerTest {
    /// `IsDoor` (+0x13A).
    pub is_door: bool,
    /// `CollisionSubst` (+0x19D).
    pub collision_subst: bool,
    /// `TotalPieces` (+0x161).
    pub total_pieces: u8,
}

impl ObjectLayerTest {
    pub fn of(row: &super::objects::ObjClientRow) -> Self {
        Self {
            is_door: row.is_door != 0,
            collision_subst: row.collision_subst != 0,
            total_pieces: row.total_pieces,
        }
    }
}

/// Which test follows the frame rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LayerTest {
    /// No further test: the frame rectangle decides.
    None,
    /// The box of component 1 (TR) alone (a single-piece unit,
    /// with EAX = 1).
    Tr,
    /// Any selectable layer's box (a multi-piece unit).
    Selectable,
}

/// The layer test applies after the frame rectangle: a
/// player (but a class 4 in mode 18, which has none), a monster whose
/// `monstats2` `TotalPieces` ≥ 2 and an object whose `TotalPieces` ≥ 2
/// take the selectable-layer test; other monsters and objects the box of
/// component 1 (TR), selectable or not. A door in mode 2, 4 or 5 and an object with `CollisionSubst`
/// take the select box instead, not modelled: the frame
/// rectangle decides (PROVISIONAL; settled by the `objects.txt`
/// `Left` / `Top` / `Width` / `Height` columns: the box at the unit's
/// screen point, edges inclusive).
/// Without the class tables
/// (empty) a unit takes no layer test.
fn layer_test(world: &ClientWorld, key: UnitKey, u: &super::world::ClientUnit) -> LayerTest {
    let pieces = |n: Option<u8>| match n {
        None => LayerTest::None,
        Some(n) if n >= 2 => LayerTest::Selectable,
        Some(_) => LayerTest::Tr,
    };
    match key.unit_type {
        0 if u.class == 4 && u.mode == 18 => LayerTest::None,
        0 => LayerTest::Selectable,
        1 => pieces(
            world
                .objclient
                .monster_pieces
                .get(u.class as usize)
                .copied(),
        ),
        2 => match world.objclient.hover_layers.get(u.class as usize) {
            None => LayerTest::None,
            Some(o) if o.is_door && matches!(u.mode, 2 | 4 | 5) => LayerTest::None,
            Some(o) if !o.is_door && o.collision_subst => LayerTest::None,
            Some(o) => pieces(Some(o.total_pieces)),
        },
        _ => LayerTest::None,
    }
}

/// Whether `path` is overlay or missile art, drawn in a host unit's slot
/// but no part of the unit's frame rectangle.
fn is_effect_art(path: &str) -> bool {
    path.starts_with("data/global/overlays/") || path.starts_with("data/global/missiles/")
}

/// The hover priority of a unit type (table:
/// player 0, monster 0, object −6, missile 0, item −4,
/// tile 0).
pub fn type_priority(unit_type: u8) -> i32 {
    match unit_type {
        2 => -6,
        4 => -4,
        _ => 0,
    }
}

/// The hover target at `mouse` by the drawn frame rectangles of the last
/// pass ([`unit_rects`]) widened by [`FRAME_RECT_PAD`], on the same
/// units as [`pick`]. `prev` is the last target: it
/// stays while it is still hit. Else the candidates are ranked:
/// the higher type priority ([`type_priority`]) wins; equal
/// priority → the one whose client point `point` is strictly nearer to
/// the local player's (`player`; squared distance).
/// Candidates are taken in unit key order. An item is hit by its component-1
/// box ([`item_box`]). No layer test: [`pick_layers`] with no
/// layers. A monster must pass [`monster_selectable`].
/// PROVISIONAL: the gate, the rest of the layer test and the object and
/// state rules are not applied; settled by porting the selectable,
/// pet and owner tests.
pub fn pick_rects(
    world: &ClientWorld,
    rects: &BTreeMap<UnitKey, (i32, i32, i32, i32)>,
    mouse: (i32, i32),
    prev: Option<UnitKey>,
    player: Option<(i32, i32)>,
    point: &dyn Fn(UnitKey) -> Option<(i32, i32)>,
) -> Option<UnitKey> {
    pick_layers(world, rects, &UnitLayers::new(), mouse, prev, player, point)
}

/// [`pick_rects`] with the layer test:
/// a player, monster or object the frame rectangle hits
/// is hit only when the point is inside (inclusive) the [`item_box`] of
/// one of its selectable layers (a multi-piece unit, [`layer_test`]) or
/// of its component 1 (TR) (single-piece). A unit with no entry in
/// `layers` takes no layer test. The base game has no pixel test here
/// (reads the layer's `selectable` byte only,
/// open question 2): the layers are the drawn cels.
pub fn pick_layers(
    world: &ClientWorld,
    rects: &BTreeMap<UnitKey, (i32, i32, i32, i32)>,
    layers: &UnitLayers,
    mouse: (i32, i32),
    prev: Option<UnitKey>,
    player: Option<(i32, i32)>,
    point: &dyn Fn(UnitKey) -> Option<(i32, i32)>,
) -> Option<UnitKey> {
    let in_box = |rect: (i32, i32, i32, i32)| {
        let (l, t, r, b) = item_box(rect);
        let (x, y) = mouse;
        x >= l && x <= r && y >= t && y <= b
    };
    let hit = |key: &UnitKey| -> bool {
        if !matches!(key.unit_type, 1 | 2 | 4 | 5) || Some(*key) == world.local_player {
            return false;
        }
        let Some(u) = world.units.get(key) else {
            return false;
        };
        if key.unit_type == 1 && !monster_hoverable(world, u) {
            return false;
        }
        if key.unit_type == 4 && !item_selectable(u) {
            return false;
        }
        if key.unit_type == 2
            && world
                .objclient
                .selectable
                .get(u.class as usize)
                .is_some_and(|s| s[(u.mode & 7) as usize] == 0)
        {
            return false;
        }
        let Some(&(l, t, r, b)) = rects.get(key) else {
            return false;
        };
        let (x, y) = mouse;
        if key.unit_type == TILE {
            // Inclusive, no padding.
            return x >= l && x <= r && y >= t && y <= b;
        }
        if key.unit_type == 4 {
            // An item's hit is its component-1 box alone.
            let (l, t, r, b) = item_box((l, t, r, b));
            return x >= l && x <= r && y >= t && y <= b;
        }
        if !(x >= l - FRAME_RECT_PAD
            && x < r + FRAME_RECT_PAD
            && y >= t - FRAME_RECT_PAD
            && y < b + FRAME_RECT_PAD)
        {
            return false;
        }
        let Some(drawn) = layers.get(key) else {
            return true;
        };
        match layer_test(world, *key, u) {
            LayerTest::None => true,
            LayerTest::Tr => drawn.iter().any(|d| d.component == 1 && in_box(d.rect)),
            LayerTest::Selectable => drawn.iter().any(|d| d.selectable && in_box(d.rect)),
        }
    };
    if let Some(p) = prev.filter(|p| hit(p)) {
        return Some(p);
    }
    let dist = |k: UnitKey| -> i64 {
        match (point(k), player) {
            (Some((x, y)), Some((px, py))) => {
                let (dx, dy) = (i64::from(x - px), i64::from(y - py));
                dx * dx + dy * dy
            }
            _ => i64::MAX,
        }
    };
    let mut best: Option<UnitKey> = None;
    for key in world.units.keys().filter(|k| hit(k)) {
        best = match best {
            None => Some(*key),
            Some(b) => {
                let (pb, pc) = (type_priority(b.unit_type), type_priority(key.unit_type));
                if pc > pb || (pc == pb && dist(*key) < dist(b)) {
                    Some(*key)
                } else {
                    Some(b)
                }
            }
        };
    }
    best
}

/// The hover target a pick left, after the messages that arrived since:
/// a player or monster whose death message came in (dead now, not dead
/// at the pick, `dead_at_pick`) is no longer the target. The death of
/// the hover target clears the hover at once (player
/// code 0x08, monster code 8) and the death drops its selectable flag,
/// so the draw that follows has no highlight (blend `h`; a dying Fallen
/// is drawn in mode 5).
/// PROVISIONAL: the keep-and-re-test branch (a held button) is not
/// modelled; settled by the packets of a held-button kill.
pub fn after_deaths(
    world: &ClientWorld,
    target: Option<UnitKey>,
    dead_at_pick: bool,
) -> Option<UnitKey> {
    target.filter(|k| dead_at_pick || !world.units.get(k).is_some_and(|u| u.is_dead()))
}

/// Whether `target` is dead now (the `dead_at_pick` of [`after_deaths`]).
pub fn target_dead(world: &ClientWorld, target: Option<UnitKey>) -> bool {
    target
        .and_then(|k| world.units.get(&k))
        .is_some_and(|u| u.is_dead())
}

#[cfg(test)]
mod feet_tests {
    use super::*;

    #[test]
    fn static_units_stand_8_rows_above_a_moving_unit_on_the_same_sub_tile() {
        use crate::rules::camera::{FrameSize, OpenMode};
        let at = moving_to_client(100 << 16, 100 << 16);
        let cam = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
        let (mx, my) = unit_feet(&cam, 1, (103, 100));
        for t in [2, 4] {
            // static: (sx − sy)·16, (sx + sy)·8; moving: the centre
            // (c << 16) | 0x8000 gives (sx + sy)·8 + 8 and the same x.
            assert_eq!(unit_feet(&cam, t, (103, 100)), (mx, my - 8), "type {t}");
        }
        assert_eq!(unit_feet(&cam, 0, (103, 100)), (mx, my));
    }

    #[test]
    fn a_non_selectable_object_under_an_npc_is_not_picked() {
        use crate::bridge::world::ClientUnit;
        use crate::rules::camera::{FrameSize, OpenMode};
        let at = moving_to_client(100 << 16, 100 << 16);
        let cam = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
        let mut w = ClientWorld::default();
        let mut npc = ClientUnit::new(UnitKey::new(1, 7));
        npc.mode = 1;
        npc.position = Some((103, 100));
        w.units.insert(npc.key, npc);
        let mut dummy = ClientUnit::new(UnitKey::new(2, 8));
        dummy.class = 1;
        dummy.position = Some((103, 100));
        w.units.insert(dummy.key, dummy);
        let (x, y) = unit_feet(&cam, 1, (103, 100));
        // A static object stands 8 rows above the NPC's feet: nearer.
        let mouse = (x, y - 8);
        w.objclient.selectable = vec![[1; 8], [0; 8]];
        assert_eq!(pick(&w, &cam, mouse), Some(UnitKey::new(1, 7)));
        // A selectable object is nearer but its type priority (−6) loses
        // to the monster's 0.
        w.objclient.selectable = vec![[1; 8], [1; 8]];
        assert_eq!(pick(&w, &cam, mouse), Some(UnitKey::new(1, 7)));
    }

    #[test]
    fn a_dropping_item_is_not_a_target_until_it_lands() {
        use crate::bridge::world::ClientUnit;
        use crate::rules::camera::{FrameSize, OpenMode};
        let at = moving_to_client(100 << 16, 100 << 16);
        let cam = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
        let mut w = ClientWorld::default();
        let key = UnitKey::new(4, 2);
        let mut item = ClientUnit::new(key);
        item.mode = 5;
        item.position = Some((102, 103));
        w.units.insert(key, item);
        let mouse = unit_feet(&cam, 4, (102, 103));
        let rects = BTreeMap::from([(key, (mouse.0 - 10, mouse.1 - 10, mouse.0 + 10, mouse.1))]);
        assert_eq!(pick(&w, &cam, mouse), None);
        assert_eq!(pick_rects(&w, &rects, mouse, None, None, &|_| None), None);
        w.units.get_mut(&key).unwrap().mode = ITEM_ON_FLOOR;
        // Landed but not yet updated in mode 3: the flag is still clear.
        assert_eq!(pick(&w, &cam, mouse), None);
        w.units.get_mut(&key).unwrap().flag_2 = Some(true);
        assert_eq!(pick(&w, &cam, mouse), Some(key));
        assert_eq!(
            pick_rects(&w, &rects, mouse, None, None, &|_| None),
            Some(key)
        );
    }
}

#[cfg(test)]
mod death_tests {
    use super::*;
    use crate::bridge::world::ClientUnit;

    #[test]
    fn a_death_since_the_pick_clears_the_hover_target() {
        let mut w = ClientWorld::default();
        let key = UnitKey::new(1, 21);
        let mut m = ClientUnit::new(key);
        m.mode = 1;
        w.units.insert(key, m);
        let dead = target_dead(&w, Some(key));
        assert!(!dead);
        assert_eq!(after_deaths(&w, Some(key), dead), Some(key));
        // The death message (code 8: mode 0) arrives after the pick.
        w.units.get_mut(&key).unwrap().mode = 0;
        assert_eq!(after_deaths(&w, Some(key), dead), None);
        // A player already dead at the pick stays (a dead player ranks 1).
        let p = UnitKey::new(0, 3);
        let mut u = ClientUnit::new(p);
        u.mode = 0;
        w.units.insert(p, u);
        assert_eq!(after_deaths(&w, Some(p), target_dead(&w, Some(p))), Some(p));
        assert_eq!(after_deaths(&w, None, false), None);
    }
}

#[cfg(test)]
mod rect_tests {
    use super::*;
    use crate::bridge::world::ClientUnit;

    fn world_with_monster() -> (ClientWorld, UnitKey) {
        let mut w = ClientWorld::default();
        let key = UnitKey::new(1, 7);
        let mut m = ClientUnit::new(key);
        m.mode = 1;
        m.position = Some((10, 10));
        w.units.insert(key, m);
        (w, key)
    }

    #[test]
    fn a_units_frame_rectangle_leaves_out_its_overlays() {
        use crate::frames::{FramePart, FrameSet, FrameSetKey, FrameStore, IndexFrame};
        use crate::rules::draw_order::{OrderKey, UnitSlot};
        use crate::scene::{DrawItem, DrawKey, ItemTag};
        let key = UnitKey::new(1, 7);
        let mut frames = FrameStore::default();
        let mut add = |path: &str, w: u32, h: u32| {
            let frame = IndexFrame::new(w, h, 0, 0, vec![1; (w * h) as usize]).unwrap();
            let set = FrameSet {
                frames: vec![frame],
            };
            frames
                .insert(FrameSetKey::new(path, FramePart::Dir(0)).unwrap(), set)
                .unwrap()
        };
        let body = add("data/global/monsters/wa/tr/watrlitwlhth.dcc", 29, 68);
        let balloon = add("data/global/overlays/npcspeechballoon.dcc", 39, 38);
        let slot = OrderKey {
            pass: crate::scene::order::pass::WALLS_UNITS,
            major: 3,
            minor: 1,
        };
        let item = |frame, x, y, sub| {
            let mut d = DrawItem::new(frame, x, y);
            d.key = DrawKey::new(slot.pass, slot.major, slot.minor, sub).unwrap();
            d.tag = ItemTag::Unit(7);
            d
        };
        let items = [item(body, 261, 253, 0), item(balloon, 256, 214, u8::MAX)];
        let slots = BTreeMap::from([(key, UnitSlot::Drawn(slot))]);
        let rects = unit_rects(&items, &frames, &slots);
        assert_eq!(rects.get(&key), Some(&(261, 253, 290, 321)));
    }

    #[test]
    fn the_cursor_hits_a_drawn_frame_rectangle_widened_by_16() {
        let (w, key) = world_with_monster();
        let rects = BTreeMap::from([(key, (100, 100, 140, 180))]);
        assert_eq!(
            pick_rects(&w, &rects, (84, 100), None, None, &|_| None),
            Some(key)
        );
        assert_eq!(
            pick_rects(&w, &rects, (83, 100), None, None, &|_| None),
            None
        );
        assert_eq!(
            pick_rects(&w, &rects, (155, 195), None, None, &|_| None),
            Some(key)
        );
        assert_eq!(
            pick_rects(&w, &rects, (156, 195), None, None, &|_| None),
            None
        );
        assert_eq!(
            pick_rects(&w, &rects, (120, 196), None, None, &|_| None),
            None
        );
    }

    #[test]
    fn the_pick_ranks_by_type_priority_then_nearness_and_keeps_the_last_target() {
        let (mut w, mon) = world_with_monster();
        let far = UnitKey::new(1, 9);
        let obj = UnitKey::new(2, 5);
        for (k, at) in [(far, (12, 12)), (obj, (10, 11))] {
            let mut u = ClientUnit::new(k);
            u.mode = 1;
            u.position = Some(at);
            w.units.insert(k, u);
        }
        let rect = (100, 100, 140, 180);
        let rects = BTreeMap::from([(mon, rect), (far, rect), (obj, rect)]);
        let point = |k: UnitKey| match k.guid {
            7 => Some((0, 10)),
            9 => Some((0, 30)),
            _ => Some((0, 0)),
        };
        let p = Some((0, 0));
        // The object is nearest but priority −6 loses to the monsters;
        // the nearer monster wins.
        assert_eq!(
            pick_rects(&w, &rects, (120, 120), None, p, &point),
            Some(mon)
        );
        // A target still hit stays (sticky).
        assert_eq!(
            pick_rects(&w, &rects, (120, 120), Some(far), p, &point),
            Some(far)
        );
    }

    #[test]
    fn an_item_is_hit_by_its_component_1_box() {
        // 12 px each side; an axis under 32 grows by half the shortfall.
        assert_eq!(item_box((0, 0, 4, 2)), (-14, -16, 18, 16));
        let mut w = ClientWorld::default();
        let key = UnitKey::new(4, 3);
        let mut it = ClientUnit::new(key);
        it.mode = ITEM_ON_FLOOR;
        it.flag_2 = Some(true);
        w.units.insert(key, it);
        // A 28 × 14 cel: box (88, 87, 140, 125), inclusive (one row above
        // the drawn rows).
        let rects = BTreeMap::from([(key, (100, 100, 128, 114))]);
        let at = |m| pick_rects(&w, &rects, m, None, None, &|_| None);
        for m in [(88, 87), (140, 125), (114, 107)] {
            assert_eq!(at(m), Some(key), "{m:?}");
        }
        for m in [(87, 100), (141, 100), (100, 126), (100, 86)] {
            assert_eq!(at(m), None, "{m:?}");
        }
    }

    #[test]
    fn a_cel_box_lies_one_row_above_the_drawn_rows() {
        // Cold Plains: the waypoint's TR cel (131 × 27,
        // offset (−71, 5)); the cursor rests at (768, 276). Drawn at
        // (719, 258) the box ends at row 275 (no highlight at tick
        // 68); at (703, 259) it ends at 276 (the highlight of tick 71).
        let cel = |x: i32, y: i32| (x - 71, y + 5 - 27 + 1, x - 71 + 131, y + 5 + 1);
        assert_eq!(item_box(cel(719, 258)), (636, 224, 791, 275));
        assert_eq!(item_box(cel(703, 259)), (620, 225, 775, 276));
    }

    #[test]
    fn a_single_piece_monster_is_hit_only_inside_its_tr_box() {
        let (mut w, key) = world_with_monster();
        w.objclient.monster_pieces = vec![1];
        let rects = BTreeMap::from([(key, (100, 100, 140, 180))]);
        // TR (component 1) box (88, 88, 140, 132); an S1 layer lower down.
        let layers = UnitLayers::from([(
            key,
            vec![
                LayerRect {
                    layer: 0,
                    component: 8,
                    selectable: true,
                    rect: (100, 150, 140, 180),
                },
                LayerRect {
                    layer: 3,
                    component: 1,
                    selectable: false,
                    rect: (100, 100, 128, 120),
                },
            ],
        )]);
        let at = |m| pick_layers(&w, &rects, &layers, m, None, None, &|_| None);
        assert_eq!(at((120, 110)), Some(key));
        // Inside the frame rectangle and the S1 box, outside the TR box.
        assert_eq!(at((120, 170)), None);
        // No layers listed: the frame rectangle decides.
        assert_eq!(
            pick_layers(
                &w,
                &rects,
                &UnitLayers::new(),
                (120, 170),
                None,
                None,
                &|_| None
            ),
            Some(key)
        );
    }

    #[test]
    fn a_multi_piece_unit_is_hit_inside_any_selectable_layer_box() {
        let (mut w, key) = world_with_monster();
        w.objclient.monster_pieces = vec![2];
        let rects = BTreeMap::from([(key, (100, 100, 140, 180))]);
        let layers = UnitLayers::from([(
            key,
            vec![
                LayerRect {
                    layer: 0,
                    component: 1,
                    selectable: false,
                    rect: (100, 100, 128, 120),
                },
                LayerRect {
                    layer: 1,
                    component: 8,
                    selectable: true,
                    rect: (100, 150, 140, 180),
                },
            ],
        )]);
        let at = |m| pick_layers(&w, &rects, &layers, m, None, None, &|_| None);
        assert_eq!(at((120, 170)), Some(key));
        // Only the non-selectable layer covers this point.
        assert_eq!(at((120, 100)), None);
    }

    #[test]
    fn inside_the_frame_rectangle_a_monster_is_hit_by_its_layer_boxes() {
        // A recorded Fallen Shaman 1:25 at tick 276 of a Charged Bolt cast
        // (attack 2, dir 61, frame 14, client point (364, 194)): TR cel
        // 58 × 71 at offset (−31, 5), S1 cel 117 × 43 at (−86, −43); each
        // drawn on rows `Y + yoff − h + 1 ..= Y + yoff`.
        let (mut w, key) = world_with_monster();
        let rects = BTreeMap::from([(key, (278, 108, 395, 199))]);
        let tr = LayerRect {
            layer: 0,
            component: 1,
            selectable: true,
            rect: (333, 129, 391, 200),
        };
        let mut s1 = LayerRect {
            layer: 1,
            component: 8,
            selectable: true,
            rect: (278, 109, 395, 152),
        };
        let pick = |w: &ClientWorld, s1: LayerRect, m| {
            let layers = UnitLayers::from([(key, vec![tr, s1])]);
            pick_layers(w, &rects, &layers, m, None, None, &|_| None)
        };
        // (300, 200) is inside the frame rectangle (262..411, 92..215)
        // but in neither box (TR 321..=403 × 116..=211, S1 266..=407 ×
        // 96..=163): the base game walks there.
        for pieces in [1, 2] {
            w.objclient.monster_pieces = vec![pieces];
            assert_eq!(pick(&w, s1, (300, 200)), None);
            assert_eq!(pick(&w, s1, (330, 190)), Some(key));
            assert_eq!(pick(&w, s1, (321, 211)), Some(key));
            assert_eq!(pick(&w, s1, (320, 211)), None);
        }
        // Only an S1 box: one piece tests TR alone; two test every
        // selectable layer.
        w.objclient.monster_pieces = vec![1];
        assert_eq!(pick(&w, s1, (300, 150)), None);
        w.objclient.monster_pieces = vec![2];
        assert_eq!(pick(&w, s1, (300, 150)), Some(key));
        s1.selectable = false;
        assert_eq!(pick(&w, s1, (300, 150)), None);
    }

    #[test]
    fn a_unit_without_a_drawn_rectangle_is_not_hovered() {
        let (w, _) = world_with_monster();
        assert_eq!(
            pick_rects(&w, &BTreeMap::new(), (120, 120), None, None, &|_| None),
            None
        );
    }
}

#[cfg(test)]
mod tile_tests {
    use super::*;
    use crate::bridge::drlg::{ClientDrlg, DrlgSource};
    use crate::bridge::world::ClientUnit;
    use crate::rules::camera::{FrameSize, OpenMode};
    use d2_sim::drlg::{DrlgData, LevelDef, LevelTypes, WarpDef};
    use std::sync::Arc;

    struct NoTypes;
    impl LevelTypes for NoTypes {}

    struct NoTiles;
    impl d2_sim::drlg::TileSource for NoTiles {
        fn dt1(&self, _: &[u8]) -> Option<&[d2_sim::drlg::TileInfo]> {
            None
        }
    }

    /// A world whose client DRLG knows one lvlwarp row (`Id` 7, select
    /// box (-30, -60, 60, 40)) and a warp tile of that class at (103, 100).
    fn world_with_tile() -> (ClientWorld, Camera, UnitKey) {
        let mut data = DrlgData {
            levels: vec![LevelDef::default(); 4],
            ..DrlgData::default()
        };
        for l in &mut data.levels {
            l.warp = [-1; 8];
        }
        data.warps = vec![WarpDef {
            id: 7,
            direction: b'b',
            select: (-30, -60, 60, 40),
            ..WarpDef::default()
        }];
        let src = DrlgSource {
            data: Arc::new(data),
            tiles: Arc::new(NoTiles),
            types: Arc::new(|| Box::new(NoTypes)),
        };
        let mut w = ClientWorld {
            drlg: Some(ClientDrlg::build(&src, 0, 1, 0).unwrap()),
            ..ClientWorld::default()
        };
        let key = UnitKey::new(TILE, 3);
        let mut t = ClientUnit::new(key);
        t.class = 7;
        t.position = Some((103, 100));
        w.units.insert(key, t);
        let at = moving_to_client(100 << 16, 100 << 16);
        (
            w,
            Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0)),
            key,
        )
    }

    #[test]
    fn a_warp_tile_is_hovered_inside_its_lvlwarp_select_box_ends_inclusive() {
        let (w, cam, key) = world_with_tile();
        let (fx, fy) = unit_feet(&cam, TILE, (103, 100));
        // left = px + SelectX, right = left + SelectDX (inclusive).
        assert_eq!(pick(&w, &cam, (fx - 30, fy - 60)), Some(key));
        assert_eq!(pick(&w, &cam, (fx + 30, fy - 20)), Some(key));
        assert_eq!(pick(&w, &cam, (fx - 31, fy - 40)), None);
        assert_eq!(pick(&w, &cam, (fx + 31, fy - 40)), None);
        assert_eq!(pick(&w, &cam, (fx, fy - 61)), None);
        assert_eq!(pick(&w, &cam, (fx, fy - 19)), None);
    }

    #[test]
    fn the_drawn_rectangle_hover_picks_a_warp_tile_by_its_select_box() {
        let (w, cam, key) = world_with_tile();
        let rects = tile_rects(&w, &cam);
        let (fx, fy) = unit_feet(&cam, TILE, (103, 100));
        assert_eq!(rects.get(&key), Some(&(fx - 30, fy - 60, fx + 30, fy - 20)));
        // No 16 px padding for a tile.
        assert_eq!(
            pick_rects(&w, &rects, (fx, fy - 40), None, None, &|_| None),
            Some(key)
        );
        assert_eq!(
            pick_rects(&w, &rects, (fx + 31, fy - 40), None, None, &|_| None),
            None
        );
    }

    #[test]
    fn a_warp_tile_of_an_unknown_class_has_no_hover_box() {
        let (mut w, cam, key) = world_with_tile();
        w.units.get_mut(&key).unwrap().class = 99;
        let (fx, fy) = unit_feet(&cam, TILE, (103, 100));
        assert_eq!(pick(&w, &cam, (fx, fy - 40)), None);
        assert!(tile_rects(&w, &cam).is_empty());
    }

    /// A warp object sharing the tile's cell does not take the pick from the
    /// tile (the click path stays the tile's), while a point outside the tile's
    /// select box still reaches the object and its own art highlight.
    #[test]
    fn a_warp_tile_outranks_a_co_located_object() {
        let (mut w, cam, key) = world_with_tile();
        let object = UnitKey::new(crate::bridge::world::OBJECT, 99);
        let mut o = ClientUnit::new(object);
        o.class = 1;
        o.position = Some((103, 100));
        w.units.insert(object, o);
        let (fx, fy) = unit_feet(&cam, TILE, (103, 100));
        assert_eq!(pick(&w, &cam, (fx, fy - 40)), Some(key));
        // The select box's left edge is fx - 30; the object's feet box
        // reaches further left, so the object is hoverable where the tile is not.
        assert_eq!(pick(&w, &cam, (fx - 40, fy)), Some(object));
        assert!(type_priority(TILE) > type_priority(crate::bridge::world::OBJECT));
    }
}

#[cfg(test)]
mod corpse_tests {
    use super::*;
    use crate::bridge::skills::{SkillEntry, SkillList};
    use crate::bridge::world::{ClientUnit, KindData, MonsterData};

    const FIND_POTION: u16 = 131;
    const FIND_ITEM: u16 = 142;
    const REVIVE: u16 = 95;
    const FALLEN: u32 = 19;

    /// A local player with `right` as its right skill and a dead Fallen
    /// (class 19, `corpseSel`, `isAtt`) under the mouse.
    fn world_with_corpse(right: u16) -> (ClientWorld, Camera, UnitKey, (i32, i32)) {
        use crate::rules::camera::{FrameSize, OpenMode};
        let at = moving_to_client(100 << 16, 100 << 16);
        let cam = Camera::new(FrameSize::D2RUST, OpenMode::NONE, at, (0, 0));
        let mut w = ClientWorld::default();
        let p = UnitKey::new(0, 1);
        let mut player = ClientUnit::new(p);
        player.skills = Some(SkillList {
            entries: vec![SkillEntry {
                skill: right,
                ..SkillEntry::default()
            }],
            right: Some(0),
            ..SkillList::default()
        });
        w.units.insert(p, player);
        w.local_player = Some(p);
        let key = UnitKey::new(1, 8);
        let mut m = ClientUnit::new(key);
        m.class = FALLEN;
        m.mode = 12;
        m.flag_2 = Some(false);
        m.flag_4 = true;
        m.position = Some((103, 103));
        w.units.insert(key, m);
        let mut selectproc = vec![0; 200];
        selectproc[FIND_POTION as usize] = 4;
        selectproc[FIND_ITEM as usize] = 5;
        selectproc[REVIVE as usize] = 3;
        let mut monsters = vec![None; 20];
        monsters[FALLEN as usize] = Some(CorpseClass {
            corpse_sel: true,
            soft: true,
            moves: Some(true),
            can_switch: true,
            base_id: FALLEN as u16,
        });
        w.corpse_tables = CorpseTables {
            selectproc,
            monsters,
            udead: vec![false, true],
        };
        let mouse = unit_feet(&cam, 1, (103, 103));
        (w, cam, key, mouse)
    }

    #[test]
    fn find_potion_and_find_item_pick_a_fallen_corpse() {
        for skill in [FIND_POTION, FIND_ITEM] {
            let (w, cam, key, mouse) = world_with_corpse(skill);
            assert_eq!(pick(&w, &cam, mouse), Some(key));
        }
    }

    #[test]
    fn a_corpse_is_not_picked_without_a_corpse_skill_or_its_gates() {
        // Attack (skill 0, proc 0): no corpse target.
        let (w, cam, _, mouse) = world_with_corpse(0);
        assert_eq!(pick(&w, &cam, mouse), None);
        // Flag 0x4 clear (`isAtt` 0, or the hide group at death).
        let (mut w, cam, key, mouse) = world_with_corpse(FIND_ITEM);
        w.units.get_mut(&key).unwrap().flag_4 = false;
        assert_eq!(pick(&w, &cam, mouse), None);
        // Still in the death mode 0: the core test wants mode 12.
        let (mut w, cam, key, mouse) = world_with_corpse(FIND_ITEM);
        w.units.get_mut(&key).unwrap().mode = 0;
        assert_eq!(pick(&w, &cam, mouse), None);
        // A state of the `udead` group.
        let (mut w, cam, key, mouse) = world_with_corpse(FIND_ITEM);
        w.units.get_mut(&key).unwrap().states.insert(1);
        assert_eq!(pick(&w, &cam, mouse), None);
        // Find Potion (proc 4) also wants `soft`; Find Item (proc 5) not.
        for (skill, want) in [(FIND_POTION, false), (FIND_ITEM, true)] {
            let (mut w, cam, key, mouse) = world_with_corpse(skill);
            w.corpse_tables.monsters[FALLEN as usize] = Some(CorpseClass {
                corpse_sel: true,
                soft: false,
                moves: Some(true),
                ..CorpseClass::default()
            });
            assert_eq!(pick(&w, &cam, mouse), want.then_some(key));
        }
    }

    #[test]
    fn revive_picks_a_switchable_corpse_that_is_not_unique() {
        // Proc 3: a Fallen corpse passes.
        let (w, cam, key, mouse) = world_with_corpse(REVIVE);
        assert_eq!(pick(&w, &cam, mouse), Some(key));
        // The switch test, `Velocity` and `BaseId` 78 each refuse it.
        type Edit = fn(&mut CorpseClass);
        let edits: [Edit; 3] = [
            |c| c.can_switch = false,
            |c| c.moves = Some(false),
            |c| c.base_id = 78,
        ];
        for edit in edits {
            let (mut w, cam, _, mouse) = world_with_corpse(REVIVE);
            edit(w.corpse_tables.monsters[FALLEN as usize].as_mut().unwrap());
            assert_eq!(pick(&w, &cam, mouse), None);
        }
        // A unique (0x8) or superunique (0x2) corpse; a champion (0x4)
        // passes.
        for (flags, want) in [(0x8, false), (0x2, false), (0x4, true)] {
            let (mut w, cam, key, mouse) = world_with_corpse(REVIVE);
            w.units.get_mut(&key).unwrap().kind = KindData::Monster(Box::new(MonsterData {
                flags,
                ..MonsterData::default()
            }));
            assert_eq!(pick(&w, &cam, mouse), want.then_some(key));
        }
    }
}
