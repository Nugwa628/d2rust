//! Random bosses (champion and unique
//! minions), packs and party minions.

use super::placement::{self, flags, place_at, place_near, spawn_point, Placed};
use super::seams::{OwnerKey, PopHost};
use super::{CoordRect, Ctx};
use crate::units::{RoomId, UnitId};

/// Monster type flags (monster data +0x16).
pub mod type_flag {
    pub const OTHER_BOSS: u16 = 0x1;
    pub const SUPERUNIQUE: u16 = 0x2;
    pub const CHAMPION: u16 = 0x4;
    pub const UNIQUE: u16 = 0x8;
    pub const MINION: u16 = 0x10;
}

/// Random-boss minion range.
pub const BOSS_MINIONS: (i32, i32) = (3, 6);

/// Tentacle offsets: set 0 then set 1.
pub const TENTACLE_OFFSETS: [(i32, i32); 12] = [
    (-1, -4),
    (1, 4),
    (1, -3),
    (-1, 3),
    (0, 2),
    (0, -2),
    (-3, -1),
    (3, 1),
    (2, -1),
    (-2, 1),
    (1, 0),
    (-1, 0),
];

/// The random-boss spawn `(game, room, cl, class, champion allowed, x, y,
/// warp check)`.
#[allow(clippy::too_many_arguments)]
pub fn random_boss<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    cl: Option<CoordRect>,
    class: i32,
    champion_allowed: bool,
    x: i32,
    y: i32,
    warp_check: bool,
) -> Option<UnitId> {
    let boss = boss_spawn(cx, room, cl, x, y, None, class, warp_check)?;
    cx.host.boss_modifiers(boss, champion_allowed);
    boss_minions_and_init(cx, boss, BOSS_MINIONS.0, BOSS_MINIONS.1, cl);
    Some(boss)
}

/// Unique minions, then
/// the modifier init functions.
pub fn boss_minions_and_init<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    boss: UnitId,
    min: i32,
    max: i32,
    cl: Option<CoordRect>,
) {
    unique_minions(cx, boss, min, max, cl);
    cx.host.boss_modifier_init(boss);
}

#[allow(clippy::too_many_arguments)]
pub fn boss_spawn<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    cl: Option<CoordRect>,
    mut x: i32,
    mut y: i32,
    guid: Option<u32>,
    class: i32,
    warp_check: bool,
) -> Option<UnitId> {
    if x == 0 && y == 0 {
        (x, y) = spawn_point(cx, room, cl, class, warp_check)?;
    }
    // Every creation call passes mode 1 (neutral), as for every other
    // population spawn.
    let mode = 1;
    let at = |cx: &mut Ctx<'_, H>, room: RoomId, x, y, r, f| {
        let req = placement::SpawnReq {
            room: Some(room),
            cl,
            class,
            mode,
            guid,
            x,
            y,
            r,
            flags: f,
        };
        placement::place(cx, req).unit()
    };
    let boss = if cl.is_some() {
        at(cx, room, x, y, -1, flags::NO_PARTY)
    } else if guid.is_none() {
        at(cx, room, x, y, -1, flags::NO_PARTY).or_else(|| at(cx, room, x, y, 5, flags::NO_PARTY))
    } else {
        // Restore paths (not population).
        let f = 0x62;
        at(cx, room, x, y, -1, f)
            .or_else(|| at(cx, room, x, y, 5, f))
            .or_else(|| {
                // The spawn-point search moves the point only when it succeeds; the
                // nearest free point then searches from wherever the point
                // is and hands its own room on.
                let (mut sx, mut sy) = (x, y);
                if let Some((px, py)) = spawn_point(cx, room, None, class, false) {
                    if let Some(u) = at(cx, room, px, py, -1, f) {
                        return Some(u);
                    }
                    (sx, sy) = (px, py);
                }
                let (fr, px, py) = cx.host.nearest_free_point(room, sx, sy)?;
                at(cx, fr, px, py, -1, f)
            })
    }?;
    if cx.host.type_flags(boss) & type_flag::UNIQUE == 0 {
        let lvl = cx.host.unit_level(boss);
        if let Some(r) = cx.state.regions.get_mut(lvl) {
            r.bosses = r.bosses.wrapping_add(1);
        }
    }
    cx.host.set_type_flags(boss, type_flag::UNIQUE);
    cx.host.set_type_flags(boss, type_flag::OTHER_BOSS);
    cx.host.boss_quest_hook(boss);
    cx.host.set_owner_data(boss, OwnerKey::Guid(boss), 1, 1, 0);
    Some(boss)
}

