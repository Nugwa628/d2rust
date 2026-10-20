// Spec: specs/world/objects-2.md §16–§18
//! Part 2 of the object functions: the operate functions of §16 (torch
//! tiki, trap door, obelisk, secret door, stands and racks, bookshelf,
//! teleport pad, slime door, exploding chest, bank, stairs, jungle stash,
//! Harrogath gate, trapped soul), the small init functions of §17 and the
//! object events 0, 3, 8, 9, 10 of §18 (event 1 is [`super::end_anim`]).
//! Rolls are on the object-control seed (`ctl.seed`); mode changes draw
//! on the object's unit seed through [`set_mode`] (§4).

use crate::units::{RoomId, UnitId};

use super::chests::{reach_distance, trap_arm, unit_type, ChestWorld};
use super::misc::stat;
use super::{
    clear_selectable, frame_cnt, schedule_endanim, selectable, set_mode, sound, MiscWorld,
    ObjectControl, ObjectError, ObjectTables, ObjectWorld, Operate,
};

#[cfg(test)]
mod tests;

/// The bank class (§16.10).
pub const BANK_CLASS: u16 = 267;
/// The fissure class (§18.3).
pub const FISSURE_CLASS: u16 = 399;
/// Player death mode (burn and spike tests skip it, §18).
pub const PLAYER_DEATH_MODE: u8 = 17;
/// Gate debounce (§16.13, as §10).
pub const GATE_DEBOUNCE: u32 = 500;
/// The teleport pad's free point: size 3, mask 0x1C09, no fallback
/// (§16.7).
pub const PAD_SPOT_SIZE: i32 = 3;
pub const PAD_SPOT_MASK: u32 = 0x1C09;
/// The gold placeholder's point mask (§17).
pub const GOLD_POINT_MASK: u32 = 0x3F11;
/// Player flags 2 bit set by the teleport pad (§16.7).
pub const PAD_FLAGS2: u32 = 0x10000;
/// Object event 0 (fire), 3 (spike trap), 8, 9 (trapped soul), 10
/// (jungle stash drop) (`sim/units.md` §6.4).
pub mod mevent {
    pub const FIRE: u8 = 0;
    pub const SPIKE: u8 = 3;
    pub const EVENT8: u8 = 8;
    pub const SOUL: u8 = 9;
    pub const STASH_DROP: u8 = 10;
}

/// A 4-character item code (little-endian dword).
const fn code(c: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*c)
}

