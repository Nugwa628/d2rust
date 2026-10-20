//! Item drop helpers of the object and quest code, on the action wiring's
//! units, stats and DRLG: armor and weapon drops (room seed, area level − 1),
//! gold, drops by source unit (the unit seed, the unit's level) and the code
//! drop ([`code_drop`]).
//!
//! The class picks are [`crate::treasure::class_pick`]; the item is a real
//! item unit created by [`Economy::create_item`] in the game's one item
//! store, placed by the floor search (the path provider's floor drop with its
//! field, else the [`FreeSpot`] seam, as the chest drop:
//! `super::death::Spots`). The pick rows ([`DeathDrops::picks`]) and the
//! created items ([`DeathDrops::placed`]) are the drop state's; a pick fatal
//! goes to [`DeathDrops::pick_errors`].

use d2_data::tables::Levels;

use super::death::Spots;
use super::{DeathDrops, DropPlacer, Economy, FreeSpot, GameFields, ItemSpawn};
use crate::items::{ItemGame, ItemRequest};
use crate::rng::Seed;
use crate::treasure::area_level;
use crate::treasure::class_pick::{self, Part};
use crate::units::hooks::Sim;
use crate::units::{RoomId, UnitId, UnitType};
use crate::wiring::action::{ActionHooks, Pending};

/// `flags2` of the armor and weapon helpers (superior fallback).
pub const FLAGS2_SUPERIOR: u32 = 0x40;
/// The gold item's flag cleared after creation.
pub const GOLD_CLEARED_FLAG: u32 = 0x2000;
/// Spawn mode 3 (ground); the init flags are 1.
const SPAWN_GROUND: u32 = 3;
const STAT_LEVEL: u16 = 12;

/// The level id of a room (DRLG), as i32 for [`area_level`].
fn room_level<X: Pending>(h: &ActionHooks<X>, sim: &Sim<'_>, room: Option<RoomId>) -> i32 {
    room.and_then(|rm| h.drlg.level_id(sim.game, rm))
        .map_or(0, |l| i32::try_from(l).unwrap_or(i32::MAX))
}

/// The area level of the room's level.
fn room_area_level<X: Pending>(
    h: &ActionHooks<X>,
    sim: &Sim<'_>,
    levels: &[Levels],
    room: Option<RoomId>,
) -> i32 {
    area_level(
        levels,
        room_level(h, sim, room),
        h.ai_info.difficulty,
        sim.data.expansion,
    )
}

/// The active room seed of the room; `None`: no such room.
fn room_seed<'h, X: Pending>(
    h: &'h mut ActionHooks<X>,
    sim: &Sim<'_>,
    room: RoomId,
) -> Option<&'h mut Seed> {
    let act = sim.game.lists.room(room)?.act;
    let d = h.drlg.dungeon.acts.get_mut(usize::from(act))?.as_mut()?;
    let r = d.drlg_room_of(room)?;
    d.active_room_seed_mut(r)
}

