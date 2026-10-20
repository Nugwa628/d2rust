// Spec: specs/world/quests-helpers.md
// Spec: specs/world/quests-act2-2.md (§5.1 quest-chest gate, §5.3 remove a unit for everyone, §5.4 scroll text)
//! The quest code's shared helper functions, each built over the
//! narrower [`QuestWorld`] seams (room box `0x00619730`, room lookup
//! `0x00463740`, box query `0x0064D800`, monster spawn `0x005B2F20`,
//! missile creation `0x0059FA30`, …): the free-spot search (§1), the
//! critical spawn (§2), the superunique spawn at a point (§3), the quest
//! missiles (§4), the end of a player's interaction (§5), the town
//! portal close (§7), and the Act II–III helpers of `quests-act2-2.md`
//! §5. §6 (game end) and the save pass are host requests
//! ([`QuestControl::end_game`], [`QuestControl::save_pass`]).
//!
//! Status: implemented from the spec, unverified (no recording).

use super::{QuestControl, QuestError, QuestWorld};
use crate::drlg::TileRect;
use crate::units::{RoomId, UnitId};

/// A zeroed `0x0059FA30` creation record with the fields the quest code
/// writes (`missiles/missiles.md` §R2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestMissile {
    /// +0x00.
    pub flags: u32,
    /// +0x04.
    pub owner: UnitId,
    /// +0x08.
    pub origin: Option<UnitId>,
    /// +0x10.
    pub class: u16,
    /// +0x14, +0x18 (flag 1: position given).
    pub x: i32,
    pub y: i32,
    /// +0x1C, +0x20 (flag 0x20: target absolute).
    pub target_x: i32,
    pub target_y: i32,
    /// +0x2C.
    pub skill: u16,
    /// +0x30.
    pub level: u8,
}

/// `0x00619730` for a room the seam does not know: the all-zero box it
/// returns for a null room (`quests-act1-rest.md` §9 item 1).
fn room_box<W: QuestWorld + ?Sized>(w: &mut W, room: RoomId) -> TileRect {
    w.room_box(room).unwrap_or_default()
}

/// The unsigned containment of §1 rule 2.2 ("unsigned compares").
fn inside_unsigned(b: TileRect, x: i32, y: i32) -> bool {
    let (x, y) = (x as u32, y as u32);
    (b.x as u32) <= x
        && x < (b.x.wrapping_add(b.w)) as u32
        && (b.y as u32) <= y
        && y < (b.y.wrapping_add(b.h)) as u32
}

/// §1 `0x00545340(room R0, &point, size s, mask m, &out, unused, limit
/// L)`: the spot and its room, `None` when nothing is accepted or the
/// spot's room lookup is null (the out room is 0; every quest caller
/// tests it). The sixth ("radius") argument is never read.
pub fn free_spot<W: QuestWorld + ?Sized>(
    w: &mut W,
    r0: RoomId,
    x: i32,
    y: i32,
    s: u32,
    m: u32,
    limit: u32,
) -> Option<(i32, i32, RoomId)> {
    let h = (s >> 1) as i32;
    let (bx, by) = (x - h, y - h);
    let size = (2 * s + 1) as i32;
    // B as last read: R0's box before the first column test.
    let mut b = room_box(w, r0);
    let mut xl = bx;
    for k in 1..i64::from(limit) {
        let k = k as i32;
        let lo = -k;
        let mut j = k;
        while j > lo {
            let yy = by + j;
            let rr = if b.y <= yy && yy < b.y + b.h {
                Some(r0)
            } else {
                w.room_at(r0, xl, yy)
            };
            if let Some(rr) = rr {
                let mut i = lo;
                while i < k {
                    let xx = bx + i;
                    xl = xx;
                    b = room_box(w, rr);
                    // Edge case 1: the row coordinate against the x end.
                    let cr = if xx < b.x || yy >= b.x + b.w {
                        w.room_at(rr, xx, yy)
                    } else {
                        Some(rr)
                    };
                    if let Some(cr) = cr {
                        if !w.box_collides(cr, xx, yy, size, m) && inside_unsigned(b, xx, yy) {
                            let (px, py) = (xx + h, yy + h);
                            return w.room_at(r0, px, py).map(|r| (px, py, r));
                        }
                    }
                    i += 2;
                }
            }
            j -= 2;
        }
    }
    None
}