/// The seams of part 2 beyond the chest and misc seams. Every default is
/// the narrowest reading: nothing happens, or none.
#[allow(unused_variables)]
pub trait MechWorld: ChestWorld + MiscWorld {
    /// `0x005550B0(game, player, tile)` (`sim/path-placement.md` §12.2):
    /// warp the player through the warp tile.
    fn warp_through_tile(&mut self, player: UnitId, tile: UnitId) {}
    /// `0x00554D00`: the unit the player interacts with (P +0x6C ≠ 0 →
    /// the unit with GUID P +0x64).
    fn interact_unit(&self, player: UnitId) -> Option<UnitId> {
        None
    }
    /// `0x00554120`: P's interact := (type, GUID) when P +0x6C = 0.
    fn set_interact(&mut self, player: UnitId, unit_type: u8, guid: u32) {}
    /// `0x0055EEA0(game, player, item)` (§19 rule 1): the item comes off
    /// the player's cursor; `false`: failure. Default: failure.
    fn remove_cursor_item(&mut self, player: UnitId, item: UnitId) -> bool {
        false
    }
    /// The `subtype` byte (+0x122) of the item's items record (§19 rule
    /// 2). Default 0.
    fn item_subtype(&self, item: UnitId) -> u8 {
        0
    }
    /// `0x006272B0`: base-stat add of `delta` to stat `stat` of the player
    /// (§19 power-up table). Default: nothing.
    fn power_up_add_stat(&mut self, player: UnitId, stat: u16, delta: i32) {}
    /// `0x00554190`: clear P's interact (+0x64 := −1, +0x68 := 6, +0x6C
    /// := 0).
    fn clear_interact(&mut self, player: UnitId) {}
    /// `0x005825B0`: P holds an item of type 20 (gem) in its item list.
    fn has_gem(&mut self, player: UnitId) -> bool {
        false
    }
    /// A message to the player's client.
    fn send(&mut self, player: UnitId, msg: &[u8]) {}
    /// `0x005594C0` (armor, `weapon` false) or `0x00559630` (weapon)
    /// with (room, &pos, −1, 0, 0) at the object (§20.1, §20.2).
    fn stand_drop(&mut self, object: UnitId, weapon: bool) {}
    /// `0x00559A30` with a quality argument (§20.4): drop the item of
    /// `code` at the object.
    fn drop_code_quality(&mut self, object: UnitId, code: u32, quality: u8) {}
    /// The room's adjacency array in index order (`drlg/rooms.md` §6),
    /// the room itself included where the array holds it.
    fn adjacent_rooms(&self, room: RoomId) -> Vec<RoomId> {
        Vec::new()
    }
    /// `0x0064E7B0(room, &point, size, mask, no fallback)`
    /// (`sim/path-placement.md` §7): the room and the free point.
    fn free_point(
        &self,
        room: RoomId,
        x: i32,
        y: i32,
        size: i32,
        mask: u32,
    ) -> Option<(RoomId, i32, i32)> {
        None
    }
    /// `0x00554EA0(game, unit, room, x, y, 0, 0)` (`sim/path-placement.md`
    /// §10); `false`: not placed.
    fn place_unit(&mut self, unit: UnitId, room: RoomId, x: i32, y: i32) -> bool {
        false
    }
    /// S→C 0x07 (`0x0053BC50`) to the player's client: the room's tile x,
    /// y (room rect +0x10, +0x14) and its level.
    fn send_room_reveal(&mut self, player: UnitId, room: RoomId) {}
    /// Unit flags 2 (+0xC8) |= `bits`.
    fn set_flags2(&mut self, unit: UnitId, bits: u32) {}
    /// `0x005DFA00(game, object, target, arg)` (combat spec): trap damage.
    fn trap_damage_arg(&mut self, object: UnitId, target: UnitId, arg: u32) {}
    /// `0x0061AB00`: the room's level is a town.
    fn in_town(&self, room: RoomId) -> bool {
        false
    }
    /// `0x0055FA40` (`items/inventory.md` §5.5): the scroll / tome
    /// recount of the player.
    fn recount_tomes(&mut self, player: UnitId) {}
    /// `0x00552F60(game, type 0, guid)`: the player with `guid`.
    fn find_player(&self, guid: u32) -> Option<UnitId> {
        None
    }
    /// `0x0061C100(game act 0, none)`: the Act I period of day.
    fn day_period_act1(&self) -> u32 {
        0
    }
}

/// Mode 0 (or none): an operate-record object in mode 0.
fn mode0<W: ObjectWorld>(w: &W, o: UnitId) -> bool {
    w.mode(o) == 0
}

/// P busy `0x00535060` (as §12 rule 1).
fn busy<W: ObjectWorld>(w: &W, p: UnitId) -> bool {
    w.interact_active(p) || w.cursor_item(p) || w.player_busy(p)
}