/// The floor search from `pos` and the creation of `rq`: `None` when no spot
/// is found (nothing created) or the creation fails
/// ([`DeathDrops::failures`]). The request's source unit is `unit`'s
/// ([`Economy::request_unit`]).
#[allow(clippy::too_many_arguments)]
fn create_at<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    spots: &mut F,
    room: Option<RoomId>,
    pos: (i32, i32),
    unit: Option<UnitId>,
    mut rq: ItemRequest,
) -> Option<UnitId> {
    let (x, y) = pos;
    // The start offset when a room exists there (the floor drop does it
    // itself with the provider).
    let start_room = room.and_then(|rm| h.drlg.find_room(sim.game, rm, x + 2, y + 3));
    let start = match start_room {
        Some(_) => (x + 2, y + 3),
        None => (x, y),
    };
    let t = d.tables.clone();
    let mut fields = GameFields::from_action(
        h.game_seed,
        &h.ai_info,
        sim.data.expansion,
        std::mem::take(&mut h.uniques),
    );
    let mut items = std::mem::take(&mut h.items);
    let out = {
        let mut econ = Economy {
            game: &mut *sim.game,
            units: &mut *sim.units,
            stats: &mut *sim.stats,
            data: sim.data,
            hooks: &mut *h,
            fields: &mut fields,
            tables: &t.items,
            items: &mut items,
        };
        let mut placer = Spots {
            inner: spots,
            room,
            start_room,
            start,
        };
        match placer.place(&mut econ, x, y) {
            None => None,
            Some(spot) => {
                rq.unit = unit.and_then(|u| econ.request_unit(u, None));
                rq.format = ItemGame::item_format(&*econ.fields);
                let spawn = ItemSpawn {
                    room: spot.room,
                    mode: SPAWN_GROUND,
                    init_flags: 1,
                };
                match econ.create_item(&mut rq, false, spawn) {
                    Ok(u) => {
                        placer.placed(&mut econ, u, spot);
                        Some((u, spot))
                    }
                    Err(e) => {
                        d.failures.push(e);
                        None
                    }
                }
            }
        }
    };
    h.items = items;
    h.game_seed = fields.seed;
    h.uniques = std::mem::take(&mut fields.uniques);
    d.fields = fields;
    let (u, spot) = out?;
    d.placed.push((u, spot));
    Some(u)
}

/// Armor or weapon (`weapon`) drop with (room, &pos, `t`, act flag `a`,
/// `unit`): L := area level of the room's level, minus 1 when > 1; the pick
/// on the room seed; floor search; request ilvl L, flags2 0x40.
#[allow(clippy::too_many_arguments)]
pub fn stand_drop<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    levels: &[Levels],
    spots: &mut F,
    room: Option<RoomId>,
    pos: (i32, i32),
    weapon: bool,
    t: i32,
    a: bool,
    unit: Option<UnitId>,
) -> Option<UnitId> {
    let mut l = room_area_level(h, sim, levels, room);
    if l > 1 {
        l -= 1;
    }
    let picks = d.picks.clone();
    let expansion = sim.data.expansion;
    // No room means no seed and no floor (the search finds nothing): no
    // draw, no item.
    let room_id = room?;
    let seed = room_seed(h, sim, room_id)?;
    let id = if weapon {
        class_pick::weapon_rack_pick(&picks, seed, l, t, a, expansion)
    } else {
        class_pick::part_pick(&picks, Part::Armor, seed, l, t, a, false, expansion)
    };
    if id < 0 {
        return None;
    }
    let rq = ItemRequest {
        ilvl: l,
        item: id,
        flags2: FLAGS2_SUPERIOR,
        ..ItemRequest::default()
    };
    create_at(h, sim, d, spots, room, pos, unit, rq)
}

/// A gold pile at `pos` in `room`, ilvl L (area level − 1 when > 1), unit
/// none; flag 0x2000 cleared on the item. The amount is the item pipeline's
/// gold rule.
pub fn gold_drop<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    levels: &[Levels],
    spots: &mut F,
    room: Option<RoomId>,
    pos: (i32, i32),
) -> Option<UnitId> {
    let mut l = room_area_level(h, sim, levels, room);
    if l > 1 {
        l -= 1;
    }
    let id = d.picks.find_code(class_pick::GOLD_CODE)? as i32;
    let rq = ItemRequest {
        ilvl: l,
        item: id,
        ..ItemRequest::default()
    };
    let u = create_at(h, sim, d, spots, room, pos, None, rq)?;
    if let Some(i) = h.items.get_mut(u) {
        i.flags &= !GOLD_CLEARED_FLAG;
    }
    Some(u)
}

/// The level of a drop by source unit: monster total stat 12, player base
/// stat 12, other units the area level of their room's level; at most 1
/// becomes 1.
pub fn source_level<X: Pending>(
    h: &ActionHooks<X>,
    sim: &Sim<'_>,
    levels: &[Levels],
    u: UnitId,
) -> i32 {
    let l = match sim.units.get(u).map(|r| r.ty) {
        Some(UnitType::Monster) => sim.stats.unit_total(u, STAT_LEVEL, 0),
        Some(UnitType::Player) => sim.stats.unit_base(u, STAT_LEVEL, 0),
        _ => {
            let room = sim.game.lists.unit(u).and_then(|e| e.room());
            room_area_level(h, sim, levels, room)
        }
    };
    l.max(1)
}