/// §2 `0x005459A0(game, x, y, room R, flag, class)` with flag 1 (every
/// quest caller): the monster, or `None`.
pub fn critical_spawn<W: QuestWorld + ?Sized>(
    w: &mut W,
    r: RoomId,
    x: i32,
    y: i32,
    class: u16,
) -> Option<UnitId> {
    // Step 1: a point inside R's box shrunk by one.
    let b = room_box(w, r);
    let (mut px, mut py) = (x, y);
    let mut inside = false;
    for _ in 0..=20 {
        if b.x <= px && b.y <= py && px < b.x + b.w - 1 && py < b.y + b.h - 1 {
            inside = true;
            break;
        }
        px += 1;
        py += 1;
    }
    if !inside {
        // Edge case 3: the x slot overwritten with y.
        (px, py) = (y, y + 21);
    }
    // Step 2.
    let (mut room, mut sx, mut sy) =
        free_spot(w, r, px, py, 2, 0x100, 100).map_or((r, x, y), |(fx, fy, fr)| (fr, fx, fy));
    // Step 3.
    let mut made = w.spawn_monster_flags(room, sx, sy, class, 1, -1, 0);
    if made.is_none() {
        made = w.spawn_monster_flags(room, sx, sy, class, 1, 5, 0);
    }
    // Step 4.
    if made.is_none() {
        for _ in 0..20 {
            sx += 1;
            sy += 1;
            let from = match w.room_at(room, sx, sy) {
                Some(next) => next,
                None => {
                    (sx, sy) = (x, y);
                    r
                }
            };
            (room, sx, sy) = free_spot(w, from, sx, sy, 2, 0x100, 100)
                .map_or((r, x, y), |(fx, fy, fr)| (fr, fx, fy));
            made = w.spawn_monster_flags(room, sx, sy, class, 1, 10, 0);
            if made.is_some() {
                break;
            }
        }
    }
    // Step 5.
    if made.is_none() {
        made = w.spawn_monster_flags(r, x, y, class, 1, 15, 0);
    }
    let m = made?;
    // Step 6 (flag 1).
    w.or_unit_flags(m, 0x0300_0000);
    Some(m)
}

/// §3 `0x00545C30(game, unit U, &point, kind, id)`. Kind 2 (every quest
/// caller) makes the class id + the `monstats.txt` row count, which the
/// preset spawn `0x0054E600` reads as superunique `id`; another kind
/// (no quest caller) is reported (`0x00659B80`) and spawns nothing.
pub fn spawn_superunique<W: QuestWorld + ?Sized>(
    w: &mut W,
    unit: UnitId,
    x: i32,
    y: i32,
    kind: u32,
    id: u16,
) -> Option<UnitId> {
    if kind != 2 {
        w.unhandled(0xFE, 0x0065_9B80);
        return None;
    }
    let (_, _, own) = w.unit_position(unit)?;
    let room = w.room_at(own, x, y)?;
    w.preset_superunique_spawn(room, x, y, id)
}

/// §4.1 `0x0058C8D0` (missile 541 from the victim to its statue `to`,
/// already found by GUID; flags 0x420, level 1 at the caller) and the
/// same record form for any (missile, flags, level).
pub fn quest_missile<W: QuestWorld + ?Sized>(
    w: &mut W,
    from: UnitId,
    to: UnitId,
    class: u16,
    flags: u32,
    level: u8,
) -> Option<UnitId> {
    let (tx, ty) = w.unit_xy(to).unwrap_or((0, 0));
    let m = w.spawn_missile(QuestMissile {
        flags,
        owner: from,
        origin: Some(from),
        class,
        x: 0,
        y: 0,
        target_x: tx,
        target_y: ty,
        skill: 0,
        level,
    })?;
    let g = w.guid(to);
    w.set_missile_guid(m, g);
    Some(m)
}