/// The operate functions of §16 (and operate 48, §18.4).
pub fn operate<W: MechWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    op: &Operate,
) -> Result<i32, ObjectError> {
    let o = op.object;
    let class = op.class;
    match op.operate_fn {
        // §16.1.
        13 => {
            match w.mode(o) {
                0 => set_mode(t, w, o, class, 1, true)?,
                1 => set_mode(t, w, o, class, 0, true)?,
                _ => {}
            }
            Ok(0)
        }
        16 => trap_door(ctl, t, w, op),
        17 => obelisk(t, w, op),
        // §16.4.
        18 => {
            if mode0(w, o) {
                set_mode(t, w, o, class, 1, true)?;
                clear_selectable(w, o);
                schedule_endanim(w, t.object(class)?, o);
                w.free_footprint(o);
            }
            Ok(1)
        }
        // §16.5: the drop's draws come before the mode change's.
        19 | 20 => {
            if mode0(w, o) {
                w.stand_drop(o, op.operate_fn == 20);
                set_mode(t, w, o, class, 2, true)?;
                clear_selectable(w, o);
            }
            Ok(1)
        }
        26 => bookshelf(ctl, t, w, op),
        27 => teleport_pad(w, op),
        // §16.8.
        29 => {
            if w.mode(o) != 0 {
                return Ok(1);
            }
            set_mode(t, w, o, class, 1, true)?;
            w.free_footprint(o);
            let r = t.object(class)?;
            let sel = selectable(r, w.mode(o)) != 0;
            let f = w.flags(o);
            w.set_flags(
                o,
                if sel {
                    f | super::oflags::SELECTABLE
                } else {
                    f & !super::oflags::SELECTABLE
                },
            );
            if r.mode2 != 0 {
                schedule_endanim(w, r, o);
            }
            Ok(1)
        }
        // §16.9.
        30 => {
            if mode0(w, o) {
                if let Some(p) = op.operator {
                    w.trap_damage_arg(o, p, 0);
                    w.trap_damage_arg(o, p, 1);
                }
                set_mode(t, w, o, class, 1, true)?;
                schedule_endanim(w, t.object(class)?, o);
            }
            Ok(0)
        }
        32 => bank(w, op),
        47 => stairs(t, w, op),
        // §16.11: only mode 2 runs 47's warp.
        50 => {
            if w.mode(o) == 2 {
                stairs(t, w, op)
            } else {
                Ok(0)
            }
        }
        51 => jungle_stash(ctl, t, w, op),
        61 => gate(ctl, t, w, op),
        48 => trapped_soul_operate(ctl, t, w, op),
        _ => Ok(0),
    }
}

/// The first unit of type 5 (warp tile) in `room`'s unit list.
fn warp_tile<W: MechWorld>(w: &W, room: RoomId) -> Option<UnitId> {
    w.room_units(room)
        .into_iter()
        .find(|&u| w.unit_type(u) == Some(unit_type::TILE))
}

/// §16.2 trap door.
fn trap_door<W: MechWorld>(
    _ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    op: &Operate,
) -> Result<i32, ObjectError> {
    let o = op.object;
    match w.mode(o) {
        0 => set_mode(t, w, o, op.class, 2, true)?,
        2 => {
            let tile = w
                .room(o)
                .and_then(|r| warp_tile(w, r))
                .ok_or(ObjectError::NoWarpTile(o))?;
            if let Some(p) = op.operator {
                w.warp_through_tile(p, tile);
            }
        }
        _ => {}
    }
    Ok(1)
}

/// §16.3 obelisk.
fn obelisk<W: MechWorld>(t: &ObjectTables, w: &mut W, op: &Operate) -> Result<i32, ObjectError> {
    let o = op.object;
    let Some(p) = op.operator else {
        return Ok(1);
    };
    match w.mode(o) {
        0 => {
            if busy(w, p) {
                return Ok(1);
            }
            if !w.has_gem(p) {
                w.sound(p, sound::PORTAL_REFUSED, Some(p), false);
                return Ok(1);
            }
            let guid = w.guid(o);
            w.set_interact(p, unit_type::OBJECT, guid);
            set_mode(t, w, o, op.class, 3, true)?;
            // Byte 6 is never written in 1.14d (stack contents); D2RUST
            // sends 0.
            let g = guid.to_le_bytes();
            w.send(p, &[0x58, g[0], g[1], g[2], g[3], 0, 0]);
        }
        3 if w.interact_unit(p) == Some(o) => {
            w.clear_interact(p);
            set_mode(t, w, o, op.class, 1, true)?;
        }
        _ => {}
    }
    Ok(1)
}