/// The drop by source unit `u` with quality `quality`, `t` and `p7`: `code`
/// is the unit's drop code (0 = none, the random class on the unit seed).
/// Returns the item and the level used (given even when nothing is
/// created).
#[allow(clippy::too_many_arguments)]
pub fn source_drop<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    levels: &[Levels],
    spots: &mut F,
    u: UnitId,
    code: u32,
    quality: u8,
    t: i32,
    p7: bool,
) -> (Option<UnitId>, i32) {
    let l = source_level(h, sim, levels, u);
    let Some(rec) = sim.units.get(u) else {
        return (None, l);
    };
    let monster = rec.ty == UnitType::Monster;
    let mut seed = rec.seed;
    let picks = d.picks.clone();
    let id = class_pick::source_class(
        &picks,
        &mut seed,
        code,
        l,
        quality,
        t,
        p7,
        monster,
        sim.data.expansion,
    );
    if let Some(r) = sim.units.get_mut(u) {
        r.seed = seed;
    }
    let id = match id {
        Ok(id) => id,
        Err(e) => {
            d.pick_errors.push(e);
            return (None, l);
        }
    };
    let room = sim.game.lists.unit(u).and_then(|e| e.room());
    let pos = h.path_position(u);
    let rq = ItemRequest {
        ilvl: l,
        item: id,
        quality,
        ..ItemRequest::default()
    };
    (create_at(h, sim, d, spots, room, pos, Some(u), rq), l)
}

/// The code drop of unit `u`: the item named by `code` (used by objects and
/// the quest gold piles). An unknown code gives none (no fatal); the floor
/// search from the unit's position comes first, then the level is read
/// (neither draws); the request carries the unit, the level, the item, spawn
/// mode 3, init flags 1, the game's format and `quality`, every other field
/// zero. No draw.
#[allow(clippy::too_many_arguments)]
pub fn code_drop<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    levels: &[Levels],
    spots: &mut F,
    u: UnitId,
    code: u32,
    quality: u8,
) -> Option<UnitId> {
    if code == 0 {
        return None;
    }
    let id = d.picks.find_code(code.to_le_bytes())? as i32;
    let room = sim.game.lists.unit(u).and_then(|e| e.room());
    let pos = h.path_position(u);
    let rq = ItemRequest {
        ilvl: source_level(h, sim, levels, u),
        item: id,
        quality,
        ..ItemRequest::default()
    };
    create_at(h, sim, d, spots, room, pos, Some(u), rq)
}

/// The gem drop and the inline potion drop of the exploding and poison
/// shrines: the floor search from the player `p`'s position and `p`'s level,
/// but a request that names no source unit (its unit fields stay zero, so the
/// item seed comes from the game's draw, not `p`'s). An unknown code gives
/// none. `quality`: 2 for the gem drop, 0 (rolled) for the potions. The
/// potion request also sets a quantity override of 1; it is left 0 here
/// because the stack roll draws either way and the caller sets stat 70 := 1
/// after creation, so the result is the same.
#[allow(clippy::too_many_arguments)]
pub fn near_player_drop<X: Pending, F: FreeSpot>(
    h: &mut ActionHooks<X>,
    sim: &mut Sim<'_>,
    d: &mut DeathDrops,
    levels: &[Levels],
    spots: &mut F,
    p: UnitId,
    code: u32,
    quality: u8,
) -> Option<UnitId> {
    let id = d.picks.find_code(code.to_le_bytes())? as i32;
    let room = sim.game.lists.unit(p).and_then(|e| e.room());
    let pos = h.path_position(p);
    let rq = ItemRequest {
        ilvl: source_level(h, sim, levels, p),
        item: id,
        quality,
        ..ItemRequest::default()
    };
    create_at(h, sim, d, spots, room, pos, None, rq)
}