/// The plain-record restore of an inactive unit (the creation chain's
/// step 4 with the record's GUID): the
/// creation at the stored point with r = −1, again with
/// r = 5, at a spawn point (no warp check) with r = −1, and
/// last at the nearest free point (mask 0x3C01, size 1)
/// with r = −1 in the room it returns. The last search starts from the
/// Point when that search found one (its creation failed), else from
/// (x, y). `flags`: 0x62 / 0x6A, | 0x80 for a flying class (the caller's).
#[allow(clippy::too_many_arguments)]
pub fn restore_plain<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    x: i32,
    y: i32,
    guid: u32,
    class: i32,
    mode: u8,
    flags: u16,
) -> Option<UnitId> {
    let at = |cx: &mut Ctx<'_, H>, room: RoomId, x, y, r| {
        let req = placement::SpawnReq {
            room: Some(room),
            cl: None,
            class,
            mode,
            guid: Some(guid),
            x,
            y,
            r,
            flags,
        };
        placement::place(cx, req).unit()
    };
    if let Some(u) = at(cx, room, x, y, -1).or_else(|| at(cx, room, x, y, 5)) {
        return Some(u);
    }
    let (mut sx, mut sy) = (x, y);
    if let Some((px, py)) = spawn_point(cx, room, None, class, false) {
        if let Some(u) = at(cx, room, px, py, -1) {
            return Some(u);
        }
        (sx, sy) = (px, py);
    }
    let (fr, px, py) = cx.host.nearest_free_point(room, sx, sy)?;
    at(cx, fr, px, py, -1)
}

/// 1–3 champion minions, each with
/// modifier 16.
pub fn champion_minions<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    cl: Option<CoordRect>,
    boss: UnitId,
    class: i32,
) {
    if cx.host.type_flags(boss) & type_flag::CHAMPION == 0 {
        return;
    }
    let count = cx.host.unit_seed(boss).step() % 3 + 1;
    for _ in 0..count {
        if let Some(m) = place_near(cx, cl, boss, class, 1, 4, 0).unit() {
            cx.host.add_modifier(m, 16, cx.state);
        }
    }
}

/// A unique's minions.
pub fn unique_minions<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    boss: UnitId,
    min: i32,
    max: i32,
    cl: Option<CoordRect>,
) {
    if cx.host.type_flags(boss) & type_flag::CHAMPION != 0 {
        return;
    }
    let boss_class = cx.host.unit_class(boss);
    let class = cx
        .tables
        .mon(boss_class)
        .map(|m| i32::from(m.minion1))
        .filter(|&m| cx.tables.mon(m).is_some())
        .unwrap_or(boss_class);
    let n = max.wrapping_sub(min).wrapping_add(1);
    let count = (cx.host.unit_seed(boss).roll(n) as i32).wrapping_add(min);
    for _ in 0..count.max(0) {
        let Some(m) = place_near(cx, cl, boss, class, 1, 3, flags::NO_PARTY).unit() else {
            continue;
        };
        cx.host.transfer_modifiers(boss, m);
        // Owner data `(game, minion, boss GUID, 1, 0, 0)`.
        cx.host.set_owner_data(m, OwnerKey::Guid(boss), 1, 0, 0);
        cx.host.add_minion(boss, m);
        cx.host.set_owner(m, boss);
        cx.host.set_type_flags(m, type_flag::MINION);
    }
}