/// The obelisk object class that skips the gem insert branch (§19 rule 1:
/// the orifice, `quests-act2-2.md`).
pub const ORIFICE_CLASS: u16 = 152;
/// The power-up table size (dword `0x00732FAC`, §19 rule 2.1).
pub const POWER_UP_COUNT: u8 = 21;

/// Power-up table `0x00732EB0` (§19): the chance of entry `s` (of 100).
fn power_up_chance(s: u8) -> u32 {
    const STEPS: [u32; 3] = [5, 10, 15];
    match s {
        0..=2 | 15..=17 => 100,
        3..=14 => STEPS[usize::from(s) % 3],
        _ => [3, 6, 10][usize::from(s) % 3],
    }
}

/// The power-up `0x00585240(game, s)` of §19 rule 2 with P in EDI: `true`
/// is b = 1.
pub fn power_up<W: MechWorld>(w: &mut W, p: UnitId, s: u8) -> bool {
    if s >= POWER_UP_COUNT {
        return false;
    }
    let r = w.unit_seed(p).map_or(0, |seed| seed.roll(100));
    if r >= power_up_chance(s) {
        return false;
    }
    let v: i32 = if s == 2 || s == 17 { 2 } else { 1 };
    match s / 3 {
        0 => {
            let m = (w.vital_stat(p, stat::MAX_MANA) as i32 + v * 256) as u32;
            w.set_vital_stat(p, stat::MANA, m);
            w.set_vital_stat(p, stat::MAX_MANA, m);
        }
        // Energy, dexterity, vitality, strength.
        g @ 1..=4 => w.power_up_add_stat(p, [0, 1, 2, 3, 0][usize::from(g)], v),
        5 => {
            let m = (w.vital_stat(p, stat::MAX_LIFE) as i32 + v * 256) as u32;
            w.set_vital_stat(p, stat::LIFE, m);
            w.set_vital_stat(p, stat::MAX_LIFE, m);
        }
        _ => w.power_up_add_stat(p, 5, v),
    }
    true
}

/// §19 C→S 0x44 action 3 on an object of class ≠ 152 (`0x005852E0`;
/// the entry checks and the other actions are `quests-act2-2.md` §3.2):
/// the item comes off the cursor, P's interact is cleared, the power-up
/// rolls on P's seed, S→C 0x58 reports it, and the object goes to mode 1
/// (then ENDANIM) or 0. No test of the object's mode or kind (edge case
/// 2). Returns 1, also after the cursor-removal failure (`mov eax, 1` at
/// `0x0058539D`, objects-2.md §19 rule 1).
pub fn obelisk_insert<W: MechWorld>(
    t: &ObjectTables,
    w: &mut W,
    player: UnitId,
    object: UnitId,
    class: u16,
    item: UnitId,
) -> Result<i32, ObjectError> {
    let g = w.guid(object).to_le_bytes();
    if !w.remove_cursor_item(player, item) {
        w.send(player, &[0x58, g[0], g[1], g[2], g[3], 4, 0]);
        return Ok(1);
    }
    w.clear_interact(player);
    let s = w.item_subtype(item);
    let b = power_up(w, player, s);
    w.send(player, &[0x58, g[0], g[1], g[2], g[3], 5, u8::from(b)]);
    if b {
        set_mode(t, w, object, class, 1, true)?;
        schedule_endanim(w, t.object(class)?, object);
    } else {
        set_mode(t, w, object, class, 0, true)?;
    }
    Ok(1)
}