/// `0x006417F0(unit, x, y)`: max(|dx|, |dy|) + ⌊min(|dx|, |dy|) / 2⌋ from
/// the unit's position.
fn point_distance(from: (i32, i32), x: i32, y: i32) -> i32 {
    let dx = (from.0 - x).abs();
    let dy = (from.1 - y).abs();
    dx.max(dy) + dx.min(dy) / 2
}

/// §4.2 `0x0056EDE0(game, owner, skill, level, class, x, y)`.
pub fn missile_at_point<W: QuestWorld + ?Sized>(
    w: &mut W,
    owner: UnitId,
    skill: u16,
    level: u8,
    class: u16,
    x: i32,
    y: i32,
) -> Option<UnitId> {
    let (x, y) = if (x, y) == (0, 0) {
        w.path_target_xy(owner).unwrap_or((0, 0))
    } else {
        (x, y)
    };
    if (x, y) == (0, 0) {
        return None;
    }
    let at = w.unit_xy(owner).unwrap_or((0, 0));
    if point_distance(at, x, y) > 100 {
        return None;
    }
    w.spawn_missile(QuestMissile {
        flags: 1,
        owner,
        origin: None,
        class,
        x,
        y,
        target_x: 0,
        target_y: 0,
        skill,
        level,
    })
}

/// The orb's stairs object (`stairsr`, class 386, §4.3).
const ORB_STAIRS: u16 = 386;
/// `orbmist` (§4.3).
const ORB_MISSILE: u16 = 368;

/// §4.3 `0x005DFEE0(game, monster M)`: the orb missile. No class-386
/// object among M's room and its adjacent rooms is the fatal 0x8F
/// (reported as [`QuestError::Fatal`] of `0x005DFEE0`).
pub fn orb_missile<W: QuestWorld + ?Sized>(ctl: &mut QuestControl, w: &mut W, m: UnitId) {
    let room = w.unit_position(m).map(|p| p.2);
    let units = room.map(|r| w.adjacent_units(r)).unwrap_or_default();
    let o = units.into_iter().find(|&u| {
        let g = w.guid(u);
        w.object_by_guid(g) == Some((u, ORB_STAIRS))
    });
    let Some(o) = o else {
        return ctl.faults.push(QuestError::Fatal(0x005D_FEE0));
    };
    let (tx, ty) = w.unit_xy(o).unwrap_or((0, 0));
    let made = w.spawn_missile(QuestMissile {
        flags: 0x420,
        owner: m,
        origin: Some(m),
        class: ORB_MISSILE,
        x: 0,
        y: 0,
        target_x: tx,
        target_y: ty,
        skill: 0,
        level: 1,
    });
    match made {
        Some(missile) => {
            let g = w.guid(o);
            w.set_missile_guid(missile, g);
            w.refresh_room(o);
        }
        None if w.object_mode(o) == 0 => {
            w.set_object_mode(o, 1);
            let at = w.frame() + (w.object_anim_length(o) >> 8);
            w.schedule_object_event(o, 1, at);
        }
        None => {}
    }
}

/// S→C 0x62 (7 bytes, `0x0053D6D0`): kind u8@1, GUID u32@2; byte 6 is a
/// stale stack byte in 1.14d, 0 here (traces mask it).
pub fn msg_end_interaction(kind: u8, guid: u32) -> [u8; 7] {
    let mut m = [0u8; 7];
    m[0] = 0x62;
    m[1] = kind;
    m[2..6].copy_from_slice(&guid.to_le_bytes());
    m
}

/// Obelisk classes 80, 90; the Steeg Stone 337; the orifice 152 (§5).
const OBELISKS: [u16; 2] = [80, 90];
const STEEG_STONE: u16 = 337;
const ORIFICE: u16 = 152;