/// A pack of `class`. Returns
/// the leader.
pub fn pack<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    room: RoomId,
    cl: CoordRect,
    class: i32,
) -> Option<UnitId> {
    let t = cx.tables;
    let mon = t.mon(class)?;
    let (min, max) = if matches!(mon.base_id, 19 | 91) {
        (1, 1)
    } else {
        (i32::from(mon.min_grp), i32::from(mon.max_grp))
    };
    if mon.sparse_populate != 0 && cx.host.game_seed().step() % 100 > u32::from(mon.sparse_populate)
    {
        return None;
    }
    if min == 0 || max == 0 || max < min {
        return None;
    }
    let (x, y) = spawn_point(cx, room, Some(cl), class, true)?;
    let leader = place_at(cx, room, Some(cl), x, y, class, 1, -1, 0).unit()?;
    if class == 528 && t.mon2(class).is_some_and(|m| m.obj_col) {
        let (lx, ly) = cx.host.unit_position(leader);
        let lroom = cx.host.unit_room(leader).unwrap_or(room);
        cx.host.create_object(lroom, 562, lx, ly);
    }
    members(cx, Some(cl), leader, class, min, max);
    Some(leader)
}

/// The pack's step 5: `roll(max − min + 1) + (min − 1)` members
/// on the leader's unit seed, each near the leader (r 3, flags 0).
pub fn members<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    cl: Option<CoordRect>,
    leader: UnitId,
    class: i32,
    min: i32,
    max: i32,
) {
    let n = (cx.host.unit_seed(leader).roll(max - min + 1) as i32) + (min - 1);
    for _ in 0..n.max(0) {
        let _ = place_near(cx, cl, leader, class, 1, 3, 0);
    }
}

/// The party of a monster created without flag 0x40.
pub fn party<H: PopHost + ?Sized>(cx: &mut Ctx<'_, H>, leader: UnitId, class: i32, cflags: u16) {
    let t = cx.tables;
    let Some(mon) = t.mon(class) else {
        return;
    };
    let m1 = i32::from(mon.minion1);
    if t.mon(m1).is_none() {
        return;
    }
    let tentacle = mon.base_id == 261;
    if mon.set_boss || tentacle {
        cx.host.set_owner_data(
            leader,
            OwnerKey::Guid(leader),
            1,
            1,
            i32::from(mon.boss_xfer),
        );
    }
    let count = if mon.party_min >= mon.party_max {
        i32::from(mon.party_min)
    } else {
        let n = i32::from(mon.party_max) - i32::from(mon.party_min) + 1;
        i32::from(mon.party_min) + cx.host.unit_seed(leader).roll(n) as i32
    };
    if tentacle {
        let set = usize::from(!(cflags >> 2) & 1 == 1);
        tentacles(cx, leader, m1, count, set);
        return;
    }
    let m2 = i32::from(mon.minion2);
    let two = t.mon(m2).is_some();
    for i in 0..count {
        let c = if i % 2 == 0 || !two { m1 } else { m2 };
        let Some(m) = place_near(cx, None, leader, c, 1, 4, flags::NO_PARTY).unit() else {
            continue;
        };
        if mon.set_boss {
            cx.host.set_owner_data(m, OwnerKey::DataOf(leader), 1, 0, 0);
            cx.host.add_minion(leader, m);
        }
    }
}

fn tentacles<H: PopHost + ?Sized>(
    cx: &mut Ctx<'_, H>,
    leader: UnitId,
    class: i32,
    count: i32,
    set: usize,
) {
    let mut k = (cx.host.unit_seed(leader).step() % 6) as usize;
    let room = cx.host.unit_room(leader);
    let (lx, ly) = cx.host.unit_position(leader);
    for _ in 0..count.max(0) {
        let (ox, oy) = TENTACLE_OFFSETS[6 * set + k];
        let req = placement::SpawnReq {
            room,
            cl: None,
            class,
            mode: 1,
            guid: None,
            x: lx + ox,
            y: ly + oy,
            r: -1,
            flags: flags::NO_PARTY,
        };
        if let Placed::Unit(m) = placement::place(cx, req) {
            // Owner data and minion list as in 10.2.3, unconditional (no
            // `SetBoss` test inside
            // R1).
            cx.host.set_owner_data(m, OwnerKey::DataOf(leader), 1, 0, 0);
            cx.host.add_minion(leader, m);
        }
        k = (k + 5) % 6;
    }
}