/// §16.6 bookshelf.
fn bookshelf<W: MechWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    op: &Operate,
) -> Result<i32, ObjectError> {
    let o = op.object;
    if !mode0(w, o) {
        return Ok(1);
    }
    set_mode(t, w, o, op.class, 2, true)?;
    clear_selectable(w, o);
    let r = ctl.seed.step() % 20;
    let bit = ctl.seed.step() & 1;
    let c = match (r < 13, bit) {
        (true, 0) => code(b"tsc "),
        (true, _) => code(b"isc "),
        (false, 0) => code(b"tbk "),
        (false, _) => code(b"ibk "),
    };
    ctl.get_mut(o)?.drop_code = c;
    w.drop_code_quality(o, c, 2);
    Ok(1)
}

/// §16.7 teleport pad.
fn teleport_pad<W: MechWorld>(w: &mut W, op: &Operate) -> Result<i32, ObjectError> {
    let o = op.object;
    let Some(r0) = w.room(o) else {
        return Ok(0);
    };
    let class = w.unit_class(o);
    let partner_in = |w: &W, room: RoomId| {
        w.room_units(room).into_iter().find(|&u| {
            u != o && w.unit_type(u) == Some(unit_type::OBJECT) && w.unit_class(u) == class
        })
    };
    let mut found = partner_in(w, r0).map(|u| (u, r0));
    if found.is_none() {
        for r in w.adjacent_rooms(r0) {
            if r == r0 {
                continue;
            }
            if let Some(u) = partner_in(w, r) {
                found = Some((u, r));
                break;
            }
        }
    }
    let Some((pt, rt)) = found else {
        return Ok(0);
    };
    let Some(p) = op.operator else {
        return Ok(0);
    };
    let (x, y) = w.position(pt);
    let Some((room, x, y)) = w.free_point(rt, x, y, PAD_SPOT_SIZE, PAD_SPOT_MASK) else {
        return Ok(0);
    };
    if !w.place_unit(p, room, x, y) {
        return Ok(0);
    }
    w.send_room_reveal(p, rt);
    w.queue_update(p);
    w.set_flags2(p, PAD_FLAGS2);
    Ok(0)
}

/// §16.10 bank.
fn bank<W: MechWorld>(w: &mut W, op: &Operate) -> Result<i32, ObjectError> {
    if op.class != BANK_CLASS {
        return Ok(0);
    }
    let Some(p) = op.operator else {
        return Ok(0);
    };
    let towns =
        w.room(p).is_some_and(|r| w.in_town(r)) && w.room(op.object).is_some_and(|r| w.in_town(r));
    if towns {
        let guid = w.guid(op.object);
        w.set_interact(p, unit_type::OBJECT, guid);
        w.send(p, &[0x77, 0x10]);
        w.recount_tomes(p);
    }
    Ok(0)
}

/// §16.11 operate 47 (and 50's warp).
fn stairs<W: MechWorld>(t: &ObjectTables, w: &mut W, op: &Operate) -> Result<i32, ObjectError> {
    let o = op.object;
    match w.mode(o) {
        0 => {
            set_mode(t, w, o, op.class, 1, true)?;
            schedule_endanim(w, t.object(op.class)?, o);
        }
        2 => {
            let Some(r0) = w.room(o) else {
                return Ok(1);
            };
            let mut tile = warp_tile(w, r0);
            if tile.is_none() {
                for r in w.adjacent_rooms(r0) {
                    if r == r0 {
                        continue;
                    }
                    if let Some(u) = warp_tile(w, r) {
                        tile = Some(u);
                        break;
                    }
                }
            }
            if let (Some(tile), Some(p)) = (tile, op.operator) {
                w.warp_through_tile(p, tile);
            }
        }
        _ => {}
    }
    Ok(1)
}