/// §5 `0x005351C0(game, player)`: end the player's interaction by kind.
pub fn end_interaction<W: QuestWorld + ?Sized>(w: &mut W, player: UnitId) {
    let found = w
        .interact_unit(player)
        .and_then(|(kind, guid)| w.unit_by_guid(kind, guid).map(|u| (kind, guid, u)));
    let Some((kind, guid, unit)) = found else {
        return w.send(player, &msg_end_interaction(6, 0));
    };
    match kind {
        0 => return w.trade_button(player, 6),
        1 => {
            w.set_interact_unit(player, None);
            w.free_chat_node(unit, player);
        }
        2 => {
            let class = w.object_by_guid(guid).map(|o| o.1);
            match class {
                Some(c) if OBELISKS.contains(&c) => w.obelisk_close(player, unit),
                Some(STEEG_STONE) => w.steeg_release(unit, player),
                Some(ORIFICE) => w.set_object_mode(unit, 0),
                _ => {}
            }
            w.set_interact_unit(player, None);
        }
        3 => return w.send(player, &msg_end_interaction(6, 0)),
        4 => {
            if w.item_code(unit) != Some(*b"box ") {
                return;
            }
            w.set_interact_unit(player, None);
            w.close_cube(player);
        }
        // The jump table `0x005352A4` has the kinds 0–4 only.
        _ => return w.unhandled(0xFE, 0x0053_51C0),
    }
    w.send(player, &msg_end_interaction(kind, guid));
}

/// `0x0059D9D0` (`quests-act3.md` §4.7, `quests-act5-2.md` §7.8): for each
/// unit of the object's room unit list of type 5 (warp tile),
/// `0x005550B0(game, player, tile)`.
pub fn stairs_warp<W: QuestWorld + ?Sized>(w: &mut W, player: UnitId, object: UnitId) {
    for tile in w.room_warp_tiles(object) {
        w.warp_through(player, tile);
    }
}

/// The town portal class (§7).
const TOWN_PORTAL: u16 = 59;

/// §7 `0x00535430(game, player)`: close the player's town portal and its
/// partner, each after chain 35's hook `0x0058CF50`.
pub fn close_town_portal<W: QuestWorld + ?Sized>(
    ctl: &mut QuestControl,
    w: &mut W,
    player: UnitId,
) {
    let Some(g) = w.town_portal_guid(player) else {
        return;
    };
    let Some((p, TOWN_PORTAL)) = w.object_by_guid(g) else {
        return;
    };
    super::act5::q5::town_portal_closed(ctl);
    let q = w.portal_partner(p);
    w.free_portal_object(p);
    if let Some(q) = q {
        super::act5::q5::town_portal_closed(ctl);
        w.free_portal_object(q);
    }
}

/// `quests-act2-2.md` §5.1 `0x00545850(op)`: the quest-chest gate. True
/// = the chest goes on (it was closed and is now opening).
pub fn quest_chest_gate<W: QuestWorld + ?Sized>(w: &mut W, object: UnitId) -> bool {
    if w.object_mode(object) != 0 {
        return false;
    }
    match w.object_mode1(object) {
        Some(true) => {
            w.set_object_mode(object, 1);
            // No "+ 1" (unlike the chest open of `objects.md` §8.1).
            let at = w.frame() + (w.object_anim_length(object) >> 8);
            w.schedule_object_event(object, 1, at);
        }
        _ => w.set_object_mode(object, 2),
    }
    w.clear_unit_flags(object, 0x2);
    true
}

/// `quests-act2-2.md` §5.4 `0x005456A0(player, object, string)`: S→C 0x27
/// (40 bytes): unit type 2, the object's GUID, count 1, entry 0 kind 0
/// with the string id; bytes 7, 9 and 12–39 are not written in 1.14d (0
/// here, masked by the traces).
pub fn msg_scroll_text(object_guid: u32, string: u16) -> [u8; 40] {
    let mut m = [0u8; 40];
    m[0] = 0x27;
    m[1] = 2;
    m[2..6].copy_from_slice(&object_guid.to_le_bytes());
    m[6] = 1;
    m[8] = 0;
    m[10..12].copy_from_slice(&string.to_le_bytes());
    m
}

#[cfg(test)]
#[path = "helpers_tests.rs"]
mod tests;