/// §16.12 jungle stash.
fn jungle_stash<W: MechWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    op: &Operate,
) -> Result<i32, ObjectError> {
    let o = op.object;
    if !mode0(w, o) {
        return Ok(1);
    }
    let r = t.object(op.class)?;
    set_mode(t, w, o, op.class, 1, true)?;
    clear_selectable(w, o);
    schedule_endanim(w, r, o);
    let at = w
        .frame()
        .wrapping_add(w.tick_rate().ticks(r.parm1 as i32))
        .wrapping_add(1);
    w.schedule(o, mevent::STASH_DROP, at);
    if r.hascollision1 == 0 {
        w.free_footprint(o);
    }
    trap_arm(ctl, t, w, o)?;
    let owner = op.operator.map_or(-1, |p| w.guid(p) as i32);
    ctl.get_mut(o)?.owner = Some(owner);
    Ok(1)
}

/// §16.13 Harrogath gate.
fn gate<W: MechWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    op: &Operate,
) -> Result<i32, ObjectError> {
    let o = op.object;
    let now = w.host_tick();
    if now < ctl.get(o)?.last_tick.wrapping_add(GATE_DEBOUNCE) {
        return Ok(1);
    }
    match w.mode(o) {
        0 => {
            w.free_footprint(o);
            set_mode(t, w, o, op.class, 1, true)?;
            ctl.get_mut(o)?.last_tick = now;
            schedule_endanim(w, t.object(op.class)?, o);
        }
        2 => {
            let (x, y) = w.position(o);
            w.stamp_footprint(o, w.room(o), x, y);
            set_mode(t, w, o, op.class, 0, true)?;
            ctl.get_mut(o)?.last_tick = now;
        }
        _ => {}
    }
    Ok(1)
}

/// Operate 48 `0x005869F0` (§18.4).
fn trapped_soul_operate<W: MechWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    op: &Operate,
) -> Result<i32, ObjectError> {
    let o = op.object;
    if w.mode(o) != 0 {
        return Ok(0);
    }
    let r = ctl.seed.step() % 100;
    if r < 90 {
        if w.chest_drop(op, 0).is_some() {
            set_mode(t, w, o, op.class, 5, true)?;
        }
    } else {
        set_mode(t, w, o, op.class, 1, true)?;
        schedule_endanim(w, t.object(op.class)?, o);
        let at = w.frame() + w.tick_rate().ticks(20);
        w.schedule(o, mevent::SOUL, at);
    }
    Ok(0)
}

// ------------------------------------------------------------------ §17

/// The small init functions of §17 (and init 51, §18.4). `room`, `x`,
/// `y`: the init record's.
#[allow(clippy::too_many_arguments)]
pub fn init<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    n: u8,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) -> Result<(), ObjectError> {
    let class = ctl.get(obj)?.class;
    let f = w.frame();
    let rate = w.tick_rate();
    match n {
        8 | 26 => set_mode(t, w, obj, class, if n == 8 { 2 } else { 1 }, true)?,
        14 => set_mode(t, w, obj, class, 1, true)?,
        10 => {
            if w.level(obj) == Some(1) {
                w.schedule(obj, mevent::EVENT8, f + rate.ticks(60));
            } else {
                set_mode(t, w, obj, class, 2, true)?;
                w.schedule(obj, mevent::FIRE, f + rate.ticks(25));
            }
        }
        13 => {
            if !w.quest_link(obj, 4) && w.mode(obj) != 2 {
                set_mode(t, w, obj, class, 2, true)?;
            }
        }
        22 => {
            let r = t.object(class)?;
            if r.mode2 != 0 && w.mode(obj) == 0 && r.mode0 == 0 {
                set_mode(t, w, obj, class, 2, true)?;
            }
            w.schedule(obj, mevent::FIRE, f + rate.ticks(25));
        }
        24 => w.schedule(obj, mevent::SPIKE, f + rate.ticks(25)),
        27 => {
            let it = if ctl.seed.step() % 1000 <= 332 { 3 } else { 0 };
            ctl.get_mut(obj)?.interact = it;
        }
        28 => gold_placeholder(ctl, t, w, obj, class, room, x, y)?,
        34 => {
            if ctl.seed.step() & 1 == 1 {
                set_mode(t, w, obj, class, 1, true)?;
            }
        }
        51 => {
            if matches!(w.mode(obj), 1 | 2) {
                w.schedule(obj, mevent::SOUL, f + rate.ticks(35));
            }
        }
        58 => {
            let lo = ctl.seed.step();
            w.schedule(obj, mevent::EVENT8, f + rate.ticks(25 + (lo % 250) as i32));
        }
        _ => {}
    }
    Ok(())
}

/// Init 28 `0x0054F8C0` (§17 "Gold placeholder").
#[allow(clippy::too_many_arguments)]
fn gold_placeholder<W: ObjectWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    obj: UnitId,
    class: u16,
    room: Option<RoomId>,
    x: i32,
    y: i32,
) -> Result<(), ObjectError> {
    if w.mode(obj) != 0 {
        return Ok(());
    }
    set_mode(t, w, obj, class, 2, true)?;
    let n = ctl.seed.step() % 9 + 1;
    let mut l = (x, y);
    for _ in 0..n {
        let dx = (ctl.seed.step() & 3) as i32;
        let dy = (ctl.seed.step() & 3) as i32;
        let Some(room) = room else {
            continue;
        };
        if w.room_at(room, l.0 + dx, l.1 + dy) == Some(room) {
            let p = (x + dx, y + dy);
            l = p;
            if w.point_free(room, p.0, p.1, GOLD_POINT_MASK) {
                w.gold_drop(room, p.0, p.1);
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ §18

/// Burn `0x00581680`: every player of O's room not in mode 17 within
/// `Parm0` + 1 (size-adjusted distance) takes trap damage (arg 1).
fn burn<W: MechWorld>(
    t: &ObjectTables,
    w: &mut W,
    o: UnitId,
    class: u16,
) -> Result<(), ObjectError> {
    let reach = t.object(class)?.parm0 as i32 + 1;
    let Some(room) = w.room(o) else {
        return Ok(());
    };
    for u in w.room_units(room) {
        if w.unit_type(u) != Some(unit_type::PLAYER) || w.mode(u) == PLAYER_DEATH_MODE {
            continue;
        }
        let d = reach_distance(w.position(o), w.unit_size(o), w.position(u), w.unit_size(u));
        if d <= reach {
            w.trap_damage_arg(o, u, 1);
        }
    }
    Ok(())
}

/// The players of O's room "on" the object (distance ≤ 0, not mode 17).
fn players_on<W: MechWorld>(w: &W, o: UnitId) -> Vec<UnitId> {
    let Some(room) = w.room(o) else {
        return Vec::new();
    };
    w.room_units(room)
        .into_iter()
        .filter(|&u| {
            w.unit_type(u) == Some(unit_type::PLAYER)
                && w.mode(u) != PLAYER_DEATH_MODE
                && reach_distance(w.position(o), w.unit_size(o), w.position(u), w.unit_size(u)) <= 0
        })
        .collect()
}

/// Object events 0, 3, 8, 9, 10 (§18).
pub fn event<W: MechWorld>(
    ctl: &mut ObjectControl,
    t: &ObjectTables,
    w: &mut W,
    o: UnitId,
    ev: u8,
) -> Result<(), ObjectError> {
    let class = ctl.get(o)?.class;
    let f = w.frame();
    let rate = w.tick_rate();
    match ev {
        // §18.1.
        mevent::FIRE => {
            if w.mode(o) == 1 {
                w.store_mode(o, 2);
            }
            burn(t, w, o, class)?;
            let lo = ctl.seed.step();
            w.schedule(o, mevent::FIRE, f + rate.ticks(15 + (lo % 35) as i32));
        }
        // §18.2.
        mevent::SPIKE => {
            let s = ctl.get(o)?.interact;
            match s {
                0 => {
                    let on = players_on(w, o);
                    for _ in &on {
                        w.sound(o, sound::TRAP_ARMED, None, false);
                    }
                    if on.is_empty() {
                        w.schedule(o, mevent::SPIKE, f + rate.ticks(15));
                    } else {
                        ctl.get_mut(o)?.interact = 1;
                        w.schedule(o, mevent::SPIKE, f + rate.ticks(25));
                    }
                }
                1 => {
                    let on = players_on(w, o);
                    for &u in &on {
                        w.trap_damage_arg(o, u, 0);
                    }
                    if on.is_empty() {
                        ctl.get_mut(o)?.interact = 0;
                    } else {
                        ctl.get_mut(o)?.interact = 2;
                        set_mode(t, w, o, class, 1, true)?;
                    }
                    let fc1 = (t.object(class)?.framecnt1 >> 8) as i32;
                    w.schedule(o, mevent::SPIKE, f + fc1 + 1);
                }
                2 => {
                    if w.mode(o) != 0 {
                        set_mode(t, w, o, class, 0, true)?;
                        ctl.get_mut(o)?.interact = 0;
                    }
                    w.schedule(o, mevent::SPIKE, f + rate.ticks(15));
                }
                _ => w.schedule(o, mevent::SPIKE, f + rate.ticks(15)),
            }
        }
        // §18.3.
        mevent::EVENT8 => {
            if class == FISSURE_CLASS {
                let m = w.mode(o);
                if m != 0 && m != 2 {
                    return Ok(());
                }
                let lo = ctl.seed.step();
                w.schedule(o, mevent::EVENT8, f + rate.ticks(25 + (lo % 250) as i32));
                set_mode(t, w, o, class, 1, true)?;
                schedule_endanim(w, t.object(class)?, o);
            } else {
                let p = w.day_period_act1();
                match p {
                    0 => {
                        if matches!(w.mode(o), 1 | 2) {
                            set_mode(t, w, o, class, 0, true)?;
                        }
                        w.schedule(o, mevent::EVENT8, f + rate.ticks(1000));
                    }
                    1..=3 => {
                        if w.mode(o) == 0 {
                            set_mode(t, w, o, class, 1, true)?;
                            schedule_endanim(w, t.object(class)?, o);
                        }
                        w.schedule(o, mevent::EVENT8, f + rate.ticks(1000));
                    }
                    _ => w.schedule(o, mevent::EVENT8, f + rate.ticks(600)),
                }
            }
        }
        // §18.4.
        mevent::SOUL => match w.mode(o) {
            1 => w.schedule(o, mevent::SOUL, f + rate.ticks(10)),
            2 => {
                let s = ctl.get(o)?.interact.wrapping_add(1);
                ctl.get_mut(o)?.interact = s;
                if s < 2 {
                    burn(t, w, o, class)?;
                    w.schedule(o, mevent::SOUL, f + rate.ticks(25));
                } else {
                    let op = Operate {
                        object: o,
                        operator: None,
                        class,
                        operate_fn: t.object(class)?.operatefn,
                    };
                    if w.chest_drop(&op, 0).is_some() {
                        set_mode(t, w, o, class, 3, true)?;
                        // Mode 3's frames, one per base tick.
                        let fc3 = (frame_cnt(t.object(class)?, 3) >> 8) as i32;
                        w.schedule(o, mevent::SOUL, f + w.tick_rate().ticks(fc3) + 1);
                    }
                }
            }
            3 => set_mode(t, w, o, class, 4, true)?,
            _ => {}
        },
        // §18.5.
        mevent::STASH_DROP => {
            let owner = ctl.get(o)?.owner;
            let operator = owner
                .and_then(|g| u32::try_from(g).ok())
                .and_then(|g| w.find_player(g));
            let op = Operate {
                object: o,
                operator,
                class,
                operate_fn: t.object(class)?.operatefn,
            };
            w.chest_drop(&op, 0);
        }
        _ => {}
    }
    Ok(())
}
